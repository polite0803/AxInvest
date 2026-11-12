# PLAN-domain-single-source — 「域」多载体收敛为一套

> 状态：active ｜ 日期：2026-10-11 ｜ 对应版本：v2.9.9


> 结论先行：**同一条语义轴（能力域）上确实存在「1 份权威 + 6 类手抄副本」，应当收敛；
> 但另有 5 处「也叫 domain 却不同义」的东西不该并进来 —— 要合并的是副本，不是词。**
>
> ⚠ 本文的「文件:行」是**时点坐标**，以**符号锚点**为准（常量名 / 函数名 / 结构体字段）。
> 行号会漂移，符号可 grep（判据 #186–#189）。

---

## 1. 权威源（唯一）

| 项 | 位置（符号锚点） | 形态 |
|---|---|---|
| 枚举 | `crates/harness/src/capability.rs` · `pub enum CapabilityDomain` | 9 值：8 业务域 + `System`（内部域） |
| 字符串化 | 同文件 · `impl CapabilityDomain` 的 `pub fn as_str` | `CapabilityDomain::Xxx => "snake_case"` |
| 反解析 | 同文件 · `impl std::str::FromStr for CapabilityDomain` 的 `fn from_str` | 9 个新值 + **27 条历史别名** |
| 内部域判定 | 同文件 · `pub fn is_system` | `matches!(self, CapabilityDomain::System)` |

**已收敛的两个先例（正确形态，勿重新分裂）**：

1. `crates/harness/src/tool.rs` · `pub use crate::capability::CapabilityDomain as ToolDomain`
   —— `ToolDomain` **不是第二套枚举**，是类型别名（2026-08 收敛，注释写明「唯一权威分类轴」）。
   ⇒ 全仓 `ToolDomain` 的 100+ 处引用（`crates/tools/src/tools/**`）**自动跟随权威源**。
2. `crates/harness/src/capability_clusters.rs` · `CapabilityCluster.domain: CapabilityDomain`
   —— L2 集群用**枚举字段**而非字符串，且有 4 个 `*_domain` 单测钉住派生关系
   （`test_cluster_id_prefix_matches_domain` / `test_every_business_domain_has_clusters` /
   `test_same_domain_clusters_are_contiguous` / `test_cluster_count`）。

**这两个先例证明：收敛是可行的，且本仓已经做过两次。**

---

## 2. 同轴副本清单（本轮实测，全部需要被钉住）

| # | 副本 | 位置（符号锚点） | 值域 | 漏同步的后果 |
|---|---|---|---|---|
| 1 | TS 联合类型 | `src/types/capability.ts` · `export type CapabilityDomain` | 9 值 | 前端新域类型不合法 ⇒ 编译能过但 `as CapabilityDomain` 断言失效 |
| 2 | 前端域元数据 | `src/lib/domainMeta.ts` · `CAPABILITY_DOMAIN_META` | 8 值 + path/color/order/labelKey | 域路径页 404、侧栏少一项、图标无色 |
| 3 | 协议顺序 | `src/lib/domainMeta.ts` · `CAPABILITY_DOMAIN_PROTOCOL_ORDER`（**本轮新增**） | 8 值顺序 | 下拉项顺序漂移 |
| 4 | 导航归域表 | `src/lib/domainMeta.ts` · `NAV_ITEM_DOMAIN_MAP` | navKey→域 | 导航项落错域或落兜底 `general` |
| 5 | L1 分类器 prompt | `src-tauri/src/init/state.rs` · `const SYS` | 9 值 | **LLM 永远选不中新域**（模型兜底整条失效） |
| 6 | OPC 域包→域映射 | `src-tauri/src/commands/opc_workflows/mod.rs` · `OPC_DOMAIN_PACK_DOMAIN` | 14 条 → 8 目标域 | 域包落「未分类」 |
| 7 | OPC 模板段→域映射 | 同文件 · `OPC_WF_SEGMENT_DOMAIN` | 16 条 → 8 目标域 | 同上 |
| 8 | i18n 域标签 | `src/i18n/locales/*.json` · `capabilityDomain.*` | 9 key × **11 语言** | 界面显示裸 id（如 `data_analysis`） |
| 9 | i18n 域标签（第二套） | 同文件 · `domain.*` | 8 key，**camelCase** | 见 §4（重复命名空间） |
| 10 | 浏览器 mock | `src/lib/browserMock.ts` · `domainRules` | 若干 | 浏览器模式下路由演示失真（可接受） |

**本轮实测结果：10 份副本与权威源**当前全部一致**（集合层面）** —— 即这是**覆盖缺口**，不是现存冲突。缺的是「守」。

---

## 3. 已落地（2026-09-15，纯增量 + 零行为变更）

### 3.1 新增门禁 `scripts/check-domain-single-source.mjs`

- **6 类硬拦**：副本 #1 / #2 / #3 / #5 / #6+#7 / #8 的**集合**与权威源比对；外加「别名不得指向未声明变体」（幽灵域）。
- **报告档（不拦，依判据 #147）**：集合相同但顺序不同（可能是有意的 UI 决策）、`domain.*` 的 camelCase 命名（待产品裁决）、mock 副本。
- **拒绝假绿**：权威源解析不出 ⇒ `exit 2`（不按「0 违规」通过）；扫描面为 0 ⇒ `exit 2`；站点清单里每条抓不到 ⇒ 硬拦。
- **已接 CI**：`scripts/ci-check.mjs` 与 `.github/workflows/ci.yml` **两处逐字同源**（AGENTS 纪律）。
- 实测：`--selftest` **23/23**；裸跑 `exit 0`（16 文件 / 6 站点 / 27 条别名全 ✔）。

### 3.2 反事实取证 `output/domain-probe.mjs`

`--selftest` 用**合成输入**，通过只证明「比较器不恒真」，**不证明门禁读到了真实现场**（判据 #202）。
故对**真实文件**逐类别各注入 1 处漂移，实测 `DOMAIN_PROBE_OK=true`（6/6）：

| 注入 | 门禁原话（节选） |
|---|---|
| TS 联合类型删 `\| "system"` | `缺 system` |
| `PROTOCOL_ORDER` 删 `"ai_media"` | `缺 ai_media` |
| `CAPABILITY_DOMAIN_META` 的 id 改成 `communication_z` | `多（幽灵域）communication_z` |
| prompt 删 `automation` | `缺 automation` |
| OPC 映射写 `"finace"` | `映射目标域未在权威源声明（会静默落到「未分类」）：finace` |
| `zh-CN.json` 的 key 改成 `ai_media_RENAMED` | `缺 ai_media` |

每例单独还原 + **sha256 逐字节自校**，收尾 `restored_exit=0`。

> **探针当场抓到的一件事**：幽灵域用例首版写成 `communicationX`，门禁只报「缺 communication」
> —— 因为 id 抽取器只认 `[a-z_]+`，含大写的值被**整体跳过** ⇒ **extra 分支当时根本没被执行**。
> 是探针的 `drift_says=false` 把它抓出来的。已把该边界写进脚本头「已知边界」。

### 3.3 消掉最直白的一处重复定义（禁区 12）

`AgentProfileManager.tsx` 与 `ExpertSelector.tsx` 里的 **8 行域下拉 options 逐字节相同**（各 8 行 × 2 处），
且两处顺序都是「协议顺序」。已收敛为从 `domainMeta.ts` 派生：

```ts
// domainMeta.ts
export const CAPABILITY_DOMAIN_PROTOCOL_ORDER: readonly CapabilityDomain[] = [...];
export const CAPABILITY_DOMAIN_OPTIONS = CAPABILITY_DOMAIN_PROTOCOL_ORDER.map((id) => ({
  value: id,
  labelKey: `capabilityDomain.${id}`,
}));
```

**为什么单独声明协议顺序而不复用 `CAPABILITY_DOMAIN_IDS`**：后者的顺序是**导航**顺序
（`CAPABILITY_DOMAIN_META.order`：general→finance→automation→…），而 L1 路由 / 工具域过滤 /
prompt 清单用的是**协议（枚举）**顺序（general→devops→ai_media→…）。两者都合法，
混用会让下拉项在两次重构之间来回跳 —— 本轮**保住原顺序**，故下拉行为零变化。

验收：`tsc --noEmit` **exit 0**、`dprint check` **exit 0**、门禁 `exit 0`。

### 3.4 §4 / §5A / §5B 已实施（2026-09-15 第二轮）

**§4 i18n 双命名空间合并**（明细见下文 §4）：

- `domainMeta.ts`：**删掉手抄字段** `CapabilityDomainMeta.labelKey`，新增唯一派生入口
  `domainLabelKey(id)`（返回 `capabilityDomain.<id>`）。
- 3 个消费点同步：`DomainHubPage.tsx` / `AppHeader.tsx` / `Sidebar.tsx`。
- 11 个 locale：删除 `domain.*` 下 8 个重复域名 key，**保留** `description` / `empty`。
  文本级操作（不重新序列化 JSON），带 **4 重断言**（恰好 8 行 / 剩余仅 description+empty /
  改后 JSON 可解析 / `capabilityDomain` 仍 9 key），任一不符即整体 abort 不写盘。
  备份在 `output/backup-2026-09-15/i18n-domain-merge/`。
- 门禁收益：报告档由 **2 条降为 1 条**（`domain.*` 的 camelCase 告警消失）。

**§5A `TaskDomain` → `TaskKind`**（`rt-workflow/src/work_engine/domain_constraints.rs`）：
枚举名 25 处 + 方法名 `for_domain` → `for_kind` 4 处 + 参数/局部名 8 处。
`DomainConstraints` 类型名**保留** —— 它是跨 crate API（`init/state.rs` 引用），
且语义是「约束注册表」而非「域」。

**§5B `AnalystDomain` → `AnalystRole`**（`analysis-engine/src/evidence_weight.rs`）：
枚举名 36 处 + 函数名 `classify_domain` → `classify_role` 3 处。
**刻意不改**局部变量 `domain`（5 处）—— 它与 `AnalystWeight.domain` 字段（对外序列化
契约，前端消费）逐字对应，改了反而制造新的不一致。

**改名方式与自证**：`output/ax-rename-domain-enums.mjs`，每条规则带**命中数断言**
（不符即整体 abort、不写任何文件）。**断言当场抓到一处漏统计**：`classify_domain` 实为
**3** 处（先前 grep 被输出上限截断，只看到 2 处 —— 漏的那处在 `.map()` 闭包内）。
备份：`output/backup-2026-09-15/domain-rename/`。

**验收**：`rustfmt --check` 两文件 **EXIT=0**；`tsc --noEmit` **EXIT=0**；
`dprint check` **EXIT=0**；门禁 **EXIT=0**；`cargo check -p axagent-rt-workflow
-p axagent-analysis-engine --all-targets` 见 §8。

---

## 4. i18n 域标签命名空间 —— ✅ 已合并为单套（2026-09-15）

**合并前**（问题形态）：

| 命名空间 | key 形态 | 值域 | 消费点 |
|---|---|---|---|
| `capabilityDomain.*` | **snake_case**（与域 id 同名） | 9（含 system） | `AgentProfileManager`、`ExpertSelector`、`TemplateList`、`DemandDiscoveryPage` |
| `domain.*` | **camelCase**（`dataAnalysis` / `contentCreation` / `aiMedia`） | 8 + `description` + `empty` | `domainMeta.ts` 的 `labelKey` 经侧栏/内容区/头部渲染 |

**合并后**：`domain.*` 只保留 `description` / `empty`；域标签一律走
`domainLabelKey(id)` → `capabilityDomain.<snake_case>`。`domainMeta.ts` 不再手抄 label
⇒ **11 语言 × 8 个重复 key 消失**，且新增域时的漏同步面从 2 张表降为 1 张。

