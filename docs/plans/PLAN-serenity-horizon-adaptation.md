# PLAN — 趋势智选（serenity）四周期适配收口

> 状态：active ｜ 日期：2026-09-30 ｜ 对应版本：v2.9.9


> 承接 `PLAN-multi-horizon-decision-science.md`（分析决策链 A~F）与 `PLAN-reco-horizon-science-alignment.md`（荐股链 R-0~R-F）。
> 本计划只动**趋势智选名目下的两条链**，不重做已经完成的 A~F / R-*。

## 〇、边界声明（先说清「适配」指什么）

趋势智选的四周期适配**不等于**把它改成四档出票。既有裁定（`serenityPanel.tierScopeHint`、
`style_matrix.rs:56-59`）已定：**瓶颈/政策/业绩催化剂类证据以周-月兑现，2 天与 1 周尺度没有可验证路径** ⇒
serenity×ultra_short / ×short 是「按设计不成立」。本轮要做的是四件事：

1. 让它**产出面的风控口径与荐股链同源**（现在两条链两套口径）；
2. 清掉荐股链尺度层（R-B）**最后一个未接的取数点**，并修好漏检它的门；
3. 让「短/超短不适用」的理由**从后端契约表走到 UI**（现在只有后端有）；
4. 买入价带的档位口径收口。

## 一、实证现状（本轮全部现场验过，非推断）

| # | 证据 | 现状 | 为什么是缺陷 |
|---|---|---|---|
| G1 | `commands/stock_workflow/serenity.rs:1383-1404` vs `recommender/mod.rs:866-1002` | 工作流面落库：`price * serenity_stop_mult`（0.80）、`price * serenity_target_mult`（1.30）、`positionPct` 取候选值或 5.0，**无** `stopSource/positionSource/priorSource` 键；策略面（`SerenityStrategy`，style='bottleneck'）经 R-C/R-D 后处理：`k1·σ_daily·√h` 止损、风险预算仓位、成本拖累、逐档先验 + 三个来源字段 | **同一「趋势智选」名目、同一张 `reco_picks` 表、同一份历史列表**（`SerenityScreeningPanel.tsx:1165` 的 `styleFilter: "serenity,bottleneck"`）里并存两套风控口径，且其中一套不声明自己不是波动率口径 —— 正是「两处各自自洽、两边打架」 |
| G2 | `recommender/strategies/serenity.rs:124` | `client.get_klines_with_adj(code, "daily", 252, None)` —— 全 `recommender/strategies/` 下唯一写死 `"daily"` 的取数 | R-B 尺度层（`scale.rs`）已覆盖 trend/value/capital/reversion/watchlist 五策略，serenity 是唯一残留 |
| G2-门 | `node scripts/check-reco-horizon-parity.mjs` 实跑：**f 段扫描面 4、结论 OK** | 门 f 判据是 `line.includes("get_klines(")`（`check-reco-horizon-parity.mjs:146`），`get_klines_with_adj(` 不含该子串 ⇒ **门的假绿**，serenity 漏检 | 判据本身漏覆盖面；「扫到 0 条的门等于没有门」的同族形态 |
| G3 | `recommender/strategies/serenity.rs:186,194-196` | `serenity_entry_range` 固定 ±5%，mid（28 天）与 long（90 天）同带；R-D 后处理只重算 stop/target，**不覆盖 entry_low/high** | 买入带是「当日噪声尺度」的函数，现是常数；与档/与 σ 都无关 |
| G4 | `grep absence_reason` 全仓：只命中 `style_matrix.rs:81` 定义处；`serenity_needs_week_or_longer_realization` 在 locales / 组件 / 门里 **0 命中** | 矩阵能表达「不成立 + 理由码」，但理由码**零消费者** ⇒ UI 上 serenity×短/超短只是空格 | 违反「拿不到可以，伪装成正常结果不行；三种缺席各占一句」——空格既不说明「按设计不做」也不说明「实现漏了」 |
| G5 | `strategies/serenity.rs:331-335`、`mod.rs:1206-1207` | 策略面注册 `serenity_mid` + `serenity_long`；工作流面 `:979` 恒 `Period::Mid` | 矩阵声明 serenity 出票档 = {mid, long}，但候选池只按 mid 落库 ⇒ 两链档位集合不一致（**属裁定项，见 §三 Q2**） |

