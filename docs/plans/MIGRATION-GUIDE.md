# AxAgent 仓库拆分迁移手册

> 创建时间：2026‑10‑31  
> 适用范围：AxAgent 仓库拆分（Plan A：按技术层拆分）  
> 关联文档：`PLAN-repo-split.md` · `dependency-graph.md` · `dependency-matrix.md`

> ⚠ **本手册对应「轨道 2：物理拆仓」（可选）。执行前请先完成 `PLAN-repo-split.md` §11–§19 的「轨道 1：逻辑拆分」并使其全绿。**
> 根据实测依赖矩阵（`dependency-matrix.md`）与 L0 裁定，需注意：
> 1. 物理拆仓的子仓库为 **8 个**（`core` / `kit` / `infra` / `ai` / `quant` / `plugins` / `runtime` / `frontend`），`axagent-kit` 单列为公共实现层。
> 2. **`axagent-mobile` 不拆仓**：它是 `pub use axagent_lib::*;` 的 staticlib 包壳，依赖整个 app 是 Tauri 移动端设计使然，
>    与 `src-tauri` 同属**应用壳层**，留在主仓库（本手册中凡涉及「mobile 独立拆仓 / 解耦」的段落均已作废）。

---

## 1. 迁移总览

```
原仓库（monorepo）                    拆分后（8 个子仓库）
──────────────────                   ──────────────────────────
src/                                 →  axagent-frontend/
src-tauri/crates/harness/            →  axagent-core/crates/harness/
src-tauri/crates/entities/           →  axagent-core/crates/entities/
src-tauri/crates/plugin-proto/       →  axagent-core/crates/plugin-proto/
src-tauri/crates/disk-cache/         →  axagent-core/crates/disk-cache/
src-tauri/crates/rt-theme/           →  axagent-core/crates/rt-theme/
src-tauri/crates/dao/                →  axagent-infra/crates/dao/
src-tauri/crates/storage/            →  axagent-infra/crates/storage/
...（其余 infra crates）              →  axagent-infra/crates/...
src-tauri/crates/runtime/            →  axagent-runtime/crates/runtime/
src-tauri/crates/runtime-core/       →  axagent-runtime/crates/runtime-core/
src-tauri/crates/tools/              →  axagent-runtime/crates/tools/
src-tauri/crates/rt-*/               →  axagent-runtime/crates/rt-*/
src-tauri/crates/agent/              →  axagent-ai/crates/agent/
src-tauri/crates/providers/          →  axagent-ai/crates/providers/
...（其余 ai crates）                 →  axagent-ai/crates/...
src-tauri/crates/quant/              →  axagent-quant/crates/quant/
src-tauri/crates/market-sim/         →  axagent-quant/crates/market-sim/
...（其余 quant crates）              →  axagent-quant/crates/...
src-tauri/crates/plugins/            →  axagent-plugins/crates/plugins/
src-tauri/schema-gen/                →  axagent-plugins/schema-gen/
src-tauri/crates/mobile/             →  保留在应用壳层（不拆仓）
src-tauri/Cargo.toml（根）           →  各子仓库 Cargo.toml（独立 workspace）
src-tauri/src/                       →  各子仓库的 src/ 目录
.github/workflows/ci.yml             →  各子仓库的 CI + .github/workflows/ci-split.yml
package.json                         →  axagent-frontend/package.json
vite.config.ts                       →  axagent-frontend/vite.config.ts
.cargo/config.toml                   →  各子仓库的 .cargo/config.toml
scripts/                             →  根仓库保留共享脚本 + 各子仓库独立脚本
```

---

## 2. 子仓库职责与依赖关系

| 子仓库 | 职责 | 依赖 |
|--------|------|------|
| **axagent-core** | trait 契约、DTO、数据模型、插件协议 | 无内部依赖（叶子层） |
| **axagent-infra** | 数据库、存储、加密、搜索、遥测、设备 | core |
| **axagent-runtime** | 工具系统、工作流引擎、消息网关、运行时核心 | core + infra |
| **axagent-ai** | 智能体引擎、LLM 提供商、MCP、提示防护、轨迹 | core |
| **axagent-quant** | 量化策略、回测、行情数据、OPC | core + infra |
| **axagent-plugins** | 插件系统、schema 生成、代码生成 | core |
| **axagent-frontend** | React 前端、UI 组件、i18n | 独立（通过 Tauri IPC 契约） |
| **应用壳层**（留主仓，不拆） | `src-tauri` 的 wiring + `crates/axagent-mobile` staticlib 包壳 | 全部（组合根） |