### ⚠ 方案原先漏掉的事实：两套 key 的值**并非全部相同**

实施前做了逐值比对（`output/ax-cmp-domain-labels.mjs`），发现 **6 处不一致**
（其余 5 个 locale 的 8 值全等）。由于 `labelKey` 已切到 `capabilityDomain.*`，
**这 6 处的界面文案随之改变**：

| locale | key | 原显示（`domain.*`） | 现显示（`capabilityDomain.*`） |
|---|---|---|---|
| ar | finance | التمويل（融资） | المالية（金融/财政） |
| fr | aiMedia | Média IA | Médias IA |
| hi | aiMedia | AI मीडिया | एआई मीडिया |
| **ja** | **devops** | **`DevOps`** | **`運用`** |
| ru | aiMedia | AI-медиа | ИИ-медиа |
| ru | communication | Коммуникации | Коммуникация |

**规律**：`capabilityDomain.*` 侧**系统性更本地化**（把 `AI` / `DevOps` 这类拉丁词也
译成了本族语：`ИИ` / `運用` / `एआई`），`domain.*` 侧较旧、混用拉丁词。
⇒ **判断：保留 `capabilityDomain.*` 侧的文案**（这 6 条更符合「11 语言全量本地化」的
项目纪律，且 `devops` 的 `運用` 与 `zh-CN` 的「运维」语义一致）。

**若判断相反**（要以原 `domain.*` 文案为准）：把对应 locale 的
`capabilityDomain.<key>` 值改回旧值即可，**不需要**恢复 `domain.*` 命名空间。

**回滚**：11 个 locale 的原文件在 `output/backup-2026-09-15/i18n-domain-merge/`。

---

## 5. 同名不同义：5 处也叫「域」的东西（**不要**并入门禁）

用户判断「多套载体 = 混乱」是对的，但混乱的**一半来自词**。以下 5 处与能力域**正交**：

| # | 名称 | 位置 | 真实语义 | 处置 |
|---|---|---|---|---|
| A | ~~`TaskDomain`~~ → **`TaskKind`** | `crates/rt-workflow/src/work_engine/domain_constraints.rs` | **任务类型**（Code/Research/Planning/Review/General/Browser） | ✅ **2026-09-15 已改名**（判据是「任务形态」，与「能力归属」无关） |
| B | ~~`AnalystDomain`~~ → **`AnalystRole`** | `crates/analysis-engine/src/evidence_weight.rs` | **分析师分工**（基本面/宏观/技术面/情绪/裁决） | ✅ **2026-09-15 已改名**（投资域**内部**角色粒度，不可复用） |
| C | `DemandDiscovery` 的 `domain_*` | `src/components/settings/DemandDiscoveryConfigPanel.tsx` + `src-tauri/src/commands/demand_discovery.rs` | **市场需求领域**（科技/设计/营销/…） | ⛔ **经核实否决改名**，理由见下 |
| D | `domain_semantics.rs` | `crates/harness/src/domain_semantics.rs` | **字段级概念域**（一名多义 / 量纲 `Ratio01`/`Percent100`） | 保留（模块头已自陈「字段层」定位，且是 `check-domain-semantics.mjs` 的输入源） |
| E | `domain_ontology.rs` | `crates/harness/src/domain_ontology.rs` | **本体**（类 / 关系 / 公理 / 权重常量） | 保留，但**正名为「finance 域业务本体」**（见 §9.1）——它实为**单域内容**，不是与能力域并列的另一轴 |

**理由**：把「域」这个词**独占给能力域**，此后「域」只有一个意思 —— 这才是用户说的「一套」。
A/B 已按此完成；D/E 保留是因为它们的「域」属**另一条正交语义轴**的既有术语，
强行改名会把两张正交词汇表搅在一起。

### ⛔ §5C 为什么不改名（方案原先的建议基于错误前提）

方案原建议把 `domain_*` 改成 `marketDomain_*`。核实后**否决**，三条硬理由：

1. **它不是「域定义载体」，是工作流模板的变量名。**
   这些名字写入 `workflow_template(id="demand-discovery")` 的 `variables`（**DB 数据**），
   前端 `DemandDiscoveryConfigPanel.tsx` 只提供默认值。方案表格把它列为「同名不同义」
   没错，但**改名不是命名治理，是数据迁移**。
2. **两侧之间存在一条隐式跨语言契约，且失效方式是静默的。**
   Rust 侧 `commands/demand_discovery.rs` 的 `extract_domain_queries()` 用
   `name.starts_with("domain_")` **按前缀通配**取用：
   ```rust
   if name.starts_with("domain_") { /* 展开为搜索关键词 */ }
   ```
   只要**任一侧**改名而另一侧没跟上，该函数就取不到任何关键词，直接落到
   `queries.is_empty()` 的**硬编码兜底**（`["AI","软件","设计","营销","写作","翻译"]`），
   **不报错、不告警** ⇒ 需求发现扫描静默退化成固定 6 个关键词。
   即：这不是「改错会红」的重命名，而是 **fail-open 静默降级**。
3. **收益极小、风险面很大**：需同时动 Rust 前缀、前端默认值、11 语言的
   `demandDiscovery.configDescriptions.domain*` key，以及 DB 里已存的用户配置值。

**若要真做，前置条件是先补守卫**：给 `extract_domain_queries()` 的兜底分支加
「未取到任何 `domain_*` 变量」的 `tracing::warn!`（或返回错误码），让失效**可见**，
再谈改名。**本条已登记，未实施**。

---

## 6. 「可供用户修改」的实况

| 用户能不能改 | 事实 | 位置 |
|---|---|---|
| **不能改域定义本身** | 枚举是编译期契约；`capability.rs` 的模块注释明确「**禁止引入自定义/产品线域**」「业务线通过 tags 表达，不占域轴」 | `capability.rs` · `pub enum CapabilityDomain` 的 rustdoc |
| **能「选域」（在 8 个里勾选子集）** | 角色/专家的 `active_domains` 存 DB（TEXT/JSON），UI 是 `AgentProfileManager` 与 `ExpertSelector` 的 multiple Select | `agent_roles` / `agency_experts` / `business_roles` 三张表的 `active_domains` 列 |
| **能「排除域」** | `capability_policies.exclude_domains` | `crates/entities/src/capability_policies.rs` |
| **不能改 L1 路由规则**（✅ 已于 2026-09-15 **只读化**） | `DomainRouter` trait 原先还有 `add_rule`/`update_rule`/`remove_rule`/`reorder_rules`，但**生产零消费**：唯一生产实例化是 `init/state.rs` 的 `Arc::new(DomainRouterImpl::new())`（另两处 `conversations` 同构），规则集是**内存态**（`tokio::sync::RwLock::new(default_domain_rules())`）⇒ 无 Tauri 命令、无 UI、**重启即回内置 9 条**。四个方法**已按裁决删除** | `crates/harness/src/domain_router.rs` · `DomainRouterImpl` |

⇒ 结论：**域定义是「只读契约 + 用户可选子集」**，这是**正确的设计**（枚举改动会击穿 DB
存量字符串与 11 语言 key）。真正的缺口是**副本没人守**，而不是"用户改不了"。

### ✅ 已裁决（2026-09-15）：选项 ① 只读化

| 选项 | 动作 | 结论 |
|---|---|---|
| **① 只读化** | 删掉 4 个 CRUD trait 方法（它们零消费） | ✅ **已采纳并落地** |
| ② 真可配 | 规则落 DB + Tauri 命令 + 设置页 UI | 未采纳（需版本治理 + 冲突消解，收益未证明） |
| ③ 现状登记 | 只在模块头标注「零消费、内存态」 | 未采纳（仍会留着"看起来能配"的假象） |

**落地内容**：删除 `DomainRouter` trait 与 `DomainRouterImpl` 的
`add_rule` / `update_rule` / `remove_rule` / `reorder_rules`，并同步删除测试替身
`MockDomainRouter`（`cognitive_router.rs`）里的同名实现。

**保留**：`route` / `list_rules`（**有生产消费** —— `cognitive_router.rs` 的
`DefaultCognitiveRouter` 用规则目标域扩展 L1 判定覆盖面）/ `decide` / `get_rule`
（纯读；零消费，但不构成「可配置」语义，故与 CRUD 区别对待）。

**未删但同样零消费**（登记，未擅自扩大范围）：`DomainRouterImpl::with_rules` 构造器 ——
文档写着「用于测试或运行时覆盖」，实测**测试与生产都没用它**。保留理由是它属构造期入口
（不构成「运行时可配」假象），且删它属另一轮范围。

**防回归**：trait 与 impl 文档均已写明「需要自适应路由时应重新设计完整链路
（落 DB + 命令 + 设置页 + 版本治理 + 冲突消解），**不要**只把方法加回来」。

### §6.1 ⏸ L2 `ClusterRouter` —— 比 L1 更彻底的「假可配」（2026-09-15 复核新发现）

**为什么上一轮没发现**：§6 只审了 L1。删完 L1 后做同轴扫描才看到 L2 是**同一形态的第二份**——
但这次不是「方法零消费」，而是「**数据源本身恒空**」。

| 项 | L1 `DomainRouterImpl` | L2 `ClusterRouterImpl` |
|---|---|---|
| 初始规则集 | `default_domain_rules()` → **内置 9 条** | `rules: RwLock::new(Vec::new())` → **空** |
| 写方法 | ✅ 已删 4 个 | ❌ **仍在**：`add_rule` / `update_rule` / `remove_rule` / `set_default_cluster` |
| 写方法生产调用 | — | **0**（全仓 grep 只命中定义 + `MockClusterRouter` 测试替身） |
| 生产实例化 | `init/state.rs` · `DomainRouterImpl::new()` | `init/state.rs` · `ClusterRouterImpl::new()`（另两处在 `commands/conversations`，变量名 `test_cluster_router`） |
| 读方法消费 | `list_rules` ✅ 有（`cognitive_router.rs`） | `list_rules` ✅ 有（同处） |
| 构造期入口 | `with_rules`（零消费，登记保留） | `with_rules`（**同样零消费**） |

**后果（比 L1 严重）**：`list_rules` 的消费点 `DefaultCognitiveRouter::list_l2_categories`
（`cognitive_router.rs`）**首个循环恒空转**：

```rust
for rule in self.cluster_router.list_rules(d).await { /* 规则集永为空 ⇒ 从不执行 */ }
for c in crate::capability_clusters::clusters_by_domain(d) { /* 实际唯一来源 */ }
```

即它自称「`ClusterRouter` 规则目标簇 ∪ 静态集群定义」，**实际产出恒等于静态定义**，且**不报错**。
（L1 至少还有 9 条内置规则可读；L2 连数据都没有。）

**判据**：#206 同族 —— 「有读接口 + 有消费点」极易被读成「这条链是活的」，
但**读的是一张永空的表**。

**三选项**（同 §6 形态，待裁决）：

| 选项 | 动作 |
|---|---|
| **① 只读化**（与 §6 一致，**建议**） | 删 4 个写方法；`list_l2_categories` 的规则循环一并删除（或保留 + 标注「恒空」） |
| ② 补数据 | 在 `ClusterRouterImpl::new()` 给出默认规则集（与 L1 的 `default_domain_rules()` 对齐） |
| ③ 现状登记 | 仅在 trait 文档标注「写接口零消费、初始规则集为空」 |

理由：L1 已建立先例，两个 trait 应同形态；且与用户「多套载体 = 混乱」的裁决同向。

---

## 7. 不变量（改任何与域相关的东西前必读）

1. **不改 9 个域 id 的字面值** —— DB 存量字符串（`capability_clusters.domain`、
   `workflow_template.route_path` 的 `/{domain}/{cluster}/{capability}`、
   `agent_roles.active_domains`、`capability_policies.exclude_domains`）全部按字符串持久化。
