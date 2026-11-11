# PLAN — 仪表盘收编（v3：接线 + 清尸 + 真 React 插件贡献点）

> 状态：active ｜ 日期：2026-09-30 ｜ 对应版本：v2.9.9


> 裁定已齐：Q1 保留后端能力 / Q2 跨市场要且实际接入（挂 InvestHub tab）/
> Q3 走 ②″ 真 React 组件注册表 / DomainComponents 其余 9 组件全部接上。
> v1→v2 的三条更正见 §0；本版按裁定重排为 P0–P6，其中 **P4 带独立安全闸门**。

## 0. 实证更正（保留，防止后续会话按 v1 结论行动）

| v1 判断 | 实证 | 证据 |
|---|---|---|
| E/F 重复应合并 | **错**。跨域汇总 vs 单域包详情，字段零交集 | `commands/opc.rs:583-587` |
| F 看板 UI 是死码 | **半错**。死的只有宿主 `DomainTabLayout.tsx`（自 `0963ffaac` 从未接线）；卡片层是活契约 | `DomainTabLayout.tsx:15`、`analytics.rs:125-145`↔`types.ts:143-156` |
| CrossMarket 应删 | **错**。纵向链完整，只缺入口 | `CrossMarketPage.tsx` 无注册 |
| commands[] 执行分发未做 | **已过时**（AGENTS.md 本身有误） | `manager.rs:452`、`commands/plugin.rs:448` |

## 1. 阶段总览与状态

| 阶段 | 内容 | 风险 | 状态 |
|---|---|---|---|
| P0 | 前端清尸 4 项 | 低（纯减法） | **已完成**（门禁全绿，未提交：locale 与他人 WIP 重叠） |
| P1 | F 域包看板接线（DomainDashboard，约 8 行） | 低 | **代码完成**，门禁全绿；**剩 `tauri dev` 实机核对** |
| P2 | 跨市场接入 InvestHub tab + 数据源可达性验证 | 中 | **代码完成**（门禁全绿、无回归）；**剩 `tauri dev` 实机核对** |
| P3 | DTO 收口（`DashboardPluginInfoDto`）**已做**；信封键错配**降级为 P3b** | 低 | 已完成（见 §5） |
| P4 | ~~②″~~ → **变体 C**（iframe 沙箱资产，主 CSP 零变化） | 高 | **代码全部落地**；门禁：前端五门 0 + 新测试 4/4、plugins 无豁免 clippy 0 + 132 测试 0 失败、主 crate 无豁免 clippy `--all-targets --all-features` 0、全仓 `cargo fmt --all --check` 0；**仅剩 `tauri dev` 实机核对（§9 四条）** |
| P5 | 9 个死组件全部接上（= 域包详情页重做） | 中高，范围最大 | 待排 |
| P6 | 命名与文档收口 | 低 | 待开工 |

## 2. P0 — 前端清尸（已授权，本轮做）

1. `src/lib/dynamicUI/dashboardSchema.ts` + `src/lib/__tests__/dashboardSchema.test.ts`
2. `src/stores/feature/opcStore.ts` 整文件（`useOpcStore:120` 全项目零引用，`stores/index.ts` 亦未 re-export）
3. `src/pages/opc/domains/DomainTabLayout.tsx`
   —— 与 P5 不冲突：其内部私有 `DomainTabContent`（`:23-230`）走 `executeWorkflow` 直跑，
   无向导 / 无 `template_id` 解析 / 无设置 Modal，已被活版取代；P5 的 9 组件落点应建在**活路径**上，不复活此宿主
4. i18n 孤儿键 `rl.policyDashboard` × 11 语言

判据：`typecheck` + oxlint + `test:run` + `format` + `check:eol` + i18n parity 全绿。

## 3. P1 — F 域包看板接线（已完成）

`DomainTabContent.tsx`：`:20` 加 `import { DomainDashboard } from "./DomainComponents";`；
`:185` 起挂 `<DomainDashboard ... />`，五个 props 全部取自 `:68` 已持有但闲置的 `data.*`
（`dashboard`/`dashboardLoading`/`kpiTimeRange`/`setKpiTimeRange`/`loadDashboard`，
`useDomainData.ts:437-455` 逐项核对存在，`kpiTimeRange` 类型正是 `"7"|"30"|"90"` 与组件签名一致）。
⇒ 零新增 hook、零新增 invoke、零 i18n/后端/类型改动。

