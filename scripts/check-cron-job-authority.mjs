#!/usr/bin/env node
/**
 * 定时任务权威门（A1，`PLAN-chain-closure-remediation.md` §2）。
 *
 * ## 为什么要有这道门
 * 内置任务播种的语义是「**存在即跳过，不改**」（`stock_automation_cron_seed.rs` 文件头写明，
 * 为的是不覆盖用户在面板改过的时刻/阈值）。这条保护是对的，但它有一个从未被记录的后果：
 * **代码不再播种的行会永远留在库里继续按天跑**。全仓没有任何一层回收 `cron_jobs` 里
 * 多出来的 active 行，也没有任何一处报告它。
 *
 * 现网实证（2026-10-20）：库里有 15 行，代码只播 11 条。多出的两行来自一个 2026-09-23 的
 * 手工插库脚本（`output/backup-20260923/add-cron.mjs`，`INSERT … WHERE NOT EXISTS` by id），
 * 在全仓 git 历史里 `--all -S` 零命中 —— 也就是说**只看代码永远发现不了它们**。
 * 其中 `batchref-due-daily` 与内置的 `reflection-due-daily` 同 `task_type=batch-reflection`、
 * 同 `0 18 * * *`，实测 10-19 两条在 18:00:34 与 18:01:13 各跑一趟 ⇒ 同一批到期 pending 被
 * 两趟消费，各自的配额（200 / 20）互不知情。
 *
 * ## 规则
 *  · R1 权威侧自证：现场从 seed 源文抽 `pub const *_JOB_ID: &str = "…"`，条数必须 ≥ 11，
 *     且 id 互不相同。条数掉下来说明**解析本身坏了**（那时白名单为空 ⇒ 后面全恒绿）。
 *  · R2 内置行必须在库里：白名单里的 id 若库里没有对应行 ⇒ 该链根本没被自动化（报缺）。
 *  · R3 重复调度：股票域内 `status=active` 的行按 `(task_type, schedule)` 分组，
 *     组内多于一条 ⇒ 逐组报出，并点明其中哪些 id 不在白名单。
 *  · R4 孤儿来源：股票域内 id 不在白名单的行 ⇒ 报「代码不再播种但仍会执行」；
 *     其中 `name === id` 的那些再标一层「非内置播种形态」（内置的 name 是中文，
 *     面板/手工建的 name 就是 id —— 本次据此一眼定位到手工脚本）。
 *  · 扫描面**按域限定**：股票域 = 内置行的 task_type 集合 ∪ `validate-decisions`
 *     （后者有 executor 分支、面板可建、但无播种，2026-10-20 实测在库里活着）。
 *     不加这条并集，`opc-demand-discovery-*` 那类**别处合法播种**的行会被误报。
 *
 * ## 自证（`--selftest`）
 * 负控拆在真正决定该断言的那道守卫上，每个破坏样本只让它自己那一格红：
 * M1 白名单条数塌陷（解析退化）/ M2 内置行缺失 / M3 同类型同刻度第二行 /
 * M4 非白名单孤儿行 / M5 把 M3 的行改成 `paused` **不得**红（闸门在 status 上）/
 * M6 非股票域（opc）的多行**不得**红（扫描面按域限定）。
 *
 * ## 现网读数是可选项
 * CI 上没有本机 PG ⇒ 只跑 R1；R2–R4 需要库读数，取不到时**明印「未检查」**，
 * 绝不把「没连上库」报成「库里没问题」。加 `--require-live` 可强制失败（本地收口用）。
 *
 * 用法：node scripts/check-cron-job-authority.mjs [--selftest] [--require-live] [--dump]
 * 退出码：0 = 通过（或读数缺席但已如实声明）；1 = 有违规 / 权威侧解析失败；2 = 参数错误。
 */
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..");
const SEED = "src-tauri/src/init/stock_automation_cron_seed.rs";

/** 内置 id 条数下限：实测 12 条（2026-10-20 A-1 加「漏检补分析」前为 11），少一条就是解析退化或播种被删。 */
const MIN_SEED_IDS = 13;
/** 见头部「扫描面按域限定」：这些 task_type 无播种但属股票域，必须纳入检查。 */
const EXTRA_STOCK_TASK_TYPES = ["validate-decisions"];

// ── 权威侧：从 seed 源文现场抽内置 job id ──────────────────────────────────

/** 抽 `pub const XXX_JOB_ID: &str = "value";` 的 value。不用正则：按行 startsWith + 引号切片。 */
function seedJobIds(seedSrc) {
  const prefix = "pub const ";
  const out = [];
  for (const line of seedSrc.split("\n")) {
    const t = line.trim();
    if (!t.startsWith(prefix) || !t.includes("_JOB_ID")) continue;
    const qi = t.indexOf('&str = "');
    if (qi < 0) continue;
    const rest = t.slice(qi + '&str = "'.length);
    const end = rest.indexOf('"');
    if (end <= 0) continue;
    out.push(rest.slice(0, end));
  }
  return out;
}

