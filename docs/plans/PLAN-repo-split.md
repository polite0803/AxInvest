# PLAN: AxAgent 仓库拆分方案

> 创建时间：2026‑10‑31  
> 状态：**提案**  
> 关联：`dependency-graph.md`

---

## 1. 背景与痛点量化

### 当前状况

| 指标 | 数值 | 影响 |
|------|------|------|
| Rust workspace 成员 | 43（41 crates + schema-gen + 根包） | 单次 `cargo check` 全量扫描所有 crate |
| 前端组件目录 | 40+ | `npm run build` 类型检查覆盖全部 |
| 根 `package.json` 依赖数 | 50+ | 每次安装需解析全量依赖树 |
| CI 单作业耗时 | ~20 min | 串行执行 fmt/clippy/test |
| 本地 `cargo check` | ~8 min | 即使 sccache 命中，仍需扫描 43 个 crate |

### 痛点清单

1. **修改等待长** —— 改一行前端代码也要等 tsc 扫完 40+ 目录
2. **并行团队互相阻塞** —— 共享 CI 作业，一个团队的格式错误阻塞所有团队
3. **新人 onboarding 重** —— 必须理解全部 43 crate + 40 组件才能开始工作
4. **测试范围不可控** —— 无法只跑某个子系统的测试
5. **发布耦合** —— 前端改版被迫等待后端 crate 发布

---

## 2. 拆分目标

| 目标 | 量化指标 | 达成方式 |
|------|----------|----------|
| **构建时间** | ≤5 min | 子仓库并行 CI + 增量构建 |
| **并行团队** | ≥3 个独立 PR 同时进行 | 子仓库独立 CI 作业 |
| **故障隔离** | 单子仓库回归测试 ≤2 min | 按域拆分测试 |
| **新人 onboarding** | 只需理解 1 个子仓库 | 子仓库独立 README + AGENTS |
| **发布解耦** | 前端可独立发版，不等待后端 | 独立版本号 + semver |

---

## 3. 推荐方案：按技术层拆分（Plan A）

### 3.1 子仓库划分（按实测依赖矩阵修正：8 个子仓库 + 1 个应用壳层）

> 数据来源：`dependency-matrix.md`（逐份 Cargo.toml 实测）。
> 修正点：① 补齐原方案漏掉的 5 个 crate；② 将跨层共享的 `kit` 单列为公共实现层；
> ③ 修正层间依赖；④ **`axagent-mobile` 归入应用壳层，不再作为独立子仓库**（实测其为 staticlib 重导出整个 app，依赖根包是设计使然）。

| # | 子仓库 / 层 | 包含的 crate / 目录 | 负责人角色 | 依赖方向（实测） |
|---|--------|---------------------|------------|----------|
| 1 | **axagent-core** | `crates/harness`、`crates/entities`、`crates/plugin-proto`、`crates/disk-cache`、`crates/rt-theme`、`crates/axagent-agent-command-types`、`crates/axagent-agent-macro` | 架构师 | 叶子层，零内部依赖 |
| 2 | **axagent-infra** | `crates/dao`、`crates/storage`、`crates/migration`、`crates/crypto`、`crates/credential`、`crates/cache`、`crates/search`、`crates/npm`、`crates/document-parser`、`crates/rt-webhook`、`crates/device`、`crates/telemetry` | 基础设施团队 | core + kit + ai |
| 3 | **axagent-kit** | `crates/kit` | 公共实现团队 | core（**被 infra/runtime/ai 三方依赖**） |
| 4 | **axagent-runtime** | `crates/runtime`、`crates/runtime-core`、`crates/tools`、`crates/rt-messaging`、`crates/rt-workflow`、`crates/crdt`、`crates/gateway`、`crates/notification` | 运行时团队 | core + infra + kit + ai + quant + plugins |
| 5 | **axagent-ai** | `crates/agent`、`crates/orchestrator`、`crates/providers`、`crates/mcp`、`crates/prompt-guard`、`crates/trajectory` | AI 团队 | core + kit |
| 6 | **axagent-quant** | `crates/quant`、`crates/market-sim`、`crates/astock-data`、`crates/analysis-engine`、`crates/company-runtime` | 量化团队 | core + infra + kit + ai |
| 7 | **axagent-plugins** | `crates/plugins`、`schema-gen` | 插件团队 | core |
| 8 | **axagent-frontend** | `src/`（React 应用） | 前端团队 | 独立，通过 IPC 契约与后端交互 |
| 9 | **应用壳层**（留在主仓库，不拆 submodule） | `src-tauri/`（含 269 个命令文件的 wiring 层）+ `crates/axagent-mobile`（staticlib 包壳） | 应用/端上团队 | 依赖全部子仓库（组合根） |

> 说明：`axagent-mobile` 的 `src/lib.rs` 仅 `pub use axagent_lib::*;`，它是 Tauri 移动端所需的
> staticlib 外壳，**依赖整个 app 是设计使然**，故与 `src-tauri` 应用壳层同组，不单独拆仓（见 §14）。

### 3.2 依赖关系图（实测，DAG 无环）

```
axagent-core (叶子: harness / entities / plugin-proto / disk-cache / rt-theme /
              agent-command-types / agent-macro)
    ↑
    ├── axagent-plugins
    ├── axagent-ai  ──────────────┐
    ├── axagent-kit (公共实现层)   │
    └── axagent-infra ────────────┘   (infra → ai: migration/storage → kit)
            ↑
        axagent-quant   (quant → ai: analysis-engine → trajectory)
            ↑
        axagent-runtime (runtime → core/infra/kit/ai/quant/plugins)
            ↑
        axagent-mobile  (当前 → 根包，须解耦)

axagent-frontend (独立，通过 Tauri IPC 契约)
```

> 拓扑序（自底向上）：`core → {plugins, ai, kit} → infra → quant → runtime → {root, mobile}`
> 该分组图**无环**，拆分在拓扑上可行。`mobile → root` 是唯一需要先行处理的耦合。

### 3.3 为何按技术层而非业务域？

| 维度 | 按技术层（Plan A） | 按业务域（Plan B） |
|------|---------------------|---------------------|
| 依赖方向 | 天然符合 harness 分层，无循环依赖风险 | 业务域间可能交叉依赖，需额外治理 |
| CI 并行度 | 每层独立作业，最高 7 路并行 | 业务域共享基础设施，并行度受限 |
| 团队职责 | 技术背景对齐（后端 infra / AI / quant） | 业务背景对齐（trading / agent / plugins） |
| 演进风险 | 新增业务域只需扩展对应子仓库 | 新增业务域可能跨多个子仓库 |
| **推荐** | ✅ | — |

---

## 4. 迁移步骤

### 阶段 1：调研与规划（2 周）

| 步骤 | 负责人 | 产出 |
|------|--------|------|
| 绘制完整依赖图（已完成） | 架构师 | `docs/architecture/dependency-graph.md` ✅ |
| 确定拆分粒度与团队划分 | 架构师 + 技术负责人 | 本文档 ✅ |
| 评估 CI 缓存策略与迁移成本 | DevOps | CI 迁移计划 |
| 确认 i18n / contracts 处理方式 | 前端负责人 | i18n 子仓库方案 |

### 阶段 2：子仓库创建与迁移（4 周）

| 步骤 | 负责人 | 具体操作 |
|------|--------|----------|
| 创建 `axagent-core` 仓库 | 架构师 | 从 `src-tauri/crates/` 提取 harness/entities/plugin-proto/disk-cache/rt-theme，配置独立 Cargo workspace |
| 创建 `axagent-infra` 仓库 | 基础设施团队 | 迁移 12 个 infra crates，调整 Cargo.toml 路径依赖 |
| 创建 `axagent-runtime` 仓库 | 运行时团队 | 迁移 runtime/runtime-core/tools/rt-*，更新依赖路径 |
| 创建 `axagent-ai` 仓库 | AI 团队 | 迁移 agent/orchestrator/providers/mcp/prompt-guard/trajectory/kit |
| 创建 `axagent-quant` 仓库 | 量化团队 | 迁移 quant/market-sim/astock-data/analysis-engine/company-runtime |
| 创建 `axagent-plugins` 仓库 | 插件团队 | 迁移 plugins/schema-gen |
| 创建 `axagent-frontend` 仓库 | 前端团队 | 提取 `src/` + `package.json` + `vite.config.ts`，配置独立 npm workspace |
| 创建 `axagent-mobile` 仓库 | 移动端团队 | 迁移 mobile crate |

