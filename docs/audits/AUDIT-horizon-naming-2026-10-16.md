# AUDIT — 股票业务四周期（超短/短/中/长）命名与时间口径一致性

> 状态：active ｜ 日期：2026-10-16 ｜ 对应版本：v2.9.9


- **日期**：2026-10-16
- **范围**：`src/`（前端 i18n / 组件 / 类型）、`src-tauri/`（harness 权威、rhai 装配、agent prompt、schema）、`scripts/`（门禁）
- **触发**：UI「周期概览」显示 `超短线 / 短线 / 中期 / 季度`，第 4 档应为「长期」—— 用户报「长期被季度混淆」
- **结论**：**时间数值唯一且一致；档名（中文标签）不唯一 —— 前端 8 套键族 + 后端 2 处，共 9 种拼法**，第 4 档存在**实质语义冲突**（后端「长线」vs 前端「季度」），且冲突原因是一个**单位换算错误**

---

## 一、结论摘要

| 维度 | 是否唯一 | 判定 |
|---|---|---|
| 档位键（`ultra_short`/`short`/`mid`/`long`） | ✅ 唯一 | Rust `Period` 枚举，snake + camel 两种拼写均为单源 |
| 持有天数（2 / 5 / 28 / 90） | ✅ 唯一 | `Period::default_holding_days()`，**单位 = 交易日** |
| 档位序 | ✅ 唯一 | `Period::ALL` = 变体声明序（短→长） |
| 尺度名（hourly/weekly/monthly/quarterly） | ✅ 唯一 | `Period::scale_key()` → `astock-data::scale` |
| **档名（中文标签）** | ❌ **不唯一** | **前端 8 套 i18n 键族 + 后端 3 处硬编码**，9 种拼法 |
| **第 4 档档名** | ❌ **自相矛盾** | 后端「**长线**」× 3 处 vs 前端「**季度**」× 8 套 |
| 天数在**文案层** | ❌ 有手抄 | `reflectionScanHint` 手抄 2/5/28/90（违反单一来源） |

---

## 二、权威定义（唯一源）

`src-tauri/crates/harness/src/holding_period.rs` —— 文件头即声明「禁止在任何 crate 再定义一份周期枚举或周期→持有天数表」。

```rust
// holding_period.rs:31-41
pub enum Period { UltraShort, Short, Mid, Long }   // serde: ultra_short / short / mid / long

// :294-301  建议持有天数
Period::UltraShort => 2, Period::Short => 5, Period::Mid => 28, Period::Long => 90
// :309 注释：「单位 = 交易日，与 default_holding_days 同口径（K 线 bar 位移天然只数交易日）」

// :330-337  技术面尺度
Period::UltraShort => "hourly", Short => "weekly", Mid => "monthly", Long => "quarterly"

// :477-484  档位中文标签
Period::UltraShort => "超短", Short => "短线", Mid => "中线", Period::Long => "长线"
```

**四档的「持有期」与「评分尺度」是两个不同的量**，权威表里分列：

| 档 | `default_holding_days`（交易日） | `scale_key` | 尺度层 `trading_days_per_bar` | 持有期 ÷ 1 bar |
|---|---|---|---|---|
| `ultra_short` | 2 | hourly | 0.25 | 8 |
| `short` | 5 | weekly | 5 | 1 |
| `mid` | 28 | monthly | 20 | 1.4 |
| `long` | **90** | **quarterly** | **60** | **1.5** |

（第 4/5 列：`astock-data/src/scale.rs:49` 注释「1 周 = 5 日、1 月 ≈ 20 日、**1 季 ≈ 60 日**」；`:100-111` `quarterly()` 的 `trading_days_per_bar: 60.0`）

---

## 三、档名实测全表（zh-CN）

### 3.1 后端（用户可见的 3 处）

| # | 载体 | 超短 | 短 | 中 | **长** |
|---|---|---|---|---|---|
| B1 | `harness/src/holding_period.rs:477-484` `Period::label_zh()` | 超短 | 短线 | 中线 | **长线** |
| B2 | `src/commands/portfolio-mgr.rhai:3434-3436` `cn_tier` | 超短线 | 短线 | 中线 | **长线** |
| B3 | `agency_experts/stock-analysis/reflection.md:237-240` | 超短线 | — | — | **长线** |
| B4 | `config/llm-output-schemas.json:507` | 超短（2 日） | — | — | **长线（90 个交易日）** |
| B5 | `crates/analysis-engine/src/intent_parser.rs:36-44` 意图词表 | 超短/超短线 | — | — | **长线** |

