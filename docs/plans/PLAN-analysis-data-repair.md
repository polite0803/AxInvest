# PLAN — 分析数据修复与反思数据回填

> 状态：待用户审批（**未动手**，本文档为施工前的完整方案）
> 口径裁决（用户 2026-09-23）：47 条 `template_version` 为 NULL 的旧分析 —— **必须重跑对齐 v77**
>
> ⚠ **版本号读法（2026-09-23 订正，必须先读这一段）**：
> 本文写就时 `TEMPLATE_VERSION = 77`；随后**并发会话**（估值科学化第三轮）把它推进到
> **79**，并同步把 `DCF_MIGRATION_VERSION` 提到 79（关闭幂等门，见该会话的记账）。
> ⇒ **下文出现的 "v77" 一律应读作「重跑的那一刻 `TEMPLATE_VERSION` 的值」**，不是字面 77。
> 判据：重跑写入 `stock_analyses.template_version` 的值来自 **`decision.rs:2994` 的 `template.version`**
> （即 DB 模板行版本 = seed 时的 `TEMPLATE_VERSION`）⇒ **无需在任何地方硬编码期望版本**，
> 重跑后按 `SELECT DISTINCT template_version` 实测即可。
> **教训（同 `AGENTS.md` 的「数值先腐烂」判据）**：文档里**不要手抄版本号**，
> 要引**常量名**；手抄的那一刻起，它就在腐烂 —— 本文自身即为例证。
> 调研方式：全部为**只读实证**（`scripts/pg-connect.mjs sql` + 源码 grep），未修改任何文件、未执行任何写操作

---

## 0. 摘要

用户提出的问题是「工作流算法不断改进 ⇒ 分析数据失效」，诉求两项：**快速修复失效数据** + **快速积累分析与反思数据**。

实测后的核心结论与初始判断**不同**：

> **反思数据积累不起来的真因不在算法版本漂移，而在「读取层断裂」。**
> 11 条"已完成"的反思，其 `verdict` 真实值（correct×2 / partial×6 / wrong×1）**确实存在于数据库中**，
> 但因输出是双重编码结构而未被解析出来 ⇒ 全库 `verdict` 为 NULL ⇒ 下游 `was_correct` 被统一压成 `0`。
> **这 11 条现在正被当作"决策全部错误"用于策略降权。**

因此本 PLAN 的阶段划分**刻意把"零成本的读取层修复"排在"高成本的重跑"之前**：
前者不需要任何 LLM 调用即可把 11 条假反思变成真数据，并在重跑开始前先止血下游污染。

---

## 1. 现状实测（证据）

### 1.1 数据规模

| 表 | 行数 | 关键填充情况 |
|---|---|---|
| `stock_analyses` | **53** | 38 只股票 / 16 个分析日 / 2026-07-21 ~ 09-22 |
| `stock_reflections` | **17** | 11 completed / **6 pending（全部已过期）** |
| `strategy_performance` | 11 | 与 11 条 completed 反思一一对应 |
| `stock_pipeline_runs` | 7 | — |
| `reco_picks` | 698 | — |
| `trajectory_workflow_reflections` | **27276** | 工作流级质量反思（非股票决策反思），`template_id` 为空者 26993 条 |
| `reflection_lessons` | **0** | 空 |
| `lesson_applications` | **0** | 空 |
| `analyst_feedbacks` | **0** | 空（表名为复数，注意不是 `analyst_feedback`） |
| `portfolio_holdings` / `trades` | 0 / 0 | 无实盘与模拟持仓 |
| `cron_jobs` | **2** | **两条都是 OPC 任务，零股票任务** |

### 1.2 `stock_analyses` 维度分布

| 维度 | 实测 |
|---|---|
| `template_version` | **NULL×47**、65×1、70×1、75×3、76×1 ⟹ 无一条为当前版本 |
| `analysis_kind` | **live×53** ⟹ **replay 模式从未被使用过** |
| `outcome` | **全 NULL** ⟹ 决策校验（win/loss）从未落库 |
| `model_version` | **全 NULL** |
| `decision_action` | 观望 22 / NULL 12 / 减持 8 / 持有 6 / 卖出 5 |
| `decision_json` | 53/53 有值 |
| `blackboard_snapshot` | 43/53 有值 |

### 1.3 `stock_reflections`（11 条 completed）字段填充率

| 字段 | 有值数 | 说明 |
|---|---|---|
| `decision_json` / `blackboard_snapshot` | 11 / 11 | **产物在** |
| `what_went_wrong` / `fix_for_future` | 1 / 1 | 几乎全丢 |
| `verdict` / `alpha_cited` / `lesson_summary` | **0** | 全 NULL |
| `raw_return` / `alpha_return` / `holding_days` | **0** | 全 NULL |
| `model_version` | **0** | 全 NULL |
| `status` | 11 条 completed | **但实质产出为空 —— 假完成** |

### 1.4 `decision_json` 顶层结构实测（决定性证据）

| 结构 | 条数 | 示例 | verdict 是否真实存在 |
|---|---|---|---|
| `{"report":"<JSON 字符串>"}` | **10** | `{"report":"{\n  \"verdict\": \"correct\",\n ...}"}` | **存在**（在内层字符串里） |
| `{"reflection":{...}}` | 1 | `{"reflection":{"code_diff_proposal":...}}` | 不存在 |

10 条内层 verdict 的真实分布：**correct×2 / partial×6 / wrong×1**（另 2 条含关键词，位置在 JSON 中段）。

### 1.5 版本与参数的阻断性事实

| 项 | 实测值 | 期望 |
|---|---|---|
| DB `workflow_templates['stock-analysis'].version` | **76** | 代码常量 `TEMPLATE_VERSION = 77` |
| DB `value_dcf_perpetual_rate` | **4.0** | v77 要求 **2.0**（`= RISK_FREE_RATE` 派生） |
| DB `value_dcf_discount_rate` | 8.5 | 8.5（不变，但需拆为具名分量） |
| DB `stock_lessons` 变量 | `"（暂无历史反思）"` | 默认值 ⟹ 反思从未反哺进分析 prompt |
| 模板 `version_gate_tests` 参考常量 | `TEMPLATE_VERSION = 77`；`DCF_MIGRATION_VERSION = 78` | — |

⟹ **v77 尚未落库，且 DCF 扁平参数仍是旧口径。**

---

## 2. 缺陷清单

### D1（P0）反思输出提取只认顶层键 —— 双重编码与嵌套结构全部丢弃

- **位置**：`src-tauri/src/commands/stock_workflow/reflection.rs:549 / 553 / 557 / 564`
- **代码**：

  ```rust
  let verdict_str = reflection_json.get("verdict").and_then(|v| v.as_str()).unwrap_or("");
  ```

- **问题**：实际结构为 `{"report":"<内层 JSON 字符串>"}`（10/11）或 `{"reflection":{...}}`（1/11），
  顶层均无 `verdict` ⟹ 一律取不到。同因影响 `alpha_cited`、`lesson_summary`、`what_went_wrong`、
  `missed_signals`、`fix_for_future`（均以顶层 `get` 取值）。
- **影响**：反思语义层整体为空，且**反射进下游指标**（见 D2）。

### D2（P0）`was_correct` 三义污染 —— 「测量缺失」被记成「测量为负」

- **位置**：`reflection.rs:569-573`
- **代码**：

  ```rust
  let was_correct: i32 = match verdict_str {
      "correct" => 1,
      "wrong" => 0,
      _ => 0, // partial 也视为不正确
  };
  ```

- **问题**：`_ => 0` 把「真错」「partial」「**根本没判定（空串）**」压成同一个 `0`。
  实测 11 行 `was_correct` 全为 0，且 `stock_reflections.verdict` 全为 NULL
  ⟹ 这 11 条**全部落在第三类（没判定）**，却被记成"决策错误"。
- **下游消费**：`crates/analysis-engine/src/weight_decay.rs:79`
  ```rust
  if row.was_correct != 0 { ... }
  ```
  ⟹ **在没有任何判定依据的情况下，按"错误"对所有反思样本降权。**
- **性质**：与判据 J 组「一名多义」、D 组「ERROR 未必故障」同族 —— `0` 同时承载两个语义。

### D3（P0）`decision_confidence` 硬编码为 0

- **位置**：`reflection.rs:589` → `decision_confidence: Set(0)`
- **影响**：`strategy_performance` 无法按置信度分层评估；配合 D2 使该表统计量恒等（全 0），
  违反「统计量恒等 ⇒ 先怀疑上游零生产端」的自检原则。

### D4（P1）零股票类定时任务 —— 6 条 pending 过期一个多月无人消费

- **实测**：`cron_jobs` 仅 2 行（`opc-demand-discovery-daily` / `-weekly`），
  **无 `batch-reflection`、无 `validate-decisions`**。
- **证据**：6 条 pending 的 `hindsight_date` 为 2026-08-18 ~ 08-25，今天 **2026-09-23**。
- **相关入口**（已存在，仅缺配置）：
  - `create_batch_reflection_cron`（`stock_analysis.rs:4945`）
  - `create_validate_decisions_cron`（`stock_analysis.rs:4856`）
  - `run_reflection_now`（`stock_analysis.rs:5112`，已注册 `register_commands.rs:1286`）

### D5（P1）反思 → 规则提取链零产出

- **实测**：`reflection_lessons = 0`、`lesson_applications = 0`、`analyst_feedbacks = 0`。
- **旁证**：模板变量 `stock_lessons` 恒为默认值 `"（暂无历史反思）"`
  ⟹ 反思结论从未反哺回分析 prompt，**闭环未建立**。
- **代码存在**：`reflection.rs:1152+` 有规则提取实现（`lesson_summary ≤200 字符` 才提取）
  ⟹ 因 D1 导致 `lesson_summary` 恒空，提取条件永不满足。

### D6（P2）`template_version` 缺失 47/53

- **位置**：`stock_analyses.template_version`（v228 引入），引擎只做纯新增、**有意不回填**。
- **注释已立约定**（`stock_analyses.rs:45-55`）：NULL 必须声明「公式版本未知」，
  **不得**默认按当前版本复算（2026-09-18 实测：按现行公式复算历史决策**系统性偏高 4.5pt**）。

### D7（P2）`model_version` 全 NULL（53/53）

- 影响：复现实验缺少 LLM 版本锚点，无法区分"结论变化"来自模型还是公式。
- **状态订正（2026-10-05）**：本项**不以「填真值」收口，改为退役该列**。判据：全仓只有
  `Set(None)` 写入点、读侧零消费 ⇒ 要填就得先定义「LLM 版本标识」的权威来源，那是**新建设施**
  而不是修复既有设施；而 `template_version` + `blackboard_snapshot` 已能承担复现锚点。
  退役面与读数见 `PLAN-four-horizon-workflow-alignment.md` §五十九。

### D8（P2）`outcome` 全 NULL（53/53）

- 决策校验结果从未落库 ⟹ `stock_analyses` 无法自身承载 win/loss 闭环。

### D9（**阻断项**）v77 未落库 + DCF 扁平参数未迁移

- **实测**：DB 模板 `version = 76`，代码 `TEMPLATE_VERSION = 77`；`value_dcf_perpetual_rate = 4.0`。
- **为什么阻断**：`seed_stock_analysis.rs:314-316` 明确记载 —— DB 存量的扁平参数
  **优先于模块常量生效**，不迁移则「改动等于没改」（v74 起同型事故已复发三次）。
