# PLAN：live K 线缓存风暴 + t-risk 超时（301302「决策信息缺失」根因修复）

日期：2026-09-27 · 状态：C1 `7ee89e62a` / C2 `acea93820` / C3 `5c8cd4a94` / C4 `e93d1dd83` 完成；C5 已验证（14:21 运行 t-risk 5ms、portfolio-mgr 出决策、73/B）；D1+D2 `f27c040ce`（模板 v86）完成**但 v86 在生产烧穿 CodeNode 操作预算 ⇒ data-quality 节点失败**，见 D4（模板 v87）；剩 D3/D4 重启重跑核对

## 一、现象与取证结论

截图：301302 live 运行「决策信息缺失」，证据审计指认 `t-risk` 超时 → portfolio-mgr 级联跳过。

DB 取证（`workflow_executions.31bb09b7`，经 `scripts/pg-connect.mjs`）：

- `t-risk`（组合风险计算，compute_portfolio_risk）10:55:02 起 **3 次重试全部 30s 超时**（30009/31867/30009ms）；
- 同窗 `t-scoring-month` 超时 2 次后第 3 次 **208ms 成功**（计算本身极快，慢在取数/调度）；
- 同日 10:40 回放运行（as_of=09-09）t-risk 走每日快照 **142ms 完成**；
- 历史唯一另一次 t-risk 三连超时 = 09-26 14:39（push2his 全挂日）——同根因族。

本机实测（2026-09-27）：

| 端点 | 结果 |
|---|---|
| 东财 `push2his`（K线/资金流） | **RST 秒断**（0.11s，Defender，已知） |
| 腾讯 `fqkline/get`（**复权专用**） | **501 → waf.tencent.com（腾讯 WAF 拦截）** |
| 腾讯 `kline/kline`（不复权） | 200，0.2s，数据正常 |
| 东财 `datacenter-web` / `push2` 行情 | 200，秒回 |
| 通达信 `119.147.212.81:7709` | **TCP 黑洞**（8s+ 无 SYN-ACK；代码 5s connect timeout） |
| 东财 `RPTA_WEB_TRADE_CALENDAR` | **9501 报表已下线** |
| 腾讯日 K 最后日期 | **2026-09-24**（09-25 周五 = 中秋，实际休市） |

## 二、根因链（四层，逐层放大）

1. **日历错**：`calendar.rs` 硬编码表**缺 2026-09-25（中秋休市）**，且含已知错误的 09-21/22（实际开市，表内注释自证）。远程日历通道双重死：`init_holiday_calendar` **全仓无启动调用点** + 端点已下线（9501）⇒ 永远走错误兜底 ⇒ `latest_trading_day()` 误报 09-25。
2. **缓存判据被 1 击穿**：M-DS-1 新鲜性判据（`get_klines_with_adj`，lib.rs:2255 附近）要求 `缓存最后K线 ≥ latest_trading_day()` ⇒ `09-24 < 09-25` **恒过期** ⇒ live 下 K 线缓存 100% 失效，每次读都重拉网络。实证：L2 里 `klines:301302:daily:adj=fwd::live` 已于 10:56:08 写入（后台任务未被超时取消），attempt 3（10:56:09）读到仍判过期弃用。
3. **前复权通道双断**：重拉对不复权链无害（腾讯普通端点 0.2s），但 t-risk 要 **adj=fwd**：腾讯 fqkline 被 WAF 501 → 东财 push2his RST（em_get 3 重试 + 1s/2s 退避）→ xueqiu 无 token → mootdx TCP 5s 黑洞 → browser_eastmoney（还是 push2his）。一次冷取实测 ≈ **66s**（attempt 1 起算至缓存落盘）。
4. **节点预算**：30s < 66s ⇒ 3 次重试全部超时（每次都因判据 2 不吃缓存）⇒ t-risk 失败 ⇒ portfolio-mgr 级联跳过 ⇒ 无决策输出。

`t-scoring-month` 前两次超时 = 同窗 health_tracker 降级窗口 / 链式竞争的伴生症状，第 3 次恢复。

## 三、修复阶段（逐阶段批准推进，每阶段一 commit）

### C1 止血：日历表纠错（`astock-data/src/calendar.rs`）

- 硬编码表补 `"2026-09-25"`（中秋），摘除已知错的 `"2026-09-21"` / `"2026-09-22"`（真实 K 线 09-18→09-21→09-22→09-23 连续开市，表内注释已自证）；
- 更新表注释：注明本次纠错的 K 线证据；
- 测试：`is_trading_day(2026-09-25)==false`、`(2026-09-21/22)==true`、`latest_trading_day` 在 09-26/27/28 窗口回归。

### C2 结构修复：缓存新鲜性对日历错误免疫（`astock-data/src/lib.rs`）

