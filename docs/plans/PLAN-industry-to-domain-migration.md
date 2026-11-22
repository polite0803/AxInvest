# industry → domain 迁移方案（执行前定稿）

> 状态：active ｜ 日期：2026-10-11 ｜ 对应版本：v2.9.9


> 目标：取消「行业 / industry」这一概念层，全量统一到 **CapabilityDomain（域）**。
> 本文只做一件事：把改名映射和执行顺序钉死，避免 6700+ 处改动返工。

---

## 一、现状：三套「域 / 行业」并存（实测）

| 概念 | 位置 | 活性 | 规模 |
|---|---|---|---|
| **CapabilityDomain**（能力域，8+1） | `src/lib/domainMeta.ts`、侧栏分组、`DomainHubPage.tsx` | **活** | 8 个业务域 + System |
| **industry pack**（行业包） | `config/opc/industries/`、`agency_experts/opc/industries/`、`opc_setup/industry_{experts,agents}.rs`、前端 `IndustryHub`、i18n `opc.industry.*` | **活（主干）** | 14 包；i18n **608 key × 11 语言** |
| **domain pack**（域包） | `config/opc/domains/`(17 manifest)、`agency_experts/opc/domains/`(19)、`opc_setup/domain_{experts,agents}.rs` | **半死**：experts/agents seed 活；**`config/opc/domains/*/manifest.yaml` 零读取端**（`resolve_domains_dir` 只存在于注释，函数不存在） | 17 / 19 |

**结论**：上次「域改造」是**加了一套新轨（domain）却没删旧轨（industry）**，且新增的 `config/opc/domains/` 配置包无人读取。前端只把「域」做成了侧栏分组标题。

### ⚠ 2026-09-15 实施时的两点实测修正（推翻本方案初稿的两条前提）

| 初稿判断 | 实测结果 | 影响 |
|---|---|---|
| 「`domains/` 与 `industries/` 同 schema 同语义，可合并」 | **不是同层**：`config/opc/domains/`（17 × 仅 `manifest.yaml`）= **职能域**（academic/engineering/marketing…，对应 `OPC_WF_SEGMENT_DOMAIN` 的段）；`config/opc/industries/`（14 × 4 yaml）= **业务包**（accounting/software_dev…，对应 `OPC_INDUSTRY_DOMAIN` 的 key） | **不合并**。新目录独立命名为 `config/opc/domain_packs/`（否则 31 个不同层级的包会挤进一个目录） |
| 「`opc_setup` 两轨 seed 是**重复劳动**，应去重」 | **不是重复，是并集**：`industry_experts` 41 个 role id ∪ `domain_experts` 76 个 = 112 个不同专家；**交集仅 5 个**（`design-system-architect`、`sales-lead-generator`、`security-{compliance-auditor,incident-responder,risk-assessor}`）。删任一条轨都会**丢 36 / 71 个专家** | **两条轨都保留**，只做词汇统一（`industry` → `domain_pack`）。真正的死轨是 `config/opc/domains/` 那 17 个零读取端 manifest |
| 「`agency_experts/opc/{design,security}` 同名 ⇒ 互为副本，合并即可」 | **跨目录同体 md = 0 组**，两侧是**两套不同内容**；且 `security-compliance-auditor.md` **同名异体**（2902B vs 3444B） | **不合并**。异体同名文件保留在各自目录（合并会丢内容或让同一 role id 有两份定义） |


---

## 二、命名对照表（迁移主表）

| 现名 | 新名（**实施后实际落地**） | 说明 |
|---|---|---|
| industry pack / 行业包 | **domain pack / 域包** | 概念统一的核心 |
| `config/opc/industries/` | `config/opc/domain_packs/` | **独立新目录，不并入 `config/opc/domains/`**（两者不同层，见 §一 修正表） |
| `agency_experts/opc/industries/` | `agency_experts/opc/domain_packs/` | 同上，不并入 `domains/`（跨目录同体 = 0 组） |
| DB 列 `industry_id` | `domain_id` | `opc_kpi_records` / `opc_rl_experience` / `opc_rl_training_stats`（**未实施，见 §六 批次④**） |
| `IndustryHub`（组件） | **`DomainPackHub`** | ✅ 已落地；避开既有 `DomainHubPage`（8 域总览页） |
| `/opc/industry/:id` | `/opc/domain/:id` | ✅ 已落地（批次②，含旧链 301 重定向） |
| `INDUSTRY_ROUTES` | **`DOMAIN_ROUTES`**（非初稿的 `DOMAIN_PACK_ROUTES`） | ✅ 已落地；因变量语义是「域包路由表」，`DOMAIN_ROUTES` 亦无歧义 |
| i18n `opc.industry.*`（608 key） | `opc.domain.*` | ✅ 已落地（批次①，×11 语言） |
| i18n `opc.industries.*` | `opc.domains.*` | ✅ 已落地 |
| `opc.nav.industries`「行业导航」 | `opc.nav.domains`「域导航」 | ✅ 已落地 |
| `opc_setup/industry_{experts,agents}.rs` | → `domain_pack_{experts,agents}.rs` | ✅ 已落地；**两轨并存**（并集，非重复） |
| `opc_industry_actions.rs` 等 6 个后端模块 | `opc_domain_pack_*.rs` | ✅ 已落地（批次③） |
| `nav.automationConsulting`「行业咨询」 | 「咨询顾问」 | ✅ 已落地（批次①） |
| `.opcip` 包格式 | **保留不改** | 改名会废掉已安装/已分发存量包 |
| pack id `industry_consulting` | **保留不改** | 见 §六 批次④（牵动 DB 存量值、URL、模板 id） |

---

## 三、`domain` 一词的冲突消歧（必读，否则四个「域」打架）

| 名称 | 含义 | 处置 |
|---|---|---|
| `CapabilityDomain`（8+1 枚举） | 能力分类轴 | **保持不动**（用户明示：目前 8+1，将来才开放用户自定义） |
| `DomainHubPage.tsx` | 8 域聚合总览页 | **保持不动** |
| `DomainPackHub`（新） | 单个域包的详情 Hub | 新名，替代 `IndustryHub` |
| 新增「域包」数据层 | `config/opc/domains/<id>/` | 承接原 `industries/`，并为「用户自定义域」打底 |

---

## 四、重名合并 —— ❌ 实测**不存在需要裁决的重名**

初稿假设 `domains/` 与 `industries/` 是同层同义的两套包，故列出 5 组「重名」。实测（2026-09-15）否定该前提：

| 目录对 | 交集 | 实测关系 |
|---|---|---|
| `config/opc/{industries,domains}` | `design`、`security`（2 组） | 同名但**内容不同**：`industries/design/manifest.yaml` 184B vs `domains/design/manifest.yaml` 228B（**异体**）。其余 12 vs 15 项**互不重名** |
| `agency_experts/opc/{industries,domains}` | `design`、`security`（2 组） | **跨目录同体 md = 0 组**。`security-compliance-auditor.md` 两侧 2902B vs 3444B（**同名异体**）；`design-system-architect.md` 3289B vs 4315B（异体） |

**处置**：**不做任何合并**。两侧各自独立改名为 `domain_packs/`（业务包）与保留 `domains/`（职能域 / 工作流模板段）。
「同名异体」不是冲突 —— 它们是两个层级上恰好同名的不同资产（见 §一 修正表）。

⚠ 若将来仍要合并，**必须先按值比对**（本仓判据：合并副本前须按值比对），
且 `security-compliance-auditor` 这类同名异体是「一名两体」，合并会让同一 role id 有两份定义，谁后 seed 谁生效。

---

## 五、后端模块改名清单（含 industry 出现次数）

| 文件 | 次数 |
|---|---|
| `commands/opc_industry_actions.rs` | 494 |
| `commands/opc_industry_bridge.rs` | 208 |
| `commands/opc_industry_runtime.rs` | 139 |
| `commands/opc_workflows/mod.rs` | 137 |
| `commands/opc_workflows/industry_pack.rs` | 73 |
| `commands/opc_learning_hook.rs` | 62 |
| `register_commands.rs` | 50（Tauri 命令名 = IPC 契约，须与前端 `invoke()` 同步） |
| `commands/opc_workflow_kpi_hook.rs` | 45 |
| `commands/opc_setup/industry_agents.rs` / `industry_experts.rs` | 64 / — |
| `commands/opc_workflows/seed_industry_*.rs` | 42+ |
| `crates/orchestrator/src/industry_learning.rs` | 113 |
| `crates/orchestrator/src/task_context.rs` | 98 |
| `crates/orchestrator/src/token_budget.rs` | 86 |
| `crates/analysis-engine/src/opc/*.rs` | 99+46+45+44+21+16+14+10 |
| `crates/dao/src/migrations/v230_opc_kpi_industry.rs` + `rl_experience_store.rs` + `repo/rl_experience.rs` | 18/16/15 |

---

## 六、执行批次（每批可独立验证、可回滚）

| 批 | 内容 | 规模 | 状态 | 验证 |
|---|---|---|---|---|
| **①** | i18n 命名空间迁移 + 文案去「行业」 | key 段 6 个/语言 × 11（`opc.industry` 一段改完全部 602 叶子跟随） | ✅ **已落地** | `check:i18n --strict` 9686 key 全通过；zh-CN 值改写 28/28；前端消费端 461 处 |
| **②** | 前端实体/路由/store 改名 + 旧链重定向 | 8 项重命名 + 20 文件内容 + 14 条路由 | ✅ **已落地** | tsc 0 / dprint 0 / vitest 1781（1 抖动复跑通过）/ 新增重定向链路测试 2/2 |
| **③** | 后端模块改名 + 配置目录合并 + 双轨 seed 去重 | ~25 文件 + 14/17/12/19 目录 | ⏳ 待开工 | `cargo check`（攒批，极慢） |
| **④** | DB 列迁移 `industry_id` → `domain_id` | 3 表 + 83 文件 | ⏳ 待开工 | 迁移前后行数比对；**先 `pg_dump` 备份** |
| **⑤** | 全量验证 | — | ⏳ | tsc / dprint / i18n 三检 / vitest 全量 / clippy zero-warning |

### 批次② 的契约分层（重要，③ 批的输入）

本批**只改「实体身份」**（文件名 / 目录名 / 组件名 / 类型名 / 路由路径 / 查询参数名 / store 名），
**刻意保留**下列「数据与 IPC 契约」，必须与 Rust 侧**同批**改，否则静默断链：

| 保留项 | 数量 | 为什么不能在前端单独改 |
|---|---|---|
| `industryId` | 175 处 | 是 `invoke()` 第二参数的 **key**，对应 Rust `#[tauri::command] fn(industry_id)`；前端单改则后端收不到参数 |
| `industry_id` / `industry_name` / `industry_count` | 12 处 | serde DTO 字段（`#[serde(rename_all="camelCase")]` 的下划线侧）。**类型定义会被一起改 ⇒ tsc 沉默自欺**（见判据 #219） |
| `opc_get_industry_pack` 等 7 个命令名 | 10 处 | IPC 命令名，Rust `generate_handler!` 注册 |
| `industry-consulting`（pack id） | 18 处 | 牵动 `config/opc/industries/<id>/` 目录、DB 值、`BUILTIN_PAGE_PATH` key |
| `industry_consulting_harness_workflow` | 1 处 | 工作流模板 id，已落库 |

**红线**：股票业务的 `industry`（`stockAnalysis.industry`「所属行业」、`industryRanking`「行业排名」、`concept_index.rs` 概念板块、`astock-data` / `analysis-engine` 的行业数据、`sector-analyst` agents、`get-industry-ranking`、`industry-chain`、`stock-industry-*`）**一律不动** —— 全项目 `industry` 命中 261 文件 / 4635 处，其中绝大多数属于此类。

---

## 七、待确认（已裁决）

1. **新词** —— 已裁决：统一到 `CapabilityDomain`，不发明 `pack` 这类新词。落地形态为 `Domain*` 前缀 + `/opc/domain/:packId`。
2. **§四 的 5 组重名**（`gis`/`geospatial`、`pm`/`project_management`、`gamedev`/`game_dev`、`sales`/`sales_growth`、`design`+`security` 同名各一份）—— 待 ③ 批执行时逐组出 diff。
3. **`.opcip` 包格式** —— 保留原名（否则存量包全废）。

---

## 八、未解决问题（不在本次改名范围，需另行裁决）

改名**不等于**「域成为唯一组织轴」。改造完成后侧栏仍是 **18 个平级入口**
（1 工作台 + 1 AxInvest + 16 域包），这与「域只有 8+1 个」的概念模型仍有落差。
真问题在结构：`pageRegistry.ts` 里 16 个域包各占一个平级 nav key（`finance-analysis`、`automation-sales`… 前缀已是域名），
但侧栏把它们平铺而非收进 8 个域入口。**是否要收敛为「侧栏 8 域 → 域页内列出域包」** —— 待裁决，本批不动。

---

## 九、实施记录（2026-09-15）

### 9.1 进度

| 批 | 内容 | 状态 |
|---|---|---|
| ① | i18n 命名空间（6 键段 × 11 语言，`opc.industry.*` → `opc.domain.*`）+ 28 条值改写 | ✅ |
| ② | 前端实体与路由（8 项重命名、16 条路由、旧链重定向、`?industry=` → `?domain=`） | ✅ |
| ③ | 后端模块 + 配置目录 + Rust 内部命名 | ✅ |
| ④ | **契约层**：Tauri 命令名 / IPC 参数 `industryId` / serde 字段 / DB 列 `industry_id` / pack id | ✅ 已落地（见 9.8） |
| ⑤ | 全量门禁（cargo clippy / vitest / i18n 三检） | ⏳ 进行中（见 9.8.7） |

### 9.2 批次③ 实际改动

- **重命名 30 项**（零删除）：目录 5（`config/opc/industries`→`domain_packs`、`agency_experts/opc/industries`→`domain_packs`、`orchestrator/src/industry_adapters`→`domain_pack_adapters`、`harness/src/industry_orchestration`→`domain_pack_orchestration`、`orchestrator/configs/industry_learning`→`domain_pack_learning`）；文件 25（4 个 `opc_industry_*` → `opc_domain_pack_*`、`industry_pack.rs`→`domain_pack.rs`、`opc_setup/industry_{agents,experts}.rs`、14 个 `seed_industry_*.rs`、`orchestrator/industry_learning.rs`、`analysis-engine/opc/industry_{config,kpi_service,validator}.rs`）
- **内容替换**：69 文件 / 220 处规则命中
- **存量兼容**：新增 `LEGACY_DOMAIN_PACKS_DIR` + `resolve_domain_packs_dir` 旧目录回退（升级后旧路径下已安装的 `.opcip` 包不会静默消失）。⚠ 该常量值**刻意用 `concat!` 拼接**，避免被更名脚本反噬
- **契约层刻意保留**（220 处规则**不覆盖**）：`industry_id`(716) / `industry_name`(55) / `industryId`(19) / `industry_count` / `canonical_industry_id` / 20+ 个 `opc_*_industry_*` Tauri 命令名 / pack id `industry_consulting` / 模板 id `industry_consulting_harness_workflow`

### 9.3 批次④ 待做（契约层，需前后端 + DB 同批）

> **2026-09-15 实测**（`output/tmp-b4-survey.mjs` / `tmp-b4-split.mjs`，扫描 1574 个 ts/tsx/rs/sql 文件）。
> 原稿的「20+ / 175 处」是粗估，下表是逐 token 实测值。

| # | 对象 | 实测规模 | 关键风险 |
|---|---|---|---|
| 1 | **Tauri 命令名** | **15 个**名字 / 全仓 **54 个引用点** | `register_commands.rs` 是 `build.rs` **生成物（勿手改）**；前端多数调用点经 `src/pages/opc/domains/useDomainData.ts` 包装；还牵动 `build_opc_industry_tool_defs` 生成的 LLM tool 名 |
| 2 | **IPC 参数 / DTO 字段** `industryId` | **178 处**（OPC **169** / 待判 9 = `src/types/index.ts` 9 个 `industryId: string;` 接口字段） | 前后端必须**同批**改；`industryId` 的 Rust 端是 Tauri 命令参数（默认 camelCase 映射） |
| 3 | **serde DTO 字段** `industry_id` | **405 处**（OPC **402** / `browserMock.ts` 3） | ⚠ 类型定义会被一起改 ⇒ **`tsc` 完全不报**，必须靠逐处 grep 复核（判据 #219） |
| 4 | **DB 列** `industry_id` | 迁移文件 3 个：`v215_opc_rl_experience`（+索引 `idx_rl_experiences_industry`）、`v230_opc_kpi_industry`、`mod.rs` | 新迁移 `RENAME COLUMN`；**先 `pg_dump` 备份 + 迁移前后行数比对**；已执行过的迁移**不改名**（见 #6） |
| 5 | **pack id** `industry_consulting` | 牵动 config 目录名、`OPC_DOMAIN_PACK_DOMAIN` 表、`workflow_templates.route_path` 存量值、模板 id | 存量数据迁移，不是纯改名 |
| 6 | 迁移文件名 / 索引名 | `v230_opc_kpi_industry.rs`、`idx_rl_experiences_industry` | 已执行过的迁移 ⇒ **建议保留不改**（改名会让「已执行/未执行」判定漂移） |

