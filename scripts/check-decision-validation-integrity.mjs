#!/usr/bin/env node
/**
 * R-3i / R-3j 门禁：决策验证表的**引用完整性、窗口口径与业务唯一键**
 *
 * 守三件事（都是 2026-10-26 实测出的真缺陷，不是假想）：
 *
 * ① **引用完整性**（R-3i）
 *    `decision_validations.pick_id` 指向 `reco_picks.id`。
 *    历史实现「同业务键覆盖」是**先删旧行再插新行** ⇒ 旧行 id 消失
 *    ⇒ 引用它的验证行全部悬空（实测 448 行 / 20.2%）。
 *    孤儿行读数系统性偏乐观（net −0.567 vs 关联行 −1.614）⇒ 混算虚高。
 *    ⇒ 门禁要求：**孤儿行数 = 0**。
 *
 * ② **窗口口径**（R-3j）
 *    `decision_validations` 是**多窗口笛卡尔积**表：每个 pick 跑 `Period::validation_windows()`
 *    = [2, 5, 28, 90] 全部窗口 ⇒ `period` 与 `t_plus_n` **必然大量错配**
 *    （实测 1262 行 / 57%）。这不是缺陷，是设计。
 *    ⚠ 但**按档统计必须只用匹配窗口**（ultra_short↔2 / short↔5 / mid↔28 / long↔90），
 *      否则宽口径 t=−4.82 vs 严口径 t=−0.92，读数差一个量级。
 *    ⇒ 门禁要求：**匹配窗口的行数 > 0**（否则说明窗口集与档位对不上，统计会恒空）。
 *
 * ③ **业务唯一键 + 幂等键**（2026-10-26 加）
 *    · `reco_picks` 业务键 `(as_of_date, purpose, period, stock_code, style)`
 *      已加库级唯一索引 `uq_reco_picks_business_key`（NULLS NOT DISTINCT）。
 *      ⇒ 门禁用**读侧**再核对一次：索引在 ≠ 键的定义对。
 *    · `decision_validations` 写入幂等键必须是 `(pick_id, t_plus_n, generated_at)`
 *      —— **必须含决策日**。旧键 `(pick_id, t_plus_n)` 过粗：同票同窗口但
 *      **不同决策日**的样本会被判成重复而跳过（实测 402 对这样的样本；
 *      同决策日的纯重复只有 50 对）。
 *
 * 退出码：0 = PASS；1 = FAIL
 */
import { execFileSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, '..');

// ⚠ `pg-connect.mjs sql` 会**过滤以 `--` 开头的参数** ⇒ SQL 开头必须有前导空格。
// ⚠ 中文字面量会触发 psql 的 UTF8 编码错误 ⇒ 用 chr(0xXXXX) 拼。
const integritySql = `
WITH j AS (
  SELECT
    v.id, v.pick_id, v.period, v.t_plus_n, v.final_return_pct, v.generated_at,
    (p.id IS NOT NULL) AS linked
  FROM decision_validations v
  LEFT JOIN reco_picks p ON p.id = v.pick_id
)
SELECT
  count(*) AS total,
  count(*) FILTER (WHERE NOT linked) AS orphan_rows,
  count(*) FILTER (WHERE linked) AS linked_rows,
  count(*) FILTER (WHERE linked AND (
       (period = 'ultrashort' AND t_plus_n = 2)
    OR (period = 'short'     AND t_plus_n = 5)
    OR (period = 'mid'       AND t_plus_n = 28)
    OR (period = 'long'      AND t_plus_n = 90)
  )) AS matched_window_rows,
  count(*) FILTER (WHERE linked AND NOT (
       (period = 'ultrashort' AND t_plus_n = 2)
    OR (period = 'short'     AND t_plus_n = 5)
    OR (period = 'mid'       AND t_plus_n = 28)
    OR (period = 'long'      AND t_plus_n = 90)
  )) AS off_window_rows,
  round(avg(final_return_pct - 0.25) FILTER (WHERE linked)::numeric, 3) AS net_linked,
  round(avg(final_return_pct - 0.25) FILTER (WHERE NOT linked)::numeric, 3) AS net_orphan
FROM j
`;

const pickDupSql = `
SELECT count(*) AS dup_groups, coalesce(sum(n), 0) AS dup_rows
FROM (
  SELECT as_of_date, purpose, period, stock_code, style, count(*) AS n
  FROM reco_picks
  GROUP BY 1,2,3,4,5
  HAVING count(*) > 1
) t
`;