- 现状 M-DS-1 判据在 **TTL=300s 的缓存上只有害处**：5 分钟窗口内 `last_date < latest_td` 的唯一解释是「vendor 本来就没有那天的数据」（日历错或数据滞后），重拉必然拿回同样的结果（实证：attempt 3 重拉仍超时）；
- 改法：判据加容忍带——`last_date ≥ latest_trading_day()` **或** 缓存写入时刻在 TTL 内即视为新鲜（等价于删掉日期判据，由 TTL 全权负责新鲜性）；保留 as-of 分支不变；
- ⚠ 决策点（开工前确认）：若你希望保留「收盘后立刻重拉当日 K 线」语义，则改为「gap ≤ 1 交易日才判过期」的折中；默认推荐 TTL 全权。
- 测试：live 下写入 last=09-24 的缓存后，`latest_trading_day=09-25` 情形仍命中（不再风暴）。

### C3 通道修复：交易日历真实数据源（`calendar.rs` + 启动接线）

- `RPTA_WEB_TRADE_CALENDAR` 已死。**方案（倾向 a）**：
  a. **K 线自证日历**：启动时拉近 40 根上证指数日 K（腾讯普通端点，健康），窗口内非周末且不在 K 线序列的日期 ⇒ 写入 `REMOTE_HOLIDAYS`；零新依赖，与「取数按真实 K 线自证」哲学一致；
  b. 调研替代报表端点（不稳定，报表说下线就下线）。
- 在 `src-tauri/src/init/` 启动流程真正调用（现状是**无人调用**）；失败静默退化走硬编码（保留）。
- 测试：populate 后 `is_trading_day` 优先远程；拉取失败路径不 panic。

### C4 复权链路：绕开腾讯 WAF 的 fqkline（`vendors/tencent.rs`）

- 腾讯 vendor 在 `adj=Some(Forward/Backward)` 时改走**不复权端点** `kline/kline` + `adj_factor=None`，由 lib 层既有 `apply_local_adjustment_if_needed`（分红事件本地复权）接管——分红源 datacenter-web 实测健康；
- 保留 fqkline 作为可选次选（若 WAF 解封仍可用）；
- 测试：本地复权 fallback 路径已有单测覆盖则复用；补「tencent 申报未复权 ⇒ lib 应用因子」链路断言。

### C5 运行期验证（不产生代码）

- 重启应用（C1–C4 生效）→ 重跑 301302 live → 预期：t-risk 正常完成（首次冷取也应 <30s：C4 后走腾讯不复权 0.2s + 本地复权）、portfolio-mgr 有决策、无级联跳过；
- 顺带核对 09-26 以来被「缓存风暴」拖累的其它 live 运行是否恢复。

### 明确不做（本轮）

- 节点 30s 超时预算不放宽（C2+C4 后冷链 <5s，无需动引擎）；
- mootdx TCP 黑洞、Defender 断 push2his 属环境/产品裁定遗留（见 [[project-codex-parity-adoption]]），不在本计划；
- portfolio-mgr 级联跳过的 UI 已有兜底，不动。

## 四、验证门禁（每阶段）

定点：`cargo check` + `cargo test -p astock-data calendar` / `cargo clippy -p astock-data --all-targets -- -D warnings`；
全部阶段完成后按 CI 口径跑一次三条全量（`fmt --all --check` / `clippy --workspace --all-targets --all-features -- -D warnings` / `__TAURI_WORKSPACE__=true cargo test --workspace`）。

## 五、取证方法备忘（复用价值）

- 运行记录：`node scripts/pg-connect.mjs sql "…jsonb_array_elements(node_executions::jsonb)…"`（`started_at/completed_at` 为毫秒，`created_at` 为秒）；
- 缓存验尸：`~/.axagent/astock_l2_cache.json` 的 `expires_at − 300s = 写入时刻`，可反推后台任务真实完成时间（超时 ≠ 取消）；
- 端点健康：curl `-w "%{time_total}"` 逐通道测，注意区分「秒断(RST)」与「黑洞(超时)」——黑洞才吃满 30s。

## 七、D 轮（C5 重跑后遗留：缺席语义仍被计为工具失败）

C5 实测（14:21 运行 `5cf5a1d9`，64 节点全 completed）：t-risk 5ms、portfolio-mgr 出决策、因子完整度 100%、综合 52.1/C → **73/B**。剩余失分：工具可信度 50.4，来自 6 个失败标记（fund 2 / hm 1 / lk 1 / sec 2），逐条对回报告原文后**全部是「数据本来就没有」被词表当成「取数机制坏了」**：