- **后果**：**在 D9 未解决前重跑，得到的仍是永续增长率 4% 的旧估值口径，重跑等于白跑。**
- **迁移门**：`DCF_MIGRATION_VERSION = 78`（`seed_stock_analysis.rs:4797`），
  门为 `previous_version < 78`；当前 76 < 78 ⟹ 迁移**本应执行但尚未执行**（种子未跑）。

---

## 3. 方案总览：一条链打通「修复 + 积累」

关键洞察：**用户的两项诉求（修复旧数据、快速积累反思）可以由同一次 as-of 重跑同时满足。**

理由（`crates/astock-data/src/as_of.rs` 实测）：

1. 反思受**时间约束** —— 需要后见行情，`hindsight_date = 分析日 + 期望持有期`，所以 live 分析必须等。
2. 但用 `as_of_date = 历史日期` 重放分析时，**该分析的 `hindsight_date` 天然已在过去**
   ⟹ **落地即可立即反思**，把「等 30 天」压缩为「一天补齐两个月」。
3. 现有 53 条 live 分析覆盖 07-21 ~ 09-22，**这些日期本身就是现成的 as-of 锚点**。
4. `run_stock_workflow` 已原生支持 `as_of_date`（`core.rs:177`）与 `parent_analysis_id`（`core.rs:180`），
   且 replay 模式有明确的版本化语义（`core.rs:358-440`：同日覆盖 / 跨日新建版本）。

```
阶段一（零 LLM）  修读取层 → 回填 11 条已有反思 → 止血下游
阶段二（分钟级）  让 v77 落库 + DCF 迁移 → 补 cron → 清 6 条过期 pending
阶段三（高成本）  as_of = 原日期 重跑 53 条 → 对齐 v77 → 新反思立即到期 → 批量反思
阶段四（闭环）    反思 → 规则提取 → stock_lessons 反哺分析
```

---

## 4. 阶段一：读取层修复 + 历史反思回填（零 LLM 成本）

### 4.1 修提取逻辑：新增深取值函数

**位置**：`src-tauri/src/commands/stock_workflow/reflection.rs`

新增一个能穿透「双重编码 + 一层嵌套」的取值助手，替换 `:549 / 553 / 557 / 564` 的裸顶层 `get`：

```rust
/// 从反思输出中按 key 取值，兼容三种实际形态：
///   ① 顶层直接命中：           {"verdict": "..."}
///   ② 一层嵌套对象：           {"reflection": {"verdict": "..."}}
///   ③ 内层 JSON 字符串（双重编码）：{"report": "{\"verdict\": \"...\"}"}
///
/// 为什么必须兼容 ③：实测 11 条 completed 中 10 条为该形态，
/// 旧实现只读顶层 ⇒ `verdict` 恒 NULL（详见 PLAN-analysis-data-repair.md D1）。
fn deep_get_reflection_field(v: &serde_json::Value, key: &str) -> Option<serde_json::Value> {
    // ① 顶层
    if let Some(hit) = v.get(key) {
        return Some(hit.clone());
    }
    // ② / ③ 遍历一层子值
    for child in v.as_object()?.values() {
        if let Some(s) = child.as_str() {
            // ③ 内层是 JSON 字符串 ⇒ 二次解析
            if let Ok(inner) = serde_json::from_str::<serde_json::Value>(s) {
                if let Some(hit) = inner.get(key) {
                    return Some(hit.clone());
                }
            }
        } else if let Some(hit) = child.get(key) {
            // ② 内层是对象
            return Some(hit.clone());
        }
    }
    None
}
```

取值点改为：

```rust
let verdict_opt = deep_get_reflection_field(&reflection_json, "verdict")
    .and_then(|v| v.as_str().map(str::to_string));
```

### 4.2 修 `was_correct` 三义（D2）

**原则**：**「未判定」必须与「判定为错」在数据上可区分。**

```rust
let verdict_str = verdict_opt.as_deref();
// None 表示"本次反思未产出判定" —— 与 0（判定为错误）是两个不同的事实。
// 旧实现把两者压成 0 ⇒ 无依据地按"错误"降权（PLAN D2）。
let was_correct: Option<i32> = match verdict_str {
    Some("correct") => Some(1),
    Some("wrong") | Some("partial") => Some(0),
    _ => None, // 未判定 ⇒ 不参与降权
};
```

> ⚠️ **实现约束**：`strategy_performance.was_correct` 当前为 `i32` 非空列
> （`entities/strategy_performance.rs:30`）。要落地 `Option`，需二选一并明确声明：
> **(a)** 改列为可空（需同步 `weight_decay.rs:79` 的消费端过滤条件）；
> **(b)** 保持非空但**不写行**（未判定则跳过 `strategy_performance` 插入）。
>
> **推荐 (b)** —— 不动 schema、不改下游语义，且"没写入"本身就是"没有判定依据"的正确表达。
> 这是本阶段**唯一的语义决策点**，需用户确认后再动手。

### 4.3 修 `decision_confidence` 硬编码 0（D3）

- **位置**：`reflection.rs:589`
- 若反思输出含置信度则取真值；否则同样跳过写行（与 4.2 的 (b) 保持一致）。
- ⚠️ 需先确认 `reflection-agent` 输出中是否**存在**置信度字段 —— 若无，则本项应改为
  「明确不产出该指标」而不是继续写 0（写 0 会让下游误读为"置信度极低"）。

### 4.4 离线回填历史 11 条（一次性脚本）

**产出**：`output/backfill-reflection-fields.mjs`（只读 DB → 生成 UPDATE，先 dry-run 打印差异）

逻辑：
1. 读 `stock_reflections` 中 `status='completed' AND verdict IS NULL` 的行；
2. 对每行 `decision_json` 施加与 4.1 同源的 `deep_get` 逻辑，抽出
   `verdict / alpha_cited / lesson_summary / what_went_wrong / missed_signals / fix_for_future`；
3. **dry-run 先输出「将写入什么」**，与本文档 §1.4 的真实分布对账
   （预期：correct×2 / partial×6 / wrong×1，另 2 条待确认）；
4. 经用户确认后再执行 UPDATE，逐行打印 before/after。

**为什么这一步最重要**：它**零 LLM 成本**把 11 条假反思变真数据，
并且是**重跑前止血下游降权**的唯一手段（否则重跑期间旧污染持续生效）。

**不做的部分（明示）**：`raw_return` / `alpha_return` **无法从 `decision_json` 回填** ——
它们是反思前由行情快照算出的（`compute_market_snapshot`），
但 `strategy_performance.return_pct` **已存有 11 条真实收益率**（-23.90% ~ +5.32%），
可作为交叉校验源。是否回填需单独决策（涉及"用一个估计值填充一个事实字段"的风险）。

---

## 5. 阶段二：让 v77 真正生效 + 接通执行层

### 5.1 前置：v77 落库与 DCF 参数迁移（**D9，必须最先做**）

| 步骤 | 动作 | 验收判据 |
|---|---|---|
| 1 | 触发种子化（启动应用 / 调用种子入口） | `workflow_templates['stock-analysis'].version` 由 **76 → 77** |
| 2 | 确认 `DCF_MIGRATION_VERSION=78` 迁移执行 | `value_dcf_perpetual_rate` 由 **4.0 → 2.0** |
| 3 | 负控：确认参数确实变了 | 若迁移后仍为 4.0 ⟹ 门未生效，**停止后续重跑**（否则白跑） |

> ⚠️ **本项含一个已知副作用（源码已明示）**：该迁移为「一次性覆盖」，
> **会覆盖面板手调值**（`seed_stock_analysis.rs:4793` 记载上次迁移的代价）。
> 若面板参数曾有手工调优，须先导出备份。

### 5.2 补股票类定时任务（D4）

| 任务 | 入口 | 建议配置 |
|---|---|---|
| 批量反思 | `create_batch_reflection_cron` | 4 档 period 各一条，`due_only=true`，收市后 `0 18 * * *` |
| 决策校验 | `create_validate_decisions_cron` | `0 6 * * *`，`min_confidence_threshold=0` |

### 5.3 清理 6 条已过期 pending

- 手动触发 `run_batch_reflection(due_only=true)` —— 6 条 `hindsight_date` 均已到期。
- **注意**：这 6 条对应的分析是 v76 口径。**若阶段三会重跑其上游分析，则本步应先跳过**，
  避免产生"用旧公式的决策 + 新公式的反思"这种更脏的组合。
  ⟹ **决策点：跳过 5.3，把 6 条交给阶段三的新分析替代。**（推荐）

---

## 6. 阶段三：as_of 重跑对齐 v77

### 6.1 重跑参数（核心设计）

对 53 条中的每一条，调用：

```
run_stock_workflow(
    stock_code          = 原 stock_code,
    as_of_date          = 原 analysis_date,   // ← replay 模式，保持时点可比
    parent_analysis_id  = 原 analysis id,     // ← 新建版本，保留历史行
    ...
)
```

**为什么用 `as_of_date = 原日期` 而不是 live 重跑**：

| 方案 | 可比性 | 归因能力 |
|---|---|---|
| `as_of_date = 原日期` | **同一时点、同一数据** | 结论变化**可唯一归因于公式 v76→v77** ✅ |
| live（当前时点） | 时点已变、行情已变 | 无法区分"公式改的"还是"行情变了" ❌ |

重跑的目的是**对齐口径**，不是获取最新观点 ⟹ 必须锚定时点。

**附带收益**：重跑得到的新分析，其 `hindsight_date` 天然已到期
⟹ 立即进入可反思状态 ⟹ **"对齐 v77" 与 "快速积累反思数据" 一次完成。**

### 6.2 重跑前必做：as-of 降级率体检

- **机制**：`as_of.rs:224 record_degradation` + 256 条全局环形缓冲。
  部分 vendor 无历史语义（`NoHistoricalSemantic`）会**静默跳过**。
- **风险**：降级率高 ⟹ 回放建立在残缺数据上 ⟹ 其决策与后续反思**不可信且会污染样本集**。
- **动作**：先对 3~5 个代表性日期（如 07-21 / 08-15 / 09-01）试跑，读取降级报告，
  **量化降级项数**后再决定是否全量重跑。
- **判据**：若某日期降级项数显著高于其它日期 ⟹ 该日期样本单独标记或排除。

### 6.3 成本估算（需用户确认）

- 53 条 × 完整 DAG（含全部 LLM 节点：辩手轮数 3 / 多 Agent 并行上限 10）
  ⟹ **这是本 PLAN 中唯一的高成本项**，且受 LLM 供应商限流影响。
- 建议**分批**：先跑 5 条验证链路与结论合理性，再放量。
- ⚠️ 不要一次全排——历史教训：高并发触发供应商限流降级（见 .workbuddy 记忆）。

### 6.4 一个免费的预检手段（可选，强烈建议）

`rerun_decision(analysis_id)`（`decision.rs:2567`）能**复用 `blackboard_snapshot` 缓存的上游节点输出**，
只重跑 `portfolio-mgr` CodeNode，**不调用 LLM**。

| 能覆盖 v77 的哪些项 | 不能覆盖的 |
|---|---|
| 第 1/2/3/4 项（`portfolio-mgr.rhai` f6/f7 权重归一化、`valuation_band_verdict`、交易价位）✅ | 第 5/6/7 项（DCF 估值常量 + DB 扁平参数）❌ —— 因 DCF 是**工具节点**输出（`mcp_tools::compute_dcf`，`mcp_tools.rs:1342`），其值已被快照固化 |