已确认**不是**缺陷（避免重复劳动）：持有天数已统一到权威表（`serenity.rs:972-976` 记录旧 20 天 vs mid=28 的矛盾已修，落库兜底 `serenity_tier_days`）；`holding_days: self.period.default_holding_days()`；`tierScopeHint` 11 语言齐全；`RecommendationPanel.tsx:955/960` 已会消费 `stopSource === "fallback_pct"` / `priorSource === "absent"` 的缺席声明（⇒ G1 补上字段后展示层基本不用改）。

## 二、阶段

### S0 先修门（不动生产代码）

- f 段判据从 `get_klines(` 扩到 `get_klines(_with_adj)?\(`，并加**扫描面下限**断言（当前 4 ⇒ 修后应为 5，含 serenity）。
- 检法：`--selftest` 增加一条正对照（复刻 serenity 现状 `get_klines_with_adj(code, "daily", 252`），证明**修前该门全绿、修后即红**；生产代码尚未改时，主流程必须报出 `serenity.rs:124`。
- 顺序说明：先让门会红，再动 S2，避免「改完靠肉眼确认」。

### S1 工作流链风控合流（G1，本轮最大项）

- `run_serenity_screening` 落库前，改用**全仓唯一实现**：`recommender::risk::{daily_closes, stop_pct, target_pct, risk_budget_position, cost_drag_factor}`（禁区 12：不新建第二套 σ / 仓位公式）。参数读同一批模板变量（`reco_stop_vol_mult` / `reco_target_vol_mult` / `reco_risk_budget_pct` / `reco_round_trip_cost_pct`），h 取 `serenity_tier_days`。
- σ 不可得 ⇒ 保留现固定乘数并写 `stopSource="fallback_pct"`；仓位不可得 ⇒ `positionSource="fallback_base"`。**不伪装成波动率口径**。
- `pick_data_val` 补 `stopSource` / `positionSource` / `priorSource` 三键，与 `RecoPick` DTO 同名同义（camelCase，禁区 13）。
- 检法：新增 Rust 单测/集成断言 —— 同一 `(code, mid)` 分别经策略面后处理与工作流面落库，stop/target/position **逐值相等**；并锁「σ 缺失分支必须写 fallback_pct」（用修复前形态自证会红：旧代码根本不出这三个键）。
- 前端：`SerenityScreeningPanel.tsx` 候选卡片补一句来源声明（波动率 / 固定百分比退化），不复用荐股面板的 i18n key 则新增 11 语言。

### S2 serenity 取数的日历口径显式化（G2）

- 「近 3 月 / 近 12 月涨幅过滤」本意是**日历窗口**，日线是它的正确尺度 ⇒ **不**换成 `scale::fetch`（换了反而语义漂移）。要做的是：显式声明这是日历窗口过滤器、并消灭静默放行 —— 现 `if klines.len() >= 63` 不满足时直接不过滤且无任何标注，次新股会「绕过涨幅闸却显示验证通过」。
- 改法：取数失败 / 样本不足 ⇒ 往 `reasons`（或 `risk_notes`）推一条「涨幅过滤未执行（K 线不足 63 根）」，并让过滤状态可机读。
- 门 f 段：serenity 的 `get_klines_with_adj(code, "daily", ...)` 需要一条**豁免登记**（写明理由 = 日历窗口，非尺度输入），豁免必须进门的白名单表并被 selftest 锁住，不能靠放宽判据过门。

### S3 买入价带按档/按波动派生（G3）—— **口径待裁定，见 §三 Q1**

### S4 矩阵理由码进 UI（G4）

- `absence_reason()` 接入回测/矩阵比较响应：每格输出 `reasonCode`（`cell_is_active` / 具体理由码 / `cell_not_in_matrix` 三态，后者代表「实现漏格」，必须与「按设计不做」区分开）。
- 前端 `RecoStrategyMatrix.tsx`：空格按 reasonCode 出文案，不再硬编码哪格为空；新增 i18n key ×11 语言。
- 门：`check-reco-horizon-parity.mjs` 的 e 段加一条「矩阵每个理由码都必须有前端 i18n key（11 语言全齐）」，并用 selftest 造「加了理由码没加翻译」的红。

### S5 全量门禁 + 判据登记

