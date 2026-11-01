# PLAN — 窗口涨幅达标漏检核查（Mover Recall）

> 状态：**已批、实施中**（Phase 0/2/3/5 完成；Phase 1 已落地且实机跑通；Phase 4 部分完成、4-3 按里程碑等待——逐阶段证据见 §5，等待期与数据现状见 §6）。
> 裁定来源：2026-09-30 用户「裁定项按照建议实施就可以了」（六条按本 PLAN 建议值固化），以及「接 package.json + CI」（门禁接线，§5）。
> 命名边界：本需求**不是**「涨停漏检」。事件判据是绝对涨幅，不含板块涨停语义 ⇒ 面板、错误码与**对外文案**一律用「窗口涨幅达标」，禁止出现「涨停」字样（名大于内容即为歧义）。**例外（不属该禁则的扫描面）**：本 PLAN 对用户原文的引述、§1/§3 对历史判决与旧实现的复述为保真保留原词；门禁 l 只覆盖呈现代码与用户可见文案。

## 0. 需求原文与收缩后的口径

原文：在最近四个周期区间（超短/短/中/长）内分别出现超过「单日涨停 / 20% / 30% / 40%」的股票，检查是否被对应时间档位的智能荐股 / 趋势智选推荐；未推荐者分析原因并优化。

收缩后（用户裁定「去掉涨停判断，硬编码为 10%」）：

- 超短档阈值 = **10%**（原「单日涨停」），短/中/长 = 20% / 30% / 40%。
- 阈值**不分板块**：主板涨 10% 与创业板涨 10% 都算事件（不再用 `get_price_limit_pct` 判事件）。
- **报告分组依据（已按实现回写）**：分组走 `detect_market_type()`（`src-tauri/crates/harness/src/market_data.rs:227-246`，取值 `main_sh` / `main_sz` / `chinext` / `bj` / `star` / `neeq` / `b_share` / `hk` / `us`），落在 `market_stock_universe.market_type` 列上，由引擎逐票读出（`crates/analysis-engine/src/mover_recall.rs:551`）。原句称「`get_price_limit_pct`（`src-tauri/crates/harness/src/market_data.rs:251`）只用于报告分组」**与实现不符**——本路径下该函数**根本未被调用**（既不用它判事件，也不用它分组）；分组目的不变：主板涨 10% ≈ 封板常买不进，创科涨 10% ≈ 寻常波动，混池会把优化方向带偏。

## 1. 承重事实（已实证，含对本会话早前错误陈述的纠正）

| # | 事实 | 出处 |
|---|---|---|
| F1 | 同花顺涨停池被塞在 `get_hot_stocks` 里，URL 硬编码 `page=1&limit=20` ⇒ 输入集合被排序截断；涨停字段（连板/封板时间/炸板/封单）在 `HotStock` 类型里全部丢失 | `src-tauri/crates/astock-data/src/vendors/ths.rs:280-339`、`src-tauri/crates/astock-data/src/types.rs:295-302` |
| F2 | as-of 下 `get_hot_stocks` 申报 `NoHistoricalSemantic`，白名单只放 `NativeDateParam`（Fallthrough 后门已按 #19 裁定关掉），miss ⇒ `record_degradation` + 返空 | `src-tauri/crates/astock-data/src/lib.rs:4401-4440` |
| F3 | `run_daily_snapshot_sweep` 的个股遍历用的是 `watchlist_items`（**自选股**），量级为用户自选、**非全市场** ⇒「每日快照已在遍历全市场个股」不成立（该误判的纠正记录见 §5「纠错记录」） | `src-tauri/src/commands/stock_analysis.rs:4224-4258`（`watchlist_codes` 取自 `watchlist_items::Entity::find()`） |
| F4 | 全市场清单接口 `push2his clist/get` 已失效，无现成「全市场当日涨幅表」通道 | `vendors/eastmoney.rs:503 / 2455 / 3201 / 3280` |
| F5 | 唯一可达的全市场枚举候选是板块成分枚举：`get_concept_board_members(board_code)`（live 可用；as-of 直接 `StructuralGap` 返空）。其上游板块清单依赖 `get_industry_ranking`，而 push2* 系本机有 RST 历史、部分 vendor 已摘除 | `src-tauri/crates/astock-data/src/lib.rs:4660-4700`、`src-tauri/crates/astock-data/src/lib.rs:319-323`、`src-tauri/crates/astock-data/src/vendors/eastmoney.rs:702,882,913-931` |
| F6 | 每日快照落盘是 **JSON DiskCache + LRU 10_000 条**，历史上就发生过 K 线条目把快照挤掉导致「哪天有哪天没」的事故 ⇒ 本需求的窗口聚合**不能**建在这层上 | `src-tauri/crates/astock-data/src/daily_snapshot.rs:1-30` |
| F7 | `reco_picks` 只有出票者；`seed_pool_json`（候选池快照）与 `strategy_weights_json`（权重快照）逐行留存；`decision_validations` 的行**只由被推荐 pick 生成** ⇒ 「入池未出票」的分数与否决原因**当前零留痕** | `crates/entities/src/reco_picks.rs`、`crates/entities/src/decision_validations.rs:3,31`、`src/commands/backtest_validation.rs:129-142` |
| F8 | 趋势智选是两条链、`style` 一名两写（工作流链 `serenity` / 荐股链 `bottleneck`），档位集合恒 {mid, long}；工作流链不消费闭环权重 | `recommender/style_matrix.rs:56-59,129-131`、`commands/stock_workflow/serenity.rs`（无 `served_vars` 消费） |
| F9 | 荐股反思闭环已是 `reco_ic_gate=shadow`（照算不进分），A/B 判据未跑；`capital_flow_*` 键被 `splitn(2,'_')` 错拆后静默丢弃仍未修 | `seed_variables.rs:768-780`、`stock_analysis.rs:4495-4500`、`recommender/mod.rs:196` |
| F10 | **建表形态已变**：迁移清单清空为 `&[]`，表结构由声明式 reconcile 引擎（L1 实体 + L2 `extras.rs`）对账产出；唯一性语义只能由实体列属性承接，丢了会「不报错不告警」地建出一张允许重复值的表 | `crates/dao/src/migrations/mod.rs:112,167`、`reconcile/expected.rs:1-35,741-747` |

