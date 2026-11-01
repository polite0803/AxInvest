# PLAN：域包（Domain Pack）收敛到本体域

> 状态：**已定案（2026-09-18，路径 B 修正）**，进入实施
> 方向性纠正（2026-09-18）：~~路径 A（14 个 OPC 业务域注册为全局域）~~ → **路径 B：14 个域包业务域收敛映射到 8+1 本体域**。
> 用户原话：「不是注册14个域啊，是要将该14个域收敛到8个本体域中」。
> 剩余三项决策仍有效：**多对一聚合**（一个域包只归属一个本体域，允许多个域包共享同一本体域）、能力超集处置 = **提示增加域**、**连带还「数据驱动」的债**。

---

## 1. 现状（证据链）

### 1.1 存在两套互不相认的「域」

| 概念 | 取值 | 权威/单源 | 证据 |
|---|---|---|---|
| **全局能力域** `CapabilityDomain` | `General / Devops / AiMedia / DataAnalysis / ContentCreation / Communication / Finance / Automation / System`（8 用户域 + `System`） | `harness/src/domain_registry.rs` 的 `DOMAIN_NODES`；`capability.rs` 枚举仅反序列化契约 | `harness/src/capability.rs`、`harness/src/domain_registry.rs` |
| **域包业务域**（自造，14 个） | `ai_research / software_dev / finance_invest / sales_growth / content_media / consulting / accounting / ecommerce / education / design / project_management / security / geospatial / game_dev` | `opc_capability_pack_actions.rs` 的 `get_all_domain_pack_configs()`（Rust 内联，非数据） | `src-tauri/src/commands/opc_capability_pack_actions.rs`（`get_all_domain_pack_configs`） |

**两者无字段级对应**：能力护照的 `domain` 是 `CapabilityDomain` 枚举；域包 manifest **无 `domain` 字段**（不声明归属本体域）；`roles.yaml` 不在仓库。

### 1.2 能力护照 domain 全部写死为自动化域

OPC 域包相关能力护照的 `domain` 统一归 `automation`（`opc` → `Automation` 别名），
不区分每个域包的实际业务域。域包该挂哪个本体域，目前**无处表达**。

### 1.3 能力集不封闭

- 域包需要哪些能力 = 工具 **id 白名单** 硬连线，无「域包承诺能力 == 索引实际能力」的校验。
- 能力清单 `CapabilityInventory` 来自上游 `capability_indexer`，与域包无映射。

### 1.4 域包「数据驱动」名不副实

- 域包工作流由 **14 个 Rust seed 函数** 生成（`commands/opc_workflows/seed_domain_pack_*.rs`），不在 YAML 里。
- 域包 config（roles 映射、actions、workflows 引用）全部来自 `ai_research_config()` 这类 Rust 常量。
- 结论：新增一个域包 = 新增 Rust 模块 + 重编译，违背「新增域不改核心代码」的动态诉求。

---

## 2. 目标（三条准入要求，已立项）

1. **域的定义全局一致**：域包不得自造新域，域归属唯一引用全局域注册表（`DOMAIN_NODES`）。
2. **域包形成与本体域一致**（8+1 + 未来用户自定义域）：域包只归属于 `CapabilityDomain` 子集；新增业务域先进全局注册表。
3. **能力集严格封闭**：域包「承诺能力」== 实际索引能力，不允许超集。
4. **1:1 归属**：一个域包只归属一个本体域（**允许多对一聚合**：多域包可共享同一本体域）。

---

## 3. 定案：路径 B — 14 域包收敛到 8+1 本体域

域包 manifest 增 `domain` 字段，声明其归属的 `CapabilityDomain` 本体域；能力护照 `domain`
按归属本体域登记，不再统一写死 `Automation`。

### 3.1 收敛映射表（已与用户确认）

| 归属本体域 | 域包（domain_pack_id） |
|---|---|
| **Devops** | `software_dev`、`security`、`project_management` |
| **Finance** | `finance_invest`、`accounting` |
| **DataAnalysis** | `ai_research`、`consulting`、`geospatial` |
| **AiMedia** | `design`、`game_dev` |
| **ContentCreation** | `content_media`、`sales_growth`、`ecommerce`、`education` |
| Communication | （暂无内包实例，保留） |
| General | （兜底，暂无内包实例） |
| Automation | （域包引擎本身仍属此域） |
| System | （内部域，不挂业务域包） |

### 3.2 不改枚举、不扩域轴

`CapabilityDomain` 枚举保持 8+1 不变；`DOMAIN_NODES` 不用新增 OPC 业务域节点。
「1:1 归属」以 manifest `domain` 字段声明为唯一承载，校验器保证其值 ∈ `DOMAIN_NODES` 用户域子集。