- `cargo fmt --all --check`；`cargo clippy --workspace --all-targets --all-features -- -D warnings`；`__TAURI_WORKSPACE__=true cargo test --workspace`；`npm run typecheck` / `test:run` / `lint:eslint` / `check:eol` / `check:i18n:key-parity` / `npm run contracts` / `check:serde` / 两个口径门（含 `--selftest`）。
- 若动 seed 模板变量：`TEMPLATE_VERSION` 必须 bump（当前 108，106/107 已被并发会话占用，先查最新）。
- 判据层追加：门的「子串判据可被同族函数名绕过」这一形态（G2-门）值得进 `MEMORY-RULES.md`。

## 三、需要你裁定的两项（我不拍数）

**Q1（S3 买入价带口径）** 两个候选，都有出处、不再是常数：

- **A：`entry = ±1·σ_daily`**（夹在 `[0.5%, 5%]`）。依据：建仓带的语义是「当日噪声幅度」，超过噪声就该等下一个 bar 而不是追价；与档无关、与标的波动有关。
- **B：`entry = ±stop_pct/2`**。依据：买入带宽 = 该档止损距离的一半 ⇒ 「带内成交 → 触止损」的最大额外亏损结构在四档间保持一致。
- 也可以两者都要：A 定带宽、B 作上限约束（取 `min`）。默认值与钳位区间若采用 A，需要新增一个可调模板变量并同步设置面板（五点对账）。

**Q2（G5 档位集合）** 趋势智选到底服务几档：

- **只 mid**：把 `SerenityStrategy::long()` 从注册表摘掉，矩阵里 serenity×long 改成带理由码的不成立格（口径最窄，与面板现文案一致）。
- **mid + long 双档**：工作流链也要按两档各落一次库（候选池复用，stop/target/仓位按各自 h 推导），面板文案从「仅服务中线」改为「服务中/长线」。
- 现状态是**两链不一致**（策略面两档、工作流面一档），这是最差的第三种，必须选一边收口。

## 四、协作约束

- `recommender/`（`scale.rs` / `risk.rs` / `style_matrix.rs` / `ic.rs` / `strategies/serenity.rs`）刚由并发会话随 `03e0f2e85` 提交，工作区当前干净，但它随时可能再改 ⇒ **S0/S2/S4 落地前先 `git status` 复核**；若与并发会话真重叠，按既有做法**只改不提交**，等那批落地后统一切。
- 工作区现有 6 个未提交文件（`plugins/{manager,types}.rs`、`stock_analysis.rs`、`stock_workflow/hooks.rs`、两处 `reflection_stats.rs`）属并发会话的分层搬移，**不动、不提交、不掺进本轮**。
- 阶段间只跑定点门（`cargo check` + `cargo clippy -p <crate> --all-targets`），全量留到 S5 一次。

## 五、顺序建议

S0（门先红）→ S1（口径合流，收益最大）→ S2 → S4 → **Q1/Q2 有答复后**再做 S3/S5 收口。

## 六、进度（2026-09-30 实施轮）

### S0 门 f 判据（已完成，实证）

- 判据 `line.includes("get_klines(")` → `/\.get_klines\w*\(/`。
- **修复前形态当场暴露**：改判据后主流程立刻报红 `strategies/serenity.rs:124`，扫描面 4 → 5；
  旧判据对 `get_klines_with_adj(` 恒不命中 ⇒ 此前「扫描面 4 / 结论 OK」是假绿。
- 豁免机制：`reco-scale-exempt: calendar-window`（码集合校验，未登记的码视为未豁免）。
- selftest 新增 3 条（同族方法名负控 / 未登记豁免码负控 / 登记过的日历窗口正控）
  + 真文件下限断言 `scanned>=5`、`exempted>=1`。全 15 条 PASS。

### S2 serenity 日历窗口显式化（代码完成，待 cargo 复验）

- 取数处登记 `reco-scale-exempt: calendar-window`（窗口本意是日历量，日线是正确尺度，**刻意不接 scale 层**）。
- 消灭三处静默放行：`bars<63` / `bars<252` / 取数失败 / 评分豁免 ⇒ 全部落 `gain_filter_skips`，
  再由 `risk_notes` 成句输出「⚠ 涨幅过滤未生效：…」。
