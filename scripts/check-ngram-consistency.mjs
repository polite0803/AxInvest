#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 中文全文检索一致性校验（PostgreSQL 侧）。
 *
 * ## 为什么需要这个脚本
 *
 * 中文全文检索靠**两侧共用同一套 n-gram 归一化**实现：
 *   - Rust  侧：`crates/search/src/text_ngram.rs::cjk_ngram`
 *   - PG   侧：`ax_cjk_ngram()`（迁移 v227 建立）
 *
 * 索引由 PG 侧计算（生成列），查询由 Rust 侧计算（构造 tsquery）。
 * 两者若对同一输入产出不同结果，会**静默失配** —— 索引里存在的 token 在查询里
 * 取不到，表现为「搜不到但也不报错」，且 Rust 单测与 cargo check 全绿。
 * 只有拿同一份 fixture 同时喂给两侧并逐字节比对，才能拦住这类缺陷。
 *
 * ## 检查项
 *
 *   1. 函数一致性：用共享 fixture 逐条比对 `ax_cjk_ngram()` 与规范期望值
 *   2. 已落地检查：`notes` / `memory_items` / 全部 `vec_*_meta` 的 tsvector
 *      生成列确实使用 `ax_cjk_ngram`，且 GIN 索引存在
 *
 * ## 运行
 *
 * 需要 `pg` 驱动（**刻意不是项目依赖**）与可连的 PG 实例：
 *
 *   # 本机：连 ~/.axagent/db_config.json 里那台
 *   NODE_PATH=<有 pg 的 node_modules> node scripts/check-ngram-consistency.mjs
 *
 *   # CI：连 job 里的 PG service（CI 没有 ~/.axagent，必须走 env）
 *   AXAGENT_NGRAM_PG_URL=postgres://postgres:postgres@localhost:5432/axagent_test \
 *     node scripts/check-ngram-consistency.mjs --only=consistency
 *
 * 三种「本环境不具备运行条件」都**诚实跳过**（打印 SKIP 与原因）并返回 0：
 * pg 驱动缺失 / 连接配置缺失 / 连不上。注意：跳过不是通过，输出里会明确区分
 * （且不阻塞 CI）。`--only=consistency` 供 CI 用：那里是**干净库**、没有跑过
 * v227 迁移，故生成列与索引尚不存在，只验「两侧函数逐字节一致」这一半。
 *
 * 可选参数：
 *   --only=consistency  只跑函数一致性（迁移尚未执行时用）
 *   --only=deployed     只跑已落地检查
 */

