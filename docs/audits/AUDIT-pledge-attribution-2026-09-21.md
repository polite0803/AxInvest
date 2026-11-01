# 「质押数据获取失败（工具调用被拒绝）」归因审计

- **日期**：2026-09-21
- **样本**：`stock_analyses.id = 48ee5352-1056-498e-a6a0-0b05e91b90d9`
- **标的**：300179 四方达 ｜ **运行时间**：2026-09-21 18:22:19（北京）｜ `template_version = 71`
- **数据来源**：PostgreSQL `axinvest`.`stock_analyses.blackboard_snapshot`（2.97 MB，只读导出）
- **触发**：用户在仪表盘看到结论「质押数据获取失败（工具调用被拒绝）」，问是**真被拒绝**还是**模型幻觉**

> **结论先行：这句话是模型的错误归因（幻觉式），不是真实事件。**
>
> 「工具调用被拒绝」这个**事件确实发生过**，但对象是另一个工具（`get_stock_margin_data` 融资融券），
> 原因也不是权限而是**限流**；**质押工具从头到尾没有被调用过，也从未被授权**。
> 模型把一个手边真实的 `permissionDenied` 记录，安到了一个物理上不存在的「质押工具调用」头上。

---

## 一、先证观察面（否则「0 命中」不可采信）

判定「从未调用」之前，必须先证明**工具调用痕迹确实会落进快照**——否则 0 命中可能只是没记录。

| 探针 | 读数 | 含义 |
|---|---|---|
| `_raw.a-lockup.tool_calls_made` | **存在**，3 条记录（`tool`/`arguments`/`result`/`is_error`） | ✅ 节点级工具调用**完整落盘**，观察面充分 |
| `get_stock_lockup_bundle` | 命中 8 处（含 `_raw.t-lockup-data.tool_name`） | ✅ ToolNode 调用同样落盘 |
| `get_stock_margin_data` | 命中 5 处（含 `_raw.a-hot-money.tool_calls_made.2`） | ✅ 跨节点调用可区分 |

⇒ 工具名在快照中**必然留下痕迹**，故下文的「0 命中」是**有效读数**，不是观察面不足。

---

## 二、a-lockup 实际发生的 3 次工具调用（全量）

**源码位置**：`src-tauri/src/commands/stock_analysis_setup/mod.rs:506-518`（`lockup-watcher` 白名单）

```rust
(
    "lockup-watcher",
    &[
        // V63 修复(2026-07-23)：移除 get_stock_lockup / get_stock_shareholder_trades /
        //   get_stock_block_trades——上游 t-lockup-data 调用 get_stock_lockup_bundle
        //   已返回 {lockup_schedule, shareholder_trades, block_trades} 三个字段的 bundled JSON。
        // 保留 get_stock_margin_data（bundle 不含融资融券）和
        //   get_stock_announcements（bundle 不含公告）作为补充数据源。
        "get_stock_margin_data",
        "get_stock_announcements",
        "search_stock",
    ],
),
```

本轮实际调用与白名单**完全一致**（`_raw.a-lockup.tool_calls_made`）：

| # | 工具 | `is_error` | 结果 |
|---|---|---|---|
| 0 | `get_stock_announcements` | `false` | 成功（返回公告列表 JSON） |
| 1 | `get_stock_margin_data` | **`true`** | `Error: Tool execution error: [permissionDenied] 工具 'get_stock_margin_data' 权限被拒绝: 工具 'get_stock_margin_data' 调用过于频繁，最小间隔 200ms（当前距上次调用 48ms）` |
| 2 | `search_stock` | `false` | 成功 |

**没有第 4 次调用。没有任何质押工具参与。**

### 报告原文（`report.a-lockup`，576 字符）

> **质押风险**：质押数据获取失败（**工具调用被拒绝**），无法评估平仓线距离与强制平仓触发条件，此维度缺失会降低分析置信度。

---

## 三、质押工具：从未被调用，也从未被授权

### 3.1 全库历史 0 次调用

```sql
SELECT id, stock_code FROM stock_analyses
WHERE blackboard_snapshot LIKE '%get_stock_pledge_data%';
-- → 0 行（全部历史运行）

SELECT id, stock_code FROM stock_analyses
WHERE blackboard_snapshot LIKE '%detect_pledge_risk%';
-- → 仅 1 行：301302 / 2026-07-21 16:54:59，此后再未出现
```

### 3.2 工具存在，但全仓零引用（孤儿工具）

`grep -rn "get_stock_pledge_data" src-tauri/` **只有两处**，且都在同一个文件：

| 位置 | 内容 |
|---|---|
| `crates/astock-data/src/mcp_tools.rs:420` | 工具 schema 定义（`"获取股权质押数据（大股东质押比例/质押股数/控股股东质押比例/风险等级）"`） |
| `crates/astock-data/src/mcp_tools.rs:1203` | dispatch 分支 |

**没有任何 profile 白名单、没有任何工作流节点的 `config.tools`、没有任何 `data_sources` 引用它。**
⇒ 工具造好了，但**从未接线**（同族：`detect_pledge_risk` 仅 2026-07-21 出现过一轮）。

### 3.3 上游预拉的数据也不含质押

`seed_stock_analysis.rs:1873`：

```rust
("t-lockup-data", "获取解禁+增减持+大宗交易", "get_stock_lockup_bundle", "stock_code"),
```

`get_stock_lockup_bundle` 的工具描述（`seed_stock_analysis.rs:1558`）明确写着
「获取筹码面分析数据（**解禁+增减持+大宗交易**三方聚合）」——**三方里没有质押**。
实测 `_raw.t-lockup-data.result.content` 只含 `block_trades` / `lockup_schedule` / `shareholder_trades`。

且 `detect_pledge_risk` 的注释（`crates/astock-data/src/mcp_tools.rs:415`）写明：
> 前置工具：LLM 在调用 `detect_pledge_risk`（`tools/finance.rs`）前**应先调用本工具**获取 `pledge_pct`。

⇒ 质押工具链设计上是「`get_stock_pledge_data` → `detect_pledge_risk`」两步，**两步都没有接进工作流**。

---

## 四、根因：prompt 要求 vs 工具授权不匹配

### 4.1 契约缺口（P0）

`src-tauri/agency_experts/stock-analysis/lockup-watcher.md` 反复要求分析质押：

| 行号 | 内容 |
|---|---|
| `:6` | `data_sources: [get_stock_lockup_bundle]` ← **唯一数据源，不含质押** |
| `:18` | 「专注于**限售解禁、大股东增减持、股权质押、股东结构变化**的专业分析方法」 |
| `:22` | 「只看筹码结构类输入——解禁清单、增减持公告、**质押数据**、股东人数」 |
| `:25` | 「**质押比例 > 50% 是高警戒线**：平仓风险、纾困可能性、强制平仓触发条件需重点评估」 |
| `:30` | 「读解禁清单…、增减持公告、**质押数据**、股东人数变化」 |
| `:33` | 「评估**质押风险敞口**（平仓线距当前价距离、纾困可能性）」 |

**分析师被要求分析一个它物理上无法获取的维度**（既无预拉、也不在白名单）。
⇒ 模型只能写「获取失败」——这一半是**事实**；但它顺手抓了一个真实的 `permissionDenied`
（来自另一个工具、另一个原因）当理由 ——这一半是**幻觉**。

**这不是偶发**：全库 15 轮里 **12 轮** a-lockup 都写了质押缺口，措辞在 `质押数据缺失` /
`质押数据获取失败` 之间漂移，本轮首次出现「（工具调用被拒绝）」这种**伪归因**。

