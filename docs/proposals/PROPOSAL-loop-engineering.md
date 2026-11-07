> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# Loop Engineering 基础设施

## 概述

在 AxAgent 基座层实现 **Loop Engineering**（自改进循环）基础设施。这是 AI 智能体的最新范式：Agent 生成输出后自我评估，根据反馈迭代改进，直到达到质量标准，而不是一次生成完事。

### 核心思想

```
生成 → 自我评估 → 识别不足 → 注入改进提示 → 重新生成 → 再次评估...
```

### 架构分层

```
AxAgent (基座 — 通用能力)
  └─ SelfImprovingRound trait (harness)     ← 契约
  └─ SelfImprovementExecutor (agent)        ← 通用执行器
  └─ ReactEngine 桥接 (agent)               ← 集成到现有循环
  └─ UI Feature Toggle (前端)               ← 用户可启用的开关

AxInvest / AxOPC / AxSim (业务 — 领域评估)
  └─ impl SelfImprovingRound for Stock/...
  └─ 只实现 evaluate_round，基座管迭代策略
```

---

## 一、Harness 层：新增 trait 契约

**文件**: `crates/harness/src/self_improving_loop.rs` — 新建

```rust
/// 一轮"执行+评估+决定"的完整回合结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundResult {
    pub round: u32,
    pub output: String,
    pub evaluation: Option<RoundEvaluation>,
    pub trace: Vec<RoundStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundEvaluation {
    pub score: f64,              // 0.0~1.0，越高越好
    pub confidence: f64,
    pub gaps: Vec<String>,       // 遗漏/不足的描述
    pub strengths: Vec<String>,
    pub raw_assessment: String,  // LLM 原始评估文本
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundStep {
    pub index: u32,
    pub kind: String,            // "think" | "act" | "observe" | "reflect"
    pub summary: String,
    pub tokens_used: u32,
}

/// 下一轮的动作决策
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NextAction {
    /// 接受当前输出，结束
    Accept,
    /// 根据给的提示继续改进（下一轮从改进提示开始）
    Refine { direction: String },
    /// 完全换方向重做
    Redirect { reason: String },
    /// 需要人类介入
    Escalate { message: String },
}

/// 自改进循环的契约 —— 由业务层实现，基座提供通用执行器
#[async_trait]
pub trait SelfImprovingRound: Send + Sync {
    /// 执行一轮：接收任务+上一轮评估，输出本轮结果
    async fn execute_round(
        &mut self,
        task: &str,
        prev_evaluation: Option<&RoundEvaluation>,
    ) -> Result<RoundResult, Box<dyn std::error::Error + Send>>;

    /// 自我评估本轮输出质量
    async fn evaluate_round(
        &self,
        task: &str,
        result: &RoundResult,
    ) -> Result<RoundEvaluation, Box<dyn std::error::Error + Send>>;

    /// 根据评估决定下一步
    async fn decide_next(
        &self,
        task: &str,
        result: &RoundResult,
        evaluation: &RoundEvaluation,
    ) -> Result<NextAction, Box<dyn std::error::Error + Send>>;
}

// 相关错误类型
#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    #[error("Exceeded maximum rounds: {0}")]
    MaxRoundsExceeded(u32),
    #[error("Escalated to human: {0}")]
    Escalated(String),
    #[error("Internal error: {0}")]
    Internal(#[from] Box<dyn std::error::Error + Send>),
}
```

**文件**: `crates/harness/src/lib.rs` — 编辑

```rust
pub mod self_improving_loop;  // 追加到 pub mod 列表中
```

---

## 二、Agent 层：通用执行器

**文件**: `crates/agent/src/self_improvement_executor.rs` — 新建

