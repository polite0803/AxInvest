# PLAN：决策结论档收归单一权威 + 分档互斥

状态：**待批**（未开工）。落此文档于 2026-10-22。
用户裁定（原话）：「项目全局只能存在一处权威定义，且定义内部分档不能有重叠和语义模糊。」
适用范围：八档结论 —— 买入 / 增持 / 持有 / 观望 / 减持 / 卖出 / 不确定 / 数据缺失。

---

## 0. 现网读数（本轮实测，不是推断）

- `stock_analyses` 共 **283 行**（今日曾为 360 行，中途 77 行 `interrupted` 被应用侧删除；全仓只有两条用户触发的删除路径，无后台作业删行）。
- 档位分布：持有 105 / 观望 77 / 数据缺失 36 / 减持 21 / 卖出 16 / 买入 14 / **action 为 NULL 8** / 增持 6。库里 `decision_action` 实发**中文**，`Option<String>`，**无 CHECK 约束、列上无值域声明**。
- **自相矛盾组合 6 行（2.1%）**：`持有 + positionState=EMPTY` 3 行、`观望 + positionState=HOLDING` 3 行 —— 即「无仓位却持有」「有仓位却观望」。
- 值域散成五套互不相交清单（`harness/src/decision_action.rs:16-35` 文件头自己列的表），另有：
  - `src-tauri/config/llm-output-schemas.json` 6 档中文 enum **复制 5 处**（:108 / :321 / :387 / :453 / :519），且该文件**运行时无人读取**（只有门脚本读）。
  - `crates/analysis-engine/src/decision.rs:59-72` 的 6 档强度阶梯（含强烈买入、无观望）—— **全仓零调用者，死表**。
  - `crates/astock-data/src/scoring.rs:749` 的 `strong_buy` 第三套强度阶梯。
  - 提示词：`agency_experts/stock-analysis/trader.md` 6 档；`custom/value-investor.md:150,254` 是另一套（强烈买入/观望/减持/规避/中性）——「规避/中性」**在八档里没有对应档**，实际落「不确定」。
- 门：**0 道**。`scripts/` 下 grep `ActionKind|decision_action|ACTION_ALIASES` 零命中；最接近的 `check-horizon-decision-keys.mjs` 只锁键名不锁值域。
- 别名表两份不同集合：后端 `ACTION_ALIASES` 多 ASCII 形态 `strong_buy/add/watch/trim/clear/strong_sell`（前端不认 ⇒ 落「不确定」）；「清仓/回避/止损/割肉/离场/未知」只出现在第三份（`decision.rs:1710-1721`）。
- 消费端仍直接比字面量（改定义端会静默漏改的点）：`portfolio_formula.rs:742`、`recommender/gates.rs:96`、`reco_loop.rs:1053`、`strategies/watchlist.rs:291`（只认中文卖/减持）、`reflection_stats.rs:166-167`（自写四值表）。
- 兜底占业务档名两处（语义模糊的主源）：
  - `portfolio_formula.rs:486/499`：未识别 / 极高风险否决 → 无条件写「观望」。
  - 四路档位脚本：「无方向腿 / 先验不可得」→ 写「观望」（`portfolio-mgr-h-ultra-short.rhai:303-306`、`-short.rhai:215`、`-mid.rhai:248-251`、`-long.rhai:355-358`）；「看多但入场/时机门不过」封顶 → 写「持有」（`-mid.rhai:245`、`-long.rhai:344-349`）。`positionState` 只由 `position_pct` 派生、**不参与档名**（`portfolio-mgr.rhai:3357-3384`）。
  - `harness/dashboard_report.rs:302-304`：action 为空 → 补「持有」（与同文件 :251 自相矛盾，属「缺失伪装成结论」）。

## 1. 唯一权威落点与八档互斥判据

权威定义只此一处：`src-tauri/crates/harness/src/decision_action.rs` 的 `ActionKind`
（依据 AGENTS.md 规则 12：共享契约的权威定义在 `axagent-harness`）。其余载体一律**派生**或**被门比对**，不得自带清单。

判据按两轴的笛卡尔积定义，**互斥且穷尽**（任一记录只能落进一档）：

| 档 | 判据 | 存储 token | 中文 |
|---|---|---|---|
| 买入 | 有变动意图 ∧ 目标仓位变动幅度 ≥ +T_hi | BUY | 买入 |
| 增持 | 有变动意图 ∧ 0 < 变动幅度 < +T_hi | INCREASE | 增持 |
| 减持 | 有变动意图 ∧ -T_lo < 变动幅度 < 0 | REDUCE | 减持 |
| 卖出 | 有变动意图 ∧ 变动幅度 ≤ -T_lo | SELL | 卖出 |
| 持有 | 无变动意图 ∧ **当前有仓位** | HOLD | 持有 |
| 观望 | 无变动意图 ∧ **当前无仓位** | WAIT | 观望 |
| 不确定 | **有**决策数据但方向判不出（解析失败 / 证据不足 / 无方向腿） | UNCERTAIN | 不确定 |
| 数据缺失 | **无**决策数据（DB 空 / 取数链路失败） | UNAVAILABLE | 数据缺失 |

三条硬规矩（写进模块头，并作为门的判据）：
1. **兜底不得占用业务档名**：「判不出」一律 `不确定`，「没数据」一律 `数据缺失`；不得再写 `观望`/`持有`（这是今天 6 行矛盾记录与全部歧义的来源）。
2. **后置规则改写不得改档名语义**：被风控否决/封顶降级，只允许改「变动意图与幅度」，档名仍按上表两轴产出，改写来历走既有的 `actionSource` 字段（已有 i18n 与呈现层）。
3. **别名只登记在权威处**：输入侧别名（加仓/减仓/清仓/强烈买入/英文 token…）是「解析入口」不是「档位」，只允许存在于 harness 一张表里，前端与提示词不得各留一份。

## 2. 怎么做到「只有一处」——两条通路本仓都有先例

- **生成**（真单源）：`src-tauri/schema-gen` 已用 `ts-rs` 产 `src/types/generated/ipc.ts`、`events.ts`（`main.rs:34-60` 有 mode 分发）。缺的 4 个改造点：① `ActionKind` 补 `#[derive(ts_rs::TS)]` 并把 `as_cn`/别名表纳入生成面；② `schema-gen` 加 mode + 清单；③ 新增 npm script 并接进 `ci-check.mjs`/`ci.yml`（**现在 CI 完全不跑 schema-gen**）；④ 前端改 import 生成物、`llm-output-schemas.json` 的 enum 由生成产出（本仓无「JSON 由代码生成」先例，需新建）。
- **门比对**（近零成本、先做）：抄 `scripts/check-domain-single-source.mjs:146-156`（权威 rs + TS + prompt + i18n 五面对账）与 `check-horizon-label-parity.mjs:146-160`（`extractFnBodyIn` 从 Rust 函数体抽字面量）两个现成手法，新建一道门把五张载体钉在一起，带 `--selftest` 负控。

顺序：**先门后生成**。门先把现有漂移全部变成红（拿到清单和数量级），生成阶段再把前端/ schema 的手写清单删掉；门在生成后仍保留，用来锁「生成物没滞后」（`check-contracts.mjs:455-475` 的 G4 型双向差集就是现成模板）。

## 3. 退役清单（按裁定要求逐条点明「退役什么」）

