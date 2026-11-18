# 插件体系

> 状态：active ｜ 日期：2026-11-04 ｜ 对应版本：v3.0.0 ｜ 功能点：plugin-system

## 定位

插件体系把「内置功能」与「外部扩展」统一成同一套装载形态：一份 `plugin.json` 清单 + 可选的**长驻 worker 子进程** + 可选的动态 UI 资产。目标是让扩展能力与内置能力走**同一注册入口、同一权限模型、同一卸载语义**，而不是给插件另开一条特权通道。

入口：`plugin_*` 命令族（安装 / 启用 / 停用 / 卸载 / 页面清单 / UI action）、设置页插件管理、动态页宿主 `DynamicPageViewer`。

## 当前行为

### 1. 长驻 worker + 长度前缀 JSON 帧

worker 是一个**独立可执行文件**，由 manifest 的 `worker.program` 声明，路径必须相对插件安装目录（`axagent-plugins/crates/plugins/src/types.rs:349-365`）。只有 `enable()` 时才 spawn，且 spawn 失败**不阻断启用**——声明式资产（hooks / tools / 技能）仍然可用，只留一条 warn（`axagent-plugins/crates/plugins/src/manager.rs:1089-1096`）。

不用 per-call spawn 的原因在代码注释里写得很直白：每次调用都要付「进程启动 + 动态链接 + 握手」的固定成本，且无法维持长连接状态（`axagent-plugins/crates/plugins/src/worker.rs:3-10`）。长驻形态下，宿主为每个 worker 起**一条专用读线程**按 `request_id` 投递响应、**一条专用写线程**独占 stdin（`:33-42`），stdout 的帧由读线程统一解析。

线格式是 `plugin-proto`（foundation crate，零 `axagent-*` 依赖）：

- 协议版本 `AXAGENT_PLUGIN_PROTO_VERSION = 1`（`axagent-core/crates/plugin-proto/src/lib.rs:77`）；
- 单帧上限 `MAX_FRAME_LEN = 64 * 1024 * 1024`（64 MiB，`:80`）；
- 4 字节大端长度前缀 + JSON 负载；写前/读前都校验上限，**先校验长度再分配内存**（`:86`、`:104-114`）；
- 畸形帧一律返回 `Err`，不 panic、不死锁（模块注释 `:3-14`）。

（对应版本：v3.0.0；worker 主干证据 11aa58772，stdin 通道修复为 unreleased 3b4c5c58c）

### 2. 星型拓扑：L1 事件桥 / L2 接缝调用

所有跨插件通信都经宿主中转，插件之间没有直连通道：

- **L1 事件桥**：插件发 `emit`，宿主代为派发领域事件；`emit` **不等待派发结果**（`axagent-plugins/crates/plugins/src/worker.rs:713-762`）。订阅侧要求声明里含 `event.dispatch`，否则不挂订阅句柄（`:5167-5183`）。注释明确「跨插件优先用 L1 事件」（`:55`）。
- **L2 接缝调用**：`call_seam` 走宿主已注册的能力接缝实现，可与内置实现或另一个插件的远程接缝同表（`:5238-5245` 说明「平权」）；请求/响应按 `request_id` 配对（`axagent-core/crates/plugin-proto/src/lib.rs:139-164`）。

防环与限深两条硬约束都在**目标端自检**：

- `call_chain` 元素是插件 ID；`check_chain` 先判断链中是否已含自己，命中即拒（`axagent-plugins/crates/plugins/src/worker.rs:3820-3825`）；
- 深度上限 `MAX_SEAM_CALL_DEPTH = 8`（`axagent-core/crates/plugin-proto/src/lib.rs:323`），入站与出站各校验一次（`axagent-plugins/crates/plugins/src/worker.rs:577-583`、`:3827-3832`）。

