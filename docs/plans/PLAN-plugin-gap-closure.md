# PLAN-plugin-gap-closure — 「一切皆为插件」缺口收编计划

> 状态：active ｜ 日期：2026-11-01 ｜ 对应版本：v3.0.0+（未定版，待确认）


> 前置文档：`PLAN-everything-is-plugin.md`（双判据 A/B 与 P0–P4 分层的权威定义）。
> 本文是 2026-09-25 对代码现状的核实结论 + 可插件化功能分析 + 收编顺序。

## 1. 完成情况核实（对照 P0–P4）

### 已落地

| 阶段 | 判据 | 代码证据 |
|---|---|---|
| P0 依赖收敛 | `requires` + pending_consumers | `crates/harness/src/capability_registry.rs`（`ServiceDefinition` 含 `requires`；`register_consumer` 依赖未就绪记 pending 而非静默 None） |
| P1 单表化 | 全部实现只写 `inner` 一张表 | `capability_registry.rs:17` 头部注明；修掉原 PLAN §2 缺陷 1/2（双表路由 write-only 消失、同名静默打死内置） |
| P2 接缝化 | 内置能力走接缝 | 14 家 LLM provider 统一经 `model.provider.*` 注册（`crates/providers/src/registry.rs:108`）；内置工具集 `tool.set`、`session.store`、`platform.adapter.*`、`system.prompt.*` 均在 `src-tauri/src/init/state.rs` 经接缝装配 |
| P3 worker 载入 | 帧协议 + 长驻子进程 + 门面 | `crates/plugin-proto/src/lib.rs`（长度前缀 JSON 帧、describe、MAX_FRAME_LEN、孤儿进程 EOF 自退）；`crates/plugins/src/worker.rs`（`LoadedPlugin::load` 握手→校验→远程门面注册→`EffectHandle` 收集，LIFO 卸载） |
| P4 跨插件调用 | L1 事件桥 + L2 call_seam | plugin-proto `ops::CALL_SEAM`、`call_chain` 环检测、`MAX_SEAM_CALL_DEPTH=8`；`HostPeer::dispatch_seam` 声明校验 + 自调检查 |
| 配套管线 | 安装/审计/构建 | manifest schema（`plugins/src/types.rs`）、install local/npm/git + OpenClaw 兼容、sha256 完整性、`source_audit.rs` 源码静态审计、`source_build.rs` 隔离构建、skills/agents/MCP/hooks 四类资产扩展点、DynamicUI origin=plugin 贡献 + `ui.action` 回流 |

### 未完成 / 缺口（本计划收编对象）

1. **B 层零真实用户**：仓内无任何插件声明 `worker` 或 `capabilities`（bundled 仅 2 个 shell hook 脚手架）；真实子进程集成测试默认门控跳过（`AXAGENT_TEST_PLUGIN_SUBPROCESS=1`）。
2. **远程化空档**：`model.provider.*`、`system.prompt.*`、`session.store` 不在 `SUPPORTED_REMOTE_SEAMS`（`crates/plugins/src/worker.rs:101-113`）；部分门面方法返回 `remote_method_not_supported`（reflector.aggregate_patterns、evolver.run）。
3. **`manifest.commands[]` 有校验无分发**：加载校验存在（`manager.rs`），全仓未见执行分发点。
4. **热替换不完整**：仅 stop-then-start，drain/双进程切换未实现（`manager.rs:545` 自述）；沙箱为准入检查非 OS 隔离（仅 env 白名单强制）。
5. **两套平行「插件」系统未合流**：`crates/rt-dashboard` dashboard 插件（独立 manifest，前端只展示不挂载组件）与 `crates/analysis-engine` 的 `AnalystPluginManager` 专家插件（`commands/stock_analysis.rs` 消费），均不走 `plugins` crate 协议。
6. **前端无通用贡献点**：`src/pages/` 40+ 页面、Sidebar、CommandPalette、设置项全硬编码；`dashboard_panels.component_name` 无挂载实现；插件 UI 唯一活路是 DynamicUI JSON Schema。

## 2. 可插件化功能分析（按性价比排序）