| # | 报告原句（命中词加粗） | 缺席语境 | 句级可抑制? |
|---|---|---|---|
| fund-1 | 「**无机构** EPS 覆盖、连续亏损年数缺失、商誉/质押**数据缺失**，使估值锚定…」 | 无机构覆盖枚举 | ✔ |
| fund-2 | 「**无商誉**/质押/审计非标**信息返回**，该维度**数据获取失败**」 | 无风险记录=不存在该风险 | ✔ |
| hm-1 | 「无法判断北向…该维度**数据缺失**」 | 北向个股停披（结构缺口） | ✔（语境词「北向」） |
| lk-1 | 结论段「质押**数据缺失**削弱结论确定性」 | 语境（不可得）在**另一段** | ✘ 维持计数 |
| sec-1 | 「量价结构**数据缺失**，**仅取到**当日快照」 | 能力面陈述 | ✔ |
| sec-2 | 「（…=null）」 | 数据枚举 | ✘ 维持计数 |

### D1 `data-quality.rhai`：抑制粒度 整篇 → **句级** + 缺席语境词表（模板 v86）

- R9b 的教训是「整篇 co-occur 会吞同报告真缺口」；正确粒度是**命中所在句**（以 `。！？；\n` 断句）：
  句内含该 marker 的定向缺席语境短语 ⇒ 该**次**命中不计，同报告其它句不受影响；
- 短语表（按 marker 定向，宁窄勿宽）：`数据缺失` 在既有 5 条上追加
  「无机构 / 无覆盖 / 无商誉 / 未提供 / 未披露 / 仅取到 / 不存在 / 无记录 / 需明确标注 / 停披」；
  （⚠ 初稿曾列「北向」，被历史判据当场否决：真缺口句「龙虎榜席位与北向数据缺失」
   同样含「北向」⇒ 该词过宽会吞真故障，D2 测试 `clarification_sentence_is_not_counted_as_gap` 抓回）
  `获取失败` 新增抑制组「无商誉 / 无信息 / 信息返回 / 未提供 / 不存在」；
  动词硬标记 `无法获取/未能获取/占位/TODO` 等**维持零抑制**；
- 现有 `soft_marker_suppression()` 的判定同步从整篇改句级（语义只收紧不放松：
  原来整篇能抑制的，句级在澄清同句时仍抑制；异句时不再连坐 ⇒ 更准）。

### D2 门禁测试（`rt-workflow/tests/data_quality_placeholder_gate.rs`）

- 新增：用上表 4 份真实报告语句断言 6 → 2 命中（fund/hm 清零、lk/sec 各保 1）；
- 保真：既有 18 测试全绿（`hard_marker_survives_clarification` 语义在句级下更强，应天然通过）；
- seed 注释 v86 + `TEMPLATE_VERSION=86`。

### D3 运行期核对（用户动作）

重启应用（v86 重新 seed）→ 重跑 301302 live → 预期：工具可信度 50.4 → ~70、低置信行 5 → ~2、综合 ≈ 79–80（B+/A-）。

### 不做

- 报告 prompt 改「数据可得性/结论信心」双字段（R9 候选②，动 10 个分析师提示词，收益与 D1 重叠，暂缓）；
- lk/sec 剩余 2 个命中的进一步抑制（跨段语境连坐风险 > 收益，维持保守）。

### D4 v86 生产回归修复（模板 v87）— 断句循环烧穿 CodeNode 操作预算

**现象**（用户实证，16:54 运行 `2bb5bb9d`）：重启后 data-quality 节点直接 failed
（`Too many operations (line 575)`），面板「决策信息」空、逐节点诊断弹窗全空 —— 比误扣分更严重。

**根因（两层）**：
1. D1 的句级断句用逐字符 `sub_string(i,1)`+拼接实现，10 份 KB 级报告烧穿生产预算
   `code_executor.rs:57` `set_max_operations(200_000)`；
2. **门禁为何没拦住**：测试引擎给了 `set_max_operations(2_000_000)`（10 倍余量掩盖了问题）。
   判据同「门禁命令覆盖面本身就是门禁的一部分」——**测试引擎参数必须与生产逐字对齐**。

**修复**（v87，全部在 `data-quality.rhai` + 门禁）：
- 断句改**链式 `split`**（C++ 侧一次调用，不计逐字符操作）：`\n`→`。`→`；`→`！`→`？` 五轮；
- `marker_counts` 预算捷径：无抑制短语组的**硬标记**整篇 `split(m)` 直接计数，不进逐句循环
  （marker 不含句读分隔符 ⇒ 与逐句求和恒等）；软标记才惰性切句一次并共享；
- **每报告只算一次** `marker_counts`：`placeholder_hits`/`placeholder_occurrences`/
  `has_placeholder` 三条重复全量统计路径收敛为 `hits_from_counts`/`occurrences_from_counts`
  两个投影（v86 里同一报告被统计 3 遍）；`report_quality` 改收第 4 参 `counts`；
