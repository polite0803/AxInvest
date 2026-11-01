# PLAN — LLM 采样参数单一权威

## 目标

同一执行体的同一 LLM 参数（max_tokens / temperature / top_p / frequency_penalty）
只允许存在一处运行时权威。默认值只能做「创建时单向初始化」，不得在运行时与权威值并行生效。

## 已拍板（2026-10-25）

1. **模型级**：保留 `ModelParamOverrides`（与 temperature/top_p 同组），删除全仓零读取的
   `Model.max_output_tokens`。
2. **会话默认**：`AppSettings.default_*` 作为新建会话初始值并真正落库；删除
   `ConversationCategory.default_*` 这一重复层。
3. **工作流**：模板变量 `agent_max_tokens` / `agent_temperature` 为该工作流唯一权威；
   删除 Agent 节点级参数入口与种子里的节点死值。LLM 节点保留自己的参数（不同执行体）。
4. **范围**：同族参数一次收口。

## 权威矩阵（收口后）

| 执行体 | max_tokens | temperature | top_p | frequency_penalty |
|---|---|---|---|---|
| 聊天会话 | `Conversation.max_tokens`，空则回退模型的 `param_overrides.max_tokens` | 同左（temperature） | 同左（top_p） | **删除**（全链从未发送） |
| Agent 查询（commands/agent） | `request.options` → 模型 `param_overrides` | 同左 | 同左 | — |
| 工作流 Agent 节点 | 模板变量 `agent_max_tokens`（无变量时用模型默认） | 模板变量 `agent_temperature` | — | — |
| 工作流 LLM 节点 | `LLMNodeConfig.max_tokens` | `LLMNodeConfig.temperature` | — | — |
| 标题摘要 | `AppSettings.title_summary_max_tokens`（默认 256） | `title_summary_temperature` | `title_summary_top_p` | — |
| 对话压缩 | `AppSettings.compression_max_tokens`（默认 1024） | `compression_temperature` | `compression_top_p` | — |

**默认值链（单向）**：`AppSettings.default_*` →（新建会话时落库）→ `Conversation.*`。
落库后运行时只读 `Conversation.*`；存量会话为空时回退模型级默认，不再使用任何硬编码常量。

## 删除项

1. `Model.max_output_tokens`（harness DTO、DAO 映射、providers 构造点、bindings）。
2. `ConversationCategory.default_temperature/default_max_tokens/default_top_p/default_frequency_penalty`
   （entity / harness DTO / DAO / 前端类型 / 分类编辑 UI / mock / 测试）。
   物理列保留（本项目表结构 additive_only，删声明不等于删列）。
3. Agent 节点参数入口：`AgentProfileList` 与 `AgentPropertyPanel` 的 temperature/maxTokens。
   股票分析**主模板**（`stock-analysis`）种子里的专家 Agent 节点值置空并 bump
   `TEMPLATE_VERSION` 180→181 —— 该模板有 `agent_temperature` / `agent_max_tokens` 变量，
   节点值恒被覆盖，属死值。
   **不删** serenity / screenshot / news / daily 四个子模板的节点值：它们没有对应的模板变量，
   节点值就是该工作流的唯一权威（删掉会变成「谁都没设」）。
4. `frequency_penalty` 的全部用户可设点与 DTO 字段：它虽然存在于会话/分类/模型/全局四层，
   但 `ChatRequest` 没有该字段、任何 provider adapter 都未发送 ⇒ 全链零生效的假配置。
   删除而非保留；DB 列保留。`AgentProfile.suggested_*` 属「建议值」语义，不参与请求，保留。

## 接通项

1. 新建会话：后端 `create_conversation` 读取 `AppSettings.default_*` 并写入会话。
2. 聊天请求：`conversation.*` 为空时回退模型 `param_overrides.*`；删除
   `force_max_tokens` 分支里的硬编码 4096。
3. 标题摘要默认值前后端统一为 256（当前后端 50、前端 256）。
4. Provider 模型编辑：「上下文窗口」保持不变；参数组里的 maxTokens 标签改为「最大输出」。

