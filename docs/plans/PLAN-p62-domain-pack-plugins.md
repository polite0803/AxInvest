# PLAN — P6.2 按域包拆分独立插件（2026-11-18）

> 状态：active ｜ 日期：2026-11-18 ｜ 对应版本：v3.0.1
> 关联：`PLAN-shell-plus-plugins.md` §0.10.5、§6.1、§180，`AUDIT-p62-start-baseline-2026-11-18.md`，`PLAN-domain-pack-consolidation.md`，`opc-capability-packs.md`
> 性质：执行计划。用户裁定：OPC 不再作为“所有领域塞进同一个插件”的单个 L1 包；按项目中已有领域划分，每个领域成为独立插件。

---

## 一、裁定与边界

### 1.1 用户裁定

P6.2 的可插单元从 `plugin-opc` 一个大包改为**按域包逐包插件化**。判据不是“能编译成一个 worker”，而是“一个用户可独立启停的业务领域 = 一个独立插件产物”。

### 1.2 权威领域清单

领域边界取仓内已存在的 `config/opc/domain_packs/` 与 `get_all_configs()` 两者交集，共 **14 个域包**：

`accounting` · `ai_research` · `consulting` · `content_media` · `design` · `ecommerce` · `education` · `finance_invest` · `game_dev` · `geospatial` · `project_management` · `sales_growth` · `security` · `software_dev`

这 14 个是业务域包的权威清单。全局 `CapabilityDomain` 的 8+1 分类轴只用于能力护照归域，不拿来做插件粒度：例如 `finance` 粗域下仍有 `accounting` 与 `finance_invest` 两个独立业务包，不能合并成一个插件，否则违背用户裁定。

### 1.3 不是插件范围的资产

下列资产保持公共底座，不随某个域包下沉：

1. `opc.store` 接缝、宿主 `OpcStoreProvider` 与共享 SQLite 表；数据主权仍在宿主。
2. 域包通用 CRUD：博客、客户、发票、项目、落地页、联系人、KPI、需求发现等跨包共享命令；它们作为 OPC 公共能力面由公共 worker 承接，不重复复制到 14 个插件。
3. 宿主 `CapabilityDomain` 枚举、`domain_registry`、`NAV_ITEM_DOMAIN_MAP` 与 8+1 能力分类轴；它们是协议与导航标准，不按插件拆。
4. 域包数据资产（manifest / roles / workflows / experts）；它们仍是数据资产包，插件化的是“用户可独立启停的领域能力”，不是把 YAML 变成 Rust crate。

## 二、目标形态

每个域包插件至少包含：

- 独立 `plugin.json`：`id` 取 `plugin-opc-<domain_pack_id>`，协议版本、命令清单、依赖接缝按该域包实际声明裁剪；
- 独立 worker 产物：只声明该域包需要的 `opc.store` op 与命令；不声明其他域包 op；
- 独立前端页面资产：该域包的路由页与必要组件；
- 独立领域声明：manifest / roles / workflows / experts 随该域包插件发布；
- 独立启停语义：禁用某域包插件后，该域包页面与命令消失，公共 OPC 与其余域包不受影响。

公共 `plugin-opc-common` 保留跨包共享的命令与接缝；它不是“大盘插件”，只承接不归属单一域包的能力。命名与边界在第一批落地时按实际命令清单再钉死。

## 三、为什么先改计划而不直接搬代码

当前 `audit-l1-blockers.mjs` 的 OPC 边界读数已为 0/0，说明旧口径无法告诉我们应该按什么边界继续拆。若继续沿用单一 `plugin-opc` 计数，14 个业务包会被一个插件吞掉；若只用 8+1 能力域，又会把 14 个业务包压缩成 5 个插件。两者都不符合用户裁定。

因此第一批必须先建立**域包插件边界判据**：每个域包能独立列出命令、页面、数据资产、接缝依赖与启停影响；没有这个底册，后续任何 bundle/CLI 改动都无法验收。

## 四、批次序列

### 批 0：领域底册与门（本次启动）

- 建立 14 域包清单的机器可读底册，字段至少包含：`domain_pack_id`、显示名、归属 `CapabilityDomain`、页面 id/path、命令集合、op 集合、资产目录。
- 门禁校验三项：14 个 id 与两处权威源码一致；每个 id 的页面与命令归属不串包；公共能力面不与域包能力面重复声明。
- 在 PLAN 与审计文档中登记“P6.2 粒度 = 域包插件”，防止后续再把 OPC 整体当成一个插件。

### 批 1：单域包垂直切片

选择一个域包做首个插件，优先条件：资产完整、页面简单、命令与他人耦合少。建议从 `accounting` 或 `security` 中二选一，落地前用批 0 底册按耦合度排序确认。

验收：

1. 该包 worker 独立产物存在；
2. 该包页面由插件资产提供或被明确登记为仍依赖宿主上下文的例外；
3. 禁用该插件后，仅该包页面/命令不可用，公共 OPC 与其他 13 包不受影响；
4. 升级前后 `opc_*` 数据保留。

### 批 2..14：逐包复制模板

每批只迁一个域包；每批必须更新底册与门禁，不允许一次迁多包后事后归因。

### 终局批：公共底座收口与独立仓

公共 worker、域包插件模板、bundle 脚本与 SDK 全部可在独立仓运行；跨域包依赖通过 `call_seam` 显式声明；宿主不再静态引用任何域包实现。

## 五、与既有判据的关系

- P6.2 的“业务迁移面归零”仍然有效；它证明宿主反向阻塞与跨包口不是当前问题。
- P6.2 的新增完成判据改为：14 个域包均有独立插件边界与启停验证；公共 OPC 能力单独成包；任一域包停用不拖垮其余域包。
- 旧 `plugin-opc` 作为大包不再是 P6.2 终局形态，但可作为过渡期的公共 worker 载体；最终拆分时按底册逐项迁移。

