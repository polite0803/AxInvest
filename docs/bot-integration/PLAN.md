# PLAN — Bot 集成 P1 纵向切片

> 状态：active ｜ 日期：2026-11-22 ｜ 对应基线：`11f32873b` ｜ 来源：本地原创 ｜ 范围：`docs/bot-integration/`

## 0. P1 目标与非目标

**目标（一条能上线的纵向切片）**

> 用户能新建一个 Bot，给它写 SOUL、选模型、指工作目录；侧栏能看到它；在这个 Bot 下开线程对话；该对话的系统提示词注入这个 Bot 的 SOUL 与 MEMORY.md；用户能在 Bot 详情里查看 SOUL 与记忆。

**非目标（明确不做，防范围膨胀）**

- 不做 Bot 级审批执行（字段可编辑，标注「P2 生效」）。
- 不做例程、应用事件自动化。
- 不做团队/部门、Chief、共享模板。
- 不做 Bot 间委派运行时。
- 不做 Bot API key。
- 不做每 Bot 技能私有化。
- 不做记忆写入/编辑（P1 只读查看）。
- 不做 `agent_profiles` 的任何语义变更。

**完成定义**：五个任务卡（T1–T5）全部合入、T6 门禁全绿、本地提交（不 push）。

## 1. 任务依赖与并行

```text
T1 存储 ──► T2 IPC ──┬──► T3 侧栏/路由（可与 T4 并行）
                     ├──► T4 线程归属（依赖 T2 的 create/list）
                     └──► T5 SOUL/MEMORY 注入（依赖 T1 的实体与目录）
T6 门禁：全程伴随，最后一次性收口
```

- 后端线：T1 → T2 → T5（T5 的提示词部分可与 T2 并行）。
- 前端线：T3 可用 mock 契约先开工；T4 等 T2 的 `list_bot_threads` 就绪。
- 门禁线：T6 在每个任务卡末尾跑定点命令，最后跑全量。

## 2. 任务卡

### T1 数据模型与存储

| 项 | 内容 |
|---|---|
| 目标 | `bots` 表可建、可读写；`conversations.bot_id` 可空列就位；记忆/SOUL 目录可创建 |
| 责任线 | **Bot 运行时后端工程师**（主）；Bot 门禁与验证工程师（验收） |
| 依赖 crate | `axagent-entities`（新实体）、`axagent-dao`（repo）、`axagent-agent`（BotWorkspace，若同批） |

**落点文件**

| 动作 | 文件 |
|---|---|
| 新增 | `axagent-core/crates/entities/src/bots.rs`（按 `MODEL.md` §2.2） |
| 修改 | `axagent-core/crates/entities/src/lib.rs`：加 `pub mod bots;` |
| 修改 | `axagent-core/crates/entities/src/conversations.rs`：加 `bot_id: Option<String>`（`#[sea_orm(indexed)]`） |
| 新增 | `axagent-infra/crates/dao/src/repo/bot.rs` |
| 修改 | `axagent-infra/crates/dao/src/repo.rs`：`pub mod bot;`（按现有模块声明方式） |
| 修改 | `axagent-infra/crates/dao/tests/schema_compliance.rs:15`：`SCHEMA` 加 `bots` 全列 + `conversations` 的 `bot_id` |
| 新增 | `axagent-ai/crates/agent/src/bot_workspace.rs` |
| 修改 | `axagent-ai/crates/agent/src/lib.rs`：`pub mod bot_workspace;` |

**测试**

- `dao/src/repo/bot.rs` 末尾内联 `#[cfg(test)] mod tests`（必须追加到文件末尾，避免 `items_after_test_module`）：CRUD、置空语义、`delete_bot` 解绑线程、`list_bot_threads` 过滤归档。
- `bot_workspace.rs` 内联测试：原子写、hash 稳定、路径拼接。

**验收命令**

```bash
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-migration
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --test schema_compliance
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --lib missing_versions_is_empty
cargo clippy --manifest-path axagent-infra/Cargo.toml -p axagent-dao --all-targets --all-features -- -D warnings
cargo clippy --manifest-path axagent-ai/Cargo.toml -p axagent-agent --all-targets --all-features -- -D warnings
```

**DoD**：空库启动后 `bots` 表存在且列齐全；存量库启动日志无 `DROP`/`heal` 异常；`schema_compliance` 覆盖新表；无新增 migration 文件。

