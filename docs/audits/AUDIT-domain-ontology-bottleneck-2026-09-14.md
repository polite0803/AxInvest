# AUDIT — 瓶颈掘金领域本体（真 O1：类 / 关系 / 公理 + 副本收敛 + CI 一致性门）

> 状态：active ｜ 日期：2026-09-14 ｜ 对应版本：v2.9.7


- **日期**：2026-09-14
- **权威源**：`src-tauri/crates/harness/src/domain_ontology.rs`（806 行｜17 测试）
- **配套门禁**：`scripts/check-ontology-consistency.mjs`（774 行｜26 selftest 用例）
- **接线点**：`src-tauri/src/commands/stock_analysis_setup/seed_serenity.rs:1268, 1350-1380`
- **授权范围**：用户选定第一刀「**瓶颈三力·含接线**」——建类/关系/公理/约束，并把权重副本收敛为「引用权威源」，加 CI 一致性门。**权重数值不变，只是来源变一处**。
- **前置背景**：本报告是对 03:15 用户质疑（「到目前为止，我还是没有见到任何类似 ontology 的任务落地」）的直接回应。上一版 `domain_semantics.rs` 是**字段层横切查找表**，不是本体 —— 诊断详见 `.workbuddy/memory/2026-09-14.md`。

---

## §0 结论摘要

| 项 | 结果 |
|---|---|
| **真 O1 本体** | ✅ `harness/src/domain_ontology.rs`：**11 类**（含 2 根 + 继承链）／**4 对象关系**（带 domain→range）／**4 度量**（带单位·值域）／**7 公理**（每条指名强制它的测试）／**4 分档**／**2 口径分歧登记** |
| **副本收敛** | ✅ 权重**唯一权威源** `BOTTLENECK_WEIGHTS`；seeder 的 3 个变量值 + 描述文本改由 `bw` / `format_weight` 渲染 |
| **CI 一致性门** | ✅ `check-ontology-consistency.mjs`：`--selftest` **26/26 通过**；实跑 **exit 0**；4 个站点全部一致；2 项口径分歧只报告不拦 |
| **活体自证** | ✅ `PROBE_OK=true`：人为把 `else { 0.35 }` 漂成 `0.40` ⇒ 门禁 **exit 1** 且指名现场（`权重副本与权威源不一致：期望 0.35，实际 0.4`）；还原后 **sha256 逐字节相同** |
| **行为变更** | **零**。`format_weight` 固定 2 位小数 ⇒ 种子产物**逐字节不变** ⇒ **不需要** 递增 `TEMPLATE_VERSION` |
| **⚠ 待裁决** | **2 项口径分歧**（D1 复合分定义式「等权 vs 加权」、D2 分档阈值「80/60 vs 75/55/35」）—— 已登记 `OPEN_DIVERGENCES`，属**产品语义决策**，未擅改 |
| **未做** | 其余 6 类副本（`strategy-scorer.rhai` 仍持一份独立兜底、prompt 文案、注释散文）**只监控未收敛**；分项权重（`0.30/0.40/0.30` 等）尚未变量化 |

---

## §1 为什么这个才是 ontology，而 `domain_semantics.rs` 不是

这不是措辞之争，是**形态**之争。两者回答的是不同问题：

| | `domain_semantics.rs`（字段层） | `domain_ontology.rs`（领域层，本次） |
|---|---|---|
| 回答的问题 | 「这个**字段**是什么单位」 | 「这个**领域里有什么**、它们怎么关联、什么必须成立」 |
| 数据结构 | `ConceptDecl { name, surface, unit }` 扁平行 | `OntoClass`（**带 `parent` 继承链**）+ `ObjectProperty`（**带 `domain`/`range`**）+ `Metric` + `Axiom` |
| 有层次吗 | ❌ 24 行平铺，无父子 | ✅ `IndustryElement → IndustryChain → ChainStage → Chokepoint`（4 层） |
| 有关系吗 | ❌ 概念之间零连线 | ✅ 4 条，且**两端必须是已声明类**（有测试强制） |
| 有公理吗 | ❌ 无 | ✅ 7 条，每条 `enforced_by` 指到具体测试名 |
| 能推吗 | ❌ 只能查表 | ✅ `is_subclass_of` 走传递闭包、`superclass_chain`、`subclasses_of`、`band_for_score` |
| 24 行构成 | `confidence` 家族 14 行（58%）+ `score` 3 + 位置 3 + 杂 4 | —— |

