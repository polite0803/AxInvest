# PLAN-office-auto-provision — 建房即成队：按域包自动初始化办公室成员与场景

> 状态：active ｜ 日期：2026-09-26 ｜ 对应版本：v2.9.9


日期：2026-09-26　状态：✅ 全计划完成——阶段 1+2+3（`983e5228b`/`677c29842`/`75fef2534`/`351f254d9`）、阶段 4-②（并入 `82b59c95c`）、4-①（`a613efd74`）、4-③（`6d30149bd`）。全部门禁绿。唯一遗留：`tauri dev` 桌面金路手测（播种/注入/YAML 场景/多语言名一次看全）

> 4-① 落地方式：一次性 vitest 生成器把 9 个 TS 模板逐字段 dump 为域包 `office_scene.yaml`（幂等拒覆盖、跑后即删），再删 TS 版——`sceneTemplates.ts` 1080→320 行，内置仅剩 2 个通用场景；拦路石以「注册表订阅」拆除（`registerSceneTemplate` 通知 + `useSceneTemplates(useSyncExternalStore)`，快照取版本号），4 个消费组件全部改订阅读取。
> 4-③ 落地方式：显示层解析不动 DB——`expertNames.ts` 由 `agentSlug=opc-<key>` 推导 `office.experts.<key>`（45 键×11 语言），命中用译文、否则回退 DB displayName；接入 AgentCard/DM 头/Token 列/移除确认 4 个展示位；门禁新增 rule f（Rust 名册 ⇔ office.experts 双向对账 ×11 语言，加专家忘翻译 / 删专家留键都红灯）。

> 阶段 3 落地补记：manifest `office.seed_members` 整表覆盖名册 + `room` 回传；`list_office_scene_templates` 扫 `office_scene.yaml`，前端启动注入门（main.tsx）；门禁扩展 rule e（YAML slug=域包 id、seed 房间∈场景、YAML 房间并入 office.room 11 语言判据）。engine 存量测试因新字段 E0063 已补 `office: None`。真机 `tauri dev` 金路手测仍未做（浏览器 mock 覆盖不到播种链路与 YAML 注入）。

## 0. 问题定义（两个用户诉求）

1. **创建房间时自动创建/绑定本体域（OPC 行业域包）所属智能体**——现状要一个个手工加；
2. **域包增加时自动获得房间模板 + 所属智能体**——现状两者都要改代码。

## 1. 现状事实（调查结论，均实证）

| 事实 | 证据 |
|---|---|
| `fleet_create` 只插一行 Fleet，零成员逻辑 | `src-tauri/src/commands/fleet/mod.rs:100-125` |
| 域包专家**已经 seed 进库**：启动 `ensure_opc_company_seeded` 把 12 域包专家写入 `agency_experts` + `agent_profiles`（id=`opc-<key>`，expert_id 绑定，含工具白名单） | `src-tauri/src/commands/opc_setup/capability_pack_agents.rs:49-184,199-347` |
| 但「域包 id → 专家清单」的映射**硬编码在 Rust 常量表**（`capability_pack_experts.rs` 清单 + 逐域包手写调用），manifest.yaml 只声明 tool capabilities，无 agents 段 | `config/opc/domain_packs/accounting/manifest.yaml:10-27`；14 manifest grep `agents|expert` 零命中 |
| 专家 md frontmatter 有 `domain:` 值（seed 时写入 `active_domains`），但取值**与域包 id 不同名且混乱**（geospatial/gis/spatial、game_dev/gamedev 并存）⇒ **不能**当域包关联的权威源；权威是 `opc-<key>` id 前缀 + Rust 清单 | `capability_pack_agents.rs:58-62`；`src-tauri/agency_experts/opc/` 实测 |
| `fleet_add_member` 已存在且幂等友好：入参含 `agent_profile_id`/`room_id`，同舰队 slug 唯一性校验 | `fleet/mod.rs:206-249` |
| 成员执行期身份来源是 `agent_profile_id` 反查 profile；`agent_slug` 仅路由键 | `fleet/executor.rs:100-140` |
| 手工添加的两条 UI 都不按域过滤；`agencyRoles` 加载依赖聊天页触发（office 单独打开时下拉近乎空） | `src/components/office/AddMemberModal.tsx:54`、`OfficeTab.tsx:468-506`、`src/stores/feature/expertStore.ts:215-232` |
| 域包「零代码扩展」已有先例：`load_capability_pack_adapters` 动态扫 `domain_packs/*/learning.yaml` | `src-tauri/src/commands/opc_workflows/mod.rs:335-376` |
| 场景模板纯前端 TS 硬编码；`registerSceneTemplate` 是唯一注入点（仅投研模板用过） | `src/components/office/phaser/sceneTemplates.ts:906`、`src/main.tsx:43` |
| 阶段 3 门禁已立 slug↔域包↔i18n 数据契约 | `scripts/check-office-scene-align.mjs` |
| `opc_sync_fleet` 同步的是 OPC 组织员工，非域包专家，`agent_profile_id: None` ⇒ 不可复用 | `src-tauri/src/init/opc_store_service.rs:2334-2341` |

