# AUDIT — 领域语义层（O1 概念注册表 + O2 扫描器 v2）

> 状态：active ｜ 日期：2026-09-14 ｜ 对应版本：v2.9.7


- **日期**：2026-09-14
- **权威源**：`src-tauri/crates/harness/src/domain_semantics.rs`（493 行）
- **配套**：`docs/plans/PLAN-semantica-borrowings.md`（O1/O2 条目）｜`output/domain-semantics-report-2026-09-14.txt`（扫描输出，94 行）
- **授权范围**：用户选定路线「先做 O2 门禁扫描（--report-only）」，本报告记录 O2 v2 改造与 O1 首版落地

---

## §0 结论摘要

| 项 | 结果 |
|---|---|
| **O1 概念注册表** | ✅ 落地 `harness/src/domain_semantics.rs`：**24 概念 / 10 表面名**，每条带源码锚点；**零行为变更** |
| **O2 扫描器 v2** | ✅ 判据升级：清单改读注册表；**A 段（未登记量纲 = 真异常）= 0**；覆盖率 **100%（556/556 强锚点）** |
| **一名多义实况** | `confidence` **10 个概念** ｜ `score` **3 个** ｜ `min_confidence` **3 个** ｜ `position_pct` **2 个** |
| **补登记** | 3 个此前漏掉的概念（`CalibrationResult.score` 损失值量纲、`ValueAssessmentAgent` / `CostAwareRouter` 的 `min_confidence`） |
| **⚠ 新发现真缺陷** | **1 个**：`evidence_citation.rs:151` 量纲不匹配 ⇒ 前端证据引用面板**恒满格 / 恒绿 / 百分比可达 450%**（见 §6） |
| **未做** | O2 未接 CI；`confidence` 等未 newtype 化（均为**行为变更**，待裁决） |

---

## §1 先纠正我上一轮的错误结论

上一轮（O2 v1，09-14 01:52）我报的是：

> 「域内同名多量纲 **2 个概念**（`confidence` / `score` —— 各自同时存在 0–1 与 0–100 用法）」

**这个结论在措辞与判据上都是错的**，且错在**脚本的数据结构**里，不在数据里：

| | v1 做法 | 事实 |
|---|---|---|
| 数据结构 | 一个 `name` 一个 `expect`（期望量纲） | 一个 `name` 承载 **多个概念**（`confidence` = 10 个） |
| 「域」的定义 | `scope` = **路径前缀** | 用位置代替身份 ⇒ `workflow_ai/generate.rs`（节点推荐）被算进投资域 |
| 后果 | 必然把其中一半的出现报成「冲突」 | 实际是**各自内部自洽的多个概念**，没有冲突 |

⇒ **v1 报的「33 处冲突」是我自己的数据结构造出来的**，不是代码里的缺陷。这是本项目反复出现的同一形态：**没有权威源 ⇒ 测量工具自己发明判据 ⇒ 报出假缺陷**。

**唯一的例外**：v1 的「PLAN 病点实检」段是有效的独立产出 —— 它证明了 PLAN 自己登记的 4 个病点**只有 1 条经得起实检，而那条是 PLAN 自己写错**（`position_pct` 实为 0–100，不是 PLAN 写的 `[0,1]`）。该结论在 v2 中保留。

---

## §2 O1 落地：`harness/src/domain_semantics.rs`

### §2.1 落点选择的依据

原 PLAN 写「落点已存在 = `analysis-engine/src/concept_index.rs`（1132 行）」—— **这是误判**，两者不同层：

| 模块 | 层次 | 回答的问题 |
|---|---|---|
| `analysis-engine::concept_index`（1132 行，**22 个消费点**） | **实体层** | 「AI 概念包含哪些股票」（`ConceptNode`/`seed_ashare_ontology`，题材/行业→成分股） |
| `harness::dashboard_report` | 运行时 | 「这份报告的值越界了吗」 |
| **`harness::domain_semantics`（新增）** | **字段层** | 「`confidence` 此刻指哪个概念、值域多少」 |