### 4.2 面板的 `expected_data` 把用户引向同一个不含质押的工具

`data_quality_summary.diagnostics.lk.expected_data`（本轮实测）：

> 「解禁减持、**质押风险**、大宗交易（**t-lockup-data**）」

面板把「质押风险」的预期数据源标成 `t-lockup-data`，而该工具**结构上不含质押字段**。
⇒ 这正是 `AUDIT-analyst-low-confidence-2026-09-21.md` §八.1 已登记的「`expected_data` 硬编码、
把用户引向错误工具」，此处**同一个缺陷又误导了一次归因**。

### 4.3 限流被报成「权限拒绝」（P1，语义混用）

真实那次拒绝的完整链路：

- `crates/tools/src/audit.rs:134-141`：`elapsed < min_interval_ms` ⇒ 返回 Err
- `crates/tools/src/audit.rs:94`：`min_interval_ms: 200`（`AuditConfig::default()`）
- `crates/tools/src/registry.rs:1354`：**把它包成 `ToolError::permission_denied(tool_name, &rate_limit_msg)`**

```rust
// registry.rs:1354
return Err(ToolError::permission_denied(tool_name, &rate_limit_msg));
```

⇒ **限流（rate limit）与权限（permission）是两类完全不同的失败**，却共用 `permissionDenied` 错误码。
对用户：显示「权限被拒绝」，会让人以为配置错了；对 LLM：拿到 `permissionDenied` 更容易
被它演绎成「我被系统挡了」→ 本轮的伪归因**正是这个措辞喂出来的**。

**并发诱因**：`get_stock_margin_data` 同时被 `a-hot-money`（`tool_calls_made.2`）与 `a-lockup`
（`tool_calls_made.1`）两个并行 agent 节点调用，48ms 内撞上 200ms 阈值。
全库 **13 轮**快照含「权限被拒绝」⇒ 属**常态误伤**，非偶发。

---

## 五、影响面

| 层 | 是否被污染 | 说明 |
|---|---|---|
| `report.a-lockup` | ✅ | 原文产生地 |
| `params.a-lockup.report` / `_raw.data-quality.input_params.lk_report` | ✅ | 快照内副本 |
| `agg-risk` / `risk-aggregated` / `risk-con` / `risk-convergence` | ✅ | 风险辩论与收敛层把该报告整段吞进 prompt，共 22 处命中 |
| `data_quality_summary.diagnostics.lk` | ✅ | `status: low`、`placeholder_hits: 2`、`report_quality: 19.0` —— **用户看到的就是这里** |
| `portfolio-mgr.data_gaps` | ❌ | 该字段按「存在性」判据独立计算，不读 data-quality 输出（见前轮审计 §六订正） |
| 最终决策 JSON | ❌ | 未出现质押字样 |

### 反向对照：另一个节点把同一件事说对了

`research-mgr` 输出的 `data_gaps`：

```json
["eps_consensus_missing", "cvd_revenue_share_missing", "pledge_ratio_unavailable", "main_flow_pct_unknown"]
```

`pledge_ratio_unavailable`（质押比例**不可得**）—— **措辞准确**，没有编造理由。
⇒ 同一底层事实（质押数据拿不到），两个 LLM 节点给出**一个准确、一个伪归因** ⇒ 判据是**措辞**层面的，
不是数据层面的。这与前轮审计 §七「面板测的是 LLM 报告里的措辞，不是数据缺口本身」同一结论。

---

## 六、修复建议

| # | 项 | 位置 | 类型 | 建议 |
|---|---|---|---|---|
| P0-1 | 质押工具链接线 | `seed_stock_analysis.rs`（新增 `t-pledge-data` ToolNode）+ `mod.rs`（白名单加入 `get_stock_pledge_data`） | **✅ 已实施（v72）** | 采纳 (a)，见 §八。⚠ 实施时又发现**第 4 层**：仅「新增节点 + 白名单授权」**不够** —— `config.tools` 由 `filter_map(tool_def_map.get)` 生成，未登记 `tool_def_map` 的工具名会被**静默丢弃**，必须同时补 `ToolDef` 并登记 |
| P0-2 | `expected_data` 归因 | `data-quality.rhai` diagnostics 的 `lk` 项 | **✅ 已实施（v72）** | 已改为「解禁减持、大宗交易（t-lockup-data）；股权质押（t-pledge-data）」，与真实来源一致 |
| P1-1 | 限流 ≠ 权限 | `crates/tools/src/registry.rs`（原 1353） | **✅ 已实施（v72 批次）** | 新增 `ToolErrorKind::RateLimited` + `ToolError::rate_limited()`，限流不再复用 `permission_denied`。**实施前实测范围**（§9.2）：唯一**活的**消费端是 `ToolError` 的 `Display` 前缀（`[permissionDenied]`）——即**喂给 LLM 的工具结果文本**；`error_code` 字段与 `tool.common.*` 这组 i18n key **两侧都无人消费** ⇒ 未为它加 11 语言 key，避免制造新的死键 |
| P1-2 | 并发撞限流 | `crates/tools/src/audit.rs:94`（`min_interval_ms: 200`） | **✅ 已实施（v72 批次）** | 采纳「只读工具免最小间隔」：豁免集合从两个只读数据源的 schema 清单**单点派生**（不手抄），并剔除 4 个写类工具；**滑动窗口上限对全部工具照旧生效**。见 §9.3 |
| P2-1 | 词表抓「伪归因」 | `data-quality.rhai` | **✅ 已实施（v73）** | 采纳「与 `tool_calls_made` 交叉核对」：判据**锚在可观测事实**（`is_error` + 工具名），措辞只用于判定「报告是否在做工具归因」。两级结论（编造 / 含糊）。见 §9.4 |

> ⚠ 与「改了不生效」有关的前置（**2026-09-21 实施阶段更正**，原表述有一处错）：
> - `data-quality.rhai` 与**节点/边定义**经 `include_str!` / seed 写进 `workflow_templates`
>   ⇒ 改它们**必须升 `TEMPLATE_VERSION`**（本轮 v71 → **v72**）。版本门实测生效（§八）。
> - **`lockup-watcher.md` 不受版本门约束**（原文「改任一处都必须升版」**不准确**）：
>   它经 `EMBEDDED_PROMPTS` → `seed_agency_experts`，该函数在
>   `ensure_stock_analysis_experts_seeded` 里**无条件调用**且是 **UPSERT**（v24 起，
>   见 `mod.rs:900` 注释）⇒ 重新编译（`include_str!` 是编译期嵌入）+ 进程重启即生效，
>   与 `TEMPLATE_VERSION` 无关。
> - **历史快照永远不会回填** —— 已产出的 15 轮记录保持旧措辞，需重跑分析才可见新行为。

---

## 七、取证方法（可复现）

本轮新建的只读探针（`output/tmp-*.mjs`，口令只经 env 传 psql，**不进 argv**）：

| 脚本 | 用途 |
|---|---|
| `output/tmp-pgprobe.mjs <sql文件> [库]` | 执行只读 SQL 并落盘 UTF-8（**绕开 PowerShell 管道的 GBK 解码**） |
| `output/tmp-dump-snap.mjs <analysis_id> <out>` | 从 PG 原样导出 `blackboard_snapshot`（**不要用 `\copy`，它会把换行转义成字面 `\n`**） |
| `output/tmp-find-ctx.mjs <snap> <关键词> <窗口> <out>` | 递归定位关键词并给出 **JSON 路径**（判定「在哪一层」） |
| `output/tmp-dump-path.mjs <snap> <路径> <out>` | 按路径取值（**快照的键是扁平化的，键名自带点号**，需先试整串当单键） |
| `output/tmp-keys.mjs <snap> <路径> <out>` | 列某层的键与类型（摸结构） |