**原「未实证」两条（Phase 0 要出结论的）—— ✅ 均已闭合**（2026-09-30）：① 全市场代码表能否稳定枚举并落库 → 正门改判为腾讯批量行情（`qt.gtimg.cn/q=`，74 请求 ≈42s 覆盖 5921 只、零失败）；② 全市场逐票日涨幅落库的单日成本 → 见 `AUDIT-mover-universe-feasibility-2026-09-30.md` 的实测表。结论正文见 §2 Phase 0。

## 2. 阶段划分

### Phase 0 — 可行性演练 ✅ 已完成（2026-09-30，结论见 `AUDIT-mover-universe-feasibility-2026-09-30.md`）

实测结论（原「未实证」两条已闭合）：

1. **全市场代码表/当日涨幅的正门 = 腾讯批量行情 `qt.gtimg.cn/q=`，80 码/请求 ⇒ 74 请求 ≈42s 覆盖 5921 只，实测零失败**；涨幅字段 f[32] 与东财 `clist` f3 逐只互校 |Δ|=0.0，口径可互认。
2. **东财 `push2 clist`（`board.rs:90`）判废为每日主路**：`pz` 服务端硬上限 100 ⇒ 全市场固定 60 页，连续爬取实测 **11/60 成功**，失败后 `push2`/`17.push2`/`82.push2`/`push2delay` 与 curl/urllib 一律连接被断（按 IP/指纹阻断，非按域名），冷却 ≥5.5min 未恢复。
3. **板块枚举路线判废**：`search_concept_boards` 是按关键词搜索、拿不到板块全集；`bkzj` 只回默认 31 条。
4. **窗口累计涨幅不需要逐票 K 线**：日涨幅逐日落库后，累计 = `∏(1+r_i)−1`、最大单日 = `max r_i`，纯本地 SQL 聚合。K 线通道只在**历史回填**时用（实测 `web.ifzq.gtimg.cn` 单票 ≈0.53s、60/60 成功 ⇒ 全市场串行≈52min、8并发≈7min，回填可行但不在主路径）。
5. 降级链定为 `qt.gtimg.cn` → `hq.sinajs.cn`（实测 10/10 可用，但 600 只中 17 只缺昨收）→ 东财 clist 单页。批量大小/间隔/退避必须做成可调（写死 80 = 把今天的观测当永久契约）。

⇒ **判据「覆盖 ≥90% 全 A 且单日采集在 cron 预算内」满足，走方案一**。唯一遗留观测污染：本次探测自身触发了封禁窗，「安全突发量」未测出（不影响结论方向）。

### Phase 1 — 事件地基：全市场日收盘价表

