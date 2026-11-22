# PLAN — 全仓插件化收尾接手单

> 状态：active ｜ 日期：2026-11-22 ｜ 对应版本：v3.0.1
> 关联：`PLAN-shell-plus-plugins.md`、`PLAN-p62-domain-pack-plugins.md`、`AUDIT-plugin-integration-acceptance-2026-11-19.md`、`AUDIT-plugin-independent-repos-2026-11-20.md`、`AUDIT-plugin-release-address-2026-11-20.md`、`AUDIT-plugin-sdk-release-manifest-2026-11-18.md`
> 性质：跨会话接手文档。记录当前盘面、已完成能力、真实剩余缺口与可复跑验收入口；不替代审计、真机验收或生产库对比。

## 一、接手结论

全仓插件化**尚未全部完成**。

当前状态可概括为：

- 插件运行时、bundle 打包、14 个 OPC 域插件边界、前端市场投影、完整性门禁已基本落地。
- 插件工程仍在主仓目录 `axagent-plugins/`，不是独立 Git 仓，也不是正式 submodule。
- 主仓外已有 17 个本地独立仓，均已配置 GitHub remote；本轮 v3.0.1 已推送 `main` 与 annotated tag，17/17 远端 Release（6 平台包 + `release.json`）已实跑校验。
- 14 个 OPC 域已全部翻牌为独立 Rust worker（原 11 个声明式域各补完整域命令集），0 个声明式域。
- 四 SDK 包已通过 `cargo package` 发布形状演练，尚未 `cargo publish`。
- 真机 Tauri GUI、远端六平台 runner、签名信任与生产历史库逐表升级对比仍待完成。

不要把以下事实写成“插件化已完成”：

- 有 17 个本地快照仓 ≠ 已物理拆仓。
- 17/17 bundle 可构建 ≠ 已发布。
- 有 SHA-256 ≠ 有发布者签名。
- E2E 覆盖启停 ≠ 真机 GUI 已手测。

## 二、仓库位置与发布边界

| 范围 | 当前事实 | 结论 |
|---|---|---|
| 插件主工程 | `axagent-plugins/` 在主仓内，是独立 Cargo workspace | 仍在主仓 |
| Git 归属 | `axagent-plugins/` 当前由主仓 Git 跟踪 | 未拆成独立 Git 仓 |
| 本地独立仓 | `/home/polit/projects/AxInvest-plugin-repos/` 下有 17 个 `.git` 仓 | 仓外源码副本 |
| 远端 | 17 个仓均登记 `origin`（`polite0803/axinvest-plugin-*`） | 已推送 17 个 `main` 与 annotated `v3.0.1`，保留历史 `v1.0.0` |
| Release | 17 仓 `Plugin CI` 与 `v3.0.1` Release 工作流全绿，每仓 7 个资产 | `publish-plugin-independent-repos.mjs --verify` 已返回 `PLUGIN_RELEASE_REPOS_VERIFY_OK` |
| SDK | 四包可 `cargo package`，独立树发布演练通过 | 未 `cargo publish` |
| `.gitmodules` | 根目录仍登记 8 个旧 submodule 占位 URL | 与实际目录结构不一致，未授权不得改 |

接手者必须先区分三个层次：

1. **源码边界已完成**：插件有独立 Cargo workspace、manifest、worker 与页面资产。
2. **发布形态已有演练**：可从主仓非破坏性生成 17 个仓外 bundle 成品。
3. **正式独立发布未完成**：仓、remote、runner、Release、registry 与签名仍未闭。

## 三、当前数量口径

本接手单统一采用下列口径，禁止把不同口径相加成“24 个插件”：

| 口径 | 数量 | 含义 |
|---|---:|---|
| 物理目录 | 22 | `axagent-plugins/crates/` 下的物理 crate 目录（含 11 个被 `exclude` 的 OPC 域 worker 模板源） |
| workspace 成员 | 4 | `plugins` / `schema-gen` / `plugin-opc-core` / `plugin-testkit` |
| 可打包 bundle | 17 | `build-plugin-bundle.mjs` 的 `PLUGIN_SPECS` |
| OPC 业务域 | 14 | P6.2 域包插件 |
| OPC 独立 worker 域 | 14 | 原 3 个垂直 worker + game_dev + 8 模板域 + finance_invest / software_dev |
| OPC 声明式域 | 0 | 已全部翻牌，公共 `plugin-opc` 只保留跨域共享能力 |
| 启停 E2E 覆盖 | 17 | 每域专属 staging 变量门控；缺变量即 fail-closed，不把 SKIP 当绿 |
| 无独立启停面 | 5 | 库/构建工具 crate |

权威复算：

```bash
node scripts/verify-plugin-acceptance-matrix.mjs
node scripts/verify-plugin-acceptance-matrix.mjs --json
```

2026-11-22 实跑读数与上述一致，退出码 0；`opcDeclarativeCount=0`，14 个 OPC 域均有各自 staging 变量与 E2E 启停证据。

早期交接稿中的“24 个插件 crate”与当前盘面不一致。除非先有第四份可复算权威，不得强行扩成 24 个验收项。

