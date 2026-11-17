# 构建体系与 CI 门禁

> 状态：active ｜ 日期：2026-11-04 ｜ 对应版本：v3.0.0 ｜ 功能点：build-ci

## 定位

仓库由 **8 个 Cargo workspace 根**（7 个 `axagent-*` 技术层 + `src-tauri`）加一个 Vite 前端组成。构建体系要解决三件事：

1. **拆分后仍能统一编译**：`[profile.*]` 必须逐根各写一份，否则各根按默认 profile 编译，产生重复产物与僵尸增量目录。
2. **门禁口径可执行且同源**：`fmt --all`、`clippy --all-targets --all-features -D warnings`、`nextest --workspace`、oxlint、dprint 必须同时出现在本地脚本与 CI。
3. **两侧命令清单不许漂移**：`ci-check.mjs` 与 `.github/workflows/ci.yml` 用对称差集护栏自动比对。

## 当前行为

### 8 个 Cargo workspace 根

- 探测规则：仓库根下含 `[workspace]` 的 `Cargo.toml` 目录，只探一层；新拆出的 workspace 自动纳入。见 `scripts/lib/workspace-roots.mjs:2-19`。
- 8 个根：`axagent-core`、`axagent-kit`、`axagent-ai`、`axagent-infra`、`axagent-quant`、`axagent-plugins`、`axagent-runtime`、`src-tauri`。
- 仓库根不存在 `Cargo.toml`（无根 `[workspace]`）。正则匹配见 `scripts/lib/workspace-roots.mjs:37`，判定函数见 `scripts/lib/workspace-roots.mjs:40-41`。
- crate 总数：7 个 `axagent-*` workspace 下共 41 个 crate（不含 `src-tauri` 主 crate 与 `schema-gen` 之外的辅助成员）。

（对应版本：未发布，`CHANGELOG.md:119`）

### `[profile.*]` 必须逐根各写一份

- **根因**：`[profile.*]` 只在「发起构建的那个 workspace 根」生效，写在成员 crate 里会被 cargo 忽略。子根缺失时退回默认值：`debug = true`、`incremental = true`。
- **实测代价**（`scripts/check-workspace-config-parity.mjs:7-32`）：
  - 同一依赖按 hash 重复编译，`axagent_harness` 实测出现 **36 份** 增量会话；
  - 僵尸增量目录 **879 个 / 68 G**；
  - 全量 clippy 从单根 **9m24s** 变为 8 根串行 **67m53s**。
- **`src-tauri/Cargo.toml:253-270`（`[profile.dev]`）**：`opt-level = 0`、`debug = "line-tables-only"`、`incremental = false`、`split-debuginfo = "unpacked"`。
- **`src-tauri/Cargo.toml:276-281`（`[profile.dev.build-override]`）**：`opt-level = 0`、`codegen-units = 16`、`debug = false`、`incremental = false`。
- **`axagent-quant/Cargo.toml:46-52`** 同样写明「只在发起构建的 workspace 根生效」，`axagent-quant/Cargo.toml:54-71`（dev profile）、`axagent-quant/Cargo.toml:77-82`（build-override）是逐根副本之一；其余 6 个根各有一份。
- **一致性校验项**（`scripts/check-workspace-config-parity.mjs:7-32`）：profile 键值完全一致；`incremental = false`；`debug` 存在且不为 `true`；`[workspace.lints.clippy]` 的 `disallowed_types` 与 `module_name_repetitions` 一致；`clippy.toml` / `rustfmt.toml` / `rust-toolchain.toml` 每根一致；`.cargo/config.toml` 的 `target-dir` 不冲突。

（对应版本：未发布，`PLAN-repo-split.md`）

### `scripts/cargo-all-workspaces.mjs`

- 四种模式（`scripts/cargo-all-workspaces.mjs:3-25`）：

| 模式 | 参数 |
|---|---|
| `fmt-check` | `fmt --all --check` |
| `clippy` | `clippy --workspace --all-targets --all-features -- -D warnings` |
| `check` | `check --workspace` |
| `nextest` | `nextest run --workspace` |

- 脚本按 `workspace-roots.mjs` 探测到的根逐个执行（`scripts/cargo-all-workspaces.mjs:32-39`）。注释说明：8 个 workspace 各有自己的测试面，只在 `src-tauri` 执行会漏掉其余 34 个 crate。

