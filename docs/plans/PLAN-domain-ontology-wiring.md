# PLAN：领域本体接入智能体（domain-ontology-wiring）

> 状态：**评审中（未实现）** · 评审拍板点见 §7 · 明确不做见 §8
>
> 本计划只覆盖「**finance 域业务本体**（`harness::domain_ontology`）如何接入 agent 侧」，不把「能力域」并进本体（理由见 `domain_ontology.rs` 模块头 L1/L2 分层与 `PLAN-domain-single-source.md` §9 事故）。

## 1. 目标

把 `src-tauri/crates/harness/src/domain_ontology.rs` 从「只被自身单测 + JS 门禁守护」推进到「**被生产链路线性消费**」，理顺「能力 / 本体 / 能力发现」分工，**并把「本体可扩展」作为方案的一等设计约束**——扩展本体（finance 域内加类、加档；乃至新增整个能力域的本体）都应是**注册式 / 表驱动**的，不需要改消费逻辑。

## 2. 概念分工（消除矛盾）

```
域路由(→finance)  →  能力发现(选中能力)  →  本体语义查询(类/关系/分档/权重)
```

| 概念 | 模块 | 职责 | 处置 |
|---|---|---|---|
| 能力域（9 值） | `capability::CapabilityDomain` | 归类轴 | 不动 |
| 能力发现 | 链路中段 | finance 路由后选可执行能力 | 不动商业模式，本体查询挂其之后 |
| 领域业务本体 | `domain_ontology` | 类/关系/公理/分档/权重 权威源 | **本方案激活其生产消费 + 注册式扩展** |
| `.rhai` 分档/权重字面量 | 两个 `.rhai` | 真实算分 | P0 收敛到本体 |

矛盾根源：两套 `.rhai` 各自维护分档，本体只是「比对基准」而非「被执行源」。P0 消除这层「声明 vs 执行」分裂。

## 3. 实证基线（2026-09-16 复核）

| 声明 | 生产消费方 | 门禁覆盖 |
|---|---|---|
| `BOTTLENECK_WEIGHTS` | 1（`seed_serenity.rs:1268`） | ✅ `SITES` |
| `READINESS_BANDS` | 0 | ✅ `SITES`（分档阈值） |
| 查询接口（`band_for_score`/`class_by_id`/`subclasses_of`/…） | 0 | ❌ 无 |

### 3.1 档名不一致（仲裁新事实）

`band_for_score` 返回档名与两个 `.rhai` 现场不一致：

| 站点 | 高→中档名 | 末档 |
|---|---|---|
| 本体 `READINESS_BANDS` | `strong_bottleneck`/`potential_bottleneck` | `no_signal` |
| `bottleneck-calc.rhai:221` | 同上（**一致**） | `no_signal` |
| `strategy-scorer.rhai:64` | **`strong_signal`/`potential_signal`**（**不同**） | `no_signal` |

- `bottleneck-calc` 档名 = 本体档名 → P0 直接用 `band_for_score` 零改动下游。
- `strategy-scorer` 档名不同且写在 **6 处产出**（`:250/:296/:342/:388/:446/:494`）→ 强改会改变下游消费字符串。

### 3.2 现行三处「finance 焊死」（扩展能力的天花板）

| # | 位置 | 焊死点 | 后果 |
|---|---|---|---|
| 1 | P1 工具名 `finance_ontology_query` + 挂载 `builtin-finance` | 域写进名字/组 | 新增 devops 域要新建另一个工具 |
| 2 | `src-tauri/crates/tools/src/registry.rs:1145` `ToolCategory::Finance → builtin-finance` | 域 → 组的映射 | 同样域级写死 |
| 3 | `check-ontology-consistency.mjs` `SITES`（`expect: 3` 硬编码） | 站点清单手动维护 | 加站点要改 JS |
| 4 | （隐含）`.rhai`/前端对档名/类 ID 精确匹配 | 隐性契约 | 加档/改名静默破坏下游 |

## 4. 分层方案（含扩展机制）

### P0（独立 PR）—— 分档收敛到本体

把两处 `.rhai` 独立分档改为**读本体**：
- `bottleneck-calc.rhai:221`：直接调用 `band_for_score(final_composite)`（rhai 调 Rust 已有先例 `crates/tools/src/rhai_engine.rs:87` `register_fn("tool")`）。行为等值。
- `strategy-scorer.rhai:64-69`：需先裁决档名（§7-a），b1 注入阈值常量保 `_signal` / b2 用 `band_for_score` 改 6 处产出。

**连带（硬约束）：** `check-ontology-consistency.mjs` 的 `rhai-band-threshold` 两条站点当前抓分档字面量。P0 改掉分档后须同步改为「断言对 `band_for_score` 的调用存在」——不改则 CI 自洽矛盾。

### P1（独立 PR）—— 通用本体查询工具（扩展机制第 1 处）

