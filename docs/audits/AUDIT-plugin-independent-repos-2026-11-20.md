# AUDIT — 插件独立仓快照与发布形态（2026-11-20）

> 状态：active ｜ 日期：2026-11-20 ｜ 对应版本：v3.0.1
> 关联：`PLAN-shell-plus-plugins.md`、`PLAN-p62-domain-pack-plugins.md`、`AUDIT-plugin-release-address-2026-11-20.md`
> 性质：时点快照；记录非破坏性独立仓生成器的实际产物与未闭合依赖债。

## 一、结论

本轮新增 `scripts/generate-plugin-independent-repos.mjs`，默认把 17 个可打包插件复制成主仓外的独立 Git 仓快照，输出根为 `/home/polit/projects/AxInvest-plugin-repos`。生成器只读主仓源码，不移动、不删除、不提交主仓文件，也不推送任何远端。

盘上已有 17 个独立 `.git` 仓库。`stock` 的跨仓业务依赖已收口到协议 SDK、harness 契约与域内 `stock-core`，生成器同时随包复制 `ui.json` 并保留 `uiSchema` 映射；重生成后 17 个工作树均为干净状态，逐仓运行 `scripts/build-plugin-bundle.mjs --plugin <name> --check` 的结果为 **17 passed / 0 failed**。`node scripts/generate-plugin-independent-repos.mjs --verify` 输出 `PLUGIN_REPOS_CHECK_OK`。

每个仓的 `release-plugin.yml` 现固定覆盖 `linux/x86_64`、`linux/aarch64`、`macos/x86_64`、`macos/aarch64`、`windows/x86_64`、`windows/aarch64` 六个平台。六个矩阵任务分别构建原生目标、执行 fmt/check/test/clippy、生成单平台 `release.json` 与 `.tar.gz` 并上传 artifact；`publish` job 下载全部平台产物，以 `--merge --require-platform` 校验平台齐全后汇总 `release.json`，再由 `gh release create/upload` 发布。该工作流在本地快照中已通过结构检查，但尚未在 GitHub runner 上实际执行。

每个仓 README 给出的主应用地址固定为 `https://github.com/<owner>/<plugin>/releases/latest/download/release.json`。正式发布把清单和全部平台包放在同一 GitHub Release，宿主输入的清单地址与实际包地址均走 Release HTTPS 资产重定向；测试仓真实安装已证明该链路可完成 Linux/x86_64 下载与安装。

因此本轮证据证明“17 个插件已有独立 Git 仓快照，且 17/17 可在仓外打包为发布成品”，但仍不能证明“所有插件已经正式独立发布”。后续已完成四 SDK 包 `cargo package` 与 17/17 仅版本依赖的发布演练；正式独立发布仍需 registry 发布、签名/发布者身份与撤销通道、远端仓库推送，以及主仓物理拆仓授权。

## 二、发布地址一键安装证据

主应用侧已具备发布地址归一化、预览冻结 SHA-256、确认时哈希快照、远程源下载前完整性门禁；公开 HTTPS `release.json` 已通过 Rust `install_verified` 真网络测试。本轮新增 `scripts/verify-plugin-release-flow.mjs` 做可复跑契约回归：独立仓 17/17 自检、17/17 bundle 均实际生成 HTTPS 单平台发布清单和 `.tar.gz` 并解压核对 sha256；额外用生产生成器汇总六平台产物、验证缺少必需平台时拒绝、验证旧版单包清单兼容；发布地址归一化、前端预览哈希冻结并传给安装/更新、Rust `parse_install_source` / `validate_plugin_source_with_expected_hash` / `install_verified` / `update_verified` / 错误码映射逐项断言；并已接入 `ci-check.mjs` 与 `.github/workflows/ci.yml`。真机 Tauri GUI 的逐项点击验收仍未完成。

## 三、本轮代码变更