### 3.2 前端 i18n：8 套键族（`src/i18n/locales/zh-CN.json`）

| # | 键族（行号） | 超短 | 短 | 中 | **长** |
|---|---|---|---|---|---|
| F1 | `stockAnalysis.period.*`（15616-15619） | 超短线 | 短线 | 中线 | **季度** |
| F2 | `stockAnalysis.periodUltraShort…periodLong`（15660-15663） | 超短线 | **短期** | **中期** | **季度** |
| F3 | `stockAnalysis.timeHorizon*`（15699-15702） | 超短线 | 短线 | **中期** | **季度** ← **截图所用** |
| F4 | `stockAnalysis.recommendation.period*`（17118-17121） | 超短线 | **短期** | **中期** | **季度** |
| F5 | `stockAnalysis.reflection.horizon*`（17507-17510） | 超短线 | 短线 | 中线 | **季度** |
| F6 | `stockAnalysis.scheduledAnalysis.period.*`（17351-17354） | 超短线 | 短线 | 中线 | **季度** |
| F7 | `replayWorkbench.batch.period.*`（14999-15002） | **超短** | 短线 | 中线 | **季度** |
| F8 | `settings.scheduled-recommendation.period-*`（6580 区） | 超短线 | 短线 | 中线 | **季度** |

**8 套键族的漂移点**：

- 第 1 档：`超短`（F7）vs `超短线`（其余 7 套）
- 第 2 档：`短期`（F2/F4）vs `短线`（其余 6 套）
- 第 3 档：`中期`（F2/F3/F4）vs `中线`（F1/F5/F6/F7/F8）
- 第 4 档：**全部 8 套 = 季度**，与后端 3 处的「长线」**全部冲突**

### 3.3 另外两套副本（首轮漏查，回退时才发现）

| # | 载体 | 形态 | 首轮为何漏 | 回退时实测 |
|---|---|---|---|---|
| C1 | `e2e/stock-reflection-four-horizon.spec.ts:232/240/259/269` | `horizonTab(expanded, "🟢 长线")` —— 4 条**硬编码档名锚点** | 只按 `src/**` 与 `src-tauri/**` 扫，e2e 在仓库根另一棵树 | **仍写 `"🟢 季度"`**：i18n 回退后**没有跟随**，全门仍绿 ⇒ 这是**第 10 套副本**，且已实际漂移 |
| C2 | `src/lib/stock-analysis-export.ts:164-169` `labelMap` | 纯静态 `超短线/短线/中线/长线`（无 i18n） | 该文件在 `src/lib`，不属 i18n 面 | 值与目标**逐字一致**，无需改（属**被动对账**：靠判据 D 同类思路守住） |

> **教训（可复用）**：审计"同一事实的副本"时，扫描面必须覆盖**非产物树**（`e2e/`、`scripts/`、`docs/`），
> 否则"共 N 套"这个数字本身就是漏报 —— 本次首轮报「9 套」实为 10 套。判据 D 已把 C1 纳入对账。

> **同名不同义（勿误伤）**：`src/lib/stock-analysis-utils.ts:755-756` 的
> `"短期" → catalystShortTerm` / `"中期" → catalystMidTerm` 属**催化剂兑现时效**轴
> （短/中/长期三种催化剂），与四周期**持有期档名**是两回事，共用「短期/中期」字面。
> 同理 `ja` 的档名 `超短期/短期/中期/長期` 是**正确日文**（已实测非漂移），
> 中文 grep 命中「短期/中期」不代表档名回潮。判据只看**8 套键族的完整路径**正是为避开此类误伤。

> 截图「周期概览」= `HorizonDecisionStrip.tsx:55` / `DecisionBanner.tsx` 读 `stockAnalysis.timeHorizon${suffix}`（`HORIZON_T_SUFFIX`，`stock-analysis-utils.ts:1401-1406`）⇒ 命中 F3 ⇒ `超短线 / 短线 / 中期 / 季度`，与截图逐字一致。

---

## 四、「季度」混淆的根因：把**尺度名**当成了**周期名**

这不是笔误，是一次**基于错误单位前提的改名**。

**改名记录**（`docs/plans/PLAN-stock-decision-reliability.md`）：