一句话：**`domain_semantics` 是横切的列，`domain_ontology` 是纵切的骨架。** 前者能防「单位写错」，后者才回答「瓶颈掘金这套方法论到底在说什么」。

### 本体的实际内容

```rust
// 类继承链（节选）
IndustryElement (root)
├── Industry
├── IndustryChain
│   └── ChainStage
│       └── Chokepoint          ← 继承实测：Chokepoint ⊑ ChainStage ⊑ …
├── Company
└── Product

BottleneckForce (root)
├── SupplyRigidity ─┐
├── DemandElasticity ├─ measured_by ─→ Chokepoint
└── Irreplaceability ┘

// 对象关系（domain → range，两端须为已声明类）
occupies   : Company   → ChainStage
produces   : Company   → Product
depends_on : Product   → Product
measured_by: Chokepoint→ BottleneckForce
```

公理 A1–A7 各自指名强制它成立的测试，例如：

- **A1** 三力顶层权重和为 1 ⇒ `test_top_level_weights_sum_to_one`（含漂移负控 `0.40×3` 必须失败）
- **A2** 每力分项权重和为 1 ⇒ `test_force_part_weights_sum_to_one`（`0.9` 必须失败；**空数组必须失败**）
- **A3** 复合分是**加权和**、不是算术均值 ⇒ `test_composite_is_weighted_not_mean`（在 `(80,20,100)` 上证明两者相差 >1.0）
- **A4** 分档**全覆盖且无缝隙** ⇒ `test_readiness_bands_are_total_and_gapfree`（底部缺口/乱序/空集三种负控）
- **A5** 类层次**无环** ⇒ `test_class_hierarchy_acyclic_with_negative_control`（自环 + 互指）
- **A6** 恰有 3 个力、每力 1 个度量、权重 >0 ⇒ `test_exactly_three_forces_match_weights`
- **A7** 权重渲染进文本**必须保尾零** ⇒ `test_format_weight_keeps_trailing_zero`

> **A7 为什么是公理而不是工具函数注释**：`format!("{}", 0.30_f64)` 得到 `"0.3"`（f64 Display 丢尾零）。若 seeder 用 `{}` 渲染权重，种子 JSON 就会变，触发 `TEMPLATE_VERSION` 重种子 —— 而**没有任何线索指向「是渲染导致的」**。测试里用 `assert_ne!(format!("{}", 0.30_f64), "0.30")` 把这个坑钉死。

---

## §2 病灶证据：三力权重原本散在 7 处

这是**实测**的副本现场（门禁每次运行都会重新打印，不是一次性快照）：

| # | 现场 | 形态 | 是否收敛 |
|---|---|---|---|
| 1 | `bottleneck-calc.rhai:175-177` | rhai 兜底字面量 `else { 0.35 }` | 监控中 |
| 2 | `strategy-scorer.rhai:212-214` | **第二份脚本独立一份兜底**（`to_f64(w_supply, 0.35)`） | 监控中 |
| 3 | `bottleneck-calc.rhai:221` 附近 | 分档阈值 `75/55/35` | 监控中 |
| 4 | `strategy-scorer.rhai:65-66` 附近 | 分档阈值 `75/55/35`（第二份） | 监控中 |
| 5 | `seed_serenity.rs:1352-1380` | 模板变量定义（3 处 `value` + 3 处描述文本） | ✅ **已收敛为引用权威源** |
| 6 | `stock-analysis/chokepoint-identifier.md:101-102` | prompt 文案 | ❌ **只登记未改**（见 §6） |
| 7 | `bottleneck-calc.rhai:3, :173` | 散文注释复述默认值 | 只报告 |

