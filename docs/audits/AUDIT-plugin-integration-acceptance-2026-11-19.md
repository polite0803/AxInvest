# AUDIT — 插件化收尾验收矩阵（2026-11-19）

> 状态：active ｜ 日期：2026-11-19 ｜ 对应版本：v3.0.1
> 关联：`PLAN-shell-plus-plugins.md`、`PLAN-p62-domain-pack-plugins.md`、`AUDIT-p62-start-baseline-2026-11-18.md`
> 性质：时点快照 + 可复跑矩阵说明；不替代真机 GUI 点击与生产库升级逐表对比。
> 更新：2026-11-19 公共 `opc` bundle 新增真安装/命令/禁用/恢复 E2E，矩阵读数更新为 17 覆盖 / 5 未覆盖；市场列表补齐分面筛选与浏览器能力边界提示；矩阵脚本增加 22 个物理 crate 目录口径，并改为按 Cargo package 真名登记 `crate`，声明式 OPC 域写 `null`。2026-11-20 发布地址链补记：矩阵 `release` 列按“是否具备可生成发布清单的成品形态”登记，不等于已上传 registry / CDN，也不等于物理独立仓。2026-11-21 接手补记：修复插件页 sandbox 运行时错误不可见、清单失败裸错误串、动态插件路由永久 loading 三处 P0 状态缺口；真机 GUI 点击仍因当前环境无图形会话而未执行。

## 2026-11-21 P0 页面状态补记

本轮先重跑接手单验收矩阵，结果为退出码 0：22 个物理 crate 目录 / 11 个 Cargo workspace crate / 17 个 bundle / 14 个 OPC 域 / 3 个独立 worker 域 / 11 个声明式域 / 17 项启停 E2E 覆盖 / 5 个无独立启停面的库或工具 crate。独立仓生成器 `--verify`、发布地址链、SDK 表面与发布清单检查、前端测试、OPC SDK 最小树检查均通过；SDK 发布演练随后输出 `PLUGIN_SDK_RELEASE_REHEARSAL_OK`，其中明确 `no cargo publish or git remote push`。

本轮只修复可从当前源码与组件行为确认的 P0 缺口：

- `PluginPageHost.tsx`：沙箱 `skill:error` 运行时错误帧原来只写日志，页面会继续停在 loading，直到 30 秒超时。现在经既有的 `event.source` 身份闸后立即写入可见错误态、撤掉装载超时并保留技术日志；外来文档的错误帧仍不改变页面状态。
- `PluginPageExplorer.tsx`：`plugin_list_pages` 失败原来直接展示 `e.message`，会把结构化后端错误暴露成 JSON 串。现在统一经 `translateBackendError()` 按 `error.${CODE}` 显示译文，未知或纯文本错误仍保留原文兜底。
- `PluginPageRouteHost.tsx`：插件页投影已刷新但仍未命中目标路由时，原来永久显示 `common.loading`。现在显示既有的 404 终态与 `error.pageNotFound`，不再把不存在的页伪装成加载中。

定点验证：

- `npm run test:run -- src/components/plugin/__tests__/PluginPageHost.test.tsx src/components/plugin/__tests__/PluginPageExplorer.test.tsx src/components/plugin/__tests__/PluginPageRouteHost.test.tsx`：3 个文件、30 passed。
- `npm run typecheck`：通过。
- `node node_modules/dprint/bin.cjs check <上述 3 个组件与 3 个测试文件>`：通过。

边界：这些是组件级行为证据，不是 Tauri 真机点击。当前执行环境 `DISPLAY` 与 `WAYLAND_DISPLAY` 均为空、`XDG_SESSION_TYPE=tty`，且未安装 `xvfb-run` / `xdotool` / `wmctrl` / `webkit2png`；现有 Playwright 配置走 Vite 浏览器 mock，不能替代 Tauri WebView。17 个 bundle 的安装、页面加载、禁用与卸载、三类完整性提示及逐页 loading / empty / error / disabled 状态仍需在桌面图形会话中人工验收并另存记录。P0 人工项未完成前不进入“正式发布或签名已闭合”的结论。