2. **`FromStr` 的 27 条历史别名不可删** —— 兼容存量标注（`core`/`invest`/`opc`/`quant`/
   `stock_analysis`/`device`/…）。删了不会编译报错，但存量数据会静默解析失败。
3. **`System` 永不进入导航与检索** —— 仅配合 `Visibility::SystemOnly`。
4. **`General` 是唯一兜底域** —— `domainForNavKey` 未知 key 落 `general`。
5. 新增/改动域 ⇒ **跑 `node scripts/check-domain-single-source.mjs`**，它会点名每个未同步的副本。

---

## 8. 执行状态

| 项 | 状态 |
|---|---|
| §3.1 门禁（6 类硬拦 + 报告档 + 接 CI） | ✅ 已落地 |
| §3.2 反事实取证（`DOMAIN_PROBE_OK=true`） | ✅ 已落地 |
| §3.3 组件重复下拉收敛 | ✅ 已落地 |
| §4 i18n 双命名空间合并 | ✅ **已落地**（2026-09-15；含 6 处文案变更，见 §4） |
| §5A/B 「域」独占化（`TaskDomain`→`TaskKind`、`AnalystDomain`→`AnalystRole`） | ✅ **已落地**（2026-09-15） |
| §5C `DemandDiscovery` 的 `domain_*` 改名 | ⛔ **经核实否决**（跨语言隐式契约 + 静默降级，见 §5） |
| §5C 前置：`extract_domain_queries` 兜底守卫（失败须可见） | ⏸ 已登记，未实施 |
| §6 L1 路由规则只读化（选项①） | ✅ **已落地**（2026-09-15，删 4 个 CRUD 方法；本节此前误标「待裁决」，已修正） |
| §6.1 L2 `ClusterRouter` 写接口（同样零消费，且**初始规则集为空**） | ⏸ **2026-09-15 复核新发现，待裁决** |
| **§9.1 P0 域元数据声明化** | ✅ **已落地**（2026-09-15；`domain_registry.rs` + 门禁 8 站点） |
| **§9.2 P1-① 别名迁入声明** | ✅ **已落地**（含行为级单测 + 门禁 3 类新硬拦 + 探针 12/12） |
| **§9.2 P1-② 协议顺序去手抄 + 门禁顺序硬拦** | ✅ **已落地**（2026-09-15 第三轮；顺序 + prompt 清单均升**硬拦**。⚠「副本未消除」，见 §8.2 诚实边界 1） |
| **§9.2 P1-③ `DomainNode::label_key()` + 两侧公式一致断言** | ✅ **已落地**（2026-09-15 第三轮；门禁新增 4b 段跨语言公式比对） |
| **§9.2 P1-④ 消除 TS 手抄 id 集合** | ✅ **已落地**（2026-09-15 第四轮；`DOMAIN_PRESENTATION` 单点 + `tsc` 穷尽检查 + 两个派生视图 + 门禁 4 类防回流。⚠ `NAV_ORDER` 仍是前端第二处手写点，见 **§8.4**） |
| **§9.3 P2 域 Action Type（overlay + DB + 命令 + UI + 3 个消费端）** | ✅ **已落地**（2026-09-15 第五轮；迁移 v231 + DAO + 2 个命令 + 设置页面板 + 11 语言，**3 个消费端各带「改前/改后」证据**。⚠ 6 条诚实边界与「备份是完成态快照」见 **§8.5.6 / §8.5.8**；静态自证、两次自我纠错、`disallowed_types` 漏豁免（已修）见 **§8.5.9 / §8.5.10**） |
| §9.3 P2 的**编译/测试闭环** | ✅ **已闭环**（明细 §8.5.11）：`clippy`（3 crate / `-D warnings`）**`EXIT=0`** · `check -p axagent --all-targets` **`EXIT=0`** · **我的 32 条 P2 用例全 `ok`**（含四条消费端验收与 4 组对照）· 域门禁 / 本体一致性 / `dprint` 全过。**外部噪声已逐条取证并剔除**：`TEST_EXIT=101` 的 4 条失败（fleet `v229` ×3 + `knowledge_graph` evidence 行号守卫 ×1）**均为他人在飞改动**；`check-layer-discipline`（L1 棘轮）红的 9 处**全在他人文件** —— 三项**只登记不代改** |
| **L1 分层门禁超基线（批次 H）** | ✅ **已修代码，未抬基线**（明细 §8.5.12）：`no-direct-db` **651/651** · `no-sibling-call` **76/76** · `EXIT=0` · `selftest 51/0` · **无「低于基线」提示**。三处按各自依赖图事实分修（MC 内核下沉服务层 / DB 访问下沉 dao / 换 dao 类型出口），**未加豁免、未调数字**。⚠ 我上轮报的「超基线 9 处」与「0 warning」**均已自行更正**（实为 4 对；日志被 `tail` 截断 ⇒ 不能作数）。**clippy 4 条 error 经 mtime 窗口 + 对方自认双重取证，确认属他人** ⇒ 只登记不代改（可粘贴补丁见 `output/p2-clippy-attribution-and-patch.md`） |

### 8.1 P1-① 实施中暴露的三个自身缺陷（均已修，记录以防重犯）

P1-① 的「别名迁出 `FromStr`」这一步**同时打断了三处依赖旧形态的检查**，而其中两处
在第一次复跑时**仍然报绿** —— 值得单列：

| # | 缺陷 | 为什么危险 | 修法 |
|---|---|---|---|
| 1 | 门禁 `parseAliasTargets` 读的是 `FromStr` 的扁平 `match`；别名迁出后该 `match` 不存在 ⇒ 正则匹配 **0 条**，而函数返回 `[]`（**不是 `null`**） | 主流程据此打印「历史别名 0 条 ✔」并 **exit 0** —— **fail-open 假通过**（判据 #7 / #216）。即：别名检查整段已死，门禁仍报绿灯 | 改为 `parseDeclaredAliases` 读 `DOMAIN_NODES.aliases`，契约改为「0 条 ⇒ `null` ⇒ 硬拦」；新增幽灵域 / 遮蔽规范 id / 跨域重复**三类硬拦** |
| 2 | selftest 的合成常量 `REG` 未同步新增的 `aliases` 必需字段 | 3 个用例从「通过」变「解析出 null」（`48 passed / 3 failed`）。**合成输入与真实文件不同形 ⇒ 假信号**（判据 #202 的反面） | 补齐合成常量；并注明「合成串必须与被解析文件同形」 |
| 3 | 探针 ⑦ 的漂移锚点写的是**旧条目形态**（无 `aliases` 行）⇒ 命中 0 次 | 探针 FATAL（**这个守卫是对的**，见 §3.2）：锚点失效必须显式失败，不得静默跳过 | 锚点补 `aliases: &[…]`；新增 ⑪ 遮蔽 / ⑫ 跨域重复两个真实注入用例 |

⇒ **可复用教训**：「把 A 处的数据搬到 B 处」不是单文件改动，**所有读 A 处的工具都要同步**，
且**每一个都要用真实注入证明它仍然活着**（只看 `exit 0` 会把「检查已死」当成「检查通过」）。


**验收快照（2026-09-15 第二轮）**：门禁 `--selftest` exit 0、裸跑 exit 0（报告档 **2→1 条**）；
`tsc --noEmit` exit 0；`dprint check` exit 0；`rustfmt --check`（两改名文件）exit 0。

**本轮改动清单**（未用 git 反查，逐项记录）：
`src/lib/domainMeta.ts`、`src/pages/DomainHubPage.tsx`、`src/components/layout/AppHeader.tsx`、
`src/components/layout/Sidebar.tsx`、`src/i18n/locales/*.json`（11 个）、
`src-tauri/crates/rt-workflow/src/work_engine/domain_constraints.rs`、
`src-tauri/crates/analysis-engine/src/evidence_weight.rs`、
`scripts/check-domain-single-source.mjs`（仅注释：旧名字引用已过期）、本文件。

**备份**：`output/backup-2026-09-15/i18n-domain-merge/`（11 locale 原件）、
`output/backup-2026-09-15/domain-rename/`（2 Rust 原件）。

### 8.2 P1-②③ 实施记录（2026-09-15 第三轮）

**验收快照**（全部当场实测）：

| 项 | 结果 |
|---|---|
| 门禁 `--selftest` | **47 passed / 0 failed**（P1-① 收尾时 33 → 本轮 +14） |
| 门禁裸跑 | **exit 0**；站点 **9** 个；报告档 **1** 条（`browserMock.ts` mock，可接受） |
| 探针（真实文件注入） | **`DOMAIN_PROBE_OK=true`，17/17 用例 `case_ok=true`**，全部 `restore_byte_identical=true` |
| 新增门禁站点 | `domain_registry.rs ↔ domainMeta.ts`：`capabilityDomain. ↔ capabilityDomain.`｜两侧公式一致 |
| `tsc --noEmit` / `dprint check` / `cargo fmt -p axagent-harness --check` | 均 exit 0 |
| **Rust 单测** | **11 passed / 0 failed**（`TEST_EXIT=0`，含 `test_label_key_formula`；时间核对通过） |

**P1-② 落地动作**：协议顺序比对由**报告档**升为**硬拦**（逐项；基准取 `declaredSlugs` = **声明顺序**，
而非 `authority` = 枚举顺序）；L1 分类器 prompt 的域清单顺序同样升级。
升级论证写在门禁头部新增的「为什么顺序在 P1-② 之后从报告档升为硬拦（#147 的反向适用）」一节 ——
要点：**判据没变软，是对象从「观点」变成了「派生量」**。

**P1-③ 落地动作**：Rust 侧新增 `DomainNode::label_key()`
（`format!("capabilityDomain.{}", self.slug())`）；门禁新增 **4b 段**：分别抽取两侧命名空间字面量并断言相等，
另校验 TS 侧模板插值必须是本函数参数（`simple` 标记，复合表达式如 `${globalThis.x}` 单独判）。
单测 `test_label_key_formula` 用**独立硬编码 key** 钉住（不从 `slug()` 派生 —— 派生版同源、恒真，判据 #174）。

**本轮发现并修掉的 4 个缺陷**（都不是新代码的 bug，是**存量**问题）：

| # | 缺陷 | 为什么危险 |
|---|---|---|
| 1 | 门禁里 `const metaOrder = parseMetaIds(metaSrc)` 是**悬空解析** —— 结果只用于 truthy 判断，无任何实际用途 | 典型「解析了但没比对」（判据 #197）：读代码者会以为顺序比对用了它 |
| 2 | 顺序比对基准取 `authority`（枚举顺序）而**非** `declaredSlugs`（声明顺序） | 二者被 1b 段强制相等 ⇒ **当前看不出差异**；但耦合方向错：声明顺序一旦漂移，1b 段拦的是声明、下游比的是枚举，等于**下游校验的是另一件事** |
| 3 | `domainMeta.ts` 注释声称「门禁**硬拦**本函数公式被改成别的命名空间」，门禁里**没有**这条检查 | 假声称（同族先例：`capability.rs` 曾声称 serde 别名「由门禁校验」而 `parseSerdeAliases` 零调用）。本轮 4b 段即补上这个守卫 |
| 4 | 新解析器对**复合插值**返回 `null`，门禁遂报「公式解析失败」 | 硬拦方向安全，但**诊断指向错误位置**（真问题是「插值不可信」而非「形态已变」）⇒ 拆出 `simple` 标记分档诊断。**这条是 selftest 新钉子抓出来的**，不是人工发现的 |