实现时相对 v3 计划加了一处判断：`DomainTabContent` 是**按 tab 挂载**的（`CapabilityPackHub.tsx:71-75`
每个 tab 一个实例），无条件渲染会让仪表盘在每个 tab 重复出现 ⇒ 用
`config.tabs?.[0]?.key === tabKey` 只在落地 tab 渲染一次（与 `CapabilityPackHub.tsx:41` 的 defaultTab 口径一致）。

同批删除 `DomainTabLayout.tsx`（原 P0 第 3 项挪来此处）—— 避免「清尸阶段把 DomainComponents 制造成孤儿文件」的中间态。
副作用：14 页「白发 IPC」转为真消费。

**待办**：`npm run tauri dev` 实机核对（域包页落地 tab 出 KPI 卡、时间窗切换重取、风控 Alert 触发态、空态文案）。

## 4. P2 — 跨市场实际接入（InvestHub tab）

1. `src/components/invest/InvestHub.tsx` 按 `:14-25` 既有 lazy-tab 模式加一 tab，
   内容直接复用 `components/stock-analysis/CrossMarketDashboard.tsx`（**不动 `pageRegistry.ts` / `ContentArea.tsx` 两张硬编码表**）
2. `pages/CrossMarketPage.tsx` 若因走 tab 而确认零引用 ⇒ 随本阶段删除（页面只是薄壳，组件保留）
3. `crossMarketStore.ts` + `stores/index.ts:102` re-export **保留**（现为真消费）
4. **前置验证**（先做再谈接入）：`get_international_stock_quote` / `get_international_stock_kline` /
   `get_benchmark_kline` / `get_forex_kline` 四命令的真实数据源可达性。
   本仓有 push2* 族被本机 RST 的前例 ⇒ 跨市场走不同源，但**不能假设可达**。
   不可达则按「结构性缺口不得在 UI 造成歧义」处理：面板显式声明取不到，不得留空冒充正常结果

## 5. P3 — DTO 收口（已完成）

**已做**：`DashboardPanel` / `DashboardPluginInfo` 两个本地 interface 从
`DashboardPluginsSettings.tsx` 移入 `src/types/index.ts` 的 Plugin System 段，
更名 `PluginDashboardPanelDto` / `DashboardPluginInfoDto`（对齐同段 `PluginSummaryDto`/`PluginManifestDto` 的 `Dto` 后缀惯例），
组件改为从 `@/types` 导入（禁区 12/13）。字段可空性按后端序列化实况修正：
`author`/`frontendEntry` 是 `string | null`（非 `?:`），`props` 恒为 `Record<string, unknown>`。
DTO 注释里记下一处不对称：`PluginManifest` 结构体无 `rename_all` ⇒ manifest 键是 snake_case
`dashboard_panels`，而投影 `DashboardPluginInfo` 有 `rename_all = "camelCase"`。

**降级为 P3b（不在本轮做）**：原计划「统一信封键 `domainPackId`→`capabilityPackId` 一处」。
实测 `"domainPackId"` 在 Rust 侧有 **19 处、跨 5 个文件**
（`opc_capability_pack_runtime.rs`、`opc_capability_pack_actions.rs`、`opc_capability_pack_bridge.rs`、
`opc_learning_hook.rs`、`crates/orchestrator/src/capability_pack_learning.rs`），
且前端 `CreateFleetModal.tsx:200` 仍以 `domainPackId` 传参 —— 这是 industry→capability 迁移的**未完成面**，
不是单点错配。只改 `:522` 一处会让 18 处与它互相矛盾 ⇒ 需独立普查 + 独立阶段，勿顺手改。

- `DashboardPluginsSettings.tsx:23` 本地重定义 `DashboardPluginInfo` ⇒ 移入 `src/types/`，
  与后端 `crates/plugins/src/types.rs:163`（serde camelCase）逐字段对齐，消费方从 `@/types` 导入
- 后端信封键错配：`opc_capability_pack_runtime.rs:529` 下发 `domainPackId`，
  前端 `domains/types.ts:201` 声明 `capabilityPackId`（今日因只读 `.dashboard` 无症状）⇒ 统一为 camelCase 一处

## 6. P4 — ②″ 真 React 组件注册表（**闸门已过，但 §6.2 前提自相矛盾，需重定**）