| 优先级 | 功能面 | 现状 | 插件化路径 |
|---|---|---|---|
| ★★★ | LLM provider 适配器 | 内置已走 `model.provider.*` 接缝，不可远程 | 加入 `SUPPORTED_REMOTE_SEAMS` + 远程门面（本计划 §3.1） |
| ★★★ | 系统提示词段 | 内置已走 `system.prompt.*` 接缝，不可远程 | 同上（本计划 §3.2） |
| ★★★ | dashboard 插件系统 | 平行 manifest、前端不挂载 | 收编进 `plugins` 协议 + 前端组件挂载注册表 |
| ★★★ | analysis-engine 专家插件 | 第二套平行机制 | 迁移为声明 `workflow.*`/`agent.loop` 能力的插件 |
| ★★ | `manifest.commands[]` | 有 schema 无分发 | 宿主分发 invoke → `plugin_invoker`，插件贡献业务命令 |
| ★★ | 工作流节点类型 | Rhai 词汇表硬编码 | 新增 `workflow.node.{type}` 接缝，复用工具集模式 |
| ★★ | session 存储远程化 | A 层已接缝化 | 可补远程门面，但数据主权在宿主，需谨慎评估 |
| ★ | 前端贡献点（路由/侧栏/命令面板/设置） | 全硬编码 | 轻量贡献点注册表或扩 DynamicUI 覆盖面，建议单独立项 |
| ★ | 审批规则 | 硬编码规则引擎 | `workflow.business_rule` 接缝可承载插件自定义规则 |
| 不建议 | `extension/` 浏览器扩展 | 宿主伴生入口 | 外部生态侧，保持解耦 |

**★★ `manifest.commands[]` 行的通道更新（2026-11-02 追加，任务 #49；§6 三期原文不动）**：

该行与 §6 的「宿主分发 invoke → spawn」都仍是事实；#49 补的是与之**并列的第二条通道**，
`PluginManager::execute_plugin_command` 签名不变、默认预算 30 s（`DEFAULT_COMMAND_TIMEOUT_MS`），
委托 `execute_plugin_command_with_timeout`；选道之前的前置链（存在→启用→命令声明→授权门槛→输入形状）
逐字保留原顺序（授权门槛先于形状校验，均在派发之前）：

| 通道 | 触发条件 | 行为 |
|---|---|---|
| 通道一 · 命令帧 | `loaded_workers` 有该插件的长驻 worker（manifest 声明 `worker` 且启用成功） | 走 `ops::COMMAND` 帧往返：`plugin_id` 取注册表权威键随帧绑定、执行预算随帧下发、宿主等待 = 预算 + 1 s 收帧宽限；三道写帧前防线与迟到帧语义见 `PLAN-everything-is-plugin.md` §15.8 |
| 通道二 · spawn-on-call | 未长驻 worker（含「声明了 `worker` 但当前没在跑」⇒ 退回本通道，命令不静默消失） | §6 落地的原路径**整体逐字保留**（`timeout` 参数对本通道不生效，既存语义无超时） |

**`CommandTimeout` 的归类**：worker 通道超时 ⇒ `PluginError::CommandTimeout` → `src-tauri/src/commands/plugin.rs`
的纯函数 `plugin_command_error_response` 映射为既有码 `COMMON_TIMEOUT`、category=`retryable`；
`PLUGIN_COMMAND_*` 五个协议码只进 detail，**不进前端错误码面** ⇒ 11 语 locale 零补译。
唯一可观察行为翻转：同时声明 `worker` + `commands` 的插件启用后命令改走帧（`command` 程序串不再执行）——
仓内 bundled 无此类插件，风险面在第三方。

## 3. 本期实施范围

- §3.1 `model.provider.*` 远程接缝门面（含 op 分派 + 单测）。
- §3.2 `system.prompt.*` 远程接缝门面（含单测）。
- `session.store` 本期**不做**（数据主权判据未定），dashboard / analysis-engine 合流与前端贡献点另行立项。

验收判据：`cargo clippy -p axagent-plugins --all-targets -- -D warnings` 绿 + 新增单测覆盖「声明→握手→注册→消费方读回」路径。

## 4. 落地状态（2026-09-25）

- §3.1 / §3.2 **已落地**：`worker.rs` 新增 `RemoteProviderAdapter`（op：`chat` / `list_models` / `embed`；`validate_key` 经 trait 默认实现复用 `list_models`；`chat_stream` 第一帧即明确不支持）与 `RemoteContextContributor`（`contribute` 单 op，远程失败按「跳过注入」+ 日志）；`SUPPORTED_REMOTE_SEAMS` 增至 13 条；`register_remote_seam` 与 `HostPeer::dispatch_seam` 双向补齐动态接缝分支。harness 侧 `ProviderRequestContext` 补 serde（camelCase 出线，禁区 13）。
- 新增单测 4 条：`remote_model_provider_uses_dynamic_seam_id`、`remote_provider_chat_forwards_context_and_request`、`remote_provider_stream_is_explicitly_unsupported`、`remote_system_prompt_section_contributes_or_skips`。
- 全量门禁：`cargo fmt --all --check` / `clippy --workspace --all-targets --all-features -- -D warnings` / `test --workspace`（复跑）均绿。
- ⚠ 附带发现（与本期改动无关的既有 flaky）：首轮 `test --workspace` 中 `crates/tools/src/tools/document.rs` 6 个测试 panic，单跑与整 crate 跑均通过。根因方向：这些测试用 `std::env::temp_dir().join("axagent-xxx-test")` **固定名临时目录**（`axagent-pdf-image-test`、`axagent-word-chart-test` 各被两个测试共用），跨测试二进制并发时互踩。建议另开小修复：临时目录名加入测试唯一后缀。

