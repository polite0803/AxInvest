# OPC 能力包体系

> 状态：active ｜ 日期：2026-11-04 ｜ 对应版本：v3.0.0 ｜ 功能点：opc-capability-packs

## 定位

OPC（业务标签 `axopc`）的「域包」是**纯数据资产包**而非代码：每个包一个目录，内含 `manifest.yaml`（承诺能力契约 + 归属域）、`roles.yaml`（专家/工具白名单）、`workflows/*.yaml`（工作流模板）。它解决的是「14 个业务域的数据资产如何用同一套 schema 装载、如何归口到全局能力域、承诺能力与实际能力如何对账」这三个问题。

入口有三处：应用启动的种子化流程、OPC 命令族（导出/导入/启停/配置查询）、前端 `CapabilityPackHub`。

## 当前行为

### 1. 域归属：多对一映射与全局域注册表

全局域的唯一权威是编译期枚举 `CapabilityDomain`（9 个变体），见 `axagent-core/crates/harness/src/capability.rs:104-126`，字符串化在 `axagent-core/crates/harness/src/capability.rs:129-141`。`DOMAIN_NODES`（`axagent-core/crates/harness/src/domain_registry.rs:143-206`）在其上补 `nav_path` / `nav_order` / 历史别名，是 L1 域元数据的声明处。

域包对域的归属是**多对一**：14 个包映射到 5 个业务域，映射表硬编码在 `src-tauri/src/commands/opc_workflows/mod.rs:397-412`（`OPC_CAPABILITY_PACK_DOMAIN`），读取函数在 `src-tauri/src/commands/opc_workflows/mod.rs:449-451`。

「禁止自造域」由三层共同保证：

- manifest 的 `domain` 字段类型是 `Option<CapabilityDomain>`（`axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:71-76`），非法字符串在 serde 反序列化阶段即失败，扫描器只 `warn` 并跳过该包（`axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:177`）；
- 运行时覆盖层只允许改「启用状态」与「追加别名」，**不开放 id 空间**，不能新增/删除域（`axagent-core/crates/harness/src/domain_registry.rs:40-90`、`:281`）；
- 静态门禁 `scripts/check-domain-single-source.mjs:143-158` 把权威枚举、`DOMAIN_NODES`、TS 联合类型、前端域集合、OPC 两张映射表与 i18n 域标签列为必抓站点（`:113` 注明 9 个站点 / 17 个文件）。

（对应版本：v2.9.9；域收敛方案见 `PLAN-domain-pack-consolidation.md`，域包卡片增强证据 0963ffaac）

### 2. 能力集封闭性：承诺 vs 实际

manifest 的 `capabilities` 是「承诺集合」，每个条目描述「来源 + 能力类型 + 匹配谓词」，不是裸 id 白名单（结构见 `axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:108-123`）。谓词有三种（`axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:125-135`）：

- `All`：该来源下全部能力（整来源封闭）；
- `IdPrefix { prefix }`：id 前缀匹配；
- `IdExact { id }`：精确单 id。

对账函数 `assert_capability_pack_capability_closed`（`axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:925-1035`）产出四类结果：

- `missing`：承诺 ⊆ 实际不成立（`axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:930-958`）；
- `uncovered`：实际 ⊄ 承诺（`axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:960-988`）；对账时排除「已由本域包反指归属」与「Tool 共享来源」两类，避免全局工具护照成片制造噪音（`:973-980`）；
- `DomainRegistrationProposal`：`uncovered` 中归属域未落现存业务域的，聚合成「提示增加域」提案（`axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:990-1015`）；
- 报告由 `audit_capability_packs_capability` 装配（`axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:1098-1122`）。

**实际处置口径是「审计性质、非硬失败」**（代码注释原文见 `axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:1097`）：启动索引重建、`opc_import_capability_pack`、`ensure_opc_workflows_seeded` 三处调用只写日志/前端提示，不阻断流程；导入命令即使校验不通过也返回 `Ok(...)` 的成功信息串（`src-tauri/src/commands/opc.rs:106-155`）。因此任务描述里的「seed 期阻塞 / 增量期告警」在当前实现中**并未落地为两档处置**，仅有统一的非阻塞审计。