/** R1：白名单自身必须站得住（条数 + 唯一性）。 */
function checkAuthority(ids) {
  const bad = [];
  if (ids.length < MIN_SEED_IDS) {
    bad.push(
      `R1 权威侧解析退化：从 ${SEED} 只抽到 ${ids.length} 个 *_JOB_ID（下限 ${MIN_SEED_IDS}）` +
        ` ⇒ 白名单不可信，R2–R4 会因此恒绿。${ids.length ? `抽到：${ids.join(", ")}` : "一条都没抽到。"}`,
    );
    return bad;
  }
  const seen = new Set();
  for (const id of ids) {
    if (seen.has(id)) bad.push(`R1 内置 job id 重复：${id}`);
    seen.add(id);
  }
  return bad;
}

// ── 现网侧：对 cron_jobs 行集做判据（纯函数，负控与生产同一道守卫）──────────

/**
 * @param rows 每行 `{id, taskType, schedule, name, status}`
 * @param seedIds 权威白名单
 * @returns 违规文本数组（空 = 通过）
 */
export function checkCronAuthority(rows, seedIds) {
  const bad = [];
  const seed = new Set(seedIds);
  const active = rows.filter((r) => r.status === "active");

  // 股票域 = 内置行实际用到的 task_type ∪ 显式并集（见头部）
  const builtIn = active.filter((r) => seed.has(r.id));
  const domain = new Set(builtIn.map((r) => r.taskType).filter(Boolean));
  for (const t of EXTRA_STOCK_TASK_TYPES) domain.add(t);

  // R2 内置行必须在库里（按 id 比，不看 status：暂停是用户意图，缺失才是断链）
  const present = new Set(rows.map((r) => r.id));
  for (const id of seed) {
    if (!present.has(id)) bad.push(`R2 内置任务 ${id} 在库里没有行 ⇒ 该链未被自动化`);
  }

  const stock = active.filter((r) => domain.has(r.taskType));

  // R3 同 (task_type, schedule) 多条 active
  const groups = new Map();
  for (const r of stock) {
    const k = `${r.taskType}@${r.schedule}`;
    if (!groups.has(k)) groups.set(k, []);
    groups.get(k).push(r);
  }
  for (const [k, list] of [...groups.entries()].sort()) {
    if (list.length < 2) continue;
    const extra = list.filter((r) => !seed.has(r.id)).map((r) => r.id);
    bad.push(
      `R3 重复调度：${k} 有 ${list.length} 条 active（${list.map((r) => r.id).join(", ")}）` +
        (extra.length ? ` ⇒ 其中不在白名单：${extra.join(", ")}` : " ⇒ 全部由播种产生，需查播种"),
    );
  }

  // R4 孤儿来源
  for (const r of stock.filter((x) => !seed.has(x.id)).sort((a, b) => (a.id < b.id ? -1 : 1))) {
    const shape = r.name === r.id ? "，name===id ⇒ 非内置播种形态（面板/手工插入）" : "";
    bad.push(`R4 库里存在但代码不再播种的 active 行：${r.id}（task_type=${r.taskType}${shape}）`);
  }
  return bad;
}

// ── 取数 ────────────────────────────────────────────────────────────────────

function sql(q) {
  const r = spawnSync("node", ["scripts/pg-connect.mjs", "sql", q], { cwd: ROOT, encoding: "utf8" });
  if (r.status !== 0) {
    throw new Error(`取数失败: ${(r.stdout || r.stderr || "").trim().slice(0, 160)}`);
  }
  // ⚠ Windows 上 psql 回 CRLF：不剥 `\r` 会让表头带尾字符 ⇒ 取值全 undefined，
  //   读数看着像「表是空的」，其实是取数姿势坏了（照抄 check-universe-coverage.mjs 的说明）。
  const lines = r.stdout.replace(/\r/g, "").trim().split("\n").filter(Boolean);
  if (lines.length < 2) return [];
  const head = lines[0].split("\t");
  return lines.slice(1).map((l) => {
    const cells = l.split("\t");
    const o = {};
    head.forEach((h, i) => {
      o[h] = (cells[i] ?? "").trim();
    });
    return {
      id: o.id,
      taskType: o.task_type,
      schedule: o.schedule,
      name: o.name,
      status: o.status,
    };
  });
}

const LIVE_QUERY =
  "select id, data::jsonb->>'task_type' as task_type, data::jsonb->>'schedule' as schedule, " +
  "data::jsonb->>'name' as name, data::jsonb->>'status' as status from cron_jobs order by id";

// ── 自证 ────────────────────────────────────────────────────────────────────

function fakeRows(specs) {
  return specs.map(([id, taskType, schedule, name, status]) => ({
    id,
    taskType,
    schedule,
    name: name ?? id,
    status: status ?? "active",
  }));
}

