> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# 自进化差距分析与改进方案

对比基准：DeepSeek Harness（Cordis 插件架构，发布于 2026-08-13）
当前版本：AxAgent（Tauri v2 + React 19 跨平台 AI 客户端）

## 一、背景

DeepSeek Harness 于 2026 年 8 月 13 日开源，基于「一切皆插件」的 Cordis 内核，实现了 Agent 运行时自修改能力。其核心公式为：

```
Model + Harness = Agent
```

Harness 不是新模型，而是一套可动态组合的 Agent 运行框架。

AxAgent 项目已有相对完整的架构（trajectory 轨迹记录、RL 引擎、工具注册表、工作流引擎、进化引擎骨架），但自进化闭环存在关键断裂，导致进化能力名不副实。

## 二、AxAgent 源码现状

### 2.1 已存在的模块

| 模块 | 路径 | 成熟度 |
|------|------|--------|
| 轨迹记录 | `src-tauri/crates/trajectory/src/trajectory.rs` | 完整，有存储/查询/分析 |
| 技能进化引擎 | `src-tauri/crates/trajectory/src/skill_evolution.rs` | 较完整：GEPA 遗传算法（种群/精英/交叉/变异 + LLM 语义变异 + 沙箱验证闭环） |
| 技能建议生成 | `src-tauri/crates/trajectory/src/skill.rs` | 部分：`SkillProposal::propose_improvement` 规则+文本拼接生成新内容 |
| 工具自动创建 | `src-tauri/crates/trajectory/src/auto_tool.rs` | 骨架：生成后挂在独立 HashMap，未注入运行时 |
| 工作流进化 | `src-tauri/crates/harness/src/workflow_evolution.rs` | 骨架：定义了 WorkflowGenome 与进化 trait 契约，无实际进化；实现方在 `src-tauri/crates/trajectory/src/workflow_adapters.rs`（WorkflowEvolverImpl） |
| RL 引擎 | `src-tauri/crates/trajectory/src/rl.rs` | 研究级：仅奖励计算+TD(λ)优势估计，无策略更新 |
| 统一工具注册表 | `src-tauri/crates/tools/src/registry.rs` | 静态装配，无运行时热插拔 |
| 工具生成器 | `src-tauri/crates/runtime/src/tool_generator/` | 骨架：仅生成 Prompt 模板，非可执行工具 |
| 进化引擎命令 | `src-tauri/src/commands/evolution_engine.rs` | 骨架：`totalEvolutions: 0u64` 写死为 0 |
| 学习图 | `src-tauri/crates/trajectory/src/learning_graph.rs` | 部分实现 |
| 模式分析器 | `src-tauri/crates/trajectory/src/pattern.rs` | 部分实现（PatternLearner / CrossSessionLearner） |

### 2.2 关键断裂点

```
轨迹记录 → 分析 → 生成工具/技能/建议 → 但未注册到运行时工具列表 → Agent 实际用不上
                                                                        ↑
                                                                  闭环断在这里
```

- `AutoToolCreator` 生成的工具仅存在自己的 `created_tools: HashMap<String, GeneratedTool>` 中
- `UnifiedToolRegistry` 是静态装配的，在 Agent 会话初始化时创建，之后不支持动态增删
- 进化引擎执行后结果不自动注入运行中的会话
- 没有运行时注册/卸载的清理机制（无副作用系统）
- 进化产物未注册为能力护照（CapabilityPassport）且不同步工作流图谱 → 认知编排器 L2 混合检索、L3 RAR/图谱路由发现不了 → 用户无感知（前版方案的最大盲区）

### 2.3 认知编排器能力盘点（自进化可直接复用的基座）

认知编排器（Cognitive Orchestrator）是全局唯一被用户消息触发的工作流，由 `cognitive_query` 统一入口驱动，已完成三层路由 DAG（L1 域路由 → L2 簇路由 → L3 RAR+图谱路由）并闭环 8 种执行模式（Ask / Plan / Act / Workflow / Direct / Delegate / ParameterExtract / Clarify）。对自进化方案而言，它提供一组现成的"发现 / 注入 / 反馈 / 安全"基础设施：

| 认知编排器能力 | 位置 | 对自进化闭环的价值 |
|------|------|------|
| 能力护照注册与索引 | `src-tauri/src/commands/capability.rs`（`capability_register_passport` / `index_passport`）+ `CapabilityIndexer` | 进化产物（工具/技能/工作流/Agent）必须注册护照才能被 L2 混合检索与 L3 RAR 发现 —— 闭环的"对外可见"出口 |
| 工作流图谱（DAG 邻接表） | `src-tauri/crates/harness/src/workflow_graph.rs`（`WorkflowGraphSync::sync_workflow`） | L3 `system_workflow_graph_router` 按路径地址路由；进化后的工作流需同步进图谱 |
| RAR 检索增强路由 | `src-tauri/crates/harness/src/rar_router.rs`（Top-K=5 召回 + Prompt 注入） | 进化技能的描述/向量随护照更新，RAR 召回质量随之提升 |
| 系统能力回调通道 | `src-tauri/crates/harness/src/cognitive_router.rs`（`system_*` 前缀节点 → CognitiveRouter） | 现成的"运行时动态能力注入"通道，自指进化工具可走此通道而非硬塞 Agent 预设 |
| 决策标签持久化 | `src-tauri/src/commands/cognitive.rs`（`persist_decision_to_message`） | 每条消息已持久化 execution_mode / route_path / confidence / 选中能力 —— 现成的结构化进化证据流 |
| Clarify / ParameterExtract 分支 | `cognitive_query`（置信度 0.60~0.90 触发澄清） | 天然的低置信度信号，可直接作为进化触发点 |
| 安全隔离与熔断 | `Visibility::SystemOnly` + `CapabilityDomain::System` + 自指熔断（protected_keywords 含 `self_improvement`） | 进化产物注册时须遵守隔离，防止自引用循环与越权路由 |
| 拒绝分支（安全拦截） | `src-tauri/src/commands/cognitive.rs` 前置 1（`PatternPromptGuard` → `PROMPT_REJECTED`） | **本方案最高优先级改造点**：拒绝是终态，未留"拒绝 → 分析 → 补齐"通道，见 2.4 |

### 2.4 认知编排器拒绝分支现状（最高优先级改造点）