（对应版本：v2.9.9；能力承诺对账随域包卡片增强引入，证据 0963ffaac）

### 3. manifest 驱动

域包根目录常量 `config/opc/domain_packs`（`axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:22`），旧目录 `config/opc/industries` 仅作存量只读回退，且刻意用 `concat!` 拼接防止批量改名脚本把回退目标一起改掉（`:24-33`）。目录解析入口 `resolve_capability_packs_dir`（`:801`）。

`CapabilityPackManifest` 的关键字段（`axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:37-85`）：

- `id` / `name` / `icon` / `version` / `enabled`；
- `domain`（归属业务域，缺省 `None`）；
- `capabilities`（承诺能力集合，缺省空 = 不参与校验）；
- `template_prefix`：模板 ID 前缀；`content_media` 的模板是 `workflow-cm-*`，与包 id 不构成前缀关系，故必须显式声明（`:63-70`）；
- `analysis` / `learning` / `runtime`：四件套配置文件名，缺省分别为 `analysis.yaml`、`learning.yaml`、`runtime.yaml`（`:49-62`）。

扫描入口 `scan_capability_packs`（`:162`），解析失败的包被跳过而不是让启动失败（`:177`）。

（对应版本：v2.9.9；manifest 的 `domain` / `capabilities` 随域包卡片承诺增强进入主干，证据 0963ffaac）

### 4. 能力护照与 `domain_pack_id` 反向指针

能力护照 DTO 定义在 `axagent-core/crates/harness/src/capability.rs:1057-1080`，其反向指针字段 `domain_pack_id` 见 `axagent-core/crates/harness/src/capability.rs:1134-1139`（注释明确「仅域包立案能力填充，非域包能力为 None」）。

指针注入的唯一透传点在 `axagent-core/crates/harness/src/workflow_types.rs:2379-2385`：显式值优先；为 `None` 时按 `id` 的 `_harness_workflow` 后缀推导，保证「启动期全量重建」与「运行时增量索引」两条投影路径逐字段一致（`:2228-2231`）。

消费侧有两处：能力封闭校验用 `passport.domain_pack_id` 排除「已归属本包」的能力（`axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:977-978`）；审计装配时用「全局工具护照 ∪ 反指本包的动态护照」构造实际集合（`:1112-1119`）。

（对应版本：v2.9.9；能力渐进式披露与 `capability_load` 扩展见 `CHANGELOG.md:447`、`CHANGELOG.md:474`）

### 5. 14 个业务包到全局域的映射实况

以 `config/opc/domain_packs/*/manifest.yaml` 的 `domain:` 字段与代码映射表交叉核对（14/14 一致）：

| 全局域 | 业务包（承诺能力条数） | 合计 |
|---|---|---|
| `finance` | `accounting`（6）、`finance_invest`（6） | 2 包 / 12 条 |
| `data_analysis` | `ai_research`（6）、`geospatial`（5）、`consulting`（5） | 3 包 / 16 条 |
| `content_creation` | `ecommerce`（5）、`education`（4）、`sales_growth`（8）、`content_media`（13） | 4 包 / 30 条 |
| `ai_media` | `game_dev`（3）、`design`（4） | 2 包 / 7 条 |
| `devops` | `security`（3）、`software_dev`（5）、`project_management`（3） | 3 包 / 11 条 |

合计 14 包、76 条承诺，**全部谓词均为 `id_exact`**（没有包使用 `All` / `IdPrefix`）。证据：各 `manifest.yaml` 的 `capabilities` 段，以及 `src-tauri/src/commands/opc_workflows/mod.rs:397-412` 的映射表。

（对应版本：v3.0.0；manifest 名册数据驱动化与 `domain:` 实况，证据 351f254d9）

## 关键代码入口