### 6.1 硬约束（实证，非工作量）

- `tauri.conf.json` 的 `app.security` **未启用 assetProtocol**；CSP 现值
  `script-src 'self' … 'unsafe-eval'`、`frame-src 'self'`、`object-src 'none'`
- 插件安装目录约定仅 `plugin.json` / `bin/<bin>.exe` / `source/`（`source_build.rs:17-23`），**无前端产物位置**
- `plugin-proto` 六个 op（`describe/event/emit/call_seam/shutdown/ui.action`，`lib.rs:236-252`）
  **无一可承载 UI 内容**
- 全 `src/` 无 `import()` 磁盘 JS / `eval` / `WebAssembly` 先例
- 后端对 `component_name` / `frontend_entry` **零校验**（`manager.rs:417-427`）
- `ComponentRegistry.register(entry, namespace)` / `resolve` 已支持 React 组件（`types/dynamicUI.ts:130-141`）
  ⇒ 注册表不是缺口，**代码交付通道才是**

### 6.2 为什么需要独立闸门（不得与 P0–P3 混批）

②″ 意味着**把第三方插件作者写的 JS 装进应用自身的 origin**，直接后果：
插件代码可读 `localStorage` 中的 provider API key、会话 token，可行任意网络请求。
这是**威胁模型变更**，不是功能开发。必须同时落地以下四件，缺一即视为不安全：

1. ~~不放宽主文档 CSP~~ **此条不成立，见 §6.4**。
2. **模块加载隔离**：插件模块经 Blob/module URL 动态 `import()`，
   不得进入主 bundle；失败/超时必须可见（沿用 `SkillSandboxContainer` 的 30s 超时与 unmount 清理）
3. **能力最小化**：插件面板组件拿到的 props/context 只含显式声明的 seam，
   **不得**直接访问 store、`invoke`、token。经 `sdk/rpcBridge.ts` 的权限白名单那一层
4. **安装期审查 + 来源标注**：manifest 增加前端入口的完整性（hash/签名）校验，
   面板渲染时显式标注「由插件 X 提供」（禁区：结构性缺席与外来内容不得伪装成内置结果）

> **闸门裁定（2026-09-30）**：第 1 条与第 4 条**均已批准** ——
> ① 接受启用受限 assetProtocol（scope 锁死插件安装目录，主文档 CSP 一行不放宽）；
> ② 接受「插件代码进入主 origin」的威胁模型变化，**并采纳安装期 hash/签名白名单门槛为必需项**（非可选标注）。
> 闸门解除，P4 可开工。实施时第 2、3 条（加载隔离、能力最小化）与第 4 条同批落地，不得先放宽再补。

### 6.3 交付物（闸门通过后）

宿主按 `position`(main/sidebar/header/footer) + `size` 落位于 A 用量仪表盘页；
空态与标题卡；样例插件真实声明非空 `dashboard_panels` 并渲染出内容（否则宿主只能被单测夹具驱动，不算交付）。

### 6.4 §6.2 前提 1 自相矛盾（写 PLAN 时未校验，现纠正）

PLAN v3 把「启用受限 assetProtocol」与「主文档 CSP 一行不放宽」并列成同一条前提。
**二者不能同时成立**：`import()` 受 `script-src` 管辖，只要插件代码要进入主文档的 React 树，
主文档 CSP 就必须新增一个可执行来源。assetProtocol 的 scope 限制管的是**文件可读范围**，
管不到**脚本执行许可**，它替代不了 `script-src`。三个自洽变体：

| 变体 | 主文档 CSP 变化 | 威胁模型净增 | 组件运行位置 |
|---|---|---|---|
| **A. ②″ 走 asset 源** | `script-src` += `asset: http://asset.localhost`；启 assetProtocol 且 scope 锁插件目录 | 所有已安装插件的代码进入主 origin，可读 `localStorage` 内 provider key / token | React 树内 |
| **B. ②″ 走文本取回 + eval** | **CSP 零变化**（现值已含 `'unsafe-eval'`；只需 `connect-src` 放行 asset/自定义协议） | 与 A **同等**的注入面，只是换通道；且绕过了 CSP 这一层本可提供的纵深 | React 树内 |
| **C. ②′ iframe 沙箱** | **CSP 零变化**，不开 assetProtocol | 插件代码困在 `sandbox="allow-scripts"` 的独立文档，读不到主 origin | 沙箱 iframe 内（非 React 树） |