```
:189  - **长期档正名**：要么做真长期（≥180 天、接受大止损、估值驱动），
        要么把 90 天档改名为"季度"并以资金/趋势腿为主，估值只作风险过滤。
:243  - P2-d：✅ 长期档正名「季度」（11 语言 × 11 键 + 相关测试断言同步）。
```

**前提错误**：这句话里的「90 天」被当作**自然日**（90 自然日 ≈ 1 季度，成立），但代码里的 90 是 **90 个交易日**：

```
90 交易日 = 18 周 ≈ 126 自然日 ≈ 4.2 个自然月 ≈ 1.5 个「尺度季度」（1 季 = 60 交易日）
```

所以：
- 「季度」在**尺度层**自洽（`Scale::Quarterly`，1 bar = 60 交易日）
- 「季度」在**持有期层**不自洽（long 档 = **90** 交易日 ≠ 60）

**自相矛盾的直接实证** —— 同一句文案里两个数互斥：

```
zh-CN.json:17317
"reflectionScanHint": "按超短线 2 天 / 短线 5 天 / 中线 28 天 / 季度 90 天的间隔，自动反思已完成的股票分析"
                                                   ↑「季度」        ↑ 90 天
```

若「季度」= 1 季 = 60 交易日，则「季度 90 天」是把两套口径焊在同一句里。**用户读到的就是"90 天 = 一个季度"，这正是报障的认知来源。**

另外这句**同时**违反了「天数唯一来源」纪律（`check-horizon-weight-parity.mjs` 判据 ④「档名标签不得手抄天数」）—— 见 §六。

---

## 五、同屏两名：用户能同时看到「长线」和「季度」

改名只改了前端 i18n，**后端权威未同步** ⇒ 同一档在同一屏上两个名字：

| 用户可见面 | 名字 | 来源 |
|---|---|---|
| 「周期概览」条（截图） | **季度** | i18n F3 |
| 逐档对照表 `agreementByHorizon[].tierLabel` | **长线** | 后端 `decision.rs:1926` `tier_label: period.label_zh()` → `core.rs:1400` 序列化 `tierLabel` |
| 逐档分歧归因 `divergenceByHorizon[].tierLabel` | **长线** | 同上（`core.rs:1428`） |
| 「数据缺口 N 项」横幅 | **长线** | `portfolio-mgr.rhai:3435` `cn_tier` → `decision.rs:1562` → `dataGaps` 文本「长线分支未产出…」 |
| LLM 系统提示（间接） | **长线** | `holding_period.rs:425-474` `verdict_contract_prompt()` 用 `label_zh()` |

`HorizonAgreement.tierLabel` 的类型注释本身写着：

```
src/types/stock-analysis.ts:431
/** 档位中文名（产端 `Period::label_zh`，前端不再手抄第四份档名） */
```

—— 设计意图是「**不再手抄第四份**」，但实际前端已有 **8 套**手抄。「季度」正好是第 9 套。

---

## 六、门禁覆盖缺口（为什么「编译通过 + 全门绿」没拦住）

| 门 | 应锁 | 实际覆盖面 | 缺口 |
|---|---|---|---|
| `check-horizon-weight-parity.mjs:332-345` 判据 ④ | 档名标签不得手抄天数 | `readHorizonLabels()` **只读 `stockAnalysis.reflection` 段**的 4 个 `LABEL_KEYS` | 8 套键族里只覆盖 **1 套**（F5）；`reflectionScanHint` 不在 `LABEL_KEYS` 内 ⇒ 手抄天数未被发现 |
| `check-reco-horizon-parity.mjs:299-315` 门 i | 面板文案须声明长线档 | 判据已改为 `/仅服务中线\|只服务中\/(长线\|季度)/` 与 `/(长线\|季度)/` | **放宽判据以适配改名** —— 门本身记录了这一点：「若只认旧名，改名后这条锁会**静默失效**」 |
| 全仓 | `Period::label_zh()` ↔ i18n 对账 | **不存在此门** | 前后端两个名字可以长期并存而无人报警 |

**形态定性**：这不是「漏检某一处」，而是**档名这条链整体没有单一真相门**。`Period::label_zh()` 声称是权威（注释「注入文案与荐股面板用同一串，不再各处手抄」），但它与前端 8 套键族之间**零对账**。

---

## 七、建议（**涉及产品语义，未擅改，待裁决**）

### 方案 A（推荐）：回退「季度」改名，第 4 档统一为「长线」