- 新表 `market_daily_close`（**实际落库形状已回写**，真源 `crates/entities/src/market_daily_close.rs`）：`id`（主键 = `"{stock_code}@{trade_date}"` ⇒ 天然去重，重复采集走 upsert 覆盖）、`stock_code`、`stock_name`、`trade_date`、`close`、`prev_close`、`change_pct`、`collected_at`、`source`。原设计的 `market_type` 列**不在本表**（已归到 `market_stock_universe.market_type`）；`(stock_code, trade_date)` 的唯一性由字符串主键承接，**未**另建唯一索引。选列理由：`stock_name` 避免回查清单、`collected_at` 区分「哪天采的」与「哪天的数据」、`source` 记来源（`tencent_batch` / `sina_batch` / `kline_backfill`）。
- **建表正门 = 声明式 reconcile，不是写 migration**（实改：`crates/dao/src/migrations/mod.rs:112` 的 `const MIGRATIONS: &[Migration] = &[]` 已清空，P6 删过 74 条迁移；`crates/dao/src/migrations/mod.rs:167` 明文告诫「往 MIGRATIONS 加回任何一条之前先读它的文档」）。⇒ 落点：① 在 `crates/entities/src/` 加 SeaORM entity 并在 `lib.rs` 注册；② 唯一性/索引靠列属性承接（`#[sea_orm(unique)]` / `#[sea_orm(indexed)]`，索引名恒为 `idx-{表}-{列}`，判据见 `crates/dao/src/reconcile/expected.rs:741-747`）；③ 需要 CHECK / 生成列等实体表达不了的叠加，写进 `reconcile/extras.rs`（L2 声明认不到表就报错，不许静默跳过）。**AGENTS.md 禁区 10 的字面「必须写 migration」在本仓的现行形态是这样落地的，不要照字面去加迁移条目。**
- **采集通道（按 Phase 0 实测定）**：主路腾讯批量 `qt.gtimg.cn/q=`（多码逗号拼接，80 码/请求，74 请求覆盖 5921 只，实测零失败）；降级 `hq.sinajs.cn` → 东财 clist 单页。复用既有件，不新写口径：
  - 代码前缀转换复用 `to_tencent_code()`（`vendors/tencent.rs:293`），**禁止**另写一份 sh/sz/bj 拼接（探测脚本里那份是临时的，不进生产）；
  - 响应解析复用 `get_quote` 的 `~` 分字段逻辑 ⇒ 先把它抽成「按行解析」的私有函数，批量与单票共用（禁区 12：不得出现第二套 f[3]/f[4]/f[32] 映射）；GBK 解码沿用 `encoding_rs::GBK`；
  - 涨幅口径已与东财 `clist` f3 互校 |Δ|=0.0，两源可互为降级而不产生两套数。
- 批量大小 / 请求间隔 / 退避参数走模板变量或 `AStockClient` 配置，**不写死 80**（今天的观测不是永久契约）。
- **停牌与缺值**：该票该日取不到现价或昨收 ⇒ **不落行**，绝不写 `change_pct = 0.0`（写 0.0 等于把「拿不到」伪装成「没涨」，直接污染事件表分母）。本轮未采到的票数必须显式计数并上报。
- 采集器：新增 `market_daily_close_sweep(db, date)`，**复用** `daily_snapshot::snapshot_target_date` 的「最近一个已完成交易日」归属日判据（`stock_analysis.rs:4204` 已示范：盘中触发不得按收盘值落盘）；采集必须在 live 语义下跑，沿用 `as_of::enter_global_asof(None)` 的 RAII 压栈写法。
- **调度形态（已按实现回写）**：实现**新开了一个采集调度器** `start_market_close_sweep`——注册于 `src-tauri/src/init/services.rs:98`，函数定义在同文件 `:3697`。原句「挂到既有每日自动化 cron，不新开调度器」已被实现取代：采集要「分 tick 摊薄扩清单」（每 tick 6 页 × 10 轮页号滚动），与既有每日一次性 cron 的形态不同，硬塞进旧注册反而会把封禁窗风险引到别的任务上。
- **落库层选 DB 表而非 DiskCache**（理由见 F6）：窗口聚合要 SQL，且不能被 LRU 挤。引擎按 PG/SQLite 双方言走 `build(Dialect::…)`，PLAN 不锁定单一引擎。
- 历史边界：核查能力起点 = 本表首个有数据交易日；早于此的日期**显式返回「未采集，不可核查」**。可选的一次性历史回填走 `web.ifzq.gtimg.cn` 日 K（实测单票 ≈0.53s、60/60 成功，全市场串行≈52min），回填完成后必须与批量口径抽样互校才允许进事件表。

### Phase 2 — 事件计算 + 反查 + 归因