## 2. 目标 / 非目标

**目标**：选行业模板建房 ⇒ 成员自动就位；新增域包 ⇒ 不改 TS/Rust 代码即可拥有场景 + 团队。

**非目标**：
- 不动专家 prompt 内容与 seed 的版本语义（`TEMPLATE_VERSION` 门沿用）；
- 不做成员 display_name 多语言（现状 seed 名字即硬编码中文，本次沿用，登记为已知限制）；
- 不碰 `CapabilityDomain` 正交轴；
- 不自动创建 AgentSession/子代理——成员只是引用 profile 的 FleetMember 行。

## 3. 分阶段方案

### 阶段 1 — 查询命令：域包 → 可入房 profile 清单（后端，wiring 层）

- 新命令 `list_domain_pack_profiles { domainPackId } -> Vec<DomainPackProfile { profile_id, name, expert_key, tools }>`；
- 权威映射**不新建第二套**：从 `capability_pack_experts.rs` / `capability_pack_agents.rs` 的既有常量表提炼一个 `pub fn domain_pack_experts(pack_id) -> &'static [ExpertSpec]` 查询面（现在这些常量是逐个 seed 函数私有引用，收口成函数即可），再按 `opc-<key>` 约定拼 profile id 并回表验存在性（seed 未跑/被删时返回缺失项标记）；
- 分层：命令在 `src/commands/`（wiring），不引新依赖；
- 验证：`cargo check` + 定点 `clippy -p` + 一条 Rust 测试锁「accounting 域包返回 ≥N 条且 profile_id 均命中 DB（test pool）」。

### 阶段 2 — 建房即成队（前端编排，最小侵入）

- `CreateFleetModal` 新增开关「按域包自动配齐成员」（默认开，仅当所选模板 ∈ `SCENE_DOMAIN_SLUGS` 时显示）；
- `createFleet` 成功后：调阶段 1 命令拿清单 → 用 `resolveSceneTemplate(slug).rooms` + 既有 `distributeMembersInRoom` 计算每人 `room_id` 与初始站位归属 → 逐条 `fleet_add_member`（`agent_slug=profile_id`、`agent_profile_id=profile_id`、`display_name=name`，与 OfficeTab:504 现有约定一致；N≤6，逐条 IPC 可接受，不新建 batch 命令）；
- 失败处理：单条 `SLUG_EXISTS` 跳过继续；返回「入房 k/n」toast；
- i18n：新增 UI 文案（开关 label、toast）× 11 语言（禁区 2）；
- 验证：vitest 给编排函数（纯函数部分：清单→成员输入映射）补单测 + `npm run dev` 浏览器实测建「金融投资」房自动出现交易员/分析师/风控员并站对房间。

### 阶段 3 — 域包数据驱动化（manifest 承载，零代码扩展）

