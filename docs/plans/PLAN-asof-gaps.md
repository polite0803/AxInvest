# PLAN：as-of（时间旅行/回放）模式剩余取数缺口

> 状态：active ｜ 日期：2026-09-25 ｜ 对应版本：v2.9.9


> 盘点日期：2026-09-25。判据来源：对 `src-tauri/crates/astock-data/src/lib.rs` 全部
> `pub fn get_* / search_*` 逐函数体扫描（按 `    pub (async )?fn` 边界切分后检查函数体内
> 是否出现 `is_asof_active` / `current_as_of` / `truncate_*_by_asof` / `*_with_asof` /
> `try_*_daily_snapshot` / `record_degradation`），不是按方法名猜。

## 0. 结论速览

| 桶 | 性质 | 条目 |
|---|---|---|
| **D 时间泄露** | 回放里返回越过截止日的实时/全量数据，**静默**（不报错、不降级） | 6 组 |
| **A 恒空** | 一进 as-of 就返回空，本可接通道 | 1（`get_pledge_data`） |
| **C 哑弹** | vendor 申报了 as-of 能力但没有实现，靠路由层兜底才没泄露 | 2 族 |
| **B 数据缺失** | 读取代码正确，本地语料/快照历史本来没有 | `news_archive` 一族 + 快照不可追补 |

D 桶优先级高于 A/B：**「取不到」会在降级面板留痕并让分析师写"数据缺失"；「取错」什么都不说。**

## 1. 基线（本轮已合流，后续阶段以此为前提）

- `get_index_quotes` 的 as-of 分支从「无条件返回空」改为 缓存 → 每日快照 → vendor 按截止日
  K 线合成（`src-tauri/crates/astock-data/src/lib.rs:4423`、合成实现
  `src-tauri/crates/astock-data/src/vendors/eastmoney.rs` 的 `get_index_quotes_with_asof`）。
- `get_margin_data` / `get_money_flow` / `get_north_bound_holding` 接上**个股级**每日快照读取
  （`DailySnapshotCache::get_stock`，此前个股快照只写无读）。
- 降级日志按 `(vendor, method, reason)` 去重；逐 vendor 的「无 as-of 能力」不再各记一条，
  收敛为一条汇总（`src-tauri/crates/astock-data/src/as_of.rs` 的 `record_degradation`）。
- 装配接线：`with_l2_cache` + `with_daily_snapshot_cache`（`src-tauri/src/init/state.rs`）、
  L2 flush 任务随 `shutdown_token` 优雅退出、新增后台 `start_daily_snapshot_sweep`
  （交易日 15:00 后幂等采集，`src-tauri/src/init/services.rs`）。
- trait 默认 `get_index_quotes_with_asof` 从「调 live」改为**显式失败**（时间泄露防线，
  `src-tauri/crates/astock-data/src/vendors/mod.rs`）。

⚠ 由基线推出的事实：**每日快照只对"今后每天采过的日期"有回放价值**，历史日期仍然空。
任何「靠快照兜底」的方案都不构成对过去回放的修复。

## 2. D 桶 · 时间泄露（逐条：现象 / 根因 / 修法 / 判据）

### D1 `get_valuation_history` —— 估值分位窗口与"当前值"都是今日口径

- 位置：`src-tauri/crates/astock-data/src/lib.rs:2317`（函数体内**零** as-of 痕迹）。
- 放大路径：`compute_valuation_band` 工具
  （`src-tauri/crates/astock-data/src/mcp_tools.rs:1666`）用
  `since_date_from_years(years)` 定窗（锚点是 `chrono::Local::now()`，
  `src-tauri/crates/astock-data/src/valuation_band.rs:187`），再
  `clip_valuation_history`（只裁下界，不裁上界）→ `current = snaps.last()`。
  ⇒ 回放里既拿到截止日之后的样本，又把今天的 PE/PB 当作"当前值"。
- 消费节点：`t-valuation-band` → `portfolio-mgr`
  （`src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:3138`、`:4246`）。
- 修法：① `ValuationSnapshot.trade_date` 已存在 ⇒ 在 `get_valuation_history` 返回前按
  截止日截断（新增 `truncate_valuation_by_asof`，与既有
  `truncate_financials_by_asof` 同形态）；② `since_date_from_years` 内部改按
  `as_of::current_as_of()` 取锚点（无 as-of 时仍是 `Local::now()`）——**不改函数签名**，
  避免动到前端 live 图口径（该函数注释已声明"显式传 0 年 = 空窗口"的语义，勿顺手加 `max(1)`）。
- 判据：单测——固定 `as_of=2024-06-01`，喂入跨截止日的快照序列，断言 ①序列末条
  `trade_date <= 2024-06-01` ②`band.current` 等于截止日当日值而非末条今日值。

### D2 `get_dividend_records` —— 截断函数早就写好，但全库零调用点

- 位置：`src-tauri/crates/astock-data/src/lib.rs:3505`；死代码
  `truncate_dividend_by_asof` 在 `src-tauri/crates/astock-data/src/lib.rs:1120`
  （全库只出现 2 次 = 定义 + 自身文档字符串）。
- 影响：截止日之后的分红/除权事件进入回放输入 ⇒ 事件驱动型判断（除权后价格、送转预期）拿到未来信息。
- 修法：在 `get_dividend_records` 的 Ok(result) 入口调用已有截断（`ex_date` 为主字段，
  `record_date` 为回退），与 D1 一并做，成本几乎为零。
- 判据：单测锁「截止日后一条分红必须被丢弃」。

### D3 `get_social_sentiment` —— 舆情取当下

- 位置：`src-tauri/crates/astock-data/src/lib.rs:2161`；消费节点 `t-sentiment-data`
  （`src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:2071`）→ a-sentiment。
- 结构限制：`SocialSentiment` 只有 `fetched_at`（采集时刻），**没有帖子发布时间**
  ⇒ 无历史语义，截断无从下手。
- 修法（二选一，推荐 ①）：① 把 `get_social_sentiment` 加入
  `src-tauri/crates/astock-data/src/daily_snapshot.rs` 的 `SNAPSHOT_METHODS` + `PER_STOCK_METHODS`，
  as-of 分支先读个股快照（与两融/资金流同口径），未命中才 `record_degradation`；
  ② 直接 `record_degradation` 返回空（诚实但让 a-sentiment 恒缺）。
- 连带：该函数内部调 `get_hot_stocks` 填 `hot_rank`，基线已让 hot_stocks 走快照，
  修完后这一段自然一致；不修则"舆情是快照、热度是今天"混合时间轴。

### D4 `get_earnings_calendar` —— 未来财报日泄露

- 位置：`src-tauri/crates/astock-data/src/lib.rs:2140`。`EarningsEvent.event_date` 存在 ⇒ 可截断。
- 语义抉择（需拍板，写进 PR）：回放时点「未来的财报日」不可知 ⇒ 只保留
  `event_date <= 截止日` 的已发生事件；若想保留"当时已公布的预告"，需 vendor 侧披露日期，
  当前 DTO 无该字段，**不实现**。