- 后端命令：
  - 导出 `src-tauri/src/commands/opc.rs:90`
  - 导入 `src-tauri/src/commands/opc.rs:106`
  - 启停 `src-tauri/src/commands/opc.rs:161`
  - 配置查询 / 动作 / 学习指标三条读口（2026-11-11 #264 批 7 刀 A 起，原宿主 `opc_capability_pack_actions.rs` 里的这三条 Tauri 命令已删除，命令面由插件命名命令逐字续用原名 `opc_list_industries` / `opc_get_capability_pack_config` / `opc_list_capability_pack_actions`，实现落回 `opc.store` 接缝的宿主服务）：
    `axagent-plugins/crates/plugin-opc/src/main.rs:2499`（`list_industries`）、`:2507`（`get_capability_pack_config`）、`:2540`（`list_capability_pack_actions`）
- 引擎/业务层：
  - 封闭校验 `axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:925`
  - 审计装配 `axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:1098`
  - 护照反向指针 `axagent-core/crates/harness/src/workflow_types.rs:2381`
  - 种子化 `src-tauri/src/commands/opc_workflows/mod.rs:145`
- 前端：
  - `src/components/opc/CapabilityPackHub.tsx:34`（单包业务 Tab 入口）
  - 域聚合页 `src/pages/opc/domains/DomainTabContent.tsx`
  - `src/lib/domainMeta.ts:106-188`（域展示/导航/协议顺序的派生视图）
- 数据表/迁移：
  - 实体 `axagent-core/crates/entities/src/opc_capability_packs.rs:9`，实际表名 `opc_domain_packs`
  - 当前仓无独立 `v211_opc_domain_packs.rs` 迁移源文件；建表由 SeaORM 实体派生 DDL（对照 `axagent-infra/crates/dao/src/reconcile/expected.rs:272-289`）

## 配置与开关

- 域包根目录：`CAPABILITY_PACKS_DIR = "config/opc/domain_packs"`，旧目录回退常量 `LEGACY_CAPABILITY_PACKS_DIR`（`axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:22`、`:33`）。
- 包级开关：`manifest.enabled` + DB 表 `opc_domain_packs.enabled`；两者以 DB 为准（`src-tauri/src/commands/opc.rs:161`）。
- 包级版本：`manifest.version`，域包间互不影响，取代旧的全局 `OPC_TEMPLATE_VERSION`（`axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:11-12`）。
- 域开关：运行时覆盖层 `capability_domain_overrides` 只改「启用 / 追加别名」，不新增域（`axagent-core/crates/harness/src/domain_registry.rs:40-90`）。
- i18n：域标签 key 由 `DomainNode::label_key()` 与前端 `domainLabelKey()` 同公式派生（`scripts/check-domain-single-source.mjs:18`）。
- 静态门禁：`node scripts/check-domain-single-source.mjs`（`scripts/check-domain-single-source.mjs:106-109`）。

## 已知限制与坑

1. **`domain` 缺省回退口径不一致**：manifest 字段注释写「缺省 `None` 回退 `Automation`」（`axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:71-74`），但「提示增加域」提案的兜底实际用 `general`（`axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:999`）。写新包时不要依赖注释里的 `automation`。
2. **`domain` 的「业务域子集」未被运行时校验**：注释称由 `assert_capability_pack_capability_closed` 保证（`axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:74`），但该函数只用 `existing_user_domains` 过滤提案（`:994-996`），并不拒绝 `domain: system`。类型层面能挡住拼写错误的自造域，挡不住把系统域写成归属域。
3. **能力集封闭是审计而非门禁**：三处调用均为非阻塞（`axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:1097`），导入失败也以 `Ok` 返回提示串（`src-tauri/src/commands/opc.rs:142-155`）。因此「seed 期阻塞 / 增量期告警」两档处置目前无代码对应。
4. **表名与注释漂移**：启用开关注释写「写 DB `opc_capability_packs.enabled`」（`src-tauri/src/commands/opc.rs:161`），实体真实表名是 `opc_domain_packs`（`axagent-core/crates/entities/src/opc_capability_packs.rs:9`）。排查 SQL 时以实体为准。
5. **旧目录回退只能读不能写**：`LEGACY_CAPABILITY_PACKS_DIR` 仅用于存量读取，写路径一律走新常量（`axagent-quant/crates/company-runtime/src/opc/capability_pack.rs:26-33`）。
6. **谓词能力未被数据覆盖**：`All` / `IdPrefix` 有实现与单测，但 14 个内置包全部用 `IdExact`（见上表），实战中前缀/整来源封闭口径尚未被真实 manifest 验证。