**并发改动事件（值得单独记）**：探针首跑 `DOMAIN_PROBE_OK=false`，但 **17 个用例全部 `case_ok=true`** ——
失败点在收尾的「全量还原自校验」。查证：`src-tauri/src/commands/opc_workflows/mod.rs` 在探针运行期间
（**22:07:41**）被**外部**改动（+153 字节，新增 14 行 `mod domain_pack` / `seed_domain_pack_*`），
并把 `OPC_INDUSTRY_DOMAIN` **改名**为 `OPC_DOMAIN_PACK_DOMAIN`（OPC「行业包 → 域包」术语统一）。

- ✅ **探针的守卫工作正常**：它把「工作区被第三方改动」判为失败，而非当成自己的还原失败 ——
  没有这道自校验，这次漂移会被记成「探针跑绿」。
- ✅ **门禁也没被绕过**：`parseOpcMappingTargets` 要求两张表都在，改名后 `found=1 ≠ 2 ⇒ null ⇒ 硬拦`。
- 🔧 **顺手改进**：常量名集中为 `OPC_MAPPING_TABLE_CONSTS`，报错消息**列出期望常量名**。
  此前诊断只说「两张表须都存在且非空」，改名致红时要回读源码才知道缺哪个名字。

**诚实边界（勿高估本轮成果）**：

1. ~~**P1-② 的「前端不再手抄」并未做到「消除副本」** —— 前端 `CAPABILITY_DOMAIN_PROTOCOL_ORDER`
   仍是 8 个手写 id。~~ **→ 2026-09-15 第四轮（P1-④）已补完**：该常量已改为
   `Object.keys(DOMAIN_PRESENTATION)` **派生**，id 手抄归零。「属 P1-④ 范围，未做」这句已过时，
   原文保留供追溯；结论与新的诚实边界见 **§8.4**。（「不引入生成物」的理由不变：会让 `tsc` 依赖 cargo 产物。）
2. **`label_key()` 无 Rust 运行时消费方** —— 消费方是门禁 + 单测，与 `nav_path` / `nav_order` 同形态
   （`domain_registry.rs` 模块头「消费方」表已自陈）。它不是「为门禁而写的死代码」：
   它把一条**跨语言字符串契约**从「两侧各自的实现细节」提升为「一处声明 + CI 强制镜像」。
3. **Rust 单测已闭环**（前两次尝试只拿到锁，第三次排队成功）：
   `cargo test -p axagent-harness --lib -- domain_registry` → **`11 passed; 0 failed`**，`TEST_EXIT=0`，
   含 `domain_registry::tests::test_label_key_formula ... ok`；日志中 `Compiling axagent-harness v2.9.9`
   确认本包被**重编**（不是复用旧 rlib）。
   按纪律 #175 核对时间先后：改动 **22:07:06** < 验证启动 **22:10:50** ⇒ **日志有效**。
   （此前 21:15 / 22:00 / 22:03 / 22:07 四次你的 `cargo run` 持锁期间，我**没动任何进程**。）

**本轮改动清单**（未用 git 反查，逐项记录）：
`src-tauri/crates/harness/src/domain_registry.rs`（+`label_key()`、+`test_label_key_formula`、模块头消费方表 +1 行）、
`src/lib/domainMeta.ts`（注释：假声称 → 指向真实检查；`PROTOCOL_ORDER` 的定性说明 + 勿降回报告档）、
`scripts/check-domain-single-source.mjs`（4b 段 + 2 个公式解析器 + 顺序升硬拦 + 悬空变量删除 +
`OPC_MAPPING_TABLE_CONSTS` + 头部文档 + selftest +14）、
`output/domain-probe.mjs`（+4 个真实注入用例）、本文件。

**备份**：`output/backup-2026-09-15/p1bc/`（4 个文件改动前原件）。

---

### 8.4 P1-④ 实施记录（2026-09-15 第四轮）

**目标（§9.2 原文）**：消除 TS 侧手抄副本；⚠ 不引入「运行时消费的生成物」；P1 只做「派生 + 门禁逐值比对」。

**落地形态**（`src/lib/domainMeta.ts` 模块头有同一张图）：

| 维度 | 载体 | 手写？ | 守卫 |
|---|---|---|---|
| 域 id **集合** | `DOMAIN_PRESENTATION`（`Record<BusinessDomain, {path,color}>`） | ✅ **唯一**手写点 | **`tsc` 穷尽检查**（漏一个域 ⇒ `TS2741`）+ 门禁集合比对 |
| 协议顺序 | `CAPABILITY_DOMAIN_PROTOCOL_ORDER = Object.keys(DOMAIN_PRESENTATION)` | ❌ 派生 | 门禁逐项比对 Rust `DOMAIN_NODES` 声明顺序 |
| 导航顺序 | `NAV_ORDER`（8 个 id） | ✅ 第二处手写 | 门禁集合比对 + 逐项比对 `nav_order`；`tsc` **只查元素合法性** |
| `path` / `color` | `DOMAIN_PRESENTATION` 的值 | ✅ | 门禁逐值比对 `nav_path`（`color` 刻意不比：纯前端决策） |
| 三个导出视图 | `CAPABILITY_DOMAIN_META`（`NAV_ORDER.map`）、`PROTOCOL_ORDER`、`CAPABILITY_DOMAIN_OPTIONS` | ❌ 全部派生 | 门禁新增 **2 类防回流**硬拦 |

**为什么不是「把两个数组都改成派生」**：域「集合」与「排序」是**两个独立维度**，一张 `Record`
只能表达一种声明顺序 ⇒ 只能是「`Record`（集合 + 协议序）+ 顺序数组（导航序）」。
两者**信息论上无法互相派生**，所以前端**必然**有两处手写 id。P1-④ 能做的三件事是：
① 让**集合**维度收敛为单点；② 让该点受编译器穷尽检查；③ 用门禁把两个顺序维度各自钉死。

**验收（全部实测）**：

| 项 | 命令 | 结果 |
|---|---|---|
| 门禁自检 | `check-domain-single-source.mjs --selftest` | **54 passed / 0 failed** |
| 门禁裸跑 | `check-domain-single-source.mjs` | **exit 0**（17 文件 / **9 站点**；报告档 1 条 mock） |
| 真实现场取证 | `output/domain-probe.mjs` | **`DOMAIN_PROBE_OK=true`（21/21）** |
| **`tsc` 穷尽检查取证** | `output/p1d-tsc-exhaustive.mjs` | **`TSC_EXHAUSTIVE_OK=true`（2/2）** |
| 类型检查 | `tsc --noEmit` | exit 0 |
| 格式 | `dprint check` | exit 0 |
| 单测 | `vitest run src/lib/__tests__/pageRegistry.test.ts` | **6 passed** |

**`tsc` 穷尽检查的实证** —— 这是本轮核心声称，而**门禁证不了它**（门禁只读源码文本，
看不见类型检查），故用独立脚本对真实文件注入、以 `tsc` 自己的输出判定：

```
A 删掉呈现表的 finance 条目：
  tsc  exit 1 — src/lib/domainMeta.ts(114,12): error TS2741:
    Property 'finance' is missing in type '{ readonly general: … }'
    but required in type 'Record<BusinessDomain, { readonly path: string; readonly color: string; }>'.
  门禁 exit 1 — 前端域集合（DOMAIN_PRESENTATION key） 与权威源不一致｜缺 finance
B 删掉 NAV_ORDER 的 "finance" 行：
  tsc  exit 0   ← ★ 不报
  门禁 exit 1 — 导航顺序集合（NAV_ORDER） 与权威源不一致｜缺 finance
```

⇒ 用例 B 是**诚实边界核查**：它把「`NAV_ORDER` 仍只有门禁守」变成实测事实，
并会在「某天 `tsc` 也开始报它」时失败，提醒更新本节（防止文档与实测分叉）。

**门禁新增 4 类硬拦**（对应探针 ⑱⑲⑳㉑）：呈现表形态漂移（缺 `} as const satisfies` 收尾 /
条目缺 `color` / key 重复）、`NAV_ORDER` 与声明不符、派生视图写回字面量数组、派生视图改从别处派生。

**诚实边界（勿高估本轮成果）**：

1. **前端仍是两处手写 id，不是一处** —— `DOMAIN_PRESENTATION` 的 8 个 key + `NAV_ORDER` 的 8 个 id。
   后者**只有门禁守**：`tsc` 查得出拼错、查不出漏项（用例 B 已实测）。
2. **协议顺序变成「隐式」** —— 它 = 呈现表 key 的声明顺序。为对齐 `color` 而重排该表会
   **连带改掉协议顺序**（门禁会红，但改动意图与后果毫不相干）。已在 `domainMeta.ts`
   与门禁头部写明「不要为排版重排该表」。
3. **「`order` 字段复活」这条防回归随解析器一并删除**（字段已无处可加）。替代物
   `parseDerivationShape` 拦的是「派生视图写回字面量数组」这一整类，**不检查字段名** ——
   即「往呈现表的值里塞新字段」不再由门禁守（是否被 `tsc` 的多余属性检查拦住**未实测**，
   登记为观察项，勿据此认定已有守卫）。
4. **`browserMock.ts` 的 `domainRules` mock 副本仍在**（报告档 1 条）—— 本轮未动，属既有欠账。

**本轮改动清单**（未用 git 反查，逐项记录）：
`src/lib/domainMeta.ts`（`BusinessDomain` + `DOMAIN_PRESENTATION` + `NAV_ORDER` + 两个派生视图 + 模块头结构图）、
`scripts/check-domain-single-source.mjs`（3 个解析器增删 + 主流程 3/4 段重写 + 2 类防回流 + 头部文档 + selftest）、
`output/domain-probe.mjs`（7 个用例换锚点 + 新增 4 个）、
`output/p1d-tsc-exhaustive.mjs`（**新建**：跨工具取证）、
`docs/plans/PLAN-domain-single-source.md`（本文件）。

**备份**：`output/backup-2026-09-15/p1d/`（3 个文件改动前原件）。

---

### 8.5 P2 实施记录（2026-09-15 第五轮）

**一句话**：域从「编译期枚举 + 只读声明」升级为「**声明（内置默认）∪ 覆盖层（用户可改）**」——
覆盖层落 DB、有命令与设置页面板，并且**三个消费端真的按它变了行为**（不是「有 UI」）。

#### 8.5.1 覆盖层（overlay）而不是「替换品」—— 本阶段最关键的一个决定

| 形态 | 后果 |
|---|---|
| 把域表**整体**搬进 DB（DB 成为权威源） | 首次启动 / DB 损坏时**一个域都没有**；门禁里「逐值比对内置声明」的断言失去稳定基准；9 个 id 与 11 语言 key 的契约散成两处 |
| **覆盖层**（DB 只存「与内置默认不同的部分」，无行 = 无覆盖 = 用内置默认） | 可回退（删表即出厂状态）；可对比（`has_override` 区分「内置启用」与「人工改过」）；内置声明仍是**纯静态 `const` 切片**，门禁的静态比对依然有效 |

第三条理由（门禁可测性）要单独说：若有人把 `DOMAIN_NODES` 改成 `static mut` /
`OnceLock<Vec<DomainNode>>` 并**就地改写**，则 ① 后续「逐值比对内置声明」的断言开始比对
**运行时可变的东西**（今天绿明天红）；②「出厂默认」概念消失（清空覆盖也回不到初始状态）。
⇒ 门禁新增 **§5b 段**硬拦这一形态（`DOMAIN_NODES` 必须仍是 `const` 切片）。

#### 8.5.2 三个消费端（验收核心）与各自的证据