**理由**：rename 的前提（90 天 = 1 季度）在代码单位上不成立 —— 90 是**交易日**（≈4.2 自然月 ≈1.5 季度）。而 `quarterly` 是**尺度名**（1 bar = 60 交易日），把尺度名当周期名会让「持有 90 交易日」被读成「持有 3 个月」。

改动面：
1. 前端 8 套键族 × 11 语言的 `long` 档值：`季度 → 长线`（zh）、`Quarterly → Long-term`（en）等
2. 修 `reflectionScanHint` 的手抄天数（改为不写数字，或引用权威）
3. 补一道门：`Period::label_zh()` ↔ i18n 四档名对账 + 键族收敛检查

### 方案 B：保留「季度」，把改名贯彻到底

同步后端 5 处（`label_zh` / `cn_tier` / `reflection.md` / `llm-output-schemas.json` / `intent_parser` 词表）+ 11 语言 8 套键族。
**代价**：需接受「季度 = 90 交易日」与尺度层「1 季 = 60 交易日」永久并存，且四档名从「持有期语义」变为「尺度语义」，`mid`（28 交易日，尺度 monthly 而言 1.4 bar）也应相应改名为「月度」才自洽 —— 但那样第 3 档又不叫「中期」了，与用户直觉（超短/短/中/长）背离。

### 方案 C：让名字成真 —— 把 long 档持有期改为 60 交易日

名实相符（= 尺度定义的 1 季），但**改动了策略参数**（影响先验、反思到期、回测窗口、Kelly 持有期），属策略级变更，不建议与命名问题捆绑。

### 独立于方案的必做项

1. **8 套键族收敛为 1 套**（推荐 `stockAnalysis.timeHorizon*`，因其已被 `horizonSuffix()` 选为「全仓唯一档名键族」，`check-reco-horizon-parity.mjs:397` 已有此判据）—— 第 2/3 档的「短期/中期」vs「短线/中线」需一并裁决
2. **加门**：① `label_zh()` ↔ i18n 对账；② 档名键族数量断言（>1 即红）；③ 把 `reflectionScanHint` 类文案纳入「不得手抄天数」
3. **统一 `intent_parser` 词表**：无论选 A/B，用户输入「长期」「长线」「季度」都应能解析到 `long`（现状只有「长线」）

---

## 八、复现与验证命令

```bash
# 档名全表（11 语言 × 9 键）
node scripts/check-horizon-weight-parity.mjs    # 判据 ④ 只覆盖 reflection 段，故全绿
node scripts/check-reco-horizon-parity.mjs      # 门 i 已认「长线|季度」两种写法

# 权威天数
rg "default_holding_days" src-tauri/crates/harness/src/holding_period.rs

# 尺度 vs 持有期
rg "trading_days_per_bar" src-tauri/crates/astock-data/src/scale.rs

# 后端「长线」全部载体
rg "长线" src-tauri/ src/
```

---

## 九、影响面清单（供裁决后排期）

| 层 | 文件数 | 说明 |
|---|---|---|
| 前端 i18n | 11 语言 × 8 键族 ≈ 88 键值 | 改 1 档名 × 8 族 × 11 语言 |
| 前端组件 | 0（全走 i18n） | 无需改组件，除非收敛键族 |
| 后端 Rust | 1（`holding_period.rs:477-484`） | 仅方案 B 需要 |
| Rhai | 1（`portfolio-mgr.rhai:3435`） | 仅方案 B 需要 |
| Agent prompt / schema | 2（`reflection.md`、`llm-output-schemas.json`） | 仅方案 B 需要 |
| 测试断言 | ≥4 文件 | `RecommendationPanel.horizonPresentation.test.tsx:175,515`、`SerenityCandidateCard.tier.test.tsx:173,198`、`ValueAssessmentPanel.render.test.tsx:452` |
| 门禁脚本 | 3 | 见 §六、§七 |

---

## 十、执行结果（2026-10-16 下午，按用户裁定的**方案 A**）

**落点**：第 4 档 `季度 → 长线`，第 1/2/3 档统一为 `超短线 / 短线 / 中线`；权威仍是 `Period::label_zh()`。