## 相关文档

| 文档 | 状态 | 版本 | 关联 |
|---|---|---|---|
| `AUDIT-capability-inventory-2026-11-01.md` | active | v3.0.0+（未定版，待确认） | 全仓功能点清点，含 OPC 命令面与注册清单的时点快照 |
| `PLAN-domain-pack-consolidation.md` | active | v2.9.9 | 14 个域包收敛到 8+1 本体域；多对一映射与「提示增加域」的决策来源 |
| `PLAN-domain-single-source.md` | active | v2.9.9 | 域多载体收敛为一套；本文件「三层防线」与静态门禁的立项依据 |
| `PLAN-industry-to-domain-migration.md` | active | v2.9.9 | industry → domain 改名映射与执行顺序；旧 `industries` 目录只读回退即该迁移的残留 |
| `PLAN-finance-pack-naming.md` | active | v2.9.9 | 「金融投资」域包命名收编；与当前 `finance` 域下 2 包映射相关 |
| `PLAN-office-scene-domain-align.md` | active | v2.9.9 | 办公室场景 ↔ 域包 ↔ i18n 对齐门禁（`check:office-align`）的落地记录 |
| `PLAN-domain-ontology-wiring.md` | active | v2.9.7 | 业务本体接入 agent 侧；该计划明确「不把能力域并进本体」，与当前分层一致 |
| `PLAN-axinvest-opc-business.md` | active | v2.8.7 | OPC 总体实施（行业 = 数据资产包）；**该计划按「9 大垂直行业」立论，与当前 14 包 → 5 域的收敛实现不一致** |
| `PLAN-axopc-merge.md` | active | v2.8.7 | AxOPC 合并索引；同上，属收敛前的历史方案 |

## 版本变更记录

| 版本 | 日期 | 该功能点在本版本改了什么 | 证据（提交短哈希或 CHANGELOG 版本节） |
|---|---|---|---|
| v2.9.4 | 2026-08-12 | OPC 需求发现能力集扫描、多平台线索与能力缺口管理起步 | `e8ad32161` |
| v2.9.4 | 2026-08-14 | 移除能力匹配器并重构需求发现命令；OPC 工作流移除行业适配器，迁到 domain/industry 分层 | `e023daa36`；`CHANGELOG.md:992-993` |
| v2.9.6 | 2026-08-15 | 需求发现新增智能体能力扫描与工作流业务域推断 | `08175e8d3`；`CHANGELOG.md:733` |
| v2.9.9 | 2026-09-02 | 恢复 14 个 OPC crate 业务主干 | `124c4ed7a`；`CHANGELOG.md:456` |
| v2.9.9 | 2026-09-18 | 域包卡片「启用/停用 + 本体域 + 能力承诺」增强；B/C 端与地理标注 | `0963ffaac`、`f6fe4607b`；`CHANGELOG.md:479` |
| v3.0.0 | 2026-09-26 | auto-provision 阶段 1/2：域包专家名册查询面、建房即成队 | `CHANGELOG.md:143-144` |
| v3.0.0 | 2026-09-26 | 域包办公室数据驱动化（manifest 名册覆盖 + `office_scene.yaml` + 门禁 e 判据） | `351f254d9`；`CHANGELOG.md:145` |
| v3.0.0 | 2026-10-31 | 版本基线确立（本文件所述实现即此版本状态） | `CHANGELOG.md:133` |
| unreleased | 2026-11-01 | `src-tauri/crates` 拆为 7 个 workspace 根，能力包路径随迁（无行为变更） | `8dfaeb4db` |