### 阶段 3：CI 并行化（2 周）

| 步骤 | 负责人 | 具体操作 |
|------|--------|----------|
| 改造 GitHub Actions | DevOps | 为每个子仓库创建独立 workflow，按依赖顺序触发 |
| 配置缓存策略 | DevOps | sccache（Rust）+ vite cache（前端）+ npm cache |
| 统一检查门禁 | QA | 将 `cargo fmt --all --check`、`clippy --workspace --all-targets`、`oxlint`、`dprint check` 复制到各子仓库 |
| 更新 contracts 检查 | 架构师 | `scripts/check-contracts.mjs` 改为跨子仓库扫描 |

### 阶段 4：本地开发体验（2 周）

| 步骤 | 负责人 | 具体操作 |
|------|--------|----------|
| 编写 `scripts/dev.sh` | DevOps | 统一入口：`./scripts/dev.sh <subrepo>` 跳转到对应子仓库执行 dev |
| 配置 git submodule | 所有团队 | 主仓库通过 `.gitmodules` 引用各子仓库 |
| 编写各子仓库 README | 各团队负责人 | 独立的安装、开发、测试指南 |
| 本地缓存优化 | DevOps | sccache 配置 + `.cargo/config.toml` 共享 |

### 阶段 5：治理与文档（持续）

| 步骤 | 负责人 | 具体操作 |
|------|--------|----------|
| 更新 AGENTS.md | 架构师 | 记录新子仓库路径、跨仓库依赖规则 |
| 维护 RepoWiki | 文档团队 | 在 QMind/RepoWiki 中记录拆分后的模块关系 |
| 版本同步策略 | 架构师 | harness/entities 采用 semver + 内部 registry 自动发布 |
| 定期审计 | QA | 每月检查跨子仓库依赖是否符合分层约束 |

---

## 5. 团队分工

| 角色 | 负责子仓库 | 职责 |
|------|-----------|------|
| **架构师** | axagent-core | 维护 harness 契约，确保分层约束不被破坏 |
| **基础设施团队**（2‑3 人） | axagent-infra | 数据库、存储、加密、搜索、遥测 |
| **运行时团队**（2‑3 人） | axagent-runtime | 工具系统、工作流引擎、消息网关 |
| **AI 团队**（2‑3 人） | axagent-ai | 智能体引擎、提供商适配、MCP、轨迹 |
| **量化团队**（2‑3 人） | axagent-quant | 策略引擎、回测、行情数据、OPC |
| **插件团队**（1‑2 人） | axagent-plugins | 插件系统、代码生成、schema 管理 |
| **前端团队**（3‑4 人） | axagent-frontend | React 应用、UI 组件、i18n |
| **移动端团队**（1‑2 人） | axagent-mobile | iOS/Android 适配 |
| **DevOps**（1‑2 人） | 全部 | CI/CD、缓存、Docker、脚本 |
| **QA**（1‑2 人） | 全部 | 统一门禁、跨仓库契约测试 |

---

## 6. 风险与缓解

| 风险 | 概率 | 影响 | 缓解措施 |
|------|------|------|----------|
| harness 版本不同步 | 高 | 编译断裂 | semver + 内部 registry + CI 自动发布 |
| 前端 `@/` 别名失效 | 中 | 构建失败 | 子仓库内重新配置 vite resolve.alias |
| Tauri IPC 契约断裂 | 中 | 运行时错误 | 独立 `axagent-contracts` crate 统一发布 |
| i18n 11 语言同步困难 | 中 | 翻译缺失 | i18n 独立子仓库 + 自动化校验脚本 |
| 跨子仓库调试复杂 | 中 | 开发效率下降 | 统一 dev 脚本 + 根仓库保留完整 CI 编排 |
| git submodule 学习成本 | 低 | 新成员困惑 | 详细文档 + 一键初始化脚本 |

---

## 7. 成功指标

| 指标 | 当前 | 目标 | 验证方式 |
|------|------|------|----------|
| CI 单次耗时 | ~20 min | ≤5 min | GitHub Actions 统计 |
| 并行 PR 数量 | 1（串行） | ≥3 | 子仓库独立 CI |
| 新人 onboarding 时间 | 2‑3 周 | ≤1 周 | 子仓库独立 README |
| 本地 `cargo check` 时间 | ~8 min | ≤2 min | 子仓库独立增量 |
| 跨子仓库违规 | N/A | 0 | 每月分层审计脚本 |

---

## 8. 回滚方案

若拆分后出现严重问题（如 CI 频繁失败、跨仓库依赖失控），可在**阶段 2 结束时**进行回滚：

1. 各子仓库保留完整的 git 历史
2. 通过 `git subtree merge` 将各子仓库合并回主仓库
3. 恢复原 CI 配置
4. 记录失败原因，调整拆分粒度后重新尝试

---

## 9. 下一步行动

1. **本周**：召开全体技术评审会，确认本方案
2. **下周**：开始阶段 1 的 CI 迁移计划与 i18n 方案
3. **第 3 周**：启动阶段 2，优先创建 `axagent-core` 和 `axagent-frontend`（影响面最大）

---

## 10. 附录：迁移后各子仓库独立门禁

每个子仓库的 CI 必须包含（与根仓库 CI 逐字同源）：

```bash
# Rust 子仓库
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
__TAURI_WORKSPACE__=true cargo test --workspace

# 前端子仓库
npm run typecheck
npm run lint:eslint
npm run format:check
npm run test:run

# 跨仓库契约
node scripts/check-contracts.mjs
```

---

## 附录 A：i18n 子仓库方案

| 方案 | 说明 | 推荐 |
|------|------|------|
| **方案 1**：i18n 作为 axagent-frontend 的子目录 | 前端团队统一管理 | ✅ 推荐 —— 11 语言文件逻辑上属于前端 UI |
| 方案 2：独立 i18n 子仓库 | 翻译团队独立维护 | 复杂 —— 需处理 npm workspace 引用 |
| 方案 3：i18n 作为独立 npm 包发布 | 前端通过 npm 引用 | 过度设计 —— 翻译通常与 UI 同步迭代 |

**推荐方案 1**：i18n 保留在 `axagent-frontend/src/i18n/`，由前端团队负责 11 语言同步，CI 中的 `check:i18n` 系列脚本确保 key 一致性。

---

## 11. 核心决策：逻辑拆分优先（两轨制）

> 这是本方案**最重要的细化**：不要一步到位拆成 8 个 git 仓库。

物理拆仓（git submodule / 多仓库）会引入 submodule 操作、跨仓 PR、版本漂移、契约断裂等高额运维成本。
而**绝大部分构建等待的收益，来自「逻辑拆分」而非「物理拆仓」**。

因此分两轨推进：

| 轨道 | 目标 | 成本 | 收益 | 是否必做 |
|---|---|---|---|---|
| **轨道 1：逻辑拆分**（推荐先做） | 同一 git 仓库内拆为 8 个**独立 Cargo workspace** + 1 个 npm workspace + 应用壳层，用边界脚本守住依赖方向 | 低（不动 git 结构） | 每个 workspace 可独立 `cargo check`；CI 可按 workspace 并行；增量构建隔离 | ✅ 必做 |
| **轨道 2：物理拆仓**（可选后置） | 拆为 8 个独立 git 仓库 + submodule（应用壳层含 mobile 留主仓） | 高（运维/契约） | 团队物理隔离、独立发布 | ⏸ 视团队规模再定 |

**判据**：只有当「多个团队需要在同一时间独立发布、且逻辑拆分后 CI 仍排队」时，才启动轨道 2。
否则长期停留在轨道 1 即可获得 80% 的收益。

---

## 12. 轨道 1：逻辑拆分的具体落地

### 12.1 目录结构（同一仓库内）