两个必须点出的细节：

1. **第 2 处是本次新发现的**。`strategy-scorer.rhai` 与 `bottleneck-calc.rhai` 是两条**独立脚本**，各自持有一份同样的兜底值 —— 只改一处会得到「同一个数在两个脚本里不一致」的静默错判。这正是「N 份实现只 1 份在跑」家族的问题。
2. **第 5 处之前**只是「同一份数值抄了 3 遍 + 描述文本里再抄 3 遍」，共 6 个数字。现在 seeder 里**一个权重数字都没了**：

```rust
let bw = axagent_harness::domain_ontology::BOTTLENECK_WEIGHTS;   // :1268
// ...
Variable { name: "w_supply".into(),
    value: serde_json::json!(bw.supply),
    description: Some(format!(
        "瓶颈评分-供给刚性权重（默认{}，c-scorer 引用）",
        axagent_harness::domain_ontology::format_weight(bw.supply))),   // :1357
```

### ⚠ 一个陷阱：同形不同义的第三组 `0.35/0.35/0.30`

`src-tauri/src/commands/data-quality.rhai:775` 里有一组**数值完全相同**的三元权重：

```rhai
let score = report_quality_avg * 0.35 + tool_credibility_score * 0.35 + factor_completeness * 100.0 * 0.30;
```

但这是**数据质量综合分**（报告质量 / 工具可信度 / 因子完整度），与瓶颈三力**毫无关系**。数值巧合相同。

⇒ 谁若用「grep 到相同数值就合并」的方式做去重，**会把两个不相干的概念缝在一起**。所以门禁的兜底值扫描**锚定 `name: "w_*"`**（变量名是身份），而不是「扫描全仓浮点数」；这也是本报告把这条写进「已知局限」的原因（见 §7）。

---

## §3 两种副本，两种处置（**刻意不合并**）

这是本报告最重要的一条设计约束，写在 `domain_ontology.rs` 模块文档顶部：

| | **数值副本** | **口径分歧** |
|---|---|---|
| 定义 | 同一个口径被抄了多份 | **两个不同定义**并存 |
| 判据 | 可机械判定（值相等 or 不等） | **无法机械判定**（谁对是业务决策）|
| 处置 | **硬拦**（CI 红） | **只登记 + 报告**（`OPEN_DIVERGENCES`）|
| 本次实例 | 7 处副本中的 5 处 | **D1 / D2** |

把两者混为一谈的后果：要么**擅自替用户做业务决策**（把 D1 直接「统一」），要么**把真缺陷降级成备注**（把数值漂移当「口径分歧」放过）。所以门禁里 `--strict-divergence` 是一个**必须显式开启**的开关，默认不拦。

---

## §4 门禁：三个「让它自己不会撒谎」的机制

`scripts/check-ontology-consistency.mjs` 的核心不是「比对数值」，而是**保证比对本身可信**。三条机制各自对应一次真实踩坑：

### 4.1 权威源解析失败 ⇒ `exit 3`，绝不静默放过

若 `domain_ontology.rs` 被重构（`BOTTLENECK_WEIGHTS` 改名、字段顺序变化、变成多行），正则解析会返回 `null`。此时**绝不能**报「0 处不一致 ⇒ 通过」—— 那正是「审计脚本自己撒谎，报 0 命中」的经典形态。

⇒ 解析失败直接 `exit 3`，与「有违规 `exit 1`」区分开，调用方能分辨「检查没跑成」和「检查跑了且发现了问题」。正则**锁死字段顺序**（`supply/demand/irreplaceability` —— 注意这个顺序**不等于**显示顺序 `供给/需求/不可替代性`），乱序即视为格式已变。