## 验证

- Rust 单测：新建会话落全局默认；聊天参数解析优先级（会话 > 模型默认）；标题默认一致。
- 前端测试：分类编辑不再渲染参数滑块；Agent 节点属性不再渲染参数；Provider 标签。
- 门禁：`cargo fmt --all --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、
  `__TAURI_WORKSPACE__=true cargo test --workspace`、`npm run typecheck`、`npm run test:run`、
  `npm run format`、`npm run lint:eslint`。

## 实测记录（2026-10-25）

- `cargo fmt --all --check`：0。
- `cargo check --all-targets -p axagent-harness -p axagent-dao`：0。
- `cargo check --all-targets -p axagent -p axagent-providers -p axagent-gateway`：0
  （先浮出一个 `GeminiModel.output_token_limit` 死字段告警，已随 `max_output_tokens` 一并删除）。
- `cargo clippy -p axagent-harness -p axagent-dao -p axagent-providers --all-targets -- -D warnings`：0。
- `cargo clippy -p axagent --all-targets -- -D warnings`：0（24m33s，依赖图全量被 lint）。
- `cargo test -p axagent-harness --lib`：828 passed / 0 failed。
- `cargo test -p axagent-dao --lib repo::conversation::conversation_defaults_tests`：2 passed。
- `cargo test -p axagent-dao --test schema_compliance`：1 passed（删列后建表清单一致）。
- `cargo test -p axagent --lib commands::conversations`：15 passed / 0 failed。
- `npm run check:schema-gen`：生成物与权威字节级一致。
- 前端：`typecheck` 0、`lint:eslint`（oxlint）0、`format` 0、`check:eol` 0、`check:i18n*` 0、
  `check:types` / `check:paired` / `check:serde` 0。
- `npm run test:run`：2740/2743。红 3 条均在 `crossCheckDrivers.test.ts`，
  根因是**并行会话未提交**的 `portfolio-mgr.rhai` 删掉了 `trade_signal` 腿而测试/文案未同步
  （工作树 11 条 vs HEAD 12 条，与本轮改动无交集）。

### 与计划的偏差（有意为之）

- `frequency_penalty`：`ChatRequest` 与所有 provider adapter 从未发送该字段，属全链假配置；
  本轮**删除**而非接通（接通需要给 ChatRequest 加字段并改 12 个 adapter，超出「消除重复定义」）。
- `gateway_link::create_gateway_conversation` 不落全局默认：该会话的 model_id 是网关端点，
  参数由网关链路协议决定，不属于聊天执行体。
- `AgentProfile.suggested_*`：`suggested_temperature` / `suggested_max_tokens` / `suggested_model_id` **已删除**（全仓零消费方，2026-10-25 复核）；`suggested_provider_id` **保留** —— 它真被读：`rt-workflow/executors/agent_executor.rs` 的 provider 优先级链最后一档 `profile_suggested`。「三条同前缀的建议字段」里只有一条活着，按消费方逐条判，不按名目整组删。


## 举一反三批（P0 接通 / P1 门 / P2 统一解析，2026-10-25）

用户指令：「按这个顺序开工，P0 里三项『接通』」。

### P0 接通（改成能生效的通路，不是换个硬编码数值）

| 项 | 接通点 | 说明 |
|---|---|---|
| 托盘开关 | `src-tauri/src/tray.rs::set_tray_enabled` + `init/services.rs` 按 `settings.tray_enabled` 决定是否 `start_tray` + 新命令 `commands/desktop.rs::set_tray_enabled` + 前端 `GeneralSettings.tsx` 新增 `general:trayEnabled` 开关（11 语言补 `settings.trayEnabled`） | 此前只有 DB 字段与设置项，没有任何执行方读它 |
| 启动最小化 | `AppInitializer.tsx::showWindow(minimized)` 由 `settings.startMinimized` 驱动 | 此前窗口恒 `show()` |
| 网关自启 | `AppInitializer.tsx` 把 `settings.gatewayAutoStart` 接进启动并行任务 | 此前开关只有写口 |
| 图片模型 | 新 `src/lib/visionModel.ts::resolveVisionModel`（单一解析点）+ `ImageAnalysisPanel.tsx` 读 `settings.imageModel` | 此前面板能设、执行侧只看会话模型 ⇒ 设了不生效 |

P0 同批删除的零消费字段：`s3_sync_*` 4 条、`mini_window_enabled`、`video_provider_id/video_model_id`、
`voice_provider_id/voice_model_id`（载体：`harness/types/settings_chat.rs` + 前端 store/DTO/转换器）。

### P1 新门 `scripts/check-stock-var-defaults-parity.mjs`

普查结论：**判据不存在**，不是扫描面漏格 —— 既有三道门各管一角且都不比全表值：
`check-variable-consumer-registry.mjs`（有没有消费者，不比值）、
`check-panel-var-landing.mjs`（已接线 15 条的三面值等式）、
`check-indicator-config-scope.mjs`（技术指标 8 条 ↔ `IndicatorConfig::default()`）、
`check-valuation-defaults-parity.mjs`（估值 DTO 7 字段跨 5 载体）。
其余 130+ 条面板 `b()` 兜底值没有任何门比过种子。vitest 里那条 `debate_rounds` 单键锁是同一缺陷的第二补法，
本门把它升成全表（值 / 键 / 类型三面对账，退出码 0/1/2，`--selftest` 9 条正负对照）。

**首跑抓出 4 条真实分叉，已全部按「种子/权威为准」修面板**：

| 变量 | 面板原值 | 权威值 | 权威在哪 |
|---|---|---|---|
| `value_dcf_perpetual_rate` | 4 | **1.3** | `mcp_tools::PERPETUAL_GROWTH = 0.013` ×100 |
| `value_dcf_discount_rate` | 8.5 | **7.7** | `mcp_tools::DISCOUNT_RATE = 0.077` ×100 |
| `reco_min_confidence` | 60 | **50** | 种子（60 会滤掉 value/reversion/watchlist 的超短线产出） |
| `reco_ic_gate` | `"shadow"` | **`"on"`** | 种子 `DEFAULT_RECO_IC_GATE`（v172 起闭环生效并覆写存量） |

前两条正是估值校准「只改到一部分载体」留下的另一半 —— `check-valuation-defaults-parity.mjs` 守的是
`DEFAULT_VALUATION_PARAMS`（camelCase DTO）那份，`getDefaultVariables()` 这份不在它的扫描面里。

共用件：把估值门内置的 Rust 常量求值器抽到 `scripts/lib/rust-const-expr.mjs`（禁区 12，不抄第二份）；
抽完复跑估值门 + 其 `--selftest` 9/9 绿。新门注册三处同位：`package.json`
（`check:stock-var-parity` / `:selftest`）、`scripts/ci-check.mjs`、`.github/workflows/ci.yml`。

**门的有电自证（两条，都不落盘、不碰共享树）**：
- 把 `git show HEAD:src/components/settings/StockAnalysisConfigPanel.tsx`（**修复前**源文）连同现网种子
  喂进同一批判据 ⇒ 恰好 4 条红（每条一个真实分叉，红条数＝被破坏那一格），现值源文 ⇒ 0 红。
- 把 `MIN_PANEL_VARS` 临时抬到 1000 ⇒ exit 2 且点名「只抓到 134 条」，改回后逐字节比对与快照一致。
`--selftest` 9 条（正样本 2 / 负样本 6 / 边界样本 1），失败时逐条打印判据消息（不靠猜）。

数字比较用相对容差 `1e-9`：只为 `…::DISCOUNT_RATE * 100.0` 这类派生式的 IEEE754 尾差放行
（`0.077 * 100 = 7.700000000000001`），两侧都是十进制字面量时永不触发 ⇒ 不比估值门的 `===` 松。

### P2 执行参数（并发 / 超时）统一解析

普查到的现状（同一属性多处支配，且**高层那份常输**）：

1. `stock_workflow/decision.rs::resolve_runtime_options` 只算 3 面；`tool_timeout_secs` 在 `core.rs`
   主链与批量链**内联重写两遍**。
2. 按类型并发 map 也在 `core.rs` 内联两遍；**荐股链（serenity）与反思链（reflection）根本没设该字段**
   ⇒ `RunOptions::default()` 的 `llm=agent=3` 把面板设的 `max_concurrent` 静默压回 3。
3. `commands/workflows/mod.rs` 与 `cognitive.rs` 走 `with_max_concurrent()`，而该 setter 只改全局值、
   不动 by_type ⇒ 同一个「用户设 16」在两条链上被 3 卡住。
4. 引擎里 `node_timeout` 的 match 在主批次与抢先调度路径**逐字抄两遍**。
5. 按类型表的键写成 `"file"`，而引擎取名的 `node_type_name` 给的是 `"fileOperation"` ⇒ 那一格**永不命中**（死项）。
6. `engine/mod.rs::node_type_of` 自带一份 32 臂 match（旧注释写「与 `node_type_name` 等价」而不调用它）
   ⇒ 全仓**两份**节点类型名映射，新增节点类型必漏一处。

改法：

- `ExecutionParams`（`decision.rs`）成为四面唯一解析点，`from_template_vars` + `concurrency_by_type()` +
  `apply_to(&mut RunOptions)`；四条链（主分析 / 批量 / 荐股 / 反思）一律走它，删掉两处内联
  `tool_timeout_secs` 与两处内联 type_limits。按类型键按引擎口径改为 `tool` / `fileOperation` / `llm` / `agent`。
- `IO_NODE_CONCURRENCY`（rt-workflow）成为「IO 类并发 10」的唯一常量；`RunOptions::default` 与
  `with_max_concurrent` 都取它；后者现在把 `llm`/`agent` 一起对齐。
- `resolve_node_timeout(node_type, base_timeout_secs, &options)` 单点解析超时，两条调度路径共用。
- `node_type_of` 改为委派 `node_type_name`（删掉 32 臂副本）。
- 新增 6 条 Rust 单测：`decision.rs` 4 条改造 + 2 条新（按类型对齐、`apply_to` 覆盖 crate 默认的优先级锁）；
  `rt-workflow` 2 条新（超时优先级、`with_max_concurrent` 对齐）。每条都先断言**默认值与模板值不同**，
  避免恒真自测。

### 本批新挂的待裁项

- **W-1 `agent_retry_max` / `tool_retry_max` 是空接线**：面板可编辑、种子有声明，但全仓**零运行时消费方**；
  真实重试次数唯一来源是 `seed_stock_analysis.rs` 里各节点写死的 `RetryConfig`（tool 2 / agent 5 / 辩手 1）。
  两个选项：A) 接通 —— 给 `RunOptions` 加 retry 上限并在引擎重试路径读它（改重试次数会影响 LLM 花费，
  且超时重试那条路径 `mod.rs:4397` 目前只认节点级、无工作流回退）；B) 撤下 —— 删两条种子变量 + 面板控件
  （需 bump `TEMPLATE_VERSION`）。**未自行拍**：A 改运行成本，B 减用户可见能力。
- **W-2 抢先调度路径不看按类型并发**（`engine/mod.rs` 原 4606 一带，只查全局 `max_concurrent`），
  与主批次过滤（`type_limits.get(nt)`）口径不同。属调度语义，本批只把**超时**并成单点，未动并发判定。
- **W-5 `AppSettings` 的字段没有「有没有人读」的门**。本轮 P0 接通的 4 项（托盘 / 启动最小化 /
  网关自启 / 图片模型）全是「存了没人读」的字段，而工作流变量那一侧早有
  `check-variable-consumer-registry.mjs` 拦新增空接线 —— 设置这一侧至今等价物为零
  （`grep -l AppSettings scripts/*.mjs` = 0 命中）。建议照它的形状做**登记制**反向门
  （每条字段：非声明面引用 ≥1 或显式 `EXEMPT{status,cite}`，`EXEMPT` 长度当待修面读数用）。
  没在本批顺手做的原因：`AppSettings` 100+ 字段的读取形态不止 `settings.xxx`
  （解构 / store 选择器 / camelCase 序列化 / 前端 mock），口径要和它一样先普查再定，
  半做会产出一条天天假红的门。

- **W-4 注释剥离器有 7 份副本，其中 2 份是同一个缺陷**。修 G1 假红时发现
  `scripts/check-variable-consumer-registry.mjs` 的 `stripComments` 是「先全局删 `/* … */`、
  再删 `// …`」两条正则 ⇒ 行注释里出现的 `/*`（现网 43 个 `.rs` 有这形态，如
  `// ST/*ST 等需要大小写不敏感匹配`）会被当成块注释开头一路吞到下一个 `*/`：
  `seed_stock_analysis.rs` 从 559 KB 剥到 180 KB，该文件后半的引用面**整体消失**。
  已把本门改成单趟扫描（行注释 → 可嵌套块注释 → 字符串/原始字符串 → 普通字符），
  剥后 296 KB、`trader_context_slim` 保留、文档正文照删、门 + `--selftest` 13/13 绿。
  **未做的部分（要拍）**：`check-panel-var-landing.mjs` 里是**逐字相同的坏副本**（它今天扫的
  13 个面恰好没有该形态 ⇒ 没爆，但下次有人写 `ST/*ST` 就静默丢引用面）；
  另有 `check-accent-foreground.mjs` 同型。建议把这 7 份（其中 4 份已是正确的字符扫描器）
  收敛到 `scripts/lib/strip-comments.mjs`，各门按语言传入分支配置。