- 判据：单测锁「event_date 晚于截止日 ⇒ 丢弃」。

### D5 `search_concept_boards` / `get_concept_board_members` —— 板块当下构成

- 位置：`src-tauri/crates/astock-data/src/lib.rs:4102`、`:4142`；trait 默认
  `*_with_asof`（`src-tauri/crates/astock-data/src/vendors/mod.rs`）即调 live。
- 定性：板块归属/成分本身无历史语义（概念板块调整是月级），历史回放里只能取「当时快照」。
- 修法：as-of 分支加 `record_degradation`（与 `get_concept_blocks` 同口径），
  或纳入每日快照。⚠ 不要给这两个补 `*_with_asof` 默认调 live 的实现——那正是基线里
  `get_index_quotes_with_asof` 改掉的那个坑。

### D6 跨市场 K 线绕过截断

- 位置：`get_international_klines` `src-tauri/crates/astock-data/src/lib.rs:1850`、
  `get_benchmark_klines` `:1879`、`get_forex_klines` `:1928`。
- A 股主路径 `get_klines_with_adj`（`:1971`）有 `get_klines_with_asof` +
  `truncate_klines_by_asof`，**这三条跨市场路径两条都没有**。
- 修法：统一在返回前过 `truncate_klines_by_asof`（international vendor 无日期参数时，
  截断是唯一可行解）。
- 判据：单测——`as_of` 早于今天，断言返回的最后一根 K 线日期 <= 截止日。

## 3. A 桶 · 恒空

### A1 `get_pledge_data`（`src-tauri/crates/astock-data/src/lib.rs:3222`）

- 现状：as-of 分支直接 `record_degradation + return Ok(None)`，一条通道都没试。
- 约束：`PledgeData` 无日期字段（`src-tauri/crates/astock-data/src/types.rs`），vendor 侧也
  没有 `get_pledge_data_with_asof`（`vendors/mod.rs` 里根本没声明该 trait 方法）。
- 修法（分层）：
  1. **本次做**：把 `get_pledge_data` 加入 `SNAPSHOT_METHODS` + `PER_STOCK_METHODS`，
     as-of 先读个股快照（基线已有 `try_stock_daily_snapshot`）——从此每天 15:00 后采集，
     回放开始有数据；未命中记一条汇总降级，文案区分「无 as-of 通道」与「确无质押」。
  2. **可选（另立项）**：给 eastmoney 质押报表（`get_pledge_data` 用的 datacenter 接口）加
     `TRADE_DATE` 参数的 `_with_asof` 实现，并在 `asof_capability` 申报 `NativeDateParam`。
- 消费方：`t-pledge-data`（`src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:2060`）。

## 4. C 桶 · 申报与实现打架（哑弹）

- `src-tauri/crates/astock-data/src/vendors/tencent.rs:442`、
  `src-tauri/crates/astock-data/src/vendors/baidu_stock.rs:749` 申报
  `get_klines = NativeDateParam`，但**均未 override** `get_klines_with_asof`
  （trait 默认 = 调 live）。
- `sina` / `mootdx` / `tencent` / `baidu` 申报 `get_quote = SynthesizeFromKline`，同样无实现。
- 当前不泄露的原因：路由层兜底——`truncate_klines_by_asof`
  （`src-tauri/crates/astock-data/src/lib.rs:1971` 函数体内）与
  `quote_from_klines`（按日期筛条）。**风险在于兜底一旦被移除即成泄露，且没有任何测试能发现。**
- 修法建议：**诚实申报**（把这几处改成 `Fallthrough` / `NoHistoricalSemantic`），
  而不是补实现。改前必须确认路由层对这两个新取径的行为：
  `Fallthrough` ⇒ 调用方自己截断（A 股 K 线已有）；`NoHistoricalSemantic` ⇒ 查快照。
  按 AGENTS.md 铁律：先 grep 既有先例照抄形态，别为过 lint 改语义。
- **实施后补记（P5）**：`get_klines` 一侧已按上述改完（tencent / baidu，行为等价）。
  `get_quote` 一侧的 `SynthesizeFromKline` 申报是**惰性的**——路由层 `get_quote` 的 as-of 分支
  自己走 `quote_from_klines` 合成，从不调 `vendor.get_quote_with_asof`，
  所以那几家即使没实现也不会泄露。**不要照着 §4 去"修"它们**：没有缺陷可修，只有噪音可造。
  真正需要记住的是那条不变量：**申报了能力的 vendor 方法，必须有对应 `_with_asof` 实现，
  否则就别申报**（`get_index_quotes` 的 trait 默认实现已于同日改为显式失败，就是这个原则的落点）。
- 判据：能力申报单测逐条对齐（`vendors/*/capability_tests` 已有同类断言形态）。

## 5. B 桶 · 数据本来就没有

- `get_news` / `get_policy_news` / `search_news`：读取侧已正确接
  `news_archive.search_asof`；表内数据依赖 live 增量 upsert，且 sink 直到 2026-09-24 才恢复注入
  （`src-tauri/src/init/state.rs` 注释），**无回填工具**。回放早于入库积累期的日期必然空 ⇒
  属结构性缺失，文案已写「该维度结构性为空」。
- 可选下一步（不在本 PLAN 实施）：找一个支持历史区间的新闻源做 backfill 任务，
  否则新闻维度在 2026-09 之前的回放永远为空。

## 6. 预检盲区（为什么 D 桶不会自己冒出来）

`data_quality_precheck`（`src-tauri/src/commands/stock_workflow/decision.rs:125`）实际调用并
校验 10 个维度：quote、financials、klines、news、money_flow、announcements、concept_blocks、
sector_info、lockup_schedule、dragon_tiger。

**不校验**（空或泄露都静默）：valuation_history、social_sentiment、earnings_calendar、
dividends、pledge、research_reports、institutional_visits、index_quotes、peers、option_pcr、
hot_stocks、industry_ranking。→ D1–D4 全部落在盲区。

⚠ 不要用「把这些维度加进预检」来治 D 桶：预检查的是「够不够数跑 DAG」，而 D 桶是「数是真的但来自错误的时点」。
正解是 §7-P1 的**时间边界不变量测试**：对固定截止日跑一遍取数面，
断言每个返回项的日期字段 `<= 截止日`。这条测试一旦建立，未来任何新增取数方法漏接 as-of 都会被它抓住，
不必再靠人工盘点。

## 7. 实施阶段（2026-09-25 一次性实施完 P1–P5，P6 留作独立立项）

**状态：P1–P5 已全部完成并转绿**（astock-data lib 单测 366 passed / 0 failed）。
逐阶段一个 commit，回滚粒度保留。

- **P1 ✅ 时间边界不变量测试（先立判据，不含功能修改）**
  落在 `src-tauri/crates/astock-data/src/lib.rs` 的 `asof_boundary_tests`（8 条断言）：
  替身 `StubVendor` 一律返回**跨截止日**数据，测试内直接改 `routing` 各字段 +
  `set_enabled_vendors` 只放行替身 ⇒ **零真实网络**（避开本仓 live-network flaky 老账）。
  红靶已实证：修之前 7 红 1 绿（绿的正是 `live_mode_is_not_clipped` 负控），
  ⇒ 断言有区分力，且泄露只发生在回放侧。