```
axagent-core
    ↑
    ├── axagent-kit (公共实现层)
    ├── axagent-ai
    ├── axagent-plugins
    └── axagent-infra ──▶ kit
            ↑
        axagent-quant ──▶ ai
            ↑
        axagent-runtime
            ↑
        应用壳层 (src-tauri + mobile, 留主仓)

axagent-frontend（独立，通过 IPC 契约）
```

---

## 3. 各子仓库详细清单

### 3.1 axagent-core

| 项 | 值 |
|----|-----|
| **路径** | `axagent-core/` |
| **包含** | `crates/harness/` · `crates/entities/` · `crates/plugin-proto/` · `crates/disk-cache/` · `crates/rt-theme/` · `crates/axagent-agent-command-types/` · `crates/axagent-agent-macro/` |
| **Cargo workspace** | `axagent-core/Cargo.toml`（独立 workspace） |
| **负责人** | 架构师 |
| **关键文件** | `Cargo.toml` · `crates/harness/src/lib.rs` · `crates/entities/src/lib.rs` |

### 3.2 axagent-infra

| 项 | 值 |
|----|-----|
| **路径** | `axagent-infra/` |
| **包含** | `crates/dao/` · `crates/storage/` · `crates/migration/` · `crates/crypto/` · `crates/credential/` · `crates/cache/` · `crates/search/` · `crates/npm/` · `crates/document-parser/` · `crates/rt-webhook/` · `crates/device/` · `crates/telemetry/` |
| **依赖** | `axagent-core` |
| **负责人** | 基础设施团队 |

### 3.3 axagent-runtime

| 项 | 值 |
|----|-----|
| **路径** | `axagent-runtime/` |
| **包含** | `crates/runtime/` · `crates/runtime-core/` · `crates/tools/` · `crates/rt-messaging/` · `crates/rt-workflow/` |
| **依赖** | `axagent-core` + `axagent-infra` |
| **负责人** | 运行时团队 |

### 3.4 axagent-ai

| 项 | 值 |
|----|-----|
| **路径** | `axagent-ai/` |
| **包含** | `crates/agent/` · `crates/orchestrator/` · `crates/providers/` · `crates/mcp/` · `crates/prompt-guard/` · `crates/trajectory/` · `crates/kit/` |
| **依赖** | `axagent-core` |
| **负责人** | AI 团队 |

### 3.5 axagent-quant

| 项 | 值 |
|----|-----|
| **路径** | `axagent-quant/` |
| **包含** | `crates/quant/` · `crates/market-sim/` · `crates/astock-data/` · `crates/analysis-engine/` · `crates/company-runtime/` |
| **依赖** | `axagent-core` + `axagent-infra` |
| **负责人** | 量化团队 |

### 3.6 axagent-plugins

| 项 | 值 |
|----|-----|
| **路径** | `axagent-plugins/` |
| **包含** | `crates/plugins/` · `schema-gen/` |
| **依赖** | `axagent-core` |
| **负责人** | 插件团队 |

### 3.7 axagent-frontend

| 项 | 值 |
|----|-----|
| **路径** | `axagent-frontend/` |
| **包含** | `src/` · `package.json` · `vite.config.ts` · `tsconfig.json` · `index.html` |
| **依赖** | 独立（通过 Tauri IPC 契约与后端交互） |
| **负责人** | 前端团队 |

### 3.8 axagent-mobile（**不拆仓**，随应用壳层保留）

| 项 | 值 |
|----|-----|
| **归属** | 与应用壳层（`src-tauri`）同组，留在主仓库 |
| **原因** | `src/lib.rs` 仅 `pub use axagent_lib::*;`，是 Tauri 移动端 staticlib 包壳，依赖整个 app 属设计使然 |
| **负责人** | 应用/端上团队 |

### 3.9 axagent-kit（公共实现层）

| 项 | 值 |
|----|-----|
| **路径** | `axagent-kit/` |
| **包含** | `crates/kit/`（computer-use） |
| **依赖** | `axagent-core`（被 infra/runtime/ai 三方依赖） |
| **负责人** | 公共实现团队 |

---

## 4. 迁移操作步骤

### 4.1 阶段 0：准备工作

```bash
# 1. 创建子仓库的 GitHub 仓库（8 个）
#    - axagent-core
#    - axagent-kit
#    - axagent-infra
#    - axagent-runtime
#    - axagent-ai
#    - axagent-quant
#    - axagent-plugins
#    - axagent-frontend
#    （axagent-mobile 不拆仓，随 src-tauri 应用壳层保留）

# 2. 更新 .gitmodules 中的 URL 为真实仓库地址

# 3. 各子仓库创建独立 Cargo workspace 或 npm package
```

