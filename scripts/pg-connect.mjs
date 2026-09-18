#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
//
// 连接本地 PostgreSQL 的**唯一**正确入口 —— 口令从 ~/.axagent 解密，绝不烧进源码。
//
// ## 为什么需要它（2026-09-13）
//
// 项目的 PG 口令以 AES-256-GCM 加密存放在 `~/.axagent/db_config.json`
// （密钥在同目录的 `master.key`）。这个做法是对的，但**缺一个正式的读取入口** ——
// 于是每需要连一次库就有人临时写一个脚本，而临时脚本最容易把明文口令写死：
// 本仓库曾有一个 `scripts/tmp_decrypt_pg.py`，第 3 行就是 `password="..."`，
// 且 `scripts/` 不在 `.gitignore` 里 ⇒ 明文口令有进版本库的风险（已删除）。
//
// 本脚本提供的正是那个缺失的入口：**解密逻辑（不含任何密钥）在仓库里，
// 密钥与密文留在 `~/.axagent`**，两边不交叉。
//
// ## 用法
//
//   node scripts/pg-connect.mjs url                       # 打印连接串（默认库 axinvest）
//   node scripts/pg-connect.mjs url --redact              # 同上但口令脱敏（看形态用）
//   node scripts/pg-connect.mjs url --db=axagent_pg_migtest
//   node scripts/pg-connect.mjs sql "SELECT 1"            # 执行 SQL（需要 npm i -D pg）
//
// 典型配合（把连接串喂给真机 PG 测试）：
//
//   AXAGENT_TEST_PG_URL="$(node scripts/pg-connect.mjs url)" \
//     cargo test -p axagent-dao --test pg_cjk_fts \
//       --manifest-path src-tauri/Cargo.toml
//
// ⚠ 2026-09-16：上面原示例用的是 `--test pg_migrations`。该文件已随 74 个版本化迁移
// 一起**退休**（迁移清单已清空 ⇒ 它断言的「跑全量迁移建全新库」不再有对象），
// 换成仍在的 `pg_cjk_fts`。其余 env-gated 目标见 `search/tests/pg_integration.rs`。
//
// ## 约定
//
// - 连接串打到 **stdout**，所有诊断信息走 **stderr** ⇒ `$(...)` 捕获时不会混入噪音；
// - `url` 默认输出**完整**连接串（它就是干这个的），但会在 stderr 提醒「该串含口令」；
//   只想看形态时用 `--redact`。

import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const CONFIG_DIR = path.join(os.homedir(), ".axagent");

function fail(msg) {
  process.stderr.write(`pg-connect: ${msg}\n`);
  process.exit(1);
}

/** 读 `master.key` + `db_config.json`，解出明文口令。 */
function readCredentials() {
  const keyPath = path.join(CONFIG_DIR, "master.key");
  const cfgPath = path.join(CONFIG_DIR, "db_config.json");
  if (!fs.existsSync(keyPath) || !fs.existsSync(cfgPath)) {
    fail(
      `未找到 ${keyPath} 或 ${cfgPath} —— ` +
        `请先在本机启动过一次应用，让初始化流程写入数据库配置。`,
    );
  }
  const key = fs.readFileSync(keyPath);
  const cfg = JSON.parse(fs.readFileSync(cfgPath, "utf8"));
  if (!cfg.pg_password_enc) {
    fail(`${cfgPath} 里没有 pg_password_enc —— 当前后端可能不是 PostgreSQL。`);
  }
  // 密文布局：iv(12) || ciphertext || authTag(16)，整体 base64
  const buf = Buffer.from(cfg.pg_password_enc, "base64");
  const decipher = crypto.createDecipheriv("aes-256-gcm", key, buf.subarray(0, 12));
  decipher.setAuthTag(buf.subarray(buf.length - 16));
  const body = buf.subarray(12, buf.length - 16);
  const password = Buffer.concat([decipher.update(body), decipher.final()]).toString("utf8");
  return { cfg, password };
}

/**
 * 组装连接串。
 *
 * 口令与用户名做 percent-encode —— 含 `@` / `:` / `/` / `#` 的口令不转义会被
 * 解析成别的东西（如 `p@ss` 会被当成 userinfo 分隔）。这是纯字符串处理，
 * 不依赖任何数据库驱动。
 */
function buildUrl(cfg, password, db) {
  const user = encodeURIComponent(cfg.pg_user);
  const pass = encodeURIComponent(password);
  return `postgres://${user}:${pass}@${cfg.pg_host}:${cfg.pg_port}/${db}`;
}

async function main() {
  const [cmd, ...rest] = process.argv.slice(2);
  const positional = rest.filter((a) => !a.startsWith("--"));
  const dbArg = rest.find((a) => a.startsWith("--db="));
  const redact = rest.includes("--redact");

  if (!cmd || cmd === "-h" || cmd === "--help") {
    process.stdout.write(
      "用法:\n" +
        "  node scripts/pg-connect.mjs url [--db=<库名>] [--redact]  打印连接串（默认 axinvest）\n" +
        '  node scripts/pg-connect.mjs sql "<SQL>"                   执行 SQL（需要 npm i -D pg）\n',
    );
    return;
  }

  const { cfg, password } = readCredentials();
  const db = dbArg ? dbArg.slice("--db=".length) : cfg.pg_database || "axinvest";

  if (cmd === "url") {
    const url = buildUrl(cfg, password, db);
    process.stdout.write((redact ? url.replace(/:[^:@]*@/, ":***@") : url) + "\n");
    if (!redact) {
      process.stderr.write(
        "pg-connect: 上面的连接串含明文口令 —— 只用于喂给本地命令，勿粘贴到公开处。" +
          "（只想看形态请加 --redact）\n",
      );
    }
    return;
  }

  if (cmd === "sql") {
    const sql = positional[0];
    if (!sql) fail('sql 子命令需要 SQL 文本，例如: node scripts/pg-connect.mjs sql "SELECT 1"');
    let pg;
    try {
      // 刻意用动态 import：`pg` 不是项目依赖，没装也不该让 url 子命令失效
      pg = await import("pg");
    } catch {
      fail("未安装 `pg` 包 —— 执行 SQL 需要它：npm i -D pg；或改用 url 子命令把连接串喂给 psql。");
    }
    const client = new pg.Client({ connectionString: buildUrl(cfg, password, db) });
    await client.connect();
    try {
      const res = await client.query(sql);
      process.stdout.write(JSON.stringify(res.rows, null, 2) + "\n");
    } finally {
      await client.end();
    }
    return;
  }

  fail(`未知子命令 \`${cmd}\`（支持 url / sql）`);
}

main().catch((e) => fail(e.message));
