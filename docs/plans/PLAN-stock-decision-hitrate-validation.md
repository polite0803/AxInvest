# PLAN：阶段 0.5 —— 单周期决策命中率验证（反思工作流增强版）

> 关联：`PROPOSAL-stock-decision-four-horizon.md` 第 3.5 节
> 状态：done ｜ 日期：2026-09-23 ｜ 对应版本：v2.9.9
> **v2 核心变更：废弃虚拟建仓路径（paper_positions 扩展 / 自动建仓 / 惰性结算全部取消）**。
> **实施记录**：M1（`was_correct` 确定性化）→ `reflection.rs`；M2（命中率聚合）→ `analysis-engine/reflection_stats.rs`；M3（前端面板）→ `ReflectionPanel.tsx`。M4（样本积累）标记可选，未启动。
> 原因（用户质疑 + 代码实证）：股票反思是**独立工作流**，且已具备完整后市验证能力（见第 2 节）——
> 虚拟建仓与其功能重叠，属重复造轮子。本版只做三件补缺：was_correct 确定性化、命中率聚合、前端展示。

## 1. 目标（北向星）

在**反思工作流已产出的验证数据**上，补齐确定性判定与聚合统计，让「单周期决策准不准」有**可复算、有基线**的后市实证：

| 口径 | 数据来源（反思工作流已有） | 意义 |
|---|---|---|
| **方向命中率** | `net_return_pct` 符号 vs 决策方向（comparator 已有 `direction_match`） | 决策方向准不准 |
| **价位命中率** | `snapshot.target_reached`（是否达目标价） | 目标价档位准不准 |
| **平均收益 / alpha** | `raw_return`、`alpha_return` | 综合盈亏与超额能力 |

**北向约束**：不新建虚拟建仓表、不动 `decision_action` 语义、不引入 LLM 新输出、不造新工作流 —— 只消费反思工作流已落库的 `stock_reflections` 数据。

## 2. 现状基线（已确认，勿重复调研）

| 项 | 现状 | 依据 |
|---|---|---|
| 反思独立工作流 | `hindsight_date`（反思日）+ 真实行情 `snapshot` + `actual_outcome` 事实描述，决策后延迟触发 | `crates/entities/src/stock_reflections.rs:16-35` |
| snapshot 判定 | `target_reached` / `target_progress_pct` / `net_return_pct` / `alpha_pct` / `trading_days` / `within_expected_horizon`（期中标注）已存在 | `commands/stock_workflow/reflection.rs:2656-2685`（含测试） |
| 方向命中 | comparator 已有 `direction_match`（曾因注入 0.0 恒 false，已修复为快照净收益） | `reflection.rs:1121-1126` |
| **缺口 1** | `was_correct` 依赖 `verdict`（LLM 反思结论）⇒ 11 条 completed 恒 0 | `reflection.rs:16-25` 注释 |
| **缺口 2** | 反思数据**未聚合**成命中率基线（无方向命中率 / 平均收益统计） | 全仓检索结果 |
| **缺口 3** | 前端无命中率面板 | 检索结果 |
| 反思触发 | 独立入口（非分析工作流内节点）；样本积累依赖触发频率（手动/定时待确认） | 用户澄清 + 检索 |

## 3. 改动清单（M1→M3，按序实施，每步独立验收）

### M1：was_correct 确定性化（核心修复）

**问题**：`was_correct` 由反思 agent 的 `verdict` 反推 ⇒ LLM 自评 ⇒ 恒 0，等于验证字段形同虚设。

**改动**：
1. 读 `commands/stock_workflow/reflection.rs` 现有 `was_correct` 填充逻辑（第 16-25 行注释对应的落库处），确认其计算来源。
2. 改为**确定性判定**：用快照数据反推 —— `direction_match`（方向对）且 `within_expected_horizon` 语义正确时 `was_correct=true`；判定不得来自 LLM 文本。具体映射表（写注释锁定）：
   - 方向命中（`net_return_pct` 符号与决策方向一致）→ `correct`
   - 方向未命中（反之）→ `incorrect`
   - 数据缺失 / 期中观察 → `unknown`（**禁止强判**）
3. 补单测：构造已知 net_return_pct 与决策方向，断言 was_correct 判定（test mod 追加文件末尾，防 `items_after_test_module`）。

**验收**：存量/新增反思记录 `was_correct` 有真实分布（非全 0）；单测覆盖 correct/incorrect/unknown 三分支。

### M2：命中率聚合服务（新增，analysis-engine）

**新增** `stock_reflections` 聚合函数（放 `crates/analysis-engine`，如 `reflection_stats`）：

