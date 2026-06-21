# AxAgent — AGENTS.md

## 核心指令

全程中文：思考、注释、输出一律简体中文。代码注释优先中文。无论用户用什么语言提问，都必须用中文思考 + 中文回答。

## 项目概述

AxAgent 是 Tauri v2 + React 19 + TypeScript 跨平台 AI 桌面客户端。支持多 LLM 提供商、智能体引擎、工作流编辑、RAG 知识库、浏览器扩展等。

## 技术栈

前端：React 19 · TypeScript (strict) · Vite 8 · Zustand 5 · Ant Design 6 + Ant Design X · Tailwind CSS 4 · React Router v7 · react-i18next (11 种语言, 默认/回退均为 zh-CN) · Vitest + Playwright
后端：Rust 2021 · Tauri 2 · Tokio (full) · Sea-ORM (SQLite) · reqwest · tracing · thiserror
格式化：dprint (TS/JSON, 双引号+强制大括号) · rustfmt (max_width=100, tab_spaces=4)
注意：Tailwind 4 使用 `@tailwindcss/vite` 插件，不要创建 tailwind.config.js 或 postcss.config.js

## 目录架构

```
src/                         # React 前端 (npm run dev 浏览器模式走 localStorage mock)
├── components/              # chat/ workflow/ settings/ files/ skill/ terminal/ gateway/ layout/
├── pages/                   # 路由页面
├── stores/                  # Zustand 状态，四层分类：
│   ├── domain/              # 核心业务：conversation, message, stream, preference, workspace
│   ├── feature/             # 功能模块：provider, agent, skill, mcp, terminal, gateway, ...
│   ├── shared/              # 跨组件共享：ui, tab, artifact, chatWorkspace
│   └── devtools/            # 调试工具：tracer, evaluator, rl, fineTune
├── hooks/ lib/ types/ i18n/ theme/

src-tauri/                   # Rust 后端 (Cargo workspace)
├── src/                     # 主 crate
│   ├── lib.rs               # run() + generate_handler![] 注册所有命令
│   ├── commands/            # 67 个命令模块，mod.rs 统一声明
│   ├── init/                # 初始化（database, plugins, services, state）
│   └── app_state.rs         # 全局 AppState
├── crates/                  # 29 个 workspace crate
│   ├── core/                # 数据库实体、向量存储、RAG、加密
│   ├── agent/               # 智能体引擎（SessionManager 等）
│   ├── providers/           # LLM 提供商抽象层
│   ├── runtime/             # WebSocket、工作流引擎、消息网关
│   ├── gateway/             # API 网关（Axum, OpenAI 兼容接口）
│   ├── tools/               # 工具系统（注册/验证/执行）
│   ├── trajectory/          # 轨迹记录、RL 引擎、技能进化
│   └── ... (code_engine, migration, plugins, telemetry, acp)

extension/ website/ e2e/ scripts/
```

## 代码规范

### 前端 (TypeScript/React)

- 组件：函数组件 + 命名导出（`export function Foo() {}`），禁止默认导出
- Store 模式：`export const useXxxStore = create<State>((set, get) => ({}))`，在 stores/index.ts re-export
- 类型：所有类型从 `@/types` 导入（barrel export），不从子文件导入
- i18n：UI 文本一律 `const { t } = useTranslation()`，禁止硬编码字符串
- 路径别名：`@/` = `src/`
- 样式：首选 Ant Design theme token + Tailwind 工具类，避免新建 CSS 文件
- **dprint 格式化**：`npm run format`（即 `dprint fmt`）必须通过，CI 强制检查，禁止提交未格式化的 TS/JSON 代码

### 后端 (Rust)

- Tauri 命令返回 `Result<T, String>`，用 `.map_err(|e| e.to_string())`
- 库 crate 错误用 `#[derive(thiserror::Error)]`，应用层（lib.rs/commands）用 `anyhow::Result`
- 模块可见性：内部用 `pub(crate)`，对外 API 用 `pub`
- 所有 `pub mod` 声明在 commands/mod.rs 中统一管理
- 数据库操作：方向在 entity 层用 sea-orm，有复杂查询逻辑用 repository 模式
- **rustfmt 格式化**：`cargo fmt` 必须通过，CI 强制检查，禁止提交未格式化的 Rust 代码
- **clippy 零警告**：`cargo clippy -- -D warnings` 必须通过，CI 强制检查，禁止提交含 clippy 警告的代码

## 常用命令

```
npm run dev           # Vite 前端（浏览器模式，走 localStorage mock）
npm run tauri dev     # 完整 Tauri 桌面应用
npm run typecheck     # tsc --noEmit
npm run test:run      # Vitest 单元测试
npm run test:e2e      # Playwright E2E 测试
npm run format        # dprint 格式化前端
npm run build         # tsc + vite build 生产构建
cargo fmt             # rustfmt 格式化（src-tauri/ 下执行）
cargo clippy          # Rust lint（src-tauri/ 下执行）
npm run bump          # 版本号升级
```

## 禁区（必须遵守）

### 前端