- **成员名册进 manifest**：`manifest.yaml` 新增可选段 `office.seed_members: [{expert: <key>, room: <roomId>}]`（room 显式指定优于自动均分；缺省段 ⇒ 沿用阶段 1 常量表推导，向后兼容）；后端 `scan_capability_packs` 系（implementor，已有 YAML 解析）读取该段并入 `DomainPackProfile` 查询面；
- **场景模板 YAML 化**：新文件 `domain_packs/<id>/office_scene.yaml`（结构 = `OfficeSceneTemplate` 的 JSON 镜像：canvas 尺寸 / rooms / furniture / decorations）；启动时前端经新命令 `office_scene_templates() -> Vec<serde_json::Value>`（后端扫目录、复用 `plugin-proto` 无关）逐个 `registerSceneTemplate` 注入——**注入通道用现成的 `registerSceneTemplate`，不改 OfficeScene**；
- 现有 11 个 TS 硬编码行业模板**本阶段不迁移**（防一次改两件事），登记为阶段 4 收尾项；
- 门禁扩展 `check-office-scene-align.mjs`：`office.seed_members[].room` 必须 ∈ 该域包场景模板房间（新增，硬拦）；YAML schema 解析失败 exit 3。

### 阶段 4 — 收尾（2026-09-26 执行结果）

- ✅ **4-② 5 行业补场景**（已提交 `bc82ee867`）：design / project_management / security / geospatial / game_dev 各建 `office_scene.yaml`（4-5 房间；8 个新房间名 ×11 语言，其余复用既有键）；`SCENE_DOMAIN_SLUGS` 9→14；门禁判据 d「域包无场景」由报告档**升硬拦**（豁免清零——全量覆盖后判据对象已成派生量，#147 反向适用）。自此任何新增域包缺场景 ⇒ CI 红灯。
- ⏸ **4-① TS 内置 9 行业场景迁 YAML**：**发现拦路石，刻意暂缓**——`SCENE_TEMPLATES` 是静态数组，YAML 经启动 IPC 异步 `registerSceneTemplate` 注入**不触发 React 重渲染**；直接迁走 TS 版会让创建弹窗下拉在注入完成前只剩 2 项且不自动补全（`investment_office` 今天已有同型隐性时序，只是无人依赖）。迁移前需先给注册表加最小订阅机制（事件/store 化），另立计划。
- ⏸ **4-③ 成员名多语言**：牵动 seed 侧 profile 名字入 DB 的存量迁移，维持单独立项裁决，不做。

## 4. 验收（每阶段完成时跑，全部阶段完成后跑提交前全量）

- 阶段 1：`cargo check` → 定点 `cargo clippy -p <crate> --all-targets -- -D warnings` → 定点 test；
- 阶段 2：typecheck / 全量 vitest / oxlint / dprint / `check:i18n`（新 key 11 语言）+ 浏览器实测（金路 + 「开关关闭 = 行为与今天完全一致」）；
- 阶段 3：以上全量 + Rust 全量三门禁（`cargo fmt --all --check` / `clippy --workspace --all-targets --all-features -- -D warnings` / `__TAURI_WORKSPACE__=true cargo test --workspace`，CI 口径）+ 新门禁 `check:office-align` 绿；
- 每阶段单独 commit（feat 前缀中文描述），禁止功能与门禁/重构混提。

## 5. 风险与决策点

1. **seed 时序**：`ensure_opc_company_seeded` 在 init 阶段跑，建房发生在其后 ⇒ 阶段 1 查询命令需容忍「profile 缺失」（返回 missing 标记，前端 toast 明示），不 panic 不阻塞建房。
2. **domain 字段是脏的**：`active_domains` 与域包 id 不同名（实测 30+ 种取值），**明令禁止**用它做过滤键——权威只认 `opc-<key>` 前缀 + Rust 清单（→阶段 3 manifest 名册后者的最终归宿）。计划阶段 1 的查询函数就是把这份私有默契变成公共契约。
3. **成员数 vs 房间容量**：`distributeMembersInRoom` 已有均匀分布算法（每房间 4 列），交易室 4 桌位等视觉容量是软约束，重叠可接受（精灵会走位）；阶段 3 的显式 `room` 指定是正解。
4. **`fleet.metadata.strategy` 与新机制的关系**：strategy（投研/交易/风控）继续只管 dispatcher prompt 注入，与成员入房解耦，不合并语义。
5. 阶段 3 的 YAML→前端注入需过 `#[serde(rename_all = "camelCase")]` 对齐禁区 13（`DomainPackProfile` 与场景 JSON 均 camelCase 出）。