- **顺带查出一个真缺陷**：近 12 月过滤旧实现直接取 `klines.first()`，不足 252 根的次新股
  把「上市以来涨幅」当成「近 12 月涨幅」去比 300% 阈值 ⇒ 窗口比阈值短、判据系统性偏松
  （注释只给 3 月那条写了守卫，12 月漏了）。现已补 `bars < 252` 守卫。

### S1 工作流链风控合流（代码完成，待 cargo 复验）

- `run_serenity_screening` 落库前改调 `recommender::risk::{daily_closes,stop_pct,target_pct,risk_budget_position,cost_drag_factor}`，
  参数读与 `recommender/mod.rs` **同一批键**（`reco_stop_vol_mult` 等四把）。
- σ 不可得 ⇒ 退回固定乘数并写 `stopSource="fallback_pct"` + 一条 riskNote 说明它不是波动率口径；
  仓位不可得 ⇒ `positionSource="fallback_base"`；逐档先验只在策略链合成 ⇒ 本链写 `priorSource="absent"`（沿用既有词表，不新造说法）。
- 候选自带 `holdingDays` 与该档权威不符 ⇒ 按权威落库并在 reasons 里**声明覆盖**，不静默改写。
- 呈现层：`RecoHistoryModal` 详情卡此前只有代码/名称/档位/置信度 ⇒ 现渲染 `pick_data.reasons` /
  `riskNotes`（复用 `row.reasons` / `row.riskNotes` 两个既有 i18n 键），两链口径差异在历史上首次可见。

### S4 矩阵契约驱动 UI（代码完成，前端门已绿）

- 后端：`ic.rs` 新增 `MatrixCellView` + `contract_cells()`（24 格恒在，与样本无关），
  `RecoIcStats.matrix` 随 `reco_ic_stats` 出边界；`style_matrix` 补
  `style_keys()` / `db_style_aliases()`（`serenity` 一名两写：工作流链 `serenity`、策略链 `bottleneck`）/
  `reason_code_by_key()` / `misfit_reason()` —— `absence_reason()` 自此有了第一个消费者。
- 前端：矩阵行集合改由契约驱动（兜底才用 4 项清单）；删除 `style === "reversion" && period === "long"`
  硬编码，不成立格按 reasonCode 出句；错配格单独出「已知错配」声明；
  「按设计不做」与「暂无已验证样本」分成两句。新增 4 个 Vitest 用例（19/19 绿）。
- i18n：`matrixNoSample` + `matrixAbsence`×4 + `matrixMisfit`×1，11 语言齐（key-parity 15293 条/文件一致）。
- 门：新增 g 段（理由码 11 语言全译，判定面 55=5 码×11，两条负控各断言 11 处红）
  与 h 段（趋势智选两链风控同源 + 策略链静默放行消灭，判定面 18，
  负控复刻「固定乘数落库」14 处红 / 「无守卫无声明」4 处红）。主流程 a~h 全绿。

### 待办

- **Q1 / Q2 仍未裁定**（§三），S3 与档位集合收口不动。
- Rust 侧复验被 build 目录文件锁挡住（并发会话在编译）：`cargo test -p axagent-analysis-engine --lib`、
  `cargo check -p axagent --all-targets` 必须重跑取正文；之后 S5 全量门禁。
- ⚠ **索引已被并发会话写入**：`git status` 显示 `MM`/`M ` 混杂，本轮 ic.rs / style_matrix.rs /
  strategies/serenity.rs / commands/stock_workflow/serenity.rs / locales 均已被 `git add`（含尚未过编译的部分）。
  本轮不提交、不改索引，等 Rust 复验后由用户决定切分。
- 发现（本轮**不动**，待裁定）：矩阵只登记 6 个名目，而 `Style` 有 11 个变体 （`Policy`/`Earnings`/`CapitalFlow`/`Event`/`Technical` 未入表）。这些子风格可能存在于历史 `reco_picks`/回测行里，因此在矩阵上**仍然不可见**（与修复前一致，不是本轮引入）。要么它们并入 `serenity` 一行（矩阵注释已这么说，但 `styles_for` 并没真的合并），要么 11 → 6 的映射该显式写进契约表；两种都需要裁定。
- 判据层已登记 **#712**（子串判据被同族函数名绕过 + 契约理由码必须有读取方）。

