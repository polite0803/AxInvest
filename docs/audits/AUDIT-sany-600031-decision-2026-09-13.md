# 三一重工 600031 决策复核 + Switch 默认分支断链修复

> 状态：active ｜ 日期：2026-09-13 ｜ 对应版本：v2.9.7


> 日期：2026-09-13
> 样本：`stock_analyses.id = d9fbe526-5e7e-46fe-ac4b-8c1d584ab02d`（600031 三一重工，2026-09-13 08:06:19）
> 触发：用户重跑三一重工后，界面显示「增持 11.5% / 中风险」，而库里落的是「减持 5% / 高风险」
> 关联：`AUDIT-xingye-601166-decision-2026-09-13.md`（上一轮 601166 的 7 项修复）

---

## 0. 一句话结论

界面那张卡片是对的（公式链产出），**库里那条是错的**。根因不在业务逻辑，而在工作流引擎：
**`SwitchNode` 的「默认分支」从来不会被拦，任何 Switch 的所有分支都会被执行**。
于是 `quality-gate` 判「A/B/C 可接受」时，本该只走 `decision-explainer`，实际连 `quality-fallback`
（LLM 保守兜底）也一起跑了，而落库的「链尾优先」规则正好优先取到了它。

本轮同时修掉 3 处缺陷（1 处引擎级 P0 + 2 处我上一轮引入的显示笔误），并给出反事实正算复现。

---

## 1. 事实基线（DB 实证，非推测）

同一次运行 `d9fbe526` 里并存的三个「决策」：

| 来源                                  | action | positionPct | riskLevel | confidence | 用户可见位置                                       |
| ------------------------------------- | ------ | ----------- | --------- | ---------- | -------------------------------------------------- |
| `portfolio-mgr`（公式）               | 增持   | 11.5        | 中风险    | 58.7       | —                                                  |
| `portfolio-risk-gate`（风控门，终值） | 增持   | 11.5        | 中风险    | —          | **界面卡片**（`risk_gate.adjusted=false`，未调整） |
| `quality-fallback`（LLM 兜底）        | 减持   | 5           | 高风险    | 30         | **落库 `decision_json`**                           |

旁证（同一份快照）：

- `quality-gate.matched_label = "acceptable"`、`actual_value = "C"`、`use_llm = false`
  ⇒ **质量门路由正确**（上一轮 `_value` 修复生效）。
- `quality-fallback` 的 LLM 文案自述：
  「`portfolio-risk-gate`输出'增持/11.5%'但trader输出'观望/0%'，方向冲突严重」
  ⇒ **LLM 自己看到的终值是增持 11.5%**，与界面一致、与落库相反。
- `trader` 侧 `agreementBreakdown`：`formulaAction="减持"`、`formulaRiskLevel="高风险"`
  —— 这两个值等于 `quality-fallback` 的输出，即**「公式侧」被 LLM 答案污染**（见 §3.3）。

---

## 2. 引擎级根因：Switch 的默认分支无条件执行（P0）

### 2.1 证据链

1. 模板拓扑（DB `workflow_templates.nodes/edges`，`stock-analysis` v40）：
   ```
   quality-gate (SwitchNode, default_case = "low-quality")
     ├─ sourceHandle="acceptable" ──► decision-explainer
     └─ sourceHandle=null         ──► quality-fallback      ← 默认分支
   ```
   前端 i18n 已把该配置命名为 **「默认分支（兜底）」**（`SwitchPropertyPanel.tsx:270`，
   `zh-CN.json` → `workflow.props.defaultCase`），设计意图就是**互斥兜底**。

2. 运行实证（**硬证据，来自 `workflow_executions.node_executions`，非推测**）：
   `stock_analyses.id=d9fbe526` 对应 `workflow_executions.id=dec22240-07ba-4fe6-96fb-81a463562c95`
   （`created_at=1789257983`，`status=completed`），其 `node_executions` 中：

   ```
   quality-gate        switch  completed   ← 路由节点
   decision-explainer  agent   completed   ← 具名分支 acceptable（正确）
   quality-fallback    agent   completed   ← 默认分支（本应 Skipped！）
   end-output          end     completed
   ```

   且 `quality-fallback` 的 `input` 里赫然带着 `quality-gate.actual_value = "C"` / `case_count=1` /
   `input_var="data-quality.result.grade"` —— 即它**拿到了 Switch 结果、看着 `matched_label` 仍然执行**。
   旁证：`blackboard_snapshot` 里两个节点同时存在。
   ⇒ **一次运行同时执行了两条互斥分支**，这是引擎级事实，与业务逻辑无关。