## 四、已闭合能力

### 4.1 插件运行时与域包边界

- 14 个 OPC 域均有独立插件身份、逻辑清单与页面资产。
- 14 个域都有独立 Rust worker；命令面由 `scripts/lib/plugin-contract.json` 单点定义，
  `scripts/generate-opc-worker-source.mjs` 生成源码，`check-plugin-contract.mjs` 逐项对账。
- 公共 `plugin-opc` 只保留跨域共享能力，不重新吞并已迁域包。
- `PluginManager` 按命令名解析唯一启用 owner；零 owner、多 owner、disabled owner 均有防护。
- 17 个 bundle 均具备安装、启用、命令执行、禁用、恢复的 E2E 证据。空转的 `e2e_opc_declarative_domains_test.rs` 已删除；9 个通用 worker 域由 `e2e_opc_worker_domains_test.rs` 按专属 staging 变量覆盖，game_dev 保留自己的命令级 E2E。

### 4.2 完整性校验

- 远程源缺少合法 SHA-256 时，在下载/安装前拒绝。
- SHA-256 必须是恰好 64 位十六进制；本地目录可省略。
- 校验与安装复用同一次物化结果，避免验证 A、安装 B。
- 完整性错误映射为 `PLUGIN_SOURCE_INTEGRITY_FAILED`。
- 前端在确认时冻结来源与哈希快照；改动输入框不影响确认安装/更新所用哈希。

边界：SHA-256 只证明内容完整性，**不是发布者身份签名**。

### 4.3 市场与页面投影

- 市场列表展示来源、版本、权限、页面、完整性状态。
- 已支持来源、启用状态、完整性状态与文本搜索的交集筛选。
- 浏览器模式明确不伪造真实安装或 `verified`。
- 动态插件页面入口与 `/opc/domain/*` 旧路径投影已接入。
- 禁用/卸载后会刷新投影，入口随插件状态变化。

### 4.4 发布地址链

- 支持 `axagent-release+https://.../release.json`。
- 支持六平台 `artifacts[]` 清单、当前 OS/ARCH 选包、缺平台拒绝和旧单包格式兼容。
- 已用真实 HTTPS GitHub Release 资产跑通 Linux/x86_64 下载、解压、哈希校验与安装。
- 发布地址链契约回归已接入 CI。

### 4.5 独立仓、SDK 与发布演练

- 生成器可非破坏性生成 17 个主仓外独立 Git 仓快照。
- 17/17 快照可独立完成 bundle 自检；17/17 远端 `v3.0.1` Release 已发布并含六平台包 + `release.json`。
- 六平台 GitHub Actions 工作流结构已生成并通过本地结构检查。
- 四 SDK 包已通过 `cargo package` 发布形状演练，17/17 仓的发布模式不依赖仓内 path 依赖。

## 五、剩余工作与优先级

### P0：保持现有绿灯并完成真机验收

1. 17 仓 Rust CI 与 Release 已完成；剩余真机项仍须在 Tauri 桌面应用逐项验收 17 个 bundle：
   - 市场安装；
   - 安装后页面实际加载；
   - 禁用后对应入口消失；
   - 卸载后入口与能力消失；
   - 完整性通过、缺失哈希、哈希不匹配三类提示正确；
   - 页面 loading / empty / error / disabled 状态逐页可见。
2. 补写真机验收记录，区分“自动 E2E 通过”与“人工点击通过”。

### P1：正式独立发布链

1. 经用户授权后完成主仓物理拆仓，处理根 `.gitmodules` 旧登记。
2. 为 17 个正式仓建立 remote 并推送。
3. 在真实 GitHub runner 跑六平台矩阵，确认原生构建、artifact 上传/下载、Release 汇总。
4. 四 SDK 包正式 `cargo publish`，或明确采用并落地私有 registry。
5. 为每个正式插件仓生成首个 GitHub Release 与稳定 `release.json` 地址。

### P1：信任模型

1. 设计并实现插件签名、发布者身份绑定与撤销通道。
2. 明确签名验证发生在“下载、安装、更新、启动”的哪一层。
3. 将签名失败映射为稳定错误码与 11 语言文案。
4. 保留 SHA-256 作为内容完整性门禁，不冒充签名。

### P2：数据兼容与生产升级

1. 备份生产历史库并记录升级前基线。
2. 在真实旧库上执行升级。
3. 逐表对比行数、关键字段、索引与约束。
4. 验证插件安装/禁用/卸载不删除或错误归属业务数据。
5. 保存升级前后对比结果，不能只引用“未改 entity/migration”的静态事实。

## 六、验收矩阵与定点命令

### 6.1 只读矩阵

```bash
node scripts/verify-plugin-acceptance-matrix.mjs
node scripts/verify-plugin-acceptance-matrix.mjs --json
```

### 6.2 独立仓与发布链