**明确的股票业务红线（④ 批一律不动，实测均为独立符号）**：
`get_industry_ranking` / `stock_get_industry_ranking` / `industryRanking`、
`industryName`(9) / `industry_name`(28)（`IndustryRankingPanel.tsx`、`SectorHeatmapPanel.tsx`）、
`concept_index.rs`（`industry_semiconductor` / `industry_bank` / `industry_insurance`）、
`industry_chain.rs`（`list_industry_chains`）、`v200_axinvest_stock_tables` 的 `industry` 列、
`astock-data` / `screener` / `mcp_tools` 的 `compute_industry_position` 等。

**④ 批的执行建议（按可回滚性排序）**：1 → 2 → 3 一批（纯代码，`tsc`+`cargo check` 可验），
再单独做 4（DB，必须先备份），5 单独做（存量数据迁移，一旦出错影响模板路由）。

### 9.4 本批实施中踩到并修掉的 4 个自身缺陷（记录以防重犯）

| # | 缺陷 | 为什么危险 | 修法 |
|---|---|---|---|
| 1 | 规则边界用 `\b`，而 token 以 `_` 结尾（`seed_industry_`）⇒ **`\b` 永不成立**，整条规则恒不命中 | 14 个 `mod seed_industry_*;` 声明**一条都没改**，而 dry-run 报「无残留」—— 静默通过 | 末尾非 `[A-Za-z0-9]` 时不加后置边界 |
| 2 | 同类边界 bug 另一面：token 以 `/` 结尾（`src/pages/opc/industries/`）同样恒不命中 | 过期注释一条未改，且**任何**以 `/` 结尾的规则都失效 | 同上 |
| 3 | **规则反噬**：`LEGACY_DOMAIN_PACKS_DIR = "config/opc/industries"` 被自己的规则改成新目录 | 存量回退**静默退化为指向同一目录**（`candidate.is_dir()` 已返回，回退分支永不执行） | 值改 `concat!("config/opc/", "industries")`，并在注释写明原因 |
| 4 | 规则表漏项：`industry_config` / `IndustryAdapterRegistry` / `OpcIndustryAdapter` / `OpsIndustryDecision` / `IndustryLearning{Sample,Metrics}` / `IndustryWorkflow` / `industry_kpi_service` / `industries_base_dir` | ⚠ `\bIndustryAdapter\b` 因词边界**不会**匹配 `IndustryAdapterRegistry`（后面是 `R`）—— 看着覆盖了，实际漏一半 | 新增「规则覆盖率审计」：分词 → 与规则集比对 → 报未覆盖清单，再逐个判「契约层保留」还是「漏项」 |

⇒ **可复用**：批量更名必须配**三道自检** —— ①规则覆盖率审计（找漏项）②mod 声明 ↔ 文件存在性 ③`include_str!` 路径存在性。仅靠 dry-run 的「残留为零」会把「检查已死」当成「检查通过」。

### 9.5 遗留

- `config/opc/industries/` **目录仍在**（`rename` 被 EPERM 拒绝，改用 `cpSync` 复制）。代码只读新目录 `domain_packs/`，旧目录无害；确认无碍后可删。
- `AppHeader.tsx` 硬编码 14 条路径，是路由的**第二真相源**（第一是 `pageRegistry.ts`），待收敛（见 §八）。

### 9.6 ⚠ 自检脚本制造了 **39 条「硬失败」**，一条真的都没有

`output/tmp-b3-rustcheck.mjs` 首跑报「硬失败 8 项」→ 修一版 23 项 → 再修 8 项 → **0 项**。
四型机制（全部是**脚本**的错，不是被检代码的错）：

| 型 | 机制 | 条数 | 修法 |
|---|---|---|---|
| ① 集成测试是 crate 根 | `tests/foo.rs` 里的 `mod common;` 该解析到 `tests/common/mod.rs`，脚本按普通模块解析成 `tests/foo/common.rs` | 7 | 把 `tests/`、`benches/` 下顶层 `.rs` 一并当 crate 根 |
| ② `#[path]` 覆盖默认解析 | `lib.rs` 用 `#[path = "divergence-log.rs"] pub mod divergence_log;`（连字符文件名；**该文件已于 2026-09-16 删除**，此处保留作判据形态） | 1 | 正则须捕获 `#[path = "..."]` 并列入候选 |
| ③ 通配 re-export | `pub use core::*;` / `manager::*` / `types::*` ⇒ 名字**不可静态枚举**，脚本把「没被列入」当「不存在」 | 23 | 检出 `::*` 即**整体跳过该 crate 并如实登记覆盖缺口**，不得判失败 |
| ④ crate 根内直接定义的 item | providers 的 `build_http_client` / `parse_base64_data_url` / `extract_reasoning_from_text` 都是 `lib.rs` 顶层 `pub fn`，可见于 `crate::x`；抽取器只收 `mod` + `pub use` | 8 | 抽取器加上 `pub fn/struct/enum/trait/const/static/type/union` + `macro_rules!` |

**③ 段噪声量级对比**：修正前按「全仓一个 crate 根」解析 ⇒ **2486 条「可疑」**；
改为「按文件自身 crate 根 + 首段名可见性」后 ⇒ 校验 **1781 次、失配 0**，另有 **523 次**因通配 re-export **跳过**。
⇒ **2486 → 0 的差值全是假阳性**；其中 523 次是**不可静态判定**的，必须登记为**覆盖缺口**而不是「通过」。

**负对照取证**（判据 #202 / #157：从没红过的守卫不可信）：
对 `providers/src/url_utils.rs` 注入 `use crate::zzz_nonexistent_probe;` ⇒
门禁**恰好 1 条**失配 + `EXIT=1` + 校验次数 **1781 → 1782**（证明注入行真被读到，而非「模式写错所以 0 命中」）；
还原后 **sha256 逐字节一致**（`67a9dd964e5ef9c0`），零足迹。

### 9.7 批次③ 编译验证

`cargo check --workspace --message-format=short`：**37 个 lib crate 全部 `Checking`、零 error**；
root package 的 lib target（`axagent_lib`，`commands/**` 属它）报 **1 条真实错误**：
`opc_workflows/mod.rs:216 cannot find value LEGACY_DOMAIN_PACKS_DIR in this scope`
—— 常量定义在 `domain_pack.rs`、`mod.rs` 用了却没 `use`（**跨文件新增常量漏引入**）。
已修：`pub use domain_pack::LEGACY_DOMAIN_PACKS_DIR;`；并把该处文档注释改为**引用常量名**
（不复写旧路径字面量 —— 否则重跑更名脚本会把「旧目录」也改成新目录，回退分支再次静默失效，属 9.4 #3 同族）。

⚠ **时序作废**：该轮首次 `cargo check` 的 `axagent_lib` 单元启动于 14:16:02，**早于**本处修复，
按判据 #175 其对 `mod.rs` **零证据力** ⇒ 已重跑（排队等 `cargo run` 释放 build 目录锁）。
**结论以重跑日志 `output/tmp-cargo-check-b3b.log` 为准。**



## 十、批次④ 实施记录（2026-09-15 22:40~23:1x）

### 10.1 落地规模

| 项 | 值 |
|---|---|
| 改名引擎 | `output/tmp-b4-rename.mjs`（116 条规则按长度降序合成单条 alternation 正则 ⇒ 长 token 天然优先） |
| 文件级分拣 | OPC 59 文件 / 916 处 · UNCLASSIFIED 8 文件 / 92 处 · STOCK 26 文件 / 5 处 · **混存 0 个** |
| 轮次 | 主轮 + 2A/2B/2C 定向轮 + KEYONLY（i18n）轮；每轮「零未登记残留 + 保护名单零破坏」 |
| 保护名单 | 职业角色名 4 · `get_industry_ranking` · **旧链兼容路由 `/opc/industry`** · DB 存量值 `"opc-industry"` |
| DB 迁移 | 新增 `v232_opc_domain_pack_rename`；备份 366MB `pg_dump` → `output/backup-db-2026-09-15/` |
| DB 改动 | 表名 `opc_industries`→`opc_domain_packs`（+ PG 索引名）· 3 列 `industry_id`→`domain_pack_id` · 4 表值 · `workflow_templates`（含版本表） |

**契约分层的四条独立机制**（风险完全不同，不可混谈）：
① Tauri **命令名**：漏改 `invoke` ⇒ 显式 `command not found`（会立刻炸，最安全）
② Tauri **命令参数**：v2 默认 camelCase↔snake_case 自动映射 ⇒ 两侧写法不同但都合法（**证不了对错**）
③ **serde DTO 字段**：本仓 `#[serde(rename_all = "camelCase")]` **1466 处**（总 1820）⇒ 前端写 `industryId` 本来就对
④ **字符串字面量**（i18n key / DB 值 / 路由 / LLM 工具名）⇒ **任何编译器都不查**，唯一高危面

### 10.2 ★★ 最大缺陷：裸词规则不认识「命名空间」⇒ i18n 跨域误伤 4 键 × 11 包

`industry→domain_pack` 的 KEYONLY 键改名把**股票业务**键一起改了：

| 被误改 | 应为 | 归属 |
|---|---|---|
| `stockAnalysis.domain_pack` | `stockAnalysis.industry` | 股票 |
| `stockAnalysis.evidence.domain_pack` | `stockAnalysis.evidence.industry` | 股票 |
| `viz.scene.domain_pack-chain` | `viz.scene.industry-chain` | 股票产业链 |
| `serenityPanel.nodeTitles.t-domain_pack-rank` | `serenityPanel.nodeTitles.t-industry-rank` | Serenity 节点 |

**两套「常规」检法都靠不住**：`tsc --noEmit` **全绿**（key 是字符串字面量，不进类型系统）；
官方 `check_i18n.py --strict` **只报出 1/4**（只发现代码用**字面量**引用且包内缺失的那一个；
另 3 个代码用**变量拼键** ⇒ 完全无感）。
**唯一完备检法 = 「旧版↔新版键集合差集」**：旧键消失 ∧ 新键存在 ∧ 旧键不在 `opc.*` ⇒ 误伤
（`output/tmp-b4-i18n-audit.mjs`，44 处，修复后归零）。

**修复手法**：先试「parse→改键→stringify 重建」⇒ **预测 diff 2892 行**（已改名键整体挪位），
安全阀拦下；改**纯文本外科手术**（按缩进/括号推 JSON 路径的行扫描器，**只改 4 行** ⇒ 预测 diff 恰 = 4）。

### 10.3 ★ 白名单按「路径」分拣漏掉跨界文件 ⇒ E0407 ×2 + E0046

`stock_orchestration.rs` 因文件名含 `stock_` 被判「股票红线」而**整个排除**，
但它 `impl DomainPackAdapter`（trait 方法名批次③ 已改）⇒ 编译报错。
判据修正：对每个「被排除」文件做**符号级**判定（是否出现 `DomainPackAdapter` 等目标层类型名）。
实测 26 个 STOCK 文件里 **2 个**是跨界文件。
该文件实为**域包层**实现（`industry_name = "股票投资分析"` 是域包显示名，不是股票行业）⇒ 判定必须改：
`StockIndustryAdapter`→`StockDomainPackAdapter`(16) · `create_stock_industry_adapter`(2) ·
`industry_id`/`industry_name`(5+5) · 中文注释 6；消费端 `stock_adaptive_engine.rs`(4+6) 与 `lib.rs`(3) 同批。

### 10.4 批次③ 遗留的两类缺陷（本批发现并修）

1. **改名只改内容不改文件名** ⇒ `mod seed_domain_pack_consulting;` 而磁盘仍是
   `seed_domain_pack_industry_consulting.rs` ⇒ `E0583`。
   ⚠ **被上游 crate 的编译失败挡住**（`analysis-engine` 失败后 cargo 不编 `axagent_lib`）⇒
   只看一次 `cargo check` 会漏掉整层下游错误。
   ⇒ 静态校验「mod 声明 ↔ 文件存在性」必须**独立于编译**先跑
   （`output/tmp-b4-modcheck.mjs`：1970 .rs / 1872 条 mod / 154 条 `include_*` ⇒ 硬失败 0）。
   解析规则注意：`foo.rs` 自身是模块文件时子模块在 **`foo/` 下**（Rust 2018）——
   混为一谈会报出上百条假阳性（`dao/src/repo.rs` + `repo/`）。
   同批重命名文件名 2 个：`seed_domain_pack_industry_consulting.rs`、`configs/domain_pack_learning/industry_consulting.yaml`。
2. **改名未同批迁移「按路径记账的门禁基线」** ⇒ 棘轮静默失效，见 10.5。

### 10.5 ★★ 门禁基线漂移：27 处超基线里 20 处是改名伪影

`scripts/layer-discipline-allowlist.json` **按文件路径**记「允许的违规数」。批次③ 改了 20 个文件名却没动基线
⇒ 旧路径成死账（按 0 计）、新路径不在基线（`base ?? 0`）⇒ **一律判「新越界文件」**。
门禁自己都报了「20 个文件已低于基线 ⇒ **棘轮失效**」。

| 规则 | 死账（旧名，文件已不存在） | 新路径超基线 | 计数 |
|---|---|---|---|
| `commands-no-direct-db` | 17 条 | 17 条 | **全部同数**（2/3/5/1×14）⇒ 1:1 伪影 |
| `commands-no-sibling-call` | 3 条 | 3 条 | `bridge` 2=2 ✓；`actions` 2→4 ✗；`runtime` 3→5 ✗ |

**修法纪律**：只把死账的**原基线计数**搬到新路径（**绝不采纳当前计数** —— 那会把真实新增违规一起吸收 = 放宽门禁），
每条规则断言「**文件计数之和不变**」（实测 656→656 / 84→84）；目标已存在条目时取**较小值**。
**禁用 `--update-baseline` 一把梭**。迁移后 **27 → 9**，「低于基线」警告消失（棘轮恢复）。
脚本：`output/tmp-b4-layer-migrate.mjs`；备份 `output/backup-2026-09-15-b4g/`。

**残留 9 处（与批次④ 无关，已用备份对照归因 —— 见 10.6）**：
`stock_workflow/sim_hook.rs`(3+1) · `stock_workflow/rhai_pm.rs`(1) · `opc_workflow_kpi_hook.rs`(1+2) ·
`opc_workflows/seed_content_media.rs`(1) · `opc_learning_hook.rs`(4>3) ·
`opc_capability_pack_actions.rs`(4>2) · `opc_capability_pack_runtime.rs`(5>3)。**待裁决**是否修。

### 10.6 两条「测量 / 归因」纪律

1. **用改动前备份对照，零成本排除自己**：比「跨模块调用数」在批次④ 前后是否变化 ⇒
   实测 `opc_capability_pack_actions.rs` 56→56、`opc_capability_pack_runtime.rs` 17→17、
   `opc_learning_hook.rs` 4→4 ⇒ **9 处残留全部与本批无关**，不必等编译也不必猜。
2. **朴素逐行 diff 在行数变化的文件上整体错位** ⇒ 首版审计脚本伪造出 **171037 条**「结构异常」
   （真值仅 2295 条含关键词改行）。判「改了哪些 token」必须用**集合差（multiset）**。

### 10.7 门禁结果

| 检查 | 结果 |
|---|---|
| `tsc --noEmit` | ✅ EXIT=0 / 0 行（**不可**用它证明契约改名正确） |
| `check_i18n.py --strict` | ✅ 全过 |
| `dprint check` | ✅ EXIT=0 |
| `mod`↔文件 / `include_*` 静态校验 | ✅ 硬失败 0 |
| `vitest run` | ⚠ 1/158 失败：`App.d2.test.tsx` 的 D2 loader 用例**超时 10000ms**（非断言失败；当时机器被 cargo 压满）⇒ 待空闲时复跑判定 |
| `check-layer-discipline` | ⚠ 27→9（20 条伪影已归位）；残留 9 处非本批 |
| `cargo check --workspace` | ✅ **EXIT=0 / `Finished in 10m 07s`**（RUN1；时序分层判定见 10.8） |
| DB 迁移 v232 | ✅ 表/列/索引/值/模板全部落地；`pg_dump` 366MB 备份 |