等待预算：接缝调用缺省 15 s（`DEFAULT_SEAM_TIMEOUT_MS = 15_000`，`axagent-core/crates/plugin-proto/src/lib.rs:358`），握手预算 10 s（`DEFAULT_HANDSHAKE_TIMEOUT_MS = 10_000`，`:478`），编译期断言握手 < 接缝（`:1735-1736`）；停机缺省 5 s（`axagent-plugins/crates/plugins/src/worker.rs:321`）。超时未收到配对响应会把该连接标记为 `seam_suspect`，后续接缝调用被拒，直到显式卸载/重载复位（`:3313`、`:3645-3646`）。

（对应版本：v3.0.0+（unreleased，2026-11-02 起接缝批量接入；证据 45221a824、ac89e1606、088d5a189））

### 3. 动态 UI 装载

插件页面在清单 `pages[]` 里逐页声明（`axagent-plugins/crates/plugins/src/types.rs:335-341`），由 `build_manifest_pages`（`axagent-plugins/crates/plugins/src/manager.rs:2760`）校验并转写；`loading` 值不在词汇表内返回 `None`，由该函数报清单校验错误（`axagent-plugins/crates/plugins/src/types.rs:237-247`）。`plugin_list_pages` 每次实时扫描已安装清单，不维护缓存副本（`:173-188`）。

Schema 存储与归属：

- 表 `dynamic_ui_schemas` 带 `origin` / `owner_id` 两列（前端类型 `src/types/dynamicUI.ts:245-263`：`origin === "plugin"` 时 `ownerId` 即 pluginId）；
- 写入路径统一经 `resolve_origin` 把关，`origin` 只接受枚举内取值（`src-tauri/src/commands/dynamic_ui.rs:94-118`）；
- 每个 schema 只保留最近 30 个版本（`src-tauri/src/commands/dynamic_ui.rs:265-273`）。

`ComponentRegistry` 用 **namespace 前缀**隔离插件组件：`register(entry, namespace)` 的键是 `${namespace}:${entry.type}`，`resolve(type, namespace)` 先查命名空间键、再回退全局键（`src/lib/dynamicUI/ComponentRegistry.ts:14-59`）。卸载时需要 `unregister` 同 namespace，并修复反向索引避免裸类型悬空（`:75-85`）。

Action 回流是单向显式链路：`DynamicPageViewer` 仅在 `record.origin === "plugin" && record.ownerId` 时把 `pluginId` 带进 `load_ok`（`src/pages/DynamicPageViewer.tsx:67-72`），`DynamicUIRenderer` 的 `onAction` 是宿主感知动作的唯一通路，直接接到 `pluginStore.runUiAction(pluginId, action)`（`:88-94`），最终落到 IPC `plugin_ui_action`（`src-tauri/src/commands/plugin.rs:495`）。渲染侧只在前端 `parseSchema` 失败时显示 `dynamicUIManager.invalidSchema`（`src/pages/DynamicPageViewer.tsx:63-65`）；后端不重复校验 JSON 结构。

（对应版本：v3.0.0+（unreleased；页面装载通道 A，证据 `CHANGELOG.md:120`、45221a824））

### 4. 生命周期：编译管线五阶段、内容寻址、卸载语义

五阶段定义在 `PLAN-everything-is-plugin.md` §12.1（`:731-739`）：**① 生成 → ② 静态审计 → ③ 隔离构建 → ④ 装载（spawn + 握手）→ ⑤ 回滚**。对应代码：

| 阶段 | 实现入口 |
|---|---|
| ② 静态审计 | `plugin_source_audit`（`src-tauri/src/commands/plugin.rs:166`）→ `axagent-plugins/crates/plugins/src/source_audit.rs` |
| ③ 隔离构建 | `plugin_source_build`（`src-tauri/src/commands/plugin.rs:193`）→ `axagent-plugins/crates/plugins/src/source_build.rs:177-207` |
| ④ 装载 | `plugin_enable`（`src-tauri/src/commands/plugin.rs:329`）→ `LoadedPlugin::load`（`axagent-plugins/crates/plugins/src/worker.rs:5063-5080`） |
| ⑤ 回滚 | `LoadedPlugin::rollback` 逐句柄 LIFO 撤销（`axagent-plugins/crates/plugins/src/worker.rs:5211-5215`） |