### S4 附带查出并修好的潜伏缺陷（由本轮第一个消费者暴露）

`style_matrix::{absence_reason, is_active}` 用 `Style::as_str()` 去比 MATRIX 的**风格列**，
而二者不在同一个名目空间：矩阵写 `serenity`（`style_from_str` 注释自己就写着「矩阵只用 serenity
一个名目，落库按子类型序列化」），`Style::Bottleneck.as_str()` 却是 `bottleneck`。
后果：serenity 四格**全部查不到该行** ⇒ `is_active(…, Mid/Long)` 恒 `false`（明明出票）、
`absence_reason(…, UltraShort)` 恒 `Some("cell_not_in_matrix")`（「按设计不做」被误报成「实现漏格」）。

这两个函数自 R-B 落地以来**零消费者**，所以缺陷一直静默；本轮 `contract_cells()` 第一次调用它，
新测试 `contract_cells_cover_the_matrix_regardless_of_samples` 当场把它撞红（547 过 1 红，红在我这边）。
修法：新增 `matrix_name_of(Style) -> &'static str` 做名目归一，`absence_reason` / `is_active` 都走它，
并加 `bottleneck_style_resolves_to_the_serenity_row` 同时锁「Style 入参」与「名目入参」两条路给出同一答案。

### 复验读数（2026-09-30）

- `cargo check -p axagent --all-targets`：**EXIT=0**，日志正文只有 `Finished dev profile … in 11m 58s`、0 条 `error` ⇒ S1 的 Rust 侧成立。
- `cargo test -p axagent-analysis-engine --lib`：**548 过 1 红**，唯一红条是 `recommender::tests::cache_live_and_replay_are_isolated`
  （他人写的 `RESULT_CACHE` 隔离测试）—— **隔离单跑 1 passed / 0.00s**，且同一棵树上一轮全量它是绿的
  （那轮红的是我自己新写的契约测试）⇒ 判定时序敏感 flaky，不改别人的测试。
  我本轮两条新测（`contract_cells_cover_the_matrix_regardless_of_samples`、
  `bottleneck_style_resolves_to_the_serenity_row`）在该轮全绿。
- `cargo clippy -p axagent-analysis-engine --all-targets -- -D warnings`：**EXIT=0**。
- 前端：`typecheck` 0 · `oxlint` 0 · `vitest src/components/stock-analysis src/lib/__tests__` **1199/1199**（含新增 4 例）
  · `check:i18n:key-parity` 11/11 · `check:eol` 过 · `contracts --only=a,c,d,f,g,h` PASS · `check:serde` **转绿** · `check:paired`/`check:types` 过。
- 门 `check-reco-horizon-parity.mjs`：a~h **全绿**（f 扫描面 5、g 判定面 55、h 判定面 18），`--selftest` 21 条 PASS / 0 FAIL。

### 门 g 首跑即红，红在门自己身上（两条同族形态，已收）

1. 扫描面写成「全文匹配 `Some("…")`」⇒ 我新加的 Rust 测试正文里的 `Some("cell_is_active")`（出票哨兵，不是理由码）
   被当成契约码，门去要一个根本不存在的翻译 ⇒ 报 11 处红。改为只解析 `const MATRIX … = [ … ];` **声明体**，
   并让「声明体解析不到」本身成为一条违规（判据失效不得静默绿）。
2. 加了声明体解析后，负控夹具 `RENAMED_MATRIX` 又证明「常量名不带词边界 ⇒ 改名后的常量照样被当成原契约解析」
   ⇒ `arrayBody()` 的正则补 `(?:^|[^A-Za-z0-9_])` 前缀。
3. 夹具 `want` 记账：我先写 12，实算 1（集合里剩下的 `cell_not_in_matrix` 是真表里**已有翻译**的兜底码，不重复计红）
   —— 按「该夹具实际命中的违规数」算，并在夹具旁留下错成什么的备注。

判据层登记：**#712 追加 ④**（扫描面必须锚在声明处并带词边界；测试正文不是契约）。

## 七、Q1 / Q2 落地（2026-09-30 第二轮）

用户裁定：**Q1=B**（建仓带 = 该档止损距离的一半）、**Q2=工作流链也出 long 档**。

### Q1=B 建仓带