```rust
use axagent_harness::self_improving_loop::*;
use std::sync::Arc;

/// 自改进循环的配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfImprovementConfig {
    /// 最大执行轮数（默认 5）
    pub max_rounds: u32,
    /// 收敛阈值（默认 0.85），高于此直接 Accept
    pub convergence_threshold: f64,
    /// 连续无进展多少次后 Escalate（默认 3）
    pub escalate_threshold: u32,
}

impl Default for SelfImprovementConfig {
    fn default() -> Self {
        Self {
            max_rounds: 5,
            convergence_threshold: 0.85,
            escalate_threshold: 3,
        }
    }
}

/// 最终输出
pub struct FinalOutput {
    pub text: String,
    pub total_rounds: u32,
    pub final_evaluation: RoundEvaluation,
    pub round_history: Vec<RoundResult>,
}

/// 通用自改进执行器
pub struct SelfImprovementExecutor {
    inner: Box<dyn SelfImprovingRound>,
    config: SelfImprovementConfig,
    round_history: Vec<RoundResult>,
    no_progress_count: u32,
}

impl SelfImprovementExecutor {
    pub fn new(
        inner: Box<dyn SelfImprovingRound>,
        config: SelfImprovementConfig,
    ) -> Self {
        Self {
            inner,
            config,
            round_history: Vec::new(),
            no_progress_count: 0,
        }
    }

    /// 运行完整的自改进循环
    pub async fn run(&mut self, task: &str) -> Result<FinalOutput, LoopError> {
        let mut prev_evaluation: Option<RoundEvaluation> = None;
        let mut best_score = 0.0_f64;

        for round in 1..=self.config.max_rounds {
            // 1. 执行一轮
            let mut result = self.inner
                .execute_round(task, prev_evaluation.as_ref())
                .await
                .map_err(LoopError::Internal)?;
            result.round = round;

            // 2. 自我评估
            let evaluation = self.inner
                .evaluate_round(task, &result)
                .await
                .map_err(LoopError::Internal)?;
            result.evaluation = Some(evaluation.clone());

            self.round_history.push(result);

            // 3. 收敛检测
            if evaluation.score >= self.config.convergence_threshold {
                return Ok(FinalOutput {
                    text: self.round_history.last().unwrap().output.clone(),
                    total_rounds: round,
                    final_evaluation: evaluation,
                    round_history: self.round_history.clone(),
                });
            }

            // 4. 连续无进展检测
            if evaluation.score > best_score {
                best_score = evaluation.score;
                self.no_progress_count = 0;
            } else {
                self.no_progress_count += 1;
            }

            if self.no_progress_count >= self.config.escalate_threshold {
                return Err(LoopError::Escalated(
                    format!("No score improvement for {} rounds (best: {})",
                        self.no_progress_count, best_score)
                ));
            }

            // 5. 决定下一步
            match self.inner
                .decide_next(task, self.round_history.last().unwrap(), &evaluation)
                .await
                .map_err(LoopError::Internal)?
            {
                NextAction::Accept => {
                    return Ok(FinalOutput {
                        text: self.round_history.last().unwrap().output.clone(),
                        total_rounds: round,
                        final_evaluation: evaluation,
                        round_history: self.round_history.clone(),
                    });
                }
                NextAction::Refine { direction } => {
                    // direction 会通过 prev_evaluation 传递
                    // 实际由 SelfImprovingRound::execute_round 的实现决定如何使用
                }
                NextAction::Redirect { reason } => {
                    // 换方向：prev_evaluation 重置，下一轮 execute_round 会感知
                }
                NextAction::Escalate { message } => {
                    return Err(LoopError::Escalated(message));
                }
            }

            prev_evaluation = Some(evaluation);
        }

        Err(LoopError::MaxRoundsExceeded(self.config.max_rounds))
    }
}
```

**文件**: `crates/agent/src/lib.rs` — 编辑

```rust
pub mod self_improvement_executor;  // 追加到 pub mod 列表中
```

---

## 三、Agent 层：集成到 ReAct 引擎

### 3.1 ReActEngine 结构体增加字段

**文件**: `crates/agent/src/react_engine.rs` — 编辑

在 `ReActEngine` 结构体（约 L30-60）追加：

```rust
pub struct ReActEngine {
    pub config: ReActConfig,
    pub reasoning_provider: Arc<dyn ReasoningProvider>,
    // ↓ 新增字段
    pub reflector: Option<Arc<Reflector>>,
    pub goal_evaluator: Option<GoalEvaluator>,
    pub enable_self_improvement: bool,
    // ↑
}
```

### 3.2 ReActConfig 增加配置字段

**文件**: `crates/agent/src/react_engine.rs` — 编辑 `ReActConfig`

```rust
pub struct ReActConfig {
    // ... 现有字段不变

    // ↓ 新增
    /// 是否启用自改进循环（最终输出质量门）
    pub self_improvement_enabled: bool,      // 默认 false
    /// 质量阈值（对应 Reflector QualityMetrics 分数，0-10）
    pub min_quality_threshold: u8,           // 默认 5
    /// 最终输出后做质量反射检查
    pub final_output_reflection: bool,       // 默认 false
    /// 自改进循环配置
    pub self_improvement_loop_config: Option<SelfImprovementConfig>,
    // ↑
}
```