`crates/harness/src/lib.rs:3-8` 载明本 crate「**零业务逻辑、零具体实现**；仅包含 trait 接口定义、纯数据 DTO、常量和错误类型」⇒ 纯声明注册表**符合该约束**。

### §2.2 结构

```rust
pub enum Unit { Ratio, Percent, Cny, Loss }        // 量纲 + canonical_range() + label()

pub struct ConceptDecl {
    pub id: &'static str,        // 全局唯一，如 "axinvest.report.confidence"
    pub name: &'static str,      // 表面名（可重复 = 多义）
    pub carrier: &'static str,   // ★ 消歧键：同名多义时承载者必须可区分
    pub unit: Unit,
    pub min: f64, pub max: f64,
    pub meaning: &'static str,
    pub evidence: &'static str,  // 源码锚点 `路径:行`，多个用 ` | `
}

pub const CONCEPTS: &[ConceptDecl] = &[ /* 24 条 */ ];
```

查询接口：`by_name()` / `by_id()` / `ambiguous_names()`（字典序，供测试锁定）。

### §2.3 三道防腐烂测试（`cargo test -p axagent-harness domain_semantics`）

| 测试 | 作用 |
|---|---|
| `test_ambiguous_names_locked` | **锁死多义名单** `["confidence","min_confidence","position_pct","score"]` —— 新增多义而不同步登记 ⇒ 测试红 |
| `test_same_name_carriers_are_distinguishable` | 同名概念的 `carrier` 必须互不相同，否则等于没消歧 |
| `test_ids_are_unique` / `test_every_decl_cites_evidence` / `test_range_within_unit_canonical` / `test_lookup_negative_control` | ID 唯一、锚点格式可解析、值域落在量纲内、查询接口**负向对照**（防过滤器写反） |

> **刻意不做 `pub use` 再导出**：`CONCEPTS` / `Unit` 名字过于通用，re-export 到 harness 根会与既有模块撞名；且本 crate 的消费方（跨 crate）用完整路径更清晰。（本项目已有 `error_code.rs` 因 re-export 触发 8 条 `unused_imports` 的先例。）

---

## §3 O2 扫描器 v2：判据升级

### §3.1 新判据（取代 v1）

```
现场量纲簇是否被「注册表里该名字全部概念的量纲」覆盖？
  ├─ 覆盖   ⇒ 已知多义（B 段）—— **不是缺陷**
  └─ 未覆盖 ⇒ 真异常（A 段）—— 新概念未登记，或量纲用错
```

量纲→可接受簇映射（`UNIT_CLUSTERS`）：`Ratio→{RATIO,ONE,ZERO}`、`Percent→{SCALED,ONE,ZERO}`、`Cny→{LARGE,ONE,ZERO}`、`Loss→{全部}`。
（`ONE`/`ZERO` 无判别力，各量纲下均视为通过。）

### §3.2 累计修掉的 6 个脚本自身 bug

| # | bug | 后果 | 版本 |
|---|---|---|---|
| 1 | 词边界 `(?<![\w.])` | `p.position_pct` / `self.confidence`（**最常见写法**）全被拒 ⇒ 漏检 | v1 内 |
| 2 | 概念未限定「域」 | `crates/agent` 的 LLM 置信度与投资决策置信度混算 ⇒ **216 条假阳性** | v1 内 |
| 3 | 注释行被当现状 | `旧式 (dqi-50)/50 把…` 命中 ⇒ 假报「仍存在」 | v1 内 |
| 4 | 换算点算进量纲簇 | `posterior * 100.0` 是合法换算 ⇒ 凭空造冲突 | v1 内 |
| 5 | **一名字一期望量纲** | 数据结构无法表达「一名多概念」⇒ **把一半出现误报成冲突** | **v2** |
| 6 | `scope` 用路径前缀当「域」 | 用位置代替身份 ⇒ 节点推荐被算进投资域 | **v2** |
| 7 | prompt 模板内的示例代码被当代码 | `helpers.rs` 3 个模板块共 **435 行**（27–112 / 189–457 / 460–542）里的 `i.score > 0.8` 被当真实锚点 | **v3** |