- **P2 ✅ D2 / D4 / D6**
  `truncate_dividend_by_asof` 摘掉 `#[expect(dead_code)]` 并接进 `get_dividend_records`；
  新增 `truncate_earnings_by_asof` / `truncate_valuation_by_asof`（与既有 `truncate_*` 同形态）；
  国际 / 外汇 / 基准三条跨市场 K 线过 `truncate_klines_by_asof`。
  **实施中新发现的同类缺陷（原盘点未列）**：这三处的 cache key 用的是裸 `format!` 而非
  `cache_key_for` ⇒ key 不带 `asof-` 后缀，live 抓到的跨截止日数据会被回放直接读走，
  光加截断拦不住缓存这条路。已一并改为 `cache_key_for`，并由
  `cross_market_cache_keys_are_mode_scoped` 单独锁住。
  另一处顺带闭合：`get_international_quote` 在回放下改为**用截断后的国际 K 线合成**
  （原先直接返回今日价，属 D 桶同型）。
- **P3 ✅ D1 估值带**：截断（先裁再落缓存）+ 窗口锚点两处一起改，
  并由 `valuation_band::tests::since_date_anchors_to_cutoff_in_replay` 同时锁住
  「回放锚截止日」与「live 仍锚今天」两个方向。
- **P4 ✅ D3 + A1**：`get_social_sentiment` / `get_pledge_data` 纳入每日快照，
  三处同步无遗漏（清单 `SNAPSHOT_METHODS` + `PER_STOCK_METHODS`、
  `run_daily_snapshot_sweep` 采集臂、as-of 读侧）。
  ⚠ 交付口径按「代码完成」计：真实数据需连续采集数日才可端到端验证，
  本轮不做端到端断言。
- **P5 ✅ D5 + C 桶**：概念板块搜索/成分在回放下留痕返回空（不再给当下构成）；
  tencent / baidu 的 `get_klines` 申报由 `NativeDateParam` 改为诚实的 `Fallthrough`
  （两者从未 override `get_klines_with_asof`，原申报是假话）。
  **行为等价**：两条路径最终都落到 `truncate_klines_by_asof`，改的只是不再谎报能力。
  ⚠ **未动**的部分（刻意留下）：sina / mootdx / tencent / baidu 申报的
  `get_quote = SynthesizeFromKline` 目前是**惰性申报**——路由层 `get_quote` 的 as-of 分支
  自己用 K 线合成（`quote_from_klines`），并不调 `vendor.get_quote_with_asof`，
  所以它们没有机会泄露。改动它没有收益、只有回归面，故保留并在 §4 记为「惰性、勿依赖」。
- **P6 ⏸ 未做（按决定单独立项）**：`news_archive` 历史回填。

每阶段收尾按 CI 口径定点跑：`cargo clippy -p axagent-astock-data --all-targets -- -D warnings`
+ `__TAURI_WORKSPACE__=true cargo test -p axagent-astock-data --lib`；
**全部阶段完成后**再跑一次全量三条（`cargo fmt --all --check` /
`cargo clippy --workspace --all-targets --all-features -- -D warnings` /
`__TAURI_WORKSPACE__=true cargo test --workspace`）。


## 8. 风险与不可逆点

- **每日快照会真的打 vendor**：P4 把舆情/质押加进采集清单 = 每日多 N×自选股 次请求。
  后台任务已有三重收敛（交易日判定 + 15:00 后 + 当日已有快照则跳过），
  且 `daily_snapshot_enabled()` 为 false 时不启动；扩容清单前先确认这两道闸仍然有效。
- **`since_date_from_years` 的 live 语义**：它是前端估值带图的窗口锚点，改动会同时影响 live；
  只允许「有 as-of 时用截止日」这一条分支，其余行为逐字保持。
- **诚实申报（P5）会改变路由分支**：把 `NativeDateParam` 改成 `Fallthrough` 后，
  路由层不再调 `*_with_asof`。必须先确认截断/合成兜底在位，否则从「拿不到历史」变成「拿到今日数据」。
- 回放结论若已落库（`analysis_results` 的 `blackboard_snapshot`），修 D 桶会让同截止日的
  新旧两次结果不可比；批量重跑（`axagent-batch-rerun`）需说明这一点。

## 9. 相关既有文档

`PLAN-analysis-data-repair.md`（§12.20 news as-of 漏接线的根因，D 桶同源）、
`PLAN-stock-audit-fixes.md`（`t-index-quotes` 载荷恒空的历史实证）、
`AUDIT-codebase-review-roadmap-2026-09-19.md`（`get_margin_data` 手写重试循环绕过
`try_vendors_retry`，解释了它为何独立于通用 helper）、
`AUDIT-duplication-2026-09-13.md`（`xxx` / `xxx_with_asof` 成对函数属有意样板）。

## 10. 二轮 T 桶（2026-09-26，源自「回放截图质量归因」+ 用户 7 行缺口清单）

用户判定：**「每日 15:00 在线采集快照」这条路基本不现实**（要保证那一刻进程在跑），
必须改成「实时向数据源取指定日期的数据」。本轮据此把 T2 从「补 news_archive 兜底」
改向为「建 vendor 侧时间回溯通道」，并把快照退为可选快路径。

| # | 现象（面板/报告原文） | 根因 | 修法 | 判据 |
|---|---|---|---|---|
| T1 | 回放行情比截止日**倒退 4 个交易日**；`get_quote` 偶发拿到截止日之后 | 交易日历靠硬编码兜底表推断（表里 2026-09-21/22 被误列为休市、2025-02-08 周六被当作调休上班） | 改成**数据自证**：`kline_cache_key` 去掉 `eff=` 派生后缀；`quote_from_klines` 取「≤ as_of 的最后一根」并用前一根算 `pre_close`；`get_klines_with_adj` 在 as-of 下跳过新鲜度校验；`calendar.rs` 删掉 `workdays` 调休表（附实测证据注释） | `weekend_never_trades_even_on_makeup_workday` + 缓存键三条用例；全绿 |
| T2 | `get_news: news_archive 无该股历史新闻（该维度结构性为空）`、政策新闻恒空 | as-of 只有 `news_archive` 一条通道，而它靠 live 抓取顺手 upsert ⇒ 没跑过那天就没语料 | **建真实回溯通道**：eastmoney `news_page`（单页，收口 `get_news`/`search_news` 两份 110 行重复实现）+ `news_pages_until_asof`（`sort:"time"` 倒序翻页裁到截止日，50×12 上限）；`get_news_with_asof` / `search_news_with_asof` / `policy_news_impl(asof)` 三条；路由层顺序 = **vendor 回溯 → news_archive → 一条汇总降级**；翻不到截止日报 `Err`（不静默返空） | 实测接口：`sort:"time"` 页 1=09-24→09-01、页 2=09-21→09-01、页 4=08-28→08-25（严格降序，通道成立）；`sort:"default"` 打乱（证明旧路径不可能裁对）；`pageSize=50` 生效；`clip_keeps_only_items_on_or_before_cutoff` + `jsonp_parser_handles_flat_and_nested_shapes` |
| T3 | `search_stock` 在回放里返空 + 「as-of 模式搜索不可用」 | 名称↔代码是**映射**、无时间语义，旧分支却按「当下语义」直接返空 ⇒ LLM 误判「该股票不存在」 | 快照退为快路径，miss 则穿透 live vendor 链；删除该条降级 | `search_stock_resolves_mapping_in_replay`（断言穿透 + 不得再申报降级） |
| T4 | 降级面板 6+ 条同义噪音（「Fallthrough vendor 不支持 as-of 参数,跳过」×N）；研报 0 条与「源报错」写同一句话 | 9 个方法各抄一份逐源循环：每源各留一条降级 + `if let Ok(..)` 吞掉 `Err` | 抽 `AStockClient::asof_probe`（唯一形态：按能力白名单遍历、失败收进 `name=reason`、由调用方汇总**一条**）改造 9 处；`truncate_klines_by_asof` 的正常截断不再记降级（只 `debug`） | `probe_folds_uncapable_vendors_into_one_entry`（折叠成一条 + 计数）+ `probe_surfaces_vendor_error_instead_of_swallowing_it`（原因必须是「反爬拦截」而非「均未提供」） |