- 推导收成一个函数：`recommender/risk.rs::entry_band_pct(vol_stop_pct, band_stop_pct, fallback_range_pct)`
  → `(带宽%, 来源)`，三档来源 `half_stop` / `half_stop_fallback_pct` / `fallback_range` 各自可判读。
  **两链都调它**（工作流链 `commands/stock_workflow/serenity.rs`、策略链 `strategies/serenity.rs`）：
  一开始我在两条链各写了一遍 `/ 2.0`，那就是本轮要消灭的形态本身，已收口。
- 落库新增 `entrySource` 键；策略链 reasons 出一句「建仓带 ±x%（mid/long 档止损距离的一半，来源 …）」。
- 单测两条：随持有期放大（28 天带 < 90 天带）、三种来源可区分、NaN 不当成可得止损。
- 门 h 加唯一性锁：`risk::entry_band_pct(` 必须在两链各出现一次，且链文件里不得再出现内联 `/ 2.0`。

### Q2 工作流链双档

- `serenity_tiers = [Period::Mid, Period::Long]`，每个候选**逐档各落一行**：h 取该档权威天数、
  止损/目标按各自 `√h` 推导、仓位按各自止损距离做风险预算与成本拖累。
- `reco_picks.id` 加档位后缀 `serenity-{ts}-{i}-{code}-{tier}`：**不带后缀会主键相撞**，
  第二档（long）会被 `pick.insert` 静默拒掉 —— 这是双档改造里最容易漏的一条，门 h 已锁。
- 候选自带 `holdingDays` 只对它自己那一档成立 ⇒ 另一档一律按权威天数，且不一致时成句声明。
- 声明面同步：`serenityPanel.tierScopeHint` 11 语言从「仅服务中线 / 落库恒为 mid」改成
  「服务中线与长线两档」，并**刻意不抄 28/90**（权威表是唯一来源）。
- 门 i 新增（面板口径文案 vs 代码）：文案含「恒为 mid / 仅服务中线」即红、缺「长线」即红、
  抄了 `28 天/90 天` 即红；负控夹具用修复前的真实文案（4 条红）。
- 一次运行 → 历史列表出现 mid / long 两条（`list_reco_history` 按 `generated_at, period` 分组，
  档位集合是分组键的一部分），这是 Q2 的预期结果而非缺陷。

### 纪律修正（我自己的两条）

1. **反向锁会被散文注释触红**：`forbid("price * serenity_stop_mult")` 命中了我写的「旧形态」说明注释。
   改为 `stripFullLineComments` 后比对；豁免标记 `reco-scale-exempt:` 必须从**原文**取（它就在注释里）。
2. **复合夹具不再逐条数违规数**（我连着算错两次：want 12→实 1、19→实 20）。selftest 支持 `min` 下限断言，
   「至少红 N 条」足以证明判别力；精确数只留给单判据夹具。

## 七、Q1 / Q2 落地（2026-09-30 第二轮，用户裁定 Q1=B、Q2=工作流链也出 long）

**Q1=B 建仓带 = 该档止损距离的一半**
- 新增唯一实现 `recommender/risk.rs::entry_band_pct(vol_stop_pct, band_stop_pct, fallback_range_pct)`，
  返回 `(带宽%, 来源)`；三种来源 `half_stop` / `half_stop_fallback_pct` / `fallback_range` 各自可判读。
- **两链都调它**：工作流链与策略链各自内联写了一遍 `/ 2.0` —— 那正是本轮要消灭的形态本身，已收口；
  门 h 现在锁「两链必须出现 `risk::entry_band_pct(`」且「链文件里不得再出现 `stop_pct / 2.0`」。
- 落库新增 `entrySource`；策略链 reasons 出一句「建仓带 ±x%（mid/long 档止损距离的一半，来源 …）」。
- `risk.rs` 新增两条单测：带随 √h 放大（28 天 < 90 天）、三态来源可区分、NaN 不被当成可得止损。

**Q2 工作流链双档**
- `serenity_tiers = [Mid, Long]`，逐档各落一行；h 取该档权威天数，止损/目标按各自 `√h`，
  仓位按各自止损距离做风险预算与成本拖累 ⇒ 同一只票在 mid 与 long 两档的口径**各自成立**。