认知编排器的前置安全拦截位于 `src-tauri/src/commands/cognitive.rs` 的前置 1（第 423~436 行）：

```rust
// ── 前置 1：安全拦截（拒绝分支）──
// 检测注入/越狱/敏感指令。命中即拒绝执行并记录安全日志，绝不透传给下游执行器。
let prompt_guard = PatternPromptGuard::new();
let input = match prompt_guard.process_user_input(&input) {
    Ok(processed) => processed,
    Err(reason) => {
        tracing::error!(%reason, "🛡️ 安全拦截：检测到注入/越狱，拒绝执行并记录安全日志");
        return Err(CommandError::new(
            axagent_harness::error_codes::cognitive::PROMPT_REJECTED,
        )
        .with_category(ErrorCategory::Unrecoverable)
        .with_detail(reason));
    },
};
```

现状问题：**拒绝是终态**——命中即返回 `COGNITIVE_PROMPT_REJECTED`（Unrecoverable），不留任何"拒绝 → 分析 → 补齐"的通道：

1. **防护规则永不进化**：`PatternPromptGuard`（`src-tauri/crates/harness/src/prompt_guard.rs`）是静态正则列表，攻击手法演进后不会自动补规则。
2. **误伤无兜底**：如 `developer mode` / `you are a developer` 等模式在本地 IDE 桌面客户端场景可能是**合法**开发诉求，却被与越狱模式混为一谈一刀切拒绝，且无申辩路径。
3. **合法能力缺口被吞掉**：用户请求可能隐含"系统当前不具备安全处理该需求的能力"，拒绝直接抹掉了这个进化信号。
4. **拒绝无证据沉淀**：拒绝本身是可贵的进化证据（防护缺口 + 能力缺口），当前只写安全日志，未进决策标签 / 证据流。

### 2.5 两条进化通道现状（能力补齐 vs 能力偏弱进化改进）

自进化闭环需回答"什么时候该产生新能力、什么时候该改进已有能力"。拆成两条通道后逐一对标源码：

| 通道 | 定义 | 源码现状 | 缺口 |
|------|------|----------|------|
| **通道一：能力补齐** | 用户请求的能力在系统中**不存在** → 识别缺口 → 提议生成新能力（**需用户同意**） | **完全不存在**：全库搜索 `capability_gap` / `gap_proposal` / `capability_missing` / `能力补齐` **零匹配**；`NO_CANDIDATE`（cognitive.rs 主 DAG 无产出）仅返回 Retryable 错误；拒绝分支（2.4）为终态拒绝 | **最大盲区**：自进化闭环的"入口"缺失，能力不存在时无任何进化动作 |
| **通道二：能力偏弱进化改进** | 已有能力（工作流/技能）执行质量差 → 反思触发 → 进化改进 | **半闭环（仅工作流侧）**：`WorkflowEvolverImpl::should_auto_evolve` + rt-workflow `post_execution_reflect` 已实现并注入 work_engine；技能侧 `SkillEvolutionEngine::should_auto_evolve` 存在但**无业务调用点** | 见下方 2.5.2 |

#### 2.5.1 通道二现状代码证据

工作流侧（已闭环）——rt-workflow 反思钩子 `post_execution_reflect`（`src-tauri/crates/rt-workflow/src/work_engine/engine/mod.rs`）：

```rust
// 低质量分 + 配置了 evolver → 触发自动进化判定
if reflection.quality_score <= 5
    && let Some(evolver) = evolver
    && let Some(template_id) = template_id_owned.clone()
{
    match evolver.should_auto_evolve(&template_id).await {
        Ok(true) => {
            tracing::info!("[Evolution] auto-evolving template={} (low quality_score={})", …);
            evolver.run(&template_id, std::slice::from_ref(&reflection)).await;
        },
        …
    }
}
```

判定规则（`src-tauri/crates/trajectory/src/workflow_adapters/evolver.rs` `should_auto_evolve`）：

```rust
// 失败率 >= (1 - success_threshold) 且使用次数 >= min_usages 才触发
let failure_threshold = 1.0 - self.config.auto_trigger_success_threshold;
Ok(failure_rate >= failure_threshold)
```

技能侧（`src-tauri/crates/trajectory/src/skill_evolution.rs` `should_auto_evolve`）——定义完整但**无任何业务调用点**（grep 仅在定义处出现）：

```rust
// 条件1：连续失败次数 >= auto_trigger_consecutive_failures
// 条件2：总使用次数 >= auto_trigger_min_usages 且成功率 < auto_trigger_success_threshold
pub fn should_auto_evolve(&self, skill: &Skill) -> bool {
    if skill.consecutive_failures >= self.config.auto_trigger_consecutive_failures {
        return true;
    }
    if skill.total_usages >= self.config.auto_trigger_min_usages
        && skill.success_rate < self.config.auto_trigger_success_threshold
    {
        return true;
    }
    false
}
```

默认阈值（`EvolutionConfig`）：`auto_trigger_min_usages = 3`、`auto_trigger_success_threshold = 0.5`。

#### 2.5.2 通道二缺口

1. **仅工作流侧接入**：技能侧 `should_auto_evolve` 是死代码，未在 Agent 会话/技能执行后触发。
2. **无贝叶斯后验**：判定用"失败率/成功率"启发式，未接 5.4.2 规划的 `P(success)` Beta 后验（置信度 + 执行结果双证据）。
3. **进化产物未注册护照/图谱**：`evolver.run` 产物只落在 evolver 内部状态，未走 5.2.5 的能力护照注册 + 图谱同步，认知编排器 L2/L3 不可见。
4. **自动进化无用户同意环节**：工作流侧 `quality_score <= 5` 即后台自动跑进化，无用户确认/回滚入口（优化器生成的 `WorkflowSuggestion` 注释仅"供前端展示/人工审核"，未实现人工审核落地）。

## 三、DeepSeek Harness 核心机制

### 3.1 架构层级