### 10.1 刻意留下的两处「软泄露」口径（与 P5 同源，非缺陷）

- `get_concept_blocks` / `get_hot_stocks` / `get_industry_ranking`：源申报 `Fallthrough`
  ⇒ 回放里容忍 live 值（板块归属/榜单变化以月计，且这些源没有日期参数可传）。
  `asof_probe` 的 `caps` 白名单把这件事写成了显式参数，不再是散在各函数里的隐式约定。
- `get_policy_news` 回溯里的行业名来自 `get_sector_info`（同样是慢变量映射）。

### 10.2 尚未收口（下一轮候选）

- `get_money_flow` 的 as-of 分支仍是**手写**逐源循环（`if let Ok(Some(..))` 吞 `Err`），
  只是它不再逐源记降级所以没进 T4 的 9 处清单；换成 `asof_probe` 是纯收益。
- `get_institutional_visits` / `get_consensus_eps` 的 vendor 侧真实回溯参数仍未验证
  （申报 `Fallthrough`，路由层直接判空）。
- P6（news_archive 历史回填工具）按原计划独立立项。

## 11. 三轮 T5/T6（2026-09-26，源自 300642 回放面板逐条实测，commit 576d2ca8c）

| # | 面板原文 | 实测归属 | 处置 |
|---|---|---|---|
| T5 | `get_pledge_data: as-of 模式无 300642 当日质押快照（该维度无历史语义）` | **假话**。中登 `RPT_CSDC_LIST` 有 `TRADE_DATE` 列：实测 `filter=(SECUCODE="300642.SZ")(TRADE_DATE<='2026-06-30')` ⇒ 返回 2026-06-26 那期（周报口径，取截止日前最近披露） | 新增 eastmoney `get_pledge_data_with_asof` + 申报 `NativeDateParam`；路由顺序 = 快照 → vendor 回溯 → 一条汇总；URL/解析收口成 `pledge_report_url` + `fetch_pledge`（live 与回放共用一份口径） |
| T6a | `get_hot_stocks: … 1 个有 as-of 通道的源均明确回答无数据` | 文案说谎：`Fallthrough` 源被算成「有 as-of 通道」，实际只是容忍它给当下值 | `AsofProbe::reason` 改措辞「被探测的源（含无通道 N 个）」 |
| T6b | `get_index_quotes … browser_eastmoney=… tencent=…` | **外部拦截，两种不同原因**：腾讯 `web.ifzq.gtimg.cn` 返回 `waf.tencent.com/501page` 挑战页（HTTP 501，纯 HTTP 客户端过不去）；东财 `push2his` curl `http=000`（既有 IPv6/RST，代码里有代理回退） | `tencent_get` 显式命名 WAF 拦截这一类，避免 300 字 HTML 混进「JSON 解析失败」；指数在回放里能否拿到 ⇒ 取决于是否配 `EASTMONEY_PROXY`（产品/环境项，未定） |
| — | `get_margin_data: 300642 无融资融券披露数据（非两融标的…）` | **实话**：同报表查 600519 有 09-22/23/24 三期，查 300642 返回 `code 9201 返回数据为空` | 不改 |
| — | `search_news / get_money_flow … 均未申报 as-of 能力` | 与已提交代码矛盾（crate 测试断言 eastmoney 对二者均为 `NativeDateParam`）⇒ 出提示的进程比代码旧 | **重启应用重跑回放**再判 |

### 11.1 仍待裁定（不是遗漏）

- `get_option_pcr`：个股期权 PCR 无公开接口（eastmoney 对非 ETF 代码直接 `Ok(None)`），
  文案「无 as-of 历史通道」易被读成「配置缺失」。要么该维度不进回放清单，要么改文案。
- `get_peers`：同行对比的涨跌幅/PE 是当下快照，接口没有日期参数。要支持得**用 K 线
  为每只同行业票合成截止日行情**（工作量 ≈ 指数 as-of 那条），要不要做属产品口径。

### 11.2 接口语法坑（写判据时踩到，别再踩）

- datacenter-web 的 filter 日期必须写成 `'YYYY-MM-DD'`（带单引号）；
  写成 `20260630` ⇒ `filter字段中日期参数格式错误`，写成 `to_date(...)` ⇒ `9501 参数预处理错误`。
- 东财搜索接口的 JSONP 在 `sort:"time"` 形态下返回**扁平数组**且日期字段是 `date`
  （不是 `showTime`）⇒ 解析必须保留字段别名链，否则整页被当成「日期不可解析」丢掉。

## 12. T7（2026-09-26）：7×24 快讯其实能跳日回溯 —— 旧结论「不支持指定日期」是错的

面板原文：`get_cls_flash: as-of 模式快讯不可用：无当日快照，3 个源无 as-of 通道
（7×24 接口按游标翻页、不支持指定日期）`。前半句（快照）没错，后半句是**未穷尽实测**下的推断。

实测（`np-listapi.eastmoney.com/comm/web/getFastNewsList`）：

| 传的 `sortEnd` | 返回条目时间 | 结论 |
|---|---|---|
| `"2026-09-18 15:00:00"`（日期串） | 2026-09-26 20:43（当下最新） | 日期串**被忽略**（旧注释把它写成日期参数，是错的） |
| `1789714800000000` = 该时刻 Unix 秒 ×**1e6** | 2026-09-18 14:58:47 起 | 真游标 = 秒 ×1e6（与条目 `realSort` 同族），**可跳日** |
| 同左 +1h / +4h | 15:58:43 / 18:57:03 | 线性对应 ⇒ 游标语义确定 |
| `…×1e9`（19 位，位数错） | 回到当下最新 | 越界**静默回落** ⇒ 必须校验量级，否则会误以为拿到了那天 |