| 面 | 改动 | 实测 |
|---|---|---|
| i18n 8 键族 × 11 语言 | 264 处档值统一（含消掉 ar/hi/ja 等多语言的**未翻译英文占位** `Short \| Short \| Mid`） | `check-horizon-label-parity` 11 语言 × 8 族 × 4 档**逐字一致** |
| 手抄天数 | 33 处文案去掉手抄天数（`reflectionScanHint`、`serenityPanel.tierScopeHint`、回测缺席项） | `check-horizon-weight-parity` 判「档名标签未手抄天数」✅ |
| 后端权威 | `label_zh()` 第 1 档 `超短 → 超短线`，重写 doc 注释（载明第 4 档**不是**季度） | 与 Rhai `cn_tier` / 导出 `labelMap` 逐字一致 |
| 输入解析别名 | `intent_parser.rs` 补 `("季度","long")`，**注明仅作输入别名、非展示名** | 用户口语仍可用，展示面不再混 |
| 测试/规格 | 3 个 vitest 文件 + `e2e/stock-reflection-four-horizon.spec.ts` 4 处锚点 | vitest 75/75；e2e 由判据 D 守住 |
| 文档 | `PLAN-stock-decision-reliability.md` 的 P2-d 改判 **❌ 已回退**（第 189/243 行）并加回退批注 | 遗留待办「长期档『季度』文案」随之消解 |

**新增门禁** `scripts/check-horizon-label-parity.mjs`（已注册 `ci-check.mjs` + `ci.yml`，两处逐字同源）：

| 判据 | 内容 | 自证 |
|---|---|---|
| 0 | 任一扫描面为 0 / 权威解析失败 / 语言 < 11 ⇒ 判红（**0 命中 ≠ 没问题**） | 正控 4/5 + 0/D |
| A | 同语言内 8 键族四档**逐字相同** | 正控「单族漂移」 |
| B | 档名禁含**尺度词**（季度/Quarterly/四半期/분기/…） | 正控「第 4 档改回季度」 |
| C | zh-CN 逐字等于 `label_zh()`；zh-TW 等于登记映射 | 正控「**全族一致地错**」⇒ A 放过、C 必抓 |
| **D** | **e2e 硬编码档名锚点**（剥徽标后）∈ 权威值集；抽不到锚点也判红 | 正控「🟢 季度」（复刻本轮实测漏网形态）+ 0/D |

`--selftest` = **7 正控 + 1 负控**全绿；正式跑 **扫描点 100**（96 = 11×8 族判据 A/B/C… + 4 e2e 锚点）。

**同日恢复的严格性**：`check-reco-horizon-parity.mjs` 门 i 此前被**放宽**成认「长线|季度」两种写法
（= 改判据去适配改名）；现已恢复「只认长线 + **显式禁季度**」，并新增独立负控（`want: 1`）。

**未决 / 已知阻塞**：`cargo check` 本轮**未能重跑** —— `target/debug/.cargo-build-lock` 被**其它会话**的
cargo 进程持有（`cargo test -p axagent --lib commands::stock_workflow` 起于 21:55:46、
`cargo run --features computer-use` 起于 22:02:12）。按项目纪律**不抢锁、不杀他人进程**；
以 `cargo fmt --all --check` **EXIT=0**（rustfmt 须解析全部 Rust 源码 ⇒ 语法成立）作为等价证据，
Rust 侧改动仅为 `label_zh()` 的**字符串字面量**与词表新增一行，无类型面变更。

---

## 十一、收敛（同日第二批）：把 7 套冗余族**删掉**，门禁从「族间一致」换成「唯一族 + 禁回流」

**触发**：方案 A 只把 8 套族的**值**统一了，**定义仍是 8 份** —— 任一族漏改即前后端同屏两名，
且新增档位要改 8 处。用户指出「只统一了值、未做收敛任务」⇒ 本批做**结构性删除**。

> **纪律**：值统一 ≠ 定义统一。冗余定义必须**删除**；删完把门禁从「族间一致」改成
> 「**唯一族 + 禁回流**」—— 此后防的不是不一致，是**重复定义再生**。

### 11.1 删除面（7 族 × 4 键 = 28 键 + 2 个被删空的壳）

| # | 已删族（4 档/族） | 备注 |
|---|---|---|
| 1 | `stockAnalysis.period.{ultraShort,short,mid,long}` | 该父对象**继续保留**（K 线粒度键与 `zoomHint` 在里面）|
| 2 | `stockAnalysis.period{UltraShort,Short,Mid,Long}` | |
| 3 | `stockAnalysis.recommendation.period{UltraShort,Short,Mid,Long}` | |
| 4 | `stockAnalysis.reflection.horizon{UltraShort,Short,Mid,Long}` | 保留同父下的 `horizonUnknown` / `horizonAuto` |
| 5 | `stockAnalysis.scheduledAnalysis.period.*` | **壳一并删除**（`"period": {` + 4 子键 + `}`）|
| 6 | `replayWorkbench.batch.period.*` | **壳一并删除** |
| 7 | `settings.scheduled-recommendation.period-*` | |