在 `impl Default for ReActConfig` 中补充：

```rust
self_improvement_enabled: false,
min_quality_threshold: 5,
final_output_reflection: false,
self_improvement_loop_config: None,
```

### 3.3 Synthesizing → Finished 路径加质量门

找到 `process_state` 方法中的 `ReasoningState::Synthesizing` 分支（约 L1284），修改为：

```rust
ReasoningState::Synthesizing => {
    let output = self.reasoning_provider.synthesize(chain, context).await?;

    // 新增：最终输出质量检查
    if self.config.final_output_reflection && self.enable_self_improvement {
        if let Some(reflector) = &self.reflector {
            let record = TaskExecutionRecord::from_chain(chain);
            let reflection = reflector.reflect(&record).await;
            if reflection.quality.score < self.config.min_quality_threshold {
                // 质量不达标：回退到 Thinking，注入改进建议
                context.add_reflection_hint(&reflection.improvement_suggestions);
                // 记录反射结果到 chain 以便后续回溯
                chain.add_reflection(reflection);
                return Ok((ReasoningState::Thinking, true));
            }
        }
    }

    Ok((ReasoningState::Finished, false))
}
```

### 3.4 Reflecting 状态改为调用 Reflector

找到 `ReasoningState::Reflecting` 分支（约 L1235），修改为：

```rust
ReasoningState::Reflecting => {
    let reflection = if let Some(reflector) = &self.reflector {
        // 使用结构化 Reflector
        let record = TaskExecutionRecord::from_chain(chain);
        let r = reflector.reflect(&record).await;
        // 将反思结果注入 context，后续 Thinking 轮能用
        context.add_reflection_hint(&r.improvement_suggestions);
        r
    } else {
        // 回退到原有的 LLM 内省
        self.reasoning_provider.reflect(chain, context).await?
    };

    self.adjust_strategy(context);
    Ok((ReasoningState::Thinking, true))
}
```

注意：为了最小化改动，`context.add_reflection_hint` 是一个新方法，需要在 `ReasoningContext` 中追加：

```rust
// 在 ReasoningContext 中追加
impl ReasoningContext {
    pub fn add_reflection_hint(&mut self, hint: &str) {
        self.reflection_hints.push(hint.to_string());
    }
}

// 在 struct 中追加字段
pub struct ReasoningContext {
    // ... 现有字段
    pub reflection_hints: Vec<String>,  // 新增，默认空
}
```

并在 `process_state` 的 `Thinking` 分支中将 `reflection_hints` 注入到 LLM 请求中（约 L1140-1160）：

```rust
ReasoningState::Thinking => {
    let mut prompt = build_thinking_prompt(user_input, chain, context);
    // 新增：如果有反射提示，注入到系统 prompt
    if !context.reflection_hints.is_empty() {
        let hints = context.reflection_hints.join("\n");
        prompt.system_hints = Some(format!(
            "Previous reflections suggest improvements:\n{hints}\n\n\
             Incorporate these insights into your thinking."
        ));
        context.reflection_hints.clear(); // 消费后清空
    }
    // ...
}
```

### 3.5 为 AgentCoordinator 新增适配器

**文件**: `crates/agent/src/agent_coordinator.rs`（或 coordinator.rs） — 在适当位置添加：