- 门禁：测试引擎额度对齐生产 `200_000`，新增压测 `d1_long_reports_fit_production_max_operations`
  （10 × KB 级报告 + 每份含真实缺口句，断言在预算内跑完且命中数正确）；
- **必须升版到 v87**：坏脚本已以 v86 播种进用户库，版本门 `existing.version >= TEMPLATE_VERSION ⇒ skip`。

## 八、E 轮（2026-09-30 实机发现：兜底表误列 09-28/29/30 连休 → 收盘采集错贴标签）

**现象**（mover-recall Phase 1 实机核对）：应用首个 tick 报「`2026-09-24` 快照: 落库 46/46」，
而当天是 09-30 交易日；库内 46 行 `market_daily_close` 全部 `trade_date=2026-09-24`，
`601231` 行 close=23.74 —— 与腾讯 09-30 日 K 收盘逐字相等 ⇒ **标签错、值是当日值**。

**根因**：硬编码兜底表把 `2026-09-28/29/30` 误列连休（三行自 09-02 建表提交起就在表内；
C1 的 K 线自证只推到 09-24，**未来日期无从证伪**）。`previous_trading_day(09-30)` 错落 09-24，
`run_close_sweep` 按传入 trade_date 落库 ⇒ 09-30 行情被贴 09-24 标签。
（09-25/28/29 各轮 sweep 同 id 覆写过同一批行，最终值 = 最后一轮 = 09-30。）

**修法**（`astock-data/src/calendar.rs`）：
1. 摘除误列三行；
2. 新增 `REMOTE_TRADING_DAYS`「确已开市」正向集合：`populate_holidays_from_kline_dates`
   同步并入 K 线日期；判据顺序 **正向自证 → 休市自证 → 周末 → 兜底表**
   —— 正向集合是唯一能推翻「表里未来日期误标」的机制（C1 靠人工发现，本次起可自纠）；
3. 测试：`hardcoded_mislabel_removed_and_proven_overrides_table`（纯函数双侧）、
   `populate_kline_dates_wires_proven_trading_days`（接线回归，持全局态串行锁）；
   `is_trading_day_with` 抽为纯函数以便用局部集合测试（零全局副作用）。

**教训**：兜底表纠错至今三轮（09-21/22 → 09-25 → 09-28/29/30），形态全是「未来日期无从证伪」——
所以修复不能只改表，必须给「真实数据」一条能自动改写判据的通道（本次即 `REMOTE_TRADING_DAYS`）。

**实机验证（2026-09-30 22:39 重启后）**：

- 22:39:39 `[init] 交易日历指数K自证完成: 60 根K线, 新增休市日 1`（K 线自证通道启动即生效）；
- 22:46:52 `[market-close] 2026-09-30 快照: 落库 46/46, 无有效值 0, 失败批次 0/1`
  —— 目标日首次正确落 09-30（修正前为 09-24）；同轮 `清单第 [1..6] 页: 落库 0 只,
  失败页 6` 为东财 clist 封禁遗留（已知，非本次回归；清单现为 bootstrap 本地 46 只，非全市场）；
- 22:44:39 `[daily-snapshot] 快照采集完成: 全市场 4 项, 个股 0 条` —— 快照文件新增
  4 个 `daily:*:2026-09-30` 键（含此前为 `[]` 的 hot_stocks，本次 20 条真实数据）。

**数据清理（2026-09-30）**：

- 删除 `market_daily_close` 中 `trade_date='2026-09-24'` 的 46 行：全批量 FULL JOIN
  逐列对比（close/prev_close/change_pct/stock_name）与 09-30 批 **0 差异、0 单侧独有**
  ⇒ 系错标签的完整副本，删除零信息损失；删后仅剩 `2026-09-30 → 46 行`；
- 污染扫描（只读）：`stock_analyses`(09-20~09-28, 19 行) 与 `stock_reflections`(09-28, 1 行)
  逐行 `created_at` 均为当天写入、as_of 即当天 ⇒ 未受污染（live as_of=运行日原值：
  09-27 周日运行的行 as_of=09-27 而非 09-24，不经 previous_trading_day）；
  `portfolio_metrics_daily` / `stock_pipeline_runs` 近窗零写入；
  快照文件旧键（09-09/09-24）未被 09-28~09-30 的轮次覆写（幂等闸门只看 index_quotes）；
- 遗留：09-24 真实收盘数据现缺（错标批已删；当日真实批即便曾采过也已同键覆写）。
  属「结构性缺口」非「伪装值」；mover-recall Phase 1 若需该窗口数据，按 Phase 0 已验通道
  （腾讯批量 80 码/请求，零失败）另行补录，不阻塞。