3. 源码定位（两处叠加，缺一不可）：
   - **`dag_store.rs::compute_ready_nodes`**：第二遍扫描里，「已完成 source + 控制边未选中」
     分支只做 `continue`，其**后没有增量语句**（`remaining_deps` 不会被加），等于空操作
     ⇒ 未选中的分支 target 的依赖照样是 0 ⇒ 直接就绪。
   - **`dag_store.rs::skip_disabled_branch_nodes`**：真正负责「跳过未选中分支」的函数，
     原实现**只认 Condition**（只看 `edge_type == ConditionTrue/ConditionFalse`），
     **Switch 没有任何等价机制**。三个调用点（`engine/mod.rs:3772 / 5892 / 6287`）
     也都写着 `matches!(node, WorkflowNode::Condition(_))`。

   类比：Condition 节点靠 `skip_disabled_branch_nodes` 实现互斥；Switch 少了这一环。

### 2.2 影响面（全库实测，不是估计）

| 模板                | Switch 节点                             | 具名出边                          | 无 handle 出边       | 受影响 |
| ------------------- | --------------------------------------- | --------------------------------- | -------------------- | ------ |
| A股多维度分析 v40   | `quality-gate`（default=`low-quality`） | `acceptable → decision-explainer` | `→ quality-fallback` | **是** |
| Startup MVP 构建 v3 | `s-gonogo`（default=`no-go`）           | `go`/`no-go` 两条都具名           | 无                   | 否     |

全库仅 2 个 Switch 节点、1 个存在无 handle 出边 ⇒ 修复面精确命中该节点。

### 2.3 为什么它一直没被发现

`quality-gate` 的 `continue_on_fail = true`，且两条分支各有下游。
默认分支被执行时唯一后果是「多产出一份结果」，工作流不报错、不失败，
只是**多了一个决策供落库规则去挑**。上一个版本落库只认 `portfolio-mgr`，所以这个多余结果
被闲置了（V40 那条 `quality-fallback` 分支长期不可达）；本轮把落库改成「链尾优先」后，
它立刻被选中 —— **两个缺陷互相掩盖，直到第二个被修好才暴露第一个**。

---

## 3. 本轮修复（6 个文件）

### 3.1 【P0·引擎】Switch 分支跳过（`rt-workflow`）

- 新增纯函数 `control_edge_selected(workflow, edge)`：把「Condition 分支边是否被选中」与
  「Switch 分支边是否被选中」统一成**一个判据**，Condition 语义逐字保持
  （`ConditionTrue/False` 仍按 `source_handle` 回退到 edge_type 比较 `result`）。
- `skip_disabled_branch_nodes` 泛化为「控制节点（Condition / Switch）」，三处调用点同步放开。
- `switch_edge_should_follow(default_case, switch_output, edge)` 判据（`compute_ready_nodes` 与
  跳过逻辑共用同一函数，避免两处漂移）：
  - 带 `source_handle`：仅 `matched_label == handle` 时激活。
  - 不带 `source_handle`（默认分支）：Switch **已声明** `default_case` ⇒ 仅 `matched_label ==
    default_case` 时激活；**未声明** `default_case` ⇒ 维持旧行为（无条件激活，兼容既有模板）；
    Switch 无结果（失败/未跑）⇒ 维持旧行为，这同时是失败兜底路径（拿不到 grade 时必须放行
    保守分支，否则尾链永久 Pending）。
- 汇合点保护（2026-08-28 引入的不动点收敛）同步改用同一判据 —— 真实模板里
  `decision-explainer` 同时挂在具名边与 `quality-fallback` 下游，必须保住它不被连带跳过。

### 3.2 【P0·我方引入】落库被 LLM 兜底覆盖（`decision.rs` / 引擎修复后自愈）

落库优先级（v40 已定为 `quality-fallback → gate → mgr`）**在引擎修好前是错的**：
它用「节点是否有 result」当「是否被走到」的代理，而默认分支无条件执行使该代理恒真。
引擎修复后正常档的 `quality-fallback` 为 Skipped（无 result），优先级链才与真实路由一致。
代码与模板注释均已补上这条前置条件（不再让后来人误以为「链尾优先」本身自足）。