### 4.2 站点清单是**契约**，不是「尽力而为的扫描范围」

```js
const SITES = [
  { file: ".../bottleneck-calc.rhai",  kind: "rhai-weight-fallback", expect: 3 },
  { file: ".../strategy-scorer.rhai",  kind: "rhai-weight-fallback", expect: 3 },
  { file: ".../bottleneck-calc.rhai",  kind: "rhai-band-threshold",  expect: 3 },
  { file: ".../strategy-scorer.rhai",  kind: "rhai-band-threshold",  expect: 3 },
];
```

每项带 `expect` 命中数。**命中数为 0 ⇒ 失败**（不是「没问题」）—— 因为 0 命中只可能意味着两件事：代码被重构了、或抽取模式腐烂了。两种都必须有人来看。

### 4.3 自锚定：脚本的脆弱**不得伪装成代码的问题**

第一版让调用方传锚点（`{ line: 250 }` / `{ fn: "..." }`），结果在 `strategy-scorer.rhai` 上踩坑：锚点落到了 `:250` 的

```rhai
"readiness_signal": readiness_signal(composite),
```

这行**含分档名但没有 `>=`** ⇒ 抽取返回 `null` ⇒ 门禁报告「代码可能已重构」。**但代码没重构，是我的锚点猜错了。**

⇒ 改为**自锚定**：函数自己找「同一行里既有 `>=` 又有分档标签」的那一行作为起点，再向后最多 8 行收集 `>=` 浮点数。这条作为回归用例钉进 selftest：

> `★抽取分档阈值：调用点（含标签但无 >=）不得被当成锚点 —— 实测踩过的 bug`

### 4.4 双向守卫：只查「有没有引用」是不够的

「声明 ≠ 接线」。若 seeder 里出现 `domain_ontology::BOTTLENECK_WEIGHTS` 就算通过，那么**把 3 个 `value` 改回字面量、同时保留一句注释里的引用**即可骗过门禁。

⇒ `WIRING` 检查**两个方向**：

1. **正向**：必须出现 `domain_ontology::BOTTLENECK_WEIGHTS` 和 `domain_ontology::format_weight(bw.supply)`；
2. **反向**：`findHardcodedWeightValues()` 锚定 `name: "w_*"`、跨行向前 8 行找 `value: serde_json::json!(N)` —— **出现任何数字字面量即失败**；
3. **锚点存在性**：若连 `name: "w_supply"` 都找不到 ⇒ 失败（防止「变量被改名 ⇒ 反向检查扫 0 处 ⇒ 假通过」）。

> 这里踩过一次真实假阳性：初版反向断言用单行正则 `/value:\s*serde_json::json!\(0\.\d+\)/`，命中了 seeder 里**无关变量** `serenity_min_revenue_growth` 的 `json!(0.0)`。⇒ 改成锚定式抽取。回归用例：`★回退守卫：无关变量的 json!(0.0) 不得报出 —— 实测的假阳性`。

### 4.5 扫描面自证

门禁末尾打印「读取文件 10 个｜注释行 107 行」。若 `readCount.n === 0` ⇒ **exit 1**。理由同 4.1：一个什么都没扫的检查，其「0 违规」毫无意义。

---

## §5 活体自证（`output/onto-probe.mjs`）

「门禁能通过」不等于「门禁能拦截」。所以做了一次**破坏性自证**——所有步骤在**同一个 Node 进程内串行**（不用外层 shell `&` 串联，那在本项目引发过一次真实事故）：