- **W-3 `RunOptions::default()` 的 `step_timeout=300s` 与股票链的 600s 不同源**：股票链现在恒由
  `ExecutionParams` 覆盖，crate 默认只服务非股票工作流；是否把两者收敛为一个常量待定。

## 实机核对清单（需重启应用，本批未提交）

改动落在 Rust 侧 ⇒ 保存即触发 `tauri dev` 重建重启（会杀掉在跑的分析，先看任务中心有没有 running）。

| # | 看什么 | 预期 | 不成立时最可能的原因 |
|---|---|---|---|
| 1 | 设置 → 通用 → 「显示系统托盘图标」关掉 | 托盘图标**立刻**消失，无需重启 | 命令 `set_tray_enabled` 没进注册表（看 `src-tauri/src/register_commands.rs`） |
| 2 | 关掉托盘后同一组里的「最小化到托盘」 | 变成不可用（灰），且勾被自动打回 | 联动那行被改回单键 `saveSettings` |
| 3 | 「启动时最小化」打开后重启 | 首屏不弹出来，窗口 minimize 后不 setFocus | `AppInitializer.showWindow(minimized)` 的参数没传下去 |
| 4 | 「网关自动启动」打开后重启 | 网关端口进入监听（网关页显示运行中） | `startGateway()` 失败被 `logIpcError` 吞掉，看控制台 |
| 5 | 默认模型 → 图片模型设一个，然后在会话里做图片分析 | 用的就是设置里那个模型（此前恒用会话当前模型） | `resolveVisionModel` 的两级回退被绕过 |
| 6 | 股票分析跑一条，看并发 | 荐股链/反思链的 LLM 并发从 3 变成面板值（出厂 8） | 429 变多 ⇒ 把面板 `max_concurrent` 调回 3 即回到今天形态，不用改码 |
| 7 | 股票分析设置 → 估值三项 | 永续 1.3 / 折现 7.7（此前面板显示 4 / 8.5） | 库里仍是旧值属正常（种子不覆盖存量），新库里才是 1.3 / 7.7 |