### 10.8 批③ 遗留的「时序作废」闭环 —— RUN1 已通过（时序必须**分层**判定）

本批 `cargo check` 由 `output/tmp-cargo-b4b.log` 记录：

| RUN | 命令 | 启动 | 结果 |
|---|---|---|---|
| RUN1 | `cargo check --workspace --message-format=short` | 22:51:03 | ✅ **`EXIT=0` / `Finished dev profile in 10m 07s`** |
| RUN2 | `cargo check -p axagent-analysis-engine --all-targets` | 23:01:11 | ⏳ `Blocking waiting for file lock`（排队中） |

**时序核对（判据 #175，逐文件 mtime vs 日志启动 22:51:03）**：

| 文件 | mtime | 判定 |
|---|---|---|
| `analysis-engine/src/stock_orchestration.rs` | 22:50:00 | ✅ 早 63s |
| `analysis-engine/src/stock_adaptive_engine.rs` | 22:50:00 | ✅ 早 63s |
| `analysis-engine/src/lib.rs` | 22:50:00 | ✅ 早 63s |
| `opc_workflows/seed_domain_pack_consulting.rs` | 22:35:05 | ✅ 早 |
| `src/i18n/locales/zh-CN.json`（i18n 面） | **22:54:14** | ⚠ **晚于日志 3min** |
| `scripts/layer-discipline-allowlist.json`（门禁面） | **22:58:25** | ⚠ **晚于日志 7min** |

⇒ ★ **判定必须分层，不能用一句「日志有效」概括两类不同的被背书对象**：

- 对 **Rust 侧**：全部 `.rs` 改动均早于日志启动 **63 秒以上**，且日志里出现
  `Checking axagent-analysis-engine` ⇒ 该轮**确实重编了**改动所在 crate
  ⇒ `EXIT=0` **有效**，本批「3 个编译错误 + 2 个文件名」的修复确已清零。
- 对 **i18n / 门禁基线**：两文件晚于日志 ⇒ **本日志不得为其背书**。
  但它们**不参与 rustc 编译**，由 `tsc`(✅) / `check_i18n.py --strict`(✅) / `check-layer-discipline`(⚠ 见 10.5) 各自覆盖
  ⇒ **无证据缺口，但严禁混用**（把「Rust 编译通过」当成「i18n 改对了」的依据 = 判据 #175 的典型误用）。

**RUN2 为何排队**：另一会话的 `cargo check -p axagent --all-targets`（22:55:27 启动，持 build 锁）
仍在跑，按「不是自己发的一律不动」不杀不抢 ⇒ 保持排队。RUN2 的增量价值是覆盖
`analysis-engine` 的 **test/bench target**（RUN1 的 `--workspace` 不含 `--all-targets`）。

### 10.9 并发会话实证

`src/components/settings/CapabilityDomainsSettings.tsx` 与 `src/lib/browserMock.ts` 的 mtime = **22:53:11**，
当轮我**没有碰过这两个文件**（只改了 Rust + 11 个 locale JSON）。首次 `dprint check` 报这 2 个未格式化、
稍后复跑变 0 ⇒ **另一会话在 22:53:11 格式化了它们**。
⇒ 印证判据 #200 / #222：并发会话会替你改工作区，**别重复修**，只需重跑被判无效的那几步。

### 10.10 ★★ DB 运行时验证 —— 批次④ 唯一能证明「契约真的生效」的证据

Rust 侧改了列名、DB 侧由 v232 迁移，而 **SQL 里的表名/列名是字符串字面量，rustc 完全不查**
⇒ `cargo check EXIT=0` 对此**零证据力**。故直连库取证（`output/tmp-b4-dbcheck.sql` → `.log`，
PG 18.4 / `axinvest`）：

| 检查项 | 实测 | 判定 |
|---|---|---|
| 表 `opc_domain_packs` / `opc_industries` | 前者存在 / 后者 **0 行（不存在）** | ✅ 表改名生效 |
| 三表列名 | 仅 `domain_pack_id`，**无 `industry_id`**；`NOT NULL` + 无 DEFAULT | ✅ 且 RENAME 保留了 v230 的「禁静默空串」语义 |
| `agency_experts.category` | `opc-domain_pack`=**45**、`opc-industry`=**0** | ✅ 与迁移文档预测 45 一致 |
| `agent_profiles.category` / `.source` | 各 **44** / **0** | ✅ 与预测一致 |
| `agency_experts.active_domains` | `industry_consulting`=**0**，`consulting`=**3** | ✅ |
| `workflow_templates` | `consulting_harness_workflow` / `/automation/consulting/harness` | ✅ |
| 索引（PG） | `opc_domain_packs_pkey` / `idx_opc_domain_packs_enabled` / `idx_rl_experiences_domain_pack`，**零 industry 残留** | ✅ |
| 模板内容 `nodes`/`edges`/`variables` | 残留 **0** | ✅ |

`opc_domain_packs` 表 **0 行属预期**（由应用启动时 `seed_opc_domain_packs_from_seed_files`
扫描 seed 文件填充，当前应用未启动），**非缺陷**，但登记为观察项。

### 10.11 ★★ 一次「历史被重写」的误判风险（本批最有价值的判据）

排查 v232 时发现：**全仓 grep 不到任何 `ADD COLUMN industry_id`，也 grep 不到 `CREATE TABLE opc_industries`** ——
`v210_opc_ext.rs` 建 `opc_kpi_records` 时的列是 `id/name/value/unit/period/recorded_at/created_at`
（**无 industry 列**），`v211_opc_domain_packs.rs` 建的也是**新名**。

若按「找不到旧名建表语句」的直觉推论「v232 在改一个不存在的列 ⇒ 应删除」，
**这个推论是错的，且删了会让老库升级后直接崩**（报 `relation "opc_domain_packs" does not exist`）。

真因：**v210 / v211 的建表语句曾被 in-place 改名**（老版本建的是 `industry_id` / `opc_industries`，
现在读到的是改后的新名）⇒

- **新库**：跑改名后的 v210/v211 ⇒ 直接建新名 ✅
- **老库**：表/列是旧名，而 `CREATE TABLE IF NOT EXISTS` **不会改既有结构** ⇒ **必须靠 v232 修** ✅

⇒ **判据**：当新增的改名迁移引用了「代码里查无建表语句的旧名」时，
**先假设建表语句已被重写**，再去要独立证据（v232 文档头的「实测基线」表即该证据），
**不得据「grep 不到」删除迁移**。

### 10.12 ★★ 路径维度：一类内容扫描完全无感的残留面

本批所有内容审计（`.rs/.ts/.tsx` **正文** grep）**看不见路径名**。补做路径审计
（`output/tmp-b4-pathcheck.mjs`）后的结果：

| 类别 | 命中 | 判定 |
|---|---|---|
| 目录 `agency_experts/skills/industry-chain-analysis` | 1 | ✅ **股票业务**（`industry_chain` 是股票工具名；审计词表漏了连字符形） |
| `agency_experts/stock-analysis/industry-chain-analyzer.md` | 1 | ✅ 同上 |
| `docs/audits/AUDIT-industry-label-removal-*.md`、本方案文档 | 2 | ✅ 标题即内容，历史存档 |
| **`agency_experts/opc/domain_packs/{consulting,finance_invest}/industry-*.md`** | **4** | ⚠ **域包资产，登记待裁决** → 10.13 #1 |
| `src/components/layout/__tests__/legacyIndustryPathRedirect.test.tsx` | 1 | ℹ️ 存在「旧路径重定向」兼容机制，含测试守护 |
| `Cargo.toml` 的 `name` / `package.json` | **0** | ✅ `crates/` 目录名零残留 |

⚠ **假阳性排除**：`target/debug/deps/axagent_opc_industries-*.d` 里写着 `crates\opc-industries\src\lib.rs`，
但实测 **`crates/opc-industries` 目录不存在**（`ls crates/ | grep industry` 为空），
那些 `.d` 是历史构建残留 ⇒ **`target/` 里的路径引用不可作为「源码存在该目录」的证据**。

⇒ **判据：改名审计必须四维全覆盖**（本批的教训）——
① 文件正文 ② 文件名 ③ **目录名** ④ 清单文件的 `name` / `identifier` 字段。
只做 ① 会把「目录名残留」**整类**漏掉。

**同族疏漏（自查）**：本批词表用了 `opc_industries`（下划线）与 `opc-industry`（单数连字符），
**漏掉 `opc-industries`（复数连字符）** ⇒ 导致 `src/types/index.ts:921` 的 `"opc-industries"` 一直未被发现。
⇒ **判据：改名词表必须枚举「分隔符 × 单复数」的全部组合**，不能只测手头已有的写法。

### 10.13 登记待裁决（本批发现，**未擅动**）

| # | 项 | 位置 | 为什么不擅动 |
|---|---|---|---|
| 1 | 4 个 `industry-*.md` 域包专家资产改名 | `agency_experts/opc/domain_packs/{consulting,finance_invest}/` | 有**真实消费端三组常量表**（`opc_setup/capability_pack_experts.rs`：expert id `L63/262/269/276`、文件路径 `L66/265/272/279`、工具白名单 `L94/285-287`）；且 `seed_capability_pack_finance_invest.rs:40` 的 `opc-finance_invest_lead-finance-industry-researcher` 是**会落库的 agent id** ⇒ 改名连带 DB 数据迁移（v233 级），会断掉既有 `agent_profiles` / 会话 / 反思记录的引用 |
| 2 | `src/types/index.ts:921` `BuiltinPageKey` 成员 `"opc-industries"` | 前端类型 | **零消费端**（全 `src/` grep 仅此 1 处 = 其自身声明）⇒ 悬空路由 key；但去留与「侧栏 18 平级入口 → 8 域收敛」裁决强耦合 |
| 3 | `.rs` 注释/日志文案里的中文「行业」 | `analysis-engine/src/opc/*`（≈148 行；如 `workflow.rs:74` 的 `format!("{domain_pack_id} 行业分析触发")`） | 属**用户可见文案 + 会落库**（生成节点标题）⇒ 改动会使代码与已落库模板标题不一致，需产品语义决策 |
| 4 | 门禁残留 9 处 | 见 10.5 | 已用改动前备份对照归因为**非本批** |

### 10.14 一条「同名不同义」的排除（防后续误并）

`docs/audits/AUDIT-industry-label-removal-2026-09-14.md` 标题含 industry，但讲的是**另一条线**：
股票**判据**去行业归属（删 `sector_beta` 关键词查表、删金融业白名单豁免）。
与「OPC 行业 → 域包」的**命名迁移**语义无关 ⇒ **不得**并入本迁移的批次计划。

⇒ 判据 K 组「一名多义」的实例：同词「行业」在本仓至少两义
（① 股票行业分类 ② OPC 域包旧名），**审计时须先判语义域再判词形**。

---

## 十一、批次 E 实施记录（2026-09-15 23:10~23:4x）—— 四项裁决的落地

批次④ 尾声登记的 4 项待裁决，用户逐项裁定：①②③ 执行、④ 维持归因结论。本节记录落地实况。

### 11.1 落地总表

| 项 | 裁决 | 实际执行 | 规模 |
|---|---|---|---|
| ① 4 个 `industry-*.md` 资产改名 | 执行 | **3 改 1 不改**（第 4 个有反证，见 11.2） | 3 md + 1 `.rs` 常量表 10 处 + DB v233 9 行 |
| ② `"opc-industries"` 悬空 key 清除 | 执行 | 删除 `BuiltinPageKey` 成员 | 1 行 |
| ③ `.rs` 注释/文案去「行业」 | 执行 | OPC 域包层 61 文件全量替换 | 714 行 / 787 处 + 补丁 23 处 |
| ④ 门禁残留 9 处 | 维持归因 | **未动**（证明见 11.8） | 0 |

### 11.2 第①项：为什么只改 3 个，第 4 个有反证

**改的 3 个**（前缀对齐，纯命名一致性）：

| 旧 | 新 | 依据 |
|---|---|---|
| `consulting/industry-research-analyst.md` | `consulting/consulting-research-analyst.md` | 全仓约定「expert id 前缀 = 目录名」 |
| `consulting/industry-strategy-advisor.md` | `consulting/consulting-strategy-advisor.md` | 同上 |
| `consulting/industry-implementation-coach.md` | `consulting/consulting-implementation-coach.md` | 同上 |

全仓 12 个域包目录里，`accounting/accounting-*`、`game_dev/game-*`、`finance_invest/finance-*`、
`software_dev/software-*` … 都是「目录名/目录名-角色.md」，**唯 `consulting/` 用 `industry-` 前缀** ——
来源是该目录旧名 `industry_consulting`（目录已改名，id 没跟）。

**不改的第 4 个 —— `finance_invest/finance-industry-researcher.md`**，反证三条（读资产与下游后取得，非推测）：

1. **`industry-` 在那里不是前缀**：前缀是已对齐的 `finance-`；`industry researcher` 是**岗位名**（行业研究员）的一部分。
2. **role 早已是域包语义**：该资产 frontmatter 是 `role: domain_pack_researcher` —— 与 `consulting/` 那个的 `role: research_analyst` 不同，说明它不是「旧前缀漏改」。
3. **下游是投研业务链**：`seed_capability_pack_finance_invest.rs` 的节点名「行业研究」（L37）、prompt「你是行业研究专家」（L38）、条件「根据行业研究结果判断」（L50）、模板描述「市场分析 → 行业研究 → 资产配置 …」（L157）；且 agent id `opc-finance_invest_lead-finance-industry-researcher`（L40）**已落库**，被 `finance_invest_harness_workflow.nodes` 引用（实测 `nodes LIKE '%finance-industry-researcher%'` = **t**）。

> 若强行改 id ⇒ 出现「id 叫 `domain-researcher` 而 prompt 说『行业研究专家』」的自相矛盾，
> 且须连带改已落库模板 nodes 并升 `TEMPLATE_VERSION`。**按纪律 5 单列告知，等一句话即可执行。**

同时删掉一处既有重名：`consulting` 与 `finance_invest` 的专家中文名**都叫「行业研究员」**。
前者 `role: research_analyst` ⇒ title 改「研究分析师」（顺带消重），后者保留。

`consulting_harness_workflow.nodes` **不引用**任何 expert id（该模板 agent 节点 expert 位为 `None`）
⇒ 无需改模板、无需升 `TEMPLATE_VERSION`。这与 finance 形成**同一次裁决下的两种代价**。

### 11.3 第②项：悬空 key 清除

`src/types/index.ts:921` 的 `| "opc-industries"` 删除。证据：全 `src/` 仅此 1 处出现（= 其自身声明），
**零消费端**；`pageRegistry.ts` 的域包路径已全部是 `/opc/domain/{packId}`（注释亦记明迁移日期）。

> 同时实测：同段的 14 个 `opc-domain-pack-*`（L907–920）**同样零消费端**（旧的域包 key 形）。
> 用户只点名 `opc-industries` ⇒ 未擅动，登记于 11.9。

### 11.4 第③项：`.rs` 中文「行业」→「域包」

**改前全仓摸底**：`.rs` 含「行业」1169 行 / 1974 文件 —— 但**绝大多数是股票业务语义**，
按既有 fileclass（OPC / OPC-irrelevant / STOCK / UNCLASSIFIED）切分后真正属批次 E 的只有 `opc/` 层那一小撮。

**执行**（61 文件 / 737 处）：

| 阶段 | 范围 | 规模 |
|---|---|---|
| 主替换 | 59 文件整文件替换 + `opc_capability_pack_actions.rs` 限行 ≥2000 + `migrations/mod.rs` 限 L392/L487 | 714 行 / 787 处 |
| 补丁 | `opc_capability_pack_actions.rs` 与 `domain_pack_experts.rs` 的**注释行** | 23 处 |

替换顺序**先长后短**：`行业咨询`→`咨询` → `行业包`→`域包` → `行业`→`域包`
（否则「行业包」会变成「域包包」）。复核「域包包 / 域包咨询 / 域包行业 / 行业域包」四类副作用均为 **0**。

**补丁的判别规则**（这批文件里业务与术语交错，逐行判不可行）：

> `opc_capability_pack_actions.rs` 的**业务语义全部落在字段值内**（`description` / `system_prompt` /
> `user_prompt_template` / `name` / `label`），而**注释行**（`//!` 文件头、`///` 字段文档、`//` 段分隔）
> 无一例外指代域包概念 ⇒「**只改注释行**」是完备且可机检的规则。