**行级实测**（与 `output/backup-2026-10-16/locales/` 逐文件 diff）：

| 读数 | 值 |
|---|---|
| 每语言 | **33 删 / 1 增** |
| 11 语言合计 | **363 删 / 11 增** |
| 那「1 增」是什么 | `"zoomHint": "框选缩放",` → `"zoomHint": "框选缩放"` —— 整段尾部 run 删完后**逗号归正**（不是新增键）|
| 33 删的构成 | 28 个叶子键 + 2 个壳的**名字行** + 2 个壳的**闭合 `}` 行** + 1 处逗号归正 |
| 保留成员扰动 | **0**（`zoomHint`/`5m`/`1M`/`1y`/`horizonUnknown`/`recommendation.title`/`scheduledAnalysis.decisionBacktestPeriodAll` 等逐字节未变）|

**判据自证（防「禁回流」空过）**：把 `RETIRED_FAMILIES` 的 28 条路径逐条回灌**删除前**的备份 JSON，
确认 **28/28 在删除前确实存在**（2 个壳在 11/11 语言里都存在），当前 **28/28 + 2 壳全部不存在**。
若路径写错（写成本就不存在的路径），「禁回流」会**恒真空过** —— 这一步就是把该风险显式排掉。

**副作用（正向）**：`stockAnalysis.period` 从此**只**装 K 线粒度键（`1m/3m/6m/1y/weekly/monthly/5m/15m/30m/60m/1d/1w/1M`）
+ `zoomHint`。删除前该父对象里**同时**躺着持有期键（`ultraShort/short/mid/long`）与粒度键
（`5m`/`1M`）—— 同一个 `period` 名下两个量纲，这正是「必须按**完整路径**逐条删、不能按同键名做文本替换」的原因。

> **核过一处「疑似漏网」而实为误报**：`KLineChart.tsx:543` 写着 `t(\`stockAnalysis.period.${curTierInfo.key}\`)`
> —— 模板串不是字面量，`check_i18n.py --strict` 抓不到它，看着像漏改。追 `curTierInfo → findTier()`
> 的 `TIERS.key` 域 = `5m/15m/30m/60m/1d/1w/1M`（**K 线粒度**）⇒ 引用的是**保留**命名空间，非缺陷。
> 7 个粒度键经核**全部存在**（`5分钟/15分钟/30分钟/60分钟/日线/周线/月线`）。**不擅改一个「看着像」的调用点。**

### 11.2 消费端：6 处改走单一存取器 `horizonLabelKey()`

| 文件 | 改法 |
|---|---|
| `src/lib/stock-analysis-utils.ts` | **新增** `horizonLabelKey(key)`：`horizonSuffix()` → `` `stockAnalysis.timeHorizon${suffix}` ``；不认识则 `null`（调用方 `?? 原值`，不再猜前缀）|
| `StockSearchBar.tsx` | 删局部 `HORIZON_LABELS` 表 ⇒ `t(horizonLabelKey(intent.timeHorizon) ?? intent.timeHorizon)` |
| `AnalystReportGrid.tsx` | 2 处 `t(\`stockAnalysis.period.${…}\`)` ⇒ `t(horizonLabelKey(entry.tier) ?? entry.tier)` |
| `batchTables.tsx`（time-travel） | 2 处 `t(\`replayWorkbench.batch.period.${p}\`, {defaultValue:p})` ⇒ `t(horizonLabelKey(p) ?? p)` |
| `TimeTravelBatchCard.tsx` | 同上 |
| `ReflectionPanel.tsx` | 局部 `horizonKeyLabel()` → 全走 `horizonLabelKey()`，`null` 兜到 `horizonUnknown`；tab label 同步 |
| `InvestTaskFields.tsx` | **两张只差键前缀的同形表**（`REFLECTION_PERIOD_OPTIONS` / `RECO_PERIOD_OPTIONS`）**并成一张** `PERIOD_OPTIONS` |

配套：「档名落在哪个面板都不改变，故**不随宿主页命名空间走**」这句写进 `InvestTaskFields.tsx` 注释 ——
解释**为什么**档名族不叫 `scheduledAnalysis.*` / `settings.*`。