```
AxInvest/                          # 仍是同一个 git 仓库
├── axagent-core/                  # 独立 Cargo workspace
│   ├── Cargo.toml                 # [workspace] members = [...]
│   └── crates/{harness,entities,plugin-proto,disk-cache,rt-theme,
│               axagent-agent-command-types,axagent-agent-macro}/
├── axagent-kit/                   # 独立 Cargo workspace（公共实现层）
│   ├── Cargo.toml
│   └── crates/kit/
├── axagent-infra/                 # 独立 Cargo workspace
├── axagent-ai/                    # 独立 Cargo workspace
├── axagent-quant/                 # 独立 Cargo workspace
├── axagent-plugins/               # 独立 Cargo workspace
├── axagent-runtime/               # 独立 Cargo workspace（含 crdt/gateway/notification）
├── axagent-frontend/              # 独立 npm workspace（src/ + package.json + vite.config.ts）
└── src-tauri/                     # 应用壳层（组合根，留主仓库）：Tauri 壳 + 根包 axagent
    ├── Cargo.toml                 # 二进制入口 + 命令 wiring，依赖各 workspace 产物
    └── crates/axagent-mobile/     # staticlib 包壳，随壳层保留（不单独成 workspace）
```

### 12.2 关键机制：跨 workspace 的「带版本 path 依赖」

Cargo 支持同时写 `version` + `path`：

```toml
# axagent-infra/crates/dao/Cargo.toml
[dependencies]
axagent-harness  = { version = "3.0.0", path = "../../axagent-core/crates/harness" }
axagent-entities = { version = "3.0.0", path = "../../axagent-core/crates/entities" }
```

- **本地开发**：走 `path`，改一处即时生效（零运维）。
- **将来物理拆仓**：删掉 `path = ...`，保留 `version = "3.0.0"` + 私有 registry / git tag 即可，**无需改任何代码**。
- 这正是「逻辑拆分 → 物理拆分」无缝衔接的关键：`version + path` 同时表达了「语义版本约束」和「本地路径」。

### 12.3 根 workspace 排除子 workspace

`src-tauri/Cargo.toml` 的 `[workspace]` 必须 `exclude` 嵌套的独立 workspace，否则 Cargo 报错：

```toml
[workspace]
members = ["."]
exclude = [
  "../axagent-core", "../axagent-kit", "../axagent-infra", "../axagent-ai",
  "../axagent-quant", "../axagent-plugins", "../axagent-runtime",
]
```

### 12.4 边界校验脚本（新增门禁）

新增 `scripts/check-workspace-boundary.mjs`：解析各 workspace 的 Cargo.toml，断言依赖方向与 §3.1 允许矩阵一致。
- **允许**：`infra → core/kit/ai`、`quant → core/infra/kit/ai`、`runtime → core/infra/kit/ai/quant/plugins` 等。
- **禁止**：`core → 任何`、`ai → infra/quant`、`plugins → infra/ai/quant`、`kit → infra/ai/quant`（防回边）。
- 与既有 `check-layer-discipline.mjs` 同形态；**先带 `--selftest` 再判定**，`scripts/ci-check.mjs` 与 `.github/workflows/ci.yml` 两处逐字同源注册（见 AGENTS.md「CI 接线」）。

### 12.5 逻辑拆分的执行顺序（每步可独立提交）

1. **抽出 `axagent-core`**：新建 `axagent-core/Cargo.toml`，把 7 个 foundation crate 移入；`src-tauri` 的 `exclude` 加上它；全部下游改为 `version + path`。
2. **抽出 `axagent-kit`**：单独成 workspace，改动面最小（只有 1 个 crate）。
3. **依次抽 `ai → infra → quant → plugins → runtime`**（按 §3.2 拓扑序，自底向上）。
4. **抽 `axagent-frontend`**：`src/` + `package.json` + `vite.config.ts` 移入，根 `package.json` 改为 workspace 编排。
5. **接线边界脚本 + CI 并行作业**。

> `axagent-mobile` 随 `src-tauri` 应用壳层保留，**不参与搬移**（见 §14）。
> 每步单独提交、单独跑 `cargo check`（该 workspace）+ `npm run typecheck`，失败即回退该步，不影响其他步。

---

## 13. Cargo.toml 重写规则与模板

### 13.1 每个子 workspace 的根 Cargo.toml 模板

```toml
# axagent-core/Cargo.toml
[workspace]
resolver = "2"
members = [
    "crates/harness",
    "crates/entities",
    "crates/plugin-proto",
    "crates/disk-cache",
    "crates/rt-theme",
    "crates/axagent-agent-command-types",
    "crates/axagent-agent-macro",
]

[workspace.package]
version = "3.0.0"
edition = "2024"
license = "AGPL-3.0-only"
rust-version = "1.97"

[workspace.dependencies]
# 组内 crate 用 path；跨组用 version + path
axagent-harness  = { version = "3.0.0", path = "crates/harness" }
axagent-entities = { version = "3.0.0", path = "crates/entities" }

# 统一第三方依赖版本（各组复制本段，保证版本一致）
serde = { version = "1", features = ["derive"] }
thiserror = "2"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"

[workspace.lints.clippy]
# 继承主仓的 lint 基线（见 AGENTS.md「已知缺口」：41 个 crate 目前仅 7 个真正继承）
```

### 13.2 下游 workspace 的根 Cargo.toml 模板

```toml
# axagent-infra/Cargo.toml
[workspace]
resolver = "2"
members = ["crates/dao", "crates/storage", /* ... */]

[workspace.package]
version = "3.0.0"
edition = "2024"
license = "AGPL-3.0-only"

[workspace.dependencies]
# 跨 workspace：version + path
axagent-harness  = { version = "3.0.0", path = "../axagent-core/crates/harness" }
axagent-entities = { version = "3.0.0", path = "../axagent-core/crates/entities" }
axagent-kit      = { version = "3.0.0", path = "../axagent-kit/crates/kit" }
```

### 13.3 成员 crate 的 Cargo.toml 统一改法

```toml
# 组内依赖：用 workspace.dependencies 别名
[dependencies]
axagent-harness = { workspace = true }

# 若成员需要 dev-dep 到别组 crate（如 storage → dao）
[dev-dependencies]
axagent-dao = { path = "../../axagent-infra/crates/dao", version = "3.0.0" }
```

> ⚠ 不要给 `std::sync::Mutex` 等被 `clippy.toml` 禁用的类型放行 —— 拆分后每个 workspace 都要**真正继承** `[lints] workspace = true`（AGENTS.md「已知缺口」里 34 个 crate 目前未继承，拆分正是补齐的时机）。

---

## 14. `axagent-mobile` 归属裁定（已实测，取代原「解耦」假设）

**实测结论**：
- `crates/axagent-mobile/src/lib.rs` 全文只有 `pub use axagent_lib::*;` 一行，`build.rs` 仅设置 `cfg(mobile)`。
- 它是 **Tauri Apple/Android 构建所需的 staticlib 包壳**，职责就是把整个 app lib 暴露给移动端原生层。
- 根包 `axagent` 不是薄壳：`src-tauri/src/commands/` 有 **269 个 .rs**、`lib.rs` 971 行，是完整的应用装配（wiring）层。

**裁定：`axagent-mobile` 依赖根包是设计使然，不存在「改成依赖少数 crate」的解耦空间。**
因此：

1. **归组**：`axagent-mobile` 与 `src-tauri` 同属「应用壳层」，**不拆为独立子仓库**，也不拆为独立 submodule。
2. **不再设 L0「解耦」阶段**：原计划里「mobile 不再依赖根包」的验收目标作废（它与架构事实冲突）。
3. **附带优化（可选）**：桌面与 Linux CI 的 `cargo --workspace` 会把 mobile 的 staticlib 也归档一遍（桌面用不到）。
   如需省这一次全 app 归档，可把 `crates/axagent-mobile` 从 workspace members 移出、改为按需 `--manifest-path` 构建；
   ⚠ 该改动会影响现有 `--workspace` 检查面，**须单独提交并跑全量 clippy/test 验证**，不作为拆分前置项。

> 拆分落地的真正第 1 步改为 **L1：抽出 `axagent-core`**（见 §17）。

---

## 15. 版本化与发布策略