```bash
node scripts/generate-plugin-independent-repos.mjs --verify
node scripts/verify-plugin-release-flow.mjs
node scripts/verify-plugin-sdk-release-rehearsal.mjs
node scripts/check-sdk-package-surface.mjs --selftest
node scripts/check-sdk-package-surface.mjs
node scripts/generate-plugin-sdk-release-manifest.mjs --check
node scripts/verify-opc-plugin-sdk.mjs
node scripts/verify-opc-plugin-sdk.mjs --full
```

### 6.3 前端相关

```bash
npm run typecheck
npm run test:run -- src/components/chat/__tests__/PluginGallery.integrity.test.tsx src/lib/__tests__/pluginSourceSecurity.test.ts src/lib/__tests__/browserMock.pluginIntegrity.test.ts src/components/chat/__tests__/PluginGallery.eventStats.test.tsx src/components/plugin/__tests__/PluginPageExplorer.test.tsx src/components/settings/__tests__/DashboardPluginsSettings.test.tsx src/__tests__/ci.workflow.test.ts
```

### 6.4 后端定点

```bash
cd axagent-plugins
cargo check -p axagent-plugins
cargo check -p axagent-plugin-wiki
cargo clippy -p axagent-plugins --all-targets -- -D warnings
cargo clippy -p axagent-plugin-wiki --all-targets -- -D warnings
```

需要全量门禁时，按 `AGENTS.md` 的 CI 口径执行；不要把 `cargo check`、单元测试或 E2E 单独当作 clippy / 真机 / 发布验收。

## 七、证据文件

接手前至少实读：

- `docs/plans/PLAN-shell-plus-plugins.md`
- `docs/plans/PLAN-p62-domain-pack-plugins.md`
- `docs/audits/AUDIT-plugin-integration-acceptance-2026-11-19.md`
- `docs/audits/AUDIT-plugin-independent-repos-2026-11-20.md`
- `docs/audits/AUDIT-plugin-release-address-2026-11-20.md`
- `docs/audits/AUDIT-plugin-sdk-release-manifest-2026-11-18.md`

关键源码与门禁：

- `axagent-plugins/Cargo.toml`
- `axagent-plugins/scripts/build-plugin-bundle.mjs`
- `axagent-plugins/crates/plugins/src/manager.rs`
- `src/lib/pluginSourceSecurity.ts`
- `src/components/chat/PluginGallery.tsx`
- `src/lib/browserMock.ts`
- `scripts/verify-plugin-acceptance-matrix.mjs`
- `scripts/generate-plugin-independent-repos.mjs`
- `scripts/verify-plugin-release-flow.mjs`
- `scripts/verify-plugin-sdk-release-rehearsal.mjs`

## 八、工作树保护与禁区

- 当前工作树有大量未提交改动，接手者不得清理、覆盖或回滚。
- 禁止 `git reset --hard`、`git checkout --`、`git switch` 丢弃现有改动。
- 禁止 `git add -A`；如需提交，只按精确 pathspec 暂存。
- 未获用户授权，不推送、不拆仓、不修改根 `.gitmodules`。
- 不删除或提交 `scripts/capability-register.tmp.json`。
- 不手改 `src-tauri/src/register_commands.rs`。
- 新文档放 `docs/` 并登记 `docs/INDEX.md`；跨文档引用使用纯文件名。
- 新增 UI 文本必须覆盖 11 种语言；DTO 与 invoke 参数保持 camelCase 契约。
- 不把自动测试、bundle 构建、本地快照或结构检查写成正式发布、签名或真机验收。
- 形态级红灯当场上报，不能只记入后续计划。

本轮接手时的工作树基线（仅用于识别“哪些改动不是本轮新增”，不是提交依据）：

```text
分支：master
HEAD：6912cfd34
相对 origin/master：ahead 53
git status --short：109 行改动 / 未跟踪项
```

## 九、完成定义

只有同时满足以下条件，才能把“全仓插件化”标为完成：

1. 17 个正式插件仓有明确 remote、可复现 CI 与正式 Release。
2. 六平台 runner 实跑通过，正式 `release.json` 可被宿主安装。
3. SDK 发布方式落地并通过干净环境解析验证。
4. 真机 GUI 逐插件完成安装、页面、禁用、卸载与完整性提示验收。
5. 签名、发布者身份与撤销通道有实现和测试。
6. 生产历史库升级前后逐表对比通过。
7. 所有剩余文档与索引状态更新，且验收证据可复跑。

仅“17/17 E2E 绿”或“`cargo package` 成功”不满足上述定义。

## 十、新会话首轮动作

1. 先读 `AGENTS.md` 与本文件，再读第六节列出的最新审计。
2. 执行 `git status --short`、`git rev-parse --short HEAD`、`git rev-list --count origin/master..HEAD`，记录当前工作树，不清理任何改动。
3. 运行只读矩阵与快速契约检查，确认数量口径和失败面。
4. 按 P0 → P1 → P2 选择第一个“真实未验证缺口”；不要重复实现已闭合的完整性门禁。
5. 每完成一项立即补定点验证与证据，再进入下一项。
6. 若遇到需要拆仓、推送、修改 `.gitmodules` 或生产库写入，先取得用户明确授权。