**四条方法学要点**（本轮踩到）：

1. **`tool_call_logs` / `tool_executions` 全库 0 行** —— 这两张「权威工具调用表」**没有任何写入路径**，
   不可作为证据源。真正的工具调用记录在 `blackboard_snapshot` 的 `<node>.tool_calls_made`。
   ⚠ 查这两张表得到 0 行，**不能**读成「没有工具调用」。
2. **0 命中必须先证观察面**（§一）——否则「从未调用」不可采信。
3. **`\copy ... TO` 会转义**，导出 JSON 会坏；用 `psql -A -t -c` 走 stdout 并**由 Node 落盘**。
4. **PowerShell 管道会按 GBK 解码 psql 的 UTF-8 输出** ⇒ 中文全乱。让 Node 脚本自己 `writeFileSync`。

> §一~§七 为**只读取证**阶段：未执行任何 git 命令，未修改任何生产源码；仅新建 `output/` 下的取证脚本与产物。
> §八 为**实施**阶段，改动清单与生效条件见该节。

---

## 八、P0 实施记录（v72，2026-09-21）

### 8.1 改动清单

| # | 文件 | 改动 |
|---|---|---|
| 1 | `seed_stock_analysis.rs` | `TEMPLATE_VERSION` 71 → **72**；新增 `t-pledge-data` ToolNode（调 `get_stock_pledge_data`）+ 入边 `e-trigger-t-pledge-data` + 出边 `e-t-pledge-data-a-lockup`；`a-lockup.config.context_sources` 由 1 项改为 2 项 |
| 2 | `seed_stock_analysis.rs` | **新增 ToolDef `td_pledge_data` 并登记进 `tool_def_map`** —— 这是「授权真正生效」的前置，见 §8.2 |
| 3 | `stock_analysis_setup/mod.rs` | `PROFILE_TOOLS["lockup-watcher"]` 增加 `get_stock_pledge_data` |
| 4 | `agency_experts/stock-analysis/lockup-watcher.md` | frontmatter `data_sources` 增加该工具；正文新增「数据来源」表 + 「命名易错点」+ **「缺失时的表述约定（强制）」**（明令禁止把缺失归因为「工具被拒绝 / 权限不足」） |
| 5 | `data-quality.rhai` | `lk` 的 `expected_data` 改为「解禁减持、大宗交易（t-lockup-data）；股权质押（t-pledge-data）」 |
| 6 | `src/stores/feature/stockWorkflowChatBridge.ts` | 新增 `"t-pledge-data" → stockAnalysis.tool.pledgeData`（缺此映射 `AnalysisProgress` 会显示裸 id） |
| 7 | `src/i18n/locales/*.json`（11 种） | 新增 `stockAnalysis.tool.pledgeData` |
| 8 | `seed_consistency_tests.rs` | **新增门禁** `analyst_profile_tools_are_all_declared_in_tool_def_map`，见 §8.3 |

### 8.2 为什么必须改 4 层（第 4 层是本轮实施才发现的）

原计划 3 层（节点 / 白名单 / prompt 声明）。实施时**先只改了白名单**，随即确认它**不生效**：

```rust
let tool_names = PROFILE_TOOLS.iter().find(|(k, _)| **k == **_expert).map(|(_, v)| *v).unwrap_or(&[]);
a.config.tools = tool_names.iter().filter_map(|&tn| tool_def_map.get(tn).cloned()).collect();
```

`filter_map` 对**查不到的工具名静默丢弃**（不报错、不留痕）。当时 `get_stock_pledge_data` 既无 `ToolDef` 也不在 `tool_def_map` 里 ⇒ **只改白名单＝假修复**：配置上「已授权」，`config.tools` 里根本没有它，行为与修复前**逐字一致**。

**为何 `config.tools` 就是 LLM 的工具可见集（闭环证据）**：

- `agent_executor.rs:243` — `let tools = tool_defs_to_chat_tools(&an.config.tools);`：送进模型的就是它；
- `agent_executor.rs:253` — `tool_permissions: None`：工作流侧**不**用权限表再拦一层（`execute_tool` 的「不在允许调用列表中」分支只对显式注入权限的调用方生效；stock 侧注入的 `strict_tool_permissions()` 管的是输出契约，不是工具白名单）；
- **反证（与 §二 实测互印）**：`lockup-watcher` 白名单当时恰好 3 项（`get_stock_margin_data` / `get_stock_announcements` / `search_stock`），而快照里的 `tool_calls_made` **恰好 3 次**且正是这三个 ⇒ 链条闭合。

### 8.3 新增门禁（防这类假修复复发）

`seed_consistency_tests.rs::analyst_profile_tools_are_all_declared_in_tool_def_map`：

- 输入面①：从 `seed_stock_analysis.rs` 抽 `analysts` 数组第 3 元素（专家 id）；
- 输入面②：从同文件抽 `tool_def_map` 的 key（覆盖「名字与值同行」与「`(` 独占一行」两种声明形态）；
- 主断言：**分析师用到的白名单工具必须全部登记** `tool_def_map`；
- 两个**正控**（判据工具会静默撒谎：少抽一个名字，下游断言照样绿）——必须看见 `lockup-watcher`、必须看见 `get_stock_margin_data` 与 `get_stock_pledge_data`。

**断言范围为何只收在分析师数组**：`PROFILE_TOOLS` 是**双消费**表 —— ① chat 侧写 `agent_profiles.recommended_tools`（经 `get_chat_tools_by_names` 按名过滤，**同样静默丢弃**）；② 工作流侧全仓**只有分析师循环这一处**读它。其余专家（如 `stop-loss-reviewer`）只走 ①，模板节点是显式 `tools: vec![]`（见 `mod.rs` 的「生效面（勿误读）」段）⇒ 要求它们也登记 `tool_def_map` 属过度约束、会制造假红。

> 试跑探针对照表（`output/tmp-toolmap-check.mjs`）：断言范围内 **0 处**缺失；范围外的 `stop-loss-reviewer → check_exit_signals` 由上述解释覆盖，**不是**缺陷。
>
> **第二轮独立复刻**（`output/tmp-145-extractor-replica.mjs`，2026-09-21 收尾）：因 `cargo test` 被并发会话的 target 锁挡住，用 Node **逐行复刻**同一套抽取器（`quoted_strings` / `analyst_expert_ids` / `tool_def_map_keys`）在真源上独立跑 —— 读数与上面一致：分析师 **10 位**全部抽到（含 `lockup-watcher`）、`tool_def_map` **40** 项、正控①② 均 PASS、主断言 **0 处缺失**。⇒ 抽取逻辑本身成立，且 v72 的 `get_stock_pledge_data` 登记**确已生效**。
> ⚠ 口径说明：这是**诊断口径**（证明「抽取逻辑 + 断言在真源上成立」），**不替代** `cargo test` —— 它不证明代码能编译、也不证明该测试被注册并真跑过（判据 #327）。

### 8.4 生效条件（尚未完成的部分）