import { existsSync, readFileSync } from "node:fs";
import { delimiter, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import { homedir } from "node:os";
import crypto from "node:crypto";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
// ⚠ 这两个路径在 2026-09-16 变过：定义原先住在 migrations/v227_cjk_fts.rs，
// 后来提取到 dao 根的 cjk_ngram.rs（否则 reconcile 会反向依赖 migrations/，
// 导致「删除版本化迁移」这件事做不成）。改路径时别只改一处。
const RUST_NGRAM_DEF = join(
  ROOT,
  "src-tauri/crates/dao/src/cjk_ngram.rs",
);
const SQL_TEMPLATE = join(
  ROOT,
  "src-tauri/crates/dao/src/sql/ax_cjk_ngram.sql",
);
const FIXTURE = join(
  ROOT,
  "src-tauri/crates/search/tests/fixtures/ngram_cases.json",
);

/** 与 Rust 侧 `ngram_consistency.rs` 保持同样的下限，防 fixture 被清空后假绿。 */
const MIN_CASES = 30;

/**
 * 从 Rust 侧分词器定义源码里提取字符类常量。
 *
 * 刻意解析源码而非在此另抄一份：脚本若自带一份定义，就又多了一处会漂移的副本，
 * 而本脚本存在的全部意义就是消除漂移。
 */
function extractClass(name, source) {
  const block = source.match(
    new RegExp(`pub const ${name}: &str = concat!\\(([\\s\\S]*?)\\n\\);`),
  );
  if (!block) {
    throw new Error(
      `未能在 ${RUST_NGRAM_DEF} 中定位常量 ${name} —— 常量可能被搬走了，` +
        `先确认真实位置再改本脚本的路径，别就地另抄一份`,
    );
  }
  const literals = [...block[1].matchAll(/r"([^"]*)"/g)].map((m) => m[1]);
  if (literals.length === 0) {
    throw new Error(`常量 ${name} 未解析出任何原始字符串字面量`);
  }
  return literals.join("");
}

/** 渲染建函数 SQL：注入两个字符类。 */
function renderFunctionSql() {
  const source = readFileSync(RUST_NGRAM_DEF, "utf8");
  const cjk = extractClass("CJK_CLASS", source);
  const sep = extractClass("SEP_CLASS", source);
  // 必须是 replaceAll：模板里 __CJK__ 出现多次，String.replace 只换第一处，
  // 会留下未替换的占位符 —— 那样建出的函数对部分分支失效且不报错。
  const sql = readFileSync(SQL_TEMPLATE, "utf8")
    .replaceAll("__CJK__", cjk)
    .replaceAll("__SEP__", sep);
  for (const placeholder of ["__CJK__", "__SEP__"]) {
    if (sql.includes(placeholder)) {
      throw new Error(`SQL 模板替换后仍残留占位符 ${placeholder}`);
    }
  }
  return { sql, cjk, sep };
}

/**
 * 解析 PG 连接配置；返回 null 表示**本环境没有可用配置**（调用方诚实 SKIP）。
 *
 * 优先级：`AXAGENT_NGRAM_PG_URL` 环境变量 > `~/.axagent/db_config.json`。
 *
 * 为什么需要 env 通道：`~/.axagent/` 是**开发机专有**的加密配置（`master.key`
 * 解出 `pg_password_enc`），CI 里不存在；而 CI 的 `test-unit` job 恰恰**有**一台
 * 真 PG service（`pgvector/pgvector:pg17`）。没有 env 通道时，这道门禁在 CI 里
 * 只能是空转 —— 而「装了门禁但从没见过它红」本身就是缺陷。
 *
 * 为什么配置缺失返回 null 而不是抛错交给 .catch：那会让退出码变成 **2（ERROR）**，
 * 语义上却是「本环境不具备运行条件」—— 与「没有 pg 驱动」同类，**不是校验失败**。
 * 实测该分支曾产出 `[ngram] ERROR: ENOENT ... db_config.json` 并 exit 2，
 * 在 CI 里表现为「恒红的噪音门」，而这正是本仓库反复踩过的形态。
 */
function resolvePgConfig() {
  const url = process.env.AXAGENT_NGRAM_PG_URL;
  if (url) {
    const u = new URL(url);
    return {
      host: u.hostname,
      port: u.port ? Number(u.port) : 5432,
      // pathname 带前导斜杠；库名可能被 percent-encode（如含中文）故解码
      database: decodeURIComponent(u.pathname.replace(/^\//, "")),
      user: decodeURIComponent(u.username),
      password: decodeURIComponent(u.password),
      ssl: false,
    };
  }

  const keyPath = join(homedir(), ".axagent", "master.key");
  const cfgPath = join(homedir(), ".axagent", "db_config.json");
  if (!existsSync(keyPath) || !existsSync(cfgPath)) return null;

  const key = readFileSync(keyPath);
  const cfg = JSON.parse(readFileSync(cfgPath, "utf8"));
  const payload = Buffer.from(cfg.pg_password_enc, "base64");
  const decipher = crypto.createDecipheriv(
    "aes-256-gcm",
    key,
    payload.subarray(0, 12),
  );
  decipher.setAuthTag(payload.subarray(payload.length - 16));
  const password = Buffer.concat([
    decipher.update(payload.subarray(12, payload.length - 16)),
    decipher.final(),
  ]).toString("utf8");
  return {
    host: cfg.pg_host,
    port: cfg.pg_port,
    database: cfg.pg_database,
    user: cfg.pg_user,
    password,
    ssl: false,
  };
}

/**
 * 尝试加载 pg 驱动；不存在则返回 null（调用方负责 SKIP）。
 *
 * 先按常规解析（项目自身 node_modules）。ESM 下 `createRequire` **不读 NODE_PATH**
 * 环境变量，因此额外扫描 NODE_PATH 里的每个目录 —— 开发机上 pg 往往装在别处
 * （例如统一的工作区 node_modules），而 CI 里根本没有 PG，需要能干净地 SKIP。
 */
function loadPg() {
  const local = createRequire(import.meta.url);
  try {
    return local("pg");
  } catch {
    // 落到 NODE_PATH 兜底
  }
  for (const dir of (process.env.NODE_PATH ?? "").split(delimiter).filter(Boolean)) {
    try {
      // 基准设为该目录下的虚拟文件：node 会依次尝试 <dir>/node_modules/pg、
      // 其父级的 node_modules/pg ……，故 dir 指向 node_modules 或其父目录都能命中。
      const scoped = createRequire(join(dir, "__pg_probe__.js"));
      return scoped("pg");
    } catch {
      // 继续尝试下一个目录
    }
  }
  return null;
}

const argOnly = process.argv.find((a) => a.startsWith("--only="));
const only = argOnly ? argOnly.slice("--only=".length).split(",") : null;
const shouldRun = (name) => !only || only.includes(name);

async function main() {
  const { sql, cjk, sep } = renderFunctionSql();
  const fixture = JSON.parse(readFileSync(FIXTURE, "utf8"));

  if (!Array.isArray(fixture.cases) || fixture.cases.length < MIN_CASES) {
    throw new Error(
      `fixture 用例数 ${fixture.cases?.length ?? 0} 少于下限 ${MIN_CASES}`,
    );
  }

  console.log(
    `[ngram] 提取字符类：CJK ${cjk.length} 字符、SEP ${sep.length} 字符`,
  );
  console.log(`[ngram] 建函数 SQL ${sql.length} 字节，占位符已全部替换`);
  console.log(`[ngram] fixture 用例 ${fixture.cases.length} 条`);

  const pg = loadPg();
  if (!pg) {
    console.log(
      "[ngram] SKIP：未找到 pg 驱动。本机可加 `NODE_PATH=<有 pg 的 node_modules>` 重跑；" +
        "CI 环境无生产 PG，跳过属预期，**不代表校验通过**。",
    );
    return 0;
  }

  const config = resolvePgConfig();
  if (!config) {
    console.log(
      "[ngram] SKIP：本环境无 PG 连接配置（`AXAGENT_NGRAM_PG_URL` 未设，" +
        "且 ~/.axagent/db_config.json 不存在）；**不代表校验通过**。",
    );
    return 0;
  }

  const { Client } = pg;
  const client = new Client(config);
  try {
    await client.connect();
  } catch (err) {
    console.log(
      `[ngram] SKIP：无法连接 PostgreSQL（${String(err.message).slice(0, 120)}）；` +
        "**不代表校验通过**。",
    );
    return 0;
  }

  const failures = [];
  let checks = 0;

  try {
    if (shouldRun("consistency")) {
      // 建/覆盖函数。与迁移执行的是同一份 SQL，故不会造成定义分叉。
      await client.query(sql);

      for (const testCase of fixture.cases) {
        const result = await client.query("SELECT ax_cjk_ngram($1) AS out", [
          testCase.input,
        ]);
        const actual = result.rows[0].out;
        checks += 1;
        if (actual !== testCase.expected) {
          failures.push(
            `  ── 规则: ${testCase.note}\n` +
              `     输入: ${JSON.stringify(testCase.input)}\n` +
              `     PG  : ${JSON.stringify(actual)}\n` +
              `     期望: ${JSON.stringify(testCase.expected)}`,
          );
        }
      }
      console.log(
        `[ngram] 函数一致性：${fixture.cases.length - failures.length}/${fixture.cases.length} 通过`,
      );

      // 反向自检：断言函数不是"恒返回空串"之类的退化实现。
      const probe = await client.query(
        "SELECT ax_cjk_ngram('向量索引') AS out",
      );
      if (!probe.rows[0].out) {
        failures.push("  ── ax_cjk_ngram('向量索引') 返回空，函数疑似退化实现");
      }
    }

    if (shouldRun("deployed")) {
      // 已落地检查：生成列表达式必须含 ax_cjk_ngram，且 GIN 索引存在。
      const expected = [
        ["notes", "tsv", "idx_notes_tsv"],
        ["memory_items", "content_tsv", "idx_memory_items_tsv"],
      ];
      const vecTables = await client.query(
        "SELECT table_name FROM information_schema.tables " +
          'WHERE table_schema = \'public\' AND table_name ~ \'^vec_.*_meta$\' ORDER BY table_name',
      );
      for (const row of vecTables.rows) {
        expected.push([
          row.table_name,
          "content_tsv",
          `idx_${row.table_name}_tsv`,
        ]);
      }

      for (const [table, column, index] of expected) {
        checks += 1;
        const col = await client.query(
          "SELECT is_generated, generation_expression FROM information_schema.columns " +
            "WHERE table_schema = 'public' AND table_name = $1 AND column_name = $2",
          [table, column],
        );
        if (col.rows.length === 0) {
          failures.push(
            `  ── ${table}.${column} 不存在；v227 迁移未执行或该表缺失`,
          );
          continue;
        }
        const expression = col.rows[0].generation_expression ?? "";
        if (!expression.includes("ax_cjk_ngram")) {
          failures.push(
            `  ── ${table}.${column} 的生成列未使用 ax_cjk_ngram（中文检索仍会失效）\n` +
              `     当前表达式: ${expression.slice(0, 160)}`,
          );
        }
        const idx = await client.query(
          "SELECT 1 FROM pg_indexes WHERE schemaname = 'public' AND tablename = $1 AND indexname = $2",
          [table, index],
        );
        if (idx.rows.length === 0) {
          failures.push(
            `  ── ${table} 缺少 GIN 索引 ${index}；查询将退化为全表扫描且不报错`,
          );
        }
      }
      console.log(
        `[ngram] 已落地检查：覆盖 ${expected.length} 张表（notes / memory_items / vec_*_meta）`,
      );
    }
  } finally {
    await client.end();
  }

  if (failures.length > 0) {
    console.error(`\n[ngram] FAIL：${checks} 项检查中 ${failures.length} 项失败\n`);
    console.error(failures.join("\n"));
    console.error(
      "\n提示：若失败项是「生成列未使用 ax_cjk_ngram」或「索引缺失」，" +
        "说明 v227 迁移尚未在本库执行。",
    );
    return 1;
  }

  console.log(`\n[ngram] PASS：${checks} 项检查全部通过`);
  return 0;
}

main()
  .then((code) => process.exit(code))
  .catch((err) => {
    console.error(`[ngram] ERROR: ${err.message}`);
    process.exit(2);
  });