**工具不叫 `finance_ontology_query`，叫 `ontology_query`，入参 `domain`**：

- 输入：`domain`（`CapabilityDomain.as_str()`）+ `concept` 或 `score`。
- 输出：该域下的类 / 子类 / 分档 / 层级快照；`finance` 是它对本体路由后的一个实例。
- 注册：**表驱动**挂进所有开启的业务域工具组，而非新增一域就写一次 —— 消除 §3.2 #1/#2 的焊死。

### P1b（扩展机制第 2 处）—— 域 → 本体注册表

新增 `domain_ontology::DomainRegistry`（或并入 `domain_registry`），把「域 × 本体内容」登记为**可查询表**：

```
domain (CapabilityDomain) → ontology tables (classes/relations/bands/weights)
```

finance → 现有 `CLASSES`/`READINESS_BANDS`/…；其它域 → `&[]` 或按需声明自己的表。新域接入 = **在注册表加一行**，查询工具自动可见，零 API 改动。

### P0 连带补（扩展机制第 3 处）—— 门禁表驱动

`SITES` 的 `expect` 期望条数由**扫描本体声明自动推导**（解析 `CLASSES`/`READINESS_BANDS`/`DOMAIN_NODES` 长度），取代手写 `expect: 3`。新增域/类/档 → 门禁期望自动更新，无需改 JS 清单。

### P1b 连带（扩展机制第 4 处）—— 档名/类 ID 禁止写死

- 门禁新增一条：**消费侧（`.rhai` 产出、前端组件）不得对档名 / 类 ID 做精确字符串匹配**，必须从本体枚举流出（或列入 `OPEN_DIVERGENCES` 登记）。
- 使 §3.2 #4 的隐性契约显性化 —— 这是「扩展不破坏下游」的保险。

### P2（可选，高风险）—— 语义注入 prompt

仅 P0/P1 消费后仍有强诉求才做。必须**结构化注入**（类表/关系表/分档窗格），禁止散文注入，避免重现 `hallucination_guard` 关掉的坑。

## 4.3 通用查询工具 + 注册表落地设计（§7-d 细化）

> 本节回答：`ontology_query(domain, ...)` + 「域 → 本体注册表」具体怎么落、新增一个能力域本体要动哪几处。

### 4.3.1 注册表数据结构（harness 内新增，纯声明）

在 `domain_ontology.rs` 新增一条**域级索引**，把「域 × 该域的本体表」绑定为一条静态记录：

```rust
pub struct OntologyDomain {
    pub domain: CapabilityDomain,          // 权威 key（as_str() = 协议 id）
    pub classes: &'static [OntoClass],
    pub object_properties: &'static [ObjectProperty],
    pub metrics: &'static [Metric],
    pub bands: &'static [Band],
    pub weights: Option<BottleneckWeights>, // 无则不参与权重评分
}

pub const ONTOLOGY_DOMAINS: &[OntologyDomain] = &[
    OntologyDomain { domain: CapabilityDomain::Finance, classes: CLASSES, /* … */ },
    // 其它域缺省为空表，由后续按需补充
];
```

- **finance 现成的 6 张表原样塞进第一条**（`CLASSES`/`OBJECT_PROPERTIES`/`METRICS`/`AXIOMS`/`READINESS_BANDS`/`BOTTLENECK_WEIGHTS`），**不重排、不改字段** —— 种子产物逐字节不变，不升 `TEMPLATE_VERSION`。
- 现有 finance 全局函数（`class_by_id` / `subclasses_of` / `band_for_score`）**保持不变**作为 finance 的便捷封装，**内部改为经注册表查找**（`ONTOLOGY_DOMAINS` 里取 finance → 现有逻辑）—— 消除双份数据，旧的 2 处生产消费方（`seed_serenity.rs:1268`）与门禁的 `SITES` 全部无感。

### 4.3.2 新增两个域化查询入口（对外唯一新增 API）

```rust
/// 按域解析本体；未登记域返回 None（不 panic）。
pub fn ontology_of(domain: CapabilityDomain) -> Option<&'static OntologyDomain>;

/// 域内按类 ID 定位类；跨域调用经 domain 路由。
pub fn class_by_id_in(domain: CapabilityDomain, id: &str) -> Option<&'static OntoClass>;
```

- 新入口**只读、无 IO**，延续本模块「纯声明 + 纯查询」约束。
- finance 既有全局函数改为 `ontology_of(Finance).and_then(...)` 的薄壳，**保持签名不变**。

### 4.3.3 通用工具 `ontology_query`（P1）

- **tool def**：`name: "ontology_query"`，入参 `{ domain: string, concept?: string, score?: number }`，`category: Knowledge`（只读，不进 `builtin-finance` 单域组，避免 §3.2 #2 的域写死）。
- **执行逻辑**：`ontology_of(domain)` → `None` 返回 `DOMAIN_UNREGISTERED`（harness `error_codes`，需同步 11 语言）；`Some` → 按 query 取 `class_by_id_in` / `subclasses_of` / `band_for_score`，返回 JSON。
- **注册**：在现有工具注册点（`registry.rs` 的 `register_all` 清单）追加一个实例即可，**一次注册全域可见** —— 不按域重复造工具。

