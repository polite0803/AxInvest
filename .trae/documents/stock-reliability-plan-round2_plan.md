# 执行计划：PLAN-stock-decision-reliability 第二轮追加（宏观因子缺陷 + 反思未激活根因）

## Context（为什么做这件事）

第一轮已交付紫金矿业 10-09 误判的七根因（R1–R7）与修复清单（P0/P1/P2）。
用户随后要求：① 把近期**整个股市走低**综合进来，查宏观因子链路是否还有其它缺陷；
② 查明**「按时反思」为什么没有自动激活**。两轮调查均已完成，证据（代码行级 + PG 实证）
已取齐，但结论尚未写入交付文档。本计划仅做**文档追加**，不改任何代码；改哪些代码、
先后顺序由用户看完文档后圈定。

交付物：`docs/plans/PLAN-stock-decision-reliability.md`（已存在，增量编辑，遵守
AGENTS.md：docs/plans/ 放置、跨文档引用只写文件名、源码引用段对齐）。

## 已核实的核心证据（本会话新增/复核）

### A. 大盘事实（东财上证指数 000001 日 K，PG/接口实证）

- 9/21 高点 3967.68 → 10/15 低点 3674.82，**-6.1% 明确下跌波段**，高低点同步下移。
- 逐日：10/8 -0.79%、10/9 +0.05%（盘中砸到 3754）、10/12 -0.43%、10/13 +0.10%、
  10/14 -0.88%、10/15 -1.59%。紫金 10/9 +4.87% 是逆势个股异动。
- 系统动作到 10/14 才转向谨慎（当日 买入6 vs 持有9/减持4/卖出1，18 条版本号为空）。

### B. pool-scan / 反思调度死亡链（本会话代码级钉死）

- `cron_jobs` 实证：`pool-scan-daily` schedule `0 2 * * *`，run_count=2，
  last_result=「非交易日（2026-10-10），本轮跳过」，**next_run_at=0**，updated_at=10/14 11:10；
  10/12–15 cron_job_history 零行（成功/失败都没有）。
- 对照组全部健康：`decision-backtest-daily` 10/15 07:01 正常跑（written=165）、
  `stock-reco-daily`/`trend-screening-daily` 10/15 05:18 重启补发、
  `reflection-due-daily` 10/14 18:13 跑过（totalPending=0，12ms）——**调度器没死**。
- 机制链：
  1. `crates/runtime-core/src/cron_job.rs:367-381` `resumed_next_run`：重启时档期已过期
     ⇒ 置 `Some(0)`（设计上补发一次），解释 next_run_at=0 的来源（**不是**闸门造成；
     闸门 `src/init/services.rs:2241-2263` 跳过后走 `record_run`，档期会正常推进）。
  2. `crates/runtime/src/cron/scheduler.rs:47,77,125-130` 在途去重表：派发后
     `record_run` 不推进 ⇒ 同进程内永久压制。
  3. `src/commands/pool_scan.rs:68-70` 内置配置 `{lookbackDays:0, maxStocks:0}` =
     **全池不截断**；`pool_scan.rs:412-469` `analyze_candidate_queue` **串行**逐只跑
     完整 LLM 分析链；`services.rs:2790` 只在全部跑完后调一次 `record_run`；
     spawn 外层**无超时**（config.timeout_seconds=300 无人消费）。
  4. 桌面端每日关 app/重启 ⇒ 长任务中途被杀 ⇒ 无成功/失败历史行、档期停 0、
     下次重启从头再来。全池积压（四档×topN 长期累积）下单轮可达数小时，近乎必被杀。
- 反思侧另有四层独立断层（上一会话已查实，记录到 R12）：
  - `reflection.rs:2141` `reviewable_horizons` 只为方向动作（Buy/Increase/Sell/Reduce）建点，
    持有/观望永不复盘；门槛 `DEFAULT_REFLECTION_MIN_CONFIDENCE=70`（reflection.rs:2091）。
  - 10/14 v154/v156 模板铺开前，历史 live 分析在 stock_reflections 零行；
    紫金三条分析（10/9、10/10、10/12）至今无反思行。
  - 到期口径用自然日（reflection.rs:2252-2256）：long 90 自然日 ⇒ 到期日 2027-01-12；
    当前 45 条 pending 全部未到期（与经验 1229656「T+N 必须用交易日 bar」冲突）。
  - ~16 条 failed 永久终局（4 条 LLM 上下文超限、5 条 ANALYSIS_SNAPSHOT_MISSING、
    6 条 7 月孤儿），无死信重试；running 仅 6h 陈旧回收（reflection.rs:2462）。
  - strategy_performance 仅 8 行且 template_version 全 NULL，被 FLOOR=125
    （harness/holding_period.rs:663）整批排除；真实胜率 3/8=37.5%、均亏 -2.82%。
  - 10/14 起实际产出 resolved 的是 `start_batch_reflection` 的**每 6h 系统兜底轮**
    （services.rs:4032 注释、`record_auxiliary_run`，cron_job.rs:653-692），
    不是 `reflection-due-daily` 档期任务——后者跑时 totalPending=0（没点可消费）。

### C. 宏观因子链路缺陷（上一会话已查实，行号复核）

- `crates/analysis-engine/src/market_regime.rs:30-132`：仅沪深300近60日 K +
  MA60/MA20/5日斜率判牛/熊/震荡；无市场级硬门禁，bear 只把先验调到 **0.45**。
- `src/commands/stock_workflow/hooks.rs:610-642`：regime 注入失败只给 unknown
  让 LLM 自判；`src/commands/regime-weights.rhai:57-58` 把 sideways/unknown 全映成 neutral。