- `scripts/generate-plugin-independent-repos.mjs`：新增非破坏性独立仓生成器与 `--verify` 验收模式；worker 插件保留 `uiSchema` 映射、`stock` 随包复制 `ui.json`、快照提交前写入 `.gitignore`（忽略 `node_modules/`、`target/`、`dist/`），初始自检生成的 `Cargo.lock` 固定在独立仓提交中。发布工作流固定六平台原生构建矩阵，README 的单平台清单命令同步要求 `--os` / `--arch`，`--verify` 同时检查这些工作流参数。
- `axagent-plugins/crates/stock-core`：股票域纯函数迁入域内 foundation；worker 不再依赖 `analysis-engine` / `astock-data`。
- `axagent-plugins/scripts/build-plugin-bundle.mjs`：修正声明式 OPC 页面数据自检。生成物经 dprint 格式化后带对象尾随逗号，原 `JSON.parse` 会误判合法 JS 静态数据为非法；改为按 JavaScript 字面量求值后做结构断言。
- `docs/INDEX.md`：登记本审计。
- `scripts/workspace-boundary-allowlist.json` / `axagent-plugins/scripts/plugin-source-audit-baseline.json`：按当前源码重登记跨根 `path` 债务。`plugin-stock` 对 `analysis-engine` / `astock-data` 的业务实现依赖已清零；`axagent-harness` 成为 SDK registry 发布前的契约层 `path` 债务。此处只记录债务位置变化，不把它描述为已正式独立发布。
- `src/__tests__/ci.workflow.test.ts`：新增具名断言，锁定独立仓生成、17/17 验仓、逐项启停矩阵三项 CI 步骤的顺序、参数、本地镜像同步与“不得静默放行”。
- `scripts/verify-opc-plugin-sdk.mjs`：`--full` 现在先 staging stock + opc-security 真 bundle（并把 `node_modules` 链接进临时树供 jsdom 页面自检），再跑 `cargo test --all-targets`，避免 `e2e_stock_uninstall_isolation_test` 因缺产物失败；同时设齐 `AXAGENT_STOCK_WORKER_BIN`，不把工具 E2E 留成静默 SKIP。非 `--full` 的 OPC 最小树按设计剔除了 plugin-stock，只在 `axagent-plugins` 测试目标里显式 `--skip` 这条 stock 专属用例；全量树仍原样执行。
- `scripts/verify-plugin-release-flow.mjs`：发布地址安装链契约回归扩展为单平台、六平台聚合、缺平台拒绝和旧格式兼容；只证明跨文件契约与包内容哈希一致，不冒充 GUI、GitHub runner 或 `cargo publish` 验收。
- `scripts/ci-check.mjs` / `.github/workflows/ci.yml` / `src/__tests__/ci.workflow.test.ts`：接入并锁定第四步“插件发布地址链契约回归”，本地与 CI 命令同源。

## 四、已执行验证