审计是**形式合规清单**，不是安全边界：`build.rs`、白名单外 proc-macro、git/path 依赖被列为首要禁止项，因为它们在编译期以宿主用户身份执行任意代码（`docs/plans/PLAN-everything-is-plugin.md` §12.2，`:741-758`）。

内容寻址存储：产物 hash 是 `H(源码 ‖ 协议版本 ‖ target triple ‖ rustc 版本 ‖ build flags)`（`axagent-plugins/crates/plugins/src/source_build.rs:10-11`、`:259-283`），落盘目录 `<build_root>/<plugin_id>/<hash>/`（`:18-21`）；**hash 命中即跳过编译**（`:189`）。构建用 `cargo build --offline` + 独立 `--target-dir`，构建后删掉 target-dir（`:312-330`）；但注释明确 `--offline` 只阻止 cargo 拉依赖，**不构成 OS 级网络隔离**（`:26-30`）。

卸载语义（`axagent-plugins/crates/plugins/src/worker.rs:5188-5208`）：

1. `drain` 优雅收尾：关闸拒新单，等在途帧回完，最多等 `shutdown_timeout`（缺省 5 s）；超时则 warn 并转急停；
2. `rollback` **逆序**（LIFO）撤销全部可逆注册句柄；
3. `shutdown` 停 worker（发 shutdown 帧，超时强杀）；
4. 最后 `reset_seam_suspect` 清接缝脏闸。

出现顺序即语义：先关闸再摘注册最后停进程，避免「直接掐死在途调用」的停机窗口（`:5191-5192`）。孤儿进程三道防线：worker 侧 stdin EOF 自退、宿主侧 shutdown 帧 + 超时强杀（`:3811`）、`WorkerInvoker` 的 `Drop` 兜底强杀；`try_wait` 顺带回收避免僵尸（`:3589-3600`）。

（对应版本：v3.0.0；drain 证据 129b21876，内容寻址随 11aa58772 引入）

### 5. 沙箱实况：准入检查，不是隔离

`sandbox.rs` 的定位在模块头就写明：在 hook/tool/lifecycle **子进程执行前**做 capability 检查，仅用 std 实现，不引入 WASM runtime / bubblewrap（`axagent-plugins/crates/plugins/src/sandbox.rs:2-9`）。它覆盖四类，但强度差别很大：

| 能力 | 实现 | 实际强度 |
|---|---|---|
| 路径白名单 | `check_path_permission`（`:179-209`） | 默认 `allowed_paths` 为空；`filesystem_read/write` 权限**不会**自动填充白名单（`:236-268`），因此默认只拦敏感目录黑名单 |
| ENV 过滤 | `apply_env_to_command`（`:165-172`） | `env_clear` 后仅回填白名单；**唯一在 OS 层真正生效的约束** |
| subprocess 权限 | `check_subprocess_permission`（`:214-222`） | 未声明 `subprocess_execution` 才拒绝，且只作用于 hook 脚本执行入口 |
| network 权限 | `note_network_access`（`:228-234`） | **只告警，不拦截**（OS 级隔离超出范围） |

ENV 白名单常量 `DEFAULT_ENV_WHITELIST` 仅含 `PATH` / `HOME` / `USERPROFILE` / `TEMP` / `TMP` / `LANG` / `LC_ALL` / `AXAGENT_`（`:23-24`）；`AXAGENT_` 前缀变量还要过 `is_env_allowed` 的敏感关键词二次过滤（`:20-22`，测试 `:310-325`）。调用侧把这一点写成了明确结论：ENV 白名单过滤是当前唯一在 OS 层生效的约束（`src-tauri/src/commands/plugin.rs:570-575`）。worker 进程本身以宿主用户身份运行，不受路径沙箱约束。

（对应版本：v2.8.7；插件沙箱基座证据 62e0ff02a）

## 关键代码入口