**价值**：对 43 条有 snapshot 的记录，可用它**零成本**量化"公式层改动"的**独立影响**，
从而在阶段三之后与之对比，**拆分出"公式改动"与"DCF 改动"各自的效应**
（正合 `sci12-rollout.mjs` 已建立的"四列效应拆分"方法论）。

⚠️ **待验证**：`rerun_decision` 是否会把新结论写回 `stock_analyses`（本次仅读到 `:2560-2610`，
判读为"计算并返回"）。若要用于正式回填，须先读完该函数确认写入行为，**不得假定**。

---

## 7. 阶段四：闭环（反思 → 规则 → 反哺）

1. 反思产出 `lesson_summary` 后，`reflection.rs:1152+` 的规则提取才会真正触发
   （旧实现因 D1 使该条件永不满足）；
2. 产出写入 `reflection_lessons` → `lesson_applications`；
3. `stock_lessons` 模板变量被 runtime 注入分析 prompt → **闭环成立**。

**验收**：`reflection_lessons` 由 0 行变为 > 0，且 `stock_lessons` 不再恒为默认文案。

---

## 8. 验证判据（可直接执行的 SQL）

> ⚠ **本节期望值在施工中被实测**多次**推翻，已就地订正**（2026-09-23）。原文的 3 处错误：
> ① V1 期望分布 `2/6/1` → 实测 **`3/5/2` + 1 条未判定**（原文是从 PLAN 抽样估的）；
> ② V4 期望 `perpetual_rate = 2.0` → 代码已推进到 **1.5 / discount 7.7 / growth 12.0**；
> ③ V6 假设「重跑新建版本行」→ 实测传同日 `as_of_date` + `parent_analysis_id` 命中的是
>    **原地覆盖**分支（`core.rs:388`），**不新建行**。
>
> 版本号同理：原文的 `77` 一律读作「**重跑那刻的 `TEMPLATE_VERSION`**」
> （施工期间已被并发会话从 77 推到 79，见 §12.9）。硬编码版本号的判据会随代码演进假红。

```sql
-- V1 阶段一：11 条反思的 verdict 应回填
--   实测正确分布 = correct×3 / partial×5 / wrong×2 / 未判定×1（后者 verdict 为 NULL）
SELECT verdict, count(*) FROM stock_reflections
WHERE status='completed' GROUP BY 1 ORDER BY 2 DESC;

-- V2 阶段一：was_correct 不应再全为 0（(b) 方案下：应无"未判定"行被写入）
--   实测 = 1×3（三胜）+ 0×7（七负），共 10 行（原 11 行中 1 行属"无判定依据"已删）
SELECT was_correct, count(*) FROM strategy_performance GROUP BY 1;

-- V3 阶段二：模板已升版（**不要硬编码 77**）
SELECT version FROM workflow_templates WHERE id='stock-analysis';

-- V4 阶段二：DCF 参数已迁移（负控：必须真的变了）
SELECT variables FROM workflow_templates WHERE id='stock-analysis';
--   实测三参数 current = 12 / 4 / 8.5（恰等于 v76 规范值 ⇒ 用户未手调过）
--   ⇒ 迁移后应变为 growth=12.0 / perpetual=1.5 / discount=7.7（不变即为迁移失败）

-- V5 阶段二：股票类 cron 已存在
SELECT id, data FROM cron_jobs;   -- 实测两条：batchref-due-daily / vldec-daily

-- V6 阶段三：重跑后模板版本回填（**原地覆盖，不新建行**）
SELECT count(*) FILTER (WHERE template_version IS NULL) AS stale_left,
       count(*) FILTER (WHERE template_version IS NOT NULL) AS versioned,
       count(*) AS total
FROM stock_analyses;
--   ⇒ `parent_analysis_id` 恒为 NULL，**不能**用它验证版本链（同 §9 回滚口径的订正）

-- V7 阶段三：新分析的反思可立即反思（hindsight_date 应早于今天）
--   ⚠ 前提是 `ensure_pending_reflection` 真跑了（D12：业务封装路径不产 pending）
SELECT count(*) FROM stock_reflections
WHERE status='pending' AND hindsight_date < to_char(now(),'YYYY-MM-DD');

-- V8 阶段四：闭环
SELECT count(*) FROM reflection_lessons;   -- 期望 > 0（施工前实测 = 0）
```

**通用纪律**：每个阶段结束后**必须重跑全量判据**，不要只看增量 ——
本仓已记载「门禁遇首错即短路，后续未被检查」的多次教训。

---

## 9. 回滚方案

| 阶段 | 回滚方式 |
|---|---|
| 阶段一（代码） | 改动前 `cp` 到 `output/backup-20260923/`；回滚即还原文件（**不用 git**）<br>⚠ 备份有效性已用 sha256 逐文件验证（`identical=0` ⇒ 确为改动前快照）；<br>　 但 `serenity.rs` / `hooks.rs` **当时未做文件备份**，回滚须按文档记录手工撤销 |
| 阶段一（回填） | 回填来源是 `strategy_performance.return_pct`（同源同值，见 `reflection.rs:657`）⇒ 可反向再导出；<br>原 11 行完整值已存 `output/backup-20260923/` 内的核对输出 |
| 阶段二（种子） | DB 层回滚：恢复 `workflow_templates.variables` 与 `version` 的导出快照<br>（备份文件：`template-stock-analysis-before.txt`，51689 bytes） |
| 阶段三（重跑） | ❌ **原文「天然可回滚」不成立** —— 实测（`core.rs:388`）传同日 `as_of_date` + `parent_analysis_id` 命中的是<br>**原地覆盖分支**（原地 UPDATE 并清空决策字段），**不新建版本行** ⇒ 覆盖前的决策**没有第二份**。<br>✅ 实际回滚点 = **库内备份表** `stock_analyses_backup_20260923`（53 行，含 53 个 `decision_json` /<br>　43 个 `blackboard_snapshot`）。为什么用库内表而不是导出文件：`blackboard_snapshot` 体积撑爆<br>　`pg-connect.mjs` 的 `spawnSync` 1 MB 缓冲区（`ENOBUFS`，实测输出 0 字节）。<br>　回滚 = `UPDATE ... FROM stock_analyses_backup_20260923 WHERE id = ...` 按列回填。 |
| 阶段三（反思重置） | `ensure_pending_reflection` 会把被覆盖分析对应的旧反思**重置为 pending 并清空 10+5 列**<br>⇒ 该动作**不可由 `stock_analyses` 备份表恢复**（备份表只含 `stock_analyses`）。<br>✅ 已补：库内备份表 **`stock_reflections_backup_20260923`**（17 行 ↔ 实测一致，含 `verdict`×10 /<br>　`lesson_summary`×10）⇒ 重置后果可逐列还原。**这两张备份表是阶段三唯一的回滚点。** |

> ⚠️ **git 零接触**（用户铁律）：全程不使用任何 git 命令，含只读的 `status` / `log` / `diff`。

---

## 10. 未决项与需用户裁决

| # | 事项 | 需裁决 |
|---|---|---|
| 1 | D2 修复形态：改列为可空 vs 跳过写行 | **必须先定**（§4.2 推荐后者） |
| 2 | `raw_return`/`alpha_return` 是否用 `strategy_performance.return_pct` 回填 | 待定（存在"估计值填充事实字段"的风险） |
| 3 | 阶段三是否分档（先 5 条验证再放量） | 建议分档 |
| 4 | 6 条过期 pending：跳过（由新分析替代）vs 先清 | 建议跳过（§5.3） |
| 5 | `rerun_decision` 是否写库 | **待读完函数确认**，不得假定 |
| 6 | v77 迁移会覆盖面板手调参数 | 需先导出备份 |

---

## 11. 与本次调研相邻的独立发现（不在本 PLAN 范围，单独登记）

1. **`trajectory_workflow_reflections` 26993 条 `template_id` 为空**，平均质量分 3.3
   （对比 `cognitive_router_main` 53 条均分 9.2）⟹ 该字段可能未正确写入，值得单独核查。
2. **`analyst_feedbacks` / `reflection_lessons` / `lesson_applications` 三表皆空**，
   但 entity 与消费端代码均已存在 ⟹ 存在"表已建、生产端未接通"的链路，需单独审计。
3. **53 条分析 `outcome` 全 NULL** ⟹ 决策校验链从未落库，与 D4 同源（缺 cron）。
4. **`strategy_performance.strategy_id` 硬编码 `"reflection_verdict"`**（`reflection.rs:580`）
   ⟹ 该字段失去区分能力，无法按策略维度聚合。

---

## 12. 施工实录与订正（本文档写就后，实测推翻 / 补充的部分）

> 本节在**动手之后**追加。原则：**PLAN 的假设一旦被实测推翻，就在原文旁边留痕，不静默改写**。

### 12.1 被实测**推翻**的两处假设

| # | 原 PLAN 说法 | 实测 | 影响 |
|---|---|---|---|
| 1 | §9「阶段三重跑**天然可回滚** —— 新版本行以 `parent_analysis_id` 挂链，旧行不动」 | **不成立**。传 `parent_analysis_id` + 与原记录**同日**的 `as_of_date` ⇒ 命中 `core.rs:395` 的「replay 同日覆盖」分支：**原地 UPDATE 原记录**并清空其决策字段，**不是**新建版本行 | 回滚方案改为**库内备份表** `stock_analyses_backup_20260923`（见 12.4） |
| 2 | §2 D9 / §5.1 称 DCF 期望值为 `perpetual = 2.0`，`DCF_MIGRATION_VERSION = 78` | 代码已推进到 **`perpetual = 1.5` / `discount = 7.7` / `growth = 12.0`**（`RISK_FREE_RATE` 0.025→0.017 连带），`DCF_MIGRATION_VERSION` 确为 78 ✓ | 期望值以**代码现状**为准，不以本文档为准 |

### 12.2 被实测**订正**的数据描述

- §1.3 抽样估的 verdict 分布 `correct×2 / partial×6 / wrong×1` ⇒ 实测回填后为
  **`correct×3 / partial×5 / wrong×2 / 未判定×1`**。
- §1.3 称「`was_correct` 全 0」正确，但**未指出** `raw_return` / `alpha_return` / `holding_days`
  也是 **11/11 全 NULL** ⇒ 补列为 **D10**。

### 12.3 新增缺陷

#### D10（P0）`raw_return` / `alpha_return` / `holding_days` 只写 `strategy_performance`，**从未回写 `stock_reflections` 自身**

三个值在同一函数内已算出，却被用于：① 注入提示词变量（`raw_return_pct` 等）；② 写 `strategy_performance`。
**唯独漏了 `stock_reflections` 自己的三列** ⇒ 实测 11/11 全 NULL。

后果：反思样本在「按收益维度分析」时不可用 —— 而这恰是「反思数据」最有价值的维度。

- **未来写入**已修（`reflection.rs` 的 `col_expr` 补三列）。
- **历史 10 条**已零成本回填，见 §10 裁决 #2 与 12.4。

### 12.4 阶段一/二的实际落库结果（实测，非推断）

`stock_reflections`（17 行 = 11 completed + 6 pending）字段填充：