1. **IPC 调用**：必须通过 `@/lib/invoke` 的 `invoke<T>()`，禁止直接 `import { invoke } from "@tauri-apps/api/core"`
2. **国际化**：新增 UI 文本必须在 locales/ 下全部 11 种语言文件中添加 key，禁止仅添加 zh-CN
3. **类型导入**：从 `@/types` 导入，不从子路径（如 `@/types/agent`）导入
4. **组件导出**：命名导出，不用默认导出（`export function X` ✓，`export default function X` ✗）
5. **Tailwind**：不要创建 tailwind.config.js 或 postcss.config.js（Tailwind 4 用 vite 插件方式）
6. **Monaco Editor**：新增语言高亮必须在 vite.config.ts 的 `SHIKI_ALLOWED_LANGS` 中添加，不加入白名单不会打包

### 后端

7. **命令注册（两步）**：新增 Tauri 命令必须同时改 `commands/mod.rs`（声明模块）+ `lib.rs`（generate_handler![] 注册），缺一不可
8. **异步锁**：必须 `tokio::sync::RwLock`，禁止 `std::sync::RwLock`（std guard 跨 await 是 UB，panic 会毒化）
9. **异步运行时**：不要在已有 tokio runtime 上下文中再创建嵌套 runtime
10. **数据库迁移**：新增/修改表结构必须写 migration，不要直接改 entity 了事

### 构建

11. **removeCrossorigin()**：vite.config.ts 中此插件不可删除（Tauri 自定义协议不支持 CORS 预检，删除会导致生产白屏）

### 全栈（禁止重复代码）

12. **禁止重复定义**：写新代码前，必须先检索项目是否已有相同或相似的定义（trait / struct / enum / type / interface / 工具函数 / 常量）。规则：

- **已有定义 → 必须复用**：通过 `pub use` re-export 或 `import` 引用已有定义
- **需要扩展 → 扩展而非重定义**：给已有类型加字段/方法，不要新建同义类型
- **后端权威来源层级**：数据模型在 `runtime-core`（底层）→ `tools`/`agent`（上层 re-export 或扩展）。`core` 是最基础层，`runtime-core` 是其上的运行时抽象层
- **前端权威来源**：类型定义在 `src/types/`，store 应 import 而非重定义 interface
- **Tauri 命令一致性**：删除后端命令时必须同步清理前端调用方；新增命令时前端类型须与后端 DTO 一致
- **删除前确认零引用**：删除任何文件/模块/函数/命令前，用 grep 确认全项目引用为零

## Store 分类规则

新增 Zustand store 按以下规则放置：

- 核心业务（消息、会话、流式）→ `stores/domain/`
- 功能特性（网关、技能、终端、知识库）→ `stores/feature/`
- 跨组件 UI 状态（标签页、侧栏、工作区布局）→ `stores/shared/`
- 开发调试图表（追踪、评估、RL）→ `stores/devtools/`

## Git 规范

Conventional Commits + 中文描述。类型映射：
`feat` → 🚀 新功能 | `fix` → 🐛 Bug 修复 | `refactor` → 🔨 重构 | `style` → 🎨 样式
`docs` → 📝 文档 | `test` → 🧪 测试 | `chore` → 📦 杂项 | `ci` → 🔧 CI/CD
`build` → 🏗️ 构建 | `perf` → ⚡ 性能提升

## 上游合并流程（必须严格遵循）

合并 upstream/master 前必须执行 `bash scripts/upstream-merge.sh`，禁止手工 git merge。流程要点：

### 前置规则

1. **locale 文件优先提交**：合并前如有 `src/i18n/locales/` 的修改，必须先 `git add && git commit`，禁止 stash locale 文件
2. **非 locale 修改**：可以 stash，合并后必须 pop 回来验证
3. **禁止跳过 CI**：合并后必须通过 `node scripts/ci-check.mjs --quick` + `bash scripts/check-hardcoded-i18n.sh --diff-only`

### 完整步骤（由脚本自动执行）

```
# 一条命令完成全部
bash scripts/upstream-merge.sh
```

脚本自动执行：

1. 前置检查：确认 upstream remote、工作区状态
2. locale 保护：检测 locale 修改 → 要求先提交
3. 非 locale 修改 → stash（合并后自动 pop）
4. 拉取上游 + 检查 commit 列表
5. 合并前运行 dprint + cargo fmt
6. 执行 merge
7. 合并后：dprint + cargo fmt + tsc + i18n key 完整性 + 硬编码字符串检查
8. fmt 修复自动提交

### 手工合并（仅脚本不可用时）

如必须手工操作：

1. `git fetch upstream --prune`
2. 检查 `src/i18n/locales/` 有未提交变更 → **先提交，绝不 stash**
3. 其他文件有未提交变更 → `git stash push -m "msg"`
4. `git merge upstream/master`
5. `npm run format && (cd src-tauri && cargo fmt)`
6. 运行 `node scripts/ci-check.mjs --quick` — i18n key 缺失必须补全
7. 运行 `bash scripts/check-hardcoded-i18n.sh --diff-only` — 零新增硬编码
8. `git stash pop`（如有 stash）

### 合并后验证清单

- [ ] `node scripts/ci-check.mjs --quick` 全部通过
- [ ] `bash scripts/check-hardcoded-i18n.sh --diff-only` 零违规
- [ ] `(cd src-tauri && cargo fmt --check)` 通过
- [ ] 所有 stash 已恢复
- [ ] 应用能正常启动（`npm run dev` / `npm run tauri dev`）