## 2026-11-20 真 bundle E2E 复跑补记

本轮在主仓工作树上用真实 bundle staging 环境变量复跑 `cd axagent-plugins && cargo test -p axagent-plugins --tests -- --nocapture`：单元/集成合计 304 passed、0 failed；其中 17 个可发布 bundle 的启停与隔离证据均实际执行：

- `e2e_opc_accounting_disable_test.rs`：install / enable / command / disable / restore，1 passed。
- `e2e_opc_sales_growth_disable_test.rs`：同上，1 passed。
- `e2e_opc_content_media_disable_test.rs`：同上，1 passed。
- `e2e_opc_declarative_domains_test.rs`：11 个声明式 OPC 域的 install / toggle / page projection，1 passed。
- `e2e_opc_common_disable_test.rs` 与 `e2e_opc_security_declarative_disable_test.rs`：公共 owner 与 security 声明式启停隔离，各 1 passed。
- `e2e_stock_bundle_install_test.rs` / `e2e_stock_agent_commands_test.rs` / `e2e_stock_tool_test.rs` / `e2e_stock_uninstall_isolation_test.rs`：stock 真 worker 命令、工具与卸载隔离，各 1 passed。
- `e2e_wiki_sync_test.rs`：wiki 真 bundle 经 host seam 执行命令并遵守禁用，1 passed。

未设置 `AXAGENT_TEST_PLUGIN_SUBPROCESS=1` 的通用子进程用例仍明确打印 SKIP，本补记不把它们计入通过；发布地址真 HTTPS E2E 未设置 `AXAGENT_RELEASE_MANIFEST_HTTPS_URL`，亦不计入本轮通过。以上 E2E 证明插件运行时的启停/隔离能力，不等价于真机 GUI 点击或 `cargo publish`。

## 一、结论摘要

插件化**尚未全部完成**。本轮只把「远程源完整性门禁 + 前端确认哈希冻结 + 浏览器 mock / 错误码 / 11 语言对齐」与逐插件验收矩阵脚本闭环；以下仍有明确缺口：

- `axagent-plugins/crates/` 实际有 **22 个物理 crate 目录**，但 `axagent-plugins/Cargo.toml` 只有 **11 个 workspace 成员**，`build-plugin-bundle.mjs` 有 **17 个可打包 bundle**，交接单里的“24 个插件 crate”与盘上口径不一致，不能混写为已验收。
- 17 个 bundle 中，现有 E2E 证据覆盖 **17 项启停链路**；公共 `opc` bundle 已由 `e2e_opc_common_disable_test.rs` 覆盖安装、命令、禁用与恢复。
- P6.2 的 14 个 OPC 域：3 个有独立 worker，11 个是声明式插件；14 域都有真 bundle 安装/页面或投影/禁用/恢复证据，但真机 GUI 点击与生产库升级逐表对比未做。
- 市场列表已能按后端投影展示来源、版本、权限、页面与完整性状态；但真机 GUI 上的逐插件页面加载与禁用后入口消失仍未逐项手测。
- 物理拆仓、SDK 正式发布仍为已知阻塞；本轮未执行任何拆仓、提交或推送。

## 二、复跑入口

只读生成矩阵：

```bash
node scripts/verify-plugin-acceptance-matrix.mjs
node scripts/verify-plugin-acceptance-matrix.mjs --json
```

脚本从五处权威取数，不复制第二份清单：

1. `axagent-plugins/crates/` 下的物理 crate 目录；
2. `axagent-plugins/Cargo.toml` 的 workspace members；
3. `build-plugin-bundle.mjs` 的 `PLUGIN_SPECS`；
4. `opc-domain-pack-plugins.json` 的 14 域底册；
5. `e2e_opc_declarative_domains_test.rs` / `e2e_opc_common_disable_test.rs` / `cargo-all-workspaces.mjs` 的 E2E 门控与 staging 证据。

版本与 `pages[].id` 通过临时模块调用 `build-plugin-bundle.mjs` 的 manifest 构造器取得，临时产物是 `.tmp/plugin-manifest-contract.mjs`，每次运行重建，不提交。页面资产按清单入口 `pages/index.html` 核对，`pages[].id` 是逻辑页 id，不是资产文件名。