| # | 退役对象 | 位置 | 处置 |
|---|---|---|---|
| R1 | 前端手写 `StockAction` 常量 + `STOCK_ACTION_LABELS` 中文别名表 | `src/lib/stock-analysis-utils.ts`（约 222 / 314-333 行） | 阶段 B 由生成物取代后**删除**，函数签名不变（消费面不动） |
| R2 | 死表 `map_posterior_to_action`（6 档强度阶梯，零调用者） | `crates/analysis-engine/src/decision.rs:59-72` | 删（连同 :1710-1721 那份第三套别名词表） |
| R3 | `dashboard_report` 空 action 补「持有」 | `crates/harness/src/dashboard_report.rs:302-304` | 撤，改判 `UNAVAILABLE`（缺失不是结论） |
| R4 | 兜底写「观望」 | `portfolio_formula.rs:486/499`；四路 `portfolio-mgr-h-*.rhai` 的无方向腿分支 | 改判 `UNCERTAIN`；降级/封顶改由 `actionSource` 表达 |
| R5 | 「无结论」这一格（我 2026-10-22 加的前端哨兵 `NO_CONCLUSION_ACTION`） | `AnalysisHistoryButton.tsx` + `stock-analysis-utils.ts` | **撤** —— 它与「数据缺失」重叠；改由列表命令下发 `UNAVAILABLE` 哨兵，弹窗那一格换成「数据缺失」标签 |
| R6 | 重复 i18n 键 `stockAnalysis.conditional.actionBuy/actionSell/actionReduce` | 11 份 locale（zh-CN:17018-17020） | 收编到顶层 `stockAnalysis.action*`；`ConditionalOrderPanel.tsx:241-243` 改引用。顺带消除「减持 vs 减仓」两值并存 |
| R7 | 已腐注释（称 dashboard 六档含「强烈买入」） | `decision_action.rs:21`、`harness/dashboard_report.rs:36`、`stock-analysis-utils.ts:874-878` | 改写为「实际产出 6 档，强烈买入经 `as_storage_cn` 折入买入」 |
| R8 | `llm-output-schemas.json` 里 5 份重复 enum | :108/:321/:387/:453/:519 | 收成 1 处（阶段 C 由生成写；阶段 A 先由门拦新增副本） |
| R9 | 消费端裸比字面量 | `portfolio_formula.rs:742`、`recommender/gates.rs:96`、`reco_loop.rs:1053`、`strategies/watchlist.rs:291`、`reflection_stats.rs:166-167` | 改 `normalize_action` + 匹配 `ActionKind`（这是「改定义端会静默漏改」的根源） |
| 附 | 9 个语言里未译的 `Buy/Increase/Hold/Reduce/Sell` | `src/i18n/locales/{de,fr,es,ru,ja,ko,ar,hi,zh-TW部分}` | 不是重复定义，但同档两处显示不一致（`conditional.*` 那套反而译了）⇒ 补译 |

**逐条状态（2026-10-22 第二轮核过，别再按上表领活）**：

| # | 状态 | 说明 |
|---|---|---|
| R1 | **未做，属阶段 B** | 生成物还没落，手写表仍在（现由门 R1/R10 逐档对账，不再自带清单） |
| R2 | **部分**，剩三张表的处置 = 待裁 **R-DA-6** | 本轮普查新发现同文件 `:76-98` 还有 **两张**：`map_signal_code_to_action`（`hold`→增持、`watch`→持有、`sell`→减持 —— 梯子整体错一格，且 `_ => "持有"` 占业务档名）、`map_evidence_action`。三者**全部零调用者**（后两张全项目零引用，只有 `src-tauri/output/backup-*` 副本里还有）；`map_posterior_to_action` 受 R-DA-3 载体约束 |
| R3 | **已做** | 补 `数据缺失`，含改测试期望 |
| R4 | **部分** | `portfolio_formula.rs` 的兜底已拆两档（空→数据缺失 / 非空不识别→不确定）；`decision.rs::opposite_direction_gated_action` 已改判「不确定」；四路 rhai 的无方向腿分支 = 待裁 R-DA-2 ①②③ |
| R5 | **已做** | 「无结论」格与 `historyFilter.noConclusion` 键 ×11 已撤；哨兵由 `list_stock_analyses` 读侧下发 |
| R6 | **已做** | 三键删除 ×11 语言，面板改引顶层；门 R8 把它当计数器（反豁免） |
| R7 | **已做** | 三处腐注释改写 |
| R8 | **未做，属阶段 C** | 五份 enum 仍各写各的（门 R5 已拦住「新增副本」与「与权威六档不一致」，`--dump` 实测五份值一致） |
| R9 | **已做** | Rust 五处改谓词；测试侧 `rhai_registry.rs:1723` 的中文三连同批改走 `implies_long` |
| 附 | **已做** | 8 语言 ×5 标签补译（门 R4 拦「仍是英文 token」） |

## 4. 分阶段与验收判据

| 阶段 | 内容 | 验收判据（每阶段一条可复核读数） |
|---|---|---|
| A1 | 新门 `scripts/check-decision-action-single-source.mjs`：五面对账（ActionKind ↔ 前端 ↔ schema enum ↔ rhai/提示词产出 ↔ i18n 键），注册进 `ci-check.mjs` 与 `ci.yml` 同位，带 `--selftest` 负控 | 首跑**必须报红**并列出现存全部漂移（预期命中 §0 那 9 条）；每条红要么修、要么进带失效条件的白名单 |
| A2 | 八档互斥判据 + 三条硬规矩写进 `decision_action.rs` 模块头；每档一条「判据唯一」单测（含 NaN/空串/未知值不入业务档） | `cargo test -p axagent-harness` 定点绿；撤任一判据能复现红 |
| A3 | R3/R4 兜底改判 + R5 哨兵统一（列表路径 NULL→UNAVAILABLE）+ 弹窗撤「无结论」格 | 现网 6 行矛盾组合的处置有读数（改后 `select` 复验）；历史弹窗「数据缺失」格条数 = 库里 NULL + 字面量「数据缺失」之和 |
| B | R1 生成通路：`ts-rs` derive + `schema-gen` mode + npm script + CI + 前端改 import + 删手写表；G4 型「生成物滞后」检查 | `cargo run -p schema-gen` 产物与前端引用一致；删掉手写表后 `tsc` 与全量 vitest 绿 |
| C | R8 schema enum 收口 + R9 消费端收编 + R6/R7/补译 | 门五面全绿且 `grep` 裸字面量比较归零（夹具/注释除外） |

**执行约束**：A2/A3/B/C 都含 Rust 改动 —— 保存 `src-tauri/` 会触发 `tauri dev` 重建重启并**杀掉正在跑的分析**（本仓实测拖过 36 分钟）。每阶段开工前先查在跑任务；主 crate 门禁是批次级（10–25 分钟、3.5GB×2），全量 `clippy --workspace` / `test --workspace` 只在最后跑一次。

## 5. 三项需要点的口径（我已给推荐，按推荐走就整批放行）

1. **持有 / 观望 按持仓状态拆**（§1 表第 5-6 行）：现网 6 行（2.1%）会换档名，新记录起按两轴产出。推荐：**做**（这是消除重叠的唯一办法，代价可量化且小）。
2. **极高风险否决后的落档**：现在无条件写「观望」。推荐：**有仓位 → 持有、无仓位 → 观望**，否决来历由 `actionSource=risk_veto_downgrade` 表达（不改风控实际动作，只改档名归属）。⚠ 这会改回测/胜率统计的分组口径，属可感知变化。
3. **`astock-data/scoring.rs:749` 的 `strong_buy`**：它是评分强度内部量还是对外结论？推荐：**留在评分层**，改名去掉 `buy` 歧义（如 `STRONG_BULLISH_SCORE`），并在门里登记为「非结论值域」。

不点这三项我就按推荐值执行，并在每阶段收尾时把实际读数写回本文件。

---

## 6. 执行状态（2026-10-22 批次，用户「批准，按推荐执行」后）