前端呈现层已用静态源文 + 单测替代实机核对的部分：托盘两个设置的上下级
（`src/components/settings/__tests__/GeneralSettings.tray.test.ts` 3/3）、
图片模型回退（`src/lib/__tests__/visionModel.test.ts` 3/3）、参数默认值前后端一致
（`src/components/settings/__tests__/llmParamDefaults.test.ts` 3/3）。**这几条替代的是
「代码里读的是哪个键」，不替代上面 7 条的运行时行为。**

## 本批（P0 接通 / P1 门 / P2 单点）实测读数（2026-10-25 04:00–05:45）

### Node / 前端门（全部取日志正文，不信通知退出码）

| 门 | 读数 |
|---|---|
| `check:stock-var-parity` | 134 条对账 / 等式不符 0 / 解析失败 0；`--selftest` 9/9 |
| 同上·有电自证 | `git show HEAD:` 未修面板源文回喂 ⇒ 恰好 4 条红；`MIN_PANEL_VARS=1000` ⇒ exit 2 点名「只抓到 134 条」，复原后与快照逐字一致 |
| `check:valuation-parity` | 抽公用求值器后仍 5 处逐字段一致；`--selftest` 9/9 |
| `check-variable-consumer-registry` | 修剥离器后通过（声明 357｜零引用面 0）；`--selftest` 13/13 |
| `check-panel-var-landing` / `check-indicator-config-scope` / `check-horizon-weight-parity` | 各 exit 0 |
| `typecheck` / `lint:eslint`(oxlint) | 0（oxlint 1401 files，0 warnings 0 errors） |
| `check:eol` / `check:i18n` / `key-parity` / `untranslated --strict-all` | 全 0 |
| `check:types` / `check:paired` / `check:serde` / `check:schema-gen` | 全 0 |
| `npm run test:run`（全量） | 262 files passed，0 failed（本轮新增 3 个测试文件：tray / visionModel / llmParamDefaults） |
| `dprint check` | 唯一红：`src/components/stock-analysis/__tests__/AnalystReportGrid.contradictionNotice.test.tsx`（并行批在写的新文件，未动） |