（对应版本：当前实现）

### CI 门禁口径

`.github/workflows/ci.yml` 的关键步骤：

| 步骤 | 行号 | 命令 |
|---|---|---|
| 格式检查 | `src-tauri` 之外由脚本遍历 | `node scripts/cargo-all-workspaces.mjs fmt-check`（`.github/workflows/ci.yml:615-616`） |
| Rust lint | 同上 | `node scripts/cargo-all-workspaces.mjs clippy`（`.github/workflows/ci.yml:618-619`） |
| 类型检查 | 同上 | `node scripts/cargo-all-workspaces.mjs check`（`.github/workflows/ci.yml:621-622`） |
| Rust 测试 | 同上 | `node scripts/cargo-all-workspaces.mjs nextest`（`.github/workflows/ci.yml:737-738`） |
| 前端 lint | — | `npx oxlint src --max-warnings=0`（`.github/workflows/ci.yml:441-442`） |
| 前端格式 | — | dprint 检查（`.github/workflows/ci.yml:436`） |
| 领域语义门禁 | — | 权威源 `axagent-core/crates/harness/src/domain_semantics.rs`（`.github/workflows/ci.yml:74-75`） |
| PG 集成测试 | — | `.github/workflows/ci.yml:742-744` |

- 门禁的**覆盖面本身就是门禁**：`fmt` 必须带 `--all`（否则只检查根 package，`crates/*` 全部漏检）；`clippy` 必须带 `--all-features`（否则 feature-gated 路径从未被检查）。
- **`__TAURI_WORKSPACE__=true`**：Windows 本地跑 Rust 测试时必须设置（否则测试 exe 未嵌入 Common Controls v6 manifest，启动报 `STATUS_ENTRYPOINT_NOT_FOUND`）。CI 侧不设置该变量，它是**本地**要求，见 `AGENTS.md` 的「常用命令」节与「Rust 验证门禁」节。

（对应版本：当前实现）

### `ci-check.mjs` 与对称差集护栏

- **本地近似脚本**：`scripts/ci-check.mjs:2-34`。`--quick` 路径只跑纯 Node 静态门 + dprint + oxlint + tsc + rustfmt。
- **已知未覆盖**：`npm run build`、contracts、行尾检查、PG 集成、E2E、`npm audit`、`cargo audit/deny`。
- **已接入**：2026-11-01 起 8 根 `cargo check` 与 `nextest` 进入脚本；缺工具链时「跳过」不再算通过，只有显式 `--strict-skip` 才判失败。
- **护栏测试**：`src/__tests__/ci.workflow.test.ts:286-298` 说明对称差集；用例入口 `src/__tests__/ci.workflow.test.ts:298`；`MAX_ALLOWLIST = 8`（`:300`）；从 `ci.yml` 提取 `scripts/**` 命令（`:315`）；只比较 `scripts/` 域（`:329`、`:348`）；allowlist 必须带 `reason`，失效不删也红（`:295-296`）。
- 判据：任一侧写了 `scripts/**` 命令而另一侧没写，护栏叫停；确属已知不对称只能进 `ALLOWLIST`（上限 8 条）。

（对应版本：v3.0.0，`CHANGELOG.md:433`）

### 静态门禁的两处同源注册（2026-10-31）

四道纯 Node 静态门必须同时出现在 `ci-check.mjs` 与 `ci.yml`，且命令行逐字同源：

| 门禁 | 查什么 |
|---|---|
| `check-component-neutrality.mjs` | 组件目录跨 feature 耦合 |
| `check-i18n-exemption-budget.mjs` | i18n 豁免预算 |
| `check-jsx-text-i18n.mjs` | JSX 里的未翻译字面量 |
| `check-i18n-orphan-diff.mjs` | 增量孤儿 i18n key |

规则：自检先行（带 `--selftest` 的必须先跑自检）；缺一即红；`--quick` 路径（pre-push）必须包含纯 Node 静态门，不加 `if (!quick)` 门控。详见 `AGENTS.md` 的「CI 接线」节与 `gate-coverage-ledger.md`。

（对应版本：v3.0.0，`gate-coverage-ledger.md`）

### release 工作流（Android 内存与 `CARGO_PROFILE`）