---

## 4. 能力集封闭 + 「提示增加域」机制（定案）

1. **域包侧声明能力契约**：manifest 增 `capabilities` 为**承诺集合**（`source + capability_type + 谓词`，非裸 id 白名单），同声 `domain` 归属。
2. **能力侧登记归属**：能力护照 domain 按域包归属本体域登记；护照/索引增 `domain_pack_id` 反向指针。
3. **校验器 `assert_domain_pack_capability_closed`**（seed / import / 索引变更时执行）：
   - 承诺 ⊆ 实际：**不满足则提示**，并给出缺失清单（不静默）。
   - 实际 ⊄ 承诺（出现域包未声明的能力 / 未落入任何现存本体域）：**触发「提示增加域」**——返回结构化「待注册域」提案（域 id、能力来源、示例场景），引导走「新增全局域」流程（注册 `DOMAIN_NODES` 用户域 → 域包挂靠），而非硬失败。
   - 该「提示增加域」是动态扩展口，不阻塞既有能力使用。

---

## 5. 连带：域包「数据驱动」还债（定案）

- **域包 config 数据化**：`DomainPackConfig`（domain / roles / actions / workflows）从 `ai_research_config()` 等 Rust 常量 → 读 `manifest.yaml`（+ `roles.yaml`）(+ seed 模板声明) 生成。
- **seed 去 Rust**：14 个 `seed_domain_pack_*.rs` 改为「读 manifest → 调通用生成器」，新增域包只放 YAML + 更新注册表，不动代码。
- 保持 `get_all_domain_pack_configs()` 作为兼容读入口（内部改从数据装配）。

---

## 6. 改动文件清单

| 主题 | 文件 | 说明 |
|---|---|---|
| 域名层契约 | `src-tauri/crates/harness/src/domain_registry.rs` | 开放用户域子集查询（`business_domains` 已存在；无新增节点） |
| 域包 domain 声明 | `config/opc/domain_packs/*/manifest.yaml`（14） | 各增 `domain: <CapabilityDomain>` 归属字段 |
| manifest schema | `src-tauri/crates/analysis-engine/src/opc/domain_pack.rs` | `DomainPackManifest` 增 `domain` + `capabilities`（承诺集合） |
| 能力护照 domain | `src-tauri/crates/analysis-engine/src/opc/capability.rs` | 护照按域包归属本体域登记，非写死 automation |
| 能力封闭校验 | `src-tauri/crates/harness/src/capability_indexer.rs`、`src-tauri/src/commands/capability.rs` | 承诺/实际对账 + `domain_pack_id` 反向指针 + 「提示增加域」 |
| 域包配置数据化 | `src-tauri/crates/analysis-engine/src/opc/domain_pack.rs` | roles/actions/workflows 抽数据 |
| 域包 seed 去 Rust | `src-tauri/src/commands/opc_workflows/seed_domain_pack_*.rs`（14）、`opc_capability_pack_actions.rs` | Rust 内联 → 读 manifest/注册表生成 |
| 前端命名对齐 | `src/i18n/locales/*.json`、`src/pages/opc/components/MarketPackTab.tsx` | 「市场包」→「域包」文案已改（zh-CN/zh-TW/ja），域包卡片展示归属本体域 |

---

## 7. 分期实施（执行顺序）

**期一 · 域归一 + 封闭**（先立架构）
1. 14 个 `manifest.yaml` 增 `domain` 归属；`DomainPackManifest` schema 增 `domain` + `capabilities`。
2. 能力护照按归属本体域登记（弃写死 automation）；护照/索引增 `domain_pack_id` 反向指针。
3. 校验器 + 「提示增加域」装配进 seed/import/索引路径。
4. 前端：域包卡片展示「归属本体域 / 能力承诺数」，安装触发校验提示。

**期二 · 数据驱动还债**（复用期一基座）
5. `DomainPackConfig` 数据化（读 manifest）；14 个 seed 函数改读数据生成；`get_all_domain_pack_configs()` 兼容读。

**验证门禁（每期收口）**
```
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
__TAURI_WORKSPACE__=true cargo test --workspace
npm run typecheck && npm run format
```
> 注意：`cargo check`/`test` 全绿 ≠ 合规，clippy 必须全量 `--all-targets`（见 `AGENTS.md` 门禁）。

---

## 8. 待确认（非阻塞，实施中可问）

- 期一、期二是否同仓同批合入，还是分期 PR。
- 「提示增加域」的触发面：仅 seed/import 时，还是也接索引实时事件流。