**第 7 条的修法**：`rawStringBlocks()` 状态机标记多行原始字符串内的行并跳过。
词边界 `(?<![A-Za-z0-9_])b?r(#{0,255})"` **是必需的** —— 否则 `let s = "four";` 里 `four"` 的 `r"` 会被误判为 raw string 开头（`four` / `color` / `their` / `our` 都很常见）。已加**正负对照**锁死。

### §3.3 自检（20 项，全过）

| 组 | 项数 | 内容 |
|---|---|---|
| 检测器正负对照 | 7 | 含 v1 两个 bug 的回归用例（`★字段访问必须抓到`、`★后缀名不得误抓`） |
| **注册表解析自证** | 9 | 含 3 个负向对照：**空文本应解析出 0 条**、缺 `unit` 的条目应被丢弃、完整条目应被接受 |
| **多行字符串自证** | 4 | 含 `★普通字符串不得误判（four/color）` |
| 扫描面自证 | — | **3074 个源文件**（0 ⇒ 非 0 退出） |

> 注册表解析为 0 条 ⇒ 顶层**直接非 0 退出**（`process.exit(3)`）：否则会「静默零命中」，让报告看起来一片绿。

---

## §4 实证：一名多义的完整现场

### §4.1 `confidence` —— 一名 **10 概念**

| 量纲 | 概念 ID | 承载者 |
|---|---|---|
| **0–100** | `axinvest.report.confidence` | `DashboardReport`（`dashboard_report.rs:42`，`:260` 判 `>100.0` 越界） |
| **0–100** | `axinvest.pick.confidence` | `RecommenderPick (u8)`（`recommender/mod.rs:649` `clamp(0.0,100.0)`） |
| 0–1 | `axinvest.candle_pattern.confidence` | `CandlePattern`（`exit_recommend.rs:347` 阈值 0.7） |
| 0–1 | `axinvest.market_regime.confidence` | `MarketRegime`（`market_regime.rs:14` 注释「置信度 0-1」） |
| 0–1 | `axagent.route_stage.confidence` | `RouteStageRecord`（`cognitive_router.rs:323` 注释「0.0 - 1.0」） |
| 0–1 | `axagent.node_rec.confidence` | `NodeRecommendation`（`workflow_ai/generate.rs:388`，20 处） |
| 0–1 | `axagent.skill_extract.confidence` | 局部变量（`skills/management.rs:1248`） |
| 0–1 | `axagent.skill_upgrade.confidence` | 局部变量（`agent_nudge.rs:134`） |
| 0–1 | `axagent.adaptive_decision.confidence` | 自适应决策结构（`stock_adaptive_engine.rs:539`） |
| 0–1 | `axagent.routing_result.confidence` | 路由结果（`cognitive.rs:2196`） |

现场强锚点分布：`RATIO 175 ｜ ONE 32 ｜ SCALED 19`。

### §4.2 其余三个多义名字

| 名字 | 概念数 | 分族 |
|---|---|---|
| `score` | **3** | `DashboardReport (u32)` 0–100 ｜ `RoundEvaluation` 0–1 ｜ **`CalibrationResult` 损失值**（正常量级 3，越小越好） |
| `min_confidence` | **3** | `RouterConfig (u8)` 0–100 ｜ `ValueAssessmentAgent` 0–1 ｜ `CostAwareRouter` 0–1 |
| `position_pct` | **2** | `DashboardReport` 0–100 ｜ `RecommenderPick` 0–100（量纲同、承载者异） |

### §4.3 量纲单义的名字（6 个，对照）

`match_confidence`、`final_position_pct`、`posterior`、`effective_posterior`、`total_mv`、`dqi_score`。