脚本对“静态可构建”和“真 bundle 启停”分开标注。矩阵输出显示：物理 crate 目录 22 / workspace crate 11 / bundle 17 / OPC 域 14；OPC 独立 worker 3 / 声明式 11；E2E 启停覆盖 17 / 未覆盖 5。未覆盖项是 5 个库/构建工具 crate，它们没有独立启停面。22 个物理目录不等于 22 个独立可交付插件：11 个声明式 OPC 目录已作为 14 个 OPC 域行纳入矩阵，不重复计成独立启停项。

矩阵的 `crate` 字段只登记各目录 `Cargo.toml` 里的 Cargo package 真名，不再把声明式 OPC 目录写成虚构的 `axagent-plugin-*` 包；11 个只有 `web/` 资产、没有独立 Cargo 工程的域写 `null`，`buildable` 明确标为「否：声明式插件无独立 crate」。脚本还断言所有非空 `crate` 都必须属于它解析出的 11 个 Cargo package 名集合，防止矩阵读数和 workspace 清单再次漂移。矩阵行名仍是 bundle / 域的稳定 id，Cargo package 真名只作为可构建性证据字段，不替代 bundle 身份。

逐项“清单名/版本”由同一只读脚本从打包器 manifest 构造器生成：`manifestName` 是插件 manifest 名，`manifestVersion` 是插件 manifest 版本；有独立 Cargo 工程的行另有 `crateVersion`，声明式域的 `crateVersion` 是 `null`。3 个独立 worker 域与 wiki / stock / opc bundle 的 manifest 版本是 `3.0.0`，11 个声明式域是 `1.0.0`，5 个库/工具 crate 只登记 Cargo 版本 `3.0.0`、不伪造 manifest 名。

## 三、安全完整性链

已实读确认的关键点：

- `install_verified` 在 `materialize_source` 之前调用 `validate_expected_sha256`，远程源缺少期望 SHA-256 时下载前拒绝；`update_verified` 按 registry 记录的 source 执行同一门禁。
- SHA-256 必须是恰好 64 位十六进制字符；本地路径可省略，填写仍校验。
- `install_staged` / `update_from_staged` 对**同一次** `materialize_source` 结果先 `verify_expected_directory_hash` 再复制，避免“验证 A、重新下载 B”。
- `validate_plugin_source_verified` 允许预览不带哈希；带哈希则按同一 `hash_plugin_directory` 口径比对。
- `PluginError::IntegrityVerifyFailed` 在宿主映射为 `PLUGIN_SOURCE_INTEGRITY_FAILED`，前端 11 语言已有译文。

## 四、前端与市场 UI 边界

`PluginGallery` 已做到：

- 远程源无合法 SHA-256 时不能发起验证；
- 确认弹窗冻结 `confirmSource` + `confirmSha256`，确认处理函数自身也执行门禁；
- 确认弹窗展示来源与冻结哈希；
- 浏览器 mock 的 `plugin_install` / `plugin_update` 返回结构化 `PLUGIN_SOURCE_INTEGRITY_FAILED`，更新返回完整 camelCase `UpdateOutcomeDto`。
- 已安装插件列表新增“更新”入口：先填写/校验 SHA-256，再在确认弹窗展示冻结的来源与哈希；确认处理函数在浏览器模式直接拒绝，不伪造更新结果。远程来源更新缺哈希不能验证，验证后改动输入框不影响确认更新使用的哈希快照。
- 后端 `plugin_list` 改走 `list_plugin_marketplace_summaries()`，`PluginSummaryDto` 带 `source` / `permissions` / `pages` / `integrityStatus` / `pinnedHash`；前端按后端枚举显示 `verified` / `mismatch` / `unpinned` / `unknown`，不自行猜测。
- 已安装插件支持按名称/来源/权限/页面筛选与空态；确认弹窗展示验证时冻结的页面 id。
- 已安装插件新增三个分面筛选：来源类型（远程/本地，复用 `pluginSourceSecurity.ts` 的统一判定）、启用状态、完整性状态；三者与文本搜索交集生效，无结果走同一空态。
- 列表与确认弹窗展示来源类型；禁用插件有可见标记；安装失败后清空确认态与哈希快照；启停/卸载失败有可译提示。
- 浏览器模式明确提示只展示列表，真实安装与插件页面加载需桌面应用；mock 不伪造 `verified`。
- 浏览器 mock 与 `pluginStore` 的本地新增项只给 `unknown`，不伪造 `verified`。