### 已完成
| 项 | 交付 | 读数 |
|---|---|---|
| A1 门 | `scripts/check-decision-action-single-source.mjs`（九判据 R0-R9 + `--selftest` + `--dump`），注册进 `scripts/ci-check.mjs` 与 `.github/workflows/ci.yml` 各两条（同位、逐字同源） | 首读红基线 **10 处**（R4×8 未译 / R6 别名缺 6 项 / R8 第二套键 1 处）；修完后**全绿**；九条负控各自有电 |
| A2 判据 | 两轴互斥表 + 三条硬规矩写进 `decision_action.rs` 模块头；新增三条单测（八档 token/中文一一对应、兜底两档不占业务档名、持有与观望按持仓状态分流） | `cargo test -p axagent-harness --lib` 817 passed；定点 9 passed（含三条新测）；`clippy -p axagent-harness --all-targets -D warnings` Finished 无告警 |
| A3 哨兵 | `fill_missing_with_placeholders` 空 action 由补「持有」改判 `数据缺失`（含改测试期望）；`apply_risk_veto` 未识别值域由降级「观望」改判 `不确定`；`query_stock_analysis_items` 对空 `decision_action` 下发 `ACTION_UNAVAILABLE`（`running` 行例外） | 前端配套撤掉自造的「无结论」格 + 删 `historyFilter.noConclusion` 键 ×11；两层测试改期望后 33 passed |
| R4/R6/R8/R7 | 8 语言 ×5 档补译（沿用 `conditional.*` 既有译法）；前端补 6 个 ASCII 别名 + 解析按小写再查一次（与后端 `to_lowercase` 同口径）；删 `conditional.actionBuy/Sell/Reduce` 三键并把 `ConditionalOrderPanel` 改引顶层；两处腐注释改写 | `tsc` 0 错、`oxlint src` 0/0、i18n 三关全绿（parity 11 语言一致）、`dprint check` 干净、locale 纯 JSON roundtrip 保真已验 |

| R-DA-4 前端半 | 新增缺失判定单一权威 `isAbsentDecisionAction`（与后端 `is_unavailable` 同语义），收编双视角面板三处手写 `!= null && !== "" && !== "null"` | 关键连带：列表/详情现在对空决策下发 `UNAVAILABLE`，旧判据会把「数据缺失」当成**有结论**渲染；新判据同时钉住「判不出 (`UNCERTAIN`) 不是缺席」 |

### 已完成（前端批读数）
- `tsc --noEmit` 0 错；`oxlint src` 0/0；vitest 全量 **2693 passed / 251 files**（含新增 4 条缺失判定用例）；`dprint check` 干净；值域门与 i18n 三关全绿。

### 已完成（R-DA-2 的 ④⑤ 两项，第二批 Rust）
| 项 | 改动 | 说明 |
|---|---|---|
| ⑤ 方向冲突兜底 | `stock_workflow/decision.rs::opposite_direction_gated_action` 压制目标由「观望」改「不确定」；幂等判据改走 `normalize_action`（中英同值域），且「数据缺失」不被压成「不确定」（两档不互串） | 方向互否的真实含义是判不出，不是「无仓位不加仓」的中性结论 |
| ④ 反思豁免 | `reflection-comparator.rhai:185` 与 `:295` 对「不确定」与「观望」同等豁免，文案改插值不再硬写「观望」 | 必须与 ⑤ 同批，否则「判不出」会被后验评价记成「方向误判」，污染胜率与归因 |
| 版本 | `TEMPLATE_VERSION` 173 → **174**（`reflection-comparator.rhai` 由 `stock_analysis_setup/mod.rs:1640` 的 `include_str!` 进种子） | 不 bump 则改动不落库（本仓版本门 `existing >= TEMPLATE_VERSION ⇒ 跳过` 的老坑） |
| 测试 | 该函数三条期望改判 + 新增两条（`SELL` 英文值域命中、`不确定`/`数据缺失` 幂等）；`观望` 的期望从 `None` 改成 `Some("不确定")`（它是可被改写的方向档） | 读数待回填：`cargo test -p axagent --lib` 与 `cargo test -p axagent-rt-workflow --test rhai_syntax_check` |

| 项 | 改动 | 说明 |
|---|---|---|
| 权威谓词补齐 | `ActionKind::implies_reduce()`（减持 / 卖出）—— 与既有 `implies_buy` 对称；新增单测断言两谓词**互斥**且英文 token / 别名经归一后同样命中 | 消费端从此不必各自 `matches!(action, "卖出" \| "减持")` |
| R-DA-4 五处收编 | `recommender/gates.rs:96`、`recommender/reco_loop.rs:1053`、`recommender/strategies/watchlist.rs:291`、`reflection_stats.rs:165-168`（自写四值表）、`portfolio_formula.rs` 仓位检查 | 四处**只认中文** ⇒ 链路上 action 为 `SELL` / `clear` / `强烈卖出` / `加仓` 时回环否决与丢档**静默不触发**（与模块头记的 fail-open 同一失效机制）；`reflection_stats` 那张表还让别名记录的 `was_correct` 恒 `None` ⇒ 样本被静默丢弃 |
| 读数 | **已取**：`cargo test -p axagent-analysis-engine --lib` → 正文 `823 passed; 0 failed`；`cargo test -p axagent-harness --lib` → `818 passed; 0 failed`（817→818 = 新增的谓词互斥单测）；`cargo clippy -p axagent-analysis-engine -p axagent-harness --all-targets -- -D warnings` → **退出码 0、零 error/warning 行**、31s | ⚠ 第一次那三个 `EXIT=0` 是管道末位 `tail` 的码不是 cargo 的（本仓记过的老坑，我自己又踩一次）；clippy 那次只留尾 12 行证据不足，已重跑不带管道的版本坐实。主 crate 因 harness 变更复检：`cargo check -p axagent --lib` → **`MAIN_CHECK_EXIT=0`（退出码由命令自己 append 进日志，无管道）**、8m16s、零 error/warning 行。 |



**R-DA-2 剩余 ①②③ 暂缓（不是遗漏）**：① 试探仓准入（`portfolio-mgr.rhai:2832`）与 ② 空头否决集合、③ 标注文案分支，都要先把 `effective_posterior` 阈值表与「观望」档的判定窗口对齐读全 —— ① 的窗口 `[0.42,0.50)` 与「观望」档共用同一概率量，改条件会**改变部分记录的仓位输出**，属可感知行为变化，必须配一次真跑分析对账，不能只靠编译与单测说绿。


### 与 PLAN 的偏差（逐条列名）
1. **撤回我自己的推荐 2**（原推荐「极高风险否决后：有仓位→持有」）。读码后不成立：`ActionKind::Hold.implies_long() == true`，把「禁止持仓」的否决结果写成「持有」既自相矛盾，又会被下一轮 `extreme && implies_long` 再次命中。该分支现输出**保持不变**，改判为待裁 **R-DA-1**。
2. 推荐 1（持有/观望拆轴）在**权威定义与判据层**已落地（表 + 单测 + 门），但**产端**未改（见 R-DA-2），所以现网那 6 行矛盾记录暂不变。
3. R2（删死表）不能照 PLAN 执行（见 R-DA-3）。

### 新增待裁 / 待办（追加编号）
- **R-DA-1** 风控否决后的落档：需先定「否决是否要求降低敞口」。若要降敞口，正确答案是减持/卖出方向而非观望或持有；若只是「本轮不动作」，则需给「无变动意图」再加一个不重叠的定义。
- **R-DA-2** 四路 `portfolio-mgr-h-*.rhai` 的「无方向腿 / 先验不可得」分支现写 `action = "观望"`（按裁定应为 `不确定`）。**普查后升级**：`"观望"` 在本仓是**五处承重**的字符串，换标签会逐处改变行为，不是改名：
  | 用处 | 位置 | 换标签后的影响 | 处置（已定，按裁定推） |
  |---|---|---|---|
  | ① 试探仓激活条件 | `portfolio-mgr.rhai:2832`（`probe_position_pct > 0.0 && base_action == "观望"`） | 降级记录不再拿到试探仓 ⇒ **仓位输出变化** | 激活条件改判「有正向意图但幅度不足」；`观望 ∧ position>0` 本身即两轴矛盾（现网 3 行 `观望+HOLDING` 的成因） |
  | ② 空头预测否决的输入档 | `:2991`（`final_action == "观望"` → 卖出） | 判不出的记录不再被空头证据否决 | 「不确定」并入可否决集合，覆盖面不变 |
  | ③ 标注文案分支 | `:3047` | 少一句「仓位修正」说明 | 同步加「不确定」 |
  | ④ 反思后验评价 | `reflection-comparator.rhai:185` 与 `:295`（`predicted_direction == "观望"` 算「保持观望」而非「方向错误」） | 降级记录会落进 `else` ⇒ **被判成「方向误判」**，污染胜率与反思 | 两处把「不确定」与「观望」同等豁免 |
  | ⑤ 方向冲突的降级兜底 | `stock_workflow/decision.rs:2565-2575`（`opposite_direction_gated_action` 把非观望动作一律改写成「观望」） | 这一处本身就是「兜底占业务档名」的违规源 | 改写目标改「不确定」，幂等判据同步 |
  ⚠ ④ 会改变**仓位输出与胜率口径**，属可感知变化，实施后必须真跑一次分析 + 一次反思对账（不能只靠编译与单测）。