| 消费端 | 接线点（**公共下游**） | 「改前/改后行为不同」的证据 |
|---|---|---|
| ① L1 分类器 prompt | `init/state.rs` 的 SYS 由 `l1_classifier_domain_list()` **派生**（不再手抄 9 个 slug） | `domain_registry` 单测 `test_disabling_domain_drops_it_from_classifier_list`（对**真实函数输出**断言）；`test_l1_classifier_domain_list_covers_all_when_enabled` 的期望值是**独立硬编码**的 9 个 slug 串 |
| ② L1 域路由 | `DomainRouterImpl::route()` 在**命中规则之后**新增启用闸门（`continue` 下一条） | `domain_router` 单测 `test_route_skips_disabled_domain`：同一输入 `分析股票301302` 改前 `== Finance`、改后 `!= Finance`（含对照组） |
| ③ 能力过滤 | `CapabilityFilterImpl::check_all()` 的**第一个**检查 `check_domain_enabled`（维度 `DomainEnabled`） | `axagent-tools` 单测 `test_check_all_rejects_disabled_domain`：同一护照改前 `Pass`、改后 `Reject{DomainEnabled}`，且**另一个域仍 `Pass`**（防「一停全停」） |
| （别名） | `resolve_enabled_domain` 参与用户输入 / LLM 输出的域解析 | DAO 单测 `extra_aliases_round_trip_and_delete_restores_default`（sqlite 内存库） |
| （落库链路） | DB → 运行时覆盖层 | DAO 单测 `upsert_then_load_changes_runtime_behavior`：写行 → `load_into_runtime` → `is_domain_enabled` / `enabled_domains` / `resolve_enabled_domain` 三处真的变了 |

**闸门为什么放在「命中之后」而不是之前**（消费端②）：放在命中之前只能表达
「这条输入压根没命中」，日志里**看不出**「本来会路由到 X，但 X 被停用」——
而后者正是排查「我停用了某个域，为什么行为不对」时唯一有用的那句话。

#### 8.5.3 两个解析入口被**刻意**分开（本阶段最容易做错的地方）

| 入口 | 语义 | 看覆盖层吗 | 不这样会怎样 |
|---|---|---|---|
| `CapabilityDomain::from_str` | **存量读取**（规范 id + 历史别名） | **不看** | DB 存量 `active_domains` / `route_path` 里的 `finance` 会静默解析失败（违反 §7 不变量） |
| `resolve_enabled_domain` | **用户输入 / LLM 输出入口**（内置 ∪ 追加别名，且**拒绝停用域**） | **看** | 刚停用的域仍能从自然语言解析出来 ⇒ 停用形同虚设 |

两者各有一条单测钉住：「停用后 `from_str` 仍能解析、`resolve_enabled_domain` 返回 `None`」。

#### 8.5.4 不可停用的两个域：读侧与写侧**同一判据**

`general`（唯一兜底域：`route()` 未命中的返回值 + `unknown()` 硬编码返回它）与
`system`（内部域：承载 `Visibility::SystemOnly` 元能力路由）不可停用。
**写侧**（命令层拒绝 `enabled=false`）与**读侧**（`is_domain_enabled` 恒 `true`）都过
`is_toggleable` —— 否则「坏数据绕过命令层直接进库」时，读侧会返回一个**没有合法兜底值**的状态。

#### 8.5.5 守卫**迁移**而不是删除（判据 #147 的现场）

消费端① 把 prompt 清单从手抄字面量改为派生后，门禁原来那段**正则逐值/逐序比对**失效。
按纪律不得「把红灯灭掉」，故做**迁移**：

| 原守卫 | 迁移到 | 强度变化 |
|---|---|---|
| 门禁：正则从源码文本抽 9 个 slug，与声明集比对 | Rust 单测对**真实函数输出**断言（`l1_classifier_domain_list()`） | **变强**（比「比源码文本」更接近行为） |
| ——（原先没有这一类） | 门禁改为**防回流**四分支（`missing_anchor` / `literal` / `derived` / `unknown_entry`） | 补齐了「抄回字面量」这一整类 |

门禁新增可测纯函数 `classifyPromptDomainSite(src)`（返回 4 种 `kind`），
使门禁自身能被 selftest 直接喂输入覆盖 —— 沿用本仓门禁的既有惯例。

#### 8.5.6 诚实边界（登记，勿当成已做）

1. **覆盖层的删除路径只有 DAO 层**（`delete_override`，含单测），**UI 无入口**。
   面板的「恢复默认行为」= `enabled: true` + 清空追加别名，**语义上等价于「没有覆盖」**，
   但不删行（`has_override` 仍为 `true`）。⇒ 若日后要求「回到出厂默认」且让徽标消失，
   需新增一条 `reset` 命令（本阶段刻意没加：它只影响一个展示字段，却要再走一遍
   11 语言错误码 + 命令注册 + browserMock 的全链路）。
2. **`browserMock.ts` 的 L1 路由 mock 不读覆盖层** —— 浏览器模式下「停用某域」不会改变
   `mockCognitiveRoute` 的结果。已在 mock 里写明为**已知偏差**（不顺手改：那会顺手改掉
   门禁报告档正在点名的那份 `domainRules` 副本，两件事该分开做）。
3. **覆盖层是进程内的**：`update_capability_domain` 只刷新**当前进程**；多窗口各自加载。
   跨进程一致性属既有欠账（本仓 DB 即单实例）。
4. **停用域不会让存量数据失效**：停用只影响「新解析 / 新路由 / 新过滤」，
   已落库的 `active_domains` 字符串照旧可读 —— 这是 `from_str` 不看覆盖层的直接后果，
   属**有意**设计（停用不是数据迁移）。
5. **`domain` 列刻意不加 `CHECK (domain IN (...))`**：加它会把这 9 个 id 又抄一份进 SQL
   （正是本 PLAN 要消灭的副本形态），且漏改时**静默拒绝写入**。
   合法性由「解析成枚举」保证（`v231` 迁移的单测同时钉住「非 9-id 字符串插入必须成功」
   —— 谁日后加 CHECK，这条用例会红并提醒他「你又抄了一份 id 集合」）。
6. **消费端① 的覆盖面限于「规则未命中」的输入**：`init/state.rs` 里那个 LLM 分类器是
   `DefaultCognitiveRouter::with_llm_reasoner(…)` 的**兜底**推理器（源码注释原文：
   「规则未命中时调用默认提供商做轻量分类」）⇒ 凡被 `DomainRouterImpl` 的 27 条别名规则
   命中的输入，**根本不会走到提示词那一步**。所以准确的表述是：
   - 消费端② （`route()` 闸门）对**所有**输入生效；
   - 消费端① 对**规则未命中**的输入生效，且它正好接在②之后 ——
     停用一个域 ⇒ 其规则被闸门拦下 ⇒ 该输入落到 LLM 兜底 ⇒ 兜底提示词里也没有它
     ⇒ 最终回落到 `general`。**两级串联，不是两条独立路径**。
   - 反面：若把这句写成「停用后每次路由都变」，就是把「接线」说成「每次生效」。

#### 8.5.7 本轮改动清单（未用 git 反查，逐项记录）

**Rust（14）**：`crates/dao/src/migrations/v231_capability_domain_overrides.rs`（新）、
`crates/dao/src/migrations/mod.rs`、`crates/dao/src/repo/capability_domain_override.rs`（新）、
`crates/dao/src/repo.rs`、`crates/entities/src/capability_domain_overrides.rs`（新）、
`crates/entities/src/lib.rs`、`crates/harness/src/domain_registry.rs`、
`crates/harness/src/lib.rs`、`crates/harness/src/domain_router.rs`、
`crates/harness/src/capability_filter.rs`、`crates/tools/src/capability_filter_impl.rs`、
`commands/capability.rs`、`commands/error_code.rs`、`init/state.rs`。

**前端（6 + 11 locale）**：`src/types/capability.ts`、
`src/components/settings/CapabilityDomainsSettings.tsx`（新）、
`src/components/settings/SettingsSidebar.tsx`、`src/components/settings/settingsSearchIndex.ts`、
`src/pages/SettingsPage.tsx`、`src/lib/browserMock.ts`、`src/i18n/locales/*.json`（×11，
`settings.capabilityDomains.*` 26 + 3 + `settingsSearch.capabilityDomains.*` 4 = 33 个 key/语言）。

**门禁**：`scripts/check-domain-single-source.mjs`（§5 段重写为四分支 + §5b 新增 + 5 条 selftest）。

#### 8.5.8 备份说明（⚠ 是**完成态快照**，不是改动前原件）

`output/backup-2026-09-15/p2/`（32 个文件）保存的是**改动之后**的副本。
本轮是**就地编辑**，未对每个文件先做「改动前」快照 —— 这是流程违规，记在此处不再重犯
（下一轮起：动手前先 `cp` 一份到 `output/backup-<日期>/<轮次>-pre/`）。

#### 8.5.9 收尾静态自证 + **两次自我纠错**（2026-09-15 23:0x）

`cargo check -p axagent --all-targets` 被**另一会话**的 `cargo check --workspace` 持锁，
排队 7 分钟（22:55:27 → 23:02）才拿到锁。等待期做了 3 项不依赖 cargo 的静态核查，
**其中 2 项是我自己的判据错**，一并记录。

**✅ 真结论：i18n 完全覆盖（33/33 × 11 语言）**

改用正确方向（以**消费端 `t()` 实际引用**为权威，反查语言包）后：

| 项 | 结果 |
|---|---|
| 目标 key | 33 个 = 27 个 `settings.capabilityDomains.*`（嵌套）+ 4 个 `settingsSearch` 内扁平 `"capabilityDomains.*"` + 模板型补全的 3 个 `blockReason` 枚举值 |
| 11 语言 | **33/33 全覆盖** |
| 解析模式统计 | 嵌套 319 次 / 扁平 `deepFind` 44 次 ⇒ 本项目语言包**两种形态混用** |
| 正对照 `settingsSearch.providers.apiKey` | 11/11 命中 ⇒ 判据**能**判「存在」 |
| 负对照 `…capabilityDomains.definitelyNotExist` | 11/11 判失败 ⇒ 判据**不恒真** |

**❌ 我报错的两轮（都是假阳性型）**

| 轮次 | 我报的"缺陷" | 真因 |
|---|---|---|
| ① | 「`settingsSearch.capabilityDomains.*` 4 个 key 在 11 语言全缺」 | 我按**嵌套**路径取值，而它们以**扁平字面 key** `"capabilityDomains.title"` 存在 ⇒ 恒 `undefined` |
| ② | 「`blockReason.general_is_fallback` / `system_is_internal` 是孤儿 key」 | 组件用的是**模板 key**（`CapabilityDomainsSettings.tsx:114`），动态消费；我的静态抽取器正则只覆盖反引号形式 |

第三轮换成 i18next `deepFind` 的忠实实现（每层把剩余 token 拼成整体 key）+ 正负对照后才判对。
⇒ **教训：「缺 key / 缺字段 / 缺注册」类判据，正对照必须比目标先跑** ——
否则「缺失」这个结论本身没有区分力。（已写成判据 **#236**）

**⏳ 编译/测试闭环状态**

- 已拿到锁并运行中（日志 `output/p2-cargo2.txt`）；等待期无 error。
- 期间 `axagent-analysis-engine` 的 3 个错误**由对方自行修好**（我未代改）。
- `cargo test`（`axagent-harness` 的 `domain_registry` / `domain_router`、`axagent-dao` 的
  `capability_domain_override` / `v231`、`axagent-tools` 的 `capability_filter_impl`）**尚未跑**。

**⚠️ 他人改动缺陷登记（不代改，单列告知）**

`v232_opc_domain_pack_rename` 已注册进 `MIGRATIONS`（`migrations/mod.rs` L91 + L496-498），
但 **`CURRENT_VERSION` 仍是 231**（L102）。

- **归属**：我的 Edit 要求 old_string 精确匹配 `= 230` 且成功了 ⇒ 我改时该值确为 230
  ⇒ **漏提是对方造成的，不是我覆盖的**。