由此 `get_cls_flash_with_asof` 只需 **1 页**即得「当日尾部 20 条」（实测游标=09-22 23:59:59(CST)
⇒ 返回 23:48:13→23:20:26 共 20 条，全部落在 09-22）；翻页上限 4 页只防极忙日。
落地：eastmoney 申报 `NativeDateParam`、路由改走 `asof_probe`；**当日无条目报 `Err` 不返空**
（空会被下游读成「那天风平浪静」）。

判据变化：`get_cls_flash_returns_empty_in_asof_scope`（前提已错）改写为
`get_cls_flash_in_asof_never_leaks_later_days` —— 不变量从「必须为空」升级为
「取到的必须不晚于截止日」；另加 `flash_asof_tests`（游标量级 / 逐日单调 / URL 原样带游标 /
字段别名链）与 `pledge_asof_url` 两条。

顺带把 `get_option_pcr` 的降级文案写实：个股 PCR 的真原因是「仅 50/300 等 ETF 期权有公开数据，
个股通常不适用」，不是「路由没配通」——旧文案会让人去找一个不存在的开关。

### 12.1 本轮后面板仍会剩什么（提前定性，避免重复排查）

- `get_margin_data`（非两融标的）、`get_research_reports`（该票窗口内确实 0 条）⇒ 实话，不改。
- `get_hot_stocks` / `get_industry_ranking` / `get_concept_blocks` ⇒ 当下榜单语义，
  只能靠当日快照；`asof_probe` 的文案已区分「无通道源」与「被探测源回答无数据」。
- `get_peers` ⇒ 待裁定：要么接受回放里没有，要么用 K 线为每只同行业票合成截止日行情。

## 13. T8（2026-09-26，重启后面板）：组合关键词翻不动 + 百度接口已迁移

重启重跑后面板只剩 5 条，逐条查到底的结果：

1. `search_news(keyword="透景生命 政策")` → `时间回溯 12 页(共 600 条)未覆盖到截止日 2026-09-22`。
   根因不是机制：**`sort:"time"` 下服务端不做相关性排序**，组合关键词等于在全量流里倒页 ——
   实测其第 1 页 50 条全是 09-26 当天，12 页只退到 09-24；同一天用纯名「透景生命」则
   1 页就覆盖 09-21→09-25（含截止日）。⇒ 回放路径新增 `asof_news_keyword`（复用
   `clean_search_keyword` 取最长中文片段，并列时保留前一段），live 行为不变；
   Err 文案带上「实际检索关键词」。
   顺带证伪一条侥幸：往 `cmsArticleWebOld` 塞 `startDate/endDate` **不生效**
   （endDate=09-22 仍返回 09-25 条目）⇒ 别再找日期参数，只有翻页这一条路。
2. `get_hot_stocks` / `get_industry_ranking` → 「被探测的源均回答无数据」。
   实测该源 `gushitong.baidu.com/opendata` 已 **301 迁到 `finance.baidu.com`**，
   新信封 `{"status":1,"msg":"参数错误","data":[]}` 根本没有 `Result` ⇒
   「接口整体失效」被各方法的 `Result.*` 读法翻译成「没有数据」。
   ⇒ `fetch_json` 增加 `baidu_envelope_error` 判定（无 `Result` 且 `status!=0` 才算失效，
   旧信封与 `status=0` 一律放行），把服务端 `msg` 原样抛成 `Err`；
   顺带修 `AsofProbe::reason` 的自相矛盾措辞（「1 个被探测…其中无通道源 1 个」
   —— 两个计数互斥，改为「被探测的 N 源…；另有 M 源无 as-of 能力」）。
   ⚠ 遗留：百度这两个接口要**重接参数**才能恢复，属独立一项（不是 as-of 问题，live 同样坏）。
3. `get_margin_data`（非两融标的）= 实话；`get_peers` = 待裁定（见 §12.1）。

## 14. T9（2026-09-26）：同行对比的截止日口径 —— 不需要 K 线合成

裁定选项原本写的是「用 K 线为每只同业股合成截止日行情」，实测后发现**成本更低且更准的路**：
live 的 `get_peers` 估值字段（PE/PB/市值/涨跌幅）来自 `RPT_VALUEANALYSIS_DET`，
该报表**本就按股票返回多日历史**（代码里就是「按 code 取 TRADE_DATE 最大的一行」）。
⇒ 只要给批量查询加 `(TRADE_DATE<='截止日')`，「每只 ≤ 截止日的最新一行」正是回放口径。

实测对照（`SECURITY_CODE in (600519,000001)`，pageSize=6）：

| filter | 首行 |
|---|---|
| 仅 `in (...)` | 09-24（pe 5.0458 / 18.989） |
| 加 `TRADE_DATE<='2026-09-22'` | **09-22**（pe 5.2289 / 19.246），`CHANGE_RATE` 即当日涨跌幅 |

编码坑（同一个坑今天踩第二次）：filter 里的**括号必须字面出现**，整体 `encodeURIComponent`
会把 `(` `)` 变成 `%28`/`%29` ⇒ 服务端 ANTLR 报 `参数预处理错误`；`<=` 与单引号才需要编码
（`%3C%3D` / `%27`）。

口径边界（如实承认）：**同业名单沿用当日成分** —— `RPT_F10_CORETHEME_BOARDTYPE` 没有日期列，
板块归属是慢变量；这一条只落 `tracing::info`，不记降级（数据面已满足）。

其余改动：`get_peers` 主体改 `peers_impl(stock_code, cutoff)`（live 与回放共用一份），
eastmoney 申报 `get_peers => NativeDateParam`，路由层仍走 T4 的 `asof_probe`。

### 14.1 待办（用户已裁定「下一轮再接」）

- 百度 `gushitong.baidu.com/opendata` → `finance.baidu.com` 迁移后的参数重接
  （现只做到「如实抛 status/msg」，该源在 live 也拿不到 hot_stocks / industry_ranking）。
- P6 news_archive 历史回填工具（独立立项）。

## 15. T10（2026-09-27）：行业分类口径自相矛盾 + 降级文案点名

面板（688072 回放）两条同类条目给的结论相反：

| 方法 | 文案 | 实际 |
|---|---|---|
| `get_concept_blocks` | 放行 `Fallthrough`，容忍 live 值 | 拿到板块归属 |
| `get_sector_info` | 「5 个源均未申报 as-of 能力」 | **拿不到行业名** |

两者数据源同一张表（`RPT_F10_CORETHEME_BOARDTYPE`，无日期列），归属是慢变量。
⇒ 统一口径：`get_sector_info` 白名单补 `Fallthrough`，命中 `NativeDateParam` 才走
`*_with_asof`，否则走 live 取值。**判据**：`asof_realtime_degrade_tests` 里行业类
方法在回放中不再记降级。