| crate | 版本策略 | 发布渠道 | 说明 |
|---|---|---|---|
| `harness` / `entities` / `plugin-proto` | **严格 semver**（破坏性改动用主版本 +1） | 私有 registry 或 git tag | 它们是所有下游的契约，版本必须可追溯 |
| `kit` | semver | 同 core | 三方共享，改动需通知 infra/runtime/ai |
| 其余 internal crate | 可跟随主仓版本（3.0.0） | 逻辑拆分阶段用 path，无需发布 | 仅在物理拆仓后才需发布 |
| 前端 `@axagent/ipc-contract` | semver（与后端契约同步） | 私有 npm registry / git tag | 见 §16 |

**发布流程（物理拆仓后）**：
1. core 改 `harness` → bump `harness` 版本 → 打 tag / 发布。
2. 下游各仓库把 `axagent-harness = "x.y.z"` 升级 → 各自 CI 通过 → 合并。
3. 主仓库（组合根）锁定各子仓库的最小子模块提交。

> 逻辑拆分阶段（同仓库）无需任何发布：`version + path` 让 Cargo 直接走本地，版本号仅作语义标注。

---

## 16. 前后端 IPC 契约

**现状**：`schema-gen` 由后端 DTO 生成 `src/types/generated/ipc.ts` / `events.ts`，提交到前端源码树；
CI 用 `cargo run -p schema-gen -- check` 做字节级漂移比对。

**拆分后契约方案**：

```
axagent-plugins/schema-gen ──生成──▶ @axagent/ipc-contract (npm 包)
                                          │  含 ipc.ts / events.ts / 版本号
                                          ▼
                                   axagent-frontend（依赖该包）
```

| 环节 | 机制 |
|---|---|
| 生成 | 后端 CI 跑 `schema-gen`，产出 `ipc.ts` / `events.ts` + `contract-version.json` |
| 发布 | 打 tag `contract-v1.2.0` 或发布私有 npm 包 `@axagent/ipc-contract@1.2.0` |
| 消费 | 前端 `package.json` 精确锁定契约版本；`src/types/generated/` 改为从包 re-export |
| 漂移检测 | 前端 CI 跑「本地生成 vs 已发布包」diff；不一致即红（防止前端悄悄改契约） |
| 命令注册 | 根包 `build.rs` 的 `register_commands.rs` 仍在 `src-tauri`（组合根）生成，不跨前端 |

> 前端**不需要** Rust 工具链：它只消费生成的 TS 类型。契约包是唯一跨仓接口。

---

## 17. 分阶段验收标准（Definition of Green）

| 阶段 | 产出 | 验收标准（必须全绿才进入下一阶段） |
|---|---|---|
| **L0 mobile 归属（已裁定）** | mobile 随应用壳层保留 | `cargo check -p axagent-mobile` 仍通过；文档已声明其与 `src-tauri` 同组；**无「解耦」动作** |
| **L1 抽出 core** | `axagent-core/` workspace | 在 `axagent-core/` 下 `cargo check` 通过；根 `cargo check --workspace` 通过；全量测试通过 |
| **L2 抽出 kit/ai/infra/quant/plugins/runtime** | 6 个 workspace | 每个 workspace 独立 `cargo check` 通过；根全量 `clippy --workspace --all-targets --all-features` 通过 |
| **L3 抽出 frontend** | `axagent-frontend/` npm workspace | `npm run typecheck` + `npm run build` + `npm run test:run` 通过；契约包消费正常 |
| **L4 边界门禁** | `check-workspace-boundary.mjs` | `--selftest` 通过 + 判定通过；CI 双处接线（`ci-check.mjs` + `ci.yml`） |
| **L5 CI 并行** | 8 个并行作业（+ 应用壳） | 全量 CI 绿；耗时 ≤5 min |
| **P1 物理拆仓（可选）** | 8 个 git 仓库 + submodule（mobile 留在主仓） | 主仓 `git submodule status` 全绿；各仓独立 CI 绿；`scripts/dev.sh` 可用 |

> **L4 是分水岭**：边界门禁一旦上线，任何越层依赖都会被 CI 拦住，之后物理拆仓只是「搬运」，风险大幅下降。

---

## 18. 风险登记（更新版）

| 风险 | 概率 | 影响 | 缓解 |
|---|---|---|---|
| mobile 归档拖慢 CI（非耦合问题） | 低 | 每次全 app 归档一次 | 可选：将 `crates/axagent-mobile` 移出 workspace members，按需 `--manifest-path` 构建（须单独验证） |
| `version + path` 的版本约束与 path 冲突 | 低 | 本地编译失败 | 统一在 `[workspace.dependencies]` 管理；CI 校验版本一致 |
| 多个 workspace 重复编译第三方依赖 | **已发生**（2026-11-01 实测） | 全量 clippy 冷轮 **41m36s** / 稳态 **1m37s**（§20.9；旧读数 67m53s 来源待证）；僵尸增量会话 879 个 / 68G → **0 个** | 已修：8 根逐字各写 `[profile.dev*]`（`[profile.*]` 只在发起构建的根生效）。**sccache 未启用**（本机与 CI 均无 `RUSTC_WRAPPER`），不作为缓解项引用；残余的 per-root feature 统一代价属形态固有，见 §20.7 |
| 边界脚本误判/漏判 | 中 | 假绿或假红 | 必须带 `--selftest`（正负对照），与既有门禁同形态 |
| 契约包与前端版本漂移 | 中 | IPC 字段不一致 | 前端精确锁版本 + CI 漂移检测 |
| submodule 操作门槛（若走 P1） | 低 | 新人困惑 | `setup-submodules.sh` + 文档；能停留在 L5 就不拆仓 |

---

## 19. 一句话结论

**先把当前 monorepo 拆成「同仓库的 8 个逻辑 workspace + 1 个应用壳层」（轨道 1），用 `version + path` 依赖 + 边界门禁守住分层；
物理 git 拆仓（轨道 2）作为可选后置项，仅在团队/发布规模确实需要时启动。**
这样能拿到约 80% 的构建等待收益，却只承担约 20% 的迁移风险。

---

## 20. L2 执行进度（2026-11-01）

### 20.1 结构（已达成）

- 7 个独立 Cargo workspace：`axagent-core`（7 成员）、`axagent-kit`（1）、`axagent-ai`（6）、`axagent-infra`（12）、`axagent-quant`（5）、`axagent-plugins`（2，含 `crates/schema-gen`）、`axagent-runtime`（8）。
- `src-tauri` 只剩根包 `axagent` + `crates/axagent-mobile`（`cargo metadata --no-deps` 读数 2 个包）。
- 所有搬移均用 `git mv`（保留历史）；跨 workspace 依赖统一 `{ version = "3.0.0", path = ... }`；各 workspace 共享 `target-dir = "../src-tauri/target"`。
- 各组 `cargo check --workspace` 均已完成（kit / ai / infra / quant / plugins / runtime 均 exit 0）。
- 收尾复核：8 个 workspace 的 `cargo metadata --no-deps` 全部 exit 0，包数依次为 `src-tauri` 2 / `axagent-core` 7 / `axagent-kit` 1 / `axagent-ai` 6 / `axagent-infra` 12 / `axagent-quant` 5 / `axagent-plugins` 2 / `axagent-runtime` 8；`src-tauri` 的 `cargo check --workspace` exit 0（`Finished` 28m06s）。

### 20.2 门禁扫描面（已改为自动探测）

`scripts/lib/workspace-roots.mjs` 自动探测「含 `[workspace]` 的目录」，门禁不再硬编码根名单。
搬移后的实测读数（与 L1 基线对账）：

| 门禁 | 读数 | 退出码 | 说明 |
|---|---|---|---|
| check-orphan-modules | 扫描 **1989** 个 `.rs`（L1: 1988） | 0 | +1 = `schema-gen/src/main.rs` 从 `src-tauri/schema-gen`（不在任何 `crates/*` 下、原先不被扫描）迁入 `axagent-plugins/crates/` |
| check-layer-discipline | crate 目录 **42** / 成员 **42** / **8** 个 workspace 根（L1: 41/42/2） | 1 | 仅剩已知用户责任红 `commands-no-sibling-call`（`master_key.rs` 95 > 94）；`--selftest` 51/0 |
| check-single-source-facts | 扫描 **2063** 个 `.rs`；LOCATED **594** / NONLOC **9** / EXTERNAL **43** / BROKEN **0** | 3 | 软判据退出码与基线一致；+3 = `schema-gen`（+1）+ 用户新增未跟踪 `.rs`（+2） |
| check-contracts | errors=0 warnings=0 | 0 | PASS |