### 4.2 阶段 1：创建 axagent-core

```bash
# 从主仓库提取 foundation crates
mkdir -p axagent-core/crates
cp -r src-tauri/crates/harness axagent-core/crates/
cp -r src-tauri/crates/entities axagent-core/crates/
cp -r src-tauri/crates/plugin-proto axagent-core/crates/
cp -r src-tauri/crates/disk-cache axagent-core/crates/
cp -r src-tauri/crates/rt-theme axagent-core/crates/

# 创建独立 Cargo workspace
cat > axagent-core/Cargo.toml << 'EOF'
[workspace]
members = [
    "crates/harness",
    "crates/entities",
    "crates/plugin-proto",
    "crates/disk-cache",
    "crates/rt-theme",
]

[workspace.package]
version = "3.0.0"
edition = "2024"
license = "AGPL-3.0-only"

[workspace.dependencies]
axagent-harness = { path = "crates/harness" }
axagent-entities = { path = "crates/entities" }
axagent-plugin-proto = { path = "crates/plugin-proto" }
axagent-disk-cache = { path = "crates/disk-cache" }
axagent-rt-theme = { path = "crates/rt-theme" }

[profile.dev]
incremental = true

[build]
jobs = 16
EOF

# 调整各 crate 的 Cargo.toml，移除对其他子仓库的路径依赖
# （harness 和 entities 本身零依赖，无需修改）
```

### 4.3 阶段 2：创建 axagent-infra

```bash
mkdir -p axagent-infra/crates

# 复制 infra crates
for crate in dao storage migration crypto credential cache search npm document-parser rt-webhook device telemetry; do
    cp -r src-tauri/crates/$crate axagent-infra/crates/
done

# 创建 workspace Cargo.toml
# 调整各 crate 的 Cargo.toml 中的路径依赖：
#   axagent-harness = { path = "../harness" } → axagent-harness = "3.0.0"（从 registry）
#   axagent-entities = { path = "../entities" } → axagent-entities = "3.0.0"（从 registry）
```

### 4.4 阶段 3：创建 axagent-runtime

```bash
mkdir -p axagent-runtime/crates

for crate in runtime runtime-core tools rt-messaging rt-workflow; do
    cp -r src-tauri/crates/$crate axagent-runtime/crates/
done

# 调整路径依赖：
#   axagent-core 的依赖 → 从 registry 引用
#   axagent-infra 的依赖 → 从 registry 引用
```

### 4.5 阶段 4：创建 axagent-ai

```bash
mkdir -p axagent-ai/crates

for crate in agent orchestrator providers mcp prompt-guard trajectory kit; do
    cp -r src-tauri/crates/$crate axagent-ai/crates/
done

# 调整路径依赖（仅依赖 axagent-core）
```

### 4.6 阶段 5：创建 axagent-quant

```bash
mkdir -p axagent-quant/crates

for crate in quant market-sim astock-data analysis-engine company-runtime; do
    cp -r src-tauri/crates/$crate axagent-quant/crates/
done

# 调整路径依赖（依赖 axagent-core + axagent-infra）
```

### 4.7 阶段 6：创建 axagent-plugins

```bash
mkdir -p axagent-plugins/crates
cp -r src-tauri/crates/plugins axagent-plugins/crates/
cp -r src-tauri/schema-gen axagent-plugins/schema-gen/

# 调整路径依赖（仅依赖 axagent-core）
```

### 4.8 阶段 7：创建 axagent-frontend

```bash
mkdir -p axagent-frontend
cp -r src axagent-frontend/src
cp package.json axagent-frontend/
cp vite.config.ts axagent-frontend/
cp tsconfig.json axagent-frontend/
cp tsconfig.node.json axagent-frontend/ 2>/dev/null || true
cp index.html axagent-frontend/
cp tailwind.css axagent-frontend/ 2>/dev/null || true

# 调整 vite.config.ts 中的 alias：
#   "@" → path.resolve(import.meta.dirname, "./src")
# （保持不变，因为 src/ 仍在子仓库内）

# 创建独立的 package.json，移除对后端的依赖（IPC 通过 @tauri-apps/api 运行时注入）
```

### 4.9 阶段 8：`axagent-mobile`（**不迁移，随应用壳层保留**）

```bash
# 无操作：crates/axagent-mobile 留在 src-tauri 应用壳层，
# 它是 pub use axagent_lib::*; 的 staticlib 包壳，依赖整个 app 属设计使然。
```