### 11.3 门禁改造：判据 A（族间一致）→ **E（禁回流）+ F（四档互异）**

`scripts/check-horizon-label-parity.mjs` 结构变化：`FAMILIES`（8 套） → **`FAMILY`（唯一族 4 条）** +
**`RETIRED_FAMILIES`（7×4 = 28 条路径须全部不存在）** + **`RETIRED_SHELLS`（2 个壳）**。

| 判据 | 内容 | 正控 |
|---|---|---|
| 0 | 任一扫描面为 0 / 权威解析失败 / 语言 < 11 ⇒ 判红 | 3 条（含 0/D）|
| B | 档名禁含**尺度词** | 1 条 |
| C | zh-CN 逐字 = `label_zh()`；zh-TW = 登记映射 | 1 条（**全族一致地错** ⇒ B 放过、C 必抓）|
| D | e2e 硬编码档名锚点 ∈ 权威值集 | 1 条（复刻真实漏网形态 `"🟢 季度"`）|
| **E** | 28 条路径 + 2 壳**不得复活**（「值一样」不是理由）；含「清单被抽空」护栏 | **1 条（新增）** |
| **F** | 唯一族四档值**必须互异**（4 档抄成同一串时 C 不报 ⇒ 本判据是那格空白） | **1 条（新增）** |

`--selftest` = **8 正控 + 1 负控**；正式跑 **扫描点 1433 → 67**（比对面从「11×8 族」收敛为「11×1 族 + e2e 4 锚点」）。

`scripts/check-horizon-weight-parity.mjs` 判据 ④ 的**读取路径随收敛迁移**：`LABEL_KEYS`
`horizon*` → `timeHorizon*`、`readHorizonLabels()` 由 `d?.stockAnalysis?.reflection` → `d?.stockAnalysis`、
错误前缀同步。**不迁移的后果是恒定 `undefined`** ⇒ 假红或假绿（已在该脚本加「⚠ 2026-10-16 收敛」批注）。

### 11.4 反面教材：判据 G（消费端禁回流）**写过一版，撤回**

直觉该加「`src/**` 不得再出现已删族字面量」—— i18n 键是**无类型字符串**，`tsc` 不会为
`t("stockAnalysis.period.short")` 报错。但**已有门覆盖**：`check_i18n.py --strict` 的
`missing = code_keys - zh_keys`（已注册 `ci-check.mjs` + `ci.yml`）。**实测**（探针文件已删）：
往 `src/` 放一句该调用 ⇒ 该门输出「❌ 代码引用但 zh-CN 缺失 (1)」并 exit 1。

撤回的版本还有个更贵的教训：它**自己写状态机剥注释**，被 `stock-analysis-utils.ts:231` 一个**含引号的正则**
`/["']?\s*(看多|看空|…)/i` 带偏 —— 字符类里的 `'` 被当成字符串起点，其后 5.6 万字符全被当字符串
⇒ 注释没剥掉 ⇒ 对我**自己写的文档注释**误报。

> **判据的测量工具自己错，比缺陷更贵。** 同型缺陷也**不铺两份实现**（本仓纪律）。本轮另有一例同源教训：
> 临时写的「CI ↔ 本地镜像门禁步骤集」比对脚本**只解析 `run: X` 同行写法、漏了 `run: |` 多行块**
> ⇒ 产出「本地比 CI 多 66 条门禁」的**假缺口清单**。修解析器后真实数字是 **71 vs 70**。

### 11.5 本轮**额外**发现（超出收敛范围，已修）：两道 i18n 门禁只在 CI

`scripts/ci-check.mjs` 头部自述「执行**所有** CI 检查步骤」，实测其 i18n 段**少两道**：

| 门禁 | 加入 CI 的时间 | 本地镜像 |
|---|---|---|
| `check-i18n-key-parity.mjs`（zh-CN ↔ 其余 10 语言的**双向叶子路径差集**）| 2026-09-21 | ❌ 一直缺 |
| `check-i18n-placeholder-usage.mjs`（值不得==键名；`{{x}}` 必须被插值）| 2026-10-01 | ❌ 一直缺 |

而 `src/__tests__/ci.workflow.test.ts` 的「同步护栏」当时**只守 2026-09-14 那三道** ⇒
两道门「CI 有、本地无」持续近一个月而**没有任何东西红** = **本地绿冒充 CI 绿**。
形态与文件头那两条反面教材同源：**守卫只守它被写下的那一刻已存在的门。**