B 比 A **严格更差**（同样的注入面，还少了 CSP 声明这一层可审计的纵深），故排除。
真正要选的是 **A vs C**。建议 C：主 origin 隔离是实打实的边界，而 A 的全部好处只是
「组件能直接用 antd theme token 与 store」，而这两样本可由宿主以 props 显式下发。

### 6.5 P4-a 已落地部分 + 一处**自指哈希**的发现（写代码时才暴露）

**已落地**（后端，未提交）：

| 位置 | 内容 |
|---|---|
| `crates/plugins/src/manager.rs` | `validate_dashboard_panels`（面板 id 非空且不重复 + `frontendEntry` 形状校验）、`panel_asset_shape_error`、扩展名白名单 `PANEL_ASSET_ALLOWED_EXTS`、大小上限 `PANEL_ASSET_MAX_BYTES=2MiB`、`read_panel_asset()`（canonicalize 包含性校验 + 走完整载入链）、新错误变体 `InvalidPanelAsset` |
| `crates/plugins/src/types.rs` | `PluginPanelAsset`（serde camelCase） |
| `src-tauri/src/commands/plugin.rs` | `plugin_read_panel_asset(pluginId, panelId)`；归类为两个码：无内容来源 → Validation，安全拒绝 → PermissionDenied |
| `src-tauri/src/commands/error_code.rs` | `PLUGIN_PANEL_ASSET_UNAVAILABLE` / `PLUGIN_PANEL_ASSET_BLOCKED` |
| 11 语言 locale | 上述两码译文已补齐（parity + `check_i18n --strict` 绿） |

**发现（纠正 PLAN v3 的 §6.2 第 4 条）**：`PluginIntegrity { algorithm, hash }` 与
`verify_integrity()` **早就存在但零调用方** —— 看起来是「只差挂上」。我挂到
`build_plugin_manifest` 上之后才验出它**挂不上去**：`hash_plugin_directory` 经
`collect_files_recursive` 遍历目录内**全部文件、不排除 manifest 自身**，
而 `integrity` 字段就写在 manifest 里 ⇒ **自指哈希，任何声明都永远无法自证**。
先前的「安装期 hash 门槛」不是没接线，而是**方法本身不成立**。

⇒ 正确锚点必须落在**被哈希目录之外**：install 成功后计算目录哈希，写入
`InstalledPluginRecord`（新增 `integrity: Option<PluginIntegrity>`，`#[serde(default)]` 向后兼容），
`read_panel_asset` / worker 启动前按记录核对。这样 manifest 本身也在被保护范围内。

**因此 §6.2 第 4 条改写为**：完整性锚点 = registry 记录（trust-on-install 钉死），
不是 manifest 自述。诚实边界：这把「防篡改」做实了，但**仍不等于防恶意作者** ——
自述哈希或本地钉死都无法阻止一个签名者发布坏内容；真正的信任根（签名公钥白名单 /
上架审核）是独立产品决策，仍未定，不得在本轮里暗示已具备。

**P4-a 未覆盖的口子（明说）**：canonicalize 逃逸分支没有测试（构造它需要符号链接或
目录 junction，Windows 上需额外权限且清理有风险）；形状校验与大小上限各有测试。

### 6.6 P4-a′：registry 钉死锚点（已落地）+ P4-b 的越权细节（未落地，动手前必读）

**锚点已按裁定移到 registry**：

| 位置 | 内容 |
|---|---|
| `plugins/src/types.rs` | `InstalledPluginRecord.integrity: Option<PluginIntegrity>` + `#[serde(default)]`（旧 registry 仍可读，读作「未钉死」） |
| `manager.rs` install / update / bundled 同步 | 三处**目录整体替换点**全部重钉哈希（update 若不重钉会把新内容判成篡改；bundled 不钉则该来源没有锚点） |
| `manager.rs read_panel_asset()` | fail-closed：无锚点 → `PluginError::IntegrityUnpinned`；不符 → `IntegrityMismatch` |
| `commands/plugin.rs` | 四种缺席分四个码归类，不再塌成两个 |