---

### T2 Bot CRUD IPC

| 项 | 内容 |
|---|---|
| 目标 | 9 个命令可从前端调用；错误码齐全；SOUL 写盘与漂移可测 |
| 责任线 | **Bot 运行时后端工程师**（主）；Bot 门禁与验证工程师（契约） |
| 依赖 crate | `axagent`（主 crate，wiring）、`axagent-dao`、`axagent-agent` |

**落点文件**

| 动作 | 文件 |
|---|---|
| 新增 | `src-tauri/src/commands/bot.rs`（按 `IPC.md` §2） |
| 修改 | `src-tauri/src/commands/mod.rs`：`pub mod bot;` |
| 修改 | `src-tauri/src/commands/error_code.rs`：`pub mod bot` + 11 个码 |
| 修改 | `src-tauri/src/register_commands.rs`：**不改**（构建产物自动生成） |

**关键实现点**

- `create_bot`：校验 → 生成 UUID → 插 DB → `BotWorkspace::ensure()` → 原子写 `SOUL.md` → 算 `soul_hash` 回写。
- `update_bot`：`Option<Option<T>>` 语义（`None`=不改，`Some(None)`=置空）；改 soul 时同步镜像与 hash。
- `write_bot_soul`：先写镜像（失败即返回 `BOT_SOUL_WRITE_FAILED`，不写 DB），成功后再写 DB，避免「DB 新、文件旧」。
- `read_bot_soul`：读 DB + 读镜像算 hash；不等则 `drift=true`，不自动覆盖。
- `read_bot_memory`：`ProjectMemory::new(bot_root).load_index()` + 扫描 `memory/**/*.md`；预算常量从 `ProjectMemory` 暴露（若未公开，加只读 getter，勿复制魔法数）。
- `delete_bot`：事务内删行 + `conversations.bot_id = NULL`；返回解绑线程数与记忆目录路径。

**测试**

- 命令层单元测试：校验失败分支（空名、超长、非法审批级别、model/provider 单边、workdir 不存在）。
- `soul_drift`：写 DB 后手改文件 → 读命令返回 `drift=true`。

**验收命令**

```bash
npm run contracts -- --only=a,d,f,g,h
npm run check:layers
cargo clippy --manifest-path src-tauri/Cargo.toml -p axagent --all-targets --all-features -- -D warnings
cd axagent-plugins && cargo run -q -p schema-gen -- check
```

> `schema-gen check` 必须在 `axagent-plugins` 目录执行；它校验 IPC 类型快照字节级无漂移。

**DoD**：`register_commands.rs` 自动出现 9 个命令；`contracts` A 段绿；错误码在 11 语言 `error` 段齐备；`check:layers` 绿。

---

### T3 Bot 侧栏与路由

| 项 | 内容 |
|---|---|
| 目标 | 侧栏出现「机器人」入口；`/bots` 页可列 Bot、创建、编辑、删除；空态可用 |
| 责任线 | **Bot 前端工程师**（主）；Bot 门禁与验证工程师（路由/门禁） |
| 依赖 | T2 的命令契约（未就绪时按 `IPC.md` 类型 mock） |

**前端路由**

| 项 | 值 |
|---|---|
| 路径 | `/bots` |
| pageRegistry | `src/lib/pageRegistry.ts:31` `BUILTIN_PAGE_PATH` 加 `bots: "/bots"` |
| 导航项 | `src/lib/navRegistry.tsx:46` 加 `{ key: "bots", path: BUILTIN_PAGE_PATH.bots, icon: <RobotOutlined />, labelKey: "nav.bots" }`（图标必须用 `@ant-design/icons` 本地组件） |
| 分组 | `src/lib/navGroups.ts:44-51` 的 `chat` 组 `itemKeys` 加 `"bots"` |
| 路由挂载 | `src/components/layout/ContentArea.tsx:21-52` 加 `LazyBotsPage`；`:174+` 的 `<Routes>` 加 `<Route path={BUILTIN_PAGE_PATH.bots} element={<SafeLazyPage Page={LazyBotsPage} />} />` |

**新增文件**