- **APK + AAB 同一次构建**，内存策略用 `CARGO_PROFILE_RELEASE_LTO=off`、`CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16`。见 `.github/workflows/release.yml:744-752`。
- 该段注释明确：**Cargo 并没有 `CARGO_PROFILE` 这个用于选择 profile 的变量**，此前的 `export CARGO_PROFILE=...` 是空操作。
- **并发控制**：ARM 目标 `CARGO_BUILD_JOBS=1`，x86_64 `=2`。见 `.github/workflows/release.yml:753-763`。
- **Linux ARM64 runner**（内存约 4 GB）同样限制 `CARGO_BUILD_JOBS=1` 并关 LTO。见 `.github/workflows/release.yml:391-400`。
- **`[profile.release-android]`**：`inherits = "release"`、`strip = true`、`lto = "off"`、`codegen-units = 16`、`opt-level = "s"`。见 `src-tauri/Cargo.toml:244-251`。

（对应版本：v3.0.0，`CHANGELOG.md:407`）

### 依赖分层与 crate 角色台账

- **依赖方向铁律**：`组件 → harness ← 实现`。consumer（`agent` / `orchestrator` / `runtime-core` / `gateway` / `quant` / `market-sim` / `notification`）只依赖 `axagent-harness`；implementor 可依赖 `harness + entities + 兄弟 implementor`；hybrid（`tools` / `rt-messaging` / `rt-workflow`）禁依赖 consumer 与 entities；wiring（`runtime` / `src/commands/` / `src/init/`）可用全部但必须经 harness trait 传递能力。
- **角色登记**：`AGENTS.md` 末尾的「附：新增 crate 角色登记（追加区）」是唯一台账；新增 crate 必须在此登记角色。
- **已知缺口**：41 个 crate 中仅 7 个真正继承 `[lints] workspace = true`（`plugins` / `plugin-proto` / `rt-messaging` / `rt-theme` / `rt-webhook` / `runtime` / `telemetry`）；其余 34 个未继承，其中 3 个是**假继承**（`axagent-runtime/crates/notification/Cargo.toml:20-21` 的 `workspace = true` 被注释掉、`axagent-runtime/crates/rt-workflow/Cargo.toml:46` 与 `axagent-runtime/crates/runtime-core/Cargo.toml:29` 的 `[lints]` 段为空表）。后果：`module_name_repetitions = "allow"` 在这些 crate 里丢失，`-D warnings` 可能把既存 warn 变成 error。

（对应版本：当前实现）

## 关键代码入口

**后端命令**

- 无（本功能不暴露 Tauri 命令）。

**脚本 / 引擎**

- workspace 探测：`scripts/lib/workspace-roots.mjs`
- 逐根执行：`scripts/cargo-all-workspaces.mjs`
- profile 一致性校验：`scripts/check-workspace-config-parity.mjs`
- 本地门禁总入口：`scripts/ci-check.mjs`
- 对称差集护栏：`src/__tests__/ci.workflow.test.ts`
- 静态门禁：`scripts/check-component-neutrality.mjs`、`scripts/check-i18n-exemption-budget.mjs`、`scripts/check-jsx-text-i18n.mjs`、`scripts/check-i18n-orphan-diff.mjs`

**前端**

- `npm run dev` / `npm run build` / `npm run typecheck` / `npm run test:run` 的脚本定义在根 `package.json`（本次未逐行取证）。

**构建配置 · 数据表**

- 8 个 workspace 根的 `Cargo.toml`（profile 与 lints）
- `.github/workflows/ci.yml`、`.github/workflows/release.yml`

## 配置与开关

- **profile**：`[profile.dev]`、`[profile.dev.build-override]`、`[profile.release-android]`，逐根一份（`src-tauri/Cargo.toml:244-281`、`axagent-quant/Cargo.toml:46-82` 等）。
- **Android 内存**：`CARGO_PROFILE_RELEASE_LTO=off`、`CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16`、`CARGO_BUILD_JOBS`（`.github/workflows/release.yml:744-763`）。
- **本地门禁模式**：`node scripts/ci-check.mjs --quick`（pre-push 路径）；`--strict-skip`（缺工具链即失败）。
- **Lint 继承**：`[lints] workspace = true`，当前仅 7 个 crate 真正继承。
- **Rust 测试环境变量**：Windows 本地必须 `__TAURI_WORKSPACE__=true`。