唯一例外 `name: "行业咨询"` 是 **consulting 域包的显示名**，按值改为「咨询」
（其余 11 个域包名均不含「行业」——「会计与财务管理 / 金融投资 / 游戏开发 …」）。

### 11.5 ★ 本批最重要的发现：外键 `agent_profiles_expert_id_fkey`

改 `agency_experts.id` 的第一次尝试被 PG 拒绝：

```
ERROR: 在 "agency_experts" 上的更新或删除操作违反了在 "agent_profiles" 上的外键约束
       "agent_profiles_expert_id_fkey"
DETAIL: 键值对(id)=(opc-industry-research-analyst)仍然是从表"agent_profiles"引用的.
```

**这是一条 `rustc`、`tsc`、grep 全都看不见的契约** —— 存在于 DB schema，只在真机执行时暴露：

- `UPDATE agency_experts SET id = 新` ⇒ 被拒（旧值仍被引用）
- `UPDATE agent_profiles SET expert_id = 新` ⇒ 同样被拒（新值在父表尚不存在）

**两个方向互锁，没有一步到位的写法**。采用三步：**① 解引用（`expert_id = NULL`）→ ② 父表改名 → ③ 子表改名 + 回填**。
`expert_id` 本就可空（entity 侧 `Option<String>`）故 ① 无损。

⇒ 判据：**凡「改主键 / 被外键引用的 id」，必须先查 `information_schema` 的外键而非只看 Rust 代码**；
「`cargo check` 通过」对这类改名的证据力为零。这正是批次④「唯一高危面 = 字符串契约」的**第二条**实例
（第一条是 i18n key，本条是 DB 外键）。

### 11.6 DB 迁移 v233

新增 `dao/src/migrations/v233_opc_consulting_expert_ids.rs`，并修正一处**既有漂移**：
`CURRENT_VERSION` 原为 `231` 而迁移数组早已含 v232 ⇒ 改为 **233**（v229 的 description 明文要求
「同步修正为数组最大 version」，属既有约定未落实）。

**实测受影响行**（`axinvest`，迁移前后对照）：

| 表 | id 形态 | 前 | 后 |
|---|---|---|---|
| `agency_experts` | `opc-{expert_id}` | 3 | 3（name 同步「研究分析师」） |
| `agent_profiles`（source=`opc-domain_pack`） | `opc-{expert_id}` | 3 | 3（name `💼 …` 同步） |
| `agent_profiles`（source=`opc-bulk`） | `exp-opc-opc-{expert_id}` | 3 | 3（name 同步） |

迁移后核查：旧 id 残留 **0**；`opc-finance-industry-researcher` 仍 **1** 行（排除项未被误伤）；
`agency_experts.category='opc-domain_pack'` 仍 **45**（未增减）。

**幂等性**：**初版写法不幂等，已修**（见 11.6b —— 这是本批最严重的事故）。
`UPDATE` 全为 `… WHERE 旧值`，本身幂等；问题出在「清理目标行」的 `DELETE`：
现为 `DELETE … WHERE id = 新值 AND EXISTS (旧行)`，重跑即 `DELETE 0`。

### 11.6b ★★ 事故与恢复：v233 的 `DELETE` 不幂等 ⇒ 真删数据

**触发**：迁移落库并核对完计数后，**顺手多跑了一次幂等性验证**（同一份 SQL 再执行一遍）。
这一步不在计划内，却是本次唯一的发现窗口。

**事故链**：

1. 初版逻辑是「先清目标行、再改名」，清理语句**无条件**：
   `DELETE FROM agency_experts WHERE id = 'opc-consulting-research-analyst'`（**新值**）。
2. **第一次**执行：新值在库里不存在（种子只产旧值）⇒ `DELETE 0`，看着完全无害。
3. **第二次**执行：第一步 `UPDATE` **已把新值变成数据本体** ⇒ 同一句 `DELETE` 命中的
   正是刚改好的行（实测 `DELETE 2` + `DELETE 1`）。
4. 且 `agent_profiles.expert_id` 外键为 **`ON DELETE SET NULL`** ⇒ 删父行**静默**把
   6 行子表 `expert_id` 置空 —— **不是报错，是「成功」**。

**实测证据**：

| 计数 | 事故前 | 事故后 |
|---|---|---|
| `agency_experts` `category='opc-domain_pack'` | 45 | **42** |
| `agent_profiles` `category='opc-domain_pack'` | 44 | 44（未减，但 6 行 `expert_id` 被置空） |
| `agent_profiles` `expert_id IS NULL` | 25（既有基线） | **31** |

**根因**：我把「清理目标行」当成了幂等操作 —— 它**只在第一次执行时幂等**。
`DELETE … WHERE id = 新值` 的语义在改名**前后发生翻转**：改名之前是「清理种子残留」，
改名之后是「删除数据本体」。

**修复**（两处 `DELETE` 各加「**旧行仍存在**」的存在性条件）：

```sql
DELETE FROM agent_profiles WHERE id = '{p}'
  AND EXISTS (SELECT 1 FROM agent_profiles WHERE id IN ('{old_id}', '{old_exp}'))
DELETE FROM agency_experts WHERE id = '{new_id}'
  AND EXISTS (SELECT 1 FROM agency_experts WHERE id = '{old_id}')
```

第二次执行时旧行已不存在 ⇒ 两个 `AND EXISTS` 皆假 ⇒ `DELETE 0`，本体安全。

**数据恢复**（`output/tmp-b4e-restore.mjs`）：**按种子逻辑重建**被删的 9 行，不手工 `INSERT`
—— 值**逐字段复刻种子源**，保证恢复结果与「应用冷启动重新种子」等价：

| 复刻来源 | 复刻内容 |
|---|---|
| `commands/opc_setup/capability_pack_agents.rs:64-83` | `agency_experts` 3 行 |
| `commands/opc_setup/capability_pack_agents.rs:140-163` | `agent_profiles`（`opc-domain_pack` 形态）`id = opc-{expert_id}` |
| `commands/opc_setup/mod.rs:493-529` | `agent_profiles`（`opc-bulk` 形态）`id = exp-opc-opc-{expert_id}` |

`system_prompt` 由 Node **读 md 全文**填入（不手抄）。**恢复后计数复原**：

```
agency_experts   opc-domain_pack |  45  ✅（事故后 42）
agent_profiles   opc-domain_pack |  44  ✅
agent_profiles   opc-bulk        | 386  ✅
旧 id 残留（应=0）               |   0  ✅
expert_id NULL（应=25 既有基线） |  25  ✅
```

**教训**（已外迁为判据 **#245**）：**幂等性必须实测** —— 成本是把 SQL 再跑一遍，
不测的代价是**静默删数据 + 静默置空外键**。

### 11.6c `agency_experts.description` 的旧名残留：**三代快照**（判据 **#251 / #247**）

批次④ 登记时只看到「37 行含 `opc-industry`」。本轮做**全代普查**后把这个结论改写了。

`description` 由 `capability_pack_agents.rs:67/143` 的 `format!("OPC {category} — {name}")`
在**种子时**生成 ⇒ 概念名改过几次，库里就沉淀几代：

| `description` 前缀 | 行数 | 性质 |
|---|---|---|
| `OPC opc-industry` | **37** | 第 1 代快照（陈旧） |
| `OPC opc-domain` | **5** | **第 2 代快照（陈旧）—— 初版普查漏掉的正是这 5 行** |
| `OPC opc-domain_pack` | 3 | 目标形态（= 本轮恢复的 3 个 consulting 专家） |
| `OPC {角色} — {domain} 领域` | 7 | **另一种模板**（`mod.rs:225`），**不是**陈旧 |

**自证式陈旧判据**：把派生串前缀与该行**自身权威列**比对 ——

```sql
SELECT split_part(description,' — ',1) AS desc_prefix, count(*)
FROM agency_experts
WHERE description LIKE 'OPC %'
  AND description NOT LIKE 'OPC ' || category || ' — %'
GROUP BY 1;
```

**不需要任何外部知识**即可判陈旧，且能自动区分「陈旧快照」与「另一种模板」
（`opc-company` 那 7 行前缀 ≠ category，但用角色模板 ⇒ 不判陈旧）。
真陈旧集 = **42 行**（37 + 5），全在 `category='opc-domain_pack'`。

**是否需修**：`capability_pack_agents.rs:88-94` 的 upsert `update_columns` **含 `Column::Description`**，
且全部种子调用点（L190 / L206–338）现在都传 `"opc-domain_pack"`
⇒ **重启应用即自愈，不必写迁移**。⇒ 该项可从待裁决清单**降级为已解释**。

**★ 测量工具翻车**（→ #247）：普查时同时写了 `LIKE '%opc-industry%'` 与 `LIKE '%opc_industry%'`，
两者**同报 37**；`%opc_domain%` 与 `%opc-domain%` 同样**同报 79**。根因：
**`LIKE` 里的 `_` 是单字符通配符**（`%opc_industry%` ≡ `%opc?industry%`）⇒ 静默超集匹配。
命中纪律 8「统计量恒等 ⇒ 先怀疑测量工具」——**两个不同模式给出同一计数就是警报**。
⇒ 那组「单复数 × 分隔符」四项里 **2 项是伪指标**，若不做恒等性怀疑就会把
「下划线写法也是 37 行」当成结论写进文档。正确写法：`position('opc_industry' in x) > 0` ／
`LIKE '%opc\_industry%' ESCAPE '\'` ／ 正则 `~ 'opc_industry'`。
（单复数全枚举的收益：`%opc-industries%` 实测 **0** ⇒ 确认**无复数代**。）

### 11.6d ★ 第③项**收口复核**：查出 **18 行漏改**（判据 **#248**）

**「61 文件 / 737 处改完」不等于第③项完成。** 本轮把 `.rs` 里**全部**残留「行业」的命中
枚举出来（**438 行 / 78 文件**）并**逐条归因**，才暴露 **18 行漏改** —— 全是 OPC 架构概念，
却落在按改名**前**的 `tmp-b4-fileclass.json` 分出的 `UNCLASSIFIED` 桶里而被静默跳过。

| 文件 | 行数 | 典型内容 |
|---|---|---|
| `src/init/state.rs` | 5 | `OPC 行业适配器注册表（P0-1-A：行业包驱动…）`、`初始化行业学习引擎`、`不阻塞行业工作流`、`tracing::info!("[init] OPC 行业适配器注册完成…")` |
| `src/init/services.rs` | 3 | `OPC 行业/领域工作流模板种子化` ×2、`14 行业 + 17 领域` |
| `src/state/learning.rs` | 2 | `OPC 行业学习引擎`、`OPC 行业适配器注册表 — 管理 9 个垂直行业的适配器` |
| `crates/analysis-engine/src/stock_adaptive_engine.rs` | 2 | `/// 获取行业适配器引用`、`// Step 1: 通过行业适配器进行编排` |
| `seed_domain_pack_consulting.rs` / `seed_domain_pack_finance_invest.rs` | 1 + 1 | `//! …流程行业工作流模板种子化` |
| `crates/harness/src/knowledge_graph.rs` | 1 | `/// （14 个行业各一组业务记录类型…）` |
| `src/commands/agent/mod.rs` | 1 | `// ── …股票分析 + OPC 行业命令注册为 Agent 工具 ──` |
| `src/server_main.rs` | 1 | `// …保证行业包/领域包可读` |
| `crates/tools/src/tools/mod.rs` | 1 | `// 内容资产（行业模板已引用，见 domain_pack.rs 的 opc_tool_defs）` |

**★ 两个判「单条是否漏改」的检法（都不依赖外部知识）**：

1. **兄弟文件对照**：`opc_workflows/` 下 **14 个** `seed_domain_pack_*.rs` 写「域包工作流模板」，
   只有 **2 个**写「行业工作流模板」⇒ **14:2 即铁证**。
2. **自证式陈旧 = 注释与被注释的标识符/类型不一致**：`/// 获取行业适配器引用` 挂在
   `fn domain_pack_adapter` 上；`// Step 1: 通过行业适配器进行编排` 的下一行就是
   `DomainPackContext::new()`；`// …见 domain_pack.rs…` 自称「见 domain_pack.rs」。

**落盘**：`output/tmp-b4e-residual.mjs` —— **逐行锚定**（行号 + 必须命中期望子串，任一不符
**整体中止不落盘**）+ 改前备份 10 文件到 `output/backup-b4e-residual/`；预检 18/18 通过后写入。

**收口验证**：复跑分诊 **438 → 420 行 / 78 → 71 文件**（恰好 −18）；「OPC 架构概念」残留
只剩保护名单内的合理保留（`v233_*` 5 处 / `v232_*:2` = 叙述改名本身；
`commands/knowledge.rs` 与 `repo/knowledge_graph.rs` = A 股行业实体），
另有 **1 项单列**：`migrations/v211_opc_domain_packs.rs:3` 是历史迁移注释里的**历史名**
（非叙述改名）⇒ 倾向保留，未擅动。

### 11.6e `cargo fmt --check` 门禁复核：本批真实贡献 **6 处**（收口后修正；判据 **#252 / #248**）

`cargo fmt --check` 是硬门禁（`.github/workflows/ci.yml:274` + `package.json:210` pre-commit）。
实测全 workspace **90 处偏差 / 25 文件**。归因途中连踩**两个审计假阴性**，值得单列：

**★ 假阴性 1 — `--config-path` 路径风格**：`--config-path /d/…/rustfmt.toml`（Git-Bash 风格）
喂 Windows 二进制 ⇒ `Error: unable to find a config file`；而脚本用
`… | grep -cE "^Diff in"` 统计 ⇒ **报错输出里没有 `Diff in` ⇒ 计数得 0**，被读成「既有」。
**退出码仍然是 0。**

**★ 假阴性 2 — 对「复制出去的树」复查**：对 `output/backup-*/…` 的副本跑 `rustfmt --check`
⇒ `Error writing files: failed to resolve mod 'domain_workflows': …backup…\domain_workflows.rs does not exist`
（rustfmt 必须**解析同级 `mod`**，副本树里没有兄弟文件）。修正版脚本加了「断言无 `Error:`」，
但**只匹配 `^Error:`、没匹配 `Error writing`** ⇒ 仍漏。

**★ 改用「内容级不变量」得到结论**：副本树不可复核时，用 `diff` 证明
**被格式化的那些行本身未变、顺序也未变** ⇒ 该偏差与本次改动无关。
实测 `diff backup/…/opc_workflows/mod.rs`：**仅中文词差异，`mod` 声明行 diff 为空**（而工具给的「备份 0」是假的）。

**★★ 但该论证在收口复核时被部分证伪** ——「**被格式化的那些行**未变」只证明**那些行**未变，
**不证明总数/归属未变**。逐块看「这块内容是谁写的」之后，`opc_workflows/mod.rs` 的 8 块里**有 2 块属本批**：
我插入的 `mod seed_domain_pack_consulting;` 与 `pub use seed_domain_pack_consulting::…` **未落字典序位置**
（rustfmt `reorder_modules` 默认开）。⇒ 已写入 **#252** 的修正条款：「数量持平 ≠ 无新增」。

**修正后的归因表（收口版，本批共 6 处，均**已修**）**：

| 文件 | 修前 → 修后 | 备 | 归因 |
|---|---|---|---|
| `opc_workflows/mod.rs` | 8 → **3** | 无法复核（缺同级 `mod`） | ★ **2 块属本批**（mod/use 排序）；余 3 块既有（`seed_domain_paidmedia…testing` 块排序、`LEGACY_DOMAIN_PACKS_DIR` 排序、`seed_capability_pack_design.rs:12`） |
| `opc_setup/domain_pack_experts.rs` | 5 → **4** | 5 | ★ **本批 +1**（`("consulting-implementation-coach", &[…])` 越 100 宽度，已修）；余 4 为 `include_str!` 长路径换行（既有） |
| `crates/dao/src/migrations/v233_*.rs` | 2 → **0** | 全备份树 ABSENT | ★ **本批新增文件**，2 处全属本批，已修 |
| `seed_domain_pack_consulting.rs` | 1 → **0** | 1（`backup-b4e-residual`） | ★ 最老可上溯快照已是本批中期，按节点 id 含本批改名痕迹判为本批，已修 |
| 余 22 文件 | — | 13 叶子逐一对跑 | **既有**（`opc_capability_pack_bridge.rs` 23/23、`opc_capability_pack_actions.rs` 13/13、`opc_capability_pack_runtime.rs` 7/7、`opc_learning_hook.rs` 5/5、`base_adapter.rs` 4/4 … 逐一对齐） |