- macroRegime 腿只在 long 档占 18% 权重，信号源是月度 PMI；10/9 快照 PMI 50.1
  （腿信号 +0.01）、CPI/PPI 为 8 月值、GDP 停在 2026 Q1（滞后半年），
  约束标 `macroPartialSeriesOnly` 却保留权重。
- 封板率是市场级量（`crates/astock-data/src/vendors/ths.rs:664`），同日全市场个股同信号；
  `crates/harness/src/holding_period.rs:490-494` 注释自承认；10/9 84% 封板率给紫金
  +0.683/54% 权重。short 档 10/10 起降为背景过滤（evidence_weight.rs:1260），
  **ultra 档仍是方向腿**。

## 文档修改步骤（唯一文件：docs/plans/PLAN-stock-decision-reliability.md）

1. **标题与导语**：副标题扩展为「个股误判 + 大盘下行宏观链路 + 反思调度失效」，
   导语加一段说明第二轮增补范围与日期。
2. **§1.3 后新增 §1.4「大盘背景：9/21–10/15 上证 -6.1% 下跌波段」**：
   写入 A 组事实表（指数逐日收盘/涨跌、紫金逆势异动、系统动作分布转向时点），
   注明数据源（东财 000001 日 K）与查询口径。
3. **§二 R7 后追加 R8–R12**（统一格式：现象 / 证据（文件:行号）/ 后果 / 与本次误判关系）：
   - **R8** 单指数滞后 regime + 无熊市硬门禁：60 日 MA 系对 -6.1% 波段识别迟缓，
     bear 仅先验 0.45，下跌波段照出 long 买入；unknown/sideways 被 rhai 映成 neutral。
   - **R9** 市场级封板率为个股背书：同日所有个股同信号，10/9 高封板率在大盘当日砸盘时
     仍给逆势个股 +0.683；short 已降背景、ultra 仍是方向腿（与 R3 联动放大）。
   - **R10** 宏观腿时效失配仍保留 18% 名义权重：GDP 滞后半年、CPI/PPI 月余、PMI 月频，
     腿信号近 0 却占权重，挤占有效腿；注入失败静默降级 unknown。
   - **R11** pool-scan「全池串行 + 末尾一次 record_run + 无超时 + 重启置 0 + 在途去重」
     复合死亡：候选池→逐只分析环节在桌面端事实上长期不产出，反思队列因此无米下炊。
   - **R12** 反思覆盖四断层：建点门槛 70 + 仅方向动作、老版本/v156 前 live 无点、
     自然日到期（long 落 2027-01-12）、failed 无重试；6h 兜底轮掩盖了档期任务的空转。
4. **§三 修复清单追加**（不动既有 P0/P1/P2 条目，新条目从 P0-6 / P1-6 起编号）：
   - P0：① pool-scan 改「分批 + 每只完成即推进/落检查点」+ spawn 加硬超时（消费
     timeout_seconds）+ 启动恢复 next=0 时先判当日是否已有成功批次（复用
     「当日已分析」判据 pool_scan.rs:261-272）；② 熊市市场级硬门禁（regime=bear
     禁新开仓/降总仓位，动作层而非展示层，针对 R5/R8）；③ failed 反思死信重试
     （上下文超限类直接走裁剪后重跑，c247e03e2 的三层预算已修根因，可安全重试）。
   - P1：④ 用 `src/bin/axagent-batch-rerun.rs` 既有
     `build_pending_reflection_rows_for` 通道为 10 月历史 live 分析补建 pending；
     ⑤ ultra 档封板率降纯背景门（对齐 short 档 10/10 改法）；
     ⑥ 宏观腿时效门禁（数据滞后超阈值权重置零并显式标注 stale）；
     ⑦ 反思到期改交易日口径（复用 astock calendar），持有/观望动作抽样复盘；
     ⑧ 调度可观测性：record_run 心跳/在途时长告警，next=0 超 N 小时未推进进任务面板异常态。
   - P2：⑨ regime 多指数 + 市场宽度（涨跌家数/新高新低）融合；
     ⑩ strategy_performance 样本量 FLOOR 之下改用收缩先验而非整批丢弃。
5. **§四 个案处置**：补一条——紫金三条历史分析用 P1-④ 通道补建反思点，
   作为修复后第一批真实验证样本。
6. **§五 证据索引追加**：market_regime.rs、regime-weights.rhai、hooks.rs、
   ths.rs、holding_period.rs、evidence_weight.rs、services.rs（闸门 2231-2263 /
   pool 分支 2704-2807 / 兜底轮 4032）、scheduler.rs、cron_job.rs、pool_scan.rs、
   reflection.rs 各关键行；cron_jobs/cron_job_history/stock_reflections/
   strategy_performance 的查询口径与关键数值。引用一律段对齐路径（`commands/...rs:行号`）。

## 不做的事

- 不改任何代码（用户明确：文档先出，P0 项由用户圈选后再实施）。
- 不新建文件、不动其它文档；不跑编译。
- 文档不臆断消息面/政策面归因（经验 1897707），所有大盘表述只用 K 线实证。

## 验证方式（文档质量自检）

1. 结构自检：新增编号 R8–R12、P0/P1 新条目与既有编号不冲突、目录锚点一致。
2. 事实自检：每个数值（-6.1%、next_run_at=0、run_count=2、45 pending、8 行等）
   均来自本会话/上会话已执行的 PG 查询或代码行，不新增未经验证的数字。
3. 引用自检：所有源码引用路径段对齐（完整仓库相对路径或 `/` 边界后缀 + 行号），
   无裸 actions.rs 式截断；跨文档引用只写文件名。
4. 格式自检：文件 LF（Edit 工具保持原行尾）；完成后用 `scripts/check-single-source-facts.mjs`
   对该文档跑一次源码引用校验（只读）。