未做到：

- 市场分面筛选已实现，但仍未在桌面 GUI 上逐插件核对页面加载、禁用后入口消失与完整性提示（并入下一条真机验收）；
- 插件页面/面板的 loading / empty / error / disabled 状态未逐页端到端验收；
- 浏览器模式对“没有真实安装目录与 asset 协议”的页面能力边界已有说明，但市场 UI 尚未统一提示。

## 五、本轮已执行的验证

详细命令与结果见本轮交付说明；本节只列矩阵相关读数：

- `node scripts/verify-plugin-acceptance-matrix.mjs`：通过，矩阵读数为 22 个物理 crate 目录 / 11 个 Cargo package / 17 个 bundle / 14 个 OPC 域 / 3 个独立 worker 域 / 11 个声明式域 / 17 项 E2E 覆盖。
- `AXAGENT_OPC_BUNDLE_DIR=<dist/plugins/opc> cargo test -p axagent-plugins --test e2e_opc_common_disable_test`：1 passed；公共 bundle 命令/页面禁用后均不再可用。
- `cargo test -p axagent-plugins --tests`（17 份 bundle staging 环境变量齐备）：298 个单元测试 + 全部启停 E2E 通过；另有 `AXAGENT_TEST_PLUGIN_SUBPROCESS` / `AXAGENT_TEST_PLUGIN_BUILD` 门控的既有用例明确打印 SKIP，不计入已验证。
- `node scripts/verify-plugin-acceptance-matrix.mjs --json`：通过，17 个 bundle 均有合法 manifest 版本，页面资产与清单声明一致。
- `node scripts/cargo-all-workspaces.mjs --selftest`：20 项通过 / 0 项失败。
- 前端相关测试 18 passed（4 个文件，含新增分面筛选用例）；随后又对 `pluginSourceSecurity`、`PluginGallery.integrity`、`PluginPageExplorer` 与既有事件统计回归共 6 个相关文件复跑，读数更新为 44 passed；补入更新确认冻结哈希用例后，`PluginGallery.integrity` 为 4 passed，其余 4 个相关文件共 23 passed，合计 27 passed。
- 两个 Rust 完整性用例分别 1 passed；`axagent-plugin-wiki` 26 passed。
- `cargo clippy -p axagent-plugins -p axagent-plugin-wiki --all-targets -- -D warnings`：通过。
- 宿主 `cargo check -p axagent --lib`：通过。
- 前端新增/回归测试 17 passed（4 个文件）；`npm run typecheck`、oxlint、错误码对齐、i18n 键对齐、dprint 检查通过。
- `node scripts/build-plugin-bundle.mjs --plugin wiki --check`：通过。

⚠ 矩阵脚本的 E2E 覆盖标注来自仓内测试源码与 staging 登记，本轮未重新 staging 全部 17 个 bundle，也未重跑全部门控 E2E；实际重跑结果按各 CI/本地执行记录另计。

## 六、已知阻塞

1. **物理拆仓 / SDK 发布**：插件 workspace 仍在主仓；SDK 三件套 `publish = false`，未发布到可解析 registry。
2. **真机 GUI**：插件市场点击、页面实际加载、禁用后 UI 入口消失未在桌面 GUI 逐项手测。
3. **数据兼容性**：升级前后逐表对比未在本轮执行；现有证据仅是 `opc_invoices` 等 schema 收敛保留测试与“未改 entity/migration”的静态事实。
4. **市场 UI 真机验收**：投影 DTO 与前端展示已实施，但还没在桌面 GUI 逐插件核对页面加载、禁用后入口消失与完整性提示。