### 20.3 已同步的接线

- `scripts/cargo-all-workspaces.mjs`（新增）：在全部 8 个 workspace 根依次执行 `fmt-check / clippy / check / nextest`，根列表与静态门禁同源自动探测。
- `scripts/ci-check.mjs`：fmt / clippy 改为跨 workspace；单测按新归属拆到 `axagent-quant` / `axagent-core` / `axagent-runtime`；schema-gen 改在 `axagent-plugins` 下执行。
- `.github/workflows/ci.yml`：fmt / clippy / check / nextest 覆盖全部 workspace；schema-gen 工作目录改 `axagent-plugins`；PG 集成测试 `--manifest-path axagent-infra/Cargo.toml`；cargo audit / deny 改为逐 lock 执行（新 lock 现场 `generate-lockfile`）。
- `package.json`：`check:schema-gen` 与 pre-commit 的 fmt 检查同步改口径。

### 20.4 待处置（本轮只记录）

- **Cargo.lock 漂移**：8 份 lock 中同一包名解析出不同版本共 **277** 个包（清单：`.tmp/cargo-lock-drift.txt`）。新 workspace 的 lock 尚未入库。
- **依赖方向偏离**：`axagent-ai/crates/trajectory` 的 dev-dependency 指向 `axagent-infra/crates/dao`，与 §12.4「禁止 ai → infra」冲突（dev-only）。
- **文档与机器状态不符（构建时间相关）**：AGENTS.md「常用命令」写「sccache 缓存 50G」、§18 把 sccache 列为「多 workspace 重复编译第三方依赖」的缓解项，但本机 `sccache` 未安装（PATH / `~/.cargo/bin` / `%LOCALAPPDATA%` 缓存目录 / `RUSTC_WRAPPER` 四处全空）。要么装上，要么把两处措辞改成「未启用」，不许留「看着有其实没有」的缓解项。详见 §20.7。
- **共享 target 目录容量**：`CARGO_TARGET_DIR` 全局指向 `C:\Users\刘小平\.cargo\target-axagent`（覆盖仓内 `.cargo/config.toml` 的 `../src-tauri/target`），8 个根 × dev/test 双 profile 都往这里写；2026-11-01 实测该盘在测试队列中途被写到 `os error 112 磁盘不足`。C: 299G / D: 尚有 240G，迁移待用户拍板。
- ~~**用户未提交在制品**：`rhai_registry.rs` 的 3 条 `include_str!` 仍指向旧路径~~ → 已在 §20.5 收敛（只改这 3 行路径，其余仍属用户）。`data-verifier.rhai`（rhai-numeric-typing 红）与 `master_key.rs`（layer 红）同属用户责任，未动。

### 20.5 L2 收口验证（2026-11-01 复扫）

**编译面全绿**：8 个 workspace 逐一 `cargo clippy --workspace --all-targets --all-features -- -D warnings` 全部 exit 0（`axagent-core` 12m52s、`src-tauri` 24m02s）。`--all-targets` 覆盖 test/bench，等价于「所有 `include_str!` 与路径字面量在编译期已通」。

**搬家遗留的断链共 5 类，已全部修完**（判据：全仓 4152 个 `.rs` 的 716 处 `include_str!`/`include_bytes!` 字面量实测零断链；50 份 `Cargo.toml` 的 139 处 path 依赖目标全部存在）：

| 类别 | 位置 | 改后 |
|---|---|---|
| 编译期 include | `rhai_registry.rs` 3 行 | `../../../../axagent-{runtime,quant}/crates/...` |
| 编译期 include | `src-tauri/tests/stock_workflow_write_side.rs:245` | `../../axagent-quant/crates/analysis-engine/src/trade_intent.rs` |
| 运行时读文件（根包） | `seed_consistency_tests.rs` 3 处 | `../axagent-quant/crates/{analysis-engine,recommender...}` |
| 运行时上溯级数 | `kit/browser_automation.rs`、`rt-workflow/tests/rhai_syntax_check.rs`、`tools/market_mainline.rs`、`dao/reconcile/mod.rs` | 按新目录深度重算 + 补 `src-tauri` 段 |
| 前端测试读源 | 4 个 `__tests__` 文件 | 指向 `axagent-{infra,ai,quant}/crates/...`（vitest 36 用例通过） |

**⚠ 最危险的一处不是断链，而是假绿**：`seed_consistency_tests.rs::every_reco_picks_insert_stamps_algorithm_version` 原以 `CARGO_MANIFEST_DIR` 为根扫 `["src", "crates"]`，L2 后 `src-tauri/crates` 只剩 `axagent-mobile` ⇒ 号称「全仓扫描」实际少查 40 个 crate，而 `assert!(files.len() > 100)` 仍为真。已改为从仓库根自动探测全部含 `[workspace]` 的 `axagent-*/crates`，并把下界改成**不会被单个大目录撑起来的形态**：`workspaces >= 7` + `crate_dirs >= 42` + `.rs >= 2000`（实测扫面 332 → 2060）。

**fmt**：`core / kit / ai / plugins` 与根包 `axagent` 均 `--check` 零 diff；`rt-workflow` 因 `include_str!` 变长超宽，已用**定点** `cargo fmt -p axagent-rt-workflow` 修复（不用全局 `cargo fmt`，否则会改写用户在制品）。剩余红项全部落在用户未提交的 `stock_factor` 一族（`analysis-engine/src/{lib,stock_factor}.rs`、`tools/src/tools/{mod,stock_factor}.rs`），非本轮引入。

**本地跑不了 `cargo test` 的两个环境坑**（记录，避免后人误判为代码错）：
1. 从 UNC 路径（`\\vmware-host\...`）链接测试 exe 必挂：`mt.exe` 无法解析 `\\?\UNC\...common-controls.manifest`（`general error c1010070`）。凡 `build.rs` 嵌 manifest 的包（`crates/agent`、`src-tauri`）⇒ **必须改用盘符路径** `D:\OneManager\AxInvest`（同一共享已映射为 `D:`）。
2. 默认并行度下出现 `os error 1450 系统资源不足` 与 rustc `STATUS_STACK_BUFFER_OVERRUN`、linker `0xc00000fd` ⇒ 统一 `-j 3` 且串行（8 个 workspace 共享同一 target 目录，并发只会互相等锁）。

**尚未取得**：运行时测试证据（受上述两坑 + 其他会话持有 cargo 包缓存锁影响，正在按盘符路径重跑）。CI 侧接线已覆盖全部 workspace（见 §20.3），推送后可由 CI 补这份证据。

### 20.6 拆分的「配置零漂移」与「覆盖集合相等」终验（2026-11-01 独立复扫）

拆成 8 个 workspace 后，风险从「路径断链」变成**两份配置悄悄不一致**与**某个 crate 不属于任何 workspace**。这两类都不会让任何一道现有门禁变红，故单独实测：

| 终验项 | 口径 | 读数 | 结论 |
|---|---|---|---|
| crate 目录 ↔ workspace members **逐元素相等** | `.tmp/verify-members-coverage.mjs`（不只比数量，比对称差） | 8 根 / members 43 / crate 目录 42；对称差为空，唯一「多出的成员」是 `src-tauri` 根包本身 | ✔ 无「存在但从不被编译」的 crate |
| `[workspace.lints.clippy]` 三键 | 8 份根 `Cargo.toml` 原文比对 | `disallowed_types = "deny"` + `module_name_repetitions = "allow"` 在 8 根**逐字相同** | ✔ 禁区 8 未因搬家失去执行面 |
| `clippy.toml` / `rustfmt.toml` / `rust-toolchain.toml` | md5 | 三组文件在 8 根**各自 md5 全同** | ✔ 无「某根用另一套 lint 白名单 / 行宽 / 工具链」的假绿 |
| `[lints] workspace = true` 实际继承 | 遍历 42 份 crate 清单 | **7 / 42**（与拆分前同一缺口，非本轮引入；AGENTS.md「已知缺口登记 2026-10-30」） | ⚠ 仍是既存待办 |
| `.cargo/config.toml` | `target-dir = "../src-tauri/target"` | 7 根全部同值 | ✔ 共享增量缓存口径一致 |