⇒ **全 workspace 复测：90 块 / 25 文件 → 83 块 / 23 文件**；**本批贡献 6 处，全部已修**，
其余 **83 处既有**（`cargo fmt --check` 在本批之前**已经红**）。
**未复核（不得宣称全归因）**：4 个 **mod 根**（备份树缺同级 `mod`，rustfmt 拒绝解析）+ 4 项「现 > 更早快照」
但快照**早于本批**、无法隔离（`rl_experience_store.rs` / `task_context.rs` / `analysis-engine/opc/domain_pack_config.rs` / `dynamic_subgraph.rs`）。
**处置**：本批 6 处已修并复测；**既有 83 处不擅动**（全量 `cargo fmt` 会产生与本次任务无关的大面积 diff，单列待裁决）。

### 11.7 保护名单：哪些「行业」**故意不改**（判据落地）

本次替换**不碰**的三类，均以实据判定而非感觉：

| 类别 | 实例 | 判据 |
|---|---|---|
| 股票业务 | `concept_index.rs` 54 处（「化工行业 / 银行业」概念图谱）、`portfolio_monitor.rs` 29 处（「行业暴露」）、`astock-data/mcp_tools.rs` 34 处 | 指**股票行业分类**，是业务对象 |
| 域包内的业务对象 | `seed_domain_pack_consulting.rs`「行业扫描 / 目标行业是否值得进入」、`seed_domain_pack_finance_invest.rs`「行业研究」、`seed_domain_marketing.rs`「搜索行业相关KOL」 | 指**被研究的行业**，是投研/咨询的研究对象 |
| 历史迁移的描述 | `migrations/mod.rs:497`（v232 描述「OPC「行业」概念层取消」）、`v232_*.rs:330`（`OPC 行业 → 域包 的存量改名完成`） | **叙述的就是这次改名本身** ⇒ 改了反而失真 |

⇒ 落地了 10.14 的「一名多义」判据：**先判语义域，再判词形**。同词「行业」在本仓两义并存，
且**同一文件内也会交错**（`opc_capability_pack_actions.rs` 注释=域包 / 字段值=业务）。

### 11.8 门禁结果

| 门禁 | 结果 |
|---|---|
| `tsc --noEmit` | ✅ `EXIT=0` / 0 行输出 |
| `check_i18n.py --strict` | ✅ `EXIT=0`（9717 引用 key 全部有定义；11 locale 语法正常） |
| `dprint check` | ✅ `EXIT=0` |
| `check-layer-discipline.mjs` | ❌ `EXIT=1` —— **9 处超基线，与本批无关**（见下） |

**layer-discipline 归因（判据 #235 零成本对照法）**：

1. 9 处清单与批次④ 结束时的 9 处**逐项相同**（同文件、同计数），且含 `stock_workflow/sim_hook.rs`、
   `stock_workflow/rhai_pm.rs` 这两个**本批完全未触碰**的文件；
2. 基线 `commands-no-direct-db 656` / `commands-no-sibling-call 84` 与批次④ 迁移后一致；
3. **严格证明本批未引入**：对 59 个备份文件施加同一替换规则后要求与当前文件**逐字节相等** ⇒
   **57/58 通过**（唯一例外 `migrations/mod.rs` 是其后**故意**追加的 v233 注册改动）；
   另 2 个只改注释行的文件用「剥离全部中文字符后必须相同」判据 ⇒ **非中文差异 0 行**。
   ⇒ 本批改动等价于「中文词替换」+「mod.rs 注册」，**不可能改变『字面量导入/路径』计数**。

### 11.9 本批新登记（未擅动）

| # | 项 | 位置 | 说明 |
|---|---|---|---|
| 1 | `finance-industry-researcher` 的 id / 中文名 | 见 11.2 | 有反证，等裁决 |
| 2 | 14 个 `opc-domain-pack-*` key | `src/types/index.ts:907-920` | 同样零消费端（旧域包 key 形） |
| 3 | `opc_capability_pack_bridge.rs` 的「**9 个**行业」 | L34 / L200 | 实际 14 个域包（`assert_eq!(configs.len(), 14)`）⇒ 数字陈旧；本批只改了词未改数，**待确认 `get_all_*` 实际返回数后修** |
| 4 | 1 行 `expert_id IS NULL` | `agent_profiles`: `exp-stock-analysis-agency-stock-analysis-data-quality-inspector` | **既有**（非本批造成），`opc-bulk` 来源的孤儿 |
| 5 | 门禁 9 处 | 见 11.8 | 维持批次④ 归因 |

### 11.10 一条测量纪律（本批新得）

**「改动前备份」的复用方式有两种，判据不同**：

- 批次④ 用法：**对照计数**（改前 X 处 vs 改后 X 处）⇒ 证明「没引入新违规」；
- 本批用法：**逐字节可复现**（对备份施加同一替换规则 ⇒ 必须等于当前文件）⇒ 证明「改动的语义范围被完全界定」。

后者更强：它一次性排除「改动里混入了非预期编辑」这类事故，
且**只需要一条不依赖被测对象自身判据的等式**。代价是要求改动必须**可规则化重放**；
若改动含人工精修（如本批的 2 个补丁文件），则须对这 2 个补丁单独换判据
（本批改用「剥离中文后必须相同」）。

### 11.11 编译判据的时序作废与重新排队（**未闭环**）

`cargo check` 的判据**三次全部作废**，原因都是同一类：**日志早于最后一次改动**（判据 #175）。

| 运行 | 启动 | 问题 |
|---|---|---|
| ① 我的 `cargo check --workspace` | 23:20:47 | 一直 `Blocking waiting for file lock`；**且晚于它的 v233 `DELETE` 修复（23:25:49）** ⇒ 即便跑完也作废 |
| ② 他会话 `cargo check -p axagent --all-targets` | 23:16:43 | 先于我的最后改动 ⇒ 对 `dao` 反映的是**旧快照**，不可复用其结论 |
| ③ 他会话 `cargo test -p axagent-harness -p axagent-dao -p axagent-tools` | 23:18:03 | 同上 |

**处置**：按「并发会话不得互相饿死」纪律，**主动撤掉自己排队中的 `--workspace` 全量检查**
（`CARGO_EXIT=127`）—— `--workspace` 一旦拿到锁会持锁 1~2 小时，把另两个 job 饿死。
改为重新排队**恰好覆盖**的窄口径：

```
cargo check -p axagent --all-targets      # 启动 23:28:56
```

**覆盖性已核**（`src-tauri/Cargo.toml:160/166/170/172/181/195`）：根 crate `axagent`
依赖 `axagent-analysis-engine` / `-dao` / `-entities` / `-harness` / `-orchestrator` / `-tools`
—— 即本批改动过的**全部 6 个 crate** + 根 crate 自身（含 `include_str!` 路径风险点）
⇒ 该口径**无覆盖缺口**，且比 `--workspace` 窄。

**★ 第一次重跑的结果：`CARGO_EXIT=101`，1 个真错误 —— 时序纪律不是学术问题**

```
crates\dao\src\migrations\v233_opc_consulting_expert_ids.rs:119:27:
  error[E0308]: mismatched types: expected `&&str`, found `String`
error: could not compile `axagent-dao` (lib) due to 1 previous error
```

**根因**：`RENAMES: &[(&str, &str, &str, &str)]` 上解构 ⇒ `old_id` / `new_id` 绑定类型是
**`&&str`**（默认绑定模式），而 `new_exp = format!("exp-opc-{new_id}")` 是 **`String`**
⇒ 数组字面量 `[new_id, new_exp]` 元素类型不一致。修法：显式统一为 `&str` ——
`for p in [*new_id, new_exp.as_str()]`。

**★ 这条错误的价值远超它本身**：它证明 **②（他会话 23:16:43 的 `-p axagent --all-targets`）
若被拿来当判据，会给出一个「绿色」的假结论** —— 那次运行对我的最后改动（23:25:49）是盲的，
而错误的引入时间（v233 的 DELETE 修复）恰好在它之后。⇒ 判据 #175「日志早于改动即作废」
在**并发会话**场景下不是形式主义：**别人的绿灯对本人的改动零证据力，且不会以任何方式提示**。

⚠ **本节原「待补」已由下节取代**（cargo3 起改为 `-p axagent-dao` + 根 crate 两段式）。

### 11.11b 判据第 4~5 次作废，以及「日志冻结 ≠ 进程已死」的误判（判据 **#255**）

| 运行 | 启动 | 结果 |
|---|---|---|
| ④ `output/tmp-b4e-cargo4.log` | 23:36:54 | DAO 轮 `DAO_EXIT=0` ✅（含 E0308 修复）；根 crate 轮 23:39:23 起，**23:49 被我 `taskkill`**（`CARGO_EXIT=1`） |
| ⑤ `output/tmp-b4e-cargo5.log` | 23:48:28 | **自证式脚本**（`output/tmp-b4e-check.mjs`）；受另一会话 `clippy -p harness/dao/tools` 抢占锁，长时间 `Blocking waiting for file lock` |

**★ ④ 的两个坑**

1. **日志冻结 ≠ 进程已死**：23:41:08 起日志 230 s 不增长且无 `CARGO_EXIT`，我据此判「轮次被掐断」；
   进程表显示 cargo + **两个** `rustc --crate-name axagent_lib`（23:41:16 启动）**正在跑** ——
   非 tty 重定向下 cargo 进度行**块缓冲**，退出时才 flush（进程退出后日志一次性补齐并打印 `CARGO_EXIT=1`，即被我杀掉的证据）。
2. **杀它的理由写错了，但动作是对的**：我在 23:46~23:47:58 修了 4 处 fmt 偏差（改了 `.rs`）
   ⇒ ④ 的结论对象已被改掉（#175 时序作废），继续跑纯属占锁。**浪费的是 9 分钟编译，不是结论。**

**★ ⑤ 的改法（把 #175 从人工比对变成脚本自证）**：`tmp-b4e-check.mjs` 在启动前扫描 `src-tauri` 下
最晚 `.rs` mtime、退出时再扫一次，把 `CARGO_EXIT` / `LAST_RS_BEFORE` / `LAST_RS_AFTER` /
`VALID_FOR_CURRENT_TREE` 直接写进日志尾部 ⇒ 任何人读日志即可判定「这份结论对当前源码树是否有效」。

**判定口径（本批已核）**：`cargo check -p axagent --all-targets`（根 crate 轮）。
**判据未闭环**：⑤ 的 `CARGO_EXIT` 在本次记录时仍未产出（另一会话持锁）。


---

## 十二、门禁「真修」：分层越界清偿（2026-09-15 24:0x 起）

> 起因：用户裁定第 ④ 项不再「维持归因」，改为**真修**（分批）。本节记录已落地部分、
> 已验证的施工方案、以及**勘探途中发现的硬障碍与既有缺陷**。

### 12.1 先纠正一个我自己的提案错误

我最初的提案是「把 `opc_workflows` 的共享函数提取到**豁免模块**，并登记进
`SHARED_INFRA_MODULES`」。**这个路线不合规**，读门禁源码后推翻：

> 规则 2 的**共享基础设施模块**豁免 —— 判据：它们**不承载业务**、被绝大多数命令当
> **公共库**用（错误类型 / 错误码 / 常量 / 共享状态 / 协议常量 / 上下文 / 任务守卫）。
> ⚠ **这是人工决定，不是从代码推出的**
> —— `scripts/check-layer-discipline.mjs`，`SHARED_INFRA_MODULES` 上方注释

⇒ `resolve_domain_packs_dir` / `upsert_template` / `domain_pack` 是**业务**，
塞进豁免名单就是**刷分式修改**（门禁绿了、规则实际失效）。正确路线只有一条：
**下沉到非 `commands` 层**（这正是规则 1 原文「须经 dao / service」的语义）。

同时确认：`commands/constants.rs` **明文禁止自行定义常量**（只允许从
`axagent_harness::constants` re-export，见 AGENTS.md 禁区 12）—— 所以「把常量放
`commands/constants.rs`」也不对，权威源必须在 **harness**。

### 12.2 已落地：M1 —— 两个协议标识符的权威源上移（门禁 9 行 → 8 行）

| 标识符 | 原位置（commands） | 新权威源 | 消费点改动 |
|---|---|---|---|
| `content-media-kpi-persist` | `commands/opc_workflow_kpi_hook.rs:121` `HOOK_NAME` | `axagent_harness::constants::domain_pack::KPI_HOOK_NAME` | `seed_content_media.rs`（import + 5 处）、`kpi_hook` 自身 2 处 |
| `domain_pack_id` | `commands/opc_workflow_kpi_hook.rs:127` `DOMAIN_PACK_INPUT_KEY` | `axagent_harness::constants::domain_pack::INPUT_KEY` | `opc_capability_pack_actions.rs:2271`、`kpi_hook` 自身 2 处 |

- **不保留旧名**（无 `as` 别名）：同一个值在项目里只允许有一个名字（判据 K 组）。
- 消费方路径写 `crate::commands::constants::domain_pack::*` —— `constants` 在豁免名单内，
  符合项目既有约定（`crate::commands::constants::role::<X>`）。
- **效果**：`seed_content_media.rs` 的 sib 1 → **0**（该文件不再是「新越界文件」）；
  `opc_capability_pack_actions.rs` 4 → **3**。超基线明细 **9 行 → 8 行**。
- ★ 施工脚本踩到的坑：**旧名是新名的子串**（`HOOK_NAME` ⊂ `KPI_HOOK_NAME`）。
  先插入新 import 再按子串替换，会把刚写进去的新名一并打断（实测命中 3 次而非 2 次）。
  修法：一律 `\b` 词边界 + **先改旧名、后插新路径**。→ 判据 `#256`。
- 备份：`output/backup-2026-09-15-gate1-m1/`（3 文件）｜脚本 `output/tmp-b4f-gate-m1.mjs`。

### 12.3 M2 方案：拆 `domain_pack.rs` 双落点（消除 8 处 sib）

**目标**：`opc_workflows/domain_pack.rs` 736 行，被 4 个命令模块跨模块引用 —— 它是
**事实上的共享域模型层**，只是寄居在 `opc_workflows/` 下。下沉后可一次消除
`opc_capability_pack_runtime.rs:18/24/43`、`opc_learning_hook.rs:64/67`、
`opc_workflow_kpi_hook.rs:429/431`、`opc_capability_pack_actions.rs:2588` 共 **8 处**。

**★ 硬障碍：不能整体下沉** —— `axagent-tools` **已经依赖 `axagent-analysis-engine`**
（`crates/tools/Cargo.toml:9`），而 `domain_pack.rs` 的尾部三个 `*_tool_defs`
（L619–735）依赖 `axagent_tools::tools::opc::*` ⇒ 给 analysis-engine 加 tools 依赖会**成环**。

**⇒ 必须按依赖切分双落点**：

| 切出的部分 | 落点 | 依据 |
|---|---|---|
| L1–618：manifest/roles schema、`scan_domain_packs`、`upsert/ensure`、`export/import`、`analysis_schema`、`runtime_schema` | `crates/analysis-engine/src/opc/domain_pack.rs` | 与同目录 `domain_pack_config.rs` / `domain_pack_kpi_service.rs` / `domain_pack_validator.rs` 同族；依赖仅 harness + sea-orm + serde + std（**零 `crate::` 引用**，已 grep 实证） |
| L619–735：`stock_tool_defs` / `opc_tool_defs` / `local_tool_defs` | `crates/tools/src/tools/domain_pack_defs.rs` | 工具白名单映射的家就在 tools crate；`axagent_tools::` → `crate::` |

**连带闭包（★ 搬迁必查项）**：`opc/capability_pack.rs:714` 调 `super::copy_dir_recursive` ——
该函数定义在 `opc_workflows/mod.rs:241`（**不是** `domain_pack` 的子模块）。搬到
analysis-engine 后 `super` 变成 `opc`，链接即断 ⇒ 必须**连 `copy_dir_recursive` 一起搬**。
同时 `resolve_domain_packs_dir`（`mod.rs:215`）与它依赖的 `domain_packs_base_dir`（`mod.rs:78`）
也要搬（前者已是 4 处调用点的目标）。

⭐ **判据**：`use super::X` 出现在**子模块内**（如 `analysis_schema` 里的
`super::DomainPackManifest`）⇒ 层级不变，搬迁安全；出现在**模块顶层函数体里**
（如 `import_domain_pack` 里的 `super::copy_dir_recursive`）⇒ **层级会变，是断链点**。
故搬迁前必须 `grep super::` 逐处判「搬迁后 super 指向谁」。