- **后果被限定在状态上报，不是「迁移不执行」**：`run_migrations`（L566-596）遍历**全部**
  `MIGRATIONS`，只按 `applied.contains(&m.version)` 跳过，**不看 `CURRENT_VERSION`**
  ⇒ v232 仍会执行。受影响的是 `get_schema_status().latest_version`（报 231 而非 232），
  前端 `DatabaseSettings.tsx` 会**少报一个版本**（该字段注释自称「一旦偏小就等于 UI 说谎」）。
- **不代改的理由**：① 该迁移对方仍在写，可能重编号 ② 同文件并发 `Edit` 会互相覆盖（纪律 2）
  ③ 属他人改动（同「报告不擅改」）。

#### 8.5.10 `disallowed_types` 未豁免 —— **我自己的缺陷，已修**（同 23:0x）

**怎么在不跑 clippy 的前提下发现**：`src-tauri/Cargo.toml:96` 是 workspace 级
`disallowed_types = "deny"`，配合 `clippy.toml` 的 `disallowed-types` 五型
（`std::sync::{RwLock, Mutex, RwLockReadGuard, RwLockWriteGuard, MutexGuard}`）。
写了静态扫描 `output/_disallowed_scan.mjs`：对我改过的 **14 个** Rust 文件，列出所有
**命名**了这五型的站点，再判断每站是否落在 allow 覆盖下。

**结果：8 处站点，2 个文件漏豁免 ⇒ 若直接跑 clippy 必红。**

| 文件 | 站点 | 修法（对齐既有写法） |
|---|---|---|
| `harness/src/domain_registry.rs` | `static OVERRIDES: OnceLock<std::sync::RwLock<…>>`、`fn overrides_lock() -> &'static std::sync::RwLock<…>`、`static OVERLAY_TEST_LOCK: std::sync::Mutex<()>`、`overlay_guard()` / `overlay_case()` 返回 `MutexGuard`（共 6 处） | 生产两处用 **item 级** `#[allow]`（同 `search/src/entity_graph.rs`、`analysis-engine/src/portfolio_monitor.rs`）；`mod test_support` 用**模块 inner** `#![allow]`（同 `harness/src/test_support.rs:13`） |
| `dao/src/repo/capability_domain_override.rs` | 单测 `static TEST_LOCK: std::sync::Mutex<()>` + `fn case() -> MutexGuard`（2 处） | `mod tests` 加**模块 inner** `#![allow]` |

⚠ **顺带纠了自己一次「注释造假」**：第一版 SAFETY 注释套了 clippy.toml 的模板
「不跨 await 使用」，但用例体实际是 `let _g = case(); … .await` —— **guard 确实跨了 await**。
已改成如实三段：① 为什么用 std 版（临界区内无异步操作、读点是同步函数）
② 安全的**真实**理由（`#[tokio::test]` 默认 current-thread runtime + guard 非 `Send`
⇒ 编译器禁止该 future 跨线程移动；各用例各自阻塞在自己的 OS 线程上且互不依赖
⇒ 最坏是**串行等待**，非 UB 亦非死锁）③ **前提失效条件**（改用 `flavor = "multi_thread"`
或把用例体 spawn 出去时须重评）。

⇒ 教训写成判据 **#237**：`disallowed_types` 是 deny 级配置，新增锁之后**不必等 clippy**
就能静态预扫出漏豁免点；且**豁免注释必须如实** —— 模板化注释会把「一个真实存在的
UB 风险」包装成「已评估过的安全例外」。

> ⚠ **本节记录的是中途修法，已被终态取代**：上面那张「8 处站点 + 2 处模块级 `#![allow]`」
> 的表**不是最终形态**。终态见 **§8.5.11**：两处测试锁全部换成 `tokio::sync::Mutex`
> ⇒ 站点 8 → **3**，且 `mod test_support` / `mod tests` **一条 `#[allow]` 都不需要**。
> 当时的 `#![allow]` 之所以被撤，正是因为 clippy 随后以
> `clippy::await_holding_lock`（「this `MutexGuard` is held across an await point」）
> 反驳了那批 SAFETY 注释 —— **判决写进判据 #246：「加了 `#[allow]` 就安全」是错的**，
> 豁免只解决被豁免的那一条 lint，而根因（std guard 跨 await）会以另一条 lint 再报一次。

---

#### 8.5.11 验证闭环与**失败归属**（2026-09-15 23:2x–24:0x）

### ① 验证链 v1 三段结果（`output/p2-verify-all.txt`，107422 B / 1606 行）

| 段 | 命令 | 结果 |
|---|---|---|
| STAGE1 | `clippy -p axagent-harness -p axagent-dao -p axagent-tools --all-targets --all-features -- -D warnings` | **`CLIPPY_EXIT=101`** —— 4 条 `doc_lazy_continuation`，**全是我的**（见 ②） |
| STAGE2 | `test -p axagent-harness -p axagent-dao -p axagent-tools --no-fail-fast` | **`TEST_EXIT=101`** —— 15 条 `test result` 行；我的 **31 条全 `ok`**；4 条失败**全属他人**（见 ④） |
| STAGE3 | `check -p axagent --all-targets` | **未完成** —— 被对方同款命令饿在 `Blocking waiting for file lock on build directory`（判据 #111 现场） |

> **先数 `test result` 行数**（判据 #179）：15 行 ⇒ 确实跑了，不是「0 行 = 一条没跑」。

### ①bis 验证链 v2（修完 clippy 后重跑，`output/p2-verify-v2.txt`）

| 段 | 结果 |
|---|---|
| STAGE1 `clippy`（同上 3 crate） | ✅ **`CLIPPY_EXIT=0`** —— `Finished dev profile … in 5m 16s`，**零 error / 零 warning**。⇒ §8.5.10 的 `disallowed_types` 终态（tokio 锁，3 处 item 级 allow）+ §8.5.11② 的 `doc_lazy_continuation` 修法 + ③ 预扫出的 dao 同款，**全部被真实工具确认** |
| STAGE2 `test`（同上 3 crate） | ⚠️ **`TEST_EXIT=1` 且 `test result` 行 = 0**（日志只有 `Blocking waiting for file lock` 一行就退出）⇒ **本段证据作废**，已拆成两次等价重跑（见 ①ter）。这正是判据 #179 的现场：**`$?` 非零 + 0 条 `test result` = 什么都没跑**，不能读成「测试失败」 |
| STAGE3 `check -p axagent --all-targets` | ✅ **`CCHECK3_EXIT=0`** —— `Finished dev profile … in 12m 42s` ⇒ **编译腿闭环**（v1 被饿死的这一步已补齐） |

### ①ter 测试腿的两次等价重跑（`output/p2-test-rerun.txt` / `output/p2-test-tools.txt`）

| 命令 | 退出码 | `test result` 行 | 我的用例 |
|---|---|---|---|
| `test -p axagent-harness -p axagent-dao --lib --no-fail-fast` | `T1_EXIT=101`（**真失败码**） | **2** 行 | **31/31 `ok`** |
| `test -p axagent-tools --no-fail-fast` | **`T2_EXIT=0`** | 4 行（398+7+11+0） | **消费端③ 用例 `ok`** |

- 逐条明细：`dao --lib` **192 passed / 3 failed**（`v229_*` 三条）；`harness --lib` **703 passed / 1 failed**
  （`knowledge_graph::type_vocabulary_tests::test_csv_import_relations_are_declared`）；
  `tools` **0 failed**。失败项的归属见 ④。
- **我的 P2 用例合计 32 条，全部 `ok`**（harness 19 + dao 9 + tools 1 + 路由 3；含 4 组**对照**）。
- ⚠ **`TEST_EXIT=1` 这个码本身就是个坑**：同一条命令在 v1 是 `101`（真失败），在 v2 是 `1`（零输出、零动作）。
  ⇒ **退出码非零时必须先看 `test result` 行数**，否则会把「没跑」读成「失败」、也会把「失败」读成「没跑」。
- ⚠ 该重跑与前一次（v1）**结果逐字一致** ⇒ 说明 §8.5.11② 的 doc 注释改动**对测试面零影响**
  （这同时验证了「doc-only 改动不改行为」这一前提，不必靠信仰）。

### ② clippy 4 条真错误 —— **我引入的，消根因修掉**

`crates/harness/src/domain_registry.rs:572-575`：markdown 列表项（`:570-571`）后跟
**未缩进**的续行 ⇒ clippy 原话 `doc list item without indentation`，`-D warnings` 下算 error。

- **修法 = 断段**（列表后补一行 `///`，使后续成为独立段落），**不是加 `#[allow(clippy::doc_lazy_continuation)]`**。
  理由同 §8.5.10 的终态：lint 指出的是「排版错了」，豁免等于把错留在文件里。
- 这条 lint 是 **lint-only**：`cargo check --all-targets` 与 `cargo test` **全绿**也照样红
  （判据 #72⑤ / #201 的又一次实证，且**这次命中的是我的 doc 注释**，不是测试代码）。

### ③ ⚠ clippy 的**级联盲区**（本轮新判据 #253）

`dao/src/repo/capability_domain_override.rs:204` **有同款错误**（同一形态：列表项 + 未缩进续行），
但 STAGE1 日志里**根本没有它** —— 因为 harness 一红，clippy 就中止了：

```
error: could not compile `axagent-harness` (lib test) due to 4 previous errors
warning: build failed, waiting for other jobs to finish...
CLIPPY_EXIT=101
```

⇒ **`-p a -p b -p c` 里后续 crate 根本没被 lint。报错清单 ≠ 问题全集。**

为了在**不跑 clippy**（拿不到 target 锁）的前提下定位同款，我按 clippy 语义写了静态预扫
`output/_doc_lazy_scan.mjs`（列表状态机 + 内容列缩进判定），并**带正负对照**
（`output/_doc_lazy_ctrl.txt`）：

| 对照 | 期望 | 实测 |
|---|---|---|
| 正对照①（修复前 harness 原文） | 命中 | **命中**（与 clippy 报的同一批行） |
| 正对照②（修复前 dao 单行续写） | 命中 | **命中** |
| 负对照①（已断段） | 0 | **0** |
| 负对照②（已缩进到列表内容列） | 0 | **0** |
| 负对照③（**代码围栏**内） | 0 | **0** |