**新暴露的三个「搬家不产生红、但埋雷」的点**（只登记，未改动）：

1. **仓库根没有 `clippy.toml` / `rust-toolchain.toml`**：这三份配置按「最近的清单目录」解析，故只有 `node scripts/cargo-all-workspaces.mjs …`（cwd = 各 workspace 根）这种跑法才吃到它们。**在仓库根直接敲 `cargo clippy` 会同时丢开 8 根的钉版与 disallowed-types 白名单** —— 这是一条新的「本地绿 ≠ CI 绿」路径，接线时不要绕开该脚本。
2. **P1（物理拆仓）的脚手架仍在仓库根，且是未跟踪状态**：`.gitmodules`（8 条指向 `polite0803/axagent-*.git` 的**占位 URL**，仓库并不存在）与 `.github/workflows/ci-split.yml`（硬编码 8 根、clippy 不带 `--all-targets --all-features`，与 `ci.yml` 触发同一 `push: master`）。本轮只做 L2「一仓多 workspace」，P1 明确 ⏸（见 §11 轨道 2 / §17 P1）。风险是**一次性 `git add .` 提交 1969 条搬家记录时会把它们一起入库**：`.gitmodules` 一旦生效而索引里没有对应 gitlink，`git submodule update --init` 当场失败；`ci-split.yml` 则会与 `ci.yml` 重复跑一套口径更弱的检查。**建议动手前删除或移出工作根**（属用户文件，本轮未动）。
3. 附带：`scripts/audit_dupdef.py:56-60` 的 `crate_of()` 仍按 `src-tauri/crates/` 前缀归类，搬家后落入 `return p` 分支（按整条路径分组，判定变粗但不报错）。该脚本未接进 `ci-check.mjs` / `ci.yml`，属临时审计工具。

### 20.7 构建时间回归：实测、根因与本轮修复（2026-11-01）

**先认账：目标没达成，本地全量反而更慢。** 同一命令的历史对照：

| 命令 | 拆分前（单 workspace / 40 成员） | 拆分后（8 根串行） |
|---|---|---|
| `clippy --workspace --all-targets --all-features -- -D warnings` | **9m24s**（本仓 docs 历史记录） | **67m53s** = 12:52 + 8:31 + 5:24 + 10:44 + 2:13 + 0:50 + 5:37 + 24:02 —— ⚠ **该读数来源待证**：缓存状态未记录，且当时用的 `cargo-all-workspaces.mjs` 参数顺序有缺陷（`--manifest-path` 落在 `--` 之后 ⇒ clippy-driver 直接报错，非代码问题）。可复现的两轮实测见 §20.9 |
| `check --workspace`（`src-tauri` 根） | L1 两根时 18–19m | L2 后 **28m06s**，而该根下只剩 2 个包 |
| §17 的 CI 目标 | 单作业 ~20m → **≤5m** | 未达成（本地口径先失败） |

**根因分三层，前两层是 L2 的执行缺陷（已可修），第三层是形态固有代价：**

1. **7 个子根丢了 `[profile.dev*]`** —— 已修，见下方「本轮修复」。机制：`[profile.*]` **只在发起构建的那个 workspace 根生效**，成员里写了 cargo 直接忽略。子根缺它 ⇒ 退回 `debug = true` / `incremental = true` / 默认 codegen-units ⇒ ① profile 不同 ⇒ fingerprint 目录不同 ⇒ **同一份第三方依赖在子根与 `src-tauri` 各编一遍**；② 重新踩回 `src-tauri/Cargo.toml` 注释里那两个坑（os error 5 僵尸增量会话 55G、20 万 `.o` 链接灾难）。
2. **§18 登记的缓解「共享 target-dir + sccache」里，sccache 在本机不存在**：`sccache` 不在 PATH、`~/.cargo/bin` 无该文件、`%LOCALAPPDATA%` 下无缓存目录、`RUSTC_WRAPPER` 未设、任何 `.cargo/config.toml` 都没有 wrapper —— 而 AGENTS.md「常用命令」上方写着「sccache 缓存 50G」。**文档声称的缓解措施与机器实际状态不符**，等于该风险零缓解。已在 §20.4 追加待处置。
3. **8 份 lock 各自解析**（277 包版本漂移，清单 `.tmp/cargo-lock-drift.txt`）+ **feature 统一不再跨 workspace**：即使 profile 对齐，同一依赖仍会因「哪个根在编、统一出哪组 feature」产生多份产物。这条**修不掉，只能靠减少全量次数或改回单 workspace 消除**——用户 2026-11-01 明确选择「保留 8 workspace，只修执行缺陷」，故本条登记为形态代价，不再尝试消除。

**本轮修复（已完成）**：`.tmp/apply-dev-profile-to-roots.mjs` 把 `src-tauri/Cargo.toml` 的 `[profile.dev]` + `[profile.dev.build-override]` **逐字**追加到 7 个子根（幂等脚本，LF 落盘，附「为什么每个根都要写一份 + 67m53s vs 9m24s 实测」注释）。复验（真 TOML 解析，非文本比对）：8 根的 `profile.dev` 与 `profile.dev.build-override` 表**完全相等** `{debug:"line-tables-only", incremental:false, opt-level:0, split-debuginfo:"unpacked"}` + `{codegen-units:16, debug:false, incremental:false, opt-level:0}`。
**未复制 `[profile.release]` / `[profile.release-android]`**：子根没有 release 入口，复制等于引入无人验证的配置；若 P1 真给每个子仓配独立发布，再各自决定。
**⚠ 副作用（已告知并被接受）**：profile 一改，target 目录里那批「默认 profile 产物」整批作废 ⇒ 下一轮全量必重编一次；收益在第二轮才可见。**尚未取得第二轮数字**（本机此刻有另一会话的 cargo 在跑，且见下条），故 67m53s → ? 的降幅当时只是推断；**同日凌晨已实测两轮，稳态 1m37s / 冷 41m36s，见 §20.9**。

**运行时测试队列的读数（`.tmp/cargo-test/summary.txt`，串行 `-j 3`，盘符路径）**：

| workspace | exit | 读数 | 归类 |
|---|---|---|---|
| axagent-ai | 101 | `linking with rust-lld failed` ×2（`axagent-agent` 的两个 test target）+ `axagent-dao (lib)` 编译失败 | 环境（与下方磁盘同窗口） |
| axagent-infra | 101 | **314 passed / 53 failed**，失败**逐条**都是 `SqliteError code 13 "database or disk is full"` | 环境（**不是**代码断言） |
| axagent-quant / plugins / runtime / src-tauri | 101 | `磁盘不足 (os error 112)` 写 `full.rmeta` / 建 `.fingerprint` 目录失败，15–90s 即退出 | 环境 |

⇒ **本轮未取得「搬家后路径断言」的运行时证据**：6 个根全部被磁盘填满打断，唯一真跑起来的 infra 面上 314 个用例（含 dao 的库测试）通过、53 个失败全是 `SQLITE_FULL`。**判据**：见到 `os error 112` / `database or disk is full` 一律归环境类，不得计入「测试失败 = 代码有问题」。
**磁盘现状**：C: 299G 分区此刻 97G free（`Get-CimInstance Win32_LogicalDisk`），即写满发生在 07:05–07:08 那个窗口、之后又被释放。共享 target 目录 `C:\Users\刘小平\.cargo\target-axagent` 由 8 个根 + dev/test 双 profile 共同写入，是这台机器上的首要容量风险；D: 尚有 240G。待处置：把 `CARGO_TARGET_DIR`（现为全局环境变量，覆盖仓内 `.cargo/config.toml` 的 `../src-tauri/target`）迁到 D:，再谈启用 sccache —— 在刚报过 112 的盘上再挂一个数十 G 的缓存目录是反向操作。

**④ 根因 1 的实物证据（2026-11-01 补测，共享 target 目录）**：`C:\Users\刘小平\.cargo\target-axagent\debug`