### Rust 门

| 命令 | 读数 |
|---|---|
| `cargo check --all-targets -p harness -p entities -p dao -p runtime-core -p providers` | 0（8m14s） |
| `cargo clippy -p runtime-core -p harness -p dao --all-targets -- -D warnings` | 0 |
| `cargo clippy -p rt-workflow --all-targets --all-features -- -D warnings` | 首跑 1 红：`field_reassign_with_default`（我新写的测试）⇒ 改成结构体字面量后 0 |
| `cargo test -p rt-workflow --lib` | 327 / 327；`--test per_node_exec_ctx_vars / lifecycle_hooks / execution_finalization` 3/4/3 全过 |
| `cargo test -p dao --lib repo::conversation` | 6 passed（353 filtered）；`--test schema_compliance` 1 passed |
| `cargo fmt --all --check` | 0（先只 `-p axagent` 绿，剩一处 `engine/mod.rs:317` 单行 if/else 超 50 列 ⇒ 展开后全绿） |
| `cargo check -p axagent --lib` | **101，5 条错全在并行批**：`commands/backtest_validation.rs:201/247/525/549`（E0599 缺 `use sea_orm::QueryOrder` + E0277 连带）、`stock_analysis_setup/seed_stock_analysis.rs:6557`（E0596 `pm.0` 可变借用）。本批自己的 `commands/agent_profile.rs:150` E0061（删 `suggested_*` 三字段后 `upsert_agent_profile` 少 3 参）**已修并从错误清单消失** |
| `cargo check -p axagent --all-targets`（含 `#[cfg(test)]`） | **101，6 条错全在并行批**：`backtest_validation.rs:201/247/525/549`、`stock_analysis_setup/seed_stock_analysis.rs:6550`（E0596 `pm.0`，属他们 `f7_independent` / `portfolio-mgr-f7-ref` 那段）、`stock_workflow/rhai_pm.rs:198`（E0061，`portfolio_formula::portfolio_risk_gate` 被他们加成 9 参）。逐条按 `git diff -U0` 的 hunk 区间对过，**无一落在我改动的区间** |
| 同一份读数的正面含义 | lib 与 lib test 两个 target 报的都是这 6 条 ⇒ 本批新写的 `decision.rs` 测试（4 改 + 2 新 + 1 文本锁）**编译通过**，只是还没能运行 |
| `cargo test -p axagent --lib commands::stock_workflow::decision` | **待补跑**（被上面 6 条挡；探针 `grep -c "use sea_orm::QueryOrder" src-tauri/src/commands/backtest_validation.rs`，当前 0） |
| 全量 `clippy --workspace` / `test --workspace` | 未跑（共享工作树 + 构建锁被并行批占；按约定留到收口一次） |