- **R-DA-3** `map_posterior_to_action`（`analysis-engine/decision.rs:59`）确为**零调用者**，但它被 `harness/src/domain_semantics.rs:423` 的 `ConceptDecl`（`axinvest.decision.posterior`）当载体引用，evidence 直指该行 ⇒ 删除会破语义登记表门。先重定位该概念的载体，再删。

- **R-DA-4** PLAN R9 的裸字面量消费端 5 处未动（`portfolio_formula.rs:742`、`recommender/gates.rs:96`、`reco_loop.rs:1053`、`strategies/watchlist.rs:291`、`reflection_stats.rs:166-167`）。
- **阶段 B（ts-rs 生成通路）与 C（schema enum 收口）未开工。**

### 未取的读数（诚实项）
- ~~本批 Rust 改动的编译读数~~ **已取**：`cargo check -p axagent-analysis-engine --lib` → `AE_EXIT=0`（8m10s）、`cargo check -p axagent --lib` → `MAIN_EXIT=0`（8m17s），日志无 error / warning。
  （第一次命令写法错：`cargo check -p A --lib -p B --lib` 报 `--lib cannot be used multiple times`，而末尾 `echo "EXIT=$?"` 让后台通知显示 exit 0 —— 那次**根本没编译**，不能计入任何「绿」。已拆成两条重跑。）
- **已取**：`cargo test -p axagent --lib` → **729 passed / 0 failed**（`MAIN_TEST_EXIT=0`）；`cargo test -p axagent-analysis-engine --lib` → 首跑 **822 passed / 1 failed**，红的是 `risk_veto_unrecognized_value_no_longer_fails_open`（锁着旧期望「观望」）。改期望后复跑 **823 passed / 0 failed**。
  ⚠ 读那条测试时发现**我自己埋的模糊**：我把「空值（没有决策数据）」与「非空但不识别（判不出）」合并降级成同一档 `不确定`，违反同一条裁定。已拆成两档 —— 空 → `Unavailable`（数据缺失）、非空未识别 → `Uncertain`（不确定），并给测试补一条 `assert_ne!` 钉住两者不得共用兜底。
- **已取**：`cargo clippy -p axagent-analysis-engine --all-targets -- -D warnings`（13m05s，见上方 R-DA-4 表）。
- **已取（本轮，读数见 §7）**：`cargo fmt --all --check` 0、`cargo clippy --workspace --all-targets --all-features -- -D warnings` 0、
  `cargo test -p axagent --lib` 729/0、`cargo test -p axagent-analysis-engine --lib` 823/0、全量 vitest 2695/0。
- **已取**：`cargo test --workspace --lib` → **`WS_TEST_EXIT=0`**，42 个 crate 的 lib target 全 ok、
  合计 **8276 passed / 0 failed**（日志 `.workbuddy/tmp/gate-test-ws.log`，无 FAILED / failures / panicked 行）。
- **取数中**：`cargo test --workspace`（**全 target**，含 `crates/rt-workflow/tests/` 里那两条读 rhai 脚本的门）
  —— 这是 CI 第三条命令的字面口径，`.rs` 集成测试不在 `--lib` 覆盖面里。
  真机核对（弹窗档位标签与哨兵行为）归用户。



## 7. 漏检清单批（2026-10-22 第二轮，用户「照这个清单直接动手」）

我上一轮自查出 8 条漏检，用户批准按清单执行。逐条状态：

| 条 | 事项 | 处置 | 读数 |
|---|---|---|---|
| 1 | 别名语义散在消费端（回环否决 / 丢档判定只认中文） | 权威补 `implies_reduce`，五处改谓词（见 §6 R-DA-4） | `clippy --workspace --all-targets --all-features -D warnings` 见下方终局读数 |
| 2 | 前端第二份档位表 `STOCK_ACTION_CANONICAL_LABELS` 是门的盲区 | 门加 **R10**：该表逐档必须等于权威 `as_storage_cn` | R10 现网**绿**；负控「HOLD 改成坚守」有电 |
| 3 | 提示词面（`agency_experts/*.md`）不在扫描面上 | **R7 按文件类型分形态**：`.md` 查被分隔符包裹的档位词，代码查 `"档位"` 整串 | 扩面后首读 2 处，其中 `portfolio-mgr.rhai` 的「加仓」6 处**全在 `decision_trail` 模板串里（说明文字，不是档位）** ⇒ 判据改位置感知后剩 **1 处真红**（`value-investor.md`），已修 ⇒ 门全绿；11 条负控各自有电 |
| 4 | `list_stock_analyses` 读侧下发哨兵后，运行抽屉把 `UNAVAILABLE` 原样显示 | `StockAnalysisRunDrawer.tsx` 改走 `actionKeyOfRecord`（与列表 Tag 同一判据） | 该文件测试 4/4 通过；**其中一条旧断言 `getByText("BUY")` 钉的是缺陷本身，本轮翻转**（改断「买入」+ 断 `BUY`/`UNAVAILABLE` 不出现），另加两条（哨兵、库里中文） |
| 5 | `HistoricalAnalysisPanel` 自带一份 inline 档位判定 | **折进阶段 B**（与生成通路同批改，避免同一文件动两次） | 未动 |
| 6 | 门没有 npm 入口（只能靠 ci-check 触发） | `package.json` 补 `check:decision-action` / `:selftest` / `:dump` | `npm run check:decision-action` → OK 五面对账一致 |
| 7 | 库的 `decision_action` 值域无 CHECK / 运行守卫 | **折进阶段 B**（写侧守卫与生成通路一起定） | 未动 |
| 8 | 撤「无结论」格后要重跑 i18n 三关 | 已跑 | parity **16119 键 × 11 语言一致**；placeholder 门干净；`check_i18n.py --strict` 全通过；untranslated 0 新增 |

### 这一轮普查浮出的新面（原清单没有）

1. **分析师的 `verdict` 是方向轴，不是操作轴**。全仓 ~20 家专家统一用 `看多|偏多|中性|偏空|看空`，
   只有 `custom/value-investor.md` 自成一梯：`强烈买入 | 买入 | 观望 | 减持 | 规避 | 中性`（六档，
   其中「规避」「中性」不在八档值域内，`规避` 归一后落 `None`），且同文件既写「上表**五**档之一」
   又列六行 ⇒ 自相矛盾。本轮把它并回方向五档，表内条件与分数带逐行保留，
   并补一条**两行同给 `中性` 的消歧要求**（「质地好但贵」vs「没有可用数据」，后者 `confidence ≤ 20` 且 report 首句写明「数据不足」）。
   ⚠ 该 md 经 `mod.rs:134` 的 `include_str!` 进主 crate ⇒ **必须重编译**才进库；但 `seed_agency_experts` 自 v24 起是
   **无条件 UPSERT**，故**不需要** bump `TEMPLATE_VERSION`（与 §6 那两条 bump 的理由并存，别混）。
   `seed_consistency_tests::dcf_availability_signal_layers_agree` 靠 `【硬不可用】/【软衰减】` 切段，
   本轮实测两者在该 md 里**各恰好 1 次**（改动没碰分段契约）。