另一处诚实化：`AsofProbe` 除计数外记下**源名**（`skipped` / `probed_names`）。
只有数字时，同一方法在不同启用集下数出的「3 个源」「5 个源」无法区分是路由变了
还是源被禁；文案改为「…（名字, 名字）」，用户能直接判断该去启用哪个源。

## 16. T11（2026-09-27）：重跑分析时头部「实时 / 历史回放」不同步

缺陷：重跑刻意沿用原记录的 `as_of_date`（后端同日覆盖、便于工作流迭代），但那段逻辑
只写 `stockAnalysisStore` 自己的 `mode/asOfDate`；`PageTimeAnchor` 读的是**全局**
`useTimeAnchorStore` ⇒ 「分析按 as-of 取数，页面顶部仍显示实时分析」，
反向（重跑 live 记录而全局停在 replay）同样错。

修法：抽出 `syncTimeAnchorForRun(anchorMode, asOfDate)` —— as-of 本轮 ⇒
`enterReplay(同日)`；live 本轮 ⇒ `enterLive()`；`backtest_sweep` 由扫描工作台持有模式，
不覆盖。导出的唯一理由是让判据不必跑通整条工作流。
**判据**：`stockAnalysisStore.test.ts` 末尾 `syncTimeAnchorForRun` 三条。

## 17. T12（2026-09-27）：概念板块的申报倒挂 + 新闻倒序索引的窗口天花板

面板（688072 回放，重启后）三条互相矛盾，逐条实测：

| 条目 | 实测结论 |
|---|---|
| `get_concept_blocks`「baidu_stock 取数失败」 | **申报倒挂**：eastmoney 用 `RPT_F10_CORETHEME_BOARDTYPE`（无日期列）答归属，却申报 `NoHistoricalSemantic` ⇒ 被 as-of 白名单跳过，只去探测已 301 失效的 baidu_stock，整维拿空；而同源同表的 `get_sector_info` 在 T10 后容忍当日值 |
| `search_news`「12 页(600 条)未覆盖截止日 09-11」 | **不是预算不够，是接口天花板**：`sort:"time"` 页 1–20 各 50 条、真降序（09-27 → **09-11**），**页 21 起退化**为固定 40 条且日期回跳（09-15→09-11）——那是「相关结果」尾巴，继续翻只拿重复内容 |
| `get_industry_ranking`「1 源答无数据（browser_eastmoney）；另有 1 源无通道」 | 如实：榜单是那天的产物，容忍当日值没意义；`eastmoney` 被跳过属**刻意**（`NoHistoricalSemantic` 只留给这类真·当日维度） |

做法：

- `get_concept_blocks` 申报改 `Fallthrough`（与 `get_sector_info` 同档），命中时 `tracing::info`
  留「归属取自当日」，**不记降级**（T9 同行对比同口径）。
- `news_pages_until_asof`：`MAX_PAGES` 12 → **20**（贴实测窗口边），并把翻页记账抽成
  `AsofPagingAudit`，让「退不到截止日」的两种根因分开说 ——
  翻到**空页** = 该词全量就这么些（换词 / 靠 `news_archive` 积累）；
  打到 20 页 = 超出索引窗口。两种文案都带**实际退到的日期**，否则判断不了差多少。
- `AsofProbe::reason` 的**取数失败**分支也点名被跳过的源（此前只有「均答无数据」分支点名），
  否则「baidu_stock 失败」看起来像整条链只有 baidu。

判据：`asof_capability_tests`（eastmoney 三档申报）、`paging_window_tests`（两种根因文案分开）、
`asof_boundary_tests::probe_names_sources_in_every_failure_wording`。

### 17.1 遗留（本轮刻意不做）

- `get_industry_ranking` 若要在回放里有真值，只能用**板块指数 K 线合成**当日排名
  （约 86 个行业板块 × 1 次 K 线请求）。成本与 `get_peers` 的 K 线合成同族但重得多，
  待裁定；现状是诚实降级。

## 18. T13（2026-09-27）：面板仍报的 5 条，逐条到底是哪一种「拿不到」

用户反馈「并不见有改善」。重启后 688072 面板剩 5 条，逐条实测/查表，结论分三种性质：

| 条目 | 性质 | 依据 |
|---|---|---|
| `get_money_flow`「无 as-of 通道，且无当日快照」 | **文案在说谎**（真缺陷） | eastmoney 申报 `NativeDateParam`，但该站是 T4 漏改的手写循环：`if let Ok(Some(..))` 吞掉 `Err` ⇒ 失败被写成机制不存在 |
| `get_north_bound_holding`（本轮未列，同型） | **文案在说谎**（真缺陷） | 同上另一处；且个股级北向持仓 2024-08 起停披，属结构性 |
| `get_industry_ranking`「1 源答无数据（browser_eastmoney）；另有 1 源无通道」 | 结构性 | 榜单是「那一天」的产物，容忍当日值=泄露未来 ⇒ `eastmoney` 被跳过是**刻意**；只剩内核源被探测而它抓的是当下页面 |
| `get_hot_stocks`「baidu_stock 取数失败；另有 1 源无通道」 | 真故障，但不是 as-of 的锅 | 唯一容忍当日值的源 = 已 301 失效的百度接口 ⇒ 任务 #19 |
| `get_social_sentiment` / `get_option_pcr` | 结构性不适用 | 舆情只有当日快照；个股无场内期权 |

### 18.1 已做（commit `21dc8d181`）

1. `get_money_flow` / `get_north_bound_holding` 的 as-of 分支改走 `asof_probe`：
   白名单只放 `NativeDateParam`（资金流的 live 值就是今日净流入，绝不进回放），
   文案由 `probe.reason()` 生成 ⇒ 失败会写成「被探测的源取数失败 [eastmoney=真实原因]」。
2. **给资金流补了真通道**：日频 `fflow/daykline` 只在 `push2his`，而本机对它是连接级拒绝
   （2026-08-01 与 2026-09-27 两次实测；今天 `curl (56) server closed abruptly`、
   node `socket hang up`，同域 `kline` 一起挂）⇒ 直连源申报了也白报。
   `browser_eastmoney`（webview 内核，带真实 TLS 指纹）补 `get_money_flow_with_asof`
   并申报 `NativeDateParam`，与直连**共用** `fflow_daykline_url` / `parse_fflow_klines` /
   `select_fflow_window` / `money_flow_from_window` —— 兜底只换通道，不换口径。

判据（`asof_boundary_tests`）：命中通道 ⇒ 不留降级且 `history` 不越截止日；
失败 ⇒ 点名源，且断言文案里**不再出现**「无 as-of 通道」。

### 18.2 待裁定：面板缺「严重度分档」，诚实降级与真故障同色

现在 5 条提示同一种视觉权重 ⇒ 用户只能读出「还是没好」。三个候选（可多选）：

- **A. 降级条目分三档**：`不适用`（个股期权）/ `窗口到顶`（新闻索引、当日榜单语义）/
  `真故障`（接口 301、连接被拒）—— 只有最后一档标红。改 `DegradationEntry` 加 `kind` 即可，
  取数侧已把三种语义写成不同句子，差一个分类。