**`opc_workflows/mod.rs` 的处理**：保留 re-export（`pub use axagent_analysis_engine::opc::domain_pack;`
+ `pub use axagent_tools::tools::domain_pack_defs::{...};` + `copy_dir_recursive` 的 re-export），
使 `opc_workflows` 内部 3 处 `copy_dir_recursive` 调用与 `seed_*` 的既有路径**全部不用改**。

**施工清单**（未执行）：
1. 新建 `crates/tools/src/tools/domain_pack_defs.rs`（按行区间搬迁，`axagent_tools::` → `crate::`）+ `tools/mod.rs` 注册
2. 新建 `crates/analysis-engine/src/opc/domain_pack.rs`（L1–618，**删 L19 `use std::sync::Arc;`** —— `Arc` 只被 tool_defs 段用，留着即 unused ⇒ clippy `-D warnings` 红）
3. 搬入 `resolve_domain_packs_dir` / `domain_packs_base_dir` / `copy_dir_recursive`
4. `company-runtime/src/opc/mod.rs` 加 `pub mod domain_pack;` + `pub use domain_pack::*;`
5. 修 `opc/capability_pack.rs:26` 的 rustdoc 链接 `[`super::resolve_domain_packs_dir`]` → `[`resolve_domain_packs_dir`]`
6. 改 5 个调用点（4 个 `domain_pack` use + 1 个 `*_tool_defs` use + 4 处 `resolve_domain_packs_dir` 调用）

**glob 撞名 go/no-go（已通过）**：`analysis-engine::opc` 是 glob re-export（`pub use X::*;`）。
检查发现 `AnalysisDataSource` 同名两份 —— `opc/capability_pack.rs:221`（在 `analysis_schema` **子模块内**）
与 `crates/company-runtime/src/opc/vendors.rs:185`（`opc` **顶层**）。
**不产生歧义**：glob 只提升直接 pub 项，子模块内的同名项藏在 `opc::analysis_schema::` 下。
⭐ 判据：判断两模块能否并入同一 glob 命名空间，必须区分「顶层项」与「子模块内的同名项」。

### 12.4 全绿的真实规模：23 处，不是一个补丁

超基线 8 行背后的完整越界清单是 **19 处 sib + 4 处 db**（`--list` 实测）：

| 组 | 处数 | 消除手段 | 难度 |
|---|---|---|---|
| `domain_pack` 模块相关 sib | 8 | M2 拆切下沉（见 12.3） | 中（已完成方案） |
| 两个协议常量 | 2 | **M1 已落地** | 低 |
| `upsert_template`（`opc_workflows/mod.rs:546`） | 1 | 下沉 dao / analysis-engine | 中 |
| `declared_kpi_thresholds`（`runtime.rs:93`） | 1 | 下沉 analysis-engine | 中 |
| `load_analysis_config`（`opc_data.rs:17`） | 1 | 薄包装，直接引 analysis-engine | 低 |
| `opc_execute_workflow` / `get_domain_pack_learning_config` / `load_rl_config`（`opc_capability_pack_actions.rs`） | 3 | **大函数**迁移，跨 crate 拆签名 | 高 |
| `run_mc_preset`（`src-tauri/src/market_sim_service.rs:222`） | 2 | 需连 `run_mc_core`+`McSimRequest`+`McScenarioSpec` 一起进 market-sim crate，**且函数体依赖 `commands::error_code` 契约** ⇒ 先解耦错误码 | 高 |
| `sim_hook.rs` 直连 DB | 3 | 需在 `axagent-dao` **新建 `stock_analyses` repository**（现状：dao 只有 migrations，零 CRUD 函数） | 中 |
| `kpi_hook.rs` 直连 DB（仅 `use sea_orm::DatabaseConnection` 做**句柄穿透**，L417/423/538 传给下游） | 1 | 需让 db 不出现在该文件签名（trait/闭包注入） | 高 |

⇒ **结论：`cargo` 门禁要转绿 = 一次完整的「命令层瘦身」重构（跨 4+ crate），
工作量以「天」计，不是单轮能收尾的补丁。** 本轮按分批推进，每批独立编译验证。

### 12.5 勘探途中发现的既有缺陷（**非本批引入，单列告知，未擅改**）

| # | 缺陷 | 证据 | 影响 |
|---|---|---|---|
| 1 | **`AnalysisDataSource` 同名不同体** | `crates/company-runtime/src/opc/capability_pack.rs:109`（YAML 解析用）vs `crates/company-runtime/src/opc/vendors.rs:185`（运行时构造用）；原宿主侧 glob 消费点已随 #264 批 7 刀 B 退役（2026-11-12），不再是第三处引用 | 一处改名/改字段，另一处静默不动 ⇒ 二者永不兼容却同享名字（判据「一名多义」族）。**分属两个 crate，当前靠路径区分，尚未爆发** |
| 2 | **`copy_dir_recursive` 三份重复实现** | `commands/opc_workflows/mod.rs:241`（`std::io::Result`）、`commands/dashboard.rs:169`（`Result<(), String>`）、`commands/skills/install.rs:1204`（`Result<(), String>`） | 同一逻辑三处维护；且**签名的 `Err` 类型不一致**（io::Error vs String）⇒ 无法直接合并，需先定统一错误类型 |
| 3 | **`opc_capability_pack_bridge.rs` 的工具 description 曾硬编码「9 个域包」** | 实测 14 个（`assert_eq!(configs.len(), 14)` @ `opc_capability_pack_actions.rs:2681` + `assert!(configs.len() >= 14)` @ `capability_pack_config.rs:1173`） | **已修**：改为不带数字（数字无信息增量，且已腐烂过一次） |

### 12.6 M2 已落地（2026-09-16 00:2x）：域包引擎下沉，门禁超基线 **8 → 4 处**

**落点（按**依赖**切分，不是按语义直觉）**：

| 切出部分 | 落点 | 依据 |
|---|---|---|
| 原 L1–617：manifest/roles schema、`scan_domain_packs`、`upsert_domain_pack_registry`、`ensure_opc_domain_packs_seeded`、`export/import_domain_pack`、`analysis_schema`、`runtime_schema` | `crates/analysis-engine/src/opc/domain_pack.rs`（670 行） | 与同目录 `domain_pack_config.rs` / `domain_pack_kpi_service.rs` / `domain_pack_validator.rs` 同族 |
| 原 L619–735：`stock_tool_defs` / `opc_tool_defs` / `local_tool_defs` | `crates/tools/src/tools/domain_pack_defs.rs`（127 行） | 依赖 `axagent_tools::tools::*`；而 tools **已依赖** analysis-engine ⇒ 反向会成环 |
| `domain_packs_base_dir` / `resolve_domain_packs_dir` / `copy_dir_recursive` | 随 A 组一并下沉 | 原在 `opc_workflows/mod.rs`；`import_domain_pack` 里的 `super::copy_dir_recursive` 是**顶层 `super::`**（搬迁即断链，判据 `#257`） |

**新增依赖**：`analysis-engine/Cargo.toml` += `serde_yaml = "0.9"` + `zip = "8"`。
★ **`sea_query::` 不需要新依赖** —— 它靠函数体内的 `use sea_orm::*;` 把 `sea_query` 带入作用域（判据 `#266`）。
★ 上一版方案判「analysis-engine 依赖不足」，是**只 grep 了 `crate::`（内部路径）而没查外部 crate**的漏检；实际 analysis-engine **已依赖** `axagent-entities` + `sea-orm` + `axagent-astock-data`，缺的只有 YAML 与 zip。

**效果**：

| 规则 | 改前 | 改后 | 基线 |
|---|---|---|---|
| `commands-no-sibling-call` | 92 处（超 6 文件） | **84 = 基线** ✅ | 84 |
| `commands-no-direct-db` | 660 处 | **655** ✅ | 656 |

超基线 **8 处 → 4 处**；M2 的 4 个目标文件（`opc_domain_pack_actions` / `opc_domain_pack_runtime` /
`opc_learning_hook` / `opc_workflow_kpi_hook` 的 sib）**全部转绿**。
剩余 4 处**全部落在 stock_workflow 域**（`sim_hook` db 3 + sib 1、`rhai_pm` sib 1、`kpi_hook` db 1），
即 §12.4 表中「中 / 高」难度项，属**独立批次**。

**保留 re-export 的取舍（明确决定，不是遗漏）**：
`opc_workflows/mod.rs` **保留** `pub use` 指向新落点（含模块名 `domain_pack` 的转发）⇒
`opc.rs`（5 处）、`opc_data.rs`（1 处）、`opc_capability_pack_actions.rs`（当时测试块内 `:2754/2756/2961`）与 `mod.rs` 自身 4 处 helper 调用
**全部零改动**。代价：`opc.rs` / `opc_data.rs` 的 6 处跨模块调用**仍是门禁命中**（均 ≤ 基线 ⇒ 不阻塞），**登记为后续批次**。

**★ 施工踩到的 4 个坑（全部由静态自检拦下，未进编译）**：

1. **可见性收窄**：`read_manifest` 原为 `pub(crate) fn`，而 `opc_learning_hook.rs:101` 要跨 crate 调它 ⇒ 必须改 `pub`。**判据 `#263`**：跨 crate 搬迁前必须逐项核对 `pub(crate)` / 私有项的**消费方是否跨 crate**。
2. **脚本产生重复 re-export（E0252）**：「块替换」已把 `export_domain_pack, import_domain_pack` 写进新六行块，脚本**又**单独替换了紧随其后的两行 ⇒ 同一符号在同模块 re-export 两次。**判据 `#264`**：块替换与逐行替换**混用**时必须先算「重叠集」。
3. **测试块内的跨模块调用漏改**：`opc_workflows/mod.rs` 的 `#[cfg(test)]` 块里有 3 处 `super::domain_pack::stock_tool_defs`（该函数已迁 tools crate）⇒ E0433。**检法**：搬迁后必须同时扫 `#[cfg(test)]` 块内的旧路径 —— **门禁不报它，只有 `--all-targets` 编译才报**。
4. **注释里的陈旧路径**（2 处）：`tools/mod.rs:363` 与 `capability_pack_kpi_service.rs:146` 仍引用旧位置 ⇒ 已更新。

### 12.7 本轮新增判据

| 编号 | 判据 |
|---|---|
| `#263` | 跨 crate 搬迁前，逐项核对被搬符号的**可见性**（`pub(crate)` 项若有跨 crate 消费方 ⇒ 必须开牌为 `pub`）。 |
| `#264` | 批量替换脚本混用「块替换 + 逐行替换」时，必须先算两者**重叠集**，否则同一符号被写两次（E0252 型重复定义）。 |
| `#265` | ★ **门禁 `driveLines` 的块过滤是 `i >= s` 而非区间**：文件里**第一个** `#[cfg(test)]` 之后的所有行**一律不计**（不是只跳过测试块区间）。⇒ 施工前必须查目标文件的测试起始行：**改在该行之后的代码，门禁计数不会变**（当时 `opc_capability_pack_actions.rs` 的 `:2959` 即因此不计）。 |
| `#266` | 判断「搬迁是否需要新依赖」不能只 grep `crate::`（内部路径）；必须逐项列**外部 crate**（`zip::` / `serde_yaml::` / `sea_query::`），其中 `sea_query::` 可被 `use sea_orm::*` 覆盖 ⇒ 无需新依赖。 |
| `#267` | ★ **cargo 结论作废的第二种检测法**：日志里**报错文件的 mtime 晚于日志 `START`** ⇒ 该轮编译对象已漂移（不止是「日志 mtime 早于改动」）。实证：cargo7 报的是它启动后才创建的新文件。 |

### 12.8 本轮新发现：英文标识符里的「行业」残留（**未擅改，登记待裁决**）

本批第③项的指令是「**.rs 注释/文案**里不再有行业概念」，故收口口径是**中文「行业」**。
本轮补扫**英文 `industr*` 标识符**（40 种形态），发现两类：

**A 类 股票业务语义（红线，一律不动）**：`get_industry_ranking`(69) / `in_industry`(27) /
`get_industry_chain_propagation`(12) / `compute_industry_position`(9) / `list_industry_chains`(7) /
`subIndustry` / `industry_screen_uses_industry_membership` 等 —— A 股行业概念，属业务语义。

**B 类 OPC 域包概念却用了英文旧名（命名债，建议单列批次）**：

| 项 | 处数 | 位置 | 说明 |
|---|---|---|---|
| `list_industries` | 8（**5 个定义**） | `analysis-engine/opc/capability_pack_config.rs:1078`、`opc/learning.rs:287`、`harness/domain_pack_orchestration/mod.rs:102`、`orchestrator/task_context.rs:263`、`commands/opc_capability_pack_runtime.rs:354` | 同名**四种返回类型**（`Vec<(String,String)>` / `Vec<&String>` / `Vec<&str>` / `Vec<String>`）⇒ 典型「一名多义」 |
| `opc_list_industries` | 4 | `register_commands.rs:815`、`opc_capability_pack_bridge.rs:33/393`、`opc_capability_pack_actions.rs:2052` | **跨前后端契约**（Tauri 命令名 + LLM 工具名）⇒ 改名须同步前端 |
| `opc_list_runtime_industries` | 已退役 | #264 批 7 刀 A（2026-11-11）三条只读包装器之一，退役注释见 `commands/opc_capability_pack_runtime.rs:369` | 调用方走 plugin-opc 同名命名命令 |
| `LegacyIndustryPathRedirect` | 2 | `src/components/layout/ContentArea.tsx:61/251` | 前端「旧路径重定向」组件（批次②清了 `opc-industries` 路由 key，组件名未改） |
| `opc_industries` / `opc_industries_pkey` / `idx_opc_industries_enabled` / `create_industries` | 4+2+2+2 | `dao/migrations/v211_opc_domain_packs.rs`、`v232_opc_domain_pack_rename.rs` | **历史迁移里的旧表名/约束名**（叙述改名本身）⇒ **保护名单**，不动 |
| `domain_pack_pack_seed_registers_industries` | 1 | `opc_workflows/mod.rs:790` | 测试函数名 |

⇒ **审计口径缺口自陈**：第③项收口用的扫描词是**中文「行业」**，**未含英文 `industr*`**
⇒ 当时报的「438→420 行 / 78→71 文件」只覆盖中文注释/文案，**符合指令范围但不等于全量**。

### 12.9 M2 收口：E0255 根因与 re-export 逐项存活判定（2026-09-16）

#### 12.9.1 门禁成果（实测对照）

| 指标 | 改动前 | 改动后 | 基线 |
|---|---|---|---|
| `commands-no-sibling-call` | 92 | **84** | 84 ⇒ 已回基线 ✅ |
| `commands-no-direct-db` | 660 | **655** | 656 ⇒ 低于基线 ✅ |
| 超基线处数 | 8 | **4** | — |

M2 的四个目标文件（`opc_domain_pack_actions` / `opc_domain_pack_runtime` / `opc_learning_hook` / `opc_workflow_kpi_hook`）的 sib 调用**全部转绿**
⇒ 剩余 4 处**全在 stock_workflow 域**（`sim_hook.rs` db 3 + sib 1、`rhai_pm.rs` sib 1、`kpi_hook.rs` db 1）= §12.4 的中/高难度项。

#### 12.9.2 编译闭环：第一轮 `cargo9` 报 3 个 E0255（根因 = 「**复制式搬迁但源未删**」）

三个连带函数（`domain_packs_base_dir` / `resolve_domain_packs_dir` / `copy_dir_recursive`）被**照搬**进新
`analysis-engine/src/opc/domain_pack.rs`，而 `commands/opc_workflows/mod.rs` 的**原定义未删**
⇒ 源侧定义**遮蔽**了 re-export ⇒ `E0255 the name ... is defined multiple times`（报在源文件 L79/217/243）。

★ 关键教训：**静态自检（逐符号普查）查不出这一类错误** —— 普查只会看到「同一符号有两个定义」，
而两个定义各自都合法时它不报错。⇒ 这类错误的唯一检法是**落盘前的双向断言**（见 #270）。

#### 12.9.3 处置（`tmp-b4g-m2d-fix.mjs`，20+ 断言，失败即中止不落盘）

1. 按「**doc 首行 → 闭合 `}`**」定位三个区间（A `L78-81` / B `L206-235` / C `L242-256`），每区间断言：
   ①「doc 与 fn 之间**无非 doc 行**」（防误吞中间函数）②「**前一行必为空行**」（防粘连）
   ③「尾随空行存在」（防 rustfmt `blank_lines_upper_bound = 1` 违约）