**已修三处**（与 `.github/workflows/ci.yml:405/418` 命令行**逐字同源**）：
① `ci-check.mjs` 补两条 step + 段注「三道 → 五道」；
② `ci.workflow.test.ts` 的 `I18N_STEPS` 由 3 条扩到 5 条、标题/批注同步、本地镜像断言补 2 条；
③ 阴性对照自证：把两条命令行从 `ci-check.mjs` 摘掉后断言**确实变假**（非恒真空转）。
守卫测试 **17/17** 通过。

> 本项**直接关系**本批安全性：33 键 × 11 语言的删除，唯一能证明「删得**对称**」的门正是 ④
> `check-i18n-key-parity` —— 而它此前在本地跑不到。

### 11.6 收敛批验证读数（全绿）

| 项 | 读数 |
|---|---|
| `check-horizon-label-parity`（`--selftest` / 正式） | 8 正控全红 + 负控绿 / 扫描点 **67**，RUN_EXIT=0 |
| `check-horizon-weight-parity` | ✅ 4 档 × 44 键、桥表 12 腿、档名 11 语言 × 4 键 |
| `check-i18n-key-parity` | ✅ 11 语言 **15911** 条路径集合一致（删 33 键后仍一致）|
| `check_i18n.py --strict` | ✅ 全部通过（代码引用 key 全在 zh-CN 中）|
| `check-tier-purity` / `check-reco-horizon-parity` / `check-horizon-decision-keys` | ✅ |
| `ci.workflow.test.ts` | ✅ **17/17** |
| vitest 三组 | `TasksCenterPage` + `time-travel` **107/107**；`stock-analysis` **411/411**；`lib` + `utils` + `ci.workflow.test.ts` **1109/1109** |
| `tsc --noEmit` / `dprint check` | EXIT=0 / EXIT=0 |
| `cargo check` | ⛔ **仍未能重跑**（他人持锁，见 §十末；Rust 侧本批仅改 doc 注释）|

**收敛后真红一例（已修）**：`ReflectionPanel.horizon.test.tsx` 用 i18n **键名前缀**定位 Tab
（`LABEL = "stockAnalysis.reflection.horizon"`）⇒ 键族换后 1 例红 ⇒ 改为 `stockAnalysis.timeHorizon`。
**这就是「i18n 键是无类型字符串」的另一面**：键族改名不会让 `tsc`/`dprint` 报，但会让**断言**报。

**保留的别名（有意，不删）**：`intent_parser.rs` 的 `("季度","long")` —— 用户口语输入面，注释已载明
「**仅作输入解析别名、非展示名**」；`e2e` 4 条锚点在判据 D 之下，档名与 UI 不再脱钩。

### 11.7 载荷抖动一例（**非本批缺陷**，如实登记）

四组并行跑（`stock-analysis` + `lib` + `utils` + `ci.workflow.test.ts`）时，
`ReflectionPanel.cronConfig.test.tsx › 回读库里任务配置：显示 prompt 里的最低置信度` **5s 超时**（实测 5150ms），
当轮读数为 `1 failed | 1519 passed`。

| 判据 | 读数 |
|---|---|
| 单跑该文件 | **4/4 通过**，且该用例只用 **717ms**（阈值 5000ms ⇒ 7 倍余量）|
| 复跑同四组 | **131 文件 / 1520 通过**，EXIT=0 |
| 该轮负载指纹 | `import 639.90s`、`environment 404.30s` 累计（同一时刻**另一会话正在 cargo 编译**）|
| 该文件是否碰档名 | **否** —— 用 `configNotSet`/`configNotReadable`/`saveConfigBtn`，与 `horizon*` 无关 |

⇒ 结论：**重载下的超时抖动**，与本批改动无因果关系。但它是一条**有 CI 价值**的观察：
CI 的 `npm run test:run` 也是并行，该文件在冷缓存 + 高并发下同样可能抖动；
若要压掉，正确做法是给该用例**显式 `timeout`**（而非把阈值全局放大 —— 那会把真超时一起放行）。
另：本节首轮那次「1 例红」若不追单跑读数，就会被误记成「收敛改动打断了 cron 配置回读」。
**「红了先别改代码，先量它的量纲」** —— 时间型失败的第一嫌疑是**负载**，不是被测对象。