---

## §5 本次补登记的 3 个概念（由 A 段报出后核实）

v2 首跑 A 段报了 2 个「未登记量纲」，逐个核实后**都不是缺陷，而是漏登记**：

| 报出 | 核实结论 | 处置 |
|---|---|---|
| `market-sim/calibration.rs:280` `score: 9999.0` ⇒ `LARGE` | `score` 在此处是**拟合误差**（文档示例 `3.18`，升序排序 + `best = results[0]` 取最小）⇒ 哨兵 9999 落最差位，**逻辑自洽** | 新增 `Unit::Loss`（越小越好、无上界）+ 登记 `axagent.calibration.score` |
| `opc/agent.rs:217` / `smart_router/mod.rs:379` `min_confidence: 0.6` ⇒ `RATIO` | 两处比的都是 **0–1 的置信度**（`opc` 还有 `clamp(0.0,1.0)` 保护） | 补登记 2 条 `min_confidence`（0–1 族） |

补登记后：**A 段 = 0，覆盖率 100%（556/556）**。

---

## §6 ⚠ 一个真缺陷 —— 而且**扫描器查不出来**

> 这一节是本报告最重要的部分：它标出**静态量纲扫描的能力边界**。

### §6.1 缺陷

```rust
// crates/analysis-engine/src/evidence_citation.rs:151
let score = jaccard * 0.7 + (number_overlap as f64 / claim_len.max(1.0)) * 30.0;
if score > 0.15 && ...                                        // :153 阈值按 0–1 设
match_confidence: (score * 100.0).round() / 100.0,            // :171 写法按 0–1 用
```

- `claim_len` = claim 的**去重字符集大小**（`:129` `claim.chars().collect()` ⇒ `HashSet` 长度），中文 claim 典型 ≈ 20
- claim 含 3 个数字 ⇒ `number_overlap / claim_len = 3/20 = 0.15` ⇒ 第二项 **× 30 = 4.5**
- ⇒ **`score` 实际量级 ≈ 4.5**，而两项权重相差 **约 40 倍**（第一项上限 0.7）
- ⇒ 阈值 `> 0.15` **近乎恒真**；`jaccard`（真正的文本相似度）**几乎不起作用**

### §6.2 下游全部按 0–1 使用 ⇒ UI 必然异常

| 位置 | 代码 | 后果 |
|---|---|---|
| 前端 `EvidenceCitationPanel.tsx:182` | `(citation.matchConfidence * 100).toFixed(0)` | Tooltip 显示 **"450"**（应为百分数） |
| 前端 `:189` | `Math.min(citation.matchConfidence * 100, 100)%` | 条形 **恒满格** |
| 前端 `:190` | `citation.matchConfidence > 0.5 ? 绿 : 黄` | **恒绿**，无区分度 |
| 后端 `:345` | `match (citation.match_confidence * 10.0) as usize { ... }` | 按 0–1 算格数，实际 45 ⇒ 落默认臂 |

**这不是边界情况，是常态**（任何含 ≥1 个数字的 claim 都可能 > 1）。

### §6.3 为什么扫描器报不出来

注册表登记 `match_confidence` 为 `Unit::Ratio`（**应然** 0–1）。该名字的强锚点只有 1 条（`score > 0.15` ⇒ `RATIO` 簇）⇒ **被覆盖 ⇒ 报绿**。

**边界**：静态扫描只判「**字面量锚点的量纲**」，判不了「**表达式的计算结果落在哪**」。`jaccard*0.7 + ratio*30` 是算术式，没有字面量暴露出 30.7 这个上界。

⇒ **该缺陷是人工读代码发现的，不是扫描器发现的。** 这条必须写进局限，否则会高估 O2 的覆盖力。

### §6.4 登记状态

**未修** —— 改它会影响证据引用的匹配结果与排序（**行为变更**），且 `match_confidence` 的**应然量纲**本身是个产品决策（0–1 还是 0–30？）。按铁律 7「报告不擅改」登记待裁决。