| 文件 | 职责 |
|---|---|
| `src/pages/BotsPage.tsx` | 两栏：左 Bot 列表（含「新建」），右详情/表单；组合各组件并传 props |
| `src/components/bot/BotList.tsx` | 列表项（图标、名称、标题、线程数、启用态） |
| `src/components/bot/BotEditor.tsx` | 创建/编辑表单（名称、标题、描述、SOUL、模型、工作目录、审批级别标注） |
| `src/components/bot/BotDetail.tsx` | 详情壳（Tab：概览 / SOUL / 记忆 / 线程） |
| `src/components/bot/BotSoulPanel.tsx` | SOUL 查看 + 「编辑」开关（P1 允许编辑 SOUL） |
| `src/components/bot/BotMemoryPanel.tsx` | MEMORY.md 只读 + 预算条 + 主题清单 |
| `src/components/bot/BotEmptyState.tsx` | 空态 |
| `src/stores/feature/botStore.ts` | `bots/loading/error/activeBotId` + `loadBots/createBot/updateBot/deleteBot/readSoul/writeSoul/readMemory` |
| `src/types/bot.ts` | `IPC.md` §3 的类型 |

**组件中立性**：`src/components/bot/**` 只允许 import `stores/feature/botStore`（同名 feature）与中性目录；**不要**在 `components/chat/**` 里 import `botStore` 或 `components/bot/**`。跨模块通过 `pages/**` 组合与 props 传递。

**i18n（11 语言）**

顶层新增 `nav.bots` 与 `bots.*`；源文案 zh-CN：

| key 组 | keys |
|---|---|
| 导航 | `nav.bots` = 机器人 |
| 页面 | `bots.title`、`bots.newBot`、`bots.createTitle`、`bots.editTitle`、`bots.empty`、`bots.emptyDescription`、`bots.createFirst` |
| 表单 | `bots.name`、`bots.namePlaceholder`、`bots.nameRequired`、`bots.role`、`bots.rolePlaceholder`、`bots.soul`、`bots.soulPlaceholder`、`bots.model`、`bots.modelPlaceholder`、`bots.workdir`、`bots.workdirPlaceholder`、`bots.avatar`、`bots.approvalMode`、`bots.approvalModeP2Hint` |
| 动作 | `bots.save`、`bots.saved`、`bots.saveFailed`、`bots.createSuccess`、`bots.createFailed`、`bots.delete`、`bots.deleteConfirm`、`bots.deleteSuccess`、`bots.deleteFailed` |
| 线程 | `bots.allBots`、`bots.unassigned`、`bots.threads`、`bots.newThread`、`bots.threadsEmpty`、`bots.threadsEmptyDescription` |
| 记忆/SOUL | `bots.soulTitle`、`bots.soulEmpty`、`bots.soulLoadFailed`、`bots.memory`、`bots.memoryTitle`、`bots.memoryEmpty`、`bots.memoryLoadFailed`、`bots.memoryBudget`、`bots.memoryBudgetHint` |

共 42 个 key（含 `nav.bots`）。`bots.deleteConfirm` 文案必须含「线程与记忆会保留」。

**测试**

| 文件 | 覆盖 |
|---|---|
| `src/stores/feature/__tests__/botStore.test.ts` | mock `@/lib/invoke`，CRUD 与错误分支 |
| `src/components/bot/__tests__/BotList.test.tsx` | 列表渲染、空态、点击选中 |
| `src/pages/__tests__/BotsPage.test.tsx` | 创建流程（表单 → createBot → 列表更新） |
| `src/lib/__tests__/pageRegistry.test.ts:14-26` | 补 `/bots` 断言 |

**验收命令**

```bash
npm run check:routes:strict
npm run test:run -- src/lib/__tests__/pageRegistry.test.ts
npm run test:run -- src/stores/feature/__tests__/botStore.test.ts
npm run test:run -- src/pages/__tests__/BotsPage.test.tsx
npm run typecheck
npm run check:i18n:key-parity
```

**DoD**：侧栏可进 `/bots`；创建后列表即时出现；11 语言 key 路径完全一致；`check:routes:strict` 绿（只加 Route 不加 nav 项会红）。

---

### T4 每 Bot 独立线程

| 项 | 内容 |
|---|---|
| 目标 | 在 Bot 下新建的会话带 `bot_id`；Bot 详情能看到自己的线程；聊天侧栏可按 Bot 过滤；工作目录落到 Bot 的 `workdir` |
| 责任线 | **Bot 前端工程师**（主）+ **Bot 运行时后端工程师**（小改） |
| 依赖 | T2 的 `list_bot_threads`、T1 的 `conversations.bot_id` |