## 已知限制与坑

1. **`[profile.*]` 写在成员 crate 里无效**：只认发起构建的 workspace 根，8 个根必须逐字各写一份（`axagent-quant/Cargo.toml:46-52`）。
2. **`cargo fmt --check` 不带 `--all` 只查根 package**：`crates/*` 全部漏检，实测 `--check` 绿而 `--all --check` 报 4 处 diff 且全在 `crates/` 下。
3. **`clippy` 不带 `--all-features` 漏 feature-gated 路径**：`computer-use` / `notification` 等从未被检查。
4. **`clippy` 遇首个 error 即短路**：后续 crate 从未被检查，不能由「只有 N 个 error」推定其余干净。
5. **`cargo check` / `cargo test` 全绿 ≠ 合规**：`disallowed_types` 等 lint 只有 clippy 执行，`check` / `test` 完全不跑。
6. **`__TAURI_WORKSPACE__` 是 Windows 本地要求**：CI 不设置它，照搬 CI 命令到本地会失败（`STATUS_ENTRYPOINT_NOT_FOUND`）。
7. **`ci-check.mjs` 只是近似**：`npm run build`、contracts、行尾检查、PG 集成、E2E、`npm audit`、`cargo audit/deny` 均未覆盖。
8. **sccache 未启用**：`AGENTS.md:105` 明确本机 PATH / `RUSTC_WRAPPER` / 缓存目录三处皆空、CI 亦未安装；`src-tauri/Cargo.toml:253-270` 区段的注释若声称已有 sccache，属过时描述，不可作为缓解项。
9. **34 个 crate 未继承 workspace lints**：`module_name_repetitions = "allow"` 丢失，全量 `-D warnings` 存在潜在既存红灯。
10. **不要手改 `src-tauri/src/register_commands.rs`**：它是 `build.rs` 生成物，手改会被下次构建静默覆盖。

## 版本变更记录

| 版本 | 日期 | 变更 | 证据 |
|---|---|---|---|
| v3.0.0 | 2026-10-31 | Android release OOM 修复与构建内存策略 | `CHANGELOG.md:407` |
| v3.0.0 | 2026-10-31 | `ci.yml` ↔ `ci-check.mjs` 命令清单自动提取比对（对称差集） | `CHANGELOG.md:433` |
| 未发布 | — | `src-tauri/crates` 按技术层拆为 7 个 workspace 根（连同 `src-tauri` 共 8 根） | `CHANGELOG.md:119` |

## 相关文档

| 文档 | 状态 | 版本 | 关联 |
|---|---|---|---|
| `PLAN-repo-split.md` | active | v3.0.0+（待确认） | 仓库拆分与 profile 逐根约束 |
| `dependency-graph.md` | active | v3.0.0 | crate 依赖图 |
| `dependency-matrix.md` | active | v3.0.0 | crate 依赖矩阵 |
| `gate-coverage-ledger.md` | active | v3.0.0 | 门禁覆盖面台账 |
| `PLAN-MIGRATION-GUIDE.md` | active | v3.0.0 | 拆分后的迁移指南 |
| `PLAN-zip-advisory-closure.md` | active | v2.9.9 | 供应链告警闭合 |
| `AUDIT-codebase-review-roadmap-2026-09-19.md` | active | v2.9.7 | 代码库审查路线图 |
| `PLAN-declarative-schema-sync.md` | done | v2.9.7 | 声明式 schema 同步 |
| `AUDIT-ci-drift-skill-desc-2026-09-14.md` | active | v2.9.7 | CI 漂移审计 |
| `AUDIT-layer-discipline-l1-2026-09-14.md` | active | v2.9.7 | L1 分层纪律 |
| `AUDIT-duplication-2026-09-13.md` | active | v2.9.7 | 重复定义审计 |
| `PLAN-axinvest-type-driven-refactor.md` | active | v2.9.5 | 类型驱动重构 |
| `PROPOSAL-type-driven-design.md` | 计划中 | v2.9.5 | 类型驱动设计 |
| `RELEASE.md` | 未标注 | v2.7.0 | 发版流程 |
| `AUDIT-harness-architecture-2026-06-04.md` | 未标注 | v2.6.0 | Harness 架构审计 |