- `mover_events` 表：`(trade_date_anchor, period, stock_code, max_gain_pct, window_gain_pct, threshold_pct, qualified)`。
- 判据：**累计涨幅达标为主判据**，同时记录**窗口内最大单日涨幅**作强度标签（两者都来自 `market_daily_close`，零额外请求）。
- **算法（Phase 0 定）**：窗口累计 = `∏(1 + r_i) − 1`（按该档 `default_holding_days` 根数复利连乘），最大单日 = `max r_i`。⇒ **主路径完全不碰逐票 K 线**，事件计算是纯本地 SQL/内存聚合，可在 as-of 回放里稳定复现。停牌日缺行 ⇒ 该票该窗口按「样本不足」处理，不得用 0 补。
- 阈值走**模板变量**（不进源码字面量）：`mover_gain_ultra_short=10` / `mover_gain_short=20` / `mover_gain_mid=30` / `mover_gain_long=40`，与 `default_holding_days`（2/5/28/90）**同表登记**，窗口天数**必须复用既有单源**，不得另写天数表（否则门禁 f 段判红）。新增变量 ⇒ bump `TEMPLATE_VERSION`。
- 反查：给定 `(stock_code, 事件窗口)` → 查 `reco_picks`（链标签按 F8 归一：`bottleneck`/`serenity` → `serenity`；id 带档位后缀）。
- 归因码（每层必须是机械可判的，不可判的不许出现在清单里）：
  - `miss_l0_timing` 推荐时点晚于事件起点 ⇒ 属「晚推荐」，不计入漏检分母；
  - `miss_l1_not_in_pool` 不在 `seed_pool_json`；
  - `miss_l1_pool_degraded` 池本身在本轮降级（沿 H 轮裁定：**降级判据必须绑定本轮**，不得用无条件样本混判）；
  - `miss_l2_by_design` 矩阵该格不出票（`style_matrix` 的 `absence_reason`）——趋势智选的超短/短两档恒属此类，**严禁进入待优化清单**，只出现在「按设计不覆盖」说明段；
  - `miss_l3_scored_out` 入池但分数未进 top-N（依赖 Phase 2 末的打分留痕）；
  - `miss_l4_gated_out` 风控/一致性/数据质量闸口否决；
  - `miss_l5_other_period` 该股在别的档位出了票；
  - `miss_unexplained` 兜底桶，必须显示占比；**占比过高即说明留痕不足，不得据此下优化结论**。
- **L3 打分留痕**（裁定要做的第 6 条）：新增 `reco_scan_audit` —— 每次扫描为池内每票落 `(scan_id, code, period, style, score, rank, outcome, reject_code)`。注意 `get_hot_stocks` 降级与 preseed 两条入口都要写；容量按 raw_seed_pool_size 实测决定，落库前先估行数成本。

### Phase 3 — 三个率（缺一不可）

1. **可及率** = 事件中曾进入候选池的比例（受 universe 与池层决定，不是算法能力）。
2. **覆盖率** = 池内事件中被评为推荐的比例。
3. **不可归因率** = `miss_unexplained` + 由 T 时点数据无信号可解释的比例。

第 3 个率决定「优化」的期望收益上限；报告若缺它，前两个率会诱导去追噪声。

### Phase 4 — 优化接入（保守，默认不生效）

- 漏检信号**只并入既有荐股闭环**，共用 `reco_ic_gate` 的 off / shadow / on 三态，**不新建第三条写权重通路**。
- **禁止用 mover 数据提权**（守住 Q1「只降不升」）。允许的写回：对「入池且被评分压出 top-N」的格做**降权**（负向）；对 `miss_l1_pool_degraded` 只修可达性，不动权重。
- 事前注册判据（写在本 PLAN，A/B 不过不转正）：as-of 回放 off vs on，须同时满足 ① 各档命中率不下降；② 池内事件覆盖率提升 ≥ 3 pp；③ 逐档 rank IC 不劣化。三条任缺即保持 shadow。
- **前置修复**：`recommender/mod.rs:196` 的 `splitn(2,'_')` 键空间 bug 必须先修（多词 style 键被静默丢弃），否则 `capital_flow` 一族的写回会无声失效。

### Phase 5 — 呈现 / i18n / 门禁

- 面板挂在荐股矩阵区（`RecoStrategyMatrix` 已有闭环视图旁），按档 × 按链 × 按板块分组；三种缺席各占一句，降级来源显式声明。
- 11 语言全覆盖（含新增归因码文案与错误码），照 `check-i18n-key-parity.mjs` 口径。
- 门禁扩展：`scripts/check-reco-horizon-parity.mjs` 新增一段 —— 涨幅阈值与窗口天数**单源**（模板变量名 + 默认值 + 逐档登记），带 `--selftest` 夹具；改契约前先跑 selftest。
- **登记项（2026-09-30 补登，源出 `crates/entities/src/market_stock_universe.rs:21-22` 指向本节的交叉引用）**：分组用的 `detect_market_type()` 按首字符分流，其中 `Some('9') => "b_share"`（`src-tauri/crates/harness/src/market_data.rs:243`）⇒ **北交所新代码段 `920xxx` 会被归入 `b_share`**，报告里「北交所」一组的计数因此偏低、「B股」一组虚高。当前**已知未修**（不在本轮范围）：该段代码出现后，不得据「北交所」分组计数下覆盖结论；修法应落在 `detect_market_type()` 的 `920` 前缀特判上，属独立改动。

## 3. 明确不做