- `node scripts/generate-plugin-independent-repos.mjs --all`：17 个独立 Git 仓生成成功，重生成后 17 个工作树均干净。
- `node scripts/generate-plugin-independent-repos.mjs --verify`：17/17 bundle 自检通过，退出码 0，输出 `PLUGIN_REPOS_CHECK_OK`。
- `node scripts/generate-plugin-independent-repos.mjs --all`（六平台工作流更新后再次执行）：17 个独立仓重新生成并提交，17 个 `git status --porcelain` 均为空；17/17 README 均包含 `--os linux --arch x86_64`。
- `node scripts/generate-plugin-independent-repos.mjs --verify`（再次执行）：17/17 `missingFiles` 为空、`workflowViolations` 为空、依赖检查通过、bundle 通过；各 worker 仓依赖清单数为 5–7，声明式仓为 1。
- `node scripts/generate-plugin-independent-repos.mjs --all`（稳定 latest README 更新后再次执行）：17/17 README 含 `releases/latest/download/release.json`，17 个工作树干净。
- `node scripts/generate-plugin-independent-repos.mjs --verify`（最终）：17/17 `workflowViolations` 为空、依赖检查通过、bundle 通过，输出 `PLUGIN_REPOS_CHECK_OK`。
- `scripts/ci-check.mjs` 与 `.github/workflows/ci.yml`：接入独立仓生成、17/17 自检和逐项启停矩阵，防止该证据面回退。
- `node scripts/check-workspace-boundary.mjs --update-baseline && node scripts/check-workspace-boundary.mjs --selftest && node scripts/check-workspace-boundary.mjs`：基线重登记 13 条已知 debt，selftest 5 passed / 0 failed，无参数门禁通过。
- `node axagent-plugins/scripts/check-plugin-source-audit.mjs --update-baseline && node axagent-plugins/scripts/check-plugin-source-audit.mjs --selftest && node axagent-plugins/scripts/check-plugin-source-audit.mjs`：基线重登记 21 条已知 debt，selftest 5 passed / 0 failed，无参数门禁通过。
- `npm run test:run -- src/__tests__/ci.workflow.test.ts`：新增接线断言后复跑 28 passed。
- `node scripts/verify-plugin-release-flow.mjs`：通过，输出 `PLUGIN_RELEASE_FLOW_OK`；独立仓 17/17、单平台发布包 17/17、六平台聚合 6/6、缺平台拒绝 PASS、旧格式兼容 PASS，发布清单 sha256 与 tar 解压内容一致。
- `AXAGENT_RELEASE_MANIFEST_HTTPS_URL=https://github.com/polite0803/axinvest-plugin-release-e2e/releases/download/v3.0.0-e2e/release.json cargo test -p axagent-plugins release_source_installs_from_https_manifest -- --nocapture`：1 passed，147.77s；测试 Release 含 `release.json` 与 Linux/x86_64 包，主应用完成真实下载安装。
- `cargo clippy -p axagent-plugins --all-targets -- -D warnings`：通过。
- `npm run typecheck`：通过。
- `npm run test:run -- src/components/chat/__tests__/PluginGallery.integrity.test.tsx src/lib/__tests__/pluginSourceSecurity.test.ts src/lib/__tests__/browserMock.pluginIntegrity.test.ts src/components/chat/__tests__/PluginGallery.eventStats.test.tsx src/components/plugin/__tests__/PluginPageExplorer.test.tsx src/components/settings/__tests__/DashboardPluginsSettings.test.tsx src/__tests__/ci.workflow.test.ts`：7 个测试文件、66 passed。
- `node node_modules/dprint/bin.cjs check scripts/verify-plugin-release-flow.mjs scripts/generate-plugin-release-manifest.mjs scripts/generate-plugin-independent-repos.mjs scripts/ci-check.mjs src/__tests__/ci.workflow.test.ts .github/workflows/ci.yml`：通过。
- `npm run test:run -- src/__tests__/ci.workflow.test.ts`：第四步接线断言后复跑 29 passed。
- `node node_modules/dprint/bin.cjs check scripts/verify-plugin-release-flow.mjs scripts/ci-check.mjs src/__tests__/ci.workflow.test.ts .github/workflows/ci.yml`：通过。
- `node scripts/verify-opc-plugin-sdk.mjs --full`：修复 staging 后通过，输出 `SDK_VERIFY_OK`；其中 `e2e_stock_uninstall_isolation_test` 1 passed，未把“缺真 bundle”降级为 SKIP。
- `node scripts/verify-opc-plugin-sdk.mjs`：非全量 OPC 树修复后通过，输出 `SDK_VERIFY_OK`；stock 专属用例在最小树中显式跳过，全量树仍覆盖。
- 主仓内 `node axagent-plugins/scripts/build-plugin-bundle.mjs --plugin wiki --check` 与 `--plugin stock --check`：均通过。
- `node axagent-plugins/scripts/build-plugin-bundle.mjs --plugin opc-security --check`：主仓内打包自检通过。
- `node scripts/verify-plugin-sdk-release-rehearsal.mjs`：通过，输出 `PLUGIN_SDK_RELEASE_REHEARSAL_OK`；四个 SDK 包按序 `cargo package`，发布清单无仓内 path；17/17 发布模式独立仓仅使用 SDK 版本依赖并完成 bundle 构建。
- `node scripts/check-sdk-package-surface.mjs --selftest && node scripts/check-sdk-package-surface.mjs`：自检 5 passed / 0 failed，四个 SDK 包表面完整。
- `node scripts/generate-plugin-sdk-release-manifest.mjs --check`：四个 SDK 包发布物清单与当前源树一致。
- `node scripts/verify-plugin-release-flow.mjs`：通过，输出 `PLUGIN_RELEASE_FLOW_OK`；17/17 发布清单与 tar 解压内容哈希一致。
- `npm run test:run -- src/__tests__/ci.workflow.test.ts`：29 passed。

## 五、未验证 / 已知阻塞

1. 四个 SDK 包发布演练已通过，但尚未执行 `cargo publish`；当前本地开发快照仍以 SDK 源码副本替代 registry 依赖，不等同于正式独立发布。
2. 主仓物理拆仓未执行；当前 17 个独立仓是非破坏性本地快照，逐一确认均无 remote，未推送任何远端，也未创建正式 GitHub Release。根 `.gitmodules` 仍有旧 submodule 登记，与实际目录结构不一致。
3. 11 个声明式 OPC 域仍使用公共 transport owner `plugin-opc-common`，没有独立 worker。
4. 真机 GUI 一键安装、安装后页面加载、禁用后入口消失未逐项点击验收。
5. 六平台 GitHub Actions 矩阵尚未在远端 runner 实际跑过；因此原生目标构建、平台 artifact 上传/下载和 Release 资产汇聚仍缺真实运行证据。
6. 插件签名、发布者身份、撤销通道未实现；SHA-256 只证明内容完整性，不等于发布者可信。