```
Cordis 内核（6 项基础能力）
  ├── 组件安装 / 卸载
  ├── 变更记录（可逆副作用）
  ├── 能力读写（依赖注入）
  ├── 环境隔离
  ├── 规则拦截
  └── 依赖自动管理
        ↓
插件层（一切皆插件）
  ├── 模型适配器 / 工具注册表 / 技能 / 会话管理
  ├── 上下文组装 / 权限控制 / 审批策略
  ├── 沙箱 / 存储 / 遥测 / UI
  └── Agent Loop 本身也是插件
        ↓
预设层（Profile / Bundle 组合）
  ├── 标准模式（读写文件、Shell、搜索、技能、子Agent）
  ├── PTC 模式（代码组合多步）
  ├── 极简模式（仅 Shell + 编辑器）
  └── 创造模式（标准 + 自指 Cordis 工具）
```

### 3.2 自修改机制

创造模式暴露 7 个 `cordis_*` 工具给 Agent：

| 工具 | 功能 |
|------|------|
| `cordis/inspect` | 检查当前运行时插件树、服务、依赖 |
| `cordis/define` | 定义新插件（校验元数据，生成稳定 Plugin ID，不激活） |
| `cordis/run` | 将已定义插件挂载到运行时（`node:vm` 求值，依赖注入，apply） |
| `cordis/stop` | 卸载插件（自动执行所有 disposer，回收副作用） |
| `cordis/undefine` | 删除插件定义 |
| `cordis/list` | 列出已安装/已定义插件 |
| `cordis/status` | 查询插件状态（ACTIVE / PENDING / FAILED） |

### 3.3 可逆副作用

每个注册动作通过 `ctx.effect()` / `ctx.on()` 安装，卸载时自动回滚：

```
注册:
  fiber = ctx.plugin(MyPlugin)  # 创建 Fiber
  disposer = ctx.tools.register(tool)  # 注册工具，返回清理函数
  ctx.effect(() => disposer())  # 绑定到 Fiber 生命周期

卸载:
  fiber.stop()  # 自动执行所有绑定的 disposer
  ✗ 残留的事件监听器 ✓
  ✗ 残留的工具注册 ✓
  ✗ 残留的网络连接 ✓
```

### 3.4 递归闭环

```
Session Log（append-only）
  ├── 系统提示词
  ├── 推理过程
  ├── 工具调用与结果
  ├── 子代理调度
  └── 每次上下文注入
        ↓
Fork / Resume / Replay / Transcript / Telemetry / Persistence
        ↓
数据沉淀 → 训练优化器 → 优化器本身也能进化
```

## 四、差距矩阵

| 维度 | AxAgent 现状 | DeepSeek Harness | 差距级别 | 追平路径 |
|------|-------------|------------------|---------|---------|
| 运行时自修改 | 进化结果不注入运行时 | Agent 运行时创建/挂载/卸载插件 | **大** | 自指工具 + 运行时热插拔 |
| 插件架构 | 静态 HashMap 注册表 | 完整生命周期 + 依赖注入 | **大** | 给 UnifiedToolRegistry 加动态通道 |
| 可逆副作用 | 无清理机制 | 每个注册有确定清理路径 | **大** | 引入 effect/disposer 模式 |
| 进化粒度 | 技能/工具/工作流有独立引擎 | 任何能力模块都是可进化粒子 | 中 | 统一收敛到工具注册表 |
| 进化闭环 | 轨迹→分析→建议，写回不完整 | 插件修改→持久化→跨会话生效 | 中 | 打通 AutoToolCreator → ToolRegistry |
| 能力发现闭环 | 进化产物未入能力护照/图谱索引 | RAR + 图谱 + 护照索引实时发现 | **大** | 进化 → 能力护照注册 → 图谱同步 → 认知编排器路由可见 |
| RL 训练管线 | 奖励计算+TD(λ)，无策略更新 | 论文级 GRPO 强化学习管线 | **大** | 暂不优先（算力需求高） |
| 工具自动生成 | 有 AutoToolCreator，挂在独立 HashMap | 创建后立即挂载到 Agent scope | 中 | 注册到 UnifiedToolRegistry |
| 组合泛化 | 无 | 通过插件重组适配不同任务 | **大** | 工作流+工具+提示词动态组合 |
| 递归优化器 | 无 | 优化器本身也可被数据驱动进化 | **大** | 先积累 Append-Only 证据 |

## 五、改进方案

### 5.1 总体路线

```
阶段零（P0 最高优先级）：双通道闭环（能力补齐 + 能力偏弱进化改进）
  ├── 通道一「能力补齐」（见 5.7）：能力不存在 → 识别缺口 → 提议生成新能力（需用户同意）
  │   └── 覆盖认知编排器拒绝分支（见 5.6）与 NO_CANDIDATE / Clarify 无候选场景
  └── 通道二「进化改进」（见 5.7）：已有能力偏弱 → 反思 → 改进 → 更新护照/图谱
      └── 补齐工作流侧半闭环（2.5.2）：接入技能侧 + 贝叶斯后验 + 用户确认

阶段一（P0）：运行时热插拔 + 闭环闭合
  └── 打通 AutoToolCreator → UnifiedToolRegistry → 能力护照/图谱（认知编排器可见）

阶段二（P1）：自指工具 + 可逆副作用
  └── 复用 system_* 系统能力通道暴露进化工具（走 SYSTEM_ONLY + 自指熔断）

阶段三（P2）：证据驱动进化
  └── 复用决策标签 + Clarify/执行反馈作为证据，贝叶斯后验决策

阶段四（P3）：组合泛化 + 递归优化（远期）
  └── 经 L3 图谱路由动态组合进化产物
```

### 5.2 阶段一：运行时热插拔（P0）

#### 5.2.1 改造 UnifiedToolRegistry

**文件：** `src-tauri/crates/tools/src/registry.rs`

新增方法：

```rust
impl UnifiedToolRegistry {
    /// 运行时注册工具，返回清理函数（disposer）
    /// 调用方在不再需要时调用 disposer() 自动卸载
    pub fn register_runtime_tool(
        &mut self,
        tool: Arc<dyn Tool>,
        source: &str,
    ) -> Result<(), ToolError> {
        let name = tool.name().to_string();
        // 检查命名冲突
        if self.tools.find(&name).is_some() || self.mcp.find_tool(&name).is_some() {
            return Err(ToolError::duplicate(name));
        }
        // 注册到工具表
        self.tools.register(tool);
        // 记录来源元数据（用于审计和回滚决策）
        self.runtime_tool_sources.insert(name.clone(), source.to_string());
        Ok(())
    }

    /// 运行时卸载工具
    pub fn unregister_runtime_tool(&mut self, name: &str) -> Result<(), ToolError> {
        // 仅允许来自 RUNTIME 来源的卸载
        if !self.runtime_tool_sources.contains_key(name) {
            return Err(ToolError::permission_denied(name, "非运行时注册的工具不可卸载"));
        }
        self.tools.remove(name);
        self.runtime_tool_sources.remove(name);
        Ok(())
    }
}
```