1. 不打开 as-of 的 `NoHistoricalSemantic` Fallthrough 后门（#19 裁定：热榜是「那一天」的产物，容忍当下值 = 把未来信息塞进回放）。
2. 不用同花顺涨停池当输入（F1/F2）。
3. 不回填 `market_daily_close` 之前的历史，不做「用当下值冒充历史」的近似。
4. 不让漏检分析提权（§Phase 4）。
5. 不动 `reco_ic_gate` 的出厂 `shadow` 默认值。

## 4. 提交与协作约束

- 并发会话已有未提交改动落在 `stock_analysis.rs`、`init/services.rs`、`evolution_drift.rs`、`stock_workflow/reflection.rs`（见当日 `git status`）⇒ 本轮对这些文件的 hunk **只改不提交**，沿用混合树裁定，待那批落地后统一切分。新文件（entity / migration / engine 模块 / 面板组件）可独立成提交。
  > ⚠ **订正（2026-09-30 晚）**：这条约束**已履行完毕**——本轮改动已随 `928f8e641` 整体提交，现状与残留未提交项见 §5「提交状态」。
- 逐阶段完成不跑全量；阶段边界用定点 `cargo clippy -p <crate> --all-targets`；全部阶段完成后跑一次 CI 口径三件套（`fmt --all --check` / `clippy --workspace --all-targets --all-features -- -D warnings` / `__TAURI_WORKSPACE__=true cargo test --workspace`）。
- 内联 `#[cfg(test)] mod` 一律追加文件末尾（避 `items_after_test_module`）。

## 5. 实施状态（2026-09-30 追加）

| 阶段 | 状态 | 证据 |
|---|---|---|
| Phase 0 | ✅ 完成 | `AUDIT-mover-universe-feasibility-2026-09-30.md`；universe 主通道改判为腾讯批量 |
| Phase 1 | ✅ 已落地且**实机跑通**（原记「实机未跑」已过期，首采见 §6） | 新表 `market_stock_universe` / `market_daily_close`（`crates/entities/src/`，走声明式对账，未加 migration）；批量通道 `tencent::fetch_quotes_batch` + `AStockClient::market_snapshot_batch` / `market_list_page`（as-of 下显式 `StructuralGap` 返空）；采集与分 tick 扩清单在 `commands/market_close_sweep.rs`，由**新增调度器** `start_market_close_sweep` 驱动（注册于 `src-tauri/src/init/services.rs:98`、定义在同文件 `:3697`；每 tick 6 页 × 10 轮摊薄）；**四条命令**（`get_market_coverage` / `refresh_stock_universe` / `sweep_market_close` / `analyze_mover_recall`）已被 `build.rs` 自动登记进 `register_commands.rs:616-618,725`（+4 行） |
| Phase 2/3 | ✅ 纯函数 + 单测落地 | `crates/analysis-engine/src/mover_recall.rs`：`tier_rules_from_vars`（阈值读 `mover_gain_*` 变量、窗口天数取 `default_holding_days` 唯一天数来源）、`window_cum_gain_pct`（复利，空/NaN ⇒ None 不补 0）、`events_for_tier`（历史长度不足显式跳过）、`attribute`（层优先级机械判据）、`compute_rates`（三率；分母 0 ⇒ NaN，不显示 0%）、`matrix_active` / `period_has_active_style`（走 `reason_code_by_key` 含 serenity/bottleneck 别名）、`code_in_pool_snapshot`。命令层 `commands/mover_recall.rs::analyze_mover_recall`。`cargo test -p axagent-analysis-engine --lib mover_recall` = **10 passed / 0 failed**（2026-10-01 复跑；原记 9 条，后补 `nan_rates_serialize_as_null_not_error` 与 `pool_snapshot_accepts_pair_and_object_shapes` 等）；`cargo check -p axagent --all-targets` 无 error 无 unused |
| Phase 4 | ◐ **部分完成**（4-3 被数据阻塞） | **4-1 键空间 bug 已修**：新增 `parse_style_period_key`（逐风格试前缀 + 余段须整体解析成档位，与生成侧 `format!("{s}_{p}")` 严格往返），`capital_flow_*` 不再被静默丢弃，往返恒等（`style_period_keys_round_trip`）与多词截胡（`multiword_style_is_not_stolen_by_shorter_prefix`）两条单测定点锁住。**4-2 已接**：`MoverCellSignal`（Laplace 比率 `(hits+1)/(scored_out+hits+1)`，`scored_out == 0 或 证据数 < 3` ⇒ `None` 不罚）经 `reco_loop.rs::compute_loop_cell_weights` 以**乘数**并入 IC 线之后（同格叠加后 clamp 到 `[WEIGHT_FLOOR, BASELINE_WEIGHT]`，只降不升），`LoopCell.mover_penalty` / `scored_out` 进 DTO；`load_mover_cell_signal` 在 `stock_analysis.rs` 三处装配点（含闭环视图）接上，进 on 仍受 `reco_ic_gate` shadow 闸。**4-3 数据等待中（2026-09-30 拍板：按里程碑等待，见 §6）**：① **已解**——2026-09-30 22:39 重启后首采 `2026-09-30 → 46 行`（bootstrap=自选+已分析股，非全市场；clist 封禁未解）；空表守卫保留（「尚未采集过全市场收盘快照，核查不可运行（非『无大涨股』）」）；② **窗口累计需 ≥2/5/28/90 个采集日**（`Period::default_holding_days()`；现 1 日）⇒ 超短档最早国庆后首个采集日（约 10-08/09）解锁，短/中/长约 10-14 / 11-18 / 2027-02（见 §6）；到点后按第 2 节 Phase 4 三条预注册判据验收，出厂 `shadow` 不动。**等待期误报已修**：窗口未满时面板原冒称「本区间无达标事件」⇒ 逐档 `windowOpen` 标注 + `emptyNotYet` 分叉（后端 `collected_days` 单源，见 §6） |
| Phase 5 | ✅ 完成 | `MoverRecallPanel.tsx` 挂进 `RecoStrategyMatrix`（闭环视图同区，说明段与三种缺席各自成句）；11 语言 `stockAnalysis.moverRecall`（键数口径：**平铺标量 29 个**——已含 §6 的 `windowOpen` / `emptyNotYet` 与 5-补 的 `malformed`；另有嵌套对象 `layer`（8 键）与 `marketType`（10 键）未计入这 29）三门禁全绿；口径门新增 **k（阈值/天数单源）** 与 **l（用户可见文案不得出现「涨停」）**，两段各带正负控 selftest；`node scripts/check-reco-horizon-parity.mjs` = **12 段全 OK**、`--selftest` = **35 条全 PASS**（2026-09-30 实测复跑；本节下方接线段记的 35 条为准，此处原记 34 条为早期读数） |
| Phase 5-补 | ✅ 完成（全量 vitest 抓出） | **第四种态**：`invoke` 返回值形状不符（浏览器 mock default / 测试通用 `mockResolvedValue`）时，旧实现 `setData(r ?? null)` 把 `{}`/`[]` 当视图 ⇒ `data.rates` 崩掉整棵 `RecoStrategyMatrix` 子树（`horizonPresentation` 4 例 + `BacktestPage` 1 例红）。修法：`missingFields()` 形状守卫（4 标量 + 3 数组），缺字段 ⇒ 走 `failed` 支并点名缺哪些字段（新键 `stockAnalysis.moverRecall.malformed` × 11 语言）—— 既不静默空白、也不带崩调用树 |