| 读数 | 值 | 含义 |
|---|---|---|
| `incremental/` 体积 | **68G** | `src-tauri` 早在 2026-08-31 就为躲 os error 5 僵尸会话把 `incremental` 设为 `false`，当时记录 **55G / 415 目录**；现已涨到 **879 目录**（160 个 crate，其中 **127 个 crate 会话数 > 1**）|
| 单 crate 会话数 top | `axagent_harness` **36**、`axagent_kit` 30、`axagent_entities` 27、`axagent_dao` 23 | 增量会话目录按 metadata hash 命名 ⇒ 一个 crate 有 36 个会话 = **同一个 foundation crate 被编了 36 种不同 (feature, profile) 组合** |
| `deps/` 产物 | 35383 个文件；`serde` 267 个不同 hash、`tokio` 197、`sea_orm` 45 | 第三方依赖同样按 hash 分裂（含历史累积，不全是本轮造成，但趋势与上表一致）|

⇒ 涨回来的直接原因与 §20.7 根因 1 同一件事：**7 个子根没有 `[profile.dev]` ⇒ `incremental` 退回默认 `true`**，于是每个子根每编一遍就落一个僵尸会话。这也解释了上面的磁盘读数：68G 的死重 + 8 根全量重编，先把 C: 写到 `os error 112`，再让门禁时间翻倍。
**待用户确认的清理动作（本轮未执行）**：删除 `target-axagent/debug/incremental/`（68G，profile 对齐后 `incremental=false`，这批目录**永不会再被复用**）与按 hash 作废的旧 `deps/` 产物；这是可再生缓存，但属仓库外、用户机器上的破坏性操作，须显式授权。

### 20.8 `check-workspace-config-parity.mjs` 的两处接线，与一次「本地镜像缺一道门」的实测复扫（2026-11-01）

**接线（两处同源，已完成）**

| 位置 | 落地形态 | 实测 |
|---|---|---|
| `scripts/ci-check.mjs` | `canRunFrontend` 段内 2 条 `step(...)`（自检 → 判定），紧跟领域本体两步 ⇒ **在 `--quick`（pre-push）路径上**，未加 `if (!quick)` 门控 | `node scripts/check-workspace-config-parity.mjs --selftest` = `9 项通过 / 0 项失败`（EXIT=0）；判定 = 8 根同源（EXIT=0，另有 1 条「src-tauri/.cargo 未定义 target-dir」只报告不拦） |
| `.github/workflows/ci.yml` | **文件末尾新增独立 job `workspace-config-parity`**（checkout + setup-node + 同两条命令） | `rust-check:` 仍在 564、`cargo-deny:` 仍在 981 ⇒ **零行号漂移** |

> **为什么不并进 `frontend-check`**：本文件被仓内 **18+ 处 `ci.yml:<行号>`** 引用（`scripts/check-contracts.mjs`、`scripts/check-horizon-label-parity.mjs:54`、`scripts/ci-check.mjs:16-21`、多份 `docs/audits/*`）。任何**中段插入**都会让其后全部引用整体移位 ⇒ 当场制造一整桶新 BROKEN（那正是 `PLAN-industry-to-domain-migration.md` 里 12/12 假信号的成因）。新 job 与 `frontend-check` 并行跑 ⇒ 「纯 Node 秒级 ⇒ 尽早失败」的性质不变，代价只是一次额外 checkout。
> **另**：`dprint.json` 的 `excludes` 含 `scripts/**/*` ⇒ 本仓 Node 门禁脚本**不受** dprint 格式门约束（别把「dprint 说 no files」当成格式化失败），语法用 `node --check` 自证。

**复扫捞出的既存缺陷（团队 L2 未提交改动引入，与本轮新增门无关）**

守卫测试 `src/__tests__/ci.workflow.test.ts`（对称差集护栏）无法从 UNC 共享目录跑（`No test files found`，root 被解析成 `Z: Folders/...`）⇒ 用本地副本 `C:\Users\Public\axinvest-vitest`（补 `.github/workflows/ci.yml` + `scripts/ci-check.mjs` + 该测试文件）跑，并**用 `git show HEAD:` 的两侧做基线对照**，才能把「红是谁造成的」分开：

| 两侧内容 | 守卫读数 |
|---|---|
| HEAD 版 ci.yml + HEAD 版 ci-check.mjs | **25 passed / 0 failed**（基线绿） |
| 工作树版（团队 L2 改动，尚未补接） | **1 failed**：差集恰为 `scripts/cargo-all-workspaces.mjs check`、`scripts/cargo-all-workspaces.mjs nextest` |
| 本轮补接后 | **25 passed / 0 failed**（EXIT=0） |

⇒ 真缺陷形态：**CI 把 fmt/clippy/check/nextest 统一改走 `cargo-all-workspaces.mjs`，但本地镜像只跟进了 fmt-check 与 clippy 两条**，`check` 与 `nextest` 只进了 CI —— 「本地绿冒充 CI 绿」，本仓反复出现的那一类。

**补法（为什么不是塞 ALLOWLIST）**

- `check`：直接补 `step("cargo check 构建检查（全部 workspace）", "node scripts/cargo-all-workspaces.mjs check")`。
- `nextest`：本机 `cargo nextest --version` 报 `error: no such command` ⇒ 命令**仍写进 `step(...)`**（提取器按文本比对，两侧同源这件事必须是真的），但执行前探测工具链，缺失时走 `skipStep(...)` 记进 `skipped` —— 按本脚本 2026-10-31 起的语义，「跳过 ≠ 通过」，结果段打黄色并逐条列出，`--strict-skip` 判失败。**不塞 ALLOWLIST 的理由**：护栏上限 8 条、现存 6 条，塞 2 条即触顶；AGENTS.md「CI 接线」节明写「不许为绕过护栏往 ALLOWLIST 塞新门」，而这里根本不需要绕过 —— 本地能诚实报「没跑」。
- 顺带修正同一注释块里的陈旧行号引用（**行数不变** ⇒ 不动其后被按行号引用的 `ci-check.mjs:316 / 347-351`）：`ci.yml:515` → `562`（`npm run contracts` 实际所在）；原「未覆盖」条目里的 `cargo check --workspace`（原引 `ci.yml:567`）与 `cargo nextest run --workspace`（原引 `ci.yml:679`）已不成立，改为只列 PG 集成三目标、E2E、`npm audit`、cargo audit / deny。

**新登记（只登记，未改）**

- **本地 CI 镜像此刻跑不到新门**：`node scripts/ci-check.mjs --quick` 在**第 4 步**即中断 —— `check-layer-discipline.mjs` 棘轮超基线 1 处：`commands-no-sibling-call  src-tauri/src/commands/master_key.rs  1 > 基线 0`（`master_key.rs:178` 调 `crate::commands::db_config::db_config_path()`）。该文件是 **10-31 23:15 新建、且 `git status` 为未跟踪**，其 `db_config_path` 的文档注释自陈「master_key 要读写同一份配置 ⇒ 路径必须同源，不许各写一份」——即**故意**的跨模块调用，但门禁按棘轮判它新越界。修法二选一（属在途会话的文件，本轮未动）：① 把 `db_config_path()` 上移到 `crate::paths`（两边都从 paths 取，既满足「路径同源」又不跨 commands 兄弟模块）；② 显式登记基线/豁免并写 reason。**在选出其一之前，`--quick` = pre-push 路径整体是红的，任何提交都过不去。**
  - 因此新门的「在镜像里真被执行」这一条**尚未取得运行时证据**，目前只有三条间接证据：两条命令单独跑绿（自检 9/9、判定 8 根同源）、`step(...)` 文本已进入 `canRunFrontend` 段且在 dprint 之前、守卫测试 25/25 认两侧同源。
- `scripts/check-horizon-label-parity.mjs:54` 声称该门注册在 `ci-check.mjs:316`，实测现落在 **324 / 325**（自检 + 判定两条）—— 一条**已腐烂**的行号引用，非本轮引入（本轮改动全在 669 行之后，以及等行数的注释块内替换）。
- `.github/workflows/ci-split.yml` 与 `.gitmodules` 仍在仓库根且为未跟踪状态（见 §20.6 埋雷 2），一次性 `git add .` 会把它们一起入库 —— 动手前仍需删除或移出，属用户文件，须授权。