- **B. 合成历史行业排名**：用各行业板块指数的 K 线取 `<= 截止日` 最后一条涨跌幅排序
  ⇒ 榜单类维度获得真 as-of 语义。成本约 86 次请求/日，按日缓存到 `daily_snapshot`。
- **C. 接 #19 百度榜单重接**：解掉 `get_hot_stocks` 那条真故障（live 同样受益）。

## 19. 下一步三项已裁定（2026-09-27，用户多选：A+B+C 全做）

面板的读法问题先于取数问题：降级条目只有「一条字符串」这一种形态，
所以「个股没有场内期权」和「接口 301 挂了」长得一样。三项按此顺序做：

### T14 面板按严重度分档（先做）

- **harness**（权威 DTO 所在地）：`DegradationEntry` 加 `kind`，
  `enum DegradationKind { Failure, NoData, StructuralGap }`
  —— `Failure`=真故障（源报错/连接被拒/接口失效）、`NoData`=取数面到顶或明确回答无数据、
  `StructuralGap`=该维度对回放本就不适用（个股期权、当日榜单语义、快照唯一通道）。
  序列化 camelCase，`as_of.rs` 的环形缓冲与 `clear/count/log` 三个命令透传。
- **谁定档**：`AsofProbe` 自己推得出（`probed==0` ⇒ StructuralGap；`failed` 非空 ⇒ Failure；
  否则 NoData）⇒ 14 处 probe 站点用 `probe.kind()`；probe 之外的（news_archive 合并文案、
  舆情/PCR 这类天然不适用）用 `record_degradation_with_kind` 显式给。
  `record_degradation` 旧签名保留并默认 `Failure` —— **宁可多标红一档，不可把真故障洗白**。
- **前端**：`timeAnchorStore.degradationLog` 元素类型加 `kind`；`PageTimeAnchor` 的
  Tooltip 按档分组（真故障在前、计数只统计真故障），三档标签进 11 语言。

### T15 合成历史行业排名

`get_industry_ranking` 在回放里改走「行业板块指数 K 线 → 取 `<= 截止日` 最后一条涨跌幅 → 排序」，
申报 `SynthesizeFromKline`。⚠ 未决风险：板块指数 K 线也在 `push2his`（本机连接级拒绝，§18 实测），
⇒ 先用 `daily_snapshot` 已有的当日板块榜做「数据自证」探测，确认哪条域拿得到历史，再定实现路径。
成本约 86 请求/日，按日缓存。

### T16 百度榜单接口重接（= 原任务 #19）

`gushitong.baidu.com/opendata` → `finance.baidu.com` 的新参数重接，解掉
`get_hot_stocks` 那条真故障（live 同样受益）。信封判据 `envelope_tests` 已在位。

## 20. T14 / T15 / T16 交付（2026-09-27）

| 提交 | 内容 | 门禁 |
|---|---|---|
| `47f7a59db` | **T14 面板分档**：`DegradationEntry.kind` = failure / noData / structuralGap（缺省 failure，宁可多标红），`AsofProbe::kind()` 自动定档 19 处，非 probe 站点显式定档；前端 `summarizeDegradations` + Tooltip 分节 + Tag 取最重一档；三档标签进 11 语言 | tsc / oxlint 0-0 / vitest 456 / harness+astock+analysis-engine 三 crate clippy 绿、测试 500+401+765 |
| `5fe35a197` | **T15 行业排名合成**：见下 | astock-data clippy 绿、`--lib` 406 通过 |
| `fb621749e` | **T16 阶段一**：识别百度风控信封 | clippy 绿、407 通过 |

### 20.1 T15 的实测依据（决定了实现路径）

`data.eastmoney.com/dataapi/bkzj/getbkzj` **忽略一切日期参数**（带 `date=2026-09-11`
与不带参数返回逐字一致，煤炭 `f3` 恒 66），但它返回的 `f13.f12`（`90.BK0437`）
正是板块指数 secid；而板块指数日 K 接口有**原生** `end=YYYYMMDD` + `lmt`
⇒ 名单取当日（慢变量，同 T9 口径）、数值按截止日取，31 个二级行业板块并发 8。

三个不能省的细节：
1. 本地仍按 `cutoff` 再裁一次（`end` 不总被尊重是 T1 的教训）；
2. 裁完不足两根 ⇒ **跳过该板块**，不编 0%；
3. 路由层白名单从 `[NativeDateParam, Fallthrough]` 收紧为 `[NativeDateParam]` ——
   原来的 Fallthrough 后门等于把「今天的榜单」放进回放。

顺带拆掉一条会在单测里真打 32 次网络的旧判据（其前提「replay 必须为空」已被推翻），
换成替身判据 `industry_ranking_in_asof_never_falls_back_to_today_list`
+ 夹具驱动的合成判据（零网络覆盖排序/跳过/越界不采信）。

### 20.2 T16 的现实结论：**不是「重接参数」能解决的**

浏览器抓 `finance.baidu.com/rank/all` 的 XHR，查清迁移目的地是**另一组接口**：

- `finance.pae.baidu.com/vapi/v1/hotrank?tn=wisexmlnew&dsp=iphone&product=stock&market=all&type=hour&day=YYYYMMDD&hour=N&pn&rn`
  —— **热股榜带 `day`/`hour`**，若可用就是 as-of 真通道；
- `sapi/v1/ranks?bizType=stock_rank&market=ab`、`vapi/v2/blocks?typeCode=HY&sortKey=amount`、
  `vapi/v1/blocks/overview?hasTrend=1`。

但实测（2026-09-27）：旧 `resource_id` 参数集在新域名一律 `status=1 参数错误`；
上述新端点对**未签名客户端**回 `{"Result":{"code":403,"isCaptchaEnabled":true,"msg":"hit risk"}}`，
连在 `finance.baidu.com` 页面里发同源 `fetch` 也一样 403，只有站点自己带签名的 XHR 是 200
（对照实验：当日与过去日的 URL **同样** 403 ⇒ 403 与 `day` 取值无关，是签名/风控）。

⇒ `baidu_stock`（reqwest 通道）要恢复，必须先逆出它的签名头；`day=` 能否取历史**仍未证实**。
本阶段先把归因修对：风控信封带 `Result` 字段，旧判据「有没有 Result」会放它过去，
于是「被风控」又会被翻译成「该维度没有数据」（本 workstream 反复踩的那类谎）。

**待裁定的岔路**：① 逆签名（不确定耗时，可能被风控升级打断）② 给百度也做一条内核通道
（照 T13/T15 的 `browser_eastmoney` 先例，新增 `browser_baidu` vendor）
③ 榜单类改找东财等价源（股吧人气榜另有 API，未测日期参数）④ 保持诚实降级，
并把 `baidu_stock` 从 `hot_stocks` 链里摘掉，免得每次回放白打一次必 403 的请求。

## 21. T16 的结论被实测推翻（commit `a1b8c61c3`）：内核通道救不了百度