---

## 第三轮（2026-10-26）：模板变量这条私通道整体退役（用户裁定，推翻 v181 的权威归属）

### 触发

用户读数：把默认模型设成 `deepseek-flash` 跑一次股票分析，DeepSeek 余额 76.56 → 65.19（**11.37 元**），
并判定「决策还不准确，严重没有价值」。对账过程见 `PLAN-stock-analysis-latency.md` 同日条。

导出 CSV（`amount-2026-10-26_2026-10-26.csv`）把 11.37 元定位到 **20:00–21:00 一个小时**：
632 次请求 / 未命中输入 4,142,421 / 命中输入 2,104,695 / 输出 1,787,958，按官方空闲档
（1 / 0.02 / 4 元每 M）= **11.336 元**。该小时内库里有 10 次完整分析。
**成本构成里输出占 63%**，缓存命中只省了 2.06 元（13.40 → 11.34）。

### 用户的三条裁定

1. `agent_max_tokens` 不该是自建烟囱，**统一到「供应商-模型 → 参数」**；统一的结果就是抬高上限。
2. **四档是设计要求，不许砍**。
3. **主模型设成什么是用户的事，代码里不得硬编码**。

### 普查出的真实形态（比"一个变量"大）

工作流引擎整体对模型层那一格**只有 `request_delay_ms` 一个消费点**；采样参数一律不读。
`ModelParamOverrides` 里 `temperature` / `max_tokens` / `top_p` / `use_max_completion_tokens` /
`thinking_param_style` 五格，聊天链全读（`commands/agent/mod.rs:893` 起的优先级注释：
请求选项 → 模型覆盖 → 默认），工作流全不读。