**四种缺席各占一句**（`error_code.rs` + 11 语言已补齐；parity / `check_i18n --strict` / `check-i18n-untranslated` 均 0）：
`PLUGIN_PANEL_ASSET_UNAVAILABLE`（没声明或不存在）、`_BLOCKED`（形状/扩展名/大小被拒）、
`_UNPINNED`（没建立锚点，重装即可，**内容并未被改**）、`_TAMPERED`（与钉死值不符，请移除或重装）。

> 诚实边界：锚点是 **trust-on-install**。它防的是安装后被改动，**不防恶意作者** ——
> 作者用自己钉的哈希发布坏内容一样能通过。信任根（签名公钥白名单 / 上架审核）仍是未决产品决策，
> 不得因本轮已具备哈希而暗示其存在。

**P4-b 动手前必须定下的 containment 细节（易被做成越权）**：
沙箱 iframe 的 `api.invoke` 若把插件传来的 `pluginId` 原样转发给 `plugin_ui_action` /
`plugin_execute_command`，则**一个面板可以驱动另一个插件的 worker**（跨插件越权）。
⇒ 宿主侧的 `api.invoke` 必须**忽略 iframe 传来的 pluginId、强制绑定本面板所属插件**，
并把 `permissions.commands` 锁成这两条、其余全拒。实现点是 `createHostApiBridge({ api, permissions })`
的 `api` 注入面（`sdk/rpcBridge.ts:209-236`），不需要新机制。

**复用判断（待定）**：`SkillSandboxContainer.tsx` 的 iframe 生命周期（先挂监听再赋 `srcdoc`、
30s 超时、unmount 清监听与双桥）是安全关键路径。新建一个 plugin 版宿主会复制这段 ⇒ 两份漂移风险。
倾向给该容器加一个可选的资产来源参数而非复制，实施时以「不复制安全生命周期」为准。

## 7. P5 — 域包面板接线（③④ 已落地，待实机核对）

范围实质 = **域包详情页重做**。裁定结果：**A 类接、B 类删**。

### 7.0 ①② 对账结论：无需后端补齐（契约缺口全在前端）

逐组件对账（`props` ↔ `useDomainData` 字段 ↔ 后端命令）后：A 类 6 个组件所需数据
**全部已由既有 7 条命令供出** —— `opc_get_capability_pack` /
`opc_get_capability_pack_dashboard` / `..._workflow_steps` / `..._automation_rules` /
`opc_get_learning_metrics` / `opc_get_learning_config` / `opc_execute_analysis`，
且这些命令早已被调用（数据一直在拉）。真正的缺口是**两处前端问题**：

1. **契约坏**：取数失败被吞成空数组/`null`，组件无从区分「没有数据」与「没取到」
2. **组件从不渲染**：5 个组件零引用（只有 `DomainDashboard` 被挂在内页），死代码

⇒ **② 后端补齐作废**，工作量全在 ③④（下 4 节）。

- **B 类 4 个已删**（零引用，`grep` 实证 = 0）：`DomainActionsPanel` / `DomainWorkflowsPanel` /
  `DomainWorkflowExecution` / `DomainHeader`
- **A 类 6 个已接**（`DomainComponents.tsx` 现存全部导出）：`DomainDashboard:134` /
  `DomainWorkflowSteps:227` / `DomainAutomationRules:288` / `DomainAnalysisDecision:411` /
  `DomainLearningMetrics:542` / `DomainLearningPanel:646`

### 7.1 落点：新增包级 tab（`pack_level`），不塞进业务 tab

业务 tab 键**逐域包而异**（`config.tabs` 由后端各域包自定），无公共落点 ⇒ 由
`CapabilityPackHub` 自持一个 `PACK_LEVEL_TAB_KEY = "pack_level"` 的 tab（label 走
`opc.domain.tab.runtime`，11 语言已补），内挂 `DomainPackLevelTab`。
同时修掉旧仪表盘寄居 `config.tabs?.[0]` 的搭车写法（域包无 tabs 时整个仪表盘不可达）。

### 7.2 域级数据上提到单一实例（消除重复 IPC）