新增字段到 `UnifiedToolRegistry`：

```rust
/// 运行时注册的工具来源（name → source），用于区分静态注册和运行时热插拔
runtime_tool_sources: HashMap<String, String>,
```

#### 5.2.2 AutoToolCreator 输出接入 ToolRegistry

**文件：** `src-tauri/crates/trajectory/src/auto_tool.rs`

将 `validate_and_register` 的签名从「注册到自身 HashMap」改为「通过回调注册到 UnifiedToolRegistry」：

```rust
pub struct AutoToolCreator {
    // ... 现有字段
    /// 注册回调：生成工具后调用此函数将其注册到运行时 ToolRegistry
    /// 返回 disposer 用于后续卸载
    on_tool_created: Option<Box<dyn Fn(GeneratedTool) -> Result<Box<dyn FnOnce() + Send>, String> + Send>>,
}
```

或者在 Tauri 命令层做桥接：`evolution_engine.rs` 中 `run_auto_tool_validate` 调用后，将 `GeneratedTool` 转为 `Arc<dyn Tool>` 再调用 `UnifiedToolRegistry::register_runtime_tool`。

#### 5.2.3 持久化

**文件：** `src-tauri/crates/runtime/src/tool_generator/persistence.rs`

`persist_to_db` 已经存在，确认其表结构支持 `source = 'runtime_evolution'` 标记。进化生成的新工具写入数据库，下次启动时自动加载（通过 `is_runtime_tool` 标记区分内置工具）。

#### 5.2.4 会话中可见

Agent 会话每次组装工具列表时，从 `UnifiedToolRegistry` 读取当前所有工具（包含运行时注册的），无需额外操作。确保 `get_chat_tools()` 方法包含 `runtime_tool_sources` 中的工具。

#### 5.2.5 能力护照注册 + 图谱同步

**关键盲区修正：** 进化产物仅注入 `UnifiedToolRegistry` 仍不够 —— 认知编排器的 L2 检索（`CapabilityRetriever`）依赖能力护照索引，L3 图谱路由依赖 `WorkflowGraph` 邻接表。不注册护照，进化产物对编排器"不可见"，用户无感知。

新增环节：

```rust
// 进化工具注册后，同步注册能力护照（使 L2/L3 路由能发现它）
// 在 evolution_engine.rs 桥接步骤中追加：
async fn register_evolution_product(
    state: &AppState,
    product: &EvolutionProduct,
) -> Result<(), String> {
    match product {
        EvolutionProduct::Tool { name, description, .. } => {
            // 1. 注册到 UnifiedToolRegistry（已由 5.2.2-5.2.3 完成）
            // 2. 注册能力护照（使认知编排器发现）
            let passport = CapabilityPassportDto {
                capability_id: format!("evolution:tool:{}", name),
                name: name.clone(),
                description: description.clone(),
                kind: CapabilityKind::Tool,
                domain: CapabilityDomain::General,
                // L2 簇（三层路由第二层）
                sub_category: "auto_generated".to_string(),
                // 进化产物默认对外可见
                visibility: Visibility::Discoverable,
                // 带 auto_evolved 标签 + 路由提示（L3 路径由图谱决定，route: 标签仅供审计）
                tags: vec![
                    "auto_evolved".to_string(),
                    format!("route:/evolution/auto_generated/tool/{}", name),
                ],
                // 其余必填字段（security_level / planning_complexity / enabled 等）
                // 注意：真实结构未派生 Default，`..Default::default()` 仅为示意省略，
                // 实施时须显式补齐全部必填字段（见实施计划 §8.1）
                ..Default::default()
            };
            state.capability_indexer.index_passport(&passport).await?;
        }
        EvolutionProduct::Workflow { id, name, .. } => {
            // 1. 同步进工作流图谱（L3 图谱路由可见）
            WorkflowGraphSync::sync_workflow(
                &mut state.workflow_graph,
                "evolution",       // domain
                "auto_generated",  // cluster
                id, name,
            );
            // 2. 注册能力护照
            let passport = CapabilityPassportDto {
                capability_id: format!("evolution:workflow:{}", id),
                name: name.clone(),
                kind: CapabilityKind::Workflow,
                domain: CapabilityDomain::General,
                // L2 簇（三层路由第二层）
                sub_category: "auto_generated".to_string(),
                visibility: Visibility::Discoverable,
                tags: vec![
                    "auto_evolved".to_string(),
                    format!("route:/evolution/auto_generated/workflow/{}", id),
                ],
                // 其余必填字段实施时显式补齐（真实结构未派生 Default）
                ..Default::default()
            };
            state.capability_indexer.index_passport(&passport).await?;
        }
    }
    Ok(())
}
```

> **⚠️ 字段修正（相对早期草案，实施前必读）：** `CapabilityPassportDto` 的真实字段以 `src-tauri/crates/harness/src/capability.rs` 为准：主键是 **`capability_id`**（不是 `id`），且**不存在 `path` / `confidence` 字段**。因此：
> - `capability_id` 用 `evolution:{kind}:{name}` 命名；
> - L2 簇通过 `sub_category` 表达（`auto_generated`）；
> - L3 路由路径由 `WorkflowGraphSync::sync_workflow` 决定（上面 Workflow 分支第 1 步），护照中仅以 `route:/evolution/...` 标签留存审计信息；
> - 初始置信度不落护照，留在决策标签（见 5.4.4）；
> - `tags` 是 `Vec<String>`（非 `Option<Vec>`）；
> - 真实结构**未派生 `Default`**，示例中 `..Default::default()` 仅为示意省略，实施时必须显式补齐全部必填字段（`name` / `kind` / `domain` / `visibility` / `security_level` / `planning_complexity` / `enabled` 等）。