| 步骤 | 断言 | 实测 |
|---|---|---|
| 1. 备份 + 自校验 | 备份文件 sha256 == 源文件 sha256 | ✅ `2434e70c…` == `2434e70c…` |
| 2. 基线必须绿 | 漂移前门禁 `exit 0` | ✅ `baseline_exit=0` |
| 3. 制造单字符漂移 | `bottleneck-calc.rhai` 的 `else { 0.35 }` → `else { 0.40 }` | 字节数不变 `14367→14367` |
| 4. **门禁必须红且指名现场** | `exit != 0` 且输出含站点路径 | ✅ `drift_exit=1`、`drift_names_site=true` |
| 5. 还原 | 还原后 sha256 == 备份 sha256 | ✅ `restore_byte_identical=true` |
| 6. 还原后必须复绿 | `exit 0` | ✅ `restored_exit=0` |

漂移态的门禁输出（原文）：

```
     顶层权重兜底（w_supply/w_demand/w_irreplace）｜实测：supply=0.4 demand=0.35 irreplaceability=0.3
     权重副本与权威源不一致：期望 0.35，实际 0.4
结论：❌ 1 处硬拦问题｜口径分歧 2 项待裁决
```

⇒ **`PROBE_OK=true`**。这条证明的是「门禁真的会红、且红得能指路」，而不只是「跑起来是绿的」。

---

## §6 待裁决：D1 / D2 口径分歧（**未擅改**）

读 `src-tauri/agency_experts/stock-analysis/chokepoint-identifier.md` 时发现两处**与 rhai 实现互相矛盾**的表述。这不是副本抄错，是**两个定义并存**：

### D1 — 复合分的定义式

| | 说法 | 现场 |
|---|---|---|
| prompt 侧 | **等权**：`(supply_rigidity + demand_elasticity + irreplaceability) / 3` | `chokepoint-identifier.md:101` |
| rhai 侧 | **加权**：`Σ wᵢfᵢ`（`0.35/0.35/0.30`） | `bottleneck-calc.rhai` 复合分计算 |

**为什么不能机械统一**：

- 若按**等权** —— 那 `BOTTLENECK_WEIGHTS` 应当全为 `1/3`，前端暴露的「可调权重」就是**假配置**（调了不起作用）；
- 若按**加权** —— 那 `chokepoint-identifier.md:101` 这一行**是错的**，会让 LLM 按 `/3` 心算，与脚本算出的分**系统性不一致**。

对错取决于**业务想要哪个**。已登记为 `OPEN_DIVERGENCES` D1，状态「待裁决」。

> 顺带说明：本体里 A3 公理（`test_composite_is_weighted_not_mean`）锁的是「**当前代码实现是加权**」这一事实，**不是**在替业务裁决「应该加权」。两者必须分清。

### D2 — 分档阈值

| | 阈值 | 现场 |
|---|---|---|
| prompt 侧 | `80 / 60` | `chokepoint-identifier.md:102` |
| rhai 侧 | `75 / 55 / 35` | `bottleneck-calc.rhai` 分档 |

**先决问题**：两侧是否对**同一个量**分档？若 prompt 的 `80/60` 分的是「复合分」、而 rhai 的 `75/55/35` 分的是「readiness_signal」这个**派生量**，那两者**不是分歧**，只是我误配了对。⇒ 登记状态写明「统一前须先确认两侧是否对同一个量分档」。

---

## §7 已知局限 / 遗留风险