`useDomainData` 从 `DomainTabContent`（每 tab 一份 ⇒ antd `Tabs` 不卸载已访 pane，访问 N 个
tab 就有 N 份并行取数）上提到 `CapabilityPackHub`，经 props 下发 `UseDomainDataReturn`。
配套修掉两处重复触发：初始化 effect 不再二次调 `loadDashboard`；该 effect 不再随
`kpiTimeRange` 重跑（它只随能力包切换）。
新增回归测试 `CapabilityPackHub.dataLoad.test.tsx`：5 条域级命令挂载后各命中**恰好 1 次**，
切业务 tab 不增，且**故意不 mock `DomainTabContent`**（mock 会掩盖回归）。
已自证有牙：临时把 `useDomainData` 塞回 `DomainTabContent` ⇒ 2 条测试转红。

### 7.3 结构性缺席：`error ≠ Empty` 全链贯通

- `useDomainData` 新增按 section 的 `errors`（`dashboard/steps/rules/metrics/learningConfig/decision`），
  每个 loader 成功清空、失败写 `translateBackendError(e)`，且**不塌成空数据**（`setX(null)`）
- 组件侧统一 `SectionError`（`Alert type="error"`）且空态渲染一律加 `error ? null :` 守卫
- **加载标志初值改 `true`**（「首帧是待答，不是答案是没有」），`decisionLoading` 保持 `false`
  （手动触发项，未点之前不处于取数中）
- `runAutomationRules` **失败改为抛出**（原实现 catch 里 `return []`，把「没跑成」伪装成
  「跑完没有规则命中」）；`DomainAutomationRules` 用 `App.useApp()` 出成功/无命中/失败三种反馈
- 新增测试 `DomainComponents.absence.test.tsx`（8 条）+ `useDomainData.honesty.test.tsx`（3 条）

### 7.4 尚未落地

- `tauri dev` 实机核对（§9 口径）
- `domainPackId`→`capabilityPackId` 一族改名（19 处 / 5 文件）待裁定
- ⚠ **既存弃用（未动，待裁定）**：`DomainTabContent.tsx:186` 的 `<Alert message={tab.description}>`
  在 antd 6.6.2 下每次渲染都打弃用警告（antd 的 Alert 弃用表把 `message` 映射到 `title`）。
  属本文件既存写法、非本轮引入；本轮只迁移了**新测试会渲染**的那几处（`DomainComponents.tsx`），
  此条同理可改 `title`，但不在 P5 授权范围内，故只登记。

## 8. P6 — 命名与文档

- `MultiAgentDashboard` → 改名（`MultiAgentPage.tsx:3/6`），名实相符
- AGENTS.md：**一律追加到文件末尾**（防 B/C 栏行号引用漂移）；更正 commands[] 缺口记载；
  补登 F 看板接线、跨市场入口、D 面板宿主与 P4 的安全前提
- `project-plugin-gap-closure` 记忆同步更新

## 9. 门禁总则（按 AGENTS.md）