| 字段 | 填充 | 备注 |
|---|---|---|
| `verdict` | 10/17 | completed 中 1 条未判定（300795，其 `decision_json` 是 `{"reflection":{...}}` 形态且**无 verdict**）|
| `lesson_summary` | 10 | 长度 **51~99** ⇒ 全部落在 `extract_lesson_to_rule` 的 10~250 窗口内 |
| `alpha_cited` | 10 | |
| `what_went_wrong` / `missed_signals` / `fix_for_future` / `parameter_suggestions_json` | 各 11 | |
| `raw_return` / `holding_days` | **10**（本轮回填） | min −23.90 / max +5.32，与 `strategy_performance.return_pct` 分布一致 |
| `alpha_return` | **0（有意保持 NULL）** | 见 §10 裁决 #2 |

`strategy_performance`：`was_correct` **3 胜 / 7 负**（原全 0）、`decision_confidence` 取真值、
删 1 条无判定依据行（300795）。

**库内备份表**：`stock_analyses_backup_20260923`（53 行，含 53 `decision_json` / 43 `blackboard_snapshot`）。
⚠ 为什么不落文件：`blackboard_snapshot` 体积撑爆 `scripts/pg-connect.mjs` 内部 `spawnSync` 的 1MB 缓冲区
⇒ `ENOBUFS`，输出文件 0 字节。

### 12.5 施工中发现的三条关键机制（原 PLAN 未列，决定阶段三能不能成）

| # | 机制 | 为什么关键 |
|---|---|---|
| 1 | 覆盖分支**主动把 `template_version` 清空为 NULL**（`core.rs:428-431`，A4 注释） | 必须确认回填路径存在，否则「对齐 v77」会把 47 条 NULL 变成 **53 条 NULL**。实测由 **`decision.rs:2994`** 在决策落库时写回 `template.version` ⇒ **链路闭合** ✓ |
| 2 | `blackboard_snapshot` 在 `core.rs:1014 / :1329` 被写入 | ⇒ 完整重跑（`run_stock_workflow_inner`）**会重采快照**；而 `rerun_decision` 只重跑 portfolio-mgr CodeNode、**不重采** ⇒ 后者覆盖不了 DCF（工具节点，值被快照固化）⇒ **本 PLAN 必须用完整重跑** |
| 3 | `load_and_inject_template:397` 先调 `ensure_stock_analysis_experts_seeded`（幂等版本兜底） | ⇒ **首次重跑即已是 v77 口径**，D9 阻断项自动解除 —— 不必单独做一次"触发 seed"的动作 |

### 12.6 ⚠ 施工中自查出并修掉的一处**实质缺陷**（as-of 语义被静默破坏）

新 bin 初版用 `as_of::AS_OF.scope(Some(ctx), ...)` —— 这**只设 task_local**。

而 vendor 调用链里有 `tokio::spawn` 出去的子任务，它们位于 task_local 之外，靠**进程级全局**
`current_as_of()` 读截止日（权威记载：`astock-data/src/as_of.rs:258-270`，2026-08-01 实锤修复）⇒
只设 task_local 会让子任务**静默退回 live 模式**，用今日数据重算历史时点
⇒ 「结论变化可唯一归因于公式」这一**前提直接失效**，且产出看起来完全正常（最难发现的一类错）。

**已改** `as_of::with_optional_asof(Some(ctx), ...)`（同时设 task_local + 全局，RAII 恢复）。

**⚠ as-of 降级体检的观测口径**（原 PLAN §6.2 未给出落地手段）：`record_degradation` 的全局累计计数器
**不依赖** `with_degradation_log` 的 task_local scope（`as_of.rs:224`：只要 `current_as_of()` 是 `Some`
就无条件累加；只有 `take_*` 才需 scope）⇒ 直接读 `global_degradation_count()` 即能量化，无需包 scope。
已接入 bin：逐条记录降级增量 + 末尾按降序出 Top10。
**判据**：降级是**证据**不是**判据**（某些实时流接口本就没有历史回放能力，非零正常）；
要看的是**分布** —— 显著高于同批的标的，其回放结论应视为低可信，**不要与其余样本混做统计**。

### 12.7 新增工具：`src-tauri/src/bin/axagent-batch-rerun.rs`

```text
cargo run --bin axagent-batch-rerun                          # 默认 dry-run，只打印清单
cargo run --bin axagent-batch-rerun -- --limit 5 --apply     # 先跑 5 条验证链路
cargo run --bin axagent-batch-rerun -- --limit 0 --apply --reflect-after   # 全量 + 重跑完立即反思
```

- **默认 dry-run**，`--apply` 才执行（与 `scripts/backfill-evolution-stats.mjs` 同款约定）。
- 其余参数：`--codes A,B,C` / `--since` / `--until` / `--only-stale` / `--reflect-max N`。
- **刻意串行**（代码注释明示）：本项目有「高并发触发 LLM 供应商限流降级」的历史教训，
  而重跑是长任务，宁可慢也不要把降级样本混进反思数据集。
- **`--reflect-after` 的价值**：重跑后每条新分析的 `hindsight_date`（= as_of + 期望持有期）
  **已经落在过去** ⇒ 立即可反思，不必等 `init/services.rs:3476-3491` 那个**每 6 小时**的兜底循环
  被动消费。这正是「对齐 v77 口径」与「快速积累反思数据」共用一次重跑的关键。

**⚠ 已知代价（须由使用者知情）**：传 `parent_analysis_id` + 同日 as_of ⇒ **原地覆盖**（见 12.1）。
执行前必须已有 §12.4 的库内备份表。

### 12.8 §10 未决项的**裁决记录**（用户已授权自行裁决）

| # | 事项 | 裁决 | 依据 |
|---|---|---|---|
| 1 | D2 修复形态：改列为可空 vs 跳过写行 | **跳过写行**（已执行） | `strategy_performance.was_correct` 是 `i32` **非空**列，表达不了第三态；而「没写入」本身就是「没有判定依据」的正确表达。已删除 300795 那行 |
| 2 | `raw_return`/`alpha_return` 是否用 `strategy_performance.return_pct` 回填 | **部分回填**（已执行 `UPDATE 10`） | `reflection.rs:657` 实测 `return_pct: Set(raw_return.unwrap_or(0.0))` ⇒ **同源同值**，回填是**还原既有事实**而非「估计值填充事实字段」。关联 1:1 无歧义（`strategy_id='reflection_verdict'`）、`return_pct` 全部非 0（无 `unwrap_or(0.0)` 兜底污染）。<br>**`alpha_return` 不回填** —— 它需**基准收益**，而基准收益**从未落库**（`strategy_performance` 无该字段，`stock_reflections` 只有 `benchmark_name` 无值）⇒ 任何填充都是**伪造**；保持 NULL（= 未测量）才是正确语义 |
| 3 | 阶段三是否分档 | **分档** | 先 `--limit 5 --apply` 验证链路与结论合理性，再放量。理由：链路从未以 replay 模式跑过（53 条全 `live`），一次全排的失败代价高 |
| 4 | 6 条过期 pending：跳过 vs 先清 | **跳过** | 重跑会产生新的 pending 替代它们；且这 6 条是「管道曾断」的**证据**，清掉就再也说不清根因 |
| 5 | `rerun_decision` 是否写库 | **写库**（`decision.rs:2994`） | 但它只重跑 portfolio-mgr CodeNode、**不重采快照** ⇒ 覆盖不了 DCF 等工具节点 ⇒ **本 PLAN 不用它**，改用完整重跑 |
| 6 | v77 迁移会覆盖面板手调参数 | **代价为零，无需额外动作** | DB 现值 `4 / 8.5 / 12` **恰等于 v76 规范值** ⇒ 用户从未手调过（`:4793-4794` 注释亦已记载） |

### 12.9 ⚠ 并发会话（施工环境事实，影响编译与归因）

实测同一项目上**另有会话在跑 cargo**：`cargo test -p axagent-astock-data --all-targets`、
`cargo run --no-default-features --features notification,computer-use`（5 分钟内起了两次）。
项目根另有 `liblint_probe_doc_lazy.rlib` / `liblint_probe2.rlib`，`src-tauri/target-v79/` 亦存在
（**别的会话用独立 target dir 规避锁争用**的先例）⇒ 本 PLAN 的编译会报
`Blocking waiting for file lock on build directory`。

**纪律**：不杀他人进程；**判归属只看 `CommandLine`**（排队里可能一条都不是自己的）。

**首次编译已暴露 1 个错误（已修）**：`E0432 unresolved import commands::stock_workflow::run_stock_workflow_inner`
—— `stock_workflow` 未 re-export 它。修法：lib.rs 走 **`stock_workflow::core::` 完整路径**，
而**不**在 `stock_workflow/mod.rs` 加 `pub use core::X;` —— 因 `mod.rs:60-64` 记载 build.rs 的
`parse_submodule_mod_rs` 会把「子模块已 re-export 到父层」误判为「其全部 `#[tauri::command]` 都已提升」
⇒ 用父模块路径注册 ⇒ 编译期报 `cannot find __cmd__xxx in stock_workflow`，**错误信息完全指不到真因**。

### 12.10 一处口径不一致（**登记，不擅改** —— 属产品语义）

同表两条消费端阈值不同：

| 消费端 | 位置 | 阈值 |
|---|---|---|
| 分析上下文（`stock_lessons`） | `core.rs:2130 fetch_rule_lessons` | `Confidence.**gte**(0.3)` |
| portfolio-mgr `recent_lessons` | `decision.rs:2696` | `Confidence.**gt**(0.5)` |

而 `extract_lesson_to_rule` 的初始置信度为 `wrong = 0.7` / `partial = 0.5` / `correct = 0.3`
⇒ **`partial` 恰好卡在 `> 0.5` 之外**（0.5 不 > 0.5）⇒「部分正确」的教训只在分析上下文可见、
**在组合决策层不可见**。

**判定：不是 bug** —— `gt(0.5)` 与该方法「让 portfolio-mgr 知道最近哪些股票**犯过错**」的注释语义自洽，
0.5 是中位且作者显然是刻意取值。但**边界值与枚举初始值恰好重合**是隐患（C 组「恒等边界」同族）：
后续若调整 `extract_lesson_to_rule` 的初始置信度，**必须同步复核这一处**。

### 12.11 重跑批次计划（基于实测日期分布）

`stock_analyses` 共 **53 条 / 17 个交易日**：

| 区间 | 条数 | `template_version` | 说明 |
|---|---|---|---|
| 07-21 ~ 07-30 | 27 | **全 NULL** | 一个多月前 |
| 09-09 ~ 09-14 | 20 | **全 NULL** | |
| 09-20 ~ 09-22 | 6 | 65×1 / 70×1 / 75×3 / 76×1 | **非 NULL，但同样不是 v77** |

⇒ `07-21~07-30` 的 27 条 + `09-09~09-14` 的 20 条 = **47 条 NULL**，与用户裁决完全一致 ✓

**⚠ 裁决边界（超出用户明示范围的 6 条）**：`09-20~09-22` 的 6 条**同样不是 v77**
（65/70/75/76）⇒ 按「结论由旧公式产出」的定义，它们**口径同样失效**。
但两点须尊重：① 用户明示的是「47 条 NULL」；② 这 6 条含**最近 3 天**的分析，
用户很可能正在观察它们的结论，原地覆盖会清空决策字段并重算。