2. 删三个定义 + 逐个吃掉尾随空行（避免连续空行被 `cargo fmt --check` 判 diff）
3. **re-export 逐项存活判定**（见 12.9.4）
4. 落盘前断言：三函数定义消失 · `copy_dir_incremental` / `OPC_CONFIG_DIR` / `find_repo_config_dir` /
   `ensure_opc_config_synced` **未被误删** · mod.rs 内三处**裸名**调用仍在（L385/327/341）· test 块内容仍在 ·
   **`fn` 行数净 −3**

⇒ `cargo fmt --all -- --check` **EXIT=0**（期间被 rustfmt 拦下 1 处：手写的 use 换行不符合期望形态，
`import_domain_pack` 须另起一行 —— 超宽行的手工折行**必须**与 rustfmt 期望一致）。

#### 12.9.4 ★ re-export 逐项存活判定（依据：`lib.rs:14` 是 `mod commands;` **私有**）

`pub use` 在私有模块内等价 `pub(crate) use` ⇒ rustc **逐项**报 `unused_imports`。故每一项都必须有 crate 内使用者：

| 项 | crate 内使用者 | 判定 |
|---|---|---|
| `DOMAIN_PACKS_DIR` | `opc_capability_pack_actions.rs`（当时 `:2756/2767`，test 区，`--all-targets` 会编） | 保留 |
| `DomainPackManifest` | `opc.rs:1167` | 保留 |
| `copy_dir_recursive` | `mod.rs:327/341` | 保留 |
| `export_domain_pack` / `import_domain_pack` | `mod.rs:853/860`（test） | 保留 |
| `resolve_domain_packs_dir` | `mod.rs:385` | 保留 |
| `DomainPackAnalysisConfig` | `opc_data.rs:12` | 保留 |
| `LEGACY_DOMAIN_PACKS_DIR` | **仅 `mod.rs:223`（待删函数体内）** | **删项** |
| `domain_packs_base_dir` | **仅 `mod.rs:234`（待删函数体内）** | **删项** |
| `local_tool_defs` / `opc_tool_defs` / `stock_tool_defs` | **crate 内 0 处**（`opc_capability_pack_actions.rs:2588` 走函数内直接路径、test 走全路径） | **整行删** |

⇒ 被删的两项仍经 `axagent_analysis_engine::opc::*`（`opc/mod.rs` 的 glob）**完全可达** ⇒ **能力面无损失**。

#### 12.9.5 全项目逐符号普查（`tmp-b4g-census.mjs`，**2070 个 `.rs`**）

20 个符号 × 双落点存在性 + 旧文件已删。结果 `stale = 22` —— **全部为假阳性**：

- `super::domain_pack::` **5 处全在 `#[cfg(test)]` 块内**（`super` = `opc_workflows`，用 re-export 的模块名 ⇒ 合法）
- `crate::commands::.*domain_pack` **17 处里 12 处是正则过宽**（`opc_domain_pack_actions` / `_bridge` / `_runtime` /
  `constants::domain_pack` 等**无关文件**）；真 5 处（`opc.rs:632/634/647/1154`、当时 `opc_capability_pack_actions.rs`
  内的一处 stale 位点——其行号已随 #264 刀 C 批 3 七命令迁出（文件缩至 2794 行）失效，行号不具引）走 re-export
  **合法**（即 §12.4 已登记的后续批次，且 ≤ 基线不阻塞）

#### 12.9.6 新增判据

| 编号 | 判据 |
|---|---|
| `#270` | ★ **复制式搬迁必须双向断言「新位有定义 + 旧位无定义」**。只断言①的脚本对 E0255 完全无感；并把「搬迁净减少量」写成断言（本次 `fn` 须恰好 −3）。★ 派生：搬迁后**必须走一次真实编译**。 |
| `#271` | ★ **私有模块内 `pub use` ≡ `pub(crate) use` ⇒ 逐项判 `unused_imports`**。收敛 re-export 面时不得以「它是 pub」当免检理由（先查 `lib.rs` 有无 `pub`）；删项前须以**全量逐符号普查**确认无其它 workspace crate 使用。 |

#### 12.9.7 编译闭环结果 + ★「并发会话」事实取证

**`cargo10` 通过**：`cargo check -p axagent --all-targets` → `Finished dev profile [unoptimized + debuginfo] target(s) in 11m 18s`，
**error / warning 行数 = 0** ✅
时序自证：`LAST_RS_BEFORE 16:30:07`（= 我最后一次 `.rs` 改动）< `START 16:30:23` ✅

**★ 但同一项目有并发会话在改一大片文件**（判据：编译窗口内 **9 个非本批文件**被写入，其中 3 处在 **30 毫秒内**连续写入 = 批量脚本）：

| 时间(UTC) | 文件 | 说明 |
|---|---|---|
| 16:34:17 | `commands/stock_workflow/rhai_pm.rs` | **正是 §12.4 登记的后续批次** |
| 16:34:32 | `crates/dao/src/repo.rs`、`src/lib.rs`、`dao/src/repo/stock_analysis_snapshot.rs` | — |
| 16:34:53 | `commands/stock_workflow/sim_hook.rs` | **正是 §12.4 登记的后续批次** |
| 16:35:10 | `commands/error_code.rs`、**`commands/opc_workflow_kpi_hook.rs`**、`commands/stock_analysis_setup/seed_stock_analysis.rs` | 后两者毫秒级连写 ⇒ 批量脚本 |
| 16:36:04 | `market_sim_service.rs`、`commands/market_sim.rs` | — |
| 16:43:19 | `src/register_commands.rs` | **编译结束后** |

**★ 归因修正（重要）**：门禁当前数字 **`sib 82` / `db 651`** 中，`sib 84→82`、`db 655→651` 的**增量归并发会话**，
**不能计作 M2 成果** —— 判据：门禁 R2 的判定是「**调用路径文本**」
（脚本原文：「`src/commands/**` 不得跨模块调 `crate::commands::<别的模块>::`」，自测 `R2+ 跨模块命中` 即按路径判），
删三个函数**定义**不改变任何调用路径的命中 ⇒ **M2 的实测成果就是 `sib 84` / `db 655`**。

**本批改动未被覆盖（已核实）**：并发会话写入 `opc_workflow_kpi_hook.rs` 后**重跑全项目普查** ⇒
我的 5 个目标文件的新路径**全部存活**（`runtime:18` / `learning_hook:64` / `kpi_hook:430` / `actions:2522` + `2595`；
`kpi_hook` 行号 426→430 说明对方在文件前部插了行），`stale = 22` 与改动前**完全一致**（全为假阳性）
⇒ **M2 成果完好**。

⇒ 因此 cargo10 的绿灯**同时覆盖并发会话 16:36 前的改动**（若其引入错误会被报出），
但 **16:43 的 `register_commands.rs` 之后当前树未被验证** —— 不属本批，不替对方断言，也不重跑白烧 11 分钟。

#### 12.9.8 仍未做（**2026-09-16 00:55 逐项实测复核**，均未获批）

> 首版登记（「`opc.rs`(4) / `opc_data.rs` / `opc_capability_pack_actions.rs`(2)」共 7 处）**已过时**，实测为 **11 处**。

**① M2 同链遗留：跨模块调用（实测 11 处全在，仍走 re-export）**

| 文件 | 行 | 走 re-export 的符号 |
|---|---|---|
| `commands/opc.rs` | 632 / 634 / 647 / 1154 / 1167 | `resolve_domain_packs_dir` · `export_domain_pack` · `import_domain_pack` · `DomainPackManifest` |
| `commands/opc_data.rs` | 12 | `DomainPackAnalysisConfig` |
| `commands/opc_capability_pack_actions.rs` | 2272 / 2609 / 2756 / 2766–2767 / 2961 | `declared_kpi_thresholds` · `upsert_template` · `DOMAIN_PACKS_DIR` · `find_repo_config_dir` · `resolve_domain_packs_dir` |

⇒ 命中数均 ≤ 基线，**不阻塞门禁**；彻底消 sib 须逐处改走 `axagent_analysis_engine::opc::…` / `axagent_tools::…`。

**② `upsert_template` 下沉**（`opc_capability_pack_actions.rs:2421`，在测试块之外）· **③ `declared_kpi_thresholds`**（`opc_capability_pack_actions.rs:2038` ← `opc_capability_pack_runtime.rs`）

**④ stock_workflow 域 4 处 —— ★ 已被并发会话做掉（待对方确认后关闭本批次）**
实测：`sim_hook.rs` 仅剩 `crate::commands::error::` / `error_code::`（二者均在门禁 `SHARED_INFRA_MODULES` 豁免名单内）⇒ **合规**；
`rhai_pm.rs` 命中 **0**；`stock_workflow/kpi_hook.rs` **文件已不存在**（对应分支的实际落点在 `stock_workflow/hooks.rs`）。

**⑤ 英文 `industr*` B 类命名债（实测全在，未动）**
- `opc_list_industries` — `opc_capability_pack_actions.rs:2052`（Tauri 命令）+ `opc_capability_pack_bridge.rs:35 / 395`（LLM 工具名）+ `register_commands.rs:821`（**build.rs 生成物**）⇒ **跨前后端契约，改名须三处同步 + 前端 i18n**
- `opc_list_runtime_industries` — `runtime.rs:510` + `register_commands.rs:839`
- `list_industries` **5 个定义 / 4 种返回类型**：`capability_pack_config.rs:1078` `Vec<(String,String)>` · `src-tauri/crates/analysis-engine/src/opc/learning.rs:287` `Vec<&String>` · `domain_pack_orchestration/mod.rs:102` `Vec<&str>` · `task_context.rs:263` `Vec<String>` · `runtime.rs:401`（纯转发）
- `LegacyIndustryPathRedirect` — `ContentArea.tsx:59` / `:304`（路由 `/opc/industry/:packId` 的兼容重定向）

**⑥ 重复实现债（实测）**
- `copy_dir_recursive` **5 份独立实现 / 3 种返回类型**：`company-runtime/src/opc/capability_pack.rs:819`(`io::Result`) · `migration/migration/mod.rs:89`(`Result<(),String>`) · `tools/tools/skill/mod.rs:1695`(`Result<(),ToolError>`) · `commands/dashboard.rs:169` · `commands/skills/install.rs:1204`
- `AnalysisDataSource` **同名两份**：`opc/capability_pack.rs:221`（嵌套模块内）与 `opc/vendors.rs:185`（`pub`）—— 值域是否一致**未核**

**⑦ ★ 新发现：`config/opc/` 三套目录并存（需裁决）**

| 目录 | 项数 | 性质 |
|---|---|---|
| `config/opc/domains/` | 17 | 与 `opc_setup/domain_experts.rs` 的 `include_str!` 对应 ⇒ **活的** |
| `config/opc/industries/` | 14（含旧名 `industry_consulting`） | `LEGACY_DOMAIN_PACKS_DIR` 的**迁移源** ⇒ 迁移完成后是否清理，**未裁决** |
| `config/opc/domain_packs/` | 14 | 现行 `DOMAIN_PACKS_DIR` |

`agency_experts/opc/` 同样是 `domains/` + `domain_packs/` 两套并存（upstream 即如此）。

**⑧ 已排除（实证非缺陷，勿再当问题查）**
`config/opc/domain_packs/` 14 个域 vs `agency_experts/opc/domain_packs/` 12 个（缺 `ai_research` / `content_media`）——
**upstream 本来就如此**（`.worktrees/upstream`：`config/opc/industries/` **14** 项 vs `agency_experts/opc/industries/` **12** 项），
且 `opc_setup/domain_pack_experts.rs`（`include_str!` 注册表，417 行）覆盖的域**恰为同 12 个** ⇒ 那 2 个域的 agent 是**根级共享 agent**（`agency_experts/opc/ai-*.md` / `ceo-*.md` 等），**非漏搬**。

**⑨ 时效声明**：并发会话于 `00:48:51` 改写 `market_sim_service.rs` / `commands/market_sim.rs` 等**源码**（非本批）⇒
`cargo10`（结束于 `00:41`）的绿灯**对当前树已失效**；重启任何 Rust 批次前**须重新编译**。
（注：`register_commands.rs` 的 `00:49:05` 改动是 **build.rs 生成物**（文件头 `Auto-generated by build.rs — DO NOT EDIT`），属编译副作用，**不计作源码改动**，但它是「当前树上有一次编译在跑」的旁证。）

---

### 12.10 b2 批次：跨模块调用改走 crate 层路径（2026-09-16）

**触发**：用户裁定「②」。**范围先修正** —— 首版报「11 处」**不准**，全项目递归实测为 **14 处**（口径缺口三型 → 判据 **#278**）。

#### 12.10.1 实测全貌（14 处）

| # | 文件 | 行 | 目标模块 | 门禁计入 | 本批处置 |
|---|---|---|---|---|---|
| 1 | `commands/opc.rs` | 632 | `opc_workflows` | ✅ | **已改**（短名） |
| 2 | `commands/opc.rs` | 634 | `opc_workflows` | ✅ | **已改**（短名） |
| 3 | `commands/opc.rs` | 647 | `opc_workflows` | ✅ | **已改**（短名） |
| 4 | `commands/opc.rs` | 1154 | `opc_workflows` | ✅ | **已改**（短名） |
| 5 | `commands/opc.rs` | 1167 | `opc_workflows` | ✅ | **已改**（短名） |
| 6 | `commands/opc_data.rs` | 12 | `opc_workflows` | ✅ | **已改**（use 行） |
| 7 | `commands/opc_capability_pack_actions.rs` | 2272 | **`opc_domain_pack_runtime`** | ✅ | ⏸ 需先下沉 `declared_kpi_thresholds` |
| 8 | 同上 | 2609 | `opc_workflows` | ✅ | ⏸ 需先下沉 `upsert_template` |
| 9 | 同上 | 2756 | `opc_workflows` | ❌ 测试块（`cfg(test)` @ **2647**） | **已改**（全路径） |
| 10 | 同上 | 2767 | `opc_workflows` | ❌ 测试块 | **已改**（全路径） |
| 11 | 同上 | 2766 | `opc_workflows` | ❌ 测试块 | ⏸ 需先下沉 `find_repo_config_dir` |
| 12 | 同上 | 2961 | `opc_workflows` | ❌ 测试块 | **已改**（全路径） |
| 13 | **`commands/opc_capability_pack_runtime.rs`** | **624** | **`opc_data`** | ✅ | ⏸ 需先下沉 `load_analysis_config` |
| 14 | 同上 | **718** | **`opc_domain_pack_actions`** | ✅ | ⏸ 需先下沉 / 抽 service `opc_execute_workflow` |

**本批已改 9 处**（6 处计入 + 3 处测试块）**+ 1 处硬编码→常量**；**5 处待下沉**未动。

#### 12.10.2 本批落盘内容（`tmp-b4h-b2a.mjs`，**事务式**：全断言通过才写盘）

`commands/opc.rs`：
- 新增显式子路径 use（**不用 glob**，规避 `pub use domain_pack::*` × `pub use domain_pack_config::*` 的潜在歧义）：
  ```rust
  use axagent_analysis_engine::opc::domain_pack::{
      DOMAIN_PACKS_DIR, DomainPackManifest, export_domain_pack, import_domain_pack,
      resolve_domain_packs_dir,
  };
  ```
- 5 处调用 → 短名（`resolve_domain_packs_dir` ×2 / `export_domain_pack` / `import_domain_pack` / `DomainPackManifest`）
- **硬编码修正**：`app_dir.join("config/opc/domain_packs")` → `app_dir.join(DOMAIN_PACKS_DIR)`（L1155）
- `let Ok(manifest) = …` 的 **let-else 由 3 行合并为 1 行**（缩短后 87 列 < 100 ⇒ rustfmt 期望形态，主动照写以免 `fmt --check` 失败）
- `import_domain_pack(…)` 调用由 6 行合并为 1 行（同上，48 → 74 列 → 由 fmt 日志反查得出）

`commands/opc_data.rs`：use 行 → `axagent_analysis_engine::opc::domain_pack::analysis_schema::DomainPackAnalysisConfig`

`commands/opc_capability_pack_actions.rs`：3 处（均 `#[cfg(test)]` 内）→ 全路径。

#### 12.10.3 静态闭环自检（`tmp-b4h-b2-verify.mjs`，**编译不可得时的有效替代**）

判据 **#69** 落地：**递归全项目 1983 个 `.rs`**（排除 `target` / `.worktrees` / `backup-*` / `output`）逐符号普查——