- 逐阶段：`cargo check` + 定点 `cargo clippy -p <crate> --all-targets -- -D warnings`，**不跑全量**
- 前端每阶段：`typecheck`、`npm run lint:eslint`(oxlint)、`test:run`、`format`、`check:eol`
- i18n：`check-i18n-key-parity.mjs` + `check_i18n.py`（11 语言，zh-CN 为源）
- 全阶段完成后一次全量：`cargo fmt --all --check`、
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`、
  `__TAURI_WORKSPACE__=true cargo test --workspace`
- P1/P2/P4/P5 属 UI 变更：`npm run tauri dev` 实机核对，不以类型/单测绿代交
- P4 实机核对清单（四条，逐条见分晓才算过）：
  ① `.claude-plugin/plugin.json` 走 Claude Code 契约检测路径，snake_case 的 `dashboard_panels` 是否被该 loader 接受；
  ② 沙箱内 `ctx.api.invoke` 能否走到受限桥（宿主绑定 pluginId 那条）；
  ③ `example-bundled` 是 `defaultEnabled: false`，须先在设置启用；版本已 bump 0.1.0→0.2.0，
     否则 `needs_sync`（`manager.rs:1374` 判 `record.version != manifest.version`）会跳过重拷；
  ④ 「仅标题的面板」必须显示成明示无来源，不得是空白卡
- **验证方法的诚实性**：主 crate 的 clippy 被 HEAD 既存违规 `crates/trajectory/src/causal.rs:578`
  （`nonminimal_bool`）短路挡住 —— 该文件 `git status` 干净，属已提交代码，非本轮改动。
  处置是跑一次性探针：同命令对该 lint 加 `-A clippy::nonminimal_bool`（**不改文件、不放宽仓库门禁**），
  以便我自己的 crate 真被 lint 到。探针结论须标注「在临时豁免一条无关 lint 的前提下」
- 工作区有其他并发 WIP：提交按 hunk 归属切分，不与他人改动混提

## 10. 顺手修的无关 hunk（不属于仪表盘收编，独立归属）

`crates/trajectory/src/causal.rs` 的 `nonminimal_bool`：同一个式子写了**两遍** ——
生产 `:578` 与测试 `:814`，均改为 `r.is_error != outcome_ok` / `is_error != outcome_ok`
（布尔恒等 `!a == b ⟺ a != b`，无 NaN 之类边界差）。只修 578 不够：`--all-targets`
会立刻在测试 target 的 814 再红一次。
验证：该 crate 无豁免 clippy 0；测试 426 + 22 passed / 0 failed，真值表由
`tool_outcome_agreement_matrix` 锁定。

### 10.1 判据抽纯函数（已落地，闭合上述遗留观察）

原缺陷：`tool_outcome_agreement_matrix` 把生产判据式子**重写一遍**再断言，改错判据测试照样绿。
现抽出 `fn o_edge_hit(tool_errored, outcome_ok) -> bool { tool_errored != outcome_ok }`，
生产 `observe_from_trajectory` 与测试共用，被测对象即判据本身。

**有牙自证（2026-09-30 实测）**：临时把 `!=` 翻转成 `==` ⇒
`tool_outcome_agreement_matrix` 立刻 `FAILED / case false/true`（EXIT=101）；还原后绿。
用例集含 `(false,false,false)`、`(true,true,false)` 两处「翻转即变答案」的输入，
故判据任何一侧被改都必被捕获，不依赖具体翻转方向。

验证：该文件 `cargo fmt -p axagent-trajectory --check` 0 ·
`cargo clippy -p axagent-trajectory --all-targets -- -D warnings` 0 ·
`__TAURI_WORKSPACE__=true cargo test -p axagent-trajectory` 426 + 22 passed / 0 failed。
（插函数时一度吃掉 `outcome_is_positive` 的 doc 注释，已复位。）

## 11. P4 最终门禁结论（2026-09-30）

无豁免、全 target、全 feature 口径下三面实证绿：
`cargo fmt --all --check` 0 · 主 crate clippy 0 · plugins clippy 0 + 132 测试 0 失败 ·
前端 typecheck/oxlint/format:check/eol/i18n 三道全 0 · 新测试 4/4 ·
全量 vitest 184 passed（唯一红为并发会话的 `StockAnalysisConfigPanel` 种子键，断言逐字未变）。
**未做**：`tauri dev` 实机核对（§9 四条）、全 workspace 的 clippy/test（按 AGENTS.md 留到全部阶段结束）。

## 12. 提交归属实况（2026-09-30 23:30 之后）

用户令「完成后提交」时，`git status` 已是 **clean** —— 本轮 P0–P5 全部产出被并发会话那笔
`389adaa6f feat: 涨幅召回归因与全市场收盘快照采集（Phase 1/4）` 整体扫走（该提交的 stat 里
逐条含 `DomainPackLevelTab.tsx`、三个新测试文件、5 个 domains/opc 改动、11 语言 locale、
以及 `trajectory/causal.rs` 的 `o_edge_hit` hunk）。⇒ 无可提交物，未造空提交、未改写已推送历史。

HEAD 复核（clean 树上）：
- 全量 vitest 两轮均报 2–3 条红，但**逐条单跑全绿**（14/14）：
  `DemandDiscoveryPage > 手动录入` 报 `Test timed out in 15000ms`、
  `ScreenerToWorkspace > 超短线 tab 点股票` 报预填值为空 —— 两条单跑各需 5.7s / 5.4s，
  同轮 `graphAggregate.settle.test.ts` 一个文件吃 130s ⇒ 判为负载挤爆（PLAN §9 flaky 族），不改被测代码。
- 对本轮红条的**因果排除**：我加的跨市场 tab 是 `React.lazy` 且 pane 未点开 ⇒ 挂载期零渲染零 IPC，
  不构成该 InvestHub 集成测试变红的原因（仅凭「单跑绿了」不算排除）。
- `StockAnalysisConfigPanel` 那颗（此前唯一的真红，horizon 三个种子键）已在同批修掉，13/13 绿。