> **路径说明：** 进化产物默认注册到 `General` 域 + `auto_generated` 簇，路径为 `/evolution/auto_generated/{type}/{name}`。后期可随证据积累升级到更精确的域/簇（贝叶斯置信度 + 域分类器自动推导）。

> **安全约束：** 进化产物护照的 `visibility` 默认 `Discoverable`，但 `route:/evolution/` 路径前缀纳入认知编排器自指熔断的 `protected_keywords`，防止编排器自身路由到进化产物产生循环。进化产物仅供业务工作流调用，不参与编排器路由回路。

### 5.3 阶段二：自指工具 + 可逆副作用（P1）

#### 5.3.1 自指工具定义

在 `src-tauri/crates/tools/src/` 新增 `runtime_mutation.rs` 模块，定义 4 个工具：

```rust
/// 检查当前运行时工具列表
pub fn inspect_tools(registry: &UnifiedToolRegistry) -> ToolResult

/// 创建新工具（基于 LLM 或 API 定义）
pub fn define_tool(registry: &mut UnifiedToolRegistry, spec: ToolSpec) -> ToolResult

/// 注册并激活工具
pub fn deploy_tool(registry: &mut UnifiedToolRegistry, tool: ToolImpl) -> ToolResult

/// 卸载工具（调用 disposer）
pub fn undeploy_tool(registry: &mut UnifiedToolRegistry, name: &str) -> ToolResult
```

这些工具通过 `ToolRegistry::register` 注册到 `UnifiedToolRegistry`。与 DeepSeek 直接塞进 Agent 预设不同，这里复用认知编排器的 `system_*` 系统能力回调通道注入（见 5.3.4），随主 DAG 的 `SubWorkflowNode` 可用。

#### 5.3.2 可逆副作用模式

引入轻量 `Disposer` 类型：

```rust
/// 撤销动作
pub type Disposer = Box<dyn FnOnce() + Send>;

/// 注册一个副作用，返回清理函数
pub fn register_effect(registry: &mut UnifiedToolRegistry, name: &str, disposer: Disposer) {
    registry.effects.push((name.to_string(), disposer));
}
```

在 `UnifiedToolRegistry` 新增字段：

```rust
/// 当前生效的副作用栈（后进先出，按注册顺序清理）
effects: Vec<(String, Disposer)>,
```

卸载时遍历 `effects` 反向执行 disposer。

#### 5.3.3 权限控制与安全隔离

自指工具应归属于 `danger-full-access` 权限组（与 DeepSeek 创造模式等同 shell 权限一致）。但认知编排器已有现成的安全隔离机制供复用：

- **`Visibility::SystemOnly` + `CapabilityDomain::System`**：自指工具注册为 `SystemOnly` 能力，内部请求可见，用户引导业务路由不可达
- **自指熔断（`SelfReferenceProtection`）**：`protected_keywords` 已含 `self_improvement`，配置 `enabled: true`，任何触发自指工具的路径自动熔断
- **`system_*` 前缀隔离**：走 `system_*` 系统能力回调通道的工具，在主 DAG 执行后由引擎内部回调完成，不经过 L3 公开路由

追加安全约束：

```rust
// 自指工具的能力护照配置（字段同 5.2.5 修正说明：capability_id / sub_category / tags: Vec / 无 path、confidence）
let passport = CapabilityPassportDto {
    capability_id: format!("system:self_evolution:{}", name),
    kind: CapabilityKind::Tool,
    domain: CapabilityDomain::System,           // System 域，业务检索不可达
    sub_category: "self_evolution".to_string(),
    visibility: Visibility::SystemOnly,         // 仅系统内部看到
    tags: vec!["SYSTEM_ONLY".to_string(), "META".to_string()],
    // 其余必填字段实施时显式补齐（真实结构未派生 Default）
    ..Default::default()
};
```

#### 5.3.4 认知编排器对齐：复用 `system_*` 系统能力通道

**文件：** `src-tauri/crates/harness/src/cognitive_router.rs`

认知编排器已实现 `system_*` 前缀节点 → CognitiveRouter 系统能力回调机制（如 `system_rar_retriever` / `system_workflow_graph_router`），这是现成的"运行时动态能力注入"通道，原方案 6.2 中"在 `src-tauri/src/commands/agent/` 注册自指工具"的方案应替换为复用此通道：

1. 把自指进化工具（`inspect` / `define` / `run` / `stop` / `undefine` / `list` / `status`）包装为 `system_evolution_*` 系统能力，通过 `SubWorkflowNode` 挂进主 DAG，由引擎系统能力回调执行 —— 不污染 Agent 预设的工具列表。
2. 系统能力天然遵守 `SYSTEM_ONLY` 隔离 + 自指熔断 + 作用域隔离，无需另建安全模型。
3. 若需要 Agent 会话内直接调用，再以只读/受限方式透出（注册为普通工具并挂 `auto_evolved` 标签），权限阈值由用户显式授权。

相比原方案，此通道复用既有编排基座、隔离与熔断能力，改动更小且更符合项目分层（进化能力走 `system_*` 系统通道，不进业务能力检索）。

### 5.4 阶段三：证据驱动进化（P2）

#### 5.4.1 Append-Only 轨迹

将 `TrajectoryStorage` 改造为 append-only 模式：

- 每条轨迹记录包含：会话 ID、时间戳、输入、推理、工具调用链、结果、奖励
- 记录不可删除（可标记为失效）
- 支持按时间范围/会话/工具名回放

#### 5.4.2 贝叶斯后验

参考 `Bayesian-Agent` 论文思路，对每个技能/工具维护一个 Beta 分布后验：

```rust
pub struct SkillPosterior {
    alpha: f64,  // 成功计数 + 1
    beta: f64,   // 失败计数 + 1
    prior_alpha: f64,  // 先验参数
    prior_beta: f64,
}
```

决策规则：

- 如果 `P(success) < 阈值` → 触发进化
- 如果 `P(success) > 高阈值` 且样本量足够 → 标记为稳定
- 如果样本量不足但有失败 → 标记为待观察

#### 5.4.3 进化策略

从当前 if-else 规则升级为基于证据的策略：

1. 收集轨迹证据 → 计算后验
2. 后验低于阈值 → 触发进化
3. 进化方式：LLM 生成结构化 diff（而非文本追加）
4. 沙箱验证 → 验证通过后更新
5. 记录进化结果到 Append-Only 日志