| # | 项 | 说明 |
|---|---|---|
| 1 | **兜底值抽取按「行内最后一个浮点数」** | `strategy-scorer.rhai:212` 是 `if w_supply != () { to_f64(w_supply, 0.35) } else { 0.35 }`，**一行内两个 0.35**。取最后一个在当前形态下正确，但若将来两处不一致，抽取只会看到其中一个 ⇒ 会漏报一半。**未修**（需按 token 切分才能彻底解决，收益暂不抵复杂度），已记录。 |
| 2 | **只扫 4 个站点** | 第 6/7 处（prompt 文案、注释散文）**未纳入硬拦**。prompt 侧即 D1/D2，属待裁决；注释侧只报告。 |
| 3 | **分项权重未变量化** | `SUPPLY_RIGIDITY_PARTS`(`0.30/0.40/0.30`)、`DEMAND_ELASTICITY_PARTS`(`0.60/0.40`)、`IRREPLACEABILITY_PARTS`(`0.30/0.40/0.30`) 在 rhai 里**仍是字面量**（`strategy-scorer.rhai:219, 235`）。本体已声明且测试已锁「和为 1」，但**没有接线** ⇒ 它们是「声明了但没人引用的权威源」。本次范围外。 |
| 4 | **同形不同义陷阱** | `data-quality.rhai:775` 的 `0.35/0.35/0.30` 与瓶颈三力**数值全同、语义无关**（见 §2 末）。门禁靠锚定变量名避开，但**未来做跨模块去重的人必须知道这条**。 |
| 5 | **`f64` 无法 `derive(Eq)`** | `Metric` / `Band` / `BottleneckWeights` 只派生 `PartialEq`。这是 Rust 的硬约束，不是遗漏；已在代码里写明注释。 |

---

## §8 验证记录

全部为本机实跑，退出码取自进程返回值：

| 检查 | 命令 | 结果 |
|---|---|---|
| harness 单测 | `cargo test -p axagent-harness domain_ontology --lib` | ✅ **17 passed / 0 failed**（`HARNESS_TEST_EXIT=0`）|
| clippy | `cargo clippy -p axagent-harness --all-targets -- -D warnings` | ✅ `HARNESS_CLIPPY_EXIT=0` |
| 门禁自检 | `node scripts/check-ontology-consistency.mjs --selftest` | ✅ **26/26 通过** |
| 门禁实跑 | `node scripts/check-ontology-consistency.mjs` | ✅ `exit 0`（4 站点一致｜2 分歧待裁决）|
| 破坏性自证 | `node output/onto-probe.mjs` | ✅ `PROBE_OK=true`（详见 §5）|
| 守卫测试 | `npx vitest run src/__tests__/ci.workflow.test.ts` | ✅ **17 passed** |
| 格式 | `npx dprint check src/__tests__/ci.workflow.test.ts` | ✅ `exit 0` |
| 类型 | `npx tsc --noEmit` | ✅ `TSC_EXIT=0` |
| 编译 | `cargo check -p axagent --lib` | 见 §8.1 |

### 8.1 `cargo check -p axagent --lib`

`axagent` 主 crate 体量大、且本机存在并发持锁，单次 check 耗时可达 6–31 分钟。本次在后台运行；`axagent-harness` 侧（本体所在 crate）已确定绿。若该步最终报错，须先判**错误文件是否属于本次改动清单** —— 不属于则几乎必然是并发会话的中间态（本项目高频形态），**不得**去改对方文件。

### 8.2 本次改动的文件清单

| 文件 | 动作 |
|---|---|
| `src-tauri/crates/harness/src/domain_ontology.rs` | **新增**（本体） |
| `src-tauri/crates/harness/src/lib.rs` | 注册 `pub mod domain_ontology;`（**刻意不 `pub use`** —— 见 G 组：`#[tauri::command]` 相关模块禁 `pub use sub::{...}` 的同类风险）|
| `src-tauri/src/commands/stock_analysis_setup/seed_serenity.rs` | 接线（3 处 `value` + 3 处描述文本）|
| `scripts/check-ontology-consistency.mjs` | **新增**（一致性门） |
| `.github/workflows/ci.yml` | 新增门禁步骤（排在 dprint 前） |
| `scripts/ci-check.mjs` | 新增本地同源两条（自检 + 判定）|
| `package.json` | 新增 4 个 script（`check:ontology` 等）|
| `src/__tests__/ci.workflow.test.ts` | 新增新门禁的**守卫测试**（防删除/改名/降级）|
| `docs/audits/AUDIT-domain-ontology-bottleneck-2026-09-14.md` | 本报告 |