| 事项 | 状态 |
|---|---|
| 代码 / 模板改动落盘 | ✅ |
| `cargo fmt --check`（本次改动的文件） | ✅ 通过 |
| `npm run typecheck`（tsc --noEmit） | ✅ 通过 |
| `dprint check` | ✅ 通过 |
| 门禁逻辑的**独立复刻验证**（`output/tmp-145-extractor-replica.mjs`） | ✅ **全绿**（细则见 §8.3 末段）。⚠ 诊断口径，不替代 `cargo test` |
| 新增代码的**结构静态自检**（编译器本会检查的那几类） | ✅ 逐项核对通过：`tool_node` 闭包（`:1040`）9 参数 vs 我的调用 9 参数**形态一致**；`edge` 闭包（`:1799`）3 参数 ✓；**定义先于使用**（`tool_node:1040` / `PLEDGE_TOOL_ID:1878` / `edge:1799` / `td_pledge_data:1611` / `tool_def_map:1638-1703` 全部早于其使用点 `1994/2007/2010/1696/2052`）；`stock_code_params`（`:1147`）在 `td_pledge_data`（`:1611`）前可解析。⚠ 这只覆盖「作用域 / 参数个数 / 名字存在性」，**不覆盖**类型推断与 trait 解析 ⇒ 仍**不等于**编译通过 |
| `cargo test`（`-p axagent-tools -p axagent-harness -p axagent --lib`） | ✅ **全绿**（`output/tmp-test-151.txt`，`exit=0`）：`axagent_lib` **433 passed** / `axagent_harness` **731 passed** / `axagent_tools` **417 passed（2 ignored）** ⇒ 合计 **1581 passed / 0 failed**。此前被并发会话的 target 锁挡住约 20 分钟，锁释放后跑成（归属判定见 §9.7） |
| 　└ **改跑全量**（AGENTS.md 的口径就是 `--workspace`，不是任选几个包） | ✅ **全绿**（`output/tmp-test-151-ws.txt`，`exit=0`）：**136 个** `test result` 块 ⇒ 合计 **7464 passed / 0 failed / 40 ignored**；`^error` / `test result: FAILED` / `^failures:` / `panicked` **均 0 行** |
| 　└ 关键一条：`rt-workflow` 的**行为级**测试真的跑了（不是只编译） | ✅ `Running tests\data_quality_placeholder_gate.rs`（日志 5908 行）内部 **11 passed / 0 failed**，含本轮新增的 `unbacked_attribution_is_flagged_in_two_severities` 与 `attribution_note_text_is_single_line`。**这是 P2-1 唯一会真执行 `data-quality.rhai` 的地方**，3 包批（§9.6 首行）覆盖不到 |
| 　└ 本轮新增 **9 个**测试**逐个确认已注册并真跑**（不止「文件里写了」） | ✅ `seed_consistency_tests::{analyst_profile_tools_are_all_declared_in_tool_def_map, data_quality_rhai_diag_for_arity_matches_all_call_sites}`（日志行 225/226）；`harness error::tests::{permission_denied_is_unchanged, rate_limited_is_distinct_from_permission_denied}`（651/652）；`tools audit::tests::{exempt_lookup_handles_server_qualified_names, read_only_exempt_set_covers_the_incident_tool, min_interval_exempts_read_only_but_still_guards_others, window_limit_still_applies_to_exempt_tools, mutating_tools_are_excluded_and_all_names_are_real}`（1212/1214/1216/1217/1227） |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ **全绿**（`output/tmp-clippy-151b.txt`，`exit=0`，`Finished dev profile in 15m 22s`）：**38 个 crate** 有 `Checking` 行、`^error` **0** 行、`^warning` **0** 行。本仓 lint 规则**只有 clippy 执行**，且遇首个 error 即短路 ⇒ 必须**全量**重跑，不能只跑单包。⚠ 上一批（`tmp-clippy-145.txt`）因编译起点**跨越了本轮改动**而作废，见 §9.7 第 1 条。**补跑**：`output/tmp-clippy-152-ws.txt`（`exit=0`，仅 2 crate 增量）＋ `output/tmp-clippy-153-main.txt`（`-p axagent`，`exit=0`，`Checking axagent` 出现，23m52s）⇒ 主 crate 亦已当场重检，**门禁闭环**；判据推导见 §9.6 覆盖度自证行 |
| **DB 重种（v71 → v73）** | ⏳ **未做，已只读实测确认**（`output/tmp-pg-version-check.mjs`，口令走 env 不进 argv） |
| 　└ 实测读数 | `stock-analysis`：**version = 71**、`nodes::text LIKE '%t-pledge-data%'` = **0**、`edges` 里同串出现 **0** 次 ⇒ 新节点/新边**尚未落库**；v73 物证三项（`attribution_note` / `lk_tool_calls` / 10 条 `tool_calls` 映射）亦全 **0**（符合预期：需重编 + 重启进程才会 seed）。**正控**：全表 111 行、`max(version) = 71`（通道有效，不是「0 命中」的假象） |
| 　└ 版本门结论 | **条件已满足**：DB 现值 71 **严格小于** 常量 **73** ⇒ `existing.version >= TEMPLATE_VERSION` 为 false ⇒ 重编 + 重启后**必然重种**，无需再动常量（判据 #651 的正向用法：先读 DB 现值再定常量） |
| 重跑一轮分析验证质押字段 | ⏳ 未做（建议用 300179 对照，检查 `a-lockup` 报告里质押维度不再是缺口） |

### 8.5 顺带登记的既有欠账（未改）

