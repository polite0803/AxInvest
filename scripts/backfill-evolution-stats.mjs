#!/usr/bin/env node
// 回填 evolution_execution_stats —— 从「记录同一事实的独立数据源」重建 (conversation_id, tool_id) 的成功/失败计数。
//
// ⚠ 设计前提（必读，否则会把这张表写脏）：
//
// 1. 目标表 `evolution_execution_stats` **只服务「进化产物」**，不是所有工具。
//    写入方 `EvolutionFeedbackSinkImpl::record` 只被 `GeneratedToolAdapter` 调用，
//    而该 adapter 的生产实例只来自 `evolution_engine` 的进化工具生成链。
//    ⇒ 回填**必须先与 `generated_tools`（source_info.source = 'runtime_evolution'）取交集**，
//      直接把所有工具塞进去是口径错误（会让贝叶斯证据看到不存在的产物）。
//
// 2. 源表选择：`tool_call_logs` 是库内**唯一**同时含
//    `conversation_id` + `tool_name` + `success` 的表。
//    已排除的候选（2026-09-17 真库实测，见 AUDIT-evolution-stats-backfill-2026-09-17.md）：
//      - `tool_executions`：0 行，且 status 是 5 值枚举、与 success 布尔非一一对应
//      - `audit_log`：13 行但 conversation_id **全为 null**，且只有内置工具
//      - `trajectory_steps`：tool_results 全 null、无 success 字段
//      - `capability_stats`：只有 workflow 维度（tool 维度 0 行）
//      - `cron_job_history` / `opc_rl_experiences` / `trajectory_learned_patterns`：粒度不符
//
// 3. ⚠ **PG 方言约束**（2026-09-17 修复的 42702 同源坑）：
//    `ON CONFLICT ... DO UPDATE SET` 的命名空间里同时有目标表与 `excluded` ⇒
//    任何**未限定**的列名都歧义。本脚本的 SET 表达式全部写成 `EXCLUDED.<col>`
//    （单独引用 excluded 是合法的），**不得**改写成裸列名。
//
// 4. 默认 **dry-run**：只打印将写入的行，绝不写库。加 `--apply` 才执行。
//    回填是**绝对赋值**（非累加），故可重复执行、幂等。
//
// 用法：
//   PG_URL=... node scripts/backfill-evolution-stats.mjs            # dry-run
//   PG_URL=... node scripts/backfill-evolution-stats.mjs --apply    # 真正写入

import pg from "pg";

const APPLY = process.argv.includes("--apply");
const url = process.env.PG_URL;
if (!url) {
  console.error("PG_URL 未设置（用 scripts/pg-connect.mjs url 生成后经 env 传入，勿走 argv）");
  process.exit(2);
}

const client = new pg.Client({ connectionString: url });
await client.connect();

// ── 1. 候选源规模（先摆事实，再决定做什么）──
const sources = {};
for (const [label, sql] of [
  ["tool_call_logs", "SELECT count(*)::int AS n FROM tool_call_logs"],
  ["generated_tools", "SELECT count(*)::int AS n FROM generated_tools"],
  ["evolution_execution_stats", "SELECT count(*)::int AS n FROM evolution_execution_stats"],
]) {
  const r = await client.query(sql);
  sources[label] = r.rows[0].n;
}
console.log("=== 源规模 ===");
console.table(sources);

if (sources.tool_call_logs === 0) {
  console.log("\n结论：源表 tool_call_logs 为 0 行 ⇒ **无可回填数据**（不会写入任何行）。");
  console.log("这不是「回填失败」，而是「没有对象」：库内不存在记录过这些工具执行的事实源。");
  await client.end();
  process.exit(0);
}

// ── 2. 聚合：只取「进化产物」工具（与 generated_tools 取交集）──
const aggSql = `
  SELECT l.conversation_id,
         l.tool_name,
         count(*)::int                        AS usage_count,
         sum(l.success)::int                  AS successes,
         (count(*) - sum(l.success))::int     AS failures
    FROM tool_call_logs l
   WHERE EXISTS (
           SELECT 1 FROM generated_tools g
            WHERE g.tool_name = l.tool_name
              AND g.source_info::jsonb ->> 'source' = 'runtime_evolution'
         )
   GROUP BY l.conversation_id, l.tool_name
   ORDER BY l.conversation_id, l.tool_name
`;
const rows = (await client.query(aggSql)).rows;

console.log(`\n=== 聚合结果：${rows.length} 个 (conversation_id, tool_id) 组合 ===`);
console.table(rows);

if (rows.length === 0) {
  console.log("\n结论：源表有数据，但没有任何一行属于「进化产物」⇒ 仍无可回填数据。");
  await client.end();
  process.exit(0);
}

if (!APPLY) {
  console.log("\n[dry-run] 未写入任何数据。加 --apply 执行。");
  await client.end();
  process.exit(0);
}

// ── 3. 写入（绝对赋值 ⇒ 幂等）──
// ⚠ SET 里只允许 `EXCLUDED.<col>` 或 `"表"."列"`；裸列名在 PG 上必报 42702。
let written = 0;
await client.query("BEGIN");
try {
  for (const r of rows) {
    await client.query(
      `INSERT INTO evolution_execution_stats (conversation_id, tool_id, usage_count, successes, failures)
       VALUES ($1, $2, $3, $4, $5)
       ON CONFLICT (conversation_id, tool_id) DO UPDATE
          SET usage_count = EXCLUDED.usage_count,
              successes   = EXCLUDED.successes,
              failures    = EXCLUDED.failures`,
      [r.conversation_id, r.tool_name, r.usage_count, r.successes, r.failures],
    );
    written += 1;
  }
  await client.query("COMMIT");
} catch (e) {
  await client.query("ROLLBACK");
  console.error(`\n写入失败已回滚：${e.message}`);
  await client.end();
  process.exit(1);
}

const after = await client.query("SELECT count(*)::int AS n FROM evolution_execution_stats");
console.log(`\n[apply] 已写入 ${written} 行；表内现有 ${after.rows[0].n} 行。`);
await client.end();