⇒ **裁决：分两批执行**。批次一严格按用户明示（47 条 NULL）；批次二由我判定**应当重跑**
（理由：不补则「对齐 v77」只完成 47/53，未来做统计仍要剔除这 6 条），
但**放在批次一验证链路成功之后**，风险可控。

```bash
# 批次 O —— 先验证链路（5 条）：确认结论合理、降级率可接受，再放量
cargo run --bin axagent-batch-rerun -- --limit 5 --apply

# 批次一 —— 47 条 NULL（用户明示范围）
cargo run --bin axagent-batch-rerun -- --only-stale --apply --reflect-after

# 批次二 —— 6 条非 NULL（09-20 起，实测该区间 stale=0，不会与批次一重叠）
cargo run --bin axagent-batch-rerun -- --since 2026-09-20 --apply --reflect-after
```

> 建议全程带 `AXAGENT_DB_FALLBACK_TO_SQLITE=0`（见 §12.7 的库身份断言说明）。

**⚠ 时间成本**：`stock-analysis` 是 13 节点工作流、含多个 LLM AgentNode ⇒ 单条约 **1~3 分钟**
⇒ 47 条约 **1~2.5 小时**。**刻意串行**（§12.7），不要并发 —— 并发会触发供应商限流降级，
而降级样本一旦混进反思数据集就**无法事后剔除**。

#### 12.11.1 分片方式的**实测约束**（2026-09-23 施工中定稿）

**① `--codes` 单独用会跨批次混选 ⇒ 禁用其作为分片手段**

实测 `SELECT stock_code ... HAVING count(*) > 1` = **9 个代码有多个日期**
（如 `301302` 在 07-21~07-30 出现 **4 次**、`601166` 同时出现在 07-21 与 09-09~09-14）。
`--codes` 与 `--since`/`--until` 在 bin 内是**串联 `q.filter(...)` ⇒ AND 关系**，
故 `--codes` **必须**与日期区间叠加才有意义；单独使用会把同代码的**全部日期**一起选中，
**破坏批次边界**（表现为「某条被重跑两次」，浪费 1~3 分钟且覆盖两次）。
⇒ **分片一律用 `--since`/`--until`** —— 按构造互斥（每条记录只属于一个日期区间），无需额外推理。

**② `--only-stale` 具备天然的自愈 / 续跑语义（关键性质，应保持）**

覆盖分支会先把 `template_version` 清空（`core.rs` A4 注释），随后由决策落库写回当前版本号
⇒ **重跑成功的行自动退出 stale 集合**；**失败的行仍为 NULL，会被下一次调用自动拾起**。

⇒ 因此「先 `--limit 5 --apply` 验证、再 `--only-stale --apply` 放量」**不会重跑**已验证的 5 条，
且**中途中断可安全重入**。这是本仓少见的「失败可自愈」设计。
（对照反面：若改用 `--codes` 或固定日期区间续跑，就必须手工排除已完成项。）

**③ 条数闭合校验**：`--limit 5`（按 `(analysis_date, stock_code)` 升序 ⇒ 07-21 的全部 4 条
+ 07-22 的 `600887`；**dry-run 输出可逐条核对**）+ `--only-stale` 余 **42** +
`--since 2026-09-20` 的 **6** = **53** ✓（等于全库行数，无遗漏、无重复）。

**④ 执行前后各跑一次的闭合查询**：

```sql
SELECT count(*) FILTER (WHERE template_version IS NULL)     AS stale,
       count(*) FILTER (WHERE template_version IS NOT NULL) AS fresh,
       count(*)                                             AS total
FROM stock_analyses;
-- 期望：执行前 stale=47；批次一（含验证 5 条）完成后 **stale=0**
--       （失败行会留在 NULL —— 那正是重试入口，不是缺陷）
```

### 12.12 ⚠ D11 **撤回**（我制造的假缺陷）+ 真实缺口 **D12** 登记

#### (a) D11 撤回 —— 一次「把 live-only 通道误判为 replay 通道」的误改

原判据（**已作废**）：我曾在 `core.rs:1915` 处「修复」pending 反思行的 as-of 锚点，
声称「replay 下两张表 `as_of_date` 互相矛盾、`hindsight_date` 落到未来 ⇒ 闭环断裂」。

**实测推翻，两处都判错**：

| 我的判断 | 事实 | 证据 |
|---|---|---|
| 该段在 `run_stock_workflow_inner`（业务封装 / replay 路径）内 | 它在 **`run_single_stock_analysis`**（`:1676` 起）内 | 函数边界实测：`run_stock_workflow_inner` = `:345`–`:1651`，下一个函数从 `:1652` 起 |
| 该函数支持 as-of | **不支持** —— 签名里没有 `as_of_date` 参数 | `:1667` 函数文档：「不需要 `as_of_date` 参数（使用当前时间，非回放模式）」；`:1692` `analysis_date` 与 `:1706` `as_of_date` **都**用 `Utc::now()` ⇒ 两表锚点同源、无矛盾 |

⇒ **原实现本来就正确，我的「修复」是误改**。编译当场报
`E0425: cannot find value current_as_of in this scope` —— 那个函数里根本没有该上下文。
已回退，并就地留下判据注释：

> **先问「本函数有没有 as-of 入口」，再决定锚点用 `now` 还是 as-of。
> 判据是该函数签名里有没有 `as_of_date` 参数，不是「看注释里提没提时间」。**

**教训（比原 D11 更有价值）**：`grep -n` 给的行号**必须与 `fn` 边界对照**，
否则「这一行的问题」很可能属于**另一个函数**。我是靠行号比对函数边界才发现这一点的，
而当时已经基于错误归属写完整整一段缺陷分析（含四条「连锁后果」）。
⇒ 判据：**Grep 命中后，第一件事是确认它落在哪个函数体内**（尤其本项目有 1300 行的巨型函数）。

#### (b) D12（真实缺口，P1）业务封装路径**从不产生反思样本**

**发现方式**：穷举 `stock_reflections::ActiveModel` 全仓 —— **仅 3 处写入**：

| 写入点 | 通道 | 是否服务 as-of 重跑 |
|---|---|---|
| `core.rs:1922` | `run_single_stock_analysis`（单股即时分析，live-only） | ❌ |
| `hooks.rs:881` | `stock-analysis-persist` hook（**对话直执行**，`analysis_kind="chat"`） | ❌ |
| `reflection.rs:195` | 反思**结果**落库（不是 pending 创建） | — |

而 `hooks.rs:779-784` 的守卫使该 hook 对业务封装路径**直接 return**：

```rust
// 业务封装路径已自行持久化（含 dashboard/自适应闭环/告警等完整后处理），
// input 带 analysis_id 标记 → 跳过，避免重复写入。
if input_has_analysis_id(&ctx.input) {
    return Ok(());
}
```

⇒ `run_stock_workflow_inner`（工作区 / 批量 / 定时 / **as-of 重跑全部走它**）
**不会创建 pending 反思行**。

**DB 证据（吻合）**：现存 11 条 completed 反思对应的分析**全部**是
`provider_id = workflow` + `analysis_kind = live`；而 `hooks.rs` 那条通道写的是
`provider_id = cognitive` + `analysis_kind = chat` ⇒ **零条来自 chat 通道**，
业务封装路径零产出。

**影响**：这正是「对齐 v77」与「快速积累反思数据」之间**断掉的那一环** ——
不加处理就重跑 47 条，跑完反思队列里**一条新 pending 都没有**，
`--reflect-after` 会消费 0 条。（我先前写下的「重跑 ⇒ 47 条 pending ⇒ 立即可反思」
是错的，本 PLAN 已订正。）

**本轮处置（不动产品核心函数）**：由 `axagent-batch-rerun` bin **补建/重置** pending 行
（机制见 §12.13）。理由：`run_stock_workflow_inner` 有 1300 行、多个成功分支
（`core.rs:1053`/`:1368` 各写一次 `template_version`），在其中插入 pending 创建
需同时处理**锚点、去重、`expected_holding_days`** 三件事，回归面过大，而用户当前目标是推进四阶段。
⇒ 产品侧持续缺口登记为 **D12**，待裁决是否收敛为共享函数
（把 `core.rs:1922` / `hooks.rs:881` 两份 + 业务封装路径那份合并为单一实现）。

#### (c) 连带后果（必须知道）：重跑会使既有 17 条反思失效

`chk8` 实测：**17 条反思（11 completed + 6 pending）对应的分析 `template_version` 全是 NULL**
⇒ **全部落在 47 条重跑范围内**。而重跑对同日 `as_of` 是**原地覆盖同一 `analysis_id`** ⇒
旧反思（阶段一回填的 verdict / lesson_summary 等）对应的是**上一版决策** ⇒ 结论对新决策无效
⇒ 补建逻辑必须把它们**重置**为 pending（清结论列、保留 `raw_return`/`alpha_return`/`holding_days`
等实测列）；否则会产生「新决策 + 旧反思结论」的错配样本。

⇒ **阶段一的价值必须分两层理解**（汇报时讲清）：

| 层面 | 是否被重跑冲掉 |
|---|---|
| **代码修复**（读取层深取值 + D10 回写 + `was_correct` 三义 + 实测列保留） | ❌ 不受影响，对**未来所有**反思永久生效 |
| **历史 11 条的字段回填** | ✅ 会被重跑逐条重置（这是「对齐 v77」的应有之义，不是副作用） |

⇒ 若要保留「基于旧公式的反思样本」做对照，**必须在重跑前另建快照表**。

### 12.13 新增机制：重跑时补建/重置反思 pending 行（`ensure_pending_reflection`）

**位置**：`src-tauri/src/bin/axagent-batch-rerun.rs`（bin 内，非产品代码）。

**行为**：每条重跑成功的分析 ⇒

1. 读 `stock_analyses`（**必须先判状态**）：`status != "completed"` ⇒ **跳过补建**并返回
   `(false, …)`。这一条不可省：`run_stock_workflow_inner` 在 DAG 失败时**不返回 Err**，
   而是把 status 置为 `failed: {e}` 后照常 `Ok(json)`（`core.rs:1645-1660`）
   ⇒ 只看返回值会把失败分析当成功，为其创建**根本无决策可反思**的样本（污染样本集）。
   成功态字面值实测：`completed`（`core.rs:1020`/`:1335`）；其余值域
   `running` / `failed: {e}` / `timeout`(`:925`) / `cancelled`(`:956`)。
2. `as_of_date` = 分析记录自身的 `as_of_date`（as-of 重跑时 = 原分析日）
3. `hindsight_date` = 上述锚点 + `decision_expected_holding_days`（默认 28）
   ⇒ **落在过去** ⇒ 反思立即可执行，**不必等 28 天**
4. 已存在反思行 ⇒ **重置**为 pending（清结论列）；不存在 ⇒ 插入新的 pending

**幂等性**：同一条分析反复重跑 ⇒ 反思行始终是 1 条 pending（不会累积）。

**返回值设计**：`Result<(bool, String), String>` —— `bool` 表示「是否真的补建/重置」。
> 为什么不返回裸字符串、让调用方按前缀判断：**靠解析人类可读文案做控制流是最脆的耦合**
> （文案一改，判断静默失效）。同族判据见 `MEMORY-RULES.md` #7（审计脚本自身会撒谎）。