| 项 | 位置 | 说明 |
|---|---|---|
| rustfmt 告警 | `crates/analysis-engine/src/position_limits.rs:63` | 条件表达式超宽未折行。**非本次改动**，未触碰（避免污染他人在飞的工作区） |
| 孤儿 ToolDef | `seed_stock_analysis.rs` 的 `td_pledge`（`detect_pledge_risk`） | 已定义但**未**登记 `tool_def_map`、也无白名单授权 ⇒ 恒不可达。本轮刻意**不**接：它要求调用方自备 `pledge_pct`，而质押工具返回 `pledge_ratio`，字段名不同，错接会引入新的口径混淆 |
| 契约未消费 | 质押数据目前只进 `a-lockup` 的 context | **未**并入 portfolio-mgr 的 f10 因子（f10 走 `lockup_bundle`）。是否并入属**评分口径决策**，未擅自改 |
| P1-1 限流报成权限 | `crates/tools/src/registry.rs` | **✅ 已完成**（§六 P1-1，见 §九） |
| **`tool.common.*` i18n 死键** | `src/i18n/locales/*.json` 的 `tool.common.{notFound,permissionDenied,…}` | 6 个 key × 11 语言**零消费**：`src/` 下无 `tool.common` / `` `tool. `` 字面或拼接用法；`lib/errorI18n.ts` 的 `ERROR_CODE_RE` 只认 `^[A-Z][A-Z0-9]+(_[A-Z0-9]+)+$`，`tool.x.permissionDenied` 匹配不上。**疑似另有 `error_code.rs`（大写码）体系承担真实翻译**，`tool.*` 这套属未接线的早期契约。未改（删 key 会动 11 个文件，且可能有人正打算接线） |

---

## 九、P1/P2 实施记录（2026-09-21，v73 批次）

### 9.1 改动清单

| 项 | 文件 | 改动 |
|---|---|---|
| P1-1 | `crates/harness/src/error.rs` | 新增 `ToolErrorKind::RateLimited` + `kind_str` 分支 `"rateLimited"` + 构造器 `ToolError::rate_limited(tool, reason)`（文案「触发调用频率限制」，**不含**「权限/被拒绝」等归因词） |
| P1-1 | `crates/tools/src/audit.rs` | `check_rate_limit` 返回值 `Result<(), String>` → `Result<(), RateLimitViolation>`；新增 `RateLimitViolationKind{MinInterval,WindowExceeded}`（供调用方区分，且 `retry_after_ms` 可操作） |
| P1-1 | `crates/tools/src/registry.rs` | 限流分支改用 `ToolError::rate_limited(...)` |
| P1-2 | `crates/tools/src/audit.rs` | 新增 `MUTATING_DATA_TOOL_NAMES`（评审单点）、`read_only_data_tool_names()`（`OnceLock` 单点派生）、`is_min_interval_exempt()`；最小间隔检查加豁免，**滑动窗口不受影响** |
| P2-1 | `src/commands/data-quality.rhai` | 新增 `attribution_markers()` / `rejection_summary()` / `attribution_note()`；`diag_for` 增第 8 形参 `attr_note`，结论并入 `gap_reason`（复用既有展示通道） |
| P2-1 | `stock_analysis_setup/seed_stock_analysis.rs` | `input_mapping` 新增 10 条 `{分析师}_tool_calls` → `{node}.tool_calls_made`；`TEMPLATE_VERSION` **72 → 73** |
| 门禁 | `stock_analysis_setup/seed_consistency_tests.rs` | 新增 `data_quality_rhai_diag_for_arity_matches_all_call_sites` + 两个词法辅助（`paren_body` / `split_top_level_commas`） |
| 门禁 | `crates/tools/src/audit.rs` | 新增 4 条测试（豁免集合正控、写类排除 + 幽灵条目反查、行为证明、窗口未被摘掉、`server_id/tool` 形态） |
| 门禁 | `crates/harness/src/error.rs` | 新增 2 条测试（`rateLimited` 契约三层锁 + `permission_denied` 形态不变的对照） |

### 9.2 为什么 P1-1 **不**加 i18n key（范围实测）

P1-1 的唯一**活**消费端是 `ToolError` 的 `Display` 前缀 —— 也就是**喂给 LLM 的工具结果文本**。另外两处都无人消费：

| 载体 | 消费端 | 实测 |
|---|---|---|
| `Display` 前缀 `[rateLimited]` | **LLM**（工具结果文本）+ 日志 | ✅ 唯一活消费端 |
| `error_code` = `tool.{name}.rateLimited` | —— | ❌ 无。`src/` 下无任何 `tool.*` 字面/拼接用法；`lib/errorI18n.ts` 的 `ERROR_CODE_RE` 只认大写码，驼峰路径匹配不上 |
| `tool.common.*`（6 key × 11 语言） | —— | ❌ **死键**：`src/` 下零消费、无映射表（见 §8.5） |

⇒ 只为「让 LLM 不再误读」而加 11 语言 key，等于**新增死键**。故本次刻意不加（判据「写进产物但没人消费」）。

### 9.3 为什么 P1-2 **不能**整包豁免

`stock_mcp_tools()` 声明 58 个工具，其中 4 个**不是**只读：`dojo_create_plan` / `dojo_revise_plan` / `dojo_execute_plan`（计划状态机，均落库）、`optimize_attention_weights`（写回权重）。整包豁免会把这些工具的最小间隔一并摘掉。

豁免集合的派生方式（关键）：

- `tools` crate **本来就依赖** `axagent-astock-data` 与 `axagent-analysis-engine` ⇒ 可在 `audit.rs` 内直接取两个 schema 清单函数，**与 `seed_consistency_tests::resolvable_tool_names()` 用同一对权威来源**。不手抄 58 个名字（手抄必腐烂）。
- **没有**新增 `UnifiedToolRegistry` 的实例字段 —— 该结构全仓有 **13 处** `new()`（`audit.rs:24` 已为同类问题确立「进程级 `OnceLock`」先例）。加实例字段就要改 13 个构造点，漏一个就是**假修复**（正是本轮 P0-1 第 4 层踩过的坑）。
- 豁免**只摘最小间隔**：滑动窗口上限（30 次 / 10 s）对所有工具照旧 —— 有专门测试 `window_limit_still_applies_to_exempt_tools` 守着「别顺手把整道闸拆了」。

### 9.4 P2-1 判据设计（对着真实样本设计）

**判据锚在可观测事实**，措辞只用于判定「报告是否在做工具归因」；**归因真假完全由 `tool_calls_made` 裁决**：

| 结论 | 触发条件 | 严重度 |
|---|---|---|
| 疑似编造 | 报告含归因短语，而本轮 `tool_calls_made` **无任何 `is_error == true`** | 硬 |
| 归因含糊 | 确有失败记录，但报告**一个被拒工具名都没提** | 软 |

**本轮实证样本正好命中第二条**：报告写「质押数据获取失败（**工具调用被拒绝**）」，而唯一失败的是 `get_stock_margin_data`（原因**限流**），报告未点名任何工具 ⇒ 使用者无法判断被拒的是哪个工具、影响哪个维度。

两个**不可删**的守卫（都是防判据自身的假阳性/假阴性）：

1. `tool_calls` 为 Null/空数组 ⇒ **不判**。旧快照缺字段、该节点本就无工具时，判「编造」会把**测不出**当成**有问题**。
2. 报告不含归因短语 ⇒ 不判（判据不适用，而非「无异常」）。

结论并入 **`gap_reason`**（既有展示通道：弹窗「缺口原因」行 + `DecisionBanner` 逐节点表的「缺口原因」列，v34/v71 都往这里写「确定归因：…」）—— 不新增结构化字段 / 前端行 / 11 语言 key。

**已知边界（不掩饰）**：措辞是词表匹配，模型换一种说法（如「数据源不可达」）即漏判。交叉核对消除的是**假阳性**方向（不会把真失败判成编造），**不消除**词表覆盖面的假阴性。若要彻底消除，需改由后端在工具执行层记录「报告是否引用了一个本轮未发生的失败」—— 属另一个量级，登记不做。

### 9.5 新增门禁

| 门禁 | 位置 | 覆盖的「编译器看不见」 |
|---|---|---|
| `data_quality_rhai_diag_for_arity_matches_all_call_sites` | `seed_consistency_tests.rs` | **Rhai 实参个数不匹配是运行期错误**（`cargo check`/`clippy` 不解析 `.rhai`）⇒ 给 `diag_for` 加形参却漏改 10 个调用点之一，data-quality 节点整体失败并拖垮 quality-gate/portfolio-mgr。5 条判据：① 形参 ≥ 8；② 10 个调用点实参个数一致；③ 第 8 实参形态为 `attribution_note(x, y)`；④ 两变量已在 `input_mapping` 声明（Rhai 引用未声明变量只得到 unit、**不报错**）；⑤ 两者前缀一致（防跨节点错配） |
| `read_only_exempt_set_covers_the_incident_tool` | `audit.rs` | 豁免集合**非空**、**覆盖本轮真实撞限流的那只工具**、规模 ≥ 50（防只抽到一支数据源） |
| `mutating_tools_are_excluded_and_all_names_are_real` | `audit.rs` | 写类工具未被豁免；`MUTATING_DATA_TOOL_NAMES` 每项都是**真实存在**的工具名（防幽灵条目） |
| `min_interval_exempts_read_only_but_still_guards_others` | `audit.rs` | 行为证明：只读紧邻两次放行、非只读被拦且类别为 `MinInterval`；违规文案不含归因性表述 |
| `window_limit_still_applies_to_exempt_tools` | `audit.rs` | 第 31 次调用必须被窗口上限拦住（防「为修 P1-2 把整道闸拆了」） |
| `rate_limited_is_distinct_from_permission_denied` | `harness/error.rs` | 三层锁：`kind` 与 `PermissionDenied` 不同、`error_code` 为 `rateLimited`、`Display` 前缀为 `[rateLimited] `、文案不含「权限被拒绝/未授权/无权限」 |
| `unbacked_attribution_is_flagged_in_two_severities` | `rt-workflow/tests/data_quality_placeholder_gate.rs` | **行为级**（真执行脚本）证明 P2-1 判据生效：A 零失败+归因措辞 ⇒ 「编造」；B 有失败+未点名 ⇒ 「含糊」且写出真实工具名；C 已点名 / D1 无记录(`()`) / D2 空数组 / E 未做工具归因 ⇒ **均不得报警**（4 个负对照防假阳性） |
| `attribution_note_text_is_single_line` | 同上 | 归因结论必须是**单行、无 `\` 残留** —— Rhai 模板字符串**无转义语义**，跨行会让 `\` 与行首缩进逐字进界面（实证见 §9.8 第 3 条） |

诊断脚本 `output/tmp-151-rhai-arity.mjs`（与上面第 1 条同判据的 Node 复刻，含 `RHAI_FILE` 环境变量以便喂改坏的副本）。**已做 3 个负对照**证明它真的有区分力（否则「全绿」可能只是抽取器空转，判据 #7）：

| 负对照 | 注入的破坏 | 结果 |
|---|---|---|
| ① 删掉一个调用点的第 8 实参 | `attribution_note(...)` → `""` | ❌ 报「第 8 实参形态不符」 ✓ |
| ② 变量名改成未声明 | `lk_tool_calls` → `lk_tool_calls_TYPO` | ❌ 报「引用了未声明变量」+ 前缀不符 ✓ |
| ③ 调用记录指到别的节点 | `lk_tool_calls` → `sec_tool_calls` | ❌ 报「不是同一个节点」 ✓ |

### 9.6 验证状态

| 门 | 结果 |
|---|---|
| `cargo fmt --check` | ✅ 本次改动文件通过（用 `cargo fmt -p axagent-harness -p axagent-tools` 定点格式化，**未**触碰 `position_limits.rs` 的既有欠账） |
| `dprint check` | ✅（本轮未改 TS/JSON/MD 表格以外的受管文件） |
| `cargo test -p axagent-tools -p axagent-harness -p axagent --lib` | ✅ **全绿**（`output/tmp-test-151.txt`，`exit=0`）：3 个测试二进制共 **1581 passed / 0 failed**（归属与读数列见 §8.4），本轮新增 9 个测试逐个确认已注册并真跑。⚠ 该批**曾被并发 target 锁挡约 20 分钟**；判明「等」而非「杀」的依据见 §9.7 第 3 条 |
| 　└ ⚠ **该批覆盖不全 —— 本轮实测教训** | 它**不含 `-p axagent-rt-workflow`** ⇒ 唯一会**编译并执行** `data-quality.rhai` 的测试（`crates/rt-workflow/tests/`）**从未运行**，因此一度漏掉我自己引入的一处回归（§9.8 第 1 条）。⇒ 已改跑 `cargo test --workspace`（`output/tmp-test-151-ws.txt`）—— AGENTS.md 的门禁口径本就是 `--workspace`，**不是**任选几个包 |
| 　└ **全量结果（已跑完）** | ✅ `exit=0`：**136 个** `test result` 块 ⇒ **7464 passed / 0 failed / 40 ignored**；`^error(`/`test result: FAILED`/`^failures:`/`panicked` 全部 **0 行**（用 Node 聚合，非肉眼扫）。点名核对：`data_quality_placeholder_gate` **11 passed**（含 `unbacked_attribution_is_flagged_in_two_severities` / `attribution_note_text_is_single_line`）、`harness error::tests::rate_limited_is_distinct_from_permission_denied`、`seed_consistency_tests::analyst_profile_tools_are_all_declared_in_tool_def_map` 均 **ok** —— 证明「写了 ≠ 注册了 ≠ 真跑了」这一步真的走完 |
| `cargo fmt --check`（补跑 rt-workflow 后） | ✅ 仅剩 `analysis-engine/src/position_limits.rs:63` 的**既有欠账**（非本轮改动，刻意不碰）。`data_quality_placeholder_gate.rs` 已用 `cargo fmt -p axagent-rt-workflow` 定点格式化 |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ **全绿**（`output/tmp-clippy-151b.txt`，`exit=0`）：**38 个 crate** 有 `Checking` 行、`error` **0** 行、`warning` **0** 行、`Finished dev profile in 15m 22s` |
| 　└ **覆盖度自证（⚠ 本行判据已撤回，见 §9.9）** | 首版我用「日志文件 `mtime` 晚于每个改动文件 `mtime`」作覆盖判据（实测 7 个文件全部满足，提前量 456s~2512s）。**该判据已撤回**：判据层 **#540** 明令「`mtime` 不可作时序判据 —— 本仓第 4 次实测，门禁日志 mtime 曾比产生它的会话晚约 **14 小时**（环境/打包/同步所致）」，且 #540 指定的替代是「内容 sha256 / 行内自证时间戳 / **当场重跑**」。<br>**方向性**：`mtime` 被外部推后时，「晚于 ⇒ 覆盖」判出的是**假绿**（激进）；而 #175/#288 的「早于 ⇒ 作废」失效时只产生**假红**（保守）。⇒ 本仓既有的 #175 与 #540 **互相矛盾且未仲裁**，#540 更晚、实证更硬，采用 #540。<br>**现采信的口径**：以「日志中 `Checking <crate>` 行存在」为**必要**条件（见上行 38 个 crate），并以**定点当场重跑**为充分条件。**补跑记录（锁已释放，2026-09-21 20:30 起）**：
- `cargo clippy --workspace --all-targets -- -D warnings`（`output/tmp-clippy-152-ws.txt`）⇒ ✅ `exit=0`、`^error` 0 / `^warning` 0、`Finished in 9m 22s`。⚠ 但它是**增量**：只重检了 2 个 crate（`rt-workflow` + `axagent-mobile`），**日志里没有 `Checking axagent`** ⇒ 若仅凭此批就宣称「主 crate 已被覆盖」属**过度声称**。
- `cargo clippy -p axagent --all-targets -- -D warnings`（`output/tmp-clippy-153-main.txt`）⇒ ✅ **`exit=0`**、`Checking axagent v2.9.9` 出现（日志第 6 行）、`^error` **0** / `^warning` **0**、`Finished in 23m 52s`。**这是补齐 152 增量的那一块**：主 crate 在**当前树**上被真检过（`data-quality.rhai` 经 `include_str!` 编进本 crate 的 seed）⇒ 覆盖度充分条件已满足，**门禁闭环**。⏱ 23m52s 也再次说明：主 crate 单次 `-p` 重检 ≈ 半小时，攒批一次跑完比边改边跑划算。

### 9.7 过程记录：一次「混合树」假报错 + 一次自伤

1. **混合树假报错**：上一批 `cargo clippy` 因抢锁排在 `harness` 之后才拿到锁，于是它用**旧** `harness`（无 `rate_limited`）检查了**新** `tools`（调用 `rate_limited`），报 `E0599: no associated function named rate_limited`。**这是竞态产物、不是真缺陷** —— 若不识别就跑去「修」一个不存在的问题。⇒ 结论：**跨越改动与编译起点的 clippy 结果一律作废重跑**（「Fresh 冒充通过」的另一形态）。
2. **自伤**：本轮我在**同一条消息**里对**同一文件**发了两个 `Edit`（§六 表格 + §8.5 表格），后者用旧快照覆盖前者 ⇒ §六 的改动静默丢失（§8.5 的留下）。这正是本项目「同文件 `Edit` 必须**串行**」那条纪律的实证 —— 串行是**硬约束**，不是效率建议。
3. **「等」而不是「杀」的判据（含我自己两次判错）**：验证批被 `Blocking waiting for file lock on build directory` 挡久时，日志只有那一行、不回答「持锁进程在推进还是僵死」。本轮**判对了两次、判错一次**，都留证：
   - **判错①（浮点尾数假推进）**：首采样把**打印出来的浮点毫秒**（`1789988653813.0642`）手工抄成整数基准再比 `deps > 315.9s ago`，得出「推进 = true」——实为**浮点尾数造成的假推进**（同族「期望值同源构造」）。真读数：45 秒内 `deps/*` **完全未变**。⇒ 采样判据必须**全程取同一种量的原始值**，禁止跨「打印值 / 内部值」比对。
   - **判错②（mtime 静置 = 卡死？）**：随后 51 秒内 `deps/*` 仍不动、clippy 日志 0 条 `Checking`，据此差点判「锁被僵死进程持有」。**错**：当时正在编的是**最后一个巨型 crate** `axagent_lib`，**单个 rustc 单次调用期间不产任何新文件**（产物只在结束时落盘）。
   - **判对（正判据 = 进程表 + CPU 时间增量）**：列全 `cargo`/`rustc` 的 `CommandLine` 后一目了然 —— 持锁者是并发会话的 `tauri dev`，其子进程 `rustc --crate-name axagent_lib`（RSS **2.58 GB**）在 20 秒墙钟内 **CPU 179.56s → 199.52s（≈ 满核）、RSS 2719.9 → 3358.7 MB** ⇒ 明确在 codegen/link，**去等**；而**我自己的** `cargo clippy` **CPU 仅 0.05s** ⇒ 真的在等锁（不是在跑）。同一时刻还有**第三个会话**的 `cargo check --workspace --all-targets` 也在等（cpu 1.09s）。
   - 最终测试**正常跑完**（§8.4）⇒ 未杀是对的。**教训**：`mtime` 静置**不能**单独推出卡死；「我的进程 cpu≈0」与「持锁进程 cpu 在涨」必须**分开测**。

### 9.8 收尾实测：一次自引入回归、一次「重复品」拦截、一次只有真执行才看得见的缺陷

#### 9.8.1 我引入的回归 —— 而且**是既有门禁抓到的，不是我自己发现的**

P2-1 往 `data-quality` 节点的 `input_mapping` 加了 **10 个 `*_tool_calls` 键**，却**没同步**
`crates/rt-workflow/tests/data_quality_placeholder_gate.rs` 顶部那份**手抄**的 `EXTERNAL_VARS`。
那份清单有一条**双向锁**门禁（`external_vars_match_node_input_mapping`，`declared` 50 vs 清单 40
⇒ `missing` 10 项 ⇒ 红）。

**回归证据（同口径复刻，`output/tmp-151b-extvars-replica.mjs`，含 3 个正控）**：

| 时点 | declared(input_mapping) | EXTERNAL_VARS | 双向差集 |
|---|---|---|---|
| 修复前 | **50** | **40** | ❌ `missing` = 10 个 `*_tool_calls` |
| 修复后 | 50 | 50 | ✅ 0 / 0 |

**为什么我一开始没看到它**：我的验证批是 `-p axagent-tools -p axagent-harness -p axagent --lib`，
**不含 `axagent-rt-workflow`** ⇒ 那个 crate 的集成测试从未运行。
⇒ **教训**：门禁的「覆盖度」是与「判据正确性」同等的一等公民；改动跨 crate 时，验证批必须按
`--workspace` 口径跑（AGENTS.md 本来就这么写）。

#### 9.8.2 原本要补的「新门禁」是**重复品** —— 查重拦下了一次无用功

起因：我注意到 `data-quality.rhai` 只有我那条**文本抽取**门禁（数 `diag_for` 实参个数），
推断「缺编译级测试」，准备照 `portfolio_mgr_rhai_compiles` 补一个。

**查重结果：它早就有了** —— `crates/rt-workflow/tests/data_quality_placeholder_gate.rs`：

| 已有能力 | 位置 |
|---|---|
| **真编译**（`engine.compile(SCRIPT).expect(...)`） | `:123` |
| **真执行**（`eval_ast_with_scope::<Map>` + 50 个外部变量注入 scope） | `:136` |
| 引擎配置与生产对齐（`set_max_expr_depths(1024, 1024)`） | `:92` |
| 手抄清单 ↔ `input_mapping` **双向锁** | `:358` |

⇒ **不写那个测试**。教训与 §9.3 的 `tools` crate 一处同源：**先 grep 既有实现，再决定要不要新写**
—— 否则新增的是「换个名字的同一件事」，还会让下一个人分不清哪份是权威。

**顺带纠正一个口径**：P2-1 的**权威**门禁家是 `data_quality_placeholder_gate.rs`（可执行口径）；
我在 `seed_consistency_tests.rs` 里那条是**文本抽取**补充（它能在**不执行**的前提下抓「加形参漏改调用点」，
代价是抽取面自身可能失效）。两者互补，**不是**替代关系。因此本轮把 P2-1 的**行为级**断言加进了前者。

#### 9.8.3 只有**真执行**才看得见的缺陷：Rhai 模板字符串的 `\` 进了界面

写了两个新测试到 `data_quality_placeholder_gate.rs`（`unbacked_attribution_...` /
`attribution_note_text_is_single_line`）后，`cargo` 仍被锁挡着，于是改用**零锁探针**
`output/tmp-151b-rhai-probe.rs`（`rustc --extern rhai=target/debug/deps/librhai-*.rlib` 直编单文件，
**完全不碰 cargo 锁**）真跑脚本。首跑即暴露：

```
⚠ 疑似编造归因：报告称工具被拒绝，但本轮 3 次工具调用**无一条失败** \
                ⇒ 该归因无事实依据，不要据此推断数据缺口的原因
```

那个 `\` 与行首 **16 个空格**是**字符串内容本身** —— Rhai 的模板字符串（反引号）**不是** C 风格转义上下文，
`\` 行续接**不会折平**。而这段文本正是弹窗「缺口原因」行与 DecisionBanner 里**给用户看**的字。

- ⚠ `cargo check` / `clippy` **不解析 `.rhai`**；`engine.compile` **也不报**（跨行模板字符串**合法**）
  ⇒ 四道机械门禁**一道都不会红**，只有「真执行 + 断言文本形态」能抓到。
- 修法：改成**长单行**（`data-quality.rhai` 里已注明「不要为可读性折行」）。
- 守护：`attribution_note_text_is_single_line`（断言结果不含 `\`、不含换行、首尾无空白）。

**零锁探针读数**（修前后各跑一次，6 情形 + 细项全过）：

| 情形 | 输入 | 期望 | 实测 |
|---|---|---|---|
| A | 3 次调用**零失败** + 归因措辞 | 「编造归因」 | ✅ |
| B | 1 次失败（`get_stock_margin_data`）+ 报告未点名 | 「归因含糊」且含真实工具名 | ✅ |
| C | 有失败 + 报告**已点名** | 不报警 | ✅ |
| D1 | `tool_calls` = `()`（旧快照缺字段） | 不报警 | ✅ |
| D2 | `tool_calls` = `[]` | 不报警 | ✅ |
| E | 有失败但报告未做工具归因 | 不报警 | ✅ |

⇒ 这条探针的价值：**在 target 锁把 cargo 全部堵死时，仍能对 `.rhai` 做真编译 + 真执行验证**
（本轮因此提前发现缺陷，而不是等到重跑一轮分析才在界面上看到 `\`）。

### 9.9 本轮最该记的一条：我自己写的验证判据**违反既有判据**，且方向错了

#### 9.9.1 事发

clippy 全量绿（38 crate / 0 error / 0 warning / `exit=0`）后，我为了回答「这份绿覆盖的是当前树吗」（本仓既有坑：「Fresh 冒充通过」与「混合树」），在 §9.6 写下判据：

> 日志文件 `mtime`（= 收尾时刻）**晚于**每个改动文件的 `mtime` ⇒ 结论覆盖当前树

实测 7 个改动文件全部满足，提前量 **456s~2512s**。数字很"好看"，我就采信了。

#### 9.9.2 它错在哪 —— 判据层里早有明令，而我写之前没查

判据层**第 540 条**（R 组，2026-09-17）写着：

> ★★ **`mtime` 不可作时序判据（本仓第 4 次实测）**。实证：门禁日志 mtime 读作 `2026-09-17T20:10:46Z`，而会话时间是 `2026-09-17T06:17Z` ⇒ 产物 mtime **比产生它的会话晚约 14 小时**（环境/打包/同步所致）。⇒ 判据：凡「日志是否早于改动」这类判据**不得用 mtime 判定**，只能靠内容 sha256 / 行内自证时间戳 / **当场重跑**。

而判据层里同时存在的 **#175 / #288** 恰恰是用 `mtime` 判的（「日志 mtime 早于改动即作废」），#288 还写着「两个方向须同时具备」。**三条并存、未仲裁**，我读到的是 #175 那一支并据此做了反向推广。

#### 9.9.3 仲裁：`mtime` 的失效模式**不对称**，两个方向不能等价

| 方向 | 判出什么 | mtime 被外部推后时 | 危害 |
|---|---|---|---|
| 「日志**晚于**改动 ⇒ 覆盖/有效」 | 激进侧 | **假绿**（实际没覆盖却判覆盖） | **漏检 → 危险** |
| 「日志**早于**改动 ⇒ 作废」 | 保守侧 | 假红（实际覆盖却判作废） | 多跑一次 → 安全 |

⇒ 结论：**激进侧禁用**（就是我在 §9.6 用的那侧，已撤回）；**保守侧保留可用**（#175 仅在此方向有效，且**不得**由它反向推出激进侧）；替代充足判据是 #540 给的 **当场重跑**。

已就地写入 `#540`（追加一段仲裁注记，指向 #175/#288），并新增判据 **#676**（R 组，元层：写验证判据前先 grep 判据层 + 先判「该量被扰动时我判出的是假绿还是假红」）。**未改 #175 原文** —— 它在保守侧仍正确，改写反而会破坏它的原始实证。

#### 9.9.4 附带发现：`mem.mjs rule <n>` **只打印主行**

我改完 #540 后用 `rule 540` 读回核对，输出**仍是旧文本**，一度以为写入失败（同文件 `Edit` 报过 `Successfully edited`，但本轮已踩过一次「同文件并发 `Edit` 后者覆盖前者」）。查 `mem.mjs:214-220` 才知实现是：

```js
const hit = parseRules().find((r) => r.n === n);
console.log(`[${hit.group}] ${hit.n}. ${hit.text}`);   // 只输出主行匹配到的 text
```

判据的**缩进续行不参与 `hit.text`** ⇒ 对「续行」的任何增删，`rule` 都读不出来。**判据工具的读回口径 ≠ 文件实际内容**，核实只能 `grep` 文件本体（grep 已确认 line 752 落盘）。同族：判据 #643 / #655（判据工具静默少抽）。

#### 9.9.5 补账：怎么补一条「硬的」替代判据（两条已实证）

#540 要求「只能靠内容 sha256 / 行内自证时间戳 / **当场重跑**」。本轮把其中两条落到可执行手段上：

| 手段 | 实证读数 | 适用 |
|---|---|---|
| **在 exe 二进制里 grep「本轮新增的测试函数名」** | `axagent_tools-8381afec5f6e6008.exe`（79 MB）：**5 个真名全命中 1 次**，负控（编造名）**命中 0** ⇒ 有区分力 | 判断「这份 exe 是否编于该测试存在之后」（替代 `exe mtime > 源码 mtime`，见 skill 修正） |
| **报告声称 ↔ 源码事实逐项核对** | `output/tmp-91-claims-check.mjs`：**28/28 通过**（含 3 条**反向**断言：`rate_limited` 函数体内无归因词、`registry` 已无「限流走 `permission_denied`」、`attribution_note` 无行续接符；另读数 `TEMPLATE_VERSION = 73`、`*_tool_calls` **恰 10 条**） | 报告 §9.1 的每一项改动都**可被机器复核**，不靠人眼 |
| **定点当场重跑** `-p <最晚改动的 crate> --all-targets` | ⏳ 待 `cargo test --workspace` 释放锁后执行 | clippy / test 结论的覆盖度 |

> ⚠ 写这条时我自己踩了两次「判据自证」：
> ① **首版核对脚本用 `execSync` 调 `grep`** ⇒ 在 cmd.exe 下路径解析失败（中文系统报「系统找不到指定的路径」）⇒ **15 个名字全部报「未见」**，看起来像「报告里写的测试名都是编的」。**判据自身失效伪装成了被测对象有问题**（#7 / #663）——而且那版**连一个正控都没有**，我差点据此去"修"那份完全正确的报告。改纯 Node 遍历 + **先跑正控/负控自证**后（`output/tmp-95-names-check.mjs`），结论反转：9 个测试名**全部真实存在**。
> ② 正控我一度填 `diag_for`（预期命中）—— 它是 **`.rhai`** 函数，本就不该被 `.rs` 扫描面命中。**期望值必须先验证过**（判据 #666），不能拿「我以为它在 `.rs` 里」当预期。

**顺带闭环**：报告 §9.5 列出的形如测试名的标识符**逐个核对**（`output/tmp-95-names-check.mjs`，扫描面 1944 个 `.rs`）：

| 名字 | 位置 |
|---|---|
| `rate_limited_is_distinct_from_permission_denied` / `permission_denied_is_unchanged` | `harness/src/error.rs` |
| `read_only_exempt_set_covers_the_incident_tool` / `mutating_tools_are_excluded_and_all_names_are_real` / `min_interval_exempts_read_only_but_still_guards_others` / `window_limit_still_applies_to_exempt_tools` | `tools/src/audit.rs` |
| `data_quality_rhai_diag_for_arity_matches_all_call_sites` | `src/commands/stock_analysis_setup/seed_consistency_tests.rs` |
| `attribution_note_text_is_single_line` / `unbacked_attribution_is_flagged_in_two_severities` | `crates/rt-workflow/tests/data_quality_placeholder_gate.rs` |

⇒ **9/9 真实存在，无悬空引用**（这正是我先前凭记忆猜名字、4 个里错 2 个的反面保险）。

> 教训归类：这不是「clippy 的问题」，而是**我给验证手段本身下的判据缺乏自证**。四道机械门禁全绿也拦不住一条方向错误的「结论有效性判据」——它是元层缺陷，只会让下层结论**假绿**。#670~#675 都在讲「怎么检出别人的坑」，本条讲「怎么检出**我自己刚写的判据**」：**写判据前先 grep 判据层有没有现成的**（本轮 675 条里就有 #540 直接管这件事，我没查）。