- 后端命令：
  - 列表 `src-tauri/src/commands/plugin.rs:15`
  - 校验安装源 `src-tauri/src/commands/plugin.rs:61`
  - 源码审计 `src-tauri/src/commands/plugin.rs:153`
  - 源码构建 `src-tauri/src/commands/plugin.rs:180`
  - 安装 `src-tauri/src/commands/plugin.rs:238`
  - 启用 `src-tauri/src/commands/plugin.rs:274`
  - 停用 `src-tauri/src/commands/plugin.rs:304`
  - 卸载 `src-tauri/src/commands/plugin.rs:340`
  - UI action `src-tauri/src/commands/plugin.rs:398`
  - 页面资产/页面清单 `src-tauri/src/commands/plugin.rs:483`、`:515`
  - 命令执行 `src-tauri/src/commands/plugin.rs:577`
- 引擎/业务层：
  - 帧协议 `axagent-core/crates/plugin-proto/src/lib.rs:76-130`
  - worker 调用面 `axagent-plugins/crates/plugins/src/worker.rs:3114`（spawn）、`:3618`（drain）、`:3667`（shutdown）
  - 管理器 `axagent-plugins/crates/plugins/src/manager.rs:81`（`loaded_workers`）、`:1089-1128`（启停）、`:1133-1137`（卸载）
  - 沙箱 `axagent-plugins/crates/plugins/src/sandbox.rs:23-24`、`:165-172`
- 前端：
  - `src/lib/dynamicUI/ComponentRegistry.ts:14-59`
  - `src/pages/DynamicPageViewer.tsx:60-94`
  - `src/pages/DynamicUIManagerPage.tsx`
  - `src/components/plugin/PluginPanelHost.tsx`、`src/components/plugin/pluginPageBridge.ts`
- 数据表/迁移：
  - `dynamic_ui_schemas` / `dynamic_ui_schema_versions`（实体引用 `src-tauri/src/commands/dynamic_ui.rs:12-18`）
  - 插件安装记录表由 `axagent-plugins/crates/plugins/src/manager.rs:972-1041`（install / enable）落库

## 配置与开关

- 清单开关：`PluginManifest.worker`（缺省 = 不 spawn，只贡献声明式资产，`axagent-plugins/crates/plugins/src/types.rs:349-351`）；`pages[]` / `dashboard_panels[]` 同为清单唯一权威声明（`:335-341`）。
- 权限词表：`PluginPermission::SubprocessExecution` / `NetworkAccess` 决定 sandbox 的 `allow_subprocess` / `allow_network`（`axagent-plugins/crates/plugins/src/sandbox.rs:251-267`）；权限在启用时随 worker 连接固定，不在运行期协商（`axagent-plugins/crates/plugins/src/manager.rs:1120-1122`）。
- 协议常量：版本 1、帧上限 64 MiB、最大调用链 8、接缝等待 15 s、握手等待 < 15 s、停机 5 s。
- 内容寻址根：`<build_root>/<plugin_id>/<hash>/`，hash 输入见 `axagent-plugins/crates/plugins/src/source_build.rs:18-30`、`:259-283`。
- Schema 版本保留：30 个（`src-tauri/src/commands/dynamic_ui.rs:265-273`）。
- i18n 前缀：动态页错误文案 `dynamicUIManager.*`（`src/pages/DynamicPageViewer.tsx:65`）、`dynamicUI.schemaNotFound`（`:77`）。

## 已知限制与坑