#### 5.4.4 认知编排器对齐：决策标签 + Clarify / 执行反馈作为证据源

**文件：** `src-tauri/src/commands/cognitive.rs`

阶段三的"证据"不必另起炉灶采集 —— 认知编排器已经在每次用户消息上持久化了结构化决策：

- **决策标签**（`persist_decision_to_message`）：每条消息已含 `execution_mode` / `route_path` / `confidence` / 选中工作流或专家，天然构成 append-only 决策流（随消息存储，不可删除，可回放）。
- **低置信度信号**：`Clarify` 分支（置信度 0.60~0.90）与 `ParameterExtract` 精准命中阈值，是现成的"该能力路由不稳"进化触发点。
- **执行结果反馈**：`Workflow` 分支的 WorkEngine 执行成败 / 超时 / 降级，作为工作流进化的奖惩信号。
- **安全拦截拒绝（阶段零产物）**：拒绝分支经 5.6 改造后，命中原因 + 补齐结果写入决策标签（拒绝即证据），补充防护缺口信号（见 5.6 步骤⑤）。

5.4.2 的贝叶斯后验直接消费这些决策记录（置信度 + 执行结果双证据），无需为决策标签另建 append-only 表 —— 决策已随消息存储（与 5.4.1 的 `TrajectoryStorage` 追加模式改造互补：决策标签为轻量证据、完整轨迹为重量证据）：

```
证据源 = 决策标签流（execution_mode / confidence / route_path）
      + 执行反馈（Workflow 成败 / Plan 拒绝 / Clarify 澄清率）
      + 安全拦截拒绝（防护缺口信号，见 5.6）
        ↓
贝叶斯后验（Beta 分布）→ 低于阈值触发进化
        ↓
进化产物 → 能力护照 / 图谱同步（见 5.2.5）→ 下一轮用户输入可命中
```

进化策略从 if-else 升级为"决策置信度 + 执行结果"双证据驱动，与认知编排器的决策观测天然打通。

### 5.5 阶段四：组合泛化（远期）

通过工作流引擎（`rt-workflow`）的热插拔节点组合，实现跨任务适配：

- 将工具/技能/提示词包装为标准工作流节点
- 运行时根据任务类型动态组合节点 DAG
- 进化结果表现为节点拓扑的调整

### 5.6 最高优先级需求：拒绝分支 → 能力补齐（需用户同意）

> **为什么是最高优先级**：其余阶段解决"进化产物如何注入运行时"，本需求解决"系统如何识别并补齐能力缺口"——它是自进化闭环的**起点**。没有它，每一次安全拦截都是终态拒绝，进化无从触发。二者是互补的上下环：阶段零让"拒绝"变成可被认可的进化机会，阶段一/二让进化产物真正落地。
>
> **铁律**：任何能力补齐动作**必须征得用户显式同意**，绝不自动执行。

#### 5.6.1 目标

将认知编排器前置 1（见 2.4）的"命中即拒绝"改为"拒绝 + 能力补齐提议"双通道：

- **安全底线不变**：命中注入/越狱模式，依然**绝不透传给下游执行器**（硬阻断保留，仍写安全日志）。
- **新增第二通道**：阻断的同时分析能力缺口，生成 `CapabilityGapProposal`，征求用户同意后执行补齐。

```
命中防护（PatternPromptGuard Err，见 2.4）
        ↓
① 保留硬阻断：绝不透传（安全底线不变，仍写安全日志）
        ↓
② 拒绝分析（CapabilityGapAnalysis）：把裸 reason 归类为结构化缺口
   ├─ 防护缺口 guard_rule：攻击手法不在 PatternPromptGuard 静态列表 → 提议补防护规则
   ├─ 误伤豁免 exempt_authorize：本地 IDE 场景的合法诉求（如 developer mode）→ 提议安全豁免授权
   └─ 能力缺口 capability_missing：用户请求系统当前无能力安全处理 → 提议生成补齐工作流/技能
        ↓
③ 生成能力补齐提议（CapabilityGapProposal）：
   ├─ gap_type：guard_rule / exempt_authorize / capability_missing
   ├─ proposal：新增防护规则 / 授权范围 / 补齐工作流（工作流按 5.2.5 注册护照+图谱）
   ├─ reason：命中原因 + 缺口说明
   ├─ impact：影响范围
   └─ rollback：可逆副作用（挂 Disposer，见 5.3.2）
        ↓
④ 征求用户同意（复用 FilePermissionRequestEvent / AgentPermissionRequest 事件通道）
   ├─ 前端弹窗：展示提议内容 / 风险 / 回滚方式
   ├─ 同意 → 执行补齐（注册 / 更新 / 授权，挂 effect/disposer）
   └─ 拒绝 → 保持原拒绝行为，记录决策标签
        ↓
⑤ 闭环记录：拒绝原因 + 补齐结果 写入决策标签 / 证据流（供 5.4 贝叶斯后验消费）
```

#### 5.6.2 改造点

**文件：** `src-tauri/src/commands/cognitive.rs`（前置 1 拒绝分支）

```rust
// 前置 1 改造：命中防护后不再直接返回错误，而是进入能力补齐提议通道
let prompt_guard = PatternPromptGuard::new();
let input = match prompt_guard.process_user_input(&input) {
    Ok(processed) => processed,
    Err(reason) => {
        tracing::error!(%reason, "🛡️ 安全拦截：检测到注入/越狱，进入能力补齐提议通道");
        // 1. 拒绝分析：把裸 reason 归类为结构化缺口
        let proposal = build_capability_gap_proposal(&reason, &input).await?;
        // 2. 征求用户同意（复用授权事件通道，前端弹窗展示提议与风险）
        if !await_user_consent(&state, &proposal).await? {
            // 用户拒绝 → 保持原拒绝行为 + 记录决策标签（拒绝即证据）
            persist_decision_to_message(db, &msg_id, &decision).await;
            return Err(CommandError::new(
                axagent_harness::error_codes::cognitive::PROMPT_REJECTED,
            )
            .with_category(ErrorCategory::Unrecoverable)
            .with_detail(reason));
        }
        // 3. 用户同意 → 执行补齐（防护规则 / 授权 / 工作流，挂 disposer）
        apply_capability_gap_proposal(&state, &proposal).await?;
        // 4. 补齐后的输入按安全化语义继续（或按用户选择直接放行）
        Ok(input.to_string())
    },
};
```

