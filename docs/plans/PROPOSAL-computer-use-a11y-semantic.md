# PROPOSAL：computer-use 从「截图+坐标」迁移到「无障碍树语义化」

> 状态：approved ｜ 日期：2026-11-17 ｜ 对应版本：v3.0.0+（未定版，待确认）
>
> 定位：桌面控制 / computer-use 能力的观察与操作范式升级；用户裁定「换成无障碍树方案 + 完整语义化 + 三平台齐做」。
> 估工：分 7 阶段实施（阶段 0–6），每阶段一个可回退提交；宿主 crate 真实编译依托远程 release 工作流。
> 关联：`PLAN-shell-plus-plugins.md`；详细执行计划为会话计划文件（不入库）。
> 盘上实读依据：2026-11-17 computer-use 能力面专项审计（本方案 §1 逐条列出）。

---

## 0. 一句话目标与验收判据

**目标**：把 computer-use 从「截图喂视觉模型 + 坐标点击」迁移到「读系统无障碍树 + 对语义节点发动作」，三平台（Windows UIA / macOS AX / Linux AT-SPI2）齐做，彻底清除 xcap 引入的 pipewire/libclang 依赖链。

**验收判据**：
1. 观察层不再依赖截图：AI 工具 `ComputerUse` 与前端控制面板均消费结构化无障碍树（role/name/value/states/actions），不再返回/点击坐标；
2. `press_key` 保留（用户裁定 Q1），但只做纯键盘注入，无坐标参数、无 xcap 依赖；
3. `cargo tree -p axagent --all-features | grep -Ei 'xcap|pipewire'` 零命中；
4. 三平台可编译（宿主 cross-target `cargo check`），语义正确性由 harness mock 全链路测试 + 远程真机人工验收保证；
5. feature 名与设置项全量改名：`computer-use` → `ui-automation`，`screenPerceptionEnabled` → `uiAutomationEnabled`（含 11 语言 i18n）。

**非目标**：
- 不新建 Rust crate（平台实现为 kit 内 `ui_tree/` 子模块，避免 K1 口径与 workspace 角色涟漪）；
- 不删除 `analyze_image`（用户上传图片分析，属独立能力，非 computer-use）；
- 不改 `ComputerUse` 工具名及其 guardian / agent 双门控接线。

---

## 1. 为什么要改：现状的结构性缺陷

1. **`find_ui_elements` 名不副实**：Windows 起 PowerShell 拼 `UIAutomationClient` 脚本（`axagent-kit/crates/kit/src/ui_automation.rs:235`）；macOS `osascript` 只列顶层窗口（`axagent-kit/crates/kit/src/ui_automation.rs:341`）；Linux `wmctrl` 列窗口（`axagent-kit/crates/kit/src/ui_automation.rs:397`），AT-SPI2 分支只是 gdbus 探测后返回占位元素 `Accessible desktop`（`axagent-kit/crates/kit/src/ui_automation.rs:460`）。查到元素后仍只取 bounds 转坐标交 Enigo 点击。
2. **xcap 拖入 pipewire/libclang 链**：xcap 在 Linux 只用于 `Monitor::all()` 查 `scale_factor`（`ui_automation.rs:67`），却引入 `pipewire → libspa-sys → bindgen → libclang`，导致本机 WSL 宿主 crate 无法编译；而 Linux 截图实际走 ImageMagick `import`（`screen_capture.rs:370-395`）。
3. **AI 工具以截图为核心观察手段**：`computer_use.rs:76-87` 把 `data:image/png;base64` 喂给 LLM，6 个 action（screenshot/click/type/key/scroll/move）全是坐标范式。
4. **类型双写**：`ui_automation::UIElement` 与 `screen_vision::UIElementInfo` 并存，harness 中零 computer-use 契约（违反「共享类型权威在 harness」铁律）。

---

## 2. 目标形态

| 层 | 现状 | 目标 |
|---|---|---|
| 观察 | 截图 + 视觉模型 | 无障碍树（role/name/value/states/supported_actions） |
| 操作 | 坐标点击 / 坐标滚动 | 语义动作（Invoke / SetValue / Focus / Toggle / Select / Scroll） |
| 键盘 | 坐标型键鼠注入 | 保留 `press_key` 纯键盘（无坐标），走 enigo 键盘 API |
| 依赖 | xcap → pipewire → libclang | xcap 清除；保留 enigo（不引入 pipewire） |

---

## 3. 契约摘要（harness foundation）

新增 `axagent-core/crates/harness/src/ui_tree.rs`（trait + 纯 DTO + `UiAutomationError`），所有 DTO `#[serde(rename_all = "camelCase")]`：

- `UiNodeId { backend, raw_id, session_gen }`：不透明句柄，query 递增 `session_gen` 防旧句柄误操作；
- `UiNode { id, role, name, value, description, states, supported_actions, application, window_title, bounds: Option<..>, children }`：`bounds` 降级为前端展示用的可选诊断字段，任何写操作拒绝坐标；
- `UiTreeSnapshot { nodes, roots, captured_at }`（扁平表 + 根列表）；
- `UiTreeQuery { application, window_title, role, name_contains, max_depth, include_offscreen }`；
- `UiActionKind { Invoke, SetValue, Focus, Toggle, Select, Expand, Collapse, ScrollIntoView }`；
- `trait UiTreeProvider`：`query` / `invoke` / `set_value` / `focus` / `scroll` / `press_key`；
- `MockUiTreeProvider`（test_support）：三平台无法同时实测时的共同语义基线。

---

## 4. 分阶段路径（每阶段一个可回退提交）

| 阶段 | 内容 | 定点验证 |
|---|---|---|
| 0 | 本提案立项 + INDEX 登记 | capability 门禁 |
| 1 | harness 契约层 + mock | harness check/clippy/test；三锚 k1RsFileCount/k1PubTraitCount |
| 2 | kit 三平台实现（windows/macos/linux 子模块） | cross-target cargo check；kit clippy/test |
| 3 | 命令面改造 + 三锚同批 + 类型一致性白名单清理 | contracts/capability 门禁；build.rs 重生成命令表 |
| 4 | AI 工具 schema、前端树面板、11 语言 i18n 全量改名 | tools check/clippy；前端 typecheck/组件中立性/i18n parity |
| 5 | 截图链与依赖清退（xcap / ImageMagick / image/base64） | `cargo tree` 自证 xcap/pipewire 零命中 |
| 6 | 全量门禁收官（CI 口径一次） | fmt/clippy/test + 全部静态门禁 + 前端四件套 |

> 纪律：阶段内只跑定点命令；全量门禁只在阶段 6 跑一次（AGENTS.md 铁律）。三锚以 `check-capability-register.mjs --dump-readings` 实测为准，禁止 `--update-*` 洗白；`register_commands.rs` 是 build.rs 产物，禁止手改。

---

## 5. 风险与人工验收

- WSL 无 GUI、macOS 无 CI runner：语义靠 mock 测试，平台实现靠 cross-target `cargo check`，真机语义靠下列人工验收。
- Windows UIA 需 STA、macOS AX 需主线程：平台实现内部用专用线程 + channel，trait async 只发消息 await。
- 远程真机人工验收（发布前）：
  1. Windows：UIA 查询并激活记事本按钮；
  2. macOS：AX 查询并设值 Safari 地址栏（含首次授权弹窗）；
  3. Linux：AT-SPI2 查询并激活 GTK 按钮；
  4. 三平台均验证 `press_key` 无坐标注入可用。