function selftest() {
  const seed = ["stock-reco-daily", "pool-scan-daily", "reflection-due-daily", "decision-backtest-daily"];
  const base = fakeRows([
    ["stock-reco-daily", "stock-recommendation", "0 0 * * *", "智能荐股（内置）"],
    ["pool-scan-daily", "pool-scan", "0 2 * * *", "候选池分析（内置）"],
    ["reflection-due-daily", "batch-reflection", "0 18 * * *", "到期反思（内置）"],
    ["decision-backtest-daily", "decision-backtest", "0 7 * * *", "决策回测（内置）"],
  ]);
  const cases = [
    {
      name: "P0 齐备态不得红",
      got: () => checkCronAuthority(base, seed),
      want: (v) => v.length === 0,
    },
    {
      name: "M1 白名单条数塌陷必须红",
      got: () => checkAuthority(["only-one"]),
      want: (v) => v.some((x) => x.startsWith("R1")),
    },
    {
      name: "M2 内置行缺失必须红",
      got: () => checkCronAuthority(base.filter((r) => r.id !== "pool-scan-daily"), seed),
      want: (v) => v.some((x) => x.startsWith("R2") && x.includes("pool-scan-daily")) && v.length === 1,
    },
    {
      name: "M3 同类型同刻度第二行必须红，且点名不在白名单的那个",
      got: () =>
        checkCronAuthority(
          base.concat(fakeRows([["batchref-due-daily", "batch-reflection", "0 18 * * *"]])),
          seed,
        ),
      want: (v) =>
        v.some((x) => x.startsWith("R3") && x.includes("batchref-due-daily")) &&
        v.some((x) => x.startsWith("R4") && x.includes("batchref-due-daily")) &&
        v.length === 2,
    },
    {
      name: "M4 孤儿行（独有 task_type）只红 R4，不冒充重复调度",
      got: () =>
        checkCronAuthority(
          base.concat(fakeRows([["vldec-daily", "validate-decisions", "0 6 * * *"]])),
          seed,
        ),
      want: (v) => v.length === 1 && v[0].startsWith("R4") && v[0].includes("vldec-daily"),
    },
    {
      name: "M5 把 M3 的重复行改成 paused 不得红（闸门在 status 上）",
      got: () =>
        checkCronAuthority(
          base.concat(fakeRows([["batchref-due-daily", "batch-reflection", "0 18 * * *", "x", "paused"]])),
          seed,
        ),
      want: (v) => v.length === 0,
    },
    {
      name: "M6 非股票域的多行不得红（扫描面按域限定）",
      got: () =>
        checkCronAuthority(
          base.concat(
            fakeRows([
              ["opc-a", "opc-demand-discovery", "0 2 * * *"],
              ["opc-b", "opc-demand-discovery", "0 2 * * *"],
            ]),
          ),
          seed,
        ),
      want: (v) => v.length === 0,
    },
    {
      name: "M7 name===id 形态指纹必须被点出来",
      got: () =>
        checkCronAuthority(
          base.concat(fakeRows([["vldec-daily", "validate-decisions", "0 6 * * *"]])),
          seed,
        ),
      want: (v) => v.some((x) => x.includes("非内置播种形态")),
    },
  ];

  let bad = 0;
  for (const c of cases) {
    const v = c.got();
    if (!c.want(v)) {
      bad += 1;
      console.error(`❌ ${c.name} —— 实得 ${JSON.stringify(v)}`);
    } else {
      console.log(`✅ ${c.name}`);
    }
  }
  if (bad) {
    console.error(`自检未通过：${bad} 例`);
    process.exit(1);
  }
  console.log(`自检通过（${cases.length} 例，判据不空转）`);
}

// ── 主流程 ──────────────────────────────────────────────────────────────────

function main() {
  const args = process.argv.slice(2);
  if (args.includes("--selftest")) {
    selftest();
    return;
  }
  const src = fs.readFileSync(path.join(ROOT, SEED), "utf8");
  const ids = seedJobIds(src);
  const violations = checkAuthority(ids);
  console.log(`权威源 ${SEED} → 内置 job id ${ids.length} 条：${ids.join(", ")}`);

  if (args.includes("--dump")) console.log(JSON.stringify(ids, null, 2));

  let rows = null;
  try {
    rows = sql(LIVE_QUERY);
  } catch (e) {
    console.log(`⚠ 现网读数未取（${e.message.slice(0, 120)}）⇒ R2–R4 **未检查**；CI 上属预期，本地收口请加 --require-live`);
    if (args.includes("--require-live")) {
      console.error("❌ --require-live 下连不上库即失败");
      process.exit(1);
    }
  }
  if (rows) {
    console.log(`现网 cron_jobs：${rows.length} 行，其中 active ${rows.filter((r) => r.status === "active").length}`);
    console.log(
      `股票域扫描面 task_type：${[...new Set(rows.filter((r) => ids.includes(r.id)).map((r) => r.taskType))].join(", ")} ∪ ${EXTRA_STOCK_TASK_TYPES.join(",")}`,
    );
    violations.push(...checkCronAuthority(rows, ids));
  }

  if (violations.length) {
    console.error(`❌ 违规 ${violations.length} 条：`);
    for (const v of violations) console.error(`  · ${v}`);
    console.error("门绿 ≠ 待修面已清：R3/R4 的处置动作是**在任务中心删除**，本门不代删（那是用户建的任务）。");
    process.exit(1);
  }
  console.log("✅ 通过");
}

main();