1. **沙箱不是安全边界**：`sandbox.rs` 只在少数执行入口做准入；worker 本身以宿主用户权限运行，路径/网络都没有 OS 级隔离。审计同样只是「形式合规」，`build.rs` 即可绕过（`docs/plans/PLAN-everything-is-plugin.md` §12.2，`:741-758`）。
2. **`--offline` 不等于断网**：只阻止 cargo 拉取依赖，`build.rs` 仍可外联（`axagent-plugins/crates/plugins/src/source_build.rs:26-30`）。
3. **filesystem 权限不扩张路径白名单**：`build_sandbox_from_manifest` 只映射 subprocess/network 两项；`allowed_paths` 恒为空，除非调用方显式 `with_allowed_paths`（`axagent-plugins/crates/plugins/src/sandbox.rs:236-268`、`:271-276`）。
4. **worker 起不来不影响「启用成功」**：spawn 失败只 warn，插件仍显示启用但没有任何远程能力（`axagent-plugins/crates/plugins/src/manager.rs:1090-1096`）。排查「启用了却没反应」先看这条日志。
5. **热替换是「先停后起」**：旧进程 drain 完才起新进程，中间存在接缝空窗；双进程并存切换尚未实现（`axagent-plugins/crates/plugins/src/manager.rs:1101-1105`）。
6. **接缝超时会污染连接**：超时后 `seam_suspect` 保持置位，后续接缝调用被拒，必须显式卸载/重载才复位（`axagent-plugins/crates/plugins/src/worker.rs:3313`、`:3645-3646`）。
7. **安装源校验不处理内联源码**：`plugin_validate_source` 只解析四类安装源（LocalPath / GitUrl / NpmPackage / OpenClaw），Rust 源码审计走独立的 `plugin_source_audit`（`docs/plans/PLAN-everything-is-plugin.md` §12.4，`:768-790`）。
8. **组件卸载的裸类型悬空风险**：`ComponentRegistry` 的 `typeIndex` 只记录首次注册；卸载时若不修复索引，裸类型解析会失效（已在 `unregister` 内处理，`src/lib/dynamicUI/ComponentRegistry.ts:75-85`，改动此处需回归测试）。

## 相关文档

| 文档 | 状态 | 版本 | 关联 |
|---|---|---|---|
| `PLAN-everything-is-plugin.md` | active | v3.0.0+（未定版，待确认） | 五阶段编译管线、帧协议与接缝 op 映射的权威定义（§12.1–§12.4） |
| `PLAN-plugin-gap-closure.md` | active | v3.0.0+（未定版，待确认） | 插件 P0–P4 缺口闭环与 dashboard 合流；结论与当前实现一致 |
| `PLAN-dsh-plugin-gap.md` | done | v2.9.9 | DSH 插件能力缺口盘点；**其页面/命令计数是 2026-10-27 时点快照，与当前仓库规模不一致** |
| `PLAN-shell-plus-plugins.md` | 待确认（无元信息块） | 待确认 | 主壳 + 按能力插件（P5）收编路线；与当前「8 个 workspace 根」规模并存，不构成行为冲突 |

## 版本变更记录

| 版本 | 日期 | 该功能点在本版本改了什么 | 证据（提交短哈希或 CHANGELOG 版本节） |
|---|---|---|---|
| v2.8.7 | 2026-07-24 | 插件沙箱、Event Bus、后台任务基座建立（`sandbox.rs` 起点） | `62e0ff02a`；`CHANGELOG.md:1251` |
| v3.0.0 | 2026-09-25 | 插件 worker / 源码审计进入主干 | `11aa58772`；`CHANGELOG.md:140` |
| v3.0.0 | 2026-09-25 | dashboard 合流：摘除 `rt-dashboard`，面板声明收口 `PluginManager` | `6df0ebf1a` |
| v3.0.0 | 2026-09-27 | worker 停机 drain：拒新单 → 等在途 → 再停进程 | `129b21876`；`CHANGELOG.md:155` |
| unreleased | 2026-11-01 | `src-tauri/crates` 拆为 7 个 workspace 根，插件路径随迁（无行为变更） | `8dfaeb4db` |
| unreleased | 2026-11-02 | 插件 worker 通道与页面容器收口；新增 `plugin_list_pages` 页面清单面 | `45221a824`；`CHANGELOG.md:12-13` |
| unreleased | 2026-11-02 | `market.quote` 接缝接入（宿主行情能力暴露给插件） | `4bbbeabbe`；`CHANGELOG.md:14` |
| unreleased | 2026-11-02 | stdin 写不再持锁阻塞，超时判连接不可用 | `3b4c5c58c`；`CHANGELOG.md:56` |
| unreleased | 2026-11-03 | 接缝写操作加 op 级 write 权限闸 | `ac89e1606` |
| unreleased | 2026-11-04 | 事件总线每订阅者兜底超时 + 两级丢弃计数 | `088d5a189` |