**单测抓出的三处实现错（已改，记录以免重犯）**：① 我以为 `value/ultra_short` 是按设计不出票，实际它出票、只是登记了 MISFIT 档-因子错配 ⇒ 断言反过来才对；② `Evidence` 默认 `matrix_active=false`，用例没置真时被 ByDesign 抢先 ⇒ 归因优先级本身正确，是夹具不全；③ 四档变量只覆盖三个时，未配置的档吃**出厂缺省**而不是缺席，条数按 3 不按 2。

**CI 口径三件套与前端套件（2026-09-30 收尾实测）**：

| 门 | 结果 | 备注 |
|---|---|---|
| `cargo fmt --all --check` | ✅ 0 | 带 `--all`（不带只查根 package） |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ✅ exit 0 | 全 40 成员，9m24s |
| `cargo clippy -p axagent-analysis-engine -p axagent --all-targets --all-features -- -D warnings` | ✅ exit 0 | 覆盖 `events_for_tier` 死参数 `date_index` 清理后的代码 |
| `__TAURI_WORKSPACE__=true cargo test --workspace` | ✅ **EXIT=0**（2026-09-30 21:47） | **8010 passed / 0 failed / 41 ignored**；101 个 target + **41 段 doctest 全跑完**。上轮曾因 `win_sandbox::tests::network_block_status_is_truthful` 偶发红在 tools 处 fail-fast 中止（当轮 0 段 doctest）；该条隔离重跑 1 passed / 0.11s ⇒ 环境敏感 flaky（与并发 CPU 争用同族），本轮未复现 |
| `npm run typecheck` / `dprint check` / `oxlint src` | ✅ 0 | — |
| i18n 三件套 + `check:eol` | ✅ 全过 | key-parity 11 档 15368 条一致 |
| `npx vitest run`（188 文件 / 2101 例） | ✅ **全过（EXIT=0，2026-09-30 21:49 复跑）** | 上轮那条红（`StockAnalysisConfigPanel.test.tsx` 报 `后端已种子化但面板缺失: review_horizon, review_expected_holding_days, analysis_primary_horizon`）已由并发会话提交 `34f1cea48` 修复 —— 三名登记进 `runtimeInjected`（+6 行），与本轮记录的修法一致 |