### 20.9 两轮全量 clippy 实测：稳态 1m37s / 冷 41m36s（2026-11-01 上午，8 核本机）

**测法**：`.tmp/clippy-round.sh <轮次>`，参数与 CI 逐字同口径（`cargo clippy --workspace --all-targets --all-features -- -D warnings`，cwd = 各根），8 根串行，逐根记时间戳与 exit。原始读数 `.tmp/clippy-summary.tsv`，日志 `.tmp/clippy-r1-20261101-092727.log` / `.tmp/clippy-r2-20261101-100925.log`。轮 1 前共享 target 已被清空（另一侧删了 `debug/incremental/*`，C: 释放 ~62G），故轮 1 = 冷、轮 2 = 全暖。

| 根 | 轮 1（冷） | 轮 2（暖） |
|---|---|---|
| axagent-ai | 261s | 32s |
| axagent-core | 106s | 10s |
| axagent-infra | 174s | 9s |
| axagent-kit | 66s | 2s |
| axagent-plugins | 58s | 2s |
| axagent-quant | 173s | 3s |
| axagent-runtime | 291s | 9s |
| src-tauri | **1366s** | 29s |
| **合计** | **41m36s** | **1m37s**（各根 exit 0） |

**能定的结论（各条都指到具体读数）**：

1. **僵尸增量会话不再产生**：轮前 `incremental/` 会话数 `0` → 跑完 8 根仍 `0`（修复前该目录 879 个会话 / 68G）。这是 §20.7 根因 1 的修复**唯一能当场自证**的点，成立。体积代价也小：整轮只让 C: 从 149G 降到 146G free。
2. **稳态全量 = 1m37s**，且 `src-tauri`（只剩 2 个包的根）暖态 29s、冷态 1366s 占冷轮 **55%** —— 它编的是主 crate 本体，且必须等前 7 根产物就位 ⇒ 是串行尾巴。**压总时长的方向在这一根的并行化 / 编译缓存，不在拆分口径本身。**
3. **关闭一条既存量化待办**：AGENTS.md「已知缺口登记 2026-10-30」说 34 个 crate 未继承 `[lints] workspace = true` ⇒ 丢了 `module_name_repetitions = "allow"`，而它 warn-by-default，`-D warnings` 下**可能**是一条既存红灯。本轮 8 根全 `exit 0` ⇒ **实测不存在**。待办可从「须反复跑全量才能定论」改为「已定论：无」。
4. **§17 的 CI 目标（单作业 ≤5m）在本地稳态口径达成（1m37s ≪ 5m），但在 CI 上不成立**：CI runner 每次都是空缓存 ⇒ 实际付出的是轮 1 那个量级（41m36s），而本机与 CI 均无 sccache（§20.7 根因 2）。**「拆分让构建变快」这句话目前只能对「同一台机器上的第二轮及以后」说。**

**不能归因的部分（明说，不顺手记功）**：41m36s 相对旧的 67m53s「少了 26 分钟」**不能**归给 profile 修复 —— 旧那轮的缓存状态未知（是否暖、有多少现成产物都没记录），且**若**它经由 `cargo-all-workspaces.mjs`（修好之前会把 `--manifest-path` 追加在 `--` 之后 ⇒ clippy-driver 报 `error: Unrecognized option: 'manifest-path'`，见任务 #24），失败短路会同时改变工作量与耗时，那串 per-root 数字就不可作基线。**唯一可比的是同一份脚本、同参数的这两轮自身**，即上表；67m53s 已降级为「来源待证的旧读数」。

### §20.10 收尾：master_key 分层红解除 + cargo 命令组装自检接线（2026-11-01）

**① §20.8 登记的 `master_key.rs` 分层棘轮红已解除。**

- 病灶：`commands/master_key.rs` 取 `db_config.json` 路径时跨模块调 `crate::commands::db_config::db_config_path()`，命中 `check-layer-discipline.mjs` 的 `commands-no-sibling-call`（棘轮基线 0 ⇒ 1 条即红，且该门在 `--quick` 路径上 ⇒ **阻塞 pre-push**）。
- 正解不是加豁免，而是**把权威上移一层**：`src-tauri/src/paths.rs` 末尾新增 `pub fn db_config_path()`（该文件已是 `axagent_home()` 的同源权威），两个 commands 模块各自转调它。
- 两处等行数改动，专为「不制造新腐烂」：`commands/db_config.rs` 的公开函数改成同名私有转调（原文件行号不变 ⇒ 前端注释里 `db_config.rs:40-42 / 78-107 / 129-168 / 139-141` 那批行号引用一个都不移）；`paths.rs` 只在**文件末尾追加**（它同样被按行号引用）。
- 证据：`node scripts/check-layer-discipline.mjs` → 「模式: ratchet ｜ 结论: ✅ 通过」`EXIT=0`（自检 + 判定两条都在）；`rustfmt --check --edition 2024` 三文件 `EXIT=0`。

**② `--quick` 现在只剩一条红，且不属于本方案。**

复跑 `node scripts/ci-check.mjs --quick`（日志 `.tmp/ci-quick-after-master-key.log`）：**35 ✓ / 1 ✗**。✗ 是 `check-rhai-numeric-typing.mjs` 判据 a —— 另一会话**未提交**的 `src-tauri/src/commands/data-verifier.rhai`（工作区状态 `M`，本轮新增 66 行「阶段 3：1 月反转因子分覆盖 + 出票排序」）里嵌层数值判据写成 `type_of(x) == "f64" || type_of(x) == "i64"` 内联形态，未走 `num_of()` 桥。按改动边界不碰他人正在写的文件，**只登记**（任务 #25）。本轮的 `db_config_path()` 上移与之无关。

**③ 任务 #24 收尾：给 cargo runner 补自检并按「两处同源」接线。**

- `scripts/cargo-all-workspaces.mjs` 新增 `--selftest`：**11 项**，纯 Node 比对组装出来的 argv，不起 cargo。四条负控中最关键的一条是把 §20.9 提到的**旧写法（无条件追加）留在文件里当坏样本**，断言它必须违反不变量 —— 否则「正例全绿」只证明检法没电。另含 extra 参数（含 extra 自带 `--`）落位、rustc 侧只剩 `-D warnings`、本机 8 根逐条守序、0 根不得当通过。实测 `[cargo-all-workspaces --selftest] 11 项通过 / 0 项失败`，`EXIT=0`。
- 两处注册，命令行逐字同源：`scripts/ci-check.mjs` 紧贴三条 cargo step 之前（`if (canRunRust && !quick)` 块内）；`.github/workflows/ci.yml` **追加在文件末尾的 `workspace-config-parity` job 里** —— 该 job 已在末尾，往里加 11 行不动任何被引用的行：`rust-check:` 仍 `564`、`cargo-deny:` 仍 `981`、`workspace-config-parity:` 仍 `1026`，文件 1040 → **1051** 行，LF 未破。
- **守卫测试的正负两侧都验了**（本地镜像 `C:/Users/Public/axinvest-vitest`，UNC 上 vitest 会 `No test files found`）：接线后 `25 passed` / `VITEST_EXIT=0`；再把 ci.yml 那一行 `run:` 删掉重跑 ⇒ `× 真实数据：对称差集必须恰好等于 allowlist` 且差集里**点名的正是** `scripts/cargo-all-workspaces.mjs --selftest`，`VITEST_EXIT=1` ⇒ 提取器真能看见这道新门，不是「两边都没提到所以差集恒空」的空转绿。
- **代价明说**：这条自检不在 `--quick` 路径（往上挪 4 行会把本文件被文档按行号引用的锚点整体移走，制造一整桶 BROKEN）⇒ pre-push 不覆盖它，由 CI 的 parity job 与全量 `ci:check` 覆盖。它是**跑在 8 根 clippy 之前**的闸门，所以真省下的是「40 分钟编在一套坏参数上」那一档。

**④ 尚缺的最后一块证据**：8 根全量 `cargo test`（`__TAURI_WORKSPACE__=true`，逐根串行，harness 见 `.tmp/run-all-tests.sh`）。首根已回：`axagent-ai exit=0 / 546s`。