### 4.3.4 新增一个能力域本体的最小改动清单

以新增 devops 域本体为例（仅打样思路）：

1. `domain_ontology.rs`：定义 `DEV_OPS_*` 表（或先空表），在 `ONTOLOGY_DOMAINS` 加 `OntologyDomain { domain: Devops, … }` 一行；
2. `domain_registry.rs`：若要进导航，`DOMAIN_NODES` 已声明则不动（devops 已存在）；
3. JS 门禁（经 §P0 连带改为表驱动后）**自动覆盖**新行，无需手改 `SITES`；
4. **消费零改动**：`ontology_query` 已按 domain 路由，自动可查。

> 对比原方案（每个域一个 `xxx_ontology_query`）—— 这里**新增域不造工具、不改注册表代码，只加一行数据**，是「扩展能力内生」的落点。

### 4.3.5 代价与风险

- **新增成员**：1 个 `OntologyDomain` 结构 + 2 个查询函数 + 1 个 `tool` def + 1 条注册 → 均在 `harness` / `tools`，无 schema、无 DB。
- **回归风险**：finance 全局函数改经注册表代理 ⇒ 需 `cargo test` 锁定这 2 处生产消费方与既有单测等价；门禁 `SITES` 只在 P0 连带时改一次。
- **真空表**：其它 8 域注册表缺省空 ⇒ `ontology_query(devops)` 返回空表而非报错，语义明确。

## 5. 扩展能力：本方案如何满足「加本体不改消费」

| 要做的扩展 | 只需做什么 | 消费逻辑改不改 |
|---|---|---|
| finance 域内加类/关系/公理 | `CLASSES` 加一条 | 不改（纯查询遍历） |
| finance 加分档/权重 | `READINESS_BANDS`/`*_PARTS` 加项 | 不改（`band_for_score` 读表） |
| finance 加度量 | `METRICS` 加项 | 不改 |
| **新增能力域本体** | **注册表加一行**（域→表映射）+ `DOMAIN_NODES` 声明 | 不改（`ontology_query(domain)` 自动可见） |
| 消费侧精确匹配 | 本体 ID 流出 / 登记分歧 | 门禁防回写死 |

> 这条主线就是本方案把「本体可扩展」做成**注册式**而不是「每个域造一个工具」的根本原因。

## 6. 影响面 / 代价

| 变更 | 文件 | 代价 / 风险 |
|---|---|---|
| P0 | 2 个 `.rhai` + `rhai_engine.rs` + `seed_serenity.rs` | 低～中；strategy-scorer 档名需裁决 |
| P0 连带 | `check-ontology-consistency.mjs` | 低；**必须同步改否则 CI 红** |
| P1 | `ontology_query` 工具 + 工具组表驱动注册 | 中；独立 PR |
| P1b | `domain_ontology` 增加注册表 + 门禁断言 | 低～中；`domain` 枚举已存在，无 schema 变更 |
| P2 | prompt 注入层 | 高；专门评审 |

## 7. 评审拍板点

- **a）strategy-scorer 分档实现**：b1 常量注入保 `_signal`（推荐，不牵动 6 处产出）vs b2 `band_for_score` 改档名 + 6 处产出。
- **b）门禁 `rhai-band-threshold` 两条站点**：改断言「调用 `band_for_score`」vs 删除硬拦。推荐前者。
- **c）档名分歧（`_signal` vs `_bottleneck`）**：登记进 `OPEN_DIVERGENCES`（D2）？推荐登记。
- **d）P1 是否做成通用 `ontology_query(domain)` + 注册表（P1b）**：这是「方案具备本体扩展能力」的关键，推荐合并纳入。
- **e）P2 是否做**：默认不做。

## 8. 明确不做

- ❌ 不把能力域枚举并进 `domain_ontology`（归类轴 vs 某域内容，非并列词汇表）。
- ❌ 不做等权/加权口径裁决（`OPEN_DIVERGENCES` D1，产品决策）。
- ❌ 不在 P0 前顺手统一 `strategy-scorer` 档名（替业务做语义决策）。
- ❌ 不给 8 个域全部建本体（只搭「注册式」管线，域内容按需补充）。

## 9. 落地顺序

1. 本方案评审通过（尤其 §7 a/b/c/d）。
2. P0（含门禁连带 + 扩展机制 #3 门禁表驱动）→ `cargo clippy -D warnings` + `npm run check:ontology` 全绿。
3. P1 + P1b（通用 `ontology_query` + 注册表 + 档名写死断言）独立 PR。
4. P2 另行立项。