**未改动**：`bottleneck-calc.rhai` / `strategy-scorer.rhai`（其副本仅被监控，值未动）｜`chokepoint-identifier.md`（D1/D2 待裁决）｜`domain_semantics.rs`。

---

## §9 与 semantica 借鉴计划（O1）的关系

`docs/plans/PLAN-semantica-borrowings.md` 中 O1 的原始验收判据是「任一概念的单位声明在全仓只有一处」。该判据在验证时被证伪——项目里真实的病是「**一个名字承载多个概念**」而非「一个概念散在多处」（诊断见 09-14 流水）。

本次按用户裁定的方向，把 O1 的**第一刀**落在**真正有重复病**的地方（三力权重，实测 7 处），且**形态**是本体（类/关系/公理）而非注册表。O1 剩余部分（`Unit` 缺 `Count`/「价格」、newtype 仅接 1/24 概念）仍待办，见 MEMORY 状态区。

---

## §10 ⚠️ 交付物自检（04:1x 用户质疑「你是不是没理解 ontology」后补）

用户质疑：**「你是不是没有理解 ontology 的含义，怎么感觉你就一直没有做对这个事情」**。

我按 OWL/Gruber 的正式定义逐层核对本交付物，结论是 —— **质疑成立，本交付物不是本体，是「常量单一真相源 + 一张类图草图」**。以下为源码级证据。

### 10.1 硬证据：本体的**自身构造**零消费

生产代码（`src-tauri/src` + `src-tauri/crates`）对 `domain_ontology` 的**全部引用 = 4 行**：

```
seed_serenity.rs:1268  let bw = axagent_harness::domain_ontology::BOTTLENECK_WEIGHTS;
seed_serenity.rs:1357  ... format_weight(bw.supply)
seed_serenity.rs:1367  ... format_weight(bw.demand)
seed_serenity.rs:1377  ... format_weight(bw.irreplaceability)
```

| 公开项 | 生产消费点 | 说明 |
|---|---|---|
| `CLASSES`（11 类） | **0** | 类层次无人读 |
| `OBJECT_PROPERTIES`（4 关系） | **0** | domain/range 无人读 |
| `METRICS`（4 度量） | **0** | 度量声明无人读 |
| `AXIOMS`（7 公理） | **0** | 公理无人读 |
| `READINESS_BANDS`（4 分档） | **0** | 分档无人读（rhai 里另有自己一份字面量） |
| `OPEN_DIVERGENCES` | **0** | 分歧登记无人读 |
| `class_by_id` / `is_subclass_of` / `subclasses_of` / `superclass_chain` / `band_for_score` | **全部 0** | 5 个查询函数全部零调用 |
| `BOTTLENECK_WEIGHTS` | 1 | ✅ 唯一真正生效 |
| `format_weight` | 3 | ✅ 唯一真正生效 |

⇒ **806 行里真正被使用的约 10 行；其余 ~790 行只有本模块自己的 17 个测试在读。**

这正是本仓反复登记的判据「**声明 ≠ 接线**」「**写进产物 ≠ 有人消费**」——**我把它用在了别人的代码上，没有用在自己这次的交付物上**。

### 10.2 按 OWL 正式定义逐层核对