2. **又浮出三张第二套值域表，且全部零调用者**：`analysis-engine/src/decision.rs` 的
   `map_posterior_to_action`（=R-DA-3）、`map_signal_code_to_action`、`map_evidence_action`。
   中间那张是**整条梯子错一格**：`hold→增持`、`watch→持有`、`sell→减持`，且 `_ => "持有"` 直接违反
   本轮写进模块头的「兜底不得占业务档名」。后两张全项目零引用（只有 `src-tauri/output/backup-2026-09-03/` 里的副本）。
3. **这类面立不了门**（诚实项）：按「生产区（`#[cfg(test)]` 之前、剥注释）出现档位中文字面量的比较形态」实测
   **235 处**命中，绝大多数是说明散文、提示文案与测试期望，无法与真比较点机械分离。
   ⇒ 本轮只做**按形态人工普查**（`matches!` / `==` / `=>` 三形态，生产区命中 **0**），不新增一条会把 235 处全部塞进基线的门。

### 消费点普查（`list_stock_analyses` 的五个前端消费者，逐个结论）

| 消费点 | 是否会把哨兵原值给用户 | 依据 |
|---|---|---|
| `AnalysisHistoryButton`（历史弹窗） | 否 | `actionKeyOfRecord` → `getActionTKey` |
| `HistoricalAnalysisPanel`（回测视图列表） | 否 | `resolveDisplayAction` + `t(getActionTKey(action))` |
| `stockAnalysis.adapter` → `StockAnalysisRunDrawer` | **曾是**（本轮已修） | 原 `raw.decisionAction ?? "-"` |
| `WhatIfBacktest` | 不相关 | 该 DTO 的档位字段未被读取 |
| `stockAnalysisStore.listHistory` | 未见直接渲染 | 仓内无其它 `decisionAction` 消费点（全量 grep 已核） |

现网读数（本轮重取，`select … group by decision_action`）：共 **283** 行 —— 持有 105 / 观望 77 / 数据缺失 36 /
减持 21 / 卖出 16 / 买入 14 / 增持 6 / **空 8**。那 8 行 `status` **全是 `completed`** ⇒ 读侧哨兵正好覆盖它们
（显示「数据缺失」而非「观望」），**未回写库**：回写属数据迁移，需要单独裁定。

### 待裁追加

- **R-DA-5** 同一失效家族还剩两处「把后端档位原值插进译文」：`MoverRecallPanel.tsx:1058`
  （载体 `get_mover_recall_event_review`）与 `EvidenceCitationPanel.tsx:319`（载体 `audit_decision_evidence`）。
  两条都不在 `list_stock_analyses` 面上、本轮刻意未动。要不要把「呈现层档位只能走 `getActionTKey`」做成门（需先解决
  上面第 3 点的 235 处噪声），请点。
- **R-DA-6** `decision.rs` 那三张零调用者表：后两张可直接删（全项目零引用，已核）；
  `map_posterior_to_action` 受 R-DA-3 约束（`domain_semantics.rs:423` 的 `ConceptDecl` 拿它当载体）。
  删或留都要一次性决定，别留成「下次再查」。

### 终局门禁读数（本轮 Rust 面 = `rhai_registry.rs` 测试侧一处 + 被 `include_str!` 的 md）

- `cargo fmt --all --check` → **`FMT_EXIT=0`**（退出码由命令自己 append 进日志，无管道）。两轮各取一次，均 0。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` → 首轮 **`CLIPPY_EXIT=101`**，
  首红**是我自己的 hunk**：`src/commands/stock_analysis.rs:1297` 的
  `map_or(true, |s| ...)` 触 `unnecessary_map_or`（1.97 新增 lint）⇒ 改 `is_none_or`，并把同一段我新写的注释里
  三个 `文件:行号` 引用去掉（本仓纪律：自己新写的注释不引行号，行号会腐烂）。
  ⚠ clippy 在首个 error 短路 ⇒ 那一次 `axagent` 之后的 crate 从未被检查，不能据此推定其余干净。
- `__TAURI_WORKSPACE__=true cargo test -p axagent --lib stock_workflow` → **168 passed / 0 failed**（561 filtered）。
- **R-DA-6 自行裁定执行**：删掉 `decision.rs` 的两张零引用表（`map_signal_code_to_action` / `map_evidence_action`），
  并同步撤掉 `map_posterior_to_action` 文档里「同时提供与 scoring.rs map_signal 的桥接」那三行 ——
  桥接的实现就是被删的那张表，留着文档就成了假陈述。删除前后各取一次全仓引用普查：`.rs/.rhai/.ts/.tsx` 零命中
  （只有 `src-tauri/output/backup-2026-09-03/` 里的历史副本）。`map_posterior_to_action` **留下**（R-DA-3 载体约束）。
- 全量终局四项（原列「待回填」，现已全部取到）：
  **已取（改完 `is_none_or` 并删两张死表后重跑，日志 `.workbuddy/tmp/gate-rust2.log`，退出码逐条由命令自己 append）**：
  `FMT_EXIT=0`；`AE_TEST_EXIT=0`（**823 passed / 0 failed**，与删除前逐条同数 ⇒ 那两张表本来就没有测试覆盖，删除不可能「靠测试掩盖」）；
  `MAIN_TEST_EXIT=0`（**729 passed / 0 failed**）；
  **`CLIPPY_EXIT=0`**（`--workspace --all-targets --all-features -- -D warnings`，全量首次推进到底）。
- 那条**读被我改过的 md 的切片门**单独坐实它真的执行了（不是被 filter 掉）：
  `cargo test -p axagent --lib dcf_availability` → `running 1 test` ⇒
  `commands::stock_analysis_setup::seed_consistency_tests::dcf_availability_signal_layers_agree ... ok`
  （1 passed / 728 filtered out）；它靠 `【硬不可用】/【软衰减】` 两个标记切段，实测两者在该 md 里**各恰好 1 次**。
- 全量前端复跑：**251 文件 / 2695 用例全通过**（上轮 2693，+2 = 抽屉新增的两条）。
- 前端已取：`tsc --noEmit` 0 错；`oxlint src --max-warnings=0` **0/0**（1384 文件）；
  `vitest run`（TasksCenter + stock-analysis + lib 三个目录）**154 文件 / 1755 用例全通过**；
  `dprint check` 干净；`npm run check:eol` 两道规则全 PASS；门与 `--selftest`（11 负控）各再跑一次仍绿。

## 8. 追加轮（用户问「历史弹窗能否标出 live / as-of 来历」）

**按名目/执行/数据三层答的结论**（先答清再动手）：

| 层 | 结论 |
|---|---|
| 数据 | 判别子是 `stock_analyses.analysis_kind`（NOT NULL，两态 `live` / `replay`，写侧 `stock_workflow/core.rs` 由 `asof_ctx.is_some()` 定）。⚠ **不能拿 `as_of_date` 是否为空来判** —— 现网实测 283 行**全部非空**（197 live + 87 replay），那样会把每条都判成回放。 |
| 执行（后端） | `list_stock_analyses` 的 SQL 早已 `SELECT` 了 `AnalysisKind` 与 `AsOfDate`，DTO `StockAnalysisListItem`（`#[serde(rename_all = "camelCase")]`）也带这两列 ⇒ **后端不用改**。 |
| 呈现 | 弹窗本地的 `AnalysisRecord` 接口**没声明**这两个字段，也没渲染 ⇒ 数据在线上走但界面看不见。详情页 `DecisionBanner` 那个 ⏪ 用的是 `useTimeAnchorStore.anchorMode`（**查看者当前的时间锚**），与「这条记录当时怎么跑出来的」同词不同源，不能顶替。 |