**那条红的归因与闭环（已实证）**：三个变量由反思工作流**运行时注入**（`commands/stock_analysis_setup/mod.rs:1897-1919` 注释原文「运行时由 run_reflection_workflow 用盖章后的 primary_horizon 覆盖」），属该测试 `runtimeInjected` 白名单该收的族；引入提交是 `67dbef8b6`（并发会话同步提交，同批只动了 `seed_variables.rs` +14 与面板 2 行，未同步扩白名单）。**实证方式**：`git status` 显示 `mod.rs` / `StockAnalysisConfigPanel.tsx` / 该测试文件三者对 HEAD 均无改动，且用同一正则从 `git show HEAD:…mod.rs` 与工作树抽出的变量集合**逐名相同**（各 6 个，工作树无新增）⇒ 该红在 HEAD 上同样成立，与本轮 mover 改动无关。**闭环**：并发会话于 21:45 提交 `34f1cea48`「fix: v3 单档复盘三变量登记运行时注入（非面板参数）」，把三名并入 `runtimeInjected` 白名单（+6 行）——与本轮此前记录的修法（一行式、注明注入点）一致；21:49 复跑前端全量 188 文件 / 2101 例**全过**。

**收尾（2026-09-30 追加）**：`events_for_tier` 的死参数 `date_index`（第 544 行 `let _ = date_index;`，唯一作用是喂第 428 行构造）已删——签名、构造行与 4 处调用点（引擎生产 1 + 引擎单测 2 + 命令层 1）同步改净，`cargo fmt --all --check` 与定点 clippy 复验 0。

**纠错记录（本会话误判的订正，避免与承重事实表混排）**：① **F3 误判** —— 早前称「每日快照 sweep 已在遍历全市场个股、吞吐基线实测过」，实为 `watchlist_items`（自选股）遍历，量级非全市场；这条错判正是本需求的立项触发点，判据见 §1 F3。② **窗口未满误报** —— 面板原冒称「本区间无达标事件」，实为窗口未满，已由 `windowOpen` / `emptyNotYet` 分叉修掉（见 §5 Phase 4/5-补行与 §6）。

**门禁复核（2026-09-30 21:43–21:48 第二轮）**：① `cargo fmt --all --check` = **0**（含最后一笔事实门修复的 3 行）；② `cargo clippy -p axagent-harness --all-targets --all-features -- -D warnings` = **exit 0 / 0 error** —— 事实门修复只改 harness 两个文件的 evidence 字符串字面量，故以 harness 定点门复验（全量 clippy 此前已实测 exit 0，9m24s）；③ `check:facts:ci`（单源事实门）修掉 2 处硬拦客观错后 **EXIT=0**：`harness/src/domain_semantics.rs` 两处 evidence 行号（`analysis-engine/src/recommender/mod.rs` 的 649/650 → **1005/1006**，按引入提交 `0963ffaac` 内容比对定真源：`confidence` / `position_pct` 的 `combined).clamp` 赋值行，工作树唯一命中）与 `harness/src/workflow_port_axioms.rs` 一处（`stock_analysis_setup/seed_stock_analysis.rs` 的 3554/3500 → **4974/4921**，按 `output/backup-2026-09-14f/` 备份对照定惯例：主引用 = 边 `source_handle` 行、括注 = case label 行；当前工作树 `source_handle: Some("acceptable".into())` 在 4974、`label: "acceptable".into()` 在 4921）。改后 git diff 恰 3 行，事实门软判据 15→14（649 处的「指向收尾括号行」已消）。

**提交状态（2026-09-30 晚订正；原记「按混合树裁定未提交」已过期）**：本轮 mover 改动**已整体提交**为 `928f8e641 feat: 涨幅召回归因与全市场收盘快照采集（Phase 1/4）` —— 含 entities（`market_daily_close` / `market_stock_universe`）、命令（`market_close_sweep.rs` / `mover_recall.rs`）、引擎（`mover_recall.rs`）、面板与 `MoverRecallPanel.windowOpen.test.tsx`、11 语言 `moverRecall`、门禁接线（`package.json` / `.github/workflows/ci.yml` / `scripts/ci-check.mjs`）与事实门修复 3 行。原先「hunk 逐块确认 / 切分清单待裁定」的说法已随该提交落地而失效。并发会话亦于 21:45 以 `34f1cea48` 带走 `e2e/stock-reflection-four-horizon.spec.ts` 并修掉前述红。
**当时遗留、现已不成立**：`opc/learning.rs`、`trajectory/causal.rs`、`opc_capability_pack_runtime.rs`、`stock_analysis.rs`、`reflection.rs`、`evolution_drift.rs` 等并发文件各自的提交去向**不在本 PLAN 管辖范围**（当时把它们写进「本线提交状态」是口径混装，此处不再追认）。
**当前工作树残留（尚未在本 PLAN 登记，属 §6 等待期与采集器收尾的后续）**：改动 `crates/analysis-engine/src/lib.rs`、`src-tauri/src/commands/market_close_sweep.rs`、`src-tauri/src/init/services.rs`，未跟踪 `crates/analysis-engine/src/market_close_store.rs`。