const dvDupSql = `
SELECT count(*) AS dup_groups
FROM (
  SELECT pick_id, t_plus_n, generated_at
  FROM decision_validations
  GROUP BY 1,2,3
  HAVING count(*) > 1
) t
`;

function query(sql) {
  const out = execFileSync('node', ['scripts/pg-connect.mjs', 'sql', ' ' + sql], {
    encoding: 'utf8',
    cwd: ROOT,
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  // ⚠ psql 输出用 **\r\n** 行分隔 ⇒ 直接 split('\n') 会在表头尾部留 '\r'
  //   ⇒ 列名取不到（表现：`row.dup_groups === undefined`，而 SQL 本身返回 0）。
  //   症状与「判据没跑到」同形 ⇒ 一律先归一化行尾。
  const lines = out.replace(/\r\n/g, '\n').trim().split('\n');
  if (lines.length < 2) return null;
  const hdr = lines[0].split('\t');
  const row = {};
  hdr.forEach((h, i) => (row[h] = lines[1].split('\t')[i]));
  return row;
}

let row;
let dupRow;
let dvDupRow;
try {
  row = query(integritySql);
  dupRow = query(pickDupSql);
  dvDupRow = query(dvDupSql);
} catch (e) {
  console.error('✖ 无法连接数据库 —— 本门禁**未执行**，不等于通过。');
  console.error(String(e.stdout || e.message));
  process.exit(1);
}

if (!row || !dupRow || !dvDupRow) {
  console.error('✖ 查询无返回行 —— 判据自身失效，不等于通过。');
  process.exit(1);
}

const total = Number(row.total);
const orphan = Number(row.orphan_rows);
const matched = Number(row.matched_window_rows);
const offWin = Number(row.off_window_rows);

console.log('— 决策验证表引用完整性 / 窗口口径 —');
console.log(`  验证行总数        ${total}`);
console.log(`  关联 reco_picks   ${row.linked_rows}`);
console.log(`  孤儿行            ${orphan}   ${orphan === 0 ? '✔' : '✖ 应为 0'}`);
console.log(`  匹配窗口行        ${matched}   ${matched > 0 ? '✔' : '✖ 为 0 ⇒ 按档统计会恒空'}`);
console.log(`  非匹配窗口行      ${offWin}   （设计行为：多窗口笛卡尔积，统计时须排除）`);
if (row.net_orphan !== '' && row.net_orphan !== undefined) {
  console.log(`  净均值（关联/孤儿） ${row.net_linked} / ${row.net_orphan}`);
}

console.log('— 业务唯一键 / 幂等键 —');
console.log(
  `  reco_picks 重复组  ${dupRow.dup_groups}   ${dupRow.dup_groups === '0' ? '✔' : '✖ 业务键重复'}`,
);
console.log(
  `  dv 幂等键重复组    ${dvDupRow.dup_groups}   ${dvDupRow.dup_groups === '0' ? '✔' : '✖ (pick_id,t_plus_n,generated_at) 重复'}`,
);

let failed = false;
if (total === 0) {
  console.error('\n✖ 验证表为空 —— 判据无对象，不等于通过。');
  failed = true;
}
if (orphan > 0) {
  console.error(`\n✖ 孤儿行 ${orphan} 条：pick_id 指向已删除的 reco_picks 行。`);
  console.error('  ⇒ 这些行的读数系统性偏乐观，混算会虚高。修法见裁决 §R-3i。');
  failed = true;
}
if (matched === 0) {
  console.error('\n✖ 匹配窗口行数为 0：按档统计（period ↔ t_plus_n）会恒空。');
  failed = true;
}
if (dupRow.dup_groups !== '0') {
  console.error(`\n✖ reco_picks 业务键重复 ${dupRow.dup_groups} 组。`);
  console.error('  ⇒ 库级唯一索引 uq_reco_picks_business_key 应已阻止它；请核对索引是否存在。');
  failed = true;
}
if (dvDupRow.dup_groups !== '0') {
  console.error(`\n✖ decision_validations 幂等键重复 ${dvDupRow.dup_groups} 组。`);
  console.error(
    '  ⇒ 写入侧幂等键必须是 (pick_id, t_plus_n, generated_at)，缺 generated_at 会吞掉不同决策日的样本。',
  );
  failed = true;
}

if (failed) process.exit(1);
console.log('\n✔ PASS：引用完整（孤儿 0）+ 匹配窗口非空 + 业务键/幂等键零重复');