**文件：** `src-tauri/crates/harness/src/prompt_guard.rs`

`process_user_input` 的 `Err(String)` 升级为结构化拒绝分类（保留兼容字符串展示）：

```rust
/// 结构化拒绝结果：分类 + 命中模式 + 建议
pub struct PromptRejection {
    pub category: PromptAttackCategory, // RoleOverride / Delimiter / TaskHijack / PromptLeak / Jailbreak
    pub pattern: String,
    pub reason: String,
}
```

**文件：** `src-tauri/crates/harness/src/error_codes.rs`（`mod cognitive`）

新增错误码（前端按 `error.${code}` 翻译并展示补齐提议）：

```rust
/// 能力补齐提议已生成，等待用户同意（非阻断，走事件通道展示）
pub const GAP_PROPOSAL_PENDING: &str = "COGNITIVE_GAP_PROPOSAL_PENDING";
```

> 错误码属后端公共契约：新增 `COGNITIVE_GAP_PROPOSAL_PENDING` 时，须同步补齐前端 11 种语言的 `error.${CODE}` 翻译（zh-CN 为源）。

#### 5.6.3 安全边界（铁律）

1. **同意是硬前提**：所有补齐动作必须用户显式同意，无"默认同意"、无静默自动补齐。
2. **补齐不降低防护**：补齐动作本身遵守 `SYSTEM_ONLY` 隔离 + 自指熔断（`/evolution/` 前缀纳入 `protected_keywords`），进化产物仅供业务工作流调用，不进编排器路由回路。
3. **可逆**：每个补齐动作挂 `effect` / `Disposer`（见 5.3.2），用户可随时撤销。
4. **豁免有界**：`exempt_authorize` 类豁免只针对命中的具体模式与作用域，不开放全量授权。

### 5.7 双通道闭环设计（能力补齐 + 能力偏弱进化改进）

> 对应 2.5 的两条通道现状：**通道一完全不存在（最大盲区）**、**通道二仅工作流侧半闭环**。本节给出统一的生命周期闭环设计，让"能力不存在"与"能力偏弱"两条路都走向"产物可见"。

#### 5.7.1 能力生命周期总览

```
用户请求
  │
  ├─ 能力不存在 ────────────► 通道一：能力补齐（5.7.2）
  │     · NO_CANDIDATE（主 DAG 无产出）
  │     · 拒绝分支（安全拦截命中，见 5.6）
  │     · Clarify 无候选 / 低置信度兜底仍无命中
  │
  └─ 能力存在但偏弱 ─────────► 通道二：进化改进（5.7.3）
        · 反思质量分低（post_execution_reflect，已存在）
        · 失败率 / 成功率触发（should_auto_evolve，已存在）
        · 贝叶斯后验 P(success) < 阈值（5.4.2 接入）
          │
          ▼
进化 / 补齐产物 ──► 能力护照注册 + 图谱同步（5.2.5）
          │
          ▼
认知编排器 L2 混合检索 / L3 RAR 可见 ──► 下一轮用户输入命中
```

两条通道共用一套下游出口（5.2.5 护照 + 图谱）与安全约束（同意铁律 / SYSTEM_ONLY / 可逆）。

#### 5.7.2 通道一：能力补齐（对不存在的能力）

**目标**：把"找不到能力"从报错升级为"提议生成新能力"。

**触发点**（三处统一走 `CapabilityGapProposal` 提议通道）：
- 主 DAG 无候选：`NO_CANDIDATE`（cognitive.rs 第 729 行）→ 不返回错误，改为生成 `capability_missing` 提议
- 拒绝分支：`PROMPT_REJECTED`（见 5.6，`gap_type = guard_rule / exempt_authorize / capability_missing`）
- Clarify 兜底无候选：候选为空且无 `forcedCapabilityId` → 同样进入提议

**流程**：

```
识别缺口（缺什么能力）→ 生成 CapabilityGapProposal（gap_type=capability_missing）
  → 征求用户同意（复用授权事件通道）
  → 同意后生成能力：
      · 简单能力 → 注册普通工具护照（evolution:tool:*）
      · 复杂能力 → 生成工作流模板 + 注册护照 + 图谱同步（走 5.2.5）
      · 领域能力 → 一并生成 AgentProfile 护照（agent_profile:*）
  → 挂 Disposer（可回滚）
```

**与 5.6 的关系**：5.6 解决"安全拦截命中"场景的补齐（可能不需要新能力，只需豁免/补规则）；本节解决"能力客观不存在"场景的补齐（必须新建能力）。两者统一走 `CapabilityGapProposal` 提议与同意通道，5.6 是通道一在安全拦截场景下的特例。

#### 5.7.3 通道二：进化改进（对已有能力偏弱）

**目标**：把工作流侧已有半闭环补完整，并扩展到技能/工具载体。

**复用现状**（不重造）：
- rt-workflow `post_execution_reflect`（质量分 <= 5 触发）—— 已存在
- `WorkflowEvolverImpl::should_auto_evolve` / `run`（失败率 + 使用次数判定）—— 已存在并注入 work_engine
- `WorkflowSuggestion`（optimizer 生成，注释标注"供人工审核"）—— 已有雏形，未落地

**补四个缺口**（对应 2.5.2）：

| 缺口 | 补齐动作 | 涉及文件 |
|------|----------|----------|
| 技能侧 `should_auto_evolve` 死代码 | 在 Agent 会话 / 技能执行完成后挂反思钩子，触发技能进化 | `src-tauri/crates/runtime-core/src/conversation.rs`、`src-tauri/crates/trajectory/src/skill_evolution.rs` |
| 无贝叶斯后验 | 判定从"失败率/成功率"升级为 `P(success)` Beta 后验（5.4.2），双证据（置信度 + 执行结果） | `src-tauri/crates/trajectory/src/evidence.rs`（新增） |
| 进化产物未注册护照/图谱 | `evolver.run` 产物走 5.2.5 护照注册 + 图谱同步，`/evolution/` 前缀 + `auto_evolved` 标签 | `src-tauri/src/commands/evolution_engine.rs`、`src-tauri/src/commands/capability.rs` |
| 自动进化无用户同意 | 进化产物落库前生成 `WorkflowSuggestion` 征求用户同意（复用授权事件通道），同意后更新模板/护照 | `src-tauri/src/commands/workflow_reflection.rs`、前端 UI |