### 3.3 【P0·我方引入】`agreementBreakdown` 的「公式侧」被 LLM 污染

`compute_decision_agreement(decision_json, llm_json)` 的第一个参数取自
`extract_decision_json`（链尾），D/F 档或（修复前）任何档都会拿到 LLM 兜底 ⇒
`formulaAction="减持"` / `formulaRiskLevel="高风险"` 挂着「公式」的名字展示 LLM 的答案，
前端渲染成「公式 ◀ 53 ▶ LLM」（铁律 41：同名字段跨链语义分裂）。

- 新增 `extract_formula_decision_json()`：只认确定性节点（`portfolio-risk-gate` → `portfolio-mgr`），
  **绝不接受 LLM 兜底**。
- `core.rs` 的 agreement 计算改用它；`extract_decision_json` 继续服务落库/仪表盘/退出紧迫度，
  两者分工写进函数文档。

### 3.4 【P1·我方引入】两处除数笔误（展示层，`portfolio-mgr.rhai`）

| 串                          | 原式                 | 0.54 / 0.16 显示为                          | 修法   |
| --------------------------- | -------------------- | ------------------------------------------- | ------ |
| `reasoning` 的 `生效后验`   | `(x*100).round()/10` | `5.4`（与相邻 `后验=0.54` 不同量纲）        | `/100` |
| `computation_logs` 的 `sig` | `(x*100).round()/10` | `1.6`（同次输出 `evidence.avgSignal=0.16`） | `/100` |

**本次运行的原文（直接引用 DB 存档值，非推导）**：

```
reasoning        = 决策=增持 置信=58.7 仓位=11.5% | 缺口(1) 证据=94%
                   | 先验=0.45(bear) 后验=0.54 生效后验=5.4   ← 0.538 被显示成 5.4
                   | 赔率=1.5(波动率fallback) 凯利=0.23 成本=0.3%
computation_logs = prior=0.45(bear) post=0.54 sig=1.6        ← avgSignal 0.1578 被显示成 1.6
                   w=1.407 odds=1.5 k=0.23 pos=11.469... f9=-0.31 ...
```

与同一次运行里 `evidence` 段的 `avgSignal` 对照 ⇒ 1.6 vs 0.1578 = **10 倍**刻度错位。

第二处是第一处的**笔误来源**（照抄了同文件既有写法）。两处都只影响展示串，
但 `computation_logs` 会被决策解释官读取做归因，刻度不一致会把信号强度读错 10 倍（铁律 15）。

`TEMPLATE_VERSION` 40 → **41**（`.rhai` 是模板内容的一部分，不升版不会重种子）。

---

## 4. 验证

### 4.1 反事实正算（铁律 4：先复现存档值）

用 DB 存档画像与存档阈值参数，**严格镜像** `portfolio-mgr.rhai:397-456` 重算：

| 标的   | 画像（存档）                                                                              | 旧判据                     | 新判据               | 存档值               | 复现          |
| ------ | ----------------------------------------------------------------------------------------- | -------------------------- | -------------------- | -------------------- | ------------- |
| 600031 | vol 31.9 / sharpe 0.742 / dd 10.7 / roe 6.2 / debt 48.6 / growth 19.5 / sector `机械设备` | 中风险                     | 中风险               | 中风险               | ✅            |
| 601166 | vol 21.1 / sharpe 0.399 / dd 8.7 / roe 4.8 / debt 91.6 / growth −0.3 / sector `金融`      | **极高风险**（bias −0.15） | **中风险**（bias 0） | 极高风险（v39 运行） | ✅ 旧判据复现 |

- 600031 **风险档未变**（中风险→中风险）：本次样本的差异**不是**风险档修复造成的，
  而是「默认分支」+「落库口径」两件事。
- 601166 的翻转已确认成立，且**与本轮修复无关**（属上一轮 A+C）。
- ⚠️ 本轮第一次写这个校验器时把 `is_high` 写成了析取
  （`debt>65 or vol>40 or …`），而 rhai 实为**合取**
  （`(vol>40 || sharpe<0 || dd>45) && (roe<5 || debt>65)`）⇒ 一度误算出「601166 新判据=高风险」。
  是靠 600031 的存档值复现不上才发现的。**校验器本身也需要正算校准**。

### 4.2 修复前后（同一份真实数据）