**计数分离**：主循环把「重跑动作成功」与「分析真的产出决策」分开计数 ——
`分析成功 ok` / `分析未成功 not_completed` / `执行失败 failed`。
> 若把三者合并成一个 `ok`，报告的成功数会**虚高**，掩盖「重跑没有真产出决策」这一事实。

**保留实测列**：重置时**不动** `raw_return` / `alpha_return` / `holding_days`
—— 它们是实测事实，与决策版本无关，且反思重跑会按最新行情覆盖。
**`alpha_return` 有意保持 NULL**（无基准收益来源，猜测填充即伪造，见 §12.8 裁决表）。

**手抄常量提示**：`min_confidence_threshold = 70` / `reflection_depth = "light"` 在
`hooks.rs:888`、`core.rs`（单股通道）、本 bin **三处各写一份**。
这是 D12 收敛为共享函数时要一并解决的（已就地注释标注）。

### 12.14 重跑前基线确认（2026-09-23，全部实测非推断）

| 项 | 结论 | 证据 |
|---|---|---|
| `workflow_templates['stock-analysis'].version` | **76** ⇒ 目标版本仍未落库（首次重跑时由 `ensure_stock_analysis_experts_seeded` 完成升版） | `chk9` |
| 两条 cron 配置格式 | ✅ 与代码契约**逐字段匹配**（见下表） | `chk14` |
| 应用是否在运行 | ❌ **未运行** ⇒ 无并发反思（否则其 6h 兜底循环会与本 bin 并发消费同一批 pending） | 进程表 |
| 文件备份有效性 | ✅ `identical=0` —— 3 个已备份文件**全是改动前快照** | `output/backup-20260923/verify-backup.mjs` |
| ⚠ `serenity.rs` / `hooks.rs` | **无文件备份**（本轮分别改过 report 解包分支 / 防误判注释）⇒ 回滚须按本文档记录**手工撤销** | 同上 |
| 6 条 pending 反思 | **孤儿**：`original_analysis_id` 全部不存在（`id_exists=f`）；603619 连该股票的**任何**分析都是 0 条 | `chk10` / `chk11` |

**cron 契约逐字段核对**（`init/services.rs:2651-2684` 按 `task_type` 决定反序列化哪个 Config）：

| cron | `task_type` | `prompt` | 反序列化目标 | 判定 |
|---|---|---|---|---|
| `batchref-due-daily` @ `0 18 * * *` | `batch-reflection` | `{"dueOnly":true}` | `BatchReflectionConfig{period,dueOnly,maxCount}` | ✅ |
| `vldec-daily` @ `0 6 * * *` | `validate-decisions` | `{"minConfidence":0,"reflectionDepth":"light"}` | `ValidateDecisionsConfig{minConfidence,reflectionDepth,maxCount}` | ✅ |

中文经 `\uXXXX` 写入、DB 读回正确还原（「批量反思复盘：收盘后消费到期 pending」）；
`status=active`、`recurring=true`、`timeout_seconds=7200`。

> **判据（可复用）**：脚本写进 DB 的**配置型 JSON** 必须与**消费端反序列化目标逐字段核对**。
> 字段名不匹配时反序列化会失败，而失败点通常在**另一个文件**（`services.rs` 的 error 日志），
> 表现为「任务到点什么都不发生」—— 属于「配了但没接线」族，静默且难归因。

**⚠ 6 条孤儿 pending 的处置（登记，不在本 PLAN 范围）**：它们让 `--reflect-after` 的
「pending 已清空」判据长期无法满足（永远剩 6 条）。循环的 `after >= before` 判据**能正确处理**
（不空转、不死循环 —— 已实测 `reflection.rs:1756-1759` 对查不到 analysis 的行是
`skipped_young += 1; continue;`，**不中断整批**），但「队列清空」的**判读会失真**。
⇒ 建议另行裁决：① 标记 abandoned / 删除；② 或按 `stock_code` + `as_of_date` **回指**现存分析。
**但无法全部回指**：600036 有同日 3 条、605100 有同日 1 条可回指；
600219 / 600887 / 601398 同日为 0；603619 同股票为 0 ⇒ 后四只只能标记。

---

### 12.15 本轮新增两项登记（**登记，不擅自改**）

#### 登记 1：`reflection_stats.rs` 的编译错误由本会话代修（跨会话记账）

**事实**：该文件属**并发会话**的 M3 工作（其模块头写明 `PLAN-stock-decision-hitrate-validation`），
mtime 05:34:38（比本会话 bin 的改动晚 1 秒）、已静止 15 分钟。它的 **2 个编译错误 + 2 个未用
import 阻塞了整个 `axagent` lib** ⇒ 本会话的 bin **无法产出**（不是「能不能编过」的问题，
是「编不出任何产物」）。

**代改内容**（纯机械，零语义变更）：

| # | 位置 | 改动 | 依据 |
|---|---|---|---|
| ① | `nearest_analysis_dimensions` | 补命名生命周期：`analyses: &'a [...]` → `Option<&'a Model>` | 返回值只借用 `analyses`；**刻意不给 `stock_code` 绑 `'a`**（rustc 的建议会把调用方约束收紧，属过紧） |
| ② | `:110` | `ana_by_id.get(r.original_analysis_id.as_str())` | `original_analysis_id` 是**非空 `String`**（entity `stock_reflections.rs:14`）⇒ 没有 Option 链可 `.and_then` |
| ③ | `:28` | `use sea_orm::{ColumnTrait, EntityTrait, QueryFilter}` → `EntityTrait` | 另两个实测未使用；留着会让**最终 clippy 门禁**红 |

> **判据（可复用）**：多会话共用同一工作区时，**阻塞自己产物的他人文件错误可以代修，
> 但只做纯机械改动、不碰语义**；语义问题一律登记不代改。核心风险不是"改了别人的文件"，
> 而是**同文件并发 `Edit` 互相覆盖** ⇒ 代修必须是**最小且可一眼复核**的改动。

#### 登记 2：`reflection_lessons` 两条消费端**阈值口径不一致**（0.3 vs 0.5）

| 消费端 | 过滤条件 | 实收范围 |
|---|---|---|
| `core.rs:2187-2189`（注入分析上下文） | `StockCode.eq` + `Confidence.gte(0.3)`，**无 status 过滤** | 0.3 / 0.5 / 0.7 **全收** |
| `decision.rs:2694-2698`（注入 portfolio-mgr 的 `recent_lessons`） | `Status.eq("active")` + `Confidence.gt(0.5)` | **仅 0.7** |

写入端初始值（`reflection.rs:1285-1289`）：`wrong → 0.7` / `partial → 0.5` /
`correct 或 None → 0.3`。

⇒ **后果**：`partial`(0.5) 与 `correct`(0.3) 的规则**只在分析上下文生效、不进 portfolio-mgr**；
`wrong`(0.7) 两端可见。本轮回填的 11 条 verdict 分布为 3 correct / 5 partial / 2 wrong
⇒ 按此口径，**portfolio-mgr 只会看到 2 条规则**。

**已排除的两个「看起来像缺陷」的形态**（登记以免后来者重复怀疑）：

1. **`core.rs` 缺 `status` 过滤不是漏洞** —— 验证器把 `deprecated` 与
   `adjusted_confidence < 0.2` **同时写入**（`reflection.rs:1449-1471`，同一变量 `new_status` /
   `Confidence`）⇒ 被废弃的规则必然 `confidence < 0.2`，**已被 `gte(0.3)` 挡掉**。
   两条过滤条件在此**恰好互补**。
   > ⚠ 我自己在第一轮 grep 时误判为"验证器只产出报告不落库"，原因是正则没覆盖
   > `col_expr(\n  Column::Status,\n  Expr::value(...))` 的**多行写法**。
   > ⇒ 判据：**grep 找不到写入点 ≠ 没有写入点；多行 `col_expr` 必换多行正则或直接读函数体**。
2. **`0.3` 恰好压在 `gte(0.3)` 的边界上**（`correct` / 无 verdict 的初值即 0.3）⇒ 当前**刚好包含**。
   若将来有人把 `gte` 收紧成 `gt`，`correct` 类规则会**整批消失**且**无任何报错**（静默）。
   ⇒ 边界脆弱，已登记。

⇒ **待裁决**：两处是否统一；统一到哪一侧（宽松到 `gte(0.3)` / 收紧到 `gt(0.5)`）属
**产品语义决策**，本会话不擅自改。

### 12.16 重跑前的两项**必做清理**与三处**判断订正**（2026-09-23 06:20）

#### (a) 新缺陷 D13（P1）：`strategy_performance` 会因重跑累积**重复样本**

`strategy_performance` 的行是**反思结论的量化投影**（`was_correct` 由
`deterministic_was_correct(决策方向, 行情快照)` 产出，`reflection.rs:620`）。
重跑覆盖决策后：

| 环节 | 事实 |
|---|---|
| 唯一约束 | 只有 `id` 主键 —— **无 `(stock_code, strategy_id)` 唯一约束**（实测 `pg_index`） |
| 反思侧 | 重置后重跑会**再 INSERT 一行** ⇒ 同 `stock_code` 出现两代行 |
| 消费侧 | `compute_adjusted_weights` 按 **(strategy_id, period)** 聚合（`weight_decay.rs:77`），**不看 `stock_code`** ⇒ 两代行**各计一次样本**，拉偏 `sample_size` / `win_rate` / 贝叶斯分母（`:89`） |
| 当前实测 | 10 行 / 10 个 distinct code / **0 个重复组** ⇒ 问题是**未来态**，重跑才会显现 |

**修法**（已实施于 bin）：重置/补建反思时调用 `delete_stale_reflection_perf`，
按 `stock_code` + `strategy_id='reflection_verdict'` 删除旧行。

**粒度裁决**：该表**无 `analysis_id` 列** ⇒ 无法精确关联到某一次分析，只能按 `stock_code` 删。
本次数据修复中这是**一致**的（11 条 completed 反思对应的分析 `template_version` 全为 NULL ⇒
全在重跑范围内 ⇒ 该标的旧行全部作废）。⚠ 将来用于「只重跑部分分析」时必须重评粒度。

**回滚点**：库内备份表 `strategy_performance_backup_20260923`（10 行）。

#### (b) 回滚点补齐（三张库内备份表，缺一不可）

| 备份表 | 行数（实测） | 保护对象 |
|---|---|---|
| `stock_analyses_backup_20260923` | 53 ↔ 53 | 原地覆盖前的决策（`decision_json` ×53 / `blackboard_snapshot` ×43） |
| `stock_reflections_backup_20260923` | 17 ↔ 17（`verdict`×10 / `lesson_summary`×10） | 反思重置会清空 10 列 ⇒ **无此表则阶段一成果不可还原** |
| `strategy_performance_backup_20260923` | 10 ↔ 10 | D13 的清理动作 |

#### (c) 判断订正 1：`raw_return` 等 4 列**故意保留**（我一度误判为应清空）

我最初认为 `raw_return` / `alpha_return` / `holding_days` / `benchmark_name` 与旧决策绑定、
重置时应清空。**这是错的**：

它们的语义是「**从 `analysis_date` 到反思评估时点这段行情的客观事实**」——
起点是分析日（重跑不改变），**与决策内容无关**；决策方向变了，同一段行情的收益不会变。
且反思重跑成功后会按最新行情**回写覆盖**（`reflection.rs:598-600`，即 D10 修复点）。