**口径门接线（2026-09-30 22:00 完成，用户已批「接 package.json + CI」）**：

| 载体 | 改动 | 位置 |
|---|---|---|
| `package.json` | 新增 `check:reco-horizon` / `check:reco-horizon:selftest` 两条脚本（与 `check:horizon-parity` / `check:valuation-parity` 同族命名） | scripts 段 |
| `.github/workflows/ci.yml` | `frontend-check` job 新增 step「Check reco horizon parity (a-l twelve gates)」，两行 `run`：`--selftest` + 正式；注释声明与 `scripts/ci-check.mjs` 对应两条 step **命令行逐字同源** | valuation-parity step 之后（纯 Node 静态门同区） |
| `scripts/ci-check.mjs` | 对应两条 `step(...)`（同命令行），含「判据 #0：任何一门解析到 0 命中即 exit 2」与 selftest 负样本说明 | 估值缺省参数两条之后 |

**接线验证（已实测）**：`node --check scripts/ci-check.mjs` = OK；`--selftest` = **35 项全 PASS**（a–l 正负控 + 三段扫描面断言）；正式步 = **12 门全 OK / EXIT=0**；`npx dprint check package.json scripts/ci-check.mjs` = **EXIT=0**；`ci.yml` 经 `yaml` 包解析通过且新 step 的 run 内容与 ci-check.mjs 逐字一致；三文件 `\r` 计数 = 0（LF）。

## 6. Phase 4-3 等待期：里程碑与窗口未满语义修复（2026-09-30 用户拍板）

**数据现状（2026-09-30 晚）**：`market_daily_close` 首采 `2026-09-30 → 46 行`（bootstrap = 自选股 + 已分析股；clist 封禁未解 ⇒ **非全市场**）；采集 cron 随应用运行（每 tick 散页摊薄重试）。**窗口累计需 2/5/28/90 个采集日**（`Period::default_holding_days()` 唯一天数来源）⇒ 解锁时间表（以真实 K 线自证为准）：

| 档位 | 窗口 | 预计最早可判定 | 备注 |
|---|---|---|---|
| 超短 | 2 采集日 | 国庆后首个采集日（约 10-08/09） | 到点即先跑该档 A/B |
| 短 | 5 | 约 10-14 | |
| 中 | 28 | 约 11-18 | |
| 长 | 90 | 约 2027-02 | 全档通过才转正 |

用户拍板：**按里程碑等待**（不动口径、不破预注册判据；「不回填采集起点前历史」维持原裁定）。到点动作 = as-of 回放 `reco_ic_gate` off vs on，按第 2 节 Phase 4 三条预注册判据（各档命中率不降 / 池内事件覆盖率 +3pp / 逐档 rank IC 不劣化）验收；出厂 `shadow` 不动。A/B 机制已核对就绪：`recalc_reco_loop_weights(as_of_date)` + `MoverCellSignal` 参数化（off 臂 = 空信号）+ `reco_ic_gate` 三态闸。

**等待期误报修复（用户拍板「修：逐档标注」）**：窗口未满时面板原显示「本区间无达标事件」——把「尚不可判定」冒称成「没有」（违反「结构性缺口不得在 UI 造成歧义」）。修法：

- 后端 `MoverRecallView` 增 `collected_days`（= 只数到 `to` 的采集日数；某档可判定 ⇔ 本值 ≥ 该档 `window_days`）；
- 面板：未满档位逐档标注 `windowOpen`（「窗口未满（已采 {{have}} 个 / 需 {{need}} 个交易日）」）；0 事件文案**仅在全部档位已满**时用 `empty`（「本区间无达标事件」），否则用 `emptyNotYet`（「窗口未满：未满档位尚不可判定，已满档位无达标事件」）；
- i18n 11 语言 ×2 键；测试 `MoverRecallPanel.windowOpen.test.tsx` 4 例（全未满 / 部分满 / 全满 / 缺 `collectedDays` 走形状守卫点名）。

**全市场语义缺口（交底）**：`market_stock_universe` 现 46 只（本地票拼成）——事件的「全市场覆盖」依赖 clist 解封后的清单扩张（每小时 tick 重试中）；解封前 4-3 判据可在 46 只样本上跑（机制验证），但需求原语义（全市场大涨股核查）尚未达成。