### 4.10 阶段 9：更新主仓库

```bash
# 添加 .gitmodules
git add .gitmodules

# 移除已迁移的目录（保留 .github/workflows/ci-split.yml）
# 注意：不要删除 docs/ 和 scripts/（共享脚本保留）

# 更新根目录 README 指向各子仓库
# 更新 AGENTS.md 记录新子仓库路径
```

---

## 5. CI 配置

### 5.1 主仓库 CI

主仓库保留 `.github/workflows/ci-split.yml`，为每个子仓库触发独立 CI 作业：

```yaml
jobs:
  core-check:      # axagent-core
  kit-check:       # axagent-kit
  infra-check:     # axagent-infra
  runtime-check:   # axagent-runtime
  ai-check:        # axagent-ai
  quant-check:     # axagent-quant
  plugins-check:   # axagent-plugins
  frontend-check:  # axagent-frontend
  # 应用壳层（含 mobile）由主仓 ci.yml 覆盖，不单列作业
```

### 5.2 子仓库独立 CI

每个子仓库在自己的仓库中维护独立的 CI workflow，包含：

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
```

### 5.3 缓存策略

| 缓存 | 工具 | 键 |
|------|------|-----|
| Rust 编译缓存 | sccache | `ubuntu-2404-rust-check` |
| Rust 依赖缓存 | Swatinem/rust-cache | `ubuntu-2404-{crate-name}` |
| npm 缓存 | actions/setup-node cache | `npm` |
| Vite 缓存 | 本地 `.vite/` | 项目内 |

---

## 6. 开发工作流

### 6.1 日常开发

```bash
# 初始化子仓库（首次）
git submodule update --init --recursive

# 或使用一键脚本
./scripts/setup-submodules.sh

# 进入子仓库开发
cd axagent-core
cargo check

# 或使用统一入口
./scripts/dev.sh core
./scripts/dev.sh frontend
./scripts/dev.sh infra

# 跨子仓库调试
./scripts/dev.sh runtime test -p axagent-rt-workflow
```

### 6.2 提交与 PR

```bash
# 在子仓库中提交
cd axagent-ai
git add .
git commit -m "feat(agent): 新增智能体编排策略"
git push origin HEAD:feature/agent-orchestration

# 在主仓库中更新子模块指针
cd ..
git add axagent-ai
git commit -m "chore(submodule): 更新 axagent-ai 到最新"
```

### 6.3 版本发布

```bash
# 各子仓库独立版本号
# 在 axagent-core 发布
cd axagent-core
cargo publish --registry internal  # 或 git tag v3.0.0

# 在 axagent-frontend 发布
cd ../axagent-frontend
npm version patch
npm publish --registry internal
```

---

## 7. 回滚方案

### 7.1 完全回滚

如果拆分后出现严重问题，可通过以下方式回滚：

```bash
# 1. 移除 .gitmodules
git rm .gitmodules

# 2. 合并子仓库回主仓库
for repo in core kit infra runtime ai quant plugins frontend; do
    git subtree add --prefix=axagent-$repo axagent-$repo-url main --squash
done

# 3. 恢复原 CI 配置
git checkout HEAD -- .github/workflows/ci.yml

# 4. 清理子仓库目录
rm -rf axagent-*
```

### 7.2 部分回滚

如果仅某个子仓库有问题，只需将其从 `.gitmodules` 中移除，并恢复该部分代码到主仓库。

---

## 8. 检查清单

### 迁移前

- [ ] 所有子仓库的 GitHub 仓库已创建
- [ ] `.gitmodules` URL 已更新为真实地址
- [ ] 各子仓库负责人已确认
- [ ] CI 并行流水线已配置

### 迁移中

- [ ] axagent-core 已创建并编译通过
- [ ] axagent-kit 已创建并编译通过
- [ ] axagent-infra 已创建并编译通过
- [ ] axagent-runtime 已创建并编译通过
- [ ] axagent-ai 已创建并编译通过
- [ ] axagent-quant 已创建并编译通过
- [ ] axagent-plugins 已创建并编译通过
- [ ] axagent-frontend 已创建并编译通过
- [ ] 应用壳层（含 mobile）仍在主仓并编译通过
- [ ] 各子仓库的 CI 已独立运行
- [ ] 本地 `scripts/dev.sh` 工作正常

### 迁移后

- [ ] 根仓库 `.gitmodules` 正确
- [ ] CI 并行作业全部通过
- [ ] 本地开发体验已验证
- [ ] AGENTS.md 已更新
- [ ] 各子仓库 README 已编写
- [ ] 团队已培训新工作流