代价实证：库里 `agnes-3.0-flash` 模型层 `maxTokens = 124672`，工作流按模板变量 **4096** 下发 ⇒
20:52 那次 50 个 LLM 节点里 **10 个的 `output_tokens` 正好等于 4096**（且都是零工具调用的单轮节点：
trader / risk-con / risk-neu / risk-agg / risk-convergence / research-mgr / rule-check / bull-r2 ×2 / bear-r2 ×3），
`rule-check` 正文断在「等回落至 MA5 14.294 / MA10 13」、`research-mgr` 断在句中，
而 `streamTruncated = false`、`truncationReason` 为空 ⇒ 界面报 completed。

面板默认值与库现值也分叉：`StockAnalysisConfigPanel.tsx` 写 32768、`DemandDiscoveryConfigPanel.tsx` 写 16384，
库里是 4096（`merge_variable_values` 无条件保留旧值所致）。

### 落地的四步

| 步 | 内容 | 落点 |
|---|---|---|
| 1 | 四处消费点收口成一个纯函数 `resolve_sampling`（节点配置 > 模型覆盖 > 不下发），并在 harness 加唯一查找入口 `model_param_overrides`（与 `request_delay_ms_for_model` 同形） | `crates/rt-workflow/.../agent_executor.rs`、`crates/harness/src/types/provider_model.rs` |
| 1b | 回退/重试路径不再沿用主模型的生效值，改按**回退模型自己**那一格算；`try_fallback_stream` 的两参数由 `temperature/max_tokens` 改成 `node_temperature/node_max_tokens` | 同上（VERDICT 兜底、H4.1、H4.2、`try_fallback_stream`） |
| 2 | 退役两个变量：删种子定义、`TEMPLATE_VERSION` 188→189、新增纯函数 `drop_variables` + 一次性删键门 `SAMPLING_VARS_RETIRED_VERSION`（编译期断言 `TEMPLATE_VERSION >= 门值`）、删 RENAME_MAP 两行别名 | `seed_variables.rs`、`seed_stock_analysis.rs`、`stock_analysis_setup/mod.rs` |
| 3 | 撤两个面板的 4 行渲染与两处 `resolve([...])` 分组项，删 `numberBounds.ts` 的温度特判（变空分支一起删），面板测试由「必须暴露」改为「必须不出现」 | `StockAnalysisConfigPanel.tsx`、`DemandDiscoveryConfigPanel.tsx`、`numberBounds.ts`、两个测试 |
| 4 | 11 语言删 `stockAnalysis.configDescriptions.agent{Temperature,MaxTokens}` 与 `demandDiscovery.…` 两键（两趟写：先全量构建再统一落盘，LF） | `src/i18n/locales/*.json` |