**改了什么**（纯前端 + i18n）：

- `AnalysisHistoryButton.tsx` 的 `AnalysisRecord` 补 `analysisKind` / `asOfDate` 声明（含「同词不同源」与「as_of_date 全表非空」两条现网事实注释）。
- `stock-analysis-utils.ts` 新增 `provenanceOfRecord()` → `live | replay | unknown`（trim + 小写；**值域外的第三种值与缺失一律 unknown，不默认成 live**）。
- 行的徽标位（与「快速链」「图代」同一紧凑样式 `COMPACT_TAG_STYLE`）：live **不打标**，replay 打「历史回放」，
  且 `asOfDate !== analysisDate` 时打「历史回放 · 2026-10-20」—— 现网 87 条回放里 **37 条不同**（如 600825 分析日 10-21 / as_of 10-20），
  相同的 50 条不重复日期。unknown 打「来源未知」⇒「无标」可被证明就是实况，不是把「拿不到」伪装成默认态。
- 11 语言新增 `stockAnalysis.provenance{Replay,ReplayWithDate,Unknown}`；措辞取自产品既有 `timeTravel.pageAnchor.{live,replay,untilDate}`
  （历史回放 / 实时分析），但**刻意另建键** —— 两个不同语义共享一键，将来改锚点措辞会拖挂记录来历。

**判据（两层，各配自证）**：

- 纯函数 `analysisHistoryFilter.test.ts` +4 条（两态归位、大小写空格、缺失/第三值归 unknown、
  以及一条**源码接线门**：组件必须声明这两列且调用 `provenanceOfRecord(r)`，防「后端发新键、组件读旧形状」的半改状态）。
  该门已自证有电：把三个被 `toContain` 的串从现文里删掉，三者同时失配。
- 组件 `AnalysisHistoryButton.loadAll.test.tsx` +4 条（live 无标 / 同日回放只写「历史回放」/ 异日回放带决策日 /
  空串与第三值都出「来源未知」且 live 行不混进来）。该文件 **18 passed**；纯函数文件 **23 passed**。
- 门与三关复跑全绿：`tsc` 0、`dprint` 0、`oxlint` 0/0、i18n parity（11 语言键集合一致）/ placeholder / 缺译 /
  `check_i18n.py --strict` 各 0、EOL 0。逐语言回读断言过（值非空、无首尾空白、`{{date}}` 槽完整、无隐形字符）。

**刻意没做（等用户点）**：

- 没加**第五维筛选 chip**（来源）。数据已到手，加一行三个 chip 是十几行的事；但用户问的是「标记」，
  且弹窗已有四维（关键词/状态/时间维度/结论），多一维要不要做由他定。
- **R-DA-7（新待裁）**：`analysis_kind` 本身也是散成字面量的第二套值域 —— Rust 侧 5 处各写 `"live"` / `"replay"`
  （`core.rs:2040` 的三元、`stock_analysis.rs` 的 match 臂、`stock_analysis_rerun_queue.rs` 两处、`core.rs:506`），
  entities 里只有注释说明「kind 只分 live/replay 两态」，无常量无类型。按「全局一处权威」那条裁定它同样该收编，
  但它不属于 action 面 ⇒ 单独点。
**顺着这条又查出一处前端镜像漂移（阶段 B 的实证理由）**：同一个后端 DTO `StockAnalysisListItem`
在前端有**两份手写形状**且已经分叉 —— `@/types` 的 `AnalysisSummary`（16 键）与弹窗本地的
`AnalysisRecord`（19 键）：

- 只在弹窗本地形状里的 4 个键：`decisionTimeHorizon`、`decisionHorizonSource`、`templateVersion`、`blackboardSnapshot`
  （前三个后端**确实在 SELECT 里下发** ⇒ `AnalysisSummary` 是**漏声明**，按它打型的消费点（运行抽屉 / store 历史 /
  任务中心 adapter）拿不到档位与图代，不是后端没给）。
- 只在 `@/types` 里的 1 个键：`completedAt`（弹窗没声明，但后端下发）。

⇒ **R-DA-8（待裁，属阶段 B）**：把这两份合成一处（正解是 ts-rs/`schema-gen` 生成 + 前端 re-export，
本仓已有 `src/types/generated/{ipc,events}.ts` 的先例，只是 CI 现在完全不跑生成）。
在此之前，任何新增 DTO 字段都要在两处镜像里各写一遍 —— 本轮我加的 `analysisKind` / `asOfDate`
就只在弹窗那份补了声明（`AnalysisSummary` 里本来就有），这本身就是一次漂移的现场样本。

## 9. 配色轮（2026-10-23 凌晨，用户「决策结果应按利空→利好强度分色，不得结果不同而颜色相同」）

**普查结论：按档位上色在前端有六份实现**，其中三份自己写值、两份写反方向：

| 位置 | 原形态 | 问题 |
|---|---|---|
| `stock-analysis-utils.ts::getActionColor` | antd 预设名：买/增持=`red`、卖/减持=`green`、持有=`blue`、观望=`orange`、不确定/数据缺失=`default` | **三对同色**（正是用户点名的缺陷） |
| `stock-analysis-utils.ts::getActionTagStyle` | CSS 变量：买/增持共用 `--color-danger`、卖/减持共用 `--color-success`、兜底两档共用 `--color-t-tertiary` | 同上三对 |
| `dual-view/CompactDecisionComparison.tsx::actionColor` | `buy → #10b981`（绿）、`sell → #ef4444`（红） | **方向与 A 股习惯相反** ⇒ 同一个「买入」在弹窗红、在这张卡绿 |
| `dual-view/DecisionComparisonPanel.tsx::actionTagColor` | `buy → "green"`、`sell → "red"` | 同上相反，且把六档拍成三色 |
| `chat/MultiLlmVotePanel.tsx::actionColor` | 已是 `getActionColor` 的薄封装 | 但**下游把它当预设色名拼 `token[color${Name}]`** ⇒ 换 hex 后每档查不到键、静默退成 `colorPrimary`（蓝） |
| `ExitRecommendationPanel.tsx::actionColor` | `SELL_NOW / SET_STOP_LOSS / CONSIDER_ADD …` | **不同值域**（退出建议档），不归这张表管，未动 |

**改法**：新增唯一权威 `ACTION_PALETTE`（八档 → `{ hex, rank }`，按 A 股红=利好渐变：
`卖出 #13c2c2 → 减持 #52c41a → 观望 #a0d911 → 持有 #faad14 → 增持 #fa541c → 买入 #f5222d`，
兜底两档 `不确定 #9254de / 数据缺失 #8c8c8c` **rank=null**（缺席不摆上强度轴））。
底色/描边用 `color-mix` 从同一 hex 派生 ⇒ 深浅主题都成立。**主题 token 只有 danger/success/info/warning 四枚，
装不下八档互异**，这是把表落成 hex 的原因，不是随手写死。

其余五处全部改为从这张表取值：`getActionColor` / `getActionTagStyle` 直读；双视角两份**删除本地表**
（`actionToDirection` 只保留给「下单方向」那条链用）；`getDashboardActionColor` **删除第二张六色渐变表**
（「强烈买入/强烈卖出」权威已折进 `BUY/SELL` ⇒ 与买入/卖出同色是裁定的结果，强度词仍由
`DASHBOARD_ACTION_TKEY_OVERRIDES` 在文字上保留）；投票面板不再拼 `token[color${…}]`，直接吃 hex。

**⚠ 有意的可见行为变化**（核对时按这几条看）：双视角两张卡的买/卖两向颜色**翻转**（原为美式绿买红卖）；
`dashboard_report` 的「强烈买入」与「买入」同色（文字仍不同）；分析师卡的「关注」类信号标签由蓝变琥珀。