| # | ontology 的构成 | 本交付物 | 判定 |
|---|---|---|---|
| 1 | **TBox** · `SubClassOf` | 有（字符串 `parent` + 传递闭包查询） | 🟡 形式完整但**零消费** |
| 2 | **TBox** · **定义类**（充分必要条件，判定「某个体是否属于此类」） | **无**。类只有自然语言 `meaning`，没有形式条件 | ❌ |
| 3 | **TBox** · **不相交公理**（`Company ⊓ Product ⊑ ⊥`） | **无** ⇒ 无法发现有实体同时是公司和产品 | ❌ |
| 4 | **TBox** · **基数 / 存在限制**（`Chokepoint ⊑ =3 measured_by.BottleneckForce`） | 无。我用**测试**断言「恰有 3 个力」—— 那是测试，**推理器看不见** | ❌ |
| 5 | **RBox** · `domain` / `range` | 有声明，但**无任何机制在违反时报错** ⇒ 纯文档 | 🟡 |
| 6 | **RBox** · 关系特性（`Transitive` / `Functional` / `InverseOf` / `Symmetric`） | **一个都没有**。而 `depends_on`（产品间依赖）**天然传递**、`measured_by` 应是 **functional** | ❌ |
| 7 | **ABox** · 个体断言（真实的行业 / 环节 / 瓶颈点 / 公司） | **零个个体**。全仓**没有** `industry_chain` / `chain_stage` / `chokepoint` / `entity` 表 ⇒ **领域数据只活在 prompt + `.rhai` + blackboard JSON 里** | ❌ |
| 8 | **推理** · 推出未显式陈述的事实，且被产品消费 | 只有沿字符串指针走一遍的 `is_subclass_of`；**没有任何新事实被产出并使用** | ❌ |
| 9 | **标准序列化**（OWL / RDF Turtle / JSON-LD / SHACL） | Rust 常量数组 ⇒ 图谱库、SPARQL、外部工具**都读不了** | ❌ |

**9 项里 2 项半成品、6 项缺失。** 本质：本交付物是**只有 TBox 的空壳，且这个 TBox 不可被解释器执行**。

### 10.3 更根本的一条：「权威源」只做到了数字，没做到语义

我把它命名为「**唯一权威源**」，但：

1. **prompt 至今仍与代码矛盾**：`chokepoint-identifier.md:101` 说复合分是等权 `(a+b+c)/3`，代码是加权 `Σwᵢfᵢ`。我把它登记成「D1 待裁决」就放过了 —— 但**若本体真是语义权威源，prompt 就该由它生成或至少被它校验**，而不是靠人记得同步。
2. **门禁根本不查 prompt**：`SITES` 只含 2 个 `.rhai` 文件；prompt 侧（第 6 处副本、也是**唯一会被 LLM 直接读走**的那处）**完全没机械化**。

⇒ 「唯一权威源」的实际覆盖面 = **3 个数字的 2 个副本点**。这个口径与「唯一」二字不符。

### 10.4 根因：没有 ABox 的落脚点

全仓没有任何瓶颈掘金的**实体/关系表**。这条流水线的产物是 prompt 驱动的**快照 JSON**，不落实体。⇒ **本体没有个体可描述** ⇒ 必然退化成「一张类图 + 几个常量」。

**结论**：不是我「没理解 ontology」，而是**我把 ontology 的验收标准定在了「形态齐全」，而不是「有推理在跑、有消费方在读」**。这与 10.1 是同一个病。

### 10.5 修正路线（待用户裁定先后）

| 选项 | 内容 | 让本体变成什么 |
|---|---|---|
| **A** | 补齐 TBox/RBox：定义类、不相交、基数、关系特性（transitive/functional）+ **导出标准序列化**（Turtle/JSON-LD） | 可被外部工具/图谱读的**真本体** |
| **B** | 建 **ABox**：瓶颈掘金实体+边表（行业 / 环节 / 瓶颈点 / 公司 / 产品 + 关系边），把 pipeline 产物结构化成个体落库 | 本体成为**表结构与写入校验的权威源** |
| **C** | **语义权威源做实**：prompt 的公式/阈值改由本体生成或被门禁校验 ⇒ 直接消灭 D1/D2 | prompt 不再能私自定义语义 |
| **D** | **推理进消费**：pipeline 改调 `band_for_score` / `is_subclass_of`，替掉 `.rhai` 里手写分档 | 本体真的参与判定 |

推荐次序：**C（最小、立刻消灭已知矛盾）→ D（把零消费的查询函数变成真消费）→ B（建 ABox，本体才有实体可讲）→ A（标准化，最后做）**。