## 5. 二期落地（2026-09-25，dashboard / analysis-engine 合流）

**dashboard 合流**（§2 ★★★ 第 3 项）：

- 面板声明唯一权威 = `PluginManifest.dashboard_panels`（`plugins/types.rs::PluginDashboardPanel` 升级为强类型：position/size 枚举、`props: Value`、`frontendEntry`、camelCase；`author` 入 manifest）。
- `PluginManager::list_dashboard_plugins()` 实时投影；`commands/dashboard.rs` 只保留 `dashboard_list_plugins` + `dashboard_open_plugins_folder`（改指 `install_root()`），**删除** register/render/reload/install/enable/disable/unregister 7 条平行命令，操作面复用 `plugin_*`（自带护照索引同步与 UI 贡献撤销）。
- **rt-dashboard crate 整体摘除**（零内部依赖、实例目录为空、无兼容包袱）：workspace 成员/runtime re-export/AppState/SkillState/启停接线全清；`error_code.rs` dashboard mod 与 11 语言废弃键同步删除（`check-contracts.mjs` 0 warning）。
- 前端 `DashboardPluginsSettings.tsx` 对齐新 DTO（camelCase）并改调 `plugin_*`。面板**渲染挂载**仍缺（前端贡献点缺口，见 §2 ★ 项，另行立项）。
- 锁定测试：`list_dashboard_plugins_projects_installed_manifest_panels`。

**analysis-engine 合流**（§2 ★★★ 第 4 项，按路径 C 执行）：

- 删除 `AnalystPluginManager` 整套磁盘扫描机制（`plugin.rs` + `list_custom_analysts` 命令）：它是相对 CWD + `create_dir_all` 副作用的第二真相源，前端零调用，custom/*.md 已由 `EMBEDDED_PROMPTS` 编译期种子化进 `agency_experts` 表。运行链真相源统一为 DB（列表/启停走 `agency_expert.rs` 既有命令）。
- 路径 A（专家 = 轻量 manifest kind）与路径 B（expert 执行接缝化）未做，保持 §2 判断。
- AGENTS.md 追加区：analysis-engine 补登 implementor；rt-dashboard 摘除 + 「面板唯一权威在 plugins」规则。

**门禁**：`fmt --all --check` / `clippy --workspace --all-targets --all-features -D warnings` / `test --workspace --no-fail-fast`（失败计数 0，document.rs flaky 本轮未复现）全绿。

## 6. 三期落地（2026-09-26，commands[] 分发 + 端到端 worker）

**阶段3-① `manifest.commands[]` 执行分发**（§2 ★★ 第 5 项）：

- 根因：`plugin_definition_from_manifest` 转换处直接丢弃 commands 字段（「有校验无分发」的来源）。修复：四变体结构 + `Plugin` trait `commands()` 访问器 + `PluginSummary.commands` 全程带上。
- 新增 `PluginManager::execute_plugin_command(plugin_id, name, input)`：复用工具执行链（`execute_sandboxed` —— 空白切分直接 exec 防注入、ENV 白名单沙箱、`subprocess_execution` 权限门槛）；未启用 `PermissionDenied`、未知命令 `NotFound`，均在 spawn 前拦截。权限档位固定 `ReadOnly`（输入走 stdin / `CLAWD_TOOL_INPUT`），需写权限的复杂操作应声明为 tool。
- 命令层：`plugin_list` DTO 携带 `commands`；新命令 `plugin_execute_command(pluginId, name, input?)`；前端 `PluginSummaryDto` / `pluginStore` / browserMock 对齐。
- 测试：`command_dispatch_gate_blocks_without_subprocess_permission`（免子进程）+ `executes_plugin_command_via_dispatch`（gated 全链）。

**阶段3-② 端到端 worker 插件**（缺口 #1「B 层零真实用户」）：

- 新增仓内**第一个真实 worker 二进制**：`crates/plugins/src/bin/e2e_business_rule_worker.rs`（plugin-proto `serve` 实现 describe 握手 + `workflow.business_rule` evaluate）。
- 集成测试 `crates/plugins/tests/e2e_worker_test.rs`：`LoadedPlugin::load` → spawn → 握手 → 门面入全局表 → 宿主经 `get_business_rule()` 消费（Violation/Pass 两分支）→ `drop` 后 LIFO 回滚断言。**实测通过**（`AXAGENT_TEST_PLUGIN_SUBPROCESS=1`，0.71s）—— 帧协议 + 进程边界首次端到端证实，非仅内存替身。
- 沿用子进程测试门控口径（exe 锁教训见 AGENTS.md）。

**剩余缺口**（§2 表）：热替换 drain/双进程（manager.rs:545 自述）、`session.store` 远程化（数据主权判据未定）、workflow 节点类型接缝、前端贡献点、审批规则外置。