**安全边界**：与 5.6 一致的铁律——改进产物默认需用户同意才落库生效；产物遵守 SYSTEM_ONLY 隔离 + 自指熔断 + Disposer 可回滚。

## 六、影响范围

### 6.1 阶段一涉及文件

| 文件 | 改动 | 风险 |
|------|------|------|
| `src-tauri/crates/tools/src/registry.rs` | UnifiedToolRegistry 新增 `register_runtime_tool` / `unregister_runtime_tool` / `runtime_tool_sources` | 低 |
| `src-tauri/crates/trajectory/src/auto_tool.rs` | AutoToolCreator 新增注册回调接口 | 中 |
| `src-tauri/src/commands/evolution_engine.rs` | 桥接 AutoToolCreator 输出到 UnifiedToolRegistry | 低 |
| `src-tauri/crates/runtime/src/tool_generator/persistence.rs` | 确认持久化标记 | 低 |
| `src-tauri/src/commands/error_code.rs` | 新增 TOOL_REGISTRATION_DUPLICATE 等错误码 | 低 |
| `src-tauri/src/commands/capability.rs` | 进化产物注册能力护照（index_passport / capability_register_passport） | 中 |
| `src-tauri/crates/harness/src/workflow_graph.rs` | 进化工作流同步图谱（WorkflowGraphSync::sync_workflow） | 中 |

### 6.2 阶段二涉及文件

| 文件 | 改动 | 风险 |
|------|------|------|
| `src-tauri/crates/tools/src/`（新增 `runtime_mutation.rs`） | 自指工具定义 | 中 |
| `src-tauri/crates/tools/src/registry.rs` | 新增 `effects` 字段和副作用管理 | 中 |
| `src-tauri/crates/harness/src/cognitive_router.rs` | `system_evolution_*` 系统能力回调 + SYSTEM_ONLY 护照（见 5.3.4） | 中 |
| `src/stores/feature/` | 前端权限控制 UI | 低 |

### 6.3 阶段三涉及文件

| 文件 | 改动 | 风险 |
|------|------|------|
| `src-tauri/crates/trajectory/src/storage.rs` | 追加模式改造 | 中 |
| `src-tauri/crates/trajectory/src/`（新增 `evidence.rs`） | 贝叶斯后验计算 | 低 |
| `src-tauri/crates/trajectory/src/skill_evolution.rs` | 进化策略从 if-else 升级为 LLM diff | 中 |
| `src-tauri/src/commands/cognitive.rs` | 决策标签 / Clarify 反馈接入证据源（见 5.4.4） | 低 |

### 6.4 阶段零（双通道闭环）涉及文件

| 文件 | 改动 | 风险 |
|------|------|------|
| `src-tauri/src/commands/cognitive.rs` | 前置 1 拒绝分支 + `NO_CANDIDATE` 改为"拒绝/无候选 + 能力补齐提议"双通道（见 5.6.2 / 5.7.2） | 中 |
| `src-tauri/crates/harness/src/prompt_guard.rs` | `process_user_input` 返回结构化 `PromptRejection`（分类 + 命中模式） | 低 |
| `src-tauri/crates/harness/src/runtime_types/`（新增 `capability_gap.rs`） | `CapabilityGapProposal` / `PromptAttackCategory` 类型定义 | 低 |
| `src-tauri/crates/harness/src/error_codes.rs` | 新增 `COGNITIVE_GAP_PROPOSAL_PENDING` 错误码 | 低 |
| `src-tauri/src/commands/capability.rs` | 补齐/进化产物（工具/工作流/技能）注册能力护照 + 图谱同步（复用 5.2.5） | 中 |
| `src-tauri/crates/trajectory/src/evidence.rs`（新增） | 贝叶斯后验 `P(success)`，通道二判定升级（见 5.7.3） | 低 |
| `src-tauri/crates/runtime-core/src/conversation.rs` | 技能执行完成后挂反思钩子，触发技能进化（补 2.5.2 缺口） | 中 |
| `src-tauri/src/commands/workflow_reflection.rs` | `WorkflowSuggestion` 征求用户同意后落库（补 2.5.2 缺口） | 中 |
| `src/stores/feature/` + `src/components/` | 能力补齐/进化改进同意弹窗 UI（展示提议/风险/回滚，复用授权事件通道） | 低 |

## 七、与 DeepSeek Harness 核心差异说明

### 7.1 不做的

1. **不照搬 Cordis 插件内核**：AxAgent 已有成熟的 Harness 架构（`crates/harness/`），构建完整的 Cordis 等价物是框架级重构，投入产出比低。

2. **不做真实 GRPO 训练**：桌面客户端缺乏 GPU 算力，且 RL 训练对用户不可见，产品价值低。

3. **不做完整插件生态**：DeepSeek 的 100+ 插件是社区贡献的，项目不需要复制数量。

### 7.2 做的

1. **只做"运行时动态能力"**：聚焦于让进化结果能即时注入 Agent 会话，这是"进化感"的直接来源。

2. **只做"可逆副作用"**：确保动态注册/卸载不污染运行状态，这是动态自改能长期存活的前提。

3. **只做"证据驱动"**：不依赖固定规则，用真实轨迹数据做决策，避免过度进化。

4. **复用认知编排器作为进化闭环基座**：进化产物经能力护照 + 图谱同步进入认知编排器的 L2/L3 发现与路由；自指工具走 `system_*` 系统能力通道；证据直接消费决策标签流 —— 让"进化感"落地为"用户下一次输入即可命中进化产物"。

## 八、参考

- [DeepSeek Harness 官方仓库](https://github.com/deepseek-ai/deepseek-harness)
- [DeepSeek Harness 论文：时空可组合性编程范式](https://github.com/cordiverse/paper)
- [Bayesian-Agent: Posterior-Guided Skill Evolution](https://arxiv.org/pdf/2606.08348)
- [DeepSeek Harness 深度解析：一切皆插件](https://blog.csdn.net/lotusxyhf/article/details/163742435)
- [DeepSeek Harness：Agent 自改造运行时](http://m.toutiao.com/group/7674206127969534516/)