**后端落点**

| 动作 | 文件 | 改动 |
|---|---|---|
| 修改 | `src-tauri/src/commands/conversation*`（`create_conversation` 所在模块） | 新增可选参数 `bot_id: Option<String>`：写 `conversations.bot_id`；若 Bot 有 `workdir` 且该会话尚无 session，则创建 `agent_sessions.cwd = workdir` |
| 修改 | `src-tauri/src/commands/agent/mod.rs:1942-1964` | 无 Bot 时行为不变；有 Bot 时优先用 Bot `workdir`（若 session.cwd 为空） |

> 若改 `create_conversation` 会牵动既有调用方，允许改为「创建后立即 `update_conversation` 设置 `botId`」的前端两步法作为 P1 兜底，但必须在 T4 结束前二选一并写进提交说明。

**前端落点**

| 动作 | 文件 | 改动 |
|---|---|---|
| 修改 | `src/stores/domain/conversationStore.ts` | 创建会话时接受 `botId`；`Conversation` 类型加 `botId` |
| 修改 | `src/types/index.ts` | `Conversation.botId?: string \| null`（camelCase，与后端 `bot_id` 对齐） |
| 修改 | `src/components/chat/ChatSidebar.tsx:415-433` | 搜索过滤后追加 Bot 过滤（`activeBotId` 由 `ChatPage` 传入，不在组件内读 `botStore`） |
| 修改 | `src/components/chat/ChatSidebar.tsx:341-384` | 新建会话传 `botId` |
| 修改 | `src/pages/ChatPage.tsx` | 读 URL `?bot=`，传 `activeBotId` 给 `ChatSidebar` |
| 修改 | `src/lib/browserMock.ts:2633-2704` | `create_conversation` / `update_conversation` 支持 `botId`；新增 9 个 bot 命令的 mock |

**测试**

- store：`createConversation({ botId })` 落库参数正确。
- 组件：给定 `activeBotId`，列表只显示该 Bot 会话 + 「未归属」开关。
- e2e：`e2e/bots.spec.ts` 覆盖「新建 Bot → 在其下建线程 → 刷新后线程仍在 Bot 下」。

> **CI 陷阱**：CI 的 Playwright 用 `--grep-invert "Agent|Cache|Conversation|Gateway"` 排除用例标题（`.github/workflows/ci.yml:952`）。新用例标题**不要**含这四个词（含 `Conversation` 也会被跳过），用「bot / 机器人 / 线程」表述。

**验收命令**

```bash
npm run test:run -- -t "bot"
npx playwright test e2e/bots.spec.ts --project=chromium
npx playwright test e2e/bots.spec.ts --project=chromium --grep-invert "(Agent|Cache|Conversation|Gateway)"
npm run contracts -- --only=g
```

**DoD**：Bot 详情线程数 = 该 Bot 会话数；聊天侧栏按 Bot 过滤正确；浏览器模式（mock）不出现「过滤后恒空」；e2e 在 CI 的 grep-invert 口径下仍被执行。

---

### T5 SOUL/MEMORY 注入与查看

| 项 | 内容 |
|---|---|
| 目标 | Bot 会话的 system prompt 含 `<bot-soul>` 与 `<bot-memory>`；Bot 会话不再注入全局 persona；Bot 详情可查看 SOUL 与 MEMORY.md |
| 责任线 | **Bot 运行时后端工程师**（主）；**Bot 前端工程师**（查看面板）；Bot 门禁与验证工程师（回归） |
| 依赖 | T1（实体/目录）、T2（读写命令）、T4（会话有 `bot_id`） |

**后端落点**

| 动作 | 文件 | 改动 |
|---|---|---|
| 修改 | `src-tauri/src/commands/agent/skill_execution.rs:614` | `build_agent_system_prompt` 增加 `bot_soul: Option<&str>`、`bot_memory: Option<&str>`；在 persona 之前输出 `<bot-soul>`，在 workspace/RAG 之后输出 `<bot-memory>` |
| 修改 | `src-tauri/src/commands/agent/mod.rs:647` | 有 `bot_id` 时：读 Bot → `persona_prompt = None`，`bot_soul = Some(bot.soul)`；`bot_memory = ProjectMemory::new(bot_root).load_index()` 的渲染结果 |
| 修改 | `src-tauri/src/commands/agent/skill_execution.rs:874` | 第二个调用点同步传参（技能执行路径） |
| 修改 | `src-tauri/src/commands/agent/mod.rs:1971` | 调用点传参 |