**门**（同一条 CI 步骤带上，无需新注册）：**R11** = 八档齐 / **八色互不相同** / 六档 rank 恰好 0..5 /
兜底两档 rank=null / 「利空<中性<利好」必须与权威谓词 `implies_reduce` / `implies_buy` **同向**
（方向不是门里抄的色序，是从 Rust 谓词推出来的）；**R12** = 除权威文件外，全前端不得再出现
`case StockAction.X:` 或第二份 `ACTION_PALETTE`。负控 **11 → 16 条**（R11 四格 + R12 一格，各自只红自己那格）。
门自己也被抓过一次：`bodyAfter` 把 `Record<…, { readonly hex… }>` 的类型注解当成对象体 ⇒ R11 首跑假红「解析不到」，
按「解析不到=当红」的既有纪律修的是抽取器，不是判据。

**判据**：纯函数 `stockAnalysisAction.test.ts` 新增 7 条（八色互异、rank 0..5、方向单调、
token/中文/别名同色、`getActionTagStyle` 与 `getActionColor` 同源、看板兜底同后端 `apply_risk_veto`、
双视角不再自带反向配表——断的是代码形态而非注释里出现过的 hex）；呈现层 `AnalysisHistoryButton.loadAll.test.tsx`
新增 2 条（八档各一枚 Tag 且 `style.color` 两两不同；库里存中文与存英文 token 渲染同色）。
**负控实测**：把 `INCREASE` 的 hex 改成与 `BUY` 相同 ⇒ 纯函数 1 条红 + 呈现层 1 条红 + 门 R11 报
「INCREASE/BUY 共用 hex #f5222d」，复原后 `cmp` 证与备份逐字节一致。

**读数**：全量 vitest **251 文件 / 2712 用例全通过**；`tsc` 0；`oxlint` 0/0；`dprint` 0；EOL 0；
门 `OK 决策结论档六面对账一致` + `--selftest` 16/16。
中途两次全量出现的 5s/6 条红里，只有 `getSignalColor 与 getActionColor 口径一致` 是真红
（那条断言钉的正是「两处各写一遍色名」的旧约定，本轮把它改为**同端点同 hex**），
其余五条单跑全绿 ⇒ 是全量并发把 500 行夹具的翻页用例挤超时；
已给这三条**显式放宽超时（20s/30s）并注明原因**，而不是留着靠机器空闲。

## 10. R-DA-7 / R-DA-8 最远裁决实施批（2026-10-23）

### 实测把「两态」这个前提打掉了

我上一条回答里说 `analysis_kind` 是 `live`/`replay` 两态 —— **错**。穷尽写侧后是**三值**：

| 写点 | 值 |
|---|---|
| `core.rs:636`（inline if）/ `core.rs:2040`（三元）/ `trade_intent.rs` ×5 / `reflection.rs:4623` 夹具 | `live` / `replay`（按 as-of） |
| `hooks.rs:1215`（`stock-analysis-persist`，对话直执行通道） | **`chat`** |

而实体注释写的第四值 `ab_test` **没有任何生产者**（全仓 `ab_test` 只出现在 `prompt_template` 字段名）。
同一行它还写了 **`source = "chat"`** ⇒ 通道这件事在 `analysis_kind` 与 `source` 两列各记一遍；
库里另一处已实测出这种重叠的代价：`source/analysis_kind` 组合是
`live+live 198` / `time_travel+replay 76` / **`live+replay 6`** / `mover_recall+replay 6`。

### 按裁定落地（不再问一遍）

两条既有裁定合推出唯一解： 「分档不得重叠」+ ② 实体注释里**已经写明**分工
（「kind 只分 live/replay 两态，source 记业务出处」）⇒ **本列收回两态**，`chat` 从 `analysis_kind` 退役、
由 `source='chat'` 唯一承担。这不是新设计，是照既有注释归位。

- 新增权威 `crates/harness/src/analysis_kind.rs`：`enum AnalysisKind{Live,Replay}` +
  `ANALYSIS_KIND_VALUES` + `as_storage` / `as_cn` / `from_storage`（严格，未知返回 `None`，
  退役值 `chat`/`ab_test` 都读不出来）+ `from_asof`（写侧唯一入口）。3 条单测。
- **15 处字面量**（8 文件，含 2 处测试夹具与 `pool_scan` 那处提到 ab_test 的注释）全部改走权威；
  `backtest_all_history` 的 `scope` 入参从 `"live" | "replay" | _` 两个 match 臂改成
  **parse-then-match**（旧写法在权威改名时静默落到 `_` = 不过滤，症状是「筛了但没筛」）。
- 实体注释改正：列注释删掉幻影 `ab_test`、指向权威；`hooks.rs` 三处 + `core.rs` 一处 +
  `axagent-batch-rerun.rs` 一处仍称 `analysis_kind="chat"` 的注释一并改为 `source="chat"`。
- **DB CHECK 两条**（`dao/reconcile/extras.rs`，声明式、无迁移）：
  `ck_stock_analyses_kind` = `analysis_kind = ANY (ARRAY['live','replay'])`；
  `ck_stock_analyses_decision_action` = `decision_action IS NULL OR = ANY (八档中文)`
  （`IS NULL` 必须留：库里 10 行 NULL 的语义是「采集时点无此信息」）。
  现网实测 286 行**全部满足**两条 ⇒ 启动期对账不会失败。`EXPECT_CHECKS 3→5`、
  `l2_total_is_seventy_one → _three`（71→73）、文件头的「库里实态 vs 静态扫描」对照表同步。
- **门 R13 + R14**：R13 = 权威内部两张清单一致 + Rust 侧（含测试夹具、剥注释后）不得出现
  `analysis_kind`/`AnalysisKind` 同行的 `"live"|"replay"` 字面量 + 前端 `provenanceOfRecord`
  认的值集合必须等于权威；R14 = DB `CHECK` 里的两份 `ARRAY[...]` 清单逐档等于两个权威
  （值域第三处表达，用门钉而不靠自觉）。负控 16 → **21 条**，全绿。
  门又抓到我自己的抽取器一处：`bodyAfter` 把 `provenanceOfRecord(r: { analysisKind?: … })`
  的**参数内联对象类型**当成函数体 ⇒ 前端面假红「只认 —」；新增 `functionBodyAfter`（按括号深度跳过参数表）。
  成功文案与 `ci-check.mjs` 标签改为「八面」。

### ⚠ 已知副作用（刻意接受，须真跑核对）

对话直执行通道的记录从此记 `analysis_kind='live'` ⇒ 会**进入** `= 'live'` 的读者
（`pool_scan`「当日已分析」、`reco_loop` 回环否决窗口）。现网 `analysis_kind='chat'` **0 行**
⇒ 今天零变化；第一次对话直执行起才会进统计。记为 **R-DA-10**（本批已按两轴归位，剩「这样统计是否可接受」的产品确认）。

### R-DA-9（新，未修）

`reco_loop.rs` 的回环否决窗口谓词是 `AsOfDate.is_null() AND AnalysisKind='live'`，而 `stock_analyses`
上 **198/198 条 live 行的 `as_of_date` 非空且等于 `analysis_date`** ⇒ 该查询今天命中 **0 行**，
机制事实失效。`as_of_date IS NULL` 只在 `reco_picks` / `decision_validations` / `reco_scan_audit` 上是
「实盘代」判据（全仓另有 20+ 处这么用，都对）。修法（删掉该谓词只留 kind）会**重新启用一条从未生效的否决**
⇒ 改变荐股输出，须配真跑对账，**不在本批夹带**。

## 11. R-DA-8 第一增量：`schema-gen check` 从「打印」变真门（2026-10-23）

最远形态是「把 `stock_analyses` 行 DTO 上移到 schema-gen 看得见的 crate，前端五处镜像合并成
生成物再导出」。那一步要先动 `entities` 依赖与 AGENTS.md 角色表，**且必须先有门**——否则
「生成物与权威一致」这条前提本身没人守。本批只做第一增量。

### 打开门才发现的四件事（都是既成事实，不是本批引入）