⚠ 首版扫描器在**代码围栏**上假阳性（报 `domain_router.rs:525` 的 `||| ``` `），
而**同一 crate 已被 clippy 完整 lint 且未报该行** ⇒ 用「clippy 的实际报错面」反证出这是扫描器的
假阳性（判据 #236 的同族用法：**跨文件判据须复刻消费端语义，并用真实工具的输出面校准**）。
修完复扫：`capability.rs` / `domain_registry.rs` / `domain_router.rs` /
`capability_domain_override.rs` / `init/state.rs` **全部 CLEAN**。

### ④ 4 条测试失败的**归属判定**（逐条取证，**均非我的改动**）

| 失败用例 | 位置 | 归属证据 |
|---|---|---|
| `v229_creates_table_and_unique_index`<br>`v229_unique_index_rejects_duplicate_seq_in_same_conversation`<br>`v229_same_seq_allowed_across_fleets_and_conversations`（3 条，同根因） | `dao/src/migrations/v229_create_fleet_messages.rs` | 该文件 mtime = **今天 11:35:23Z**，属 **Fleet 群聊工作流**（另一会话）。其 `fresh_db()` 只跑 `v102`（**不插父行**）就以 `fleet_id='f1'` 插子行，而 FK **实际是开的**（`dao/src/db.rs:65/559` 显式 `PRAGMA foreign_keys=ON`；`repo/reading_list_items.rs:353` 亦注释此事实）⇒ SQLite **code 787 `FOREIGN KEY constraint failed`**。对方自己的日志第十段 E) 写着「**本轮编译一行都没跑**」 |
| `test_csv_import_relations_are_declared` | `harness/src/knowledge_graph.rs`（**evidence 行号腐烂守卫**，`:1139` 处自诊断） | 该文件 mtime = **15:36:49Z**，而我的验证日志 **15:39:36Z** 才收尾 ⇒ **对方在我的验证窗口之内改了这个文件**。同族当天已是**第二次**（`MEMORY-RULES` #200 记录第一次：「knowledge_graph.rs 的 evidence 行号在 91 秒后被对方改好」） |

**我的 31 条 P2 用例全绿**，其中三条是 §9.3 定义的**消费端验收**本体：

| 用例 | 对应消费端 |
|---|---|
| `test_route_skips_disabled_domain`（`domain_router.rs`，含对照组：未停用时断言 `Finance`） | **② `route()` 不得返回停用域** |
| `test_check_all_rejects_disabled_domain`（`tools` 的 `capability_filter_impl`） | **③ 工具/能力过滤按停用域裁剪** |
| `test_disabling_domain_drops_it_from_classifier_list` + `test_l1_classifier_domain_list_covers_all_when_enabled` | **① L1 分类器 prompt 按启用集合生成** |
| `upsert_then_load_changes_runtime_behavior`（`dao`，**含对照组**：清空后内置默认下 finance 必须在清单里；并断言停用 finance **不牵连** data_analysis） | **P2-⑥①「落库 → load → 行为变」闭环** |

> ⚠ **纪律结论**：我的验证窗口与对方编辑窗口**重叠** ⇒ 全量测试的「红」**不可归因于我**，
> 但也**不能当作我的证据**。这是判据 **#175**（日志 mtime 早于改动即作废）的**镜像面**：
> **别人的在飞改动会污染我的红**。故本轮**只把「我的 31 条全绿」当作证据**，
> 4 条失败逐条取证归属后**登记告知，不代改**（同「报告不擅改」）。

### ⑤ 不争锁的静态自证（全部通过）

| 项 | 命令 | 结果 |
|---|---|---|
| 域单一真源门禁 | `scripts/check-domain-single-source.mjs` | **`RUN_EXIT=0`**；10 个副本站点全一致。**新证据**：`init/state.rs` 的 L1 分类器 prompt 已被门禁识别为**派生**站点（「调用 `l1_classifier_domain_list()`；完整性/顺序由 `domain_registry` 单测守」）⇒ P2 把这一处从「手抄副本」改成了「派生」，门禁面自行承认 |
| `disallowed_types` 豁免面 | `output/_disallowed_scan.mjs` | **3 处站点，未覆盖 0 处**；全部在 `domain_registry.rs` **生产码**（item 级 `allow` + 如实 SAFETY），两处测试锁**零 `#[allow]`** |
| 前端格式 | `node_modules/dprint/dprint.exe check` | **`EXIT=0`**（零差异）。意义：rustfmt 先前改写过 `entities/src/lib.rs` / `harness/src/lib.rs`，dprint 面**未被连带破坏** |
| 我的 P2 代码存活 | `grep l1_classifier_domain_list` / `with_llm_reasoner` | 对方 23:36:49 改过 `init/state.rs`，但我的消费端① 代码**完好**（`:1285-1293` + `:1323`） |

### ⑥ 新的判据 —— ⚠ **诚实标注：其中两条是「再发现」，不是首创**

**#253（L 组）**：clippy 一红即中止 ⇒ 同一条 `-p a -p b -p c` 命令里**后续 crate 根本没被 lint**
（日志仅以 `build failed, waiting for other jobs to finish` 收尾）。⇒ 报错清单 ≠ 问题全集，修完必重跑全量。

> ⚠ **事后核对：这一条 skill `cargo-win-lock-troubleshoot` 里早已记过**
> （其「环境故障解除后，第一跑若报 lint error —— **别以为只差这一个**」段，
> 举的例子是 `clippy::manual_option_as_slice` + `axagent-search` 之后的 crate 未被 lint）。
> 同理，v2 的 `TEST_EXIT=1`（零输出、零动作）也已被该 skill 记为
> 「锁争用下的**工具层终止**」。
> ⇒ **我把它写成「新判据」是记忆检索先于证据核对的失误**：正确做法是先读既有 skill 再判新旧。
> 数值判据仍保留（因为本条带的是**本轮的具体实证**：harness 4 条 `doc_lazy_continuation`
> ⇒ dao 同款第 204 行未被报出），但**不得据此声称这是新发现**。
> 已把本轮真正新增的取证手法（归属三问 + `101`/`1` 语义相反 + 仲裁者只能是 `test result` 行数）
> **补进该 skill 本体**，而不是另建重复 skill。

### ⑦ 附带跑到的其余门禁（**含一条红的，但不在我的清单里**）

| 门禁 | 退出码 | 结论 |
|---|---|---|
| `scripts/check-domain-semantics.mjs` | `EXIT=0`（report-only） | 注册表覆盖 **100.0%（553/553 强锚点）**、**未登记量纲 0**；4 个「一名多义」为**已登记**、待人工核对（非缺陷） |
| `scripts/check-ontology-consistency.mjs` | **`EXIT=0`** | 副本与权威源一致（含 `bottleneck-calc.rhai` / `strategy-scorer.rhai` 两处**独立**权重兜底，实测值相同）；口径分歧 2 项**待裁决**（D1 等权 vs 加权、D2 阈值 80/60） |
| `scripts/check-hardcoded-i18n.sh --strict` | **无法运行（`EXIT=127`）** | shim 的 PATH 全废，连 `bash` 都 `command not found` ⇒ **环境故障，非代码缺陷**。替代证据：本轮已用 node 脚本 + i18next `deepFind` 忠实语义 + 正负对照证明 **33/33 × 11 语言零缺失**（见 §8.5.9） |
| `scripts/check-layer-discipline.mjs`（L1 棘轮） | ❌ **`EXIT=1`** | **超基线 9 处，全部在他人文件**（`opc_capability_pack_actions.rs` / `opc_capability_pack_runtime.rs` / `opc_learning_hook.rs` / `opc_workflow_kpi_hook.rs` / `opc_workflows/seed_content_media.rs` / `stock_workflow/rhai_pm.rs` / `stock_workflow/sim_hook.rs`）；`commands-no-direct-db` 660 > 基线 656、`commands-no-sibling-call` 94 > 基线 84。**我的改动 0 处越界**（⚠「9 处」计数有误，实为 4 对，见 §8.5.12） |

> ⚠ **L1 棘轮红的处置必须是「修代码」或「经裁决抬基线」，绝不能为了变绿而调基线数字** ——
> 抬基线 = 把这条门禁的守卫面永久削掉一块（该脚本本身是 P0(semantica) 轮的产物）。
> 本轮**只登记、不代改**（同「报告不擅改」；且 9 处全在对方的在飞文件里）。
> ⚠ **该「9 处」计数是错的** —— 实测为 **4 个 (规则, 文件) 对 / 5 个站点**。详见 §8.5.12。

---

#### 8.5.12 批次 H：L1 分层门禁超基线 4 处 → 0（**真下沉，不抬基线**）（2026-09-16 01:3x 北京）

**背景**：§8.5.11 ⑦ 报 `check-layer-discipline` 红。**用户裁决：修代码**（明确拒绝抬基线）。

**⚠ 先纠我自己的错**：⑦ 表里写的「超基线 **9** 处」不成立。按脚本自身口径（规则 + 文件的**对数**）
实测为 **4 对 / 5 站点**：`commands-no-direct-db` 3 处 + `commands-no-sibling-call` 2 处。
成因：把「违规站点数」当成了「超基线条目数」—— 两者不是同一个量纲。

**三处按各自的依赖图事实采用不同修法**（判据：**移动代码以规避门禁，仅当新位置在依赖图上真的更正确时才算真修**）：

| # | 站点 | 修法 | 为什么这不是刷分 |
|---|---|---|---|
| 1 | `commands/market_sim.rs` 的 MC 内核（4 DTO + `run_mc_core` + `run_mc_preset`，236 行 / 8230 B） | 逐字搬迁到新服务层 `src/market_sim_service.rs`（`pub(crate)`），命令层留薄壳 | 该内核有 **2 个命令层之外的消费者**（`stock_workflow/sim_hook.rs`、`stock_workflow/rhai_pm.rs`）⇒ 留在 `commands/` 会把依赖图**从树退化成网**；搬到 `src/` 顶层使横向边**真的消失** |
| 2 | `stock_workflow/sim_hook.rs` 直连 `sea_orm` + `stock_analyses` 实体 | 新建 `crates/dao/src/repo/stock_analysis_snapshot.rs`（`read_/write_blackboard_snapshot`），命令层改为委托 | 实体与 `sea_query::Expr` **真的离开命令层**，且规则原文即要求「须经 dao / service」——这是规则的设计意图，不是绕过 |
| 3 | `commands/opc_workflow_kpi_hook.rs` 的 `use sea_orm::DatabaseConnection;` | 改走 `axagent_dao::db::DatabaseConnection`（同批 `opc_capability_pack_runtime.rs:11` 既有出口） | 该模块**零 SQL**（唯一用法 `DefaultAnalyticsService::new(self.db.clone())`）⇒ 它从来不是「直连」，只是拿了 sea_orm 的类型名；且**未**把它塞进 `SHARED_INFRA_MODULES` 白名单（那才是刷分） |

**修法 1 的搬迁安全取证**（搬迁前全仓 grep）：`McSimRequest` / `McScenarioSpec` / `McSimResult` /
`McScenarioResultItem` / `run_mc_core` **零外部引用**；`run_mc_preset` 恰有 2 个命令层外消费者；
`register_commands.rs:625-629` 只认命令函数；`scripts/**` 零 `market_sim` 引用。

**顺带修掉的两个真缺陷**（纯 bug，非本次任务）：
1. `commands/market_sim.rs` 的 `ErrorCategory` 搬迁后只剩 import ⇒ `-D warnings` 下是 error。
   **在编译前被静态预扫抓到**；按判据 #268 杀掉已排队 12 分钟的任务后修，再重起。
2. `sim_hook.rs` 写回只判 `Ok/Err` ⇒「影响 0 行」被打成 success 日志。改为 `Ok(0) => warn!(…)`，
   与本文件头「静默无变化比明确失败危险得多」对齐。

**验证闭环（实测）**：

| 项 | 结果 |
|---|---|
| `check-layer-discipline` | `EXIT=0`；`no-direct-db` **651 / 基线 651**、`no-sibling-call` **76 / 基线 76**；「✅ 无超基线违规」；**无「低于基线」提示** |
| 我改的 3 个文件在基线里 | **零条目**（改前它们本就是「基线之后的新越界」，清回 0 后不该有条目） |
| `--selftest` | **51 passed / 0 failed** ⇒「不报」是真实无违规，非检测器失灵 |
| `check -p axagent-dao --all-targets` | `EXIT=0`（2.11s） |
| `check -p axagent --all-targets` | `EXIT=0`（10m11s）—— `sim_hook.rs` 消费新 DAO API ⇒ 通过即证明**两 crate 接口真的对上** |
| 相邻门禁 | `check-domain-single-source` / `-semantics` / `-ontology-consistency` / `type-consistency` / `check-contracts` 全 `EXIT=0` |

**❌ 我上一轮报告的两处不准确（自行更正）**：
1. 「超基线 9 处」→ 实为 **4 对**（见上）。
2. 「`cargo check` **0 warning**」→ **不成立**：该次日志用 `tail -5` 收集，warning 行被截断；
   `Finished` 只证明**主 crate 编译成功**（无 error），**warning 数实际未知**。
   ⇒ 判据固化：**截断过的日志不能用于「0 warning」类断言**。