```
HitrateStats {
  total_samples,            // 总样本
  direction_hit_rate,       // 方向命中率 = was_correct=correct / 可判样本
  target_hit_rate,          // 价位命中率 = target_reached=true / 有目标价样本
  avg_raw_return_pct,       // 平均净收益
  avg_alpha_pct,            // 平均超额收益
  by_action: Vec<{action, samples, direction_hit_rate}>,  // 按决策方向分组
  by_horizon: Vec<{horizon, samples, direction_hit_rate}>,// 按时间维度分组（为四周期打底）
}
```

规则：
- **样本 < 5 时 `direction_hit_rate` 返回 `null` 并标注「样本不足」**，不产出伪结论（诚实性铁律）。
- 只统计已结算（有 snapshot）记录；`pending` / `unknown` 单列，不进命中率分子。
- 按 `time_horizon` 分组保留 —— 四周期评估的数据基础。

**验收**：构造 5+ 样本可产出分组统计；样本不足返回 null；字段 camelCase 与前端一致。

### M3：前端命中率面板（最小改动）

1. `src/types/stock-analysis.ts`：新增 `HitrateStats` DTO（与 M2 后端对齐，camelCase，禁止本地重定义）。
2. 后端新增只读命令 `reflection_stats`（`#[tauri::command]` + `pub mod` 声明，不手改 register_commands.rs）；前端经 `@/lib/invoke` 调用。
3. 展示：反思/历史分析面板增加命中率区块（方向命中率 / 价位命中率 / 平均收益 / alpha / 样本数，按 action 与 horizon 分组）。
4. i18n：新增 key 须 11 语言补齐（铁律 2）；优先复用既有文案。

**验收**：面板可读、样本不足显式标注、前后端字段一致（跑 `.workbuddy/tmp/scan_ipc_args.py`）。

### M4（可选）：样本积累

**问题**：命中率统计质量取决于反思工作流的触发频率。确认现有触发机制（手动 / 定时 / 分析后自动），若为手动低频，评估是否补「分析完成后延迟 N 天自动排程反思」。
**约束**：不引入常驻调度器；仅在有现成排程机制时挂载（先调研，不盲做）。

## 4. 验收门槛（阶段 0.5 门槛规则，写回 PROPOSAL 3.5）

1. `was_correct` 有真实分布（非全 0），判定为确定性反推（非 LLM 自评）。
2. 命中率统计可产出、可复算，样本不足显式标注，有历史基线可对比。
3. 方向命中率 / 价位命中率 / 平均收益三类指标前端可读。
4. **达成以上 → 恢复四周期评估；未达成 → 阶段 1/2 不得实施。**

## 5. 风险与回退

| 风险 | 等级 | 缓解 / 回退 |
|---|---|---|
| 样本不足（反思触发频率低） | 中 | 样本 < 5 显式标注；M4 评估自动排程（先调研不盲做） |
| `was_correct` 判定歧义（期中观察 / 数据缺失强判） | 低-中 | 三分支映射表 + `unknown` 单列；单测锁死 |
| 历史反思记录 snapshot 缺失无法判定 | 低 | 只对**有 snapshot** 的记录判定；缺失标 `unknown` |
| 误改反思工作流主逻辑 | 低 | 只改 was_correct 填充点，不动 snapshot / actual_outcome / comparator 主路径 |
| clippy 连锁（test mod 位置、浮点判据） | 低 | 三进门禁全量跑；浮点比较逐情形等价（NaN/±INF） |

**回退**：本计划不改反思工作流主干、不动决策语义，任一步可独立回退。

## 6. 门禁与规范（贯穿全程）

- 三进门禁（CI 口径）：`cargo fmt --all --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`__TAURI_WORKSPACE__=true cargo test --workspace`；前端 `npm run lint:eslint` + `npm run typecheck`。
- DTO：Rust snake_case + `#[serde(rename_all = "camelCase")]`；前端消费 camelCase；invoke 传参 camelCase。
- 新增 UI 文本 11 语言补齐；命令注册只做 `pub mod` + `#[tauri::command]`，不手改 `register_commands.rs`。
- 不新增表结构（**本版无 migration**，除非 M4 排程需要）；`stock_reflections` 字段扩展须 migration。

## 7. 执行提示（防迷失）

- 每步开工前回读「目标」节：本次改动只服务于命中率实证，**不新建虚拟建仓、不扩大范围**。
- M1→M3 按序推进，每步验收打勾；M4 先调研触发机制再决定，不盲做。
- 遇到与「现状基线」不符的事实，先更新第 2 节再继续。