# AUDIT — 插件发布地址安装链（2026-11-20）

> 状态：active ｜ 日期：2026-11-20 ｜ 对应版本：v3.0.1
> 关联：`PLAN-shell-plus-plugins.md`、`PLAN-p62-domain-pack-plugins.md`、`AUDIT-plugin-integration-acceptance-2026-11-19.md`
> 性质：时点快照 + 可复跑事实；不替代真机 GUI 与物理拆仓验收。

## 一、结论摘要

发布地址安装链从“初版可编译”推进到“预览单次拉取 + 发布包可生成 + 定点安全测试通过”，但目标仍未完成：

- 已修复发布源预览的 A/B 漂移风险：宿主 `plugin_validate_source` 调用 `validate_plugin_source_with_expected_hash`，发布清单和成品包只拉取/物化一次，并把这次锁定的哈希随 DTO 返回前端。
- 已新增发布包生成器 `scripts/generate-plugin-release-manifest.mjs`，从已验收的 bundle 生成 HTTPS `release.json` + `.tar.gz`；哈希口径与 Rust `hash_plugin_directory` 对齐。默认清单按 `artifacts[]` 描述 `os` / `arch`，同一发布地址可携带六平台产物。
- 发布地址链路有 6 个 Rust 定点测试：发布源优先解析、发布 SHA-256 格式、安装前预览哈希门禁、预览/当前清单哈希一致性、归档路径/symlink 防护、真实 HTTPS 发布清单安装；HTTPS 用例仍由环境变量门控。
- 真实 HTTPS 安装已用新版平台清单 `https://github.com/polite0803/axinvest-plugin-release-e2e/releases/download/v3.0.0-e2e/release.json` 跑通：`validate_plugin_source_with_expected_hash` 预览返回清单哈希，`install_verified` 成功完成 Linux/x86_64 选包、GitHub Release 302 下载、解压、SHA-256 校验与安装，耗时 147.77 秒。此前 `raw.githubusercontent.com` 清单地址在当前网络路径被对端重置，故正式 README 统一给出不受 raw 域名影响的 `releases/latest/download/release.json` 地址。
- 浏览器 mock 的发布源预览返回固定合法哈希，但列表仍保持 `unpinned` / `unknown`，不伪造真安装。
- 仍未验证：真机 GUI、主仓物理拆仓、SDK 正式发布、远端六平台 GitHub Actions 实跑。

## 二、发布地址契约

用户输入：

```text
axagent-release+https://example.com/axagent/plugins/wiki/release.json
```

平台发布清单（当前格式）：

```json
{
  "version": "3.0.0",
  "artifacts": [
    {
      "os": "linux",
      "arch": "x86_64",
      "artifactUrl": "https://example.com/axagent/plugins/wiki-3.0.0-linux-x86_64.tar.gz",
      "sha256": "<64 位十六进制>"
    }
  ]
}
```

旧版 `{ artifactUrl, sha256, version }` 仍可解析和安装；新版独立仓发布工作流通过 `--merge --require-platform` 生成六平台合集。

硬约束：

1. 发布清单 URL 与成品包 URL 都必须是 HTTPS；HTTP、file、本地路径拒绝。
2. 成品包只接受 `.tar.gz` / `.tgz`；归档条目只接受普通相对路径，绝对路径、`..`、symlink 拒绝。
3. SHA-256 是解压后目录的规范化哈希，路径按字节排序后与文件内容一起喂给 SHA-256；安装与更新必须复用同一次物化目录。
4. 发布源哈希来自清单，不接受调用方覆盖；普通 Git / npm / OpenClaw 远程源仍要求用户填写合法 SHA-256。
5. 浏览器 mock 无法拉取清单，发布源预览返回确定性占位哈希，只用于前端冻结交互演练。
6. 宿主按 `std::env::consts::OS` / `ARCH` 精确选择产物；没有当前平台、同平台重复或平台字段非法时拒绝安装，绝不回退下载其他平台包。旧版单包清单继续兼容。

## 三、本轮代码变更

- `axagent-plugins/crates/plugins/src/manager.rs`：新增 `validate_plugin_source_with_expected_hash`，发布源一次物化返回 `(manifest, expected_sha256)`；文件末尾新增 3 个测试。
- `src-tauri/src/commands/plugin.rs`：`plugin_validate_source` 改用一次性方法，DTO 的 `expectedSha256` 来自同一次验证。
- `src/lib/browserMock.ts` / `src/lib/__tests__/browserMock.pluginIntegrity.test.ts`：发布源预览返回固定合法哈希与 `capabilities: []`，补发布源 mock 测试。
- `scripts/generate-plugin-release-manifest.mjs`：新增发布包生成器，带 `--selftest` 与 `--check`；默认生成 `artifacts[]` 平台清单，支持 `--legacy`、`--merge`、`--require-platform`、`--stage-artifacts`。
- `axagent-plugins/crates/plugins/src/types.rs` / `manager.rs`：发布清单增加平台产物数组，宿主按当前 OS/ARCH 选包；缺平台和重复平台明确拒绝，旧字段兼容。
- `axagent-plugins/scripts/build-plugin-bundle.mjs`：支持 `--target <cargo-triple>`，从对应 target 目录取 worker 产物。
- 17 个独立仓的 `release-plugin.yml`：六平台原生矩阵分别构建与校验，最终 job 合并清单并上传全部包。
- `scripts/ci-check.mjs` / `.github/workflows/ci.yml`：接入发布包生成器自检。

## 四、已执行验证

- `cargo test -p axagent-plugins release_source -- --nocapture`：6 项测试进程均为 passed，其中真实 HTTPS E2E 因未设置 URL 自身打印 SKIP，不能算本次真实网络通过。
- `AXAGENT_RELEASE_MANIFEST_HTTPS_URL=<公开 raw HTTPS release.json> cargo test -p axagent-plugins release_source_installs_from_https_manifest -- --nocapture`：1 passed。
- `cargo clippy -p axagent-plugins -p axagent-plugin-wiki --all-targets -- -D warnings`：通过。
- `cargo check -p axagent --lib`：通过。
- `npm run typecheck`：通过。
- `node scripts/generate-plugin-release-manifest.mjs --selftest`：通过。
- `node scripts/verify-plugin-release-flow.mjs`：通过；17/17 单平台清单和包哈希一致，六平台 merge 6/6，缺平台拒绝和旧格式兼容均通过。
- `cargo test -p axagent-plugins release_source_tests -- --nocapture`：11 passed；真实 HTTPS 环境变量用例本次 SKIP，不计入网络通过。
- `AXAGENT_RELEASE_MANIFEST_HTTPS_URL=https://github.com/polite0803/axinvest-plugin-release-e2e/releases/download/v3.0.0-e2e/release.json cargo test -p axagent-plugins release_source_installs_from_https_manifest -- --nocapture`：1 passed，147.77s；真实 Release 清单与本平台包均经 HTTPS 302 下载，安装成功。
- `cargo clippy -p axagent-plugins --all-targets -- -D warnings`：通过。
- `node axagent-plugins/scripts/build-plugin-bundle.mjs --plugin wiki --out /tmp/axinvest-wiki-bundle` 后生成并校验发布包：`--check` 通过；tar 条目为 `bin/`、`bin/axagent-plugin-wiki`、`plugin.json`。
- `npm run test:run -- src/components/chat/__tests__/PluginGallery.integrity.test.tsx src/lib/__tests__/pluginSourceSecurity.test.ts src/lib/__tests__/browserMock.pluginIntegrity.test.ts src/components/chat/__tests__/PluginGallery.eventStats.test.tsx`：22 passed（本次代码改动后待复跑确认，见下节）。

## 五、未验证 / 已知阻塞

1. 已用公开 HTTPS 清单跑通 Rust `PluginManager::install_verified` 层。宿主 Tauri 命令、真机 GUI 输入、确认弹窗、安装后页面启停仍未逐项点击验收。
2. 主仓物理拆仓未做：`axagent-plugins/` 仍是主仓内目录；根 `.gitmodules` 的旧 submodule 登记与实际结构不一致。
3. 四个 SDK 包已通过 `cargo package` 发布形状演练，但尚未执行 `cargo publish`；17 个独立仓尚无 remote，也未推送。
4. 真机 GUI 的发布地址输入、确认弹窗、安装后页面启停未逐项点击验收。
5. 17 个正式独立仓尚无 remote；当前只有测试仓 `axinvest-plugin-release-e2e` 的真实 prerelease 安装证据。六平台 GitHub Actions 与正式六平台 Release 资产尚未实跑。
6. 第三方插件签名、发布者身份与撤销通道未实现；不能把当前 SHA-256 门禁等同于签名信任模型。