| 检查项 | 结果 |
|---|---|
| `opc.rs` / `opc_data.rs` 的 `opc_workflows::` 残留 | **0** ✅ |
| 新增 use 的 5 个符号是否都被使用（`unused_imports` 风险） | 使用 1/1/1/1/2 ⇒ **全部 > 0** ✅ |
| 目标符号在 analysis-engine 的 `pub` 可见性 | 6/6 ✅（`DOMAIN_PACKS_DIR`:21 · `DomainPackManifest`:37 · `analysis_schema`:`pub mod`:125 · `export`:475 · `import`:526 · `resolve`:635 · `DomainPackAnalysisConfig`:140） |
| 全项目还有谁经该门面引用 | `opc_capability_pack_actions.rs`（当时 `:2609/2766`，待下沉）+ **`init/services.rs:3287`**、**`init/state.rs:1087/1089`**（**不在门禁扫描面**） |

#### 12.10.4 门禁实测

| 指标 | 之前 | **本批后** | 基线 |
|---|---|---|---|
| `commands-no-sibling-call` | 82 | **76**（-6，符合预测） | 84 |
| `commands-no-direct-db` | 651 | 651（未动 db 层，符合预期） | 656 |

「5 个文件已低于基线」（`opc.rs` / `opc_data.rs` 新入列）。

#### 12.10.5 5 个待下沉函数的定位与依赖（**架构决策，未擅动**）

| 函数 | 定义位置 | 可见性 | 依赖特征 | 建议落点 |
|---|---|---|---|---|
| `load_analysis_config` | `opc_data.rs:17` | `pub fn` | 读 `analysis.yaml` + `serde_yaml` | **analysis-engine**（零新增依赖，与 `read_manifest` 同类） |
| `declared_kpi_thresholds` | `runtime.rs:93` | `pub(crate) fn` | 读域包 `runtime.yaml` 声明阈值 | **analysis-engine**（须先核 runtime schema 落点） |
| `find_repo_config_dir` | `opc_workflows/mod.rs:246` | `pub fn` | 纯 `fs` / `path` | analysis-engine 或公共 utils |
| `upsert_template` | `opc_workflows/mod.rs:546` | `pub(crate) async fn` | **DB 写 `workflow_templates`** | **dao 层**（不宜落 analysis-engine） |
| `opc_execute_workflow` | `opc_capability_pack_actions.rs:2361` | ~~`pub async fn`（**Tauri 命令**）~~ 已于 #264 刀 C 批 2 退役（该行是退役注释；内核 `execute_workflow_core` 现为 `:2248`） | 命令体 | **须抽 service**：命令变薄包装 ⇒ 属重构，非搬迁 |

⇒ 后 2 项（`upsert_template` / `opc_execute_workflow`）**超出「机械替换」范畴**，待获批后单独立项。

#### 12.10.6 ★ 本批暴露的「审计口径缺口」（已入判据 #278/#279）

1. **模块名缺口**：只扫 `opc_workflows::` ⇒ 漏 `opc_domain_pack_runtime::`（`opc_capability_pack_actions.rs:2259`）。
2. **文件面缺口**：只扫「我以为相关的 3 个文件」⇒ 漏整个 `opc_capability_pack_runtime.rs`（`624/718` 两处计入）。
3. **扫描面 ≠ 影响面**：门禁只覆盖 `src/commands/**`，而 `init/**` 同样经该门面引用。
4. **全量 fmt 退出码不是个人判据**：`fmt --all -- --check` `EXIT=1`，4 处偏差**全在并发会话的 dao crate**，我改的 3 个文件 **0 处** ⇒ 必须按文件归属过滤（判据 #279）。

#### 12.10.7 时效声明

编译验证 `tmp-b4h-b2-cargo1.log` 启动时 `LAST_RS_BEFORE 17:00:15` 指向 **`semantic_cache/mod.rs`（并发会话的改动）** ⇒ 本轮编译结论**混合双方改动**，报错时必须先按文件归属分辨（判据 #268 同族）。

---

### 12.11 ① 门禁基线重算（2026-09-16 01:0x 执行）

#### 12.11.1 执行方式（可回滚 + 逐项归因，判据 #281）

**前置事实（决定做法）**：

1. **基线文件不是纯脚本产物**：`scripts/layer-discipline-allowlist.json` 的 `generatedAt` 写着 `2026-09-13`，但文件 mtime = `2026-09-15T14:58:25Z` ⇒ **被手工编辑过**（脚本写入会同步更新 `generatedAt`）。
2. **`--update-baseline` 是「全量重写」**（`main()` L544-562）：`out.rules[id] = { total, files: 当前 byFile }`，且 `files` **只含当前有违规的文件** ⇒ 它会把当前树的所有数字**一次性锁死**，包括并发会话的中间态。
3. **拿不到「当前每文件计数」而不落盘**：门禁只 export 检测器原语（`driveLines`/`isCommentLine`/`ownCommandModule`…），`buildContext`/`scanAll`/`loadBaseline` **未 export** ⇒ 只能靠 `--update-baseline` 的副作用 + 备份回滚。

**执行脚本** `output/tmp-b4i-baseline.mjs`（自带 ① 备份 ② 重算 ③ diff ④ 断言「无抬高」）：

| 步骤 | 内容 |
|---|---|
| 备份 | `scripts/layer-discipline-allowlist.json` → `output/backup-2026-09-16-m2b/layer-discipline-allowlist.json.bak`（10729 B） |
| 重算 | 调 `--update-baseline`（写文件） |
| diff | 逐规则比 `total`；逐文件分「新增 / 移除 / 计数变化」三类 |
| 断言 | **任一规则 total 上升 ⇒ 标红**（棘轮「只减不增」被违反） |

#### 12.11.2 diff 结果：**全部持平或下调，无抬高** ✅

| 规则 | total | 逐文件构成 |
|---|---|---|
| `commands-no-direct-db` | 656 → **651**（−5） | **唯一变化** = `opc_workflows/domain_pack.rs` 从基线移除（旧 5，**文件已删**） |
| `commands-no-sibling-call` | 84 → **76**（−8） | 移除 `opc.rs`(旧 5) + `opc_data.rs`(旧 1)；计数变化 `opc_capability_pack_runtime.rs` 3→2 · `opc_learning_hook.rs` 3→2 |
| `harness-fanout-zero` / `crate-has-consumer` / `crate-dir-not-orphan` | 0 → 0 | 不变 |
| `stores-domain-no-feature` | 13 → 13 | 不变 |

新基线 `generatedAt=2026-09-15`（脚本按 **UTC** 写日期，东八区 00:00–08:00 生成会与本地日期差一天 ⇒ **登记为工具瑕疵，未擅改**）。

#### 12.11.3 ★ 归因分账（并修正 §12.9.8 的一处过宽归因）

−8 与 −5 可**逐项分账到具体改动**，与 b2 报的「82→76 = −6」**独立吻合**：

| 贡献方 | sib | db | 依据 |
|---|---|---|---|
| **本会话 M2** | — | **−5** | 唯一变化是「文件已删」`domain_pack.rs`（搬去 `crates/analysis-engine` ⇒ 脱离扫描面 `src/commands/**`） |
| **本会话 b2** | **−6** | — | `opc.rs` −5 + `opc_data.rs` −1 = 与预测的 6 处计入**完全一致** |
| **并发会话** | **−2** | — | `runtime.rs` −1 + `learning_hook.rs` −1 |

★ **修正**：上一轮我据「时间窗口内对方改了文件」把 db 的 −5 **整笔归给并发会话** —— 实测 **100% 是 M2 删文件**造成的。⇒ **归因必须逐项 diff 到文件级，时间窗口只是召回线索、不是证据**（判据 #282）。

#### 12.11.4 验证闭环

- 复跑门禁：**651 = 651 / 76 = 76**，`✅ 无超基线违规`，且**「低于基线」提示消失**（此前 5 个文件）⇒ 棘轮重新咬合
- 跑 `--selftest`：**51 passed / 0 failed** ⇒ 「不报」是真实「无违规」，不是检测器失灵（避免「统计量恒等 ⇒ 先怀疑测量工具」那类误判）
- 备份可回滚：`cp output/backup-2026-09-16-m2b/layer-discipline-allowlist.json.bak scripts/layer-discipline-allowlist.json`

#### 12.11.5 ⏸ 仍未闭环（**基线 76 的前置**）

新基线数值的有效性**依赖 b2 编译通过**：若编译红导致 `opc.rs`/`opc_data.rs` 的替换被回滚，则 −6 不成立、基线虚低 6 ⇒ **须重跑 `--update-baseline`**。
`tmp-b4h-b2-cargo1.log`（PID 34032）全程 `Blocking waiting for file lock`；进程普查显示**多会话共编同一 target**（第三方 clippy PID 13656 持锁，另一会话 check PID 34132 亦在等待）⇒ 按纪律**不抢锁**。

#### 12.11.6 环境新事实（供后续批次判据）

- **同一项目活跃 3+ 个会话**，且**都遇到同一把锁**（互相排队）。判「进程是否自己的」只能靠 `CommandLine` + 日志路径，**不能靠进程数**。
- 日流水 `.workbuddy/memory/2026-09-16.md` 是**多会话共享追加**（已达 590 行）⇒ 追加前必须重读末尾取锚点（判据已有，实例新增）。
- 并发会话亦已用掉 RULES **#280**（也遇「注入层余量 21 B」问题）⇒ 编号必须现查现取。

---

### 12.12 b2 的编译验证与副作用收口（01:15–01:25）

#### 12.12.1 首次编译：`CARGO_EXIT=101`，但**错误全在并发会话**

```
src\semantic_cache\mod.rs:303:14: error[E0599]: no method named `into_query` found for struct `sea_orm::Select<E>`
src\semantic_cache\mod.rs:366:36: error[E0034]: multiple applicable items in scope: multiple `max` found
```

- `semantic_cache/mod.rs` = **并发会话的文件**（其日流水自陈在改它；mtime `17:00:15`，我的编译结束于 `17:15:25` 仍未变 ⇒ **对方尚未修**）
- ★ **我方零 error**，但有 3 条真 warning（12.12.2）⇒ **b2 的 9 处路径替换未引入编译错误**
- 另：`fmt --all -- --check` 报 7 处偏差，**全在对方文件**（`dao/evolution_execution_stats.rs` ×2 + `semantic_cache/mod.rs` ×5），我方文件 **0 处**

#### 12.12.2 ★★ b2 的副作用：**改消费点会反向杀死 re-export**（判据 #283）

```
opc_workflows\mod.rs:57:5: warning: unused imports: `DOMAIN_PACKS_DIR`, `DomainPackManifest`, `export_domain_pack`, and `import_domain_pack`
opc_workflows\mod.rs:61:9: warning: unused import: `...analysis_schema::DomainPackAnalysisConfig`
```

**根因**：b2 把 9 处调用从「re-export 短名」改走显式全路径 ⇒ `opc.rs:28` 成为这 5 个符号的**唯一来源** ⇒ `mod.rs` 的 re-export 项零 crate 内消费者 ⇒ `unused_imports`（`-D warnings` 下即 **error**）。
★ **M2 时做过逐项存活判定（判据 #271），但 b2 改动消费点后没有重做** —— 判定的**输入**（谁在用短名）变了，判定必须重跑。

**逐项处置**：

| re-export 项 | lib | lib test | 消费者现状 | 处置 |
|---|---|---|---|---|
| `copy_dir_recursive` | ✅ | ✅ | `mod.rs:278/292`（本文件） | 保留 |
| `resolve_domain_packs_dir` | ✅ | ✅ | `mod.rs:336`（本文件） | 保留 |
| `export_domain_pack` | ❌ | ✅ | 仅 `mod.rs:804`（**test**） | 测试补显式 use ⇒ 删 |
| `import_domain_pack` | ❌ | ✅ | 仅 `mod.rs:811`（test） | 同上 ⇒ 删 |
| `DOMAIN_PACKS_DIR` / `DomainPackManifest` | ❌ | ❌ | **零** | 删 |
| `DomainPackAnalysisConfig` | ❌ | — | **零**（`opc_data.rs:17` 用的是自己的 use） | 删整行 |

**修法**（脚本 `output/tmp-b4i-reexport-fix.mjs`，事务式，**15/15 断言通过**）：re-export 收窄为 2 项 + 测试块顶部补 `use axagent_analysis_engine::opc::domain_pack::{export_domain_pack, import_domain_pack};`（避免在测试里写超长全路径）+ 注释说明已删项及原因。备份 `output/backup-2026-09-16-m2c/`。行数 1194 → 1195（OLD 9→8 = −1，测试 use +2）。

★ **断言自身的两个 bug**（被事务式拦下，未写盘）：
1. `/pub use[^;]*\bX\b/` —— `[^;]` **匹配换行** ⇒ 从远处的 `pub use` 跨行匹配到注释里的符号 ⇒ **假阳性**。改用「逐行判 + 精确包含」。
2. 行宽检查把**既有**的 250 字符字符串字面量（含 `\n` 转义，rustfmt 无法折行）算成违规 ⇒ 改为**只查自己新增的行**。

#### 12.12.3 编译判据的自指缺陷（判据 #284）

`tmp-b4e-check.mjs` 原判据 `post.mtime <= startedAt` 把 `src/register_commands.rs`（`build.rs` **编译期生成**，头部 `Auto-generated by build.rs — DO NOT EDIT`）算作「窗口内改动」⇒ **必然 false**（我 `17:07:44` 拿到锁，它 `17:07:47` 就被写出）⇒ 每次并发编译都假报 `VALID_FOR_CURRENT_TREE=false`。
**修法**：按**文件头内容指纹**（`Auto-generated` / `DO NOT EDIT` / `@generated`）判生成物，只对窗口内变动文件读首部；输出 `WINDOW_CHANGES real=N generated=M` 并逐项列出。

#### 12.12.4 静态闭环（编译不可得时的判据）

改后重跑全项目逐符号普查（2044 `.rs`）：`mod.rs` **已从「定义处」消失**；所有短名消费者的来源均有效（`opc.rs:28` / `opc_data.rs:17` / `mod.rs:278/292/336`）⇒ 无 E0425 风险。
`fmt --all -- --check`：我方文件 **0 处**偏差。

#### 12.12.5 二次编译：**我方 warning 归零** ✅（`tmp-b4i-cargo2.log`，7m30s）

```
src\semantic_cache\mod.rs:303:14: error[E0599]: no method named `into_query` ...
src\semantic_cache\mod.rs:366:36: error[E0034]: multiple applicable items in scope: multiple `max` found
error: could not compile `axagent` (lib) due to 2 previous errors
error: could not compile `axagent` (lib test) due to 2 previous errors
```

- ★ **我方 3 条 `unused_imports` warning 全部消失**。权威口径：摘要行由首次的 `due to 2 previous errors; **2 warnings emitted**` 变为 `due to 2 previous errors`（**无 warnings 子句**）—— cargo 有 warning 时必写该子句。
- 诊断清单中**无任何我方文件**（唯一命中是日志头的 `LAST_RS_BEFORE`，不是诊断行）⇒ **M2 + b2 的路径替换零 error、零 warning**。
- 新时序判据输出：`WINDOW_CHANGES real=1 generated=1` ⇒ `REAL semantic_cache/mod.rs @17:21:19`（对方的真实改动）、`GEN register_commands.rs @17:20:44`（生成物，已排除）。
  ⇒ 本次 `VALID_FOR_CURRENT_TREE=false` 是**真作废**（对方在窗口内改了树），**不再自指**。
  ⚠ 但不影响我方判据：我的文件（`opc_workflows/mod.rs` @`17:17:32`）早于 `START 17:18:37`，窗口内未变 ⇒ 「我对该文件的结论」有效。

#### 12.12.6 ⏸ 仍未闭环（诚实边界）

- **「全绿」待并发会话修完 `semantic_cache/mod.rs` 的 2 个 error**（其 `17:21:19` 的改动**未修掉**这 2 处，错误仍在）。
- 门禁复跑：`651 = 651 / 76 = 76`，无超基线、无「低于基线」⇒ re-export 收窄**不影响**基线 76 的有效性。
- 对方文件的 fmt 偏差（`dao` ×2 + `semantic_cache` ×5）**不属本批，未动**（纪律 5：只登记 + 单列告知）。

#### 12.12.7 本轮结论

| 项 | 状态 |
|---|---|
| M2 + b2 的跨 crate 路径替换 | ✅ 零 error、零 warning |
| b2 副作用（re-export 被反向杀死） | ✅ 已修（15/15 断言 + 二次编译自证） |
| 门禁基线 76 / 651 | ✅ 成立（b2 的 −6 贡献已被编译证明有效） |
| 编译判据自指 | ✅ 已修（判据 #288） |
| 「全绿」 | ⏸ 阻塞于并发会话的 `semantic_cache` |