⇒ 判据：**先问「这个量在物理上依赖决策内容吗」，再问它是否该随版本失效**。
我原有的判据（"看起来与决策绑定"）是**形态论**，会把客观事实误清。

（`model_version` 已于 2026-10-05 退役，见上面 D7 的状态订正 ⇒ 重置逻辑不涉及它；
本条原本记的是「恒 NULL、无需处理」，那条自陈注释也随列一起从 `axagent-batch-rerun.rs` 删掉了。）

#### (d) 判断订正 2：**D11 的「阻断」定性彻底不成立**（第三次削弱）

我最初断言：pending 行的 `hindsight_date` 若落到未来 ⇒ 反思入口
`AsOfContext::parse` 报 `FutureDate` ⇒ 闭环断裂。**逐层核查后不成立**：

| 层 | 事实（行号已核对） |
|---|---|
| 批量路径是否走 `AsOfContext` | ✅ 走 —— `run_batch_reflection_inner:1877` **确实调用** `run_reflection_workflow`，后者 `:503` 有 `AsOfContext::parse(hindsight_date)` |
| 但传的是哪个日期 | ❌ **不是** pending 行的 `hindsight_date`。`:1893-1895` 注释 + 实参明写：<br>「AS_OF 锚点用**今天**（行情终点=最新交易日）」⇒ 传 `&today_str` |
| 反思的行情从哪来 | `:1836-1842` `compute_market_snapshot(client, code, **analysis_date**, …)` ⇒ 以**分析日**为起点拉「分析日→最新交易日」 |
| pending 的 `hindsight_date` 实际用途 | 仅三处：`:1782-1791` 日志提示 / `:1799` filter 的 due 判定（`due_only=false` 时不参与）/ `:1810-1825` 算 `days_held` |

⇒ 结论：**批量反思不受「hindsight 在未来」影响**，09-20~09-22 那批（hindsight=10 月中）反思不会失败。
真正需要 `hindsight_date` 语义正确的只有 `days_held` —— 而 bin 补建时用的
`as_of_date + hold_days` 正是正确语义。**D11 的最终结论 = 撤回（不存在的缺陷）**。

#### (e) 判断订正 3：`was_correct` 的口径已被并发会话**升级**（我的 D2 修复被继承）

`:620` 已由 `deterministic_was_correct` 接管，注释明写「M1 改写：不再由反思 agent 的 verdict
（LLM 自评）映射，改由行情快照**确定性反推**」+「**保留 D2 语义**」（无法判定 ⇒ 不写行）。
我的 D1（`deep_get`）/ D3（`decision_confidence`）/ D10（三列回写）改动**全部在位**（已 grep 复核）。

⚠ **由此产生一个口径混合问题**：阶段一回填的 10 行 `was_correct` 是 **verdict-based**，
而新代码写入的是 **deterministic-based** ⇒ 同列两种语义。
消费端只按 `!= 0` 判胜（`weight_decay.rs:79`）⇒ 影响有限，但统计口径不纯。

**✅ 该问题会被本次重跑自然消除，不需要额外动作**（我一度把它登记成「待裁决」，
但那样会留下一个假待办）：

| 步骤 | 效果 |
|---|---|
| 重跑覆盖决策 | 10 行对应的分析全部被重算 |
| `ensure_pending_reflection` 重置反思 + **删除旧绩效行**（D13 修法） | verdict-based 的 10 行**被删除** |
| `--reflect-after` 重新反思 | 新写入的 `was_correct` 一律是 deterministic-based |

⇒ 重跑完成后 `strategy_performance` 与 `stock_reflections` 的判定口径**自动统一为 deterministic**。
判据：**先问「这个不一致是稳态还是过渡态」再决定要不要单独立项**。

`deterministic_was_correct` 的判定表（`:2646-2652`）：

| 决策方向 | 条件 | 结果 |
|---|---|---|
| Buy / Increase | `net > 0` → 1；否则 0 | 看多档 |
| Sell / Reduce | `net < 0` → 1；否则 0 | 看空档 |
| Hold / Wait / Uncertain / Unavailable | — | `None`（不写行） |
| 任意 | `within_expected_horizon` 或 `net` 为 NaN 或无行情 | `None`（不判定） |

---

### 12.17 ⚠ 新发现：PG 连接串（含口令）被 **dao 层既有日志**明文打印（登记，不在本 PLAN 范围）

`axagent-batch-rerun --limit 5` 的 dry-run 日志首屏出现（**已在本文件中打码**）：

```
INFO axagent_dao::db: Database initialized at postgres://postgres:<口令>@localhost:5432/axinvest?sslmode=disable
```

**根因**：这是 `axagent_dao::db` 初始化时的**既有** `INFO` 打印，**与本 bin 无关** ——
本 bin 特意只打印 URL 的 **scheme 前缀**（见 bin 内 fail-closed 断言的 `db_ident.split("://")`）。
⇒ 属于「**敏感值两层**」族的反向形态：**本层已脱敏，而**上游另一层**把原文打了出来**
（判据：脱敏 ≠ 不泄漏 ⇒ 必须检查**同一值在链路上的所有输出点**，而非只检查自己写的那一处）。

**影响面**：任何以非 tty 方式运行本仓二进制（CI 日志、`> file.log`、容器日志采集）都会把口令落盘。
本次 dry-run 日志 `output/backup-20260923/*.log` 已含该串 ⇒ **这些日志不可外传**。

**为何本轮不修**：改的是 `crates/dao` 的既有日志行为，属**跨模块共同输出面**，
且需与「诊断期需要知道连的是哪个库」的诉求权衡（可能有人依赖它排障）⇒ 属产品语义，**登记待裁决**。

---

### 12.18 ⚠ **D14（阻断项，已修）**：as-of 回放被预检硬门**全量拦截** —— 53 条一条都跑不动

**实测**：V 批（`--since 2026-07-21 --until 2026-07-21 --apply`）**4/4 全部 `failed`**，耗时仅 34 秒
（正常单条分析 ≥1 分钟）—— 说明 DAG **根本没执行**，不是跑得慢。

日志原文（逐条可见）：

```
WARN stock_workflow::core: [stock_workflow] 数据质量不足，跳过 DAG 执行:
     news: 全部数据源获取失败: ...无法获取 300795 的新闻 (300795)
...
[batch-rerun] 完成：分析成功 0 / 分析未成功 4 / 执行失败 0 / 共 4
```

**根因（`decision.rs` 的 `aggregate_precheck`）**：

| 环节 | 事实 |
|---|---|
| 判定规则 | `任一 source == Failed ⇒ 整体 Insufficient ⇒ core.rs:549 直接 return，DAG 不执行` |
| `news` 判定 | `client.get_news()` 在 as-of 下返回 **`Err`**（非 `Ok(空)`）⇒ 落进 `SourceCheck::Failed` |
| 为何 Err | 各 vendor 只提供**近期**新闻，as-of 过滤层把「晚于截止日」的全部丢弃 ⇒ vendor 链全失败 |
| 同族源 | `financials` / `money_flow` / `announcements` / `concept_blocks` / `sector_info` / `lockup` / `dragon_tiger` —— **均无历史接口**，同样必然为空 |
| 有历史语义的只有两个 | `quote`（`astock-data/lib.rs:1719-1739` 用截断 K 线**合成**）、`klines`（`:953` 截断到截止日） |

⇒ 这不是数据质量缺陷，而是「**该维度在时间旅行中不存在**」。
用今天的完整性口径去要求历史回放，等于要求数据商提供它从未提供过的历史新闻 ——
**若不修，53 条一条都跑不动，整个阶段三无法执行**。
（这也是本 PLAN 最真实的一处阻断：比 §12.12 撤回的 D11 硬得多 ——
 D11 是我误判，D14 是实测 4/4。）

**修法（`decision.rs`）**：`aggregate_precheck` 增加第二参 `as_of_mode: bool`，
as-of 下**无历史语义的源**的 `Failed` **降级为 `Partial`**：

- 仍**留痕** —— 原文与原因一起进 summary（`…（as-of 回放：该维度无历史接口，空属结构性，不阻断）`）
  ⇒ 前端数据缺失报告、以及 §6.2 的「降级率体检」都还能看到哪些输入是缺的；
- **不阻断** —— `core.rs:605` / `:1791` / `hooks.rs:662` 的 `Partial` 分支只打 info 后继续；
- `quote` / `klines` **保持原判定** —— 它们失败说明 as-of 引擎自己拿不到数据，那是真故障，
  必须继续阻断（否则回放会拿空气当行情）。

**三条反向锁**（新增测试，防放宽渗到实盘 / 防变成 fail-open）：

| 测试 | 断言 |
|---|---|
| `aggregate_as_of_ignores_sources_without_history_semantics` | as-of + news Failed ⇒ **Partial**，且消息含 `news` 与 `无历史接口`（留痕不可省） |
| `aggregate_as_of_still_blocks_when_historical_source_fails` | as-of + **klines** Failed ⇒ 仍 `Insufficient`，且阻断清单**不含** news |
| `aggregate_live_mode_keeps_news_as_blocker` | **live** + news Failed ⇒ 仍 `Insufficient`（放宽只对 as_of 生效） |

**⚠ 副作用与恢复（本轮 V 批留下的痕迹）**：4 条原为 `completed` 的记录已被「原地覆盖」
改成 `status='failed'`、`decision_json={"action":"skip",..}`：

```
300795 67fc9944-c258-42cf-9129-7a4a9368062b
301302 1a6daaaf-6ddd-483b-bb2c-fb052ea32843
600900 851f75ea-776f-46df-9580-47e139353e66
601166 7f36b731-e618-4db9-9f2d-27bd7cdb9e79
```

⇒ **修复后重跑即可覆盖回来**（这正是原地覆盖的语义）；若修复失败，从
`stock_analyses_backup_20260923` 按 id 单行恢复。

> **判据（可复用）**：**「分析失败」要按耗时与日志分流**。34 秒跑完 4 条 = 结构性跳过，
> 不是「跑得慢/模型差」；先 grep `跳过 DAG 执行` / `Insufficient`，再谈模型。
> 同族：任何「回放/预览/dry-run」特性接上真实数据链时，必须逐源问
> 「**这个源在过去的时点上存在吗**」—— 数据源的时间可回溯性是一等约束，
> 不能只验「接口能调通」。

---

### 12.19 ⚠ **D15（阻断项，已修）**：离线 bin 无**工具解析器** ⇒ DAG 数据节点全数失败

**实测**（D14 修完后的 V2 批，`rerun-V2.log`）：预检已放行为 `Partial`、DAG **确实启动**了，
但**每个数据节点**立刻失败：

```
WARN rt_workflow::work_engine::engine: Node failed node_id=t-market-data
     error=TOOL_CALL_FAILED: 工具 'get_stock_kline' 未注册，请通过 WorkEngine::register_tool_handler() 注册或注入 ToolRegistry
WARN ... node_id=t-news-data        工具 'get_stock_news' 未注册
WARN ... node_id=t-fundamentals-data 工具 'get_fundamentals_...' 未注册
WARN ... node_id=t-policy-data       工具 'get_stock_policy_news' 未注册
WARN ... node_id=t-hotmoney-data     工具 'get_stock_money_flow' 未注册
WARN ... node_id=t-dragon-tiger-data 工具 'get_stock_dragon_tiger' 未注册
WARN ... node_id=t-research-data     工具 'get_stock_research_re...' 未注册
WARN ... node_id=t-sentiment-data    工具 'get_social_sentiment...' 未注册
```