给页面**成功**的 XHR 挂探针（`XMLHttpRequest.prototype.open/send/setRequestHeader` +
点击「A股热门板块」触发一次真实请求），抓到它比我手工 fetch 多出的两个头：

```
Accept: application/vnd.finance-web.v1+json
Acs-Token: 1790424005698_1790450941498_G0+HUJipb1hG…（前端 JS 现算的签名串）
```

对照实验（同一页面、同一会话）：

| 请求 | 结果 |
|---|---|
| 站点自己的 XHR（hotrank 当日） | **200** |
| 我在页面里发同源 `fetch`（不带 Acs-Token），当日 | **403 hit risk** |
| 同上，过去日 `day=20260911&hour=15` | **403 hit risk** |

⇒ 403 与 `day` 无关，**不是**「历史日期被禁」，是缺签名 ⇒ §20.2 里设想的
「给百度也开 webview 内核通道」**不成立**：签名由站点的请求层现算，不是浏览器环境自带。
`day=` 能否取历史因此**仍然未知**，除非先逆出 Acs-Token 的算法（下一档成本，不确定能成）。

面板侧已改为如实说：`新接口在 finance.pae.baidu.com 且要求前端签名的 Acs-Token 头，非浏览器请求层拿不到`。

### 21.1 收口（commit `7e79f882b`，用户裁定「摘掉必 403 的源」）

落地前先看链上还剩谁 —— **光摘 baidu 不够**：`neodata` 也没申报能力（走 trait 默认
`Fallthrough`），照样被放行并白打一次（无凭据必 `TOKEN_MISSING`）。故该维度按 T15 同一论证收口：

1. as-of 白名单 `[NativeDateParam, Fallthrough]` → 只放 `[NativeDateParam]`；
2. `routing.hot_stocks` 摘掉 `baidu_stock`（同 2026-08-01 对 industry_ranking 的处理先例）。

判据升级：热股榜在回放里留 **一条 StructuralGap（灰档）**，文案「N 个源均未申报 as-of 能力」，
且链上没有任何源被真正探测（零白打请求）。

### 22. 面板现状（重启后重跑 688072 应看到的）

- 消失：`get_concept_blocks`（T12）、`search_news`（T12 窗口提到 20 页）、
  `get_money_flow`（T13 补内核日 K 通道；若那台机器 push2his 仍被拒，会变成点名原因的红档而不是假灰档）、
  `get_industry_ranking`（T15 合成）。
- 转为灰档「回放不适用」：`get_hot_stocks`（#19）、`get_social_sentiment`、`get_option_pcr`、
  非两融标的的 `get_margin_data`。
- 仍是红档才是真该修的：源报错/连接被拒/签名失效（T14 之后一眼可分）。

## 23. T17（2026-09-27）：内核通道救不了 push2his —— 实测与换源

用户问「为什么还要代理，不是有 browser_eastmoney 吗」。用仓库自带的 Playwright 按 app
**完全相同**的启动方式复测（真 Edge headless、`--disable-ipv6`、同一套反检测参数）：

| 目标 | 结果 |
|---|---|
| `page.goto` push2his 日 K | `net::ERR_EMPTY_RESPONSE` |
| `page.goto` push2his 资金流 daykline | `Navigation ... interrupted`（JSON 响应另一类失败） |
| `page.goto` push2his 根路径 | `net::ERR_HTTP_RESPONSE_CODE_FAILURE` |
| node/curl 打 92.push2his | socket hang up |
| 同时刻 data.eastmoney / datacenter-web / search-api / push2 当日榜 | 全部 200 |

⇒ `browser_eastmoney` 绕的是 TLS 指纹与 CORS，绕不了「主机连上了但不回包」。
代理只是「换个地方终止连接」的旁路，不是内核通道的替代品 —— 我之前的表述把这两件事混在一起了。

### 23.1 换源（已做，commit `5ca25d52c`）

`sina` 的 `MoneyFlow.ssl_qsfx_zjlrqs` 实测是按日历史（`sort=opendate&asc=0`，09-24 起连续），
且不在被墙的域上 ⇒ 给它实现 `get_money_flow_with_asof` 并申报 `NativeDateParam`，
裁窗与装配**复用** eastmoney 的 `select_fflow_window` / `money_flow_from_window`
⇒ 两条源同一形态同一口径，路由层白名单不动（T13 收紧后的 `[NativeDateParam]` 正好放行）。

### 23.2 顺带修掉的谎：`None` ≠ `0.0`

`MoneyFlow` / `MoneyFlowDaily` 的超大/大/中/小单四档由裸 `f64` 改 `Option<f64>`：
tencent 只有主力与散户两档却写 `0.0`、baidu 解析失败 `unwrap_or(0.0)` ⇒
下游把「该源不披露」读成「这一档净额为零」。四档加 `#[serde(default)]`，
改造前的快照缺键也不会整条反序列化失败。

### 23.3 仍开放

- **行业排名**：板块指数日 K 同样只在 push2his（`92.` 子域、tencent `fqkline` 回 501 WAF、
  猜的 datacenter 报表名不存在）⇒ 需要再找一个未被墙的板块历史源，或用户侧配代理。
  在该源找到前，`get_industry_ranking` 的红档是实话，不是文案问题。
- 代理可用时两条会一起活（同一域名），届时要复测确认。

## 24. T19（2026-09-27）：「另有 0 源无 as-of 能力」

T18 生效后 301302 面板：`get_pledge_data` 正确落到「取数窗口到顶」档，
但文案出现 `被探测的 1 源均回答无数据（eastmoney）；另有 0 源无 as-of 能力` ——
计数为 0 时那一节不该存在（一句「还有 0 个」既占篇幅又让人怀疑看错数字）。
`AsofProbe::reason` 的「均回答无数据」分支改成条件拼接，判据补在
`probe_names_sources_in_every_failure_wording` 的第 ④ 条。

> ⚠ 提交 `b24350e54` 的 message 是错的（复用了 T18 那条质押文案）。**已核实它已在
> `origin/master` 上**（`git branch -r --contains b24350e54` 命中 origin/master）⇒
> 不 amend、不 force-push（禁区：改写已发布历史）。正确映射以此处为准：
> **T18（质押空答案）= `fc564a443`**，**T19（0 计数整节不出现）= `b24350e54`**（内容见
> `AsofProbe::reason` 的 `tail` 分支 + 该测试第 ④ 条断言）。

## 25. T20（2026-09-28）：`reason()` 五种形态共用一个分隔符

「未探测」那一支曾写成 `{subject} 无 as-of 历史通道：…`（空格 + 第二个冒号），其余分支都是
`{subject}：…`。面板四行摆在一起时，**冒号的位置**就是「压根没有通道」与「有通道但那天没拿到」
的分界线 —— 它一歪，两类语义看起来像同一类（与 `feedback-structural-gap-no-ui-ambiguity`
同源）。修法：该支统一成 `{subject}：无 as-of 历史通道，…`，判据补在
`probe_names_sources_in_every_failure_wording` 第 ⑤ 条（五种形态逐一断言
`starts_with("{subject}：")`，新增分支漏接时该测试变红）。