**clippy 4 条 error 的归属（逐条取证，均为他人）**：

`cargo clippy … -- -D warnings` `EXIT=101`，报 4 条（去重后 3 个缺陷），**全部不在我的 8 文件改动清单内**：

| # | 位置 | 实质 | mtime (UTC) |
|---|---|---|---|
| 1+2 | `commands/opc_workflows/mod.rs:57` / `:61` | 4 项 re-export 在 **lib 目标**零使用者；`DomainPackAnalysisConfig` 的 `pub use` 零使用者 | **16:30:07** |
| 3 | `commands/opc_workflows/mod.rs:57` | 同一条在 **lib test 目标**再报一次、但只剩 2 项 —— 因为 `export_domain_pack`/`import_domain_pack` **只在 `#[cfg(test)]` 内被调**（`commands/opc_workflows/mod.rs:804/811`） | 16:30:07 |
| 4 | `semantic_cache/mod.rs:303` | `Select::into_query()` 需 `QueryTrait` in scope，use 块（44-47）**漏了** | **17:00:15** |
| 5 | `semantic_cache/mod.rs:366` | `total_hits.max(0)` 歧义：该文件 43 行新引入 `ExprTrait`（blanket impl 给所有类型加 `.max()`）与 `Ord::max` 撞名 | 17:00:15 |

- **mtime 判据（决定性）**：我的改动窗口 = **16:34:17 ~ 16:48:51**；`opc_workflows/mod.rs` = 16:30:07（窗口**前** 4 分钟）、
  `semantic_cache/mod.rs` = 17:00:15（窗口**后** 11 分钟）⇒ **两者都不在我窗口内**。
- **旁证 1（对方自认）**：对方当日日志原文「`semantic_cache/mod.rs` … 17:00:15 的 mtime … 实际**那是我的 Write**」，
  并明列工作项「4 张…补实体 + 改造：… · `semantic_cache`(v100)」；该文件头亦有「## 改造记录（2026-09-16）」。
- **旁证 2（时序断点）**：`check` 日志 mtime 16:59:14 **早于** `semantic_cache` 的 17:00:15
  ⇒ check 跑时它还是**能编译的旧版** ⇒ 这解释了「check 绿 / clippy 红」。
- ⇒ **4 条全属他人**（对方 M2 域包下沉 + 声明式 schema 同步两批在飞改动），**按纪律只登记、不代改**。
  可粘贴补丁与逐条根因已写进 `output/p2-clippy-attribution-and-patch.md`。
  ⚠ 其中一条**必须提醒**：clippy 建议的 `ExprTrait::max(total_hits, 0)` **是错的**（返回 `SimpleExpr`，类型不符），
  正确修法是 `std::cmp::max(total_hits, 0)`。

**⚠ clippy 结论的有效性边界（判据 #268）**：该任务启动（约 17:00:03）后 **12 秒**，
`semantic_cache/mod.rs` 被他人改写（17:00:15）⇒ **该次 clippy 对该文件的判定基于漂移中的树**。
error **文本**可信（我已在当前树上逐行核对 `ExprTrait` 已引入、`QueryTrait` 确实缺失），但**须他人重跑才能闭环**。

**关于基线更新（对方执行，非我；结论一致）**：本轮我**刻意未跑** `--update-baseline` —— 检查前我改的 3 个文件
在基线里**零条目**，无条目可下调；跑全量会一次性确认**我无法核实的他人减量**。
对方随后跑了（`db 656→651`、`sib 84→76`），当前**实测 = 基线，棘轮重新咬合** ⇒ 分工正确、结果一致。

**本轮改动清单（未用 git 反查，逐项记录）**：6 改 + 2 新建
`src/market_sim_service.rs`（新，9593 B）· `commands/market_sim.rs`（19415→11185 B）· `lib.rs`（+`pub mod market_sim_service;`）
· `crates/dao/src/repo/stock_analysis_snapshot.rs`（新）· `crates/dao/src/repo.rs`（+`pub mod`）
· `commands/stock_workflow/sim_hook.rs` · `commands/stock_workflow/rhai_pm.rs` · `commands/opc_workflow_kpi_hook.rs`
+ 2 处**陈旧路径注释**修正：`commands/error_code.rs:336`、`stock_analysis_setup/seed_stock_analysis.rs:545`（`market_sim::run_mc_*` → `market_sim_service::run_mc_*`）
+ `stock_analysis_snapshot.rs` 的三态语义表（记录在+可解析 → `Ok(Some(对象))`；记录在+空/损坏 → `Ok(Some({}))`；记录不存在 → `Ok(None)`；查询失败 → `Err`）
备份：`output/_backup-2026-09-16-layerfix/`（5 文件回滚点）

**诚实边界（登记，勿当成已做）**：
1. **clippy 未重跑** —— 他人持锁中；且 4 条属他人，我修不掉 ⇒「全量 clippy 绿」**本轮未闭环**。
2. `check-hardcoded-i18n.sh --strict` **按用户指示本轮未运行**（shim PATH 故障仍在）。
3. 本轮只修了 `.rs` 内的两处 `market_sim::run_mc_*` 陈旧注释；**未查 `.md` 文档**里是否还有同款路径引用。

---

## 9. P0 / P1 / P2 —— 「域」从编译期枚举升级为有声明、有守卫、可治理的一层

> **起因**：2026-09-15 复核发现三个真缺口（不是「本体没包含域」）：
> ① `domain_ontology.rs` 实为 **finance 单域内容**，却自称「唯一权威源」；
> ② 域层**只有枚举、没有本体语义** —— 层级/路径/顺序/别名全硬编码散布在 10 处副本里，
>    只能靠门禁**硬编码比对**守；
> ③ **两个本体都没有 Action Types**（Palantir 口径里 Ontology 与 KG 的分水岭）
>    ⇒ 域只能是编译期常量，前端不可编辑。
>
> 三个阶段的**共同不变量**（违反即违反本文 §7）：
> **不动 9 个 id 的字面值、不删 27 条别名、`System` 不进入导航/检索、`General` 保持唯一兜底。**
> 阶段的划分依据是**风险递增**：P0 纯声明（零行为变更）→ P1 派生化（等价改写）→ P2 引入持久化。

### 9.1 P0 —— 声明化 + 门禁派生（零行为变更）

| 项 | 动作 | 判据 |
|---|---|---|
| **P0-①** | `domain_ontology.rs` 模块头**正名**为「finance 域业务本体」，并写明「域层不在这里，见 `capability.rs` / `capability_clusters.rs` / `domain_registry.rs`」；修正「本体的顶层类」单数说法（实测 `parent: None` 的顶层类有**两个**：`IndustryElement` + `BottleneckForce`） | 文档与内容覆盖范围一致（判据 #218：核「自称覆盖范围」是否等于「实际覆盖范围」） |
| **P0-②** | `crates/harness/src/domain_registry.rs` 新增 `DomainNode` + `DOMAIN_NODES` 声明：`domain: CapabilityDomain`（**存枚举变体，不存 id 字符串**）+ `nav_path` + `nav_order`；`is_business()` / `label_key()` 一律**派生**，不落字段 | 「第 N 套词汇表」的唯一防线：**id 由 `as_str()` 派生** ⇒ 无法与权威源漂移 |
| **P0-③** | `capability_clusters.rs` 测试里硬编码的 `BUSINESS_DOMAINS: [CapabilityDomain; 8]` 改为从声明派生 | 消除「新增域时测试静默少检一个域」 |
| **P0-④** | 门禁新增 `domain_registry.rs` 站点：**声明必须覆盖 `as_str()` 全部变体**（缺一个 = 硬拦）、`nav_order` 唯一且连续、`nav_path` 唯一、TS 侧 `path`/`order` 逐值比对 | 让 P0-② 的声明**有人消费**（判据 #183：零读取端的字段不写） |

**P0 的收益边界（自陈，不高估）**：新增枚举变体**不会编译报错**（`as_str` 的 match 穷尽性只管枚举自身，
管不到声明数组）⇒ 覆盖完整性由**门禁 + 单测**双保险，不是编译期强制。

### 9.2 P1 —— 别名 / 协议顺序 / labelKey 全部派生化（等价改写）

| 项 | 动作 | 风险控制 |
|---|---|---|
| **P1-①** | 27 条历史别名从 `FromStr` 的扁平 match **迁入 `DOMAIN_NODES`**（按目标域分组），`FromStr` 改为遍历声明 | **必须**补一条**行为级**单测：把 27 条别名 + 9 个规范值各自的**期望目标域**逐条列出并调用 `from_str` 断言 ⇒ 迁移漏一条即红。这是「等价改写」的唯一证据 |
| **P1-②** | 协议顺序 = `DOMAIN_NODES` 的**声明顺序**（与枚举声明顺序一致）；前端 `CAPABILITY_DOMAIN_PROTOCOL_ORDER` 不再手抄 | 门禁比对顺序（现为报告档 ⇒ 升级为**硬拦**，因为顺序不再是「可能刻意的 UI 决策」，而是声明派生） |
| **P1-③** | `labelKey` 在 Rust 侧补 `DomainNode::label_key()`（派生 `capabilityDomain.<slug>`），前端 `domainLabelKey()` 保留为 TS 侧唯一入口 | 两侧同一公式 ⇒ 门禁断言两侧公式一致（防止一侧改了命名空间另一侧不知道） |
| **P1-④** | 消除 TS 侧**手抄副本**：`CAPABILITY_DOMAIN_META` 与 `PROTOCOL_ORDER` 的 id 集合不再各自手写 | ⚠ **不引入「运行时消费生成物」** —— 实测 `schema-gen` 生成的 `src/types/generated/ipc.ts` **零运行时消费**且**未接 CI**；照搬该模式会让 `tsc` 依赖 cargo 产物（构建顺序耦合）。P1 只做「派生 + 门禁逐值比对」 |

### 9.3 P2 —— 域 Action Type（落 DB + 命令 + UI + **真实消费端**）

**为什么这一阶段风险最高**：本仓已有一次反面教材 —— `DomainRouter` 的 4 个 CRUD 方法
「有接口、有实现、**零生产消费** + 内存态」，看起来能配、实际重启即丢（§6 已只读化）。

⇒ **P2 的验收标准不是「有 UI」，而是「改完之后，某个代码路径的行为真的变了」**（判据 #152：接线点须落在公共下游）。
故 P2 必须自带**消费端清单**，且每个消费端都要能给出「改前/改后行为不同」的证据。

| 层 | 内容 |
|---|---|
| 声明 | P0 的 `DOMAIN_NODES` 为**内置默认值**；P2 的持久化数据是**覆盖层**（overlay），不修改内置默认 |
| 持久化 | 新表（域 → `enabled` / `extra_aliases`），迁移 + entity + DAO |
| 命令 | `list_capability_domain_registry`（内置 ∪ 覆盖，返回合并结果）/ `update_capability_domain`（写覆盖） |
| UI | 设置页「能力域」面板：逐域启用/停用 + 追加别名（9 个 id 只读、不可增删域） |
| **消费端** | ① **L1 分类器 prompt**（`init/state.rs` 的 `const SYS`）改为**按启用集合生成** ⇒ 停用的域从 LLM 候选里消失；② 路由 `route()` 不得返回被停用域；③ 工具/能力过滤按停用域裁剪 |

**边界（明确不做）**：
- **不开放 id 空间** —— `capability.rs` 明令「禁止引入自定义/产品线域」，且 9 个 id 是 DB 存量字符串 + 11 语言 key 的契约。
- **不复活 `DomainRouter` 的 CRUD** —— 见 §6 的防回归注释；P2 的对象是**域元数据**，不是路由规则。
- **不把域塞进 `domain_ontology.rs`** —— `page_type.rs` 已记录 2026-09-14 因此造出并列词汇表的事故。