- ⚠ `reco_picks.id` 必须带档位后缀：不带则两档**主键相撞**，第二行被 `insert` 静默拒掉
  （表现是「long 档一条没有」而日志只有 warn）—— 门 h 已把后缀锁住。
- 候选自带 `holdingDays` 只对它自己那档成立 ⇒ 另一档一律按权威天数，不一致时成句声明。
- 声明面同步：`serenityPanel.tierScopeHint` 11 语言改为「服务中线与长线两档」，
  且**刻意不抄 28/90**（权威表唯一来源）；`SerenityScreeningPanel` 的代码注释同步。
- 呈现层新增**门 i**（文案 vs 代码档位集合）：文案含「恒为 mid / 仅服务中线」即红、
  缺「长线」即红、抄了 `28 天/90 天` 即红；负控夹具用修复前的真实文案（4 条红）。
- 连带修正两处会误导后来者的前提：`list_reco_history` 的注释写着「一次执行只对应一个 period，
  加 period 到 GROUP BY 不会拆分分组」—— 双档后**不成立**（同一 generated_at 拆成多条），
  已就地更正；面板恢复候选改为按 `stock_code` 去重、保留较短持有期那一档
  （卡片是「候选」不是「逐档 pick」，长线记录在历史弹窗里仍逐条可见）。

**纪律修正（我自己的两条，已进判据层 #712 ④⑤）**
1. 反向锁会被**散文注释**触红（`price * serenity_stop_mult` 命中我写的「旧形态」注释）
   ⇒ 判据先 `stripFullLineComments` 再比对；豁免标记必须从原文取（它就写在注释里）。
2. 复合夹具不再逐条数违规数（我算错两次：want 12→实 1、19→实 20）⇒ selftest 支持 `min` 下限断言。
3. 判据扫描面必须锚在**声明处**并带**词边界**：全文 `Some("…")` 会把测试正文里的哨兵断言
   当成契约码去要翻译；`RENAMED_MATRIX` 含 `MATRIX` ⇒ 改名后的常量照样被当成原契约解析。

**当前门读数**：`check-reco-horizon-parity.mjs` a~i 全绿（f 5 / g 55 / h 28 / i 5），
`--selftest` 25 条 PASS、0 FAIL；前端 typecheck 0、oxlint 0、i18n key-parity 11/11、EOL 过。
Rust 复验（engine test + clippy + `cargo check -p axagent --all-targets`）在构建锁后跑，读数见下一节。

### 复验读数（第二轮，2026-09-30）

- `cargo clippy -p axagent-analysis-engine --all-targets -- -D warnings` **EXIT=0**（15m57s 正文 `Finished`）
- `cargo test -p axagent-analysis-engine --lib --no-run` **EXIT=0** ⇒ 含 `cfg(test)` 的引擎测试目标可编
- `cargo test -p axagent-analysis-engine --lib recommender` **69 passed / 0 failed**，其中本轮新增两条
  `recommender::risk::tests::entry_band_is_half_stop_and_grows_with_horizon`、
  `entry_band_fallbacks_are_distinguishable` 均 ok
- `cargo check -p axagent --all-targets` **被他人 in-flight 打断**：`commands/pool_scan.rs:201`
  `E0599 no method named and_utc_time`（该文件 `git status` = `M`、HEAD 里没有这一行），
  另有两条 warning 落在 `stock_workflow/hooks.rs:34` / `stock_workflow/reflection.rs:1858`（同为 `M`）。
  同一次编译里 **我的文件零报错**（`stock_workflow/serenity.rs` 在日志中出现 0 次），
  rustc 是整 crate 一次性报所有类型错误 ⇒ Q2 的双档代码类型层无错，但「整条 axagent lib 绿」这一读数**尚未取得**，
  要等他们把 `pool_scan.rs` 修完再跑一次才能宣布。
- 前端/门：`check-reco-horizon-parity.mjs` a~i 全绿 + `--selftest` 25 PASS / 0 FAIL、
  typecheck 0、oxlint 0、EOL 过、i18n key-parity 11/11、面板 vitest 30/30、dprint 我改的文件 0。

**待办收口条件**：① 他人 `pool_scan.rs` 修好后重跑 `cargo check -p axagent --all-targets` 与
`cargo test -p axagent --lib`；② S5 全量三条按 CI 口径跑一次；③ `tauri dev` 实机核对
（一次趋势智选运行要看到 mid **和** long 两条 reco_picks、建仓带随档变化、σ 取不到时 riskNotes 出退化句）。