```rust
use crate::self_improvement_executor::{SelfImprovementExecutor, SelfImprovementConfig, FinalOutput};
use axagent_harness::self_improving_loop::*;

/// 把 ReActEngine 包装为 SelfImprovingRound
pub struct AgentRoundAdapter {
    engine: Arc<ReActEngine>,
}

#[async_trait]
impl SelfImprovingRound for AgentRoundAdapter {
    async fn execute_round(
        &mut self,
        task: &str,
        prev_evaluation: Option<&RoundEvaluation>,
    ) -> Result<RoundResult, Box<dyn std::error::Error + Send>> {
        let input = if let Some(eval) = prev_evaluation {
            format!(
                "{task}\n\n## Previous attempt feedback\n\
                 Score: {}/1.0\n\
                 Gaps identified: {}\n\n\
                 Improve based on these gaps and produce a better result.",
                eval.score,
                eval.gaps.join("; ")
            )
        } else {
            task.to_string()
        };

        let result = self.engine.run(&input).await?;
        Ok(RoundResult {
            round: 0, // 由 executor 填充
            output: result.final_output(),
            evaluation: None,
            trace: result.steps.into_iter().map(|s| RoundStep {
                index: s.index as u32,
                kind: s.state.as_str().to_string(),
                summary: s.summary,
                tokens_used: s.tokens_used as u32,
            }).collect(),
        })
    }

    async fn evaluate_round(
        &self,
        _task: &str,
        result: &RoundResult,
    ) -> Result<RoundEvaluation, Box<dyn std::error::Error + Send>> {
        // 默认实现：用 reasoning_provider 的 LLM 做评估
        let assessment = self.engine.reasoning_provider.evaluate_output(&result.output).await?;
        Ok(RoundEvaluation {
            score: assessment.score,
            confidence: assessment.confidence,
            gaps: assessment.gaps,
            strengths: assessment.strengths,
            raw_assessment: assessment.raw_text,
        })
    }

    async fn decide_next(
        &self,
        _task: &str,
        _result: &RoundResult,
        evaluation: &RoundEvaluation,
    ) -> Result<NextAction, Box<dyn std::error::Error + Send>> {
        if evaluation.gaps.is_empty() {
            Ok(NextAction::Accept)
        } else {
            Ok(NextAction::Refine {
                direction: format!("Address these gaps: {}", evaluation.gaps.join("; ")),
            })
        }
    }
}
```

### 3.6 在 AgentCoordinator 中嵌入选自改进循环

在 `run_impl` 方法中（约 L400-650），找到调用 `self.engine.run(user_input)` 的地方：

```rust
// 在 run_impl 内部
if self.config.self_improvement_enabled {
    if let Some(loop_config) = &self.config.self_improvement_loop_config {
        let adapter = AgentRoundAdapter::new(self.engine.clone());
        let mut executor = SelfImprovementExecutor::new(
            Box::new(adapter),
            loop_config.clone(),
        );
        match executor.run(user_input).await {
            Ok(final_output) => {
                // 从 FinalOutput 提取文本作为最终回复
                result = Ok(final_output.text);
            }
            Err(LoopError::MaxRoundsExceeded(n)) => {
                // 超轮数仍返回最后一轮输出
                if let Some(last) = executor.round_history.last() {
                    result = Ok(last.output.clone());
                }
            }
            Err(LoopError::Escalated(msg)) => {
                // 需要人类介入，输出当前最佳结果 + 提示
                if let Some(last) = executor.round_history.last() {
                    result = Ok(format!(
                        "{}\n\n---\n⚠️ 自动改进已暂停：{msg}",
                        last.output
                    ));
                }
            }
            Err(e) => {
                tracing::warn!("SelfImprovementLoop failed: {e:?}");
                // 降级到直接执行
            }
        }
    }
}

// 如果未启用自改进或降级了，走原来的路径
if result.is_none() {
    result = Some(self.engine.run(user_input).await?);
}
```

### 3.7 AgentConfig 新增字段

**文件**: `crates/agent/src/agent_config.rs` — 编辑

```rust
pub struct AgentConfig {
    // ... 现有字段不变
    /// 是否启用自改进循环
    pub enable_self_improving_loop: bool,     // 默认 false
    /// 自改进循环配置
    pub self_improvement_loop: Option<SelfImprovementConfig>,  // 默认 None
}
```

在 `impl Default for AgentConfig` 中：

```rust
enable_self_improving_loop: false,
self_improvement_loop: None,
```

---

## 四、后端命令层：AppConfigState 新增字段

**文件**: `src/commands/agent/config_commands.rs` 或 `src/app_state.rs`（位置根据实际结构）— 编辑

在 `AppConfigState`（或对应的序列化结构体）中追加：

```rust
/// 自改进循环开关
#[serde(default)]
pub self_improving_loop: bool,

/// 最终输出质量门开关
#[serde(default)]
pub final_output_reflection: bool,
```

确保 `get_app_config` 和 `save_app_config` 命令中处理这两个新字段。

---

## 五、前端：Feature Flags UI 开关

### 5.1 FeatureFlags 接口扩展

**文件**: `src/stores/feature/appConfigStore.ts`

```typescript
export interface FeatureFlags {
  // ... 现有 7 个
  selfImprovingLoop: boolean;      // 新增
  finalOutputReflection: boolean;  // 新增
}
```