**注入顺序（固定）**

```text
<bot-soul> → <agent-profile> → <user-custom-prompt> → 默认指令
→ <workspace> → <retrieved-context> → <bot-memory>
→ <working-memory> → <user-profile> → <adaptation-hint> → skills
```

**互斥规则**

- `conversation.bot_id.is_some()` 且 `bot.soul` 非空 ⇒ **不注入**全局 persona。
- `bot.soul` 为空 ⇒ 仍不注入全局 persona（Bot 会话身份由 Bot 决定，空就是空，避免「悄悄退回全局人格」）。
- 非 Bot 会话 ⇒ 完全维持现状。

**前端落点**

| 动作 | 文件 | 改动 |
|---|---|---|
| 修改 | `src/components/bot/BotSoulPanel.tsx` | 调 `read_bot_soul` / `write_bot_soul`；漂移时显示提示与「以文件为准」按钮（P1 只提示，不自动合并） |
| 修改 | `src/components/bot/BotMemoryPanel.tsx` | 调 `read_bot_memory`；渲染 `indexContent`、预算条（`lineCount/maxLines`、`byteCount/maxBytes`）、主题清单 |

**测试（后端）**

- `build_agent_system_prompt` 的纯函数单测：有 bot_soul 时 `<bot-soul>` 在 `<agent-profile>` 之前；有 bot_memory 时 `<bot-memory>` 存在；两者都空时输出与旧版逐字一致（用快照锁回归）。
- `agent_query` 的集成点：Bot 会话不出现 `<persona>`；非 Bot 会话仍出现。
- `ProjectMemory` 预算：>200 行的 MEMORY.md 渲染后 ≤200 行且有截断提示。

**测试（前端）**

- `BotMemoryPanel.test.tsx`：预算条比例、空态、错误态。
- `BotSoulPanel.test.tsx`：漂移提示。

**验收命令**

```bash
npm run test:run -- -t "memory"
npm run test:run -- -t "bot soul"
npm run contracts -- --only=f
cargo test --manifest-path src-tauri/Cargo.toml -p axagent --lib commands::agent
cargo clippy --manifest-path src-tauri/Cargo.toml -p axagent --all-targets --all-features -- -D warnings
```

> 提示词 slot 目前**没有专用门禁**；本任务必须新增纯函数单测 + 快照，作为唯一防回归手段。P2 可考虑抽 `check-prompt-slots.mjs`，但不在 P1。

**DoD**：Bot 会话首轮请求的系统提示词可见两段（用现有 tracer 或单测快照证明）；非 Bot 会话行为零变化；MEMORY.md 超预算被正确截断。

---

### T6 门禁收口

| 项 | 内容 |
|---|---|
| 目标 | CI 口径全绿；文档与索引登记；本地提交 |
| 责任线 | **Bot 门禁与验证工程师**（主）；架构师（验收） |

**分阶段（每个任务卡完成时跑）**

```bash
# T1
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-migration
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --test schema_compliance
# T2
npm run contracts -- --only=a,d,f,g,h
npm run check:layers
# T3
npm run check:routes:strict
npm run check:i18n:key-parity
# T4
npm run test:run -- -t "bot"
# T5
npm run test:run -- -t "memory"
```

**提交前全量（一次）**

```bash
npm run check:eol
npm run format:check
npm run lint:eslint
npm run typecheck
npm run test:run
npm run check:i18n
npm run check:i18n:key-parity
node scripts/check-i18n-orphan-diff.mjs
node scripts/check-jsx-text-i18n.mjs
npm run contracts -- --only=a,c,d,f,g,h
npm run check:layers
node scripts/cargo-all-workspaces.mjs fmt-check
node scripts/cargo-all-workspaces.mjs clippy
__TAURI_WORKSPACE__=true cargo test --manifest-path src-tauri/Cargo.toml -p axagent --workspace
npx playwright test e2e/bots.spec.ts --project=chromium --grep-invert "(Agent|Cache|Conversation|Gateway)"
```

**DoD**：以上全绿；`git status` 只含预期文件；本地 commit（不 push）。

## 3. 全局验收（产品视角）

1. 新建 Bot「投研助手」，SOUL 写「只回答 A 股问题」；保存后重启 App，Bot 仍在。
2. 在其下新建线程问「今天天气」，模型按 SOUL 拒答 A 股以外问题。
3. 在 `~/.axagent/bots/<id>/MEMORY.md` 写一行偏好，重启后该线程仍遵守。
4. 在 Bot 详情「记忆」中看到同一行，预算条显示 1/200 行。
5. 删除 Bot，线程仍在（未归属），`~/.axagent/bots/<id>/` 仍在。
6. 普通对话（无 Bot）行为与升级前一致。

## 4. 风险（最高两处）

### 风险 1：声明式建表的「静默失败」

`MIGRATIONS = &[]`，建表完全依赖引擎按实体收敛；而引擎会 DROP「实体未覆盖的孤儿表」（P0a 事故形态）。若 `entities/src/lib.rs` 漏 `pub mod bots;`、或 `schema_compliance` 漏登记，**不会编译报错**，只会表现为「表没建/列没加/测试覆盖不到」。

**缓解**：T1 的 DoD 强制三件事——空库启动后 `PRAGMA table_info(bots)` 有 20 列；`schema_compliance` 通过；`missing_versions_is_empty` 仍绿。另在 T1 提交说明里贴启动日志中的 reconcile 结果。

### 风险 2：SOUL 双注入与「假绿」e2e

两处调用点（`agent/mod.rs:1971`、`skill_execution.rs:874`）只要漏改一处，Bot 会话就会在不同路径下收到不同人格；而提示词没有门禁，单测不覆盖就无人发现。另一面，浏览器 mock 不写 `botId` 会让「按 Bot 过滤」在 Web 模式恒空，CI 的 `--grep-invert` 又会跳过标题含 `Agent|Conversation` 的用例——测试写着、实际没跑。

**缓解**：T5 用纯函数快照锁两处调用点的输出；T4 强制补 `browserMock.ts` 并在 CI 同款 `--grep-invert` 下跑一遍；e2e 标题禁用四个被排除的词。

## 5. 估时与人力

| 任务 | 后端 | 前端 | 门禁 | 合计（人日） |
|---|---|---|---|---|
| T1 | 2 | — | 0.5 | 2.5 |
| T2 | 2 | — | 0.5 | 2.5 |
| T3 | — | 2.5 | 0.5 | 3 |
| T4 | 0.5 | 1.5 | 0.5 | 2.5 |
| T5 | 1.5 | 1 | 0.5 | 3 |
| T6 | — | — | 1.5 | 1.5 |
| **合计** | **6** | **5** | **4** | **15** |

> 可并行：T3 在 T2 契约冻结后即可开工；T5 的提示词部分与 T2 并行。串行关键路径约 **9–10 人日**。

## 6. P2 范围（只列不展开）

1. **例程与自动化**：`cron_jobs` 加 `botId`；routine run 记录、暂停/完成态；应用事件触发。
2. **审批**：Bot/线程级别解析 `max(own, inherited)`；引擎能力封顶；接 `agent_sessions.permission_mode`；「读自己文件不询问」。
3. **团队/部门**：`fleets` 成员指向 `bots.id`；Chief；共享模板白名单。
4. **委派运行时**：`Assignment` 合同（树/深度/预算/去重/waiting/needs_input）。
5. **每 Bot 技能与预设**：Bot 私有 `skills/`；预设导入（技能默认关）。
6. **Bot API**：每 Bot key（只存哈希）+ `/messages` 入口。
7. **记忆写入与维护**：编辑器、变更日志、归档、脱敏、写审批。
8. **通知/未读**：Bot 级未读与提醒。

## 7. 提交策略

- 每个任务卡一个 commit（`feat(bots): …` / `fix(bots): …` / `test(bots): …`），Conventional Commits + 中文描述。
- 文档（本目录五份）单独一个 `docs(bot-integration): …` commit，先落盘再开工。
- **不 push**；全部本地提交。
- 降级提示：发布说明必须写「回退到不支持 Bot 的版本会丢失 `bots` 表」（ADR-008 / `MODEL.md` §6）。