> 补一条（同日晚些）：`__TAURI_WORKSPACE__=true cargo test -p axagent-analysis-engine --lib`
> **566 passed / 0 failed** —— 上一轮那条时序敏感的 `cache_live_and_replay_are_isolated` 本轮自然绿，
> 再次印证它是并行/TTL 抖动而非缺陷（已登记在项目记忆的 flaky 清单第 15 条）。

## 八、A 裁定落地：呈现层跟上产出层（2026-09-30 第三轮）

**触发**：用户问「为什么前端 UI 没有任何适应性变化」。核查后确认该判断在主面板上成立，根因三条：
① `SerenityCandidate` 类型只有 `holding_days`，从来没有档/风控口径字段（不是本轮改坏）；
② `SerenityCandidateCard.tsx:129` 把天数兜底成 `?? 20` —— 落库侧那个「20 天 vs mid=28 天」的
矛盾我修了，**呈现层的同一个旧值还活着**；
③ 我上一轮在恢复查询里按 `stock_code` 去重、保留较短那档 ⇒ Q2 的 long 档在面板被折叠，
只在历史弹窗可见 —— 等于把产出层的变化挡在呈现层之外。

**改了什么**
- 类型：`SerenityCandidate` 补 `period/price/stopLoss/targetPrice/entryLow/entryHigh/positionPct/stopSource/entrySource`。
- 恢复路径：`restoreCandidate` 从 `pick_data` 逐字段解出；**去掉去重**，改为按 (代码, 档序) 排序，
  让同票两张卡相邻；`pick_data` 损坏时保持全缺省（卡片出声明），不再兜底成数字。
- 卡片：档徽标复用全仓唯一键族 `stockAnalysis.timeHorizon*`（经 `horizonSuffix`），认不出/未标档
  出 `stockAnalysis.reflection.horizonUnknown`（**不猜档**）；时间基线分「有天数 / 无天数」两支，
  后者出 `serenityPanel.timeBasisNoTier`；新增止损/目标/仓位/建仓带四行，
  `stopSource=fallback_pct` 复用既有 `row.rowStopFallback` 句，建仓带退化另出两句
  （`entryBandHalfFallback` / `entryBandRangeFallback`）。
- 面板标题：`(N)` 现在是**逐档行数** ⇒ 与去重票数不等时并列显示「{{stocks}} 只 · 逐档各一行」。
- i18n：5 个新键 × 11 语言（key-parity 一致）。
- 门：**新增 j 段**（卡片必须调 `horizonSuffix(`、必须有 `serenity-tier` 徽标、必须有
  `timeBasisNoTier` 分支、必须分「有/无天数」两支，且反向锁 `?? 20` 不得回来）；
  负控夹具用修复前的真实卡片源码（5 处红）。

**读数**
- `cargo check -p axagent --all-targets`：**MAIN_EXIT=0、0 条 error**（3 条 warning 全在他人
  `pool_scan.rs`：deprecated `timestamp_millis` / unused import / dead code —— 不代修）⇒ Q1/Q2 主 crate 侧编译闭合。
- 引擎：`cargo test -p axagent-analysis-engine --lib` **566 passed / 0 failed**；
  `clippy -p axagent-analysis-engine --all-targets -- -D warnings` **EXIT=0**。
- 前端：新增 10 例（含 4 条反控：无 period 不出档名、未标档不出现 20、波动率路径不挂退化句、
  同票两档两张卡 + 注记）；`vitest src/components/stock-analysis src/stores` **653/653**；
  typecheck 0、oxlint 0、EOL 过、i18n key-parity 11/11、dprint 0。
- 门 a~j 全绿（f 5 / g 55 / h 28 / i 5 / j 5），`--selftest` **27 条 PASS、0 FAIL**。

**仍欠**：`tauri dev` 实机核对（同票两张卡、建仓带随档变化、实时候选出「未知周期」）；
S5 三条全量门禁（会被他人 `pool_scan.rs` 的 3 条 warning 在 `-D warnings` 下挡住，归属已标明）；
提交切分待用户发话（本轮全部改动仍未提交，树里另有他人在制品）。