| 环节                               | 修复前                                     | 修复后                                       |
| ---------------------------------- | ------------------------------------------ | -------------------------------------------- |
| `quality-gate` 路由                | `matched_label="acceptable"`（v40 已正确） | 同                                           |
| 默认分支 `quality-fallback`        | **被执行**，产出 LLM 决策                  | **Skipped**（无 result）                     |
| 落库 `decision_json`               | `减持 / 5 / 高风险 / conf 30`              | `增持 / 11.5 / 中风险`（= gate，与界面一致） |
| `agreementBreakdown.formulaAction` | `减持`（实为 LLM 输出）                    | `增持`（gate）                               |
| `reasoning` 的 `生效后验`          | `5.4`                                      | `0.54`                                       |
| `computation_logs` 的 `sig`        | `1.6`                                      | `0.16`                                       |

### 4.3 可达性手算：修复后**不会卡尾**（最高风险点）

模板真实尾链拓扑（DB `edges` 实测，`quality-fallback → decision-explainer` 是**汇合点**）：

```
portfolio-risk-gate ──► rule-check ──► quality-gate ──[acceptable]──► decision-explainer
data-quality ─────────────────────────► quality-gate ──[null]──────► quality-fallback ──► decision-explainer
decision-explainer ──► notify-result ──► store-result          portfolio-risk-gate ──► store-result
store-result ──► end-output（叶）
```

逐档手算（`skip_disabled_branch_nodes` 三阶段）：

| 档                    | 具名边 `acceptable`                 | 默认边                       | 阶段 2 汇合点保护                                                                                                                  | 结果                                 |
| --------------------- | ----------------------------------- | ---------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------ |
| A/B/C                 | selected=true → 不跳过              | selected=false → 入 skip_set | `decision-explainer` 的具名入边 selected=true ⇒ **被移出 skip_set**；其下游 `notify-result`/`store-result`/`end-output` 逐个被移出 | 只跳 `quality-fallback`，尾链达成 ✅ |
| D/F                   | selected=false → 入 skip_set ⇒ 跳过 | selected=true → 不跳过       | `decision-explainer` 的直连入边来自 `quality-fallback`（Pending，非 Skipped）⇒ 移出 skip_set                                       | 走 `quality-fallback` → 尾链达成 ✅  |
| Switch 失败/无 result | `matched=None` ⇒ 不激活（正确）     | `(None, _) ⇒ true` ⇒ 激活    | 同上                                                                                                                               | 放行保守分支，尾链达成 ✅            |

⇒ **三个档位都不会留下 Pending 的尾巴**（否则工作流会永久挂住，比多跑一个分支严重得多）。
该性质由 `switch_downstream_merge_point_not_skipped` 单测锁定。

### 4.4 门禁

| 门禁                                                                            | 范围                               | 结果                                                 |
| ------------------------------------------------------------------------------- | ---------------------------------- | ---------------------------------------------------- |
| `cargo fmt --all -- --check`                                                    | 全仓                               | ✅ 无 diff                                           |
| `cargo test -p axagent-rt-workflow --lib`                                       | 引擎全量（含 dag_store 新增 5 项） | ✅ **253 passed / 0 failed**                         |
| `cargo test -p axagent-rt-workflow --lib work_engine::engine::dag_store`        | 定位到 `dag_store` 子集            | ✅ **16 passed / 0 failed**（含 5 项新增，逐条可见） |
| `cargo test -p axagent-rt-workflow --test rhai_syntax_check`                    | 改动的 `.rhai`                     | ✅ 2/2                                               |
| `cargo clippy -p axagent-rt-workflow -p axagent --all-targets -- -D warnings`   | 2 包                               | ✅ 0 warning（10m21s 全量完成，链式 `&&` 未中断）    |
| `__TAURI_WORKSPACE__=true cargo test -p axagent --lib stock_workflow::decision` | 落库口径（含新增 2 项）            | ✅ **26 passed / 0 failed**                          |

新增回归测试（7 项，全部实际执行通过）：

- `dag_store`（5 项）：`switch_default_branch_skipped_when_named_case_matched`、
  `switch_named_branch_skipped_when_default_case_matched`、`switch_downstream_merge_point_not_skipped`、
  `switch_without_default_case_keeps_no_handle_edge`、`condition_false_branch_still_skipped`
- `decision`（2 项）：`formula_decision_json_never_returns_llm_fallback`、
  `formula_decision_json_is_none_without_formula_nodes`