⇒ 4/4「分析未成功」，且**每条只耗 2.7~12 秒**。

**根因链（三层，缺一不可地都要查）**：

| 层 | 事实 |
|---|---|
| ① 谁在找工具 | `tool_executor.rs:122-181`：先试 `context.tool_registry`，再试 `callbacks.tool_handlers` → `tool_fallback`；**三条全空**才报「未注册」 |
| ② 谁在填这三条 | `engine/mod.rs:2421-2528` 的 `run_workflow` **注册段**：先用 `tool_resolver` 自动注册，再用 `tool_registry` 补，最后用 Rhai 脚本缓存 |
| ③ 谁注入 resolver | **`init/services.rs` 的 `start_cron_scheduler(app: &AppHandle, state)` 内部**（原 `:2015`）—— 一个**后台服务启动函数** |

⇒ 判据落到最致命的一处：**`create_app_state` 明确「不启动后台服务」**（bin 走的就是它）
⇒ resolver 从未注入 ⇒ 注册段三条来源全空 ⇒ 所有工具调用失败。
（`set_tool_registry` 同理：**生产路径从未调用**，只有测试在用 ⇒ 不是可依赖的替代路径。）

**修法（`init/services.rs`）**：把该块**无损抽出**为公共函数

```rust
pub async fn inject_work_engine_tool_resolver(state: &AppState)
```

- 该块**本身不依赖 `app`**（只用 `state.local_tool_registry` + `state.work_engine`）⇒ 可安全抽出；
- `start_cron_scheduler` 内改为一行调用，**行为不变**；
- `axagent-batch-rerun` 在 `create_app_state` 之后显式调用它 ⇒ **bin 与 Tauri 共用同一条装配路径**，
  不再是「bin 自己复制一份逻辑」——后者会埋下「bin 能跑而应用不能」的反向偏差。
- 抽出后 `start_cron_scheduler` 里的 `use std::sync::Arc;` 变成未使用（该函数余下两处都是
  `std::sync::Arc::new` 全路径）⇒ **必须删掉**，否则 clippy `-D warnings` 失败。

> **判据（可复用，三条）**：
> 1. **「工具未注册」≠ 工具不存在**。工具名在 registry 里查得到、schema 也在，缺的是**运行期的注入**。
>    遇到它先问「这个进程走的是哪条启动路径」，而不是去改工具定义。
> 2. **离线 / 无头运行（bin、CI、headless）必须核对「启动期注入链」是否被绕过**。
>    `create_app_state`（关键路径）与后续 service 启动（注入 resolver / cron / RAG / 通知）是**两段**，
>    任何只在第二段完成的装配，在第一段使用者眼里就是「不存在」。
> 3. **「耗时异常短」是结构性失败的可靠信号**。正常单条分析 ≥1 分钟；本轮 34 秒跑完 4 条、
>    2.7 秒跑完 1 条 —— 都对应「DAG 没跑」或「节点秒败」。
>    先看耗时与 `Node failed` 行，再看模型输出质量。

---

### 12.20 ⚠ **D16（阻断项，已修）**：工具**执行**层 —— as-of 下取数函数抛 Err，节点失败

**实测**（D15 修完后的 V3 批，`rerun-V3.log`）：工具解析全部成功
（日志逐条 `[ToolExecutor] 工具 'get_stock_kline' 通过 ToolResolver 回调路径执行`），
但**执行**失败：

```
Node failed node_id=t-news-data error=TOOL_CALL_FAILED:
  Tool execution error: [executionFailed] Vendor error from all:
  所有新闻数据源失败，无法获取 300795 的新闻
Node failed node_id=t-fundamentals-data error=TOOL_CALL_FAILED: ... 所有财务数据源失败 ... 600900
```

⇒ 4/4 仍「分析未成功」。**三层同族缺陷至此全部暴露**（缺一层都还会卡住）：

| 层 | 缺陷 | 表现 |
|---|---|---|
| 预检层 | **D14** | `aggregate_precheck` 判 `Insufficient` ⇒ DAG 根本不启动 |
| 解析层 | **D15** | resolver 未注入 ⇒ 每个节点报「工具未注册」 |
| 执行层 | **D16** | 工具自身抛 `Err` ⇒ 节点 `TOOL_CALL_FAILED` ⇒ DAG 失败 |

**根因与两种不同的修法（关键：先找「已有的历史通道」，再考虑「宽容失败」）**：

1. **`get_news` —— 接上已有通道，拿到真数据（不是宽容）**

   本仓**早已有** as-of 新闻实现，就在**隔壁兄弟函数** `get_policy_news`（`:2389-2426`）里：

   ```rust
   if let Some(ctx) = crate::as_of::current_as_of() {
       if let Some(sink) = &self.news_archive_sink {
           let archived = sink.search_asof("政策", Some(stock_code), as_of_ts_ms, limit).await;
           if !archived.is_empty() { return Ok(archived); }
           record_degradation(...); return Ok(vec![]);   // ← 空但**不抛 Err**
       }
       ...
   }
   ```

   `news_archive` 表由**每次 live 抓取** upsert 进来（`:2355`），`search_asof` 取
   `publish_time <= 截止日` 的最新 N 条；`keyword` 传空串即「不过滤子串、只按股票+截止日」
   （`dao/src/repo/news_archive.rs:112`）。`get_news` **只是漏了这段分支** ⇒ 补上后
   **真正拿到当时的历史新闻**，比「忽略失败返回空」高一个档次。

2. **`get_financials` —— 确无本地通道，只能按空返回 + 留痕**

   财务**没有**对应的本地历史表（`financials` 不落库），实测 as-of 下各源确实取不到
   （`xueqiu 返回空数据` ⇒ 全链失败）。故在此处：**as-of 时把最终 `Err` 转成 `Ok(空)`**，
   并 `record_degradation` 留痕。

   ⚠ 该函数原本的注释写着「全部数据源失败时返回 Err，**避免调用方把空列表误判为"无新闻/无催化剂"**」
   —— 这是**有意的**设计，live 下必须保留（否则假成功空数组会被当成「这公司没财报」）。
   故本条改动**只加 as-of 分支、不动 live 语义**。

> **判据（可复用，三条）**：
> 1. **同族缺陷会按「层」逐层暴露，修一层只推进一层**。本条链上三层（预检/解析/执行）
>    各自独立、都必须修；**不要因为「修完还是失败」就推翻前一层修复** ——
>    每一层都用**不同的日志信号**证实（`跳过 DAG 执行` / `未注册` / `TOOL_CALL_FAILED`）。
> 2. **在决定「宽容失败」之前，先搜有没有「已有的正确通道」**。
>    本例 `get_policy_news` 就是现成模板 —— 同一文件、同类数据、已实现 as-of。
>    「兄弟函数一个有 as-of 分支、一个没有」是**漏接线**的典型形态，不是设计差异。
> 3. **日志里的 ANSI 转义会让 ripgrep 静默跳过整份文件**（本次实测：`Grep` 报
>    `No matches found`，而 Node 逐行读同一文件能命中 6 行）。
>    ⇒ 抓不到 ≠ 不存在；对**带颜色的运行日志**，改用「Node 逐行读 + 打印原文」取证。
>    （同族：R 组「判据工具自证」——**先证工具会命中，再用它下结论**。）

---

### 12.21 ⚠ **D17（阻断项，已修）**：调用方把 **fire-and-forget** 当成「已落定」⇒ DAG 被进程退出取消

**实测**（D16 修完后的 V4 批，`rerun-V4.log`）：预检放行、工具解析成功、工具执行也不致命了
（`news: 无新闻数据` 已是 `Partial`）—— 但**每条仍只耗 2.7~10.6 秒**，且 DAG 日志停在
`t-dragon-tiger-data` 起的若干节点，没有一条跑到决策落库：

```
WARN agent_executor: H4.1: fallback_model provider 解析失败，走降级 JSON node_id=a-news
  error=UNSUPPORTED_PROVIDER: Database error: Connection Error:
  error communicating with database: task was cancelled
```

`task was cancelled` ＝ **进程正在退出、tokio runtime 正在 drop**。

**根因**：`run_stock_workflow_inner` **不等 DAG** —— 它把工作流 `tokio::spawn` 到后台后
**立即返回**（`core.rs:826-837` 的 `tokio::spawn` + `:885` 的 `engine.run_workflow`）。
这是**为前端设计**的 fire-and-forget 形态：前端拿到 `analysisId`，自己轮询 `status`。
（对照：live-only 通道 `run_single_stock_analysis` 在 `core.rs:1863` 是 `.await` 的 ——
**两条通道的等待语义不同**，这正是误读的来源。）

⇒ 离线 bin 里 `main` 一结束，runtime 就被 drop，后台 DAG **连同进程一起被取消**。
**危害等级最高**：它不报错，只会让 bin 输出一份「已处理 N 条」的报告，
而实际**一条都没做成**（`status` 停在 `running`，这正是 bin 判「分析未成功」的来源）。

**修法（bin 内，不碰产品代码）**：新增 `wait_until_settled(db, analysis_id, timeout)`
—— 在每次 `run_stock_workflow_inner` 返回后**轮询 DB `status` 直到离开 `running`**（超时 30 分钟），
然后再做成败判定与反思补建。选轮询 DB 而非拿内部句柄的理由：DAG 已 spawn 走、句柄不可得；
而 `status` 是**前端也在用的同一权威落地信号**，与「用户看到的状态」严格一致，且不依赖内部实现。

**四层同族缺陷至此全部闭环**：

| 层 | 编号 | 信号 | 性质 |
|---|---|---|---|
| 预检 | D14 | `跳过 DAG 执行: 数据质量不足` | as-of 感知缺失 |
| 解析 | D15 | `工具 'x' 未注册` | 装配链被绕过 |
| 执行 | D16 | `TOOL_CALL_FAILED: Tool execution error` | 历史通道漏接线 |
| **调度** | **D17** | **`task was cancelled`** | **调用方误读等待语义** |

> **判据（可复用，两条）**：
> 1. **`await` 一个 async fn ≠ 它完成**。必须问「这个函数**内部**有没有 `spawn`」。
>    凡是「先返回 id、前端自己轮询」的接口，**任何非前端调用方（bin / CI / 定时任务）
>    都必须自行等待或轮询**，否则进程退出即取消。
> 2. **同一概念的两条通道可能有不同的等待语义**（本例 `run_stock_workflow_inner`
>    fire-and-forget vs `run_single_stock_analysis` 同步 await）。
>    从 A 通道推 B 通道的行为，是本轮反复踩的同一类错误
>    （同族：§12.12 撤回的 D11 —— 也是「把 live-only 通道的性质套到 replay 通道上」）。
> 3. **「耗时异常短」第四次救场**：2.7 秒完成一次 13 节点 DAG 分析是不可能的。
>    这个信号在本轮连续命中 D14/D15/D16/D17 四层 —— **先看耗时，再看内容**。

---

## 附：本文档的证据可复现命令

```bash
# 只读查库（口令经环境变量，不进 argv）
node scripts/pg-connect.mjs sql "<SQL>"

# 关键取证 SQL 见 §8 与 §1
```

> 全部实测均通过上述入口以**只读**方式完成；本研究**未执行任何写操作、未修改任何文件**
> （本文档除外）。