## 六、执行纪律

- 不改 `CapabilityDomain` 枚举。8+1 是能力分类轴，域包是用户可插单元，两者多对一映射。
- 不把 OPC 公共 CRUD 复制进 14 个插件；共享能力只能留在公共 worker 或经 `call_seam` 调用。
- 不把 14 个域包再合并成 5 个粗域插件；用户明确要求“每一个领域一个独立插件”。
- 不跑全量门禁；阶段内只用定点脚本与 `cargo check -p <crate>` / 定点 clippy。
- 形态级红灯当场上报；底册或门禁出现“域包串包”时不得继续 bundle。
- 所有新文档放 `docs/`，跨文档引用用纯文件名；本文件已登记 `docs/INDEX.md`。


## 七、实施结果（2026-11-18）

### 7.1 已落地的 14 域插件边界

底册 `opc-domain-pack-plugins.json` 的 14 条 `verticalSlices` 与盘上插件产物一一对应：

- 独立 worker：accounting（`opc-accounting`）、sales_growth（`opc-sales-growth`）、content_media（`opc-content-media`）。
- 声明式独立插件：ai_research、consulting、design、ecommerce、education、finance_invest、game_dev、geospatial、project_management、security、software_dev。这 11 域的 manifest 能力均为宿主共享 Tool / 公共 OPC 命令，按“共享能力只归公共底座”的裁定不复制 worker 命令；插件贡献独立逻辑清单与页面资产。
- 公共 `plugin-opc` 的 `domainPacks` 已归零，仅保留跨包共享命名命令与 `opc.store` 接缝；已迁命令从源清单删除，由构建脚本门禁阻止公共包与域包双 owner。

### 7.2 门禁与验证

- `check-opc-domain-pack-plugins.mjs` / `check-opc-domain-pack-manifest.mjs`：14 域清单一致，独立 owner 合法，公共包与域包命令不交叉；selftest 覆盖 14 独立 + 0 公共计数、错误 owner 拦截、重复命令检测。
- `build-plugin-bundle.mjs --check`：13 个域插件产物加公共包全部通过，页面入口白名单、worker 握手/停机、sha256 一致；声明式插件明确跳过 worker 帧握手。
- `PluginManager` 按命令名解析唯一启用 owner：0 owner 报 NotFound、多 owner 报冲突、disabled 不算命中；域包禁用投影只影响该插件。
- 数据保留：本批未改任何 `opc_*` entity / migration / schema 文件，升级前后表结构同源，数据不因插件归属变化而迁移或删除；真机升级后的数据重载仍列入独立端到端验收。

### 7.3 独立仓验收进展

- **真 bundle 启停端到端已验收**：三个独立 worker 域（accounting / sales_growth / content_media）覆盖命令安装 → 启用 → 执行 → 禁用 → 命令链不可用且域包投影消失 → 恢复启用 → 再执行；声明式插件以 security（零命令、零 worker）为代表完成页面资产与域包投影的启停端到端。CI 会在 `cargo nextest` 前串行 staging 这四个 bundle，避免空跑 SKIP 假绿。

- **2026-11-18 已建立可复跑验收链**：`scripts/verify-opc-plugin-sdk.mjs` 会拷出 OPC 插件 SDK 最小独立树（axagent-plugins + axagent-core 根配置 + harness/plugin-proto），在其中完成 `cargo metadata` / `cargo check` / `cargo clippy` / `cargo test` 全绿；`scripts/check-workspace-boundary.mjs` 把插件 workspace 对外部 workspace 的 path 依赖量化为 11 条并按基线只减不增锁定。两道门均已接入 `scripts/ci-check.mjs` 与 `.github/workflows/ci.yml`。
- **持续验证**：OPC 六个 crate（plugin-opc-core / accounting / sales-growth / content-media / plugin-opc / plugins）在独立树中 `check / clippy / test` 全部通过；`--full` 模式另验全插件树 `cargo check --workspace`。域包底册已迁入 SDK workspace（`axagent-plugins/scripts/opc-domain-pack-plugins.json`），独立树内 `build-plugin-bundle.mjs --plugin opc-accounting/opc-security --check` 均通过，bundle 生成链不再假设主仓布局。
- **剩余 debt**：`workspace-boundary-allowlist.json` 当前登记 11 条跨根 path 依赖，核心缺口仍是 plugin-stock 对 `analysis-engine` / `astock-data` 的业务实现依赖，以及 `plugins` / `schema-gen` 对 harness 的实现依赖。终局仍需把 SDK crate 发布成可解析版本，或提供私有 registry。

### 7.4 数据保留实测

- **升级收敛数据保留已验收**：`axagent-infra/crates/dao/tests/opc_data_preservation.rs` 用 `opc_invoices` 的交付 + 客户账单两种投影形态插入真实行，然后重跑生产同源的声明式收敛引擎（`bootstrap_schema`），断言行数不变、逐字段保留（lead_id / customer_id / invoice_number / total / line_items_json）。手工 migration 清单为空，这一步就是「升级」的建表/加列/索引收敛面；真机升级前的数据逐表对比仍列于 7.5。

### 7.5 尚未闭合

- 物理拆仓（把插件 workspace 独立成 git 仓库）尚未执行；当前证据是“可拷出独立树构建”而非“仓库已独立”。
- 真机禁用/启用插件后的页面端到端点击链路与升级前数据逐表对比尚未做；当前证据是插件边界门禁、禁用投影单测与 schema 未变证明。