---

## §7 未做的 / 待你裁决

| # | 项 | 选项 |
|---|---|---|
| 1 | **`evidence_citation.rs:151` 量纲不匹配**（§6） | **A** 修权重使 score ∈ [0,1]（如第二项改 `* 0.3`）｜**B** 改下游按 0–30 显示｜**C** 不动 |
| 2 | `market-sim` 哨兵值 9999 参与排序 | 若**所有参数都失败**，`results[0]` 会是 `score=9999, total_trades=0` 的失败项 —— 静默把「全失败」报成「最佳参数」。是否加显式检查？ |
| 3 | `confidence` / `score` 等的**同名消歧** | 是否需要 **newtype 化**（`DecisionConfidence(f64)` vs `MatchConfidence(f64)`）—— 让**编译器**替静态脚本做归属判定。**行为变更，改动面大** |
| 4 | O2 接 CI | 加 `--strict` 到 `ci.yml`。当前存量：**A 段 0 违规**，但 PLAN 病点 1 条（`PLAN 写错`）会让 strict 失败 —— 需先决定该条怎么处理 |
| 5 | `Unit` 缺变体 | 暂无 `Count`（计数）与「价格」（与 `Cny` 同量纲、语义不同）—— 出现时再补 |

---

## §8 验证

| 项 | 命令 | 结果 |
|---|---|---|
| 脚本语法 | `node --check` | ✅ |
| 脚本自检 | `node scripts/check-domain-semantics.mjs --selftest` | ✅ **7 + 9 + 4 = 20 项全过**；扫描面 **3074 文件** |
| 真扫描 | `node scripts/check-domain-semantics.mjs` | ✅ 退出 0；**A 段 0 违规**；覆盖率 **100%（556/556）**；报告 94 行 |
| 注册表测试 | `cargo test -p axagent-harness domain_semantics` | ✅ **8 passed / 0 failed**（7m07s；编译 0 warning） |
| 注册表 clippy | 未单独运行（上条的 test 编译即 `--all-targets` 口径，0 warning） | ⚪ |

### §8.1 编译期的一次真实等待

本轮的 `cargo check -p axagent-harness --all-targets` 曾**卡在文件锁上 5 分 19 秒**（日志只有一行 `Blocking waiting for file lock on build directory`），现场有 **4 个 `cargo` + 2 个 `rustc`（各占 ~2GB 内存）** 进程 —— **并发会话正在编译主 crate**。该任务被终止，改在测试时统一验证。

---

## §9 改动清单

| 文件 | 动作 | 备份 |
|---|---|---|
| `src-tauri/crates/harness/src/domain_semantics.rs` | **新增**（493 行：注册表 + 9 测试） | — |
| `src-tauri/crates/harness/src/lib.rs` | 加 `pub mod domain_semantics;`（**不做 re-export**） | `output/backup-2026-09-14/domain-semantics/harness-lib.rs.before` |
| `scripts/check-domain-semantics.mjs` | **重写核心**（读注册表 + 新判据 + 多行字符串识别 + 自检扩至 20 项） | `.../check-domain-semantics.mjs.before` |
| `docs/plans/PLAN-semantica-borrowings.md` | O1/O2 状态、更正两处**我自己写错的验收标准/病点** | `.../PLAN-semantica-borrowings.md.before-o1` |
| `output/domain-semantics-report-2026-09-14.txt` | 扫描输出（94 行） | — |
| `output/domain-semantics-2026-09-14.json` | `--json` 全量输出 | — |

**行为变更：零。**（注册表是纯声明；脚本是只读工具。）

---

## §10 一句话总结

**这一轮真正的产出不是「发现了多少冲突」，而是：把一个凭直觉设期望值的扫描器，改造成「以代码里的权威声明为准」的扫描器；并在这个过程中证明了上一轮报的「冲突」是我的工具自己造出来的。**