初始值：

```typescript
const initialFeatures: FeatureFlags = {
  // ... 现有
  selfImprovingLoop: false,
  finalOutputReflection: false,
};
```

### 5.2 FEATURE_FLAG_META 追加条目

**文件**: `src/components/settings/SettingsPanel.tsx`

在 `FEATURE_FLAG_META` 数组末尾追加：

```typescript
{
  key: "selfImprovingLoop",
  icon: <SyncOutlined />,
  labelKey: "settings.agent.featureFlags.selfImprovingLoop",
  descKey: "settings.agent.featureFlags.selfImprovingLoopDesc",
},
{
  key: "finalOutputReflection",
  icon: <CheckCircleOutlined />,
  labelKey: "settings.agent.featureFlags.finalOutputReflection",
  descKey: "settings.agent.featureFlags.finalOutputReflectionDesc",
},
```

确认 `@ant-design/icons` 已导入 `SyncOutlined` 和 `CheckCircleOutlined`。如果已有其他导入，直接用即可。

### 5.3 i18n 翻译

**文件**: `src/i18n/locales/zh-CN.json`

在 `settings.agent.featureFlags` 对象中追加：

```json
"selfImprovingLoop": "自改进循环",
"selfImprovingLoopDesc": "Agent 在生成输出后自我评估，根据反馈迭代改进，直到达到质量标准",
"finalOutputReflection": "最终输出质量门",
"finalOutputReflectionDesc": "Agent 合成最终回答前自动检查质量，不达标时自动回退改进"
```

**文件**: `src/i18n/locales/en-US.json`

```json
"selfImprovingLoop": "Self-Improving Loop",
"selfImprovingLoopDesc": "Agent self-evaluates its output and iteratively improves based on feedback until quality threshold is met",
"finalOutputReflection": "Final Output Reflection",
"finalOutputReflectionDesc": "Auto-check output quality before final synthesis; revert and improve if below threshold"
```

其余 9 种语言加同 key 占位（值可用机器翻译或暂时留空）。

---

## 六、变更文件清单汇总

| 操作 | 文件 | 说明 |
|------|------|------|
| 新建 | `crates/harness/src/self_improving_loop.rs` | RoundResult/NextAction 类型 + SelfImprovingRound trait |
| 编辑 | `crates/harness/src/lib.rs` | + pub mod self_improving_loop; |
| 新建 | `crates/agent/src/self_improvement_executor.rs` | SelfImprovementExecutor 通用执行器 |
| 编辑 | `crates/agent/src/react_engine.rs` | 新增字段 + 质量门 + Reflecting 调用 Reflector |
| 编辑 | `crates/agent/src/agent_config.rs` | 新增配置字段 |
| 编辑 | `crates/agent/src/coordinator.rs` | run_impl 嵌入选自改进循环 |
| 编辑 | `crates/agent/src/lib.rs` | + pub mod self_improvement_executor; |
| 编辑 | `src/commands/.../config_commands.rs` | AppConfigState 新增字段 |
| 编辑 | `src/stores/feature/appConfigStore.ts` | FeatureFlags + 初始值 |
| 编辑 | `src/components/settings/SettingsPanel.tsx` | FEATURE_FLAG_META 加两个条目 |
| 编辑 | `src/i18n/locales/zh-CN.json` | i18n key 翻译 |
| 编辑 | `src/i18n/locales/en-US.json` | i18n key 翻译 |

---

## 七   向后兼容说明

- 所有新增 Rust struct 字段均有 `#[serde(default)]` 或 `Default` 实现
- 所有前端新增字段初始化为 `false`/`None`
- 所有配置项默认关闭，不影响现有行为
- 现有测试应全部通过，无需修改

---

## 八、验证方式

1. `cargo check -p axagent-harness -p axagent-agent` 通过
2. `cargo clippy -p axagent-harness -p axagent-agent -- -D warnings` 通过
3. 现有单元测试全部通过
4. 新增单元测试覆盖 `SelfImprovementExecutor`：
   - `test_converges_on_high_score`
   - `test_exceeds_max_rounds`
   - `test_escalates_on_no_progress`
5. 前端：打开设置 → Agent 控制面板 → 功能 Tab，看到"自改进循环"和"最终输出质量门"两个开关
6. 开关开启后，Agent 在复杂任务中应出现多轮改进行为