### 被明确推翻 / 退役的东西

- **v181 条的「唯一权威 = 模板变量」**：改为「唯一权威 = 模型层参数」。v181 内联注释已就地标注被 v189 推翻。
- `seed_stock_analysis.rs` 里「本模板的采样参数唯一权威是模板变量」的节点注释（同段等行数改写，避免行号引用整体移位）。
- 面板两格、RENAME_MAP 两行别名、`numberBounds` 温度特判、11×4 个 i18n 键。

### 按设计保留，不动（并登记理由）

- `switch_executor.rs:293` / `llm_classifier_executor.rs:164` / `condition_executor.rs:308` 的
  `temperature = 0.0` 与 `max_tokens = 512 / 10`：**判定类节点**，0 温度是可复现性要求，
  不能被模型层的 0.7 盖掉；小上限是结构化短 JSON 的刻意额度。改它们会让判据不可复现且成本上升。
- 四档分析师（21 个节点、占那次运行输入的 59%）：用户裁定不砍。
- 主模型 / 回退模型的取值：属用户设置，代码与本次改动都不写死任何 model id。

### 风险与要如实说的一件事

上限一撤，输出预算交给模型：`deepseek-flash` 官方最大输出 384K，思考模式默认开启，
推理 token 按输出计价 ⇒ **单次分析的输出成本可能明显高于现在的 0.77 元**。
控成本的旋钮从此只有一个：「供应商-模型 → 最大输出」那一格（`agnes-3.0-flash` 现填 124672，
`deepseek-flash` 为空）。我不会替用户填。

### 门读数

| 门 | 读数 |
|---|---|
| `dprint fmt`（5 个改动 TS 文件） | 无 diff |
| `npm run typecheck` | 0 |
| `npm run lint:eslint`（oxlint） | 0 / 0（1407 files） |
| `vitest` `numberBounds.test.ts` + `StockAnalysisConfigPanel.test.tsx` | 20 passed |
| `check-i18n-key-parity.mjs` | 11 份 16239 条一致；正控（查不存在的 key）报不存在 |
| `rustfmt --edition 2024 --check` 逐个改动文件 | 我的区间 0；剩 2 处 diff 在 `seed_consistency_tests.rs:2449/2471`，属并行批 `@@ -2423,0 +2424,58 @@` hunk 区间内，**不动** |
| `cargo check -p harness -p rt-workflow -p axagent --all-targets` | 见下（首跑 2 红均在本批：E0106 缺生命周期标注、E0382 `vars` 移动后再借用，均已修） |
| `cargo test -p axagent-rt-workflow --lib sampling_source_tests` / `cargo test -p axagent --lib sampling_template_variables_are_retired` | 待补跑 |