> `skip_disabled_branch_nodes` 此前**零测试覆盖**（Condition 路径也没有），本轮补齐 —— 它正是被这次缺陷穿透的那个函数。

---

## 5. 本轮新发现（记录，未改）

| 级别 | 现象                                                                                                            | 说明                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| ---- | --------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| P1   | `hooks.rs::extract_decision_from_results` 是**第二个 `decision_json` 写者**                                     | 只读 `portfolio-mgr`（既不认风控门也不认质量门），且自建 `analysis_kind="chat"` 新行。它被 `input_has_analysis_id` 短路守住（DB 实证：44 行全是 `live`、0 行 `chat`），所以当前无实害；但**两个提取器口径已经分裂**，一旦走「对话直执行」路径就会写出与主链不同的决策                                                                                                                                                                                     |
| P2   | 界面（工作流 `output`）与落库（`decision_json`）在 D/F 档仍会不一致                                             | 界面读 end-output（`source="portfolio-risk-gate"`），落库取链尾（= `quality-fallback`）。两者对「最终决策」的定义不同，需产品裁决：把 D/F 档的兜底决策也提升为工作流输出，还是取消 V40 的替代语义                                                                                                                                                                                                                                                         |
| P2   | **同一次决策有两个「置信度」，消费端各取一个**                                                                  | `confidence`=58.7（贝叶斯置信度，受 `confidence_quality_cap` 质量上限约束）与 `decisionConfidence`=53.8（方向确信度 = `max(eff,1−eff)×100`，`portfolio-mgr.rhai:1441-1445`）。界面卡片显示 **58.7**，而 `decision-explainer` 的 LLM 在 summary 里写「决策置信度**53.8%**」⇒ 用户在卡片与解释文之间看到两个数。**不是公式 bug**（两量定义不同且都有注释），是**消费端选择未约束**；建议解释官 prompt 显式指定读 `confidence`，或卡片同步展示两值并标注语义 |
| P2   | `quality-fallback` 在 `node_executions` 里排在 **`end-output` 之后**                                            | 未选中分支被调度器排到最后执行 —— 反证这不是「模板刻意并行设计」，而是引擎调度副产物（与 §2.1 硬证据互补）                                                                                                                                                                                                                                                                                                                                                |
| P2   | `stock_analyses.created_at`(ms) 与 `workflow_templates.updated_at`(ms) 相差 4 秒，本次运行早于模板 `updated_at` | 内容证据（`posteriorRaw` 存在、`matched_label="acceptable"`）证明该运行**确实用了 v40**，`updated_at` 应是启动时幂等重种子所致。仅记录，不影响结论                                                                                                                                                                                                                                                                                                        |

---

## 6. 需裁决（沿用上一轮，仍未处置）

| 级别 | 项                                                                                                    | 建议                                                                                                                                                                                                                       |
| ---- | ----------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| P1   | `pm_classify_risk` 已注册但**全仓零调用**（含 DB 模板），运行时实际走 `portfolio-mgr.rhai` 的内联镜像 | **不要接线**：Rust 版硬编码 85/60/−1.5/… 而 rhai 读 `RISK_*` **面板参数**，接线会**丢掉面板可调性**（行为回退）。建议二选一：① 删注册 + 保留 Rust 函数作参考实现（需先在调用处注明）；② 保留现状（已加警示注释）。我倾向 ① |
| P1   | `R-200/201/206/207` 在「公式决策层」与「组合风控门」各自有不同语义                                    | 彻底修需把风控门迁到独立号段（如 `R-3xx`）并同步 `divergence-log.rs` 映射。**属行为变更**，未动                                                                                                                            |
| P2   | `_exitUrgency` 只写不读（后端/前端/类型/e2e 零消费者）                                                | 已把 has_holding 修对并在代码里标注建议；仍需裁决「接消费者」还是「连计算一起删」                                                                                                                                          |

---

## 7. 部署注意

- **需重启应用触发重种子**（`TEMPLATE_VERSION` 41 > DB 现值 40）。
- 本版**不改决策数值**（仅展示串与注释）；上一版（v40）的数值影响见
  `AUDIT-xingye-601166-decision-2026-09-13.md` §5/§9。
- 重跑 600031 后应观察到：界面与落库都为 `增持 11.5% / 中风险`，
  `quality-fallback` 不在 `blackboard_snapshot` 中（无该键），`agreementBreakdown.formulaAction="增持"`。