1. `run_check` **从不非零退出**：四条检查只 `println!`，无论 `check` 还是 `--mode all`
   一律 `EXIT=0`。CI 里根本没有这道门，本地 `cargo run -p schema-gen -- check` 的绿是恒绿。
2. `--mode all` 遇到无 `definitions` 的 schema 直接 **panic**（`expect("schema.definitions")`）。
   即「跑一次全量生成」这条命令是坏的，所以漂移多年没人发现。
3. 生成器往产物头部写 `// Generated at: <SystemTime>` ⇒ 每次生成都不同 ⇒ **任何**字节级
   比对永远红。这行已删（同类先例：`events.ts` 就没有时间戳，注释里写明「不要照抄到这里」）。
4. 已提交的 `ipc.ts` 里有**手加的** `MessageRole` / `JsonValue` 基类型（不在生成器里）⇒
   谁重新生成，产物就编译不过。已把这两个基类型搬进生成器，手改来源随之消失。

外加**两个工具在抢同一份产物**：`src/types/generated/**` 没被 dprint 排除，生成器写
`a, b,`、dprint 改成 `a; b` ⇒ 裸字节比对一次报 118 行。按该 config 已有先例
（`schema-gen/**`、`crates/**/bindings/**` 均排除）把生成目录交给生成器独占。

### 落地

- `check_generated_drift`：重新生成到临时目录 + **逐字节**比对 `ipc.ts` / `events.ts`，
  不一致时报「首个差异在第 N 行；在库 X 字节 vs 重跑 Y 字节」，随后清理临时目录。
- `check_event_names_sync`：`events.ts` 的事件名联合必须等于 Rust 侧 83 个成员。
- `run_check` 收 `failures: Vec<String>`，末尾非空则 `exit(1)`。
- 注册三处（逐字同源）：`scripts/ci-check.mjs`（Rust 块，`cargo test` 之后）、
  `.github/workflows/ci.yml`（`rust-check` job，`Check build` 之后，`working-directory: src-tauri`
  以吃到 1.97 钉版）、`package.json` 的 `check:schema-gen`。

### 读数

- 负控：把已提交的旧 `ipc.ts` 喂进同一检查 ⇒ `NEG_CHECK_EXIT=1`，报「首个差异在第 12 行；
  在库 4090 字节 vs 重跑 3805 字节」。**门有电**。
- 修完后 `cargo run -q -p schema-gen -- check` 连跑两次 `EXIT=0`（幂等），
  `npm run check:schema-gen` `NPM_EXIT=0`；`npx dprint check src/types/generated/ipc.ts`
  报「No files found」= 排除生效。
- `ipc.ts` 净变化 44 插入 / 68 删除（时间戳行 + dprint 强加的改写消失，基类型进生成器）。
- `node --check scripts/ci-check.mjs` 通过；`ci.yml` 用 PyYAML 解析通过且新步在
  `Check build` 与 `Setup Node.js` 之间；`package.json` JSON.parse 通过；dprint 对
  `ci-check.mjs` + `package.json` 两个改动文件 `EXIT=0`。

### 重新生成之后，另外两道门复跑（防「改生成物⇒消费门假绿」）

- `check-decision-action-single-source.mjs`：`GATE_EXIT=0`（八面对账）+
  `--selftest` `SELFTEST_EXIT=0`（21 个破坏样本各自有电）。该门的 Rust 扫描面会把
  `output/`、`fixtures/`、`target/` 剔掉，前端面剔 `__tests__` 与 `generated/` ⇒
  重新生成的 `ipc.ts` 不会被当成「第二份档位定义」误报，但也不会漏掉它真的被写脏。
- `npm run typecheck`：`TSC_EXIT=0`。这一步是必要的——生成器新吃进 `MessageRole`/`JsonValue`
  两个基类型，若签名与原先手写的那两份不同，前端会在编译期红。

## 12. 全量批被超时掐断 + 两条 CHECK 的现网预检（2026-10-23 02:00）

### 全量批读数（只到第一步）

`.workbuddy/tmp/rda7-gate.log` 43 行，`FMT_EXIT=0`（**`cargo fmt --all --check` 已过**），
其后 clippy 推进到 `Checking axagent-runtime` 被杀 —— 我把 `run_in_background` 的 timeout 设成
600000（该工具的上限就是 10 分钟），而全量 clippy 本仓要 11–25 分钟。**这是取数姿势错，不是代码红**。
同时我在命令里把 clippy 接了 `| tail -60`，被杀时 `tail` 未 flush ⇒ 正文只剩编译进度行，
没有结论。⇒ 以后长批：**每条门单独一次调用、输出直接重定向落盘、不经 `tail`**（被杀也留得下正文）。

### 两条 CHECK 的现网预检（应用即将带我的代码启动，这是它第一次真加约束）

`node scripts/pg-connect.mjs` 实测：

| 判据 | 读数 | 结论 |
|---|---|---|
| `stock_analyses` 现有约束 | 14 条，**全是 `NOT NULL` + `PRIMARY KEY`，一条 `CHECK` 都没有** | 我的两条是首两条，启动期 `ADD CONSTRAINT` 会**真跑** |
| `analysis_kind` 违例 | `bad_kind = 0` | 可加 |
| `decision_action` 分布 | 持有 105 / 观望 77 / 数据缺失 36 / 减持 21 / 卖出 17 / 买入 14 / **NULL 10** / 增持 6 | 七档全部在权威八档内、`不确定` 当期无人吐；NULL 10 行必须放行 |
| 先例约束的存储形态 | `agency_experts_category_check` / `agent_profiles_category_check` 在库里是 `CHECK ((category = ANY (ARRAY['general'::text, …])))`，声明侧写的是不带 `::text` 的同形 | `normalize_sql_expr` 剥 `::type` + 剥配平外层括号 ⇒ **单项 `= ANY` 形态零噪声**（这两条先例即证据） |

### R-DA-11（新，我自己的缺陷，未修）

我给 `decision_action` 声明的是**复数项** `decision_action IS NULL OR decision_action = ANY (ARRAY[…])`，
意图是「显式写出 NULL 放行」。但 PG 会把每个操作数**各自加括号**回读成
`CHECK (((decision_action IS NULL) OR (decision_action = ANY (ARRAY['买入'::text, …]))))`，
而 `normalize_default` 只剥**配平的外层**括号、不剥操作数级内层括号
（`model.rs:444-477` 自陈「本函数是近似，不是等价判定」）。
⇒ `plan.rs:1226` 那句 `Some(ac) if ac.expr == bc.expr => {}` 走不进 ⇒ 每次启动打一条
`text-diff …（按名判同一约束，不产生变更）` 的 advisory，**永久长亮**。

- 危害分级：**不产生变更、不失败**（`check_level` 按名判同一约束，advisory 是设计内的），
  只是把一条永不自愈的噪声留在启动日志里 —— 与 [[project-utf8-byte-slice-family]] 里
  「hitrate 的 reflection_id WARN 永久长亮」同一类：以后没人分得清它是缺陷还是噪声。
- 修法：两条都写成与先例同形的**单项** `col = ANY (ARRAY[…])`。SQL 的 CHECK 语义下
  表达式为 NULL 即视为满足 ⇒ 10 行 NULL 照样放行（**这是标准语义，不是漏洞**，
  注释里要写明「NULL 由 CHECK 的 unknown 语义放行，故不再显式写 `IS NULL OR`，
  否则每次启动多一条 text-diff advisory」）。R14 门解析的是 `ARRAY[…]` 里的清单，改形态不影响它。
- **为什么不当场改**：现在 02:00 是每日自动化窗口、`cargo run`（tauri dev）正在编译，
  保存 `src-tauri` 会触发重建重启并杀掉在跑的分析（见 [[feedback-tauri-dev-restart-kills-runs]]）。
  改点是一行 expr，等应用空闲一次再做，并与 R-DA-9 的真跑核对同批。
