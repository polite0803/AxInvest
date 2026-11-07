> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# 多 Agent 无人值守编排系统设计方案

> 2026-06-21 | 状态: **草案，待评审** | 作者: AI 助理

---

## 1. 背景与目标

### 1.1 问题域

复杂长任务在 AI 执行过程中面临的核心挑战：

| 问题 | 表现 | 现有方案不足 |
|------|------|-------------|
| **上下文遗忘** | Agent 处理长任务时中间结果丢失、思路跑偏 | 工作流引擎有 DAG 但缺乏"保洁交接"（结构化 handover） |
| **职责模糊** | Agent 间自由对话，交流范围不可控 | Swarm/Debate 节点没有固定的结构化交接协议 |
| **不可审计** | AI 做了哪些修改、是否真实执行无法验证 | AuditEntry 记录事件但不验证工作质量 |
| **环境混用** | 所有节点共用同一上下文，工具链不可控 | 无环境隔离机制 |
| **编排瓶颈** | 人工需要监控每个步骤 | 无顶层 Orchestrator 自动分解+监控 |

### 1.2 设计目标

对现有 `rt-workflow` 引擎做**增量扩展**，构建多 Agent 无人值守编排系统：

1. **长任务拆分** — Orchestrator 将高层任务分解为子任务图，每个子 Agent 在干净上下文中完成单件事后交棒
2. **结构化图编排** — 串行/并行/扇出/汇聚/打回/异步投递，拓扑锁死执行顺序，节点间通过固定格式交接
3. **无人值守** — Orchestrator 不编码，只做任务分解、启动图、监控和重规划兜底
4. **YAML 接口** — 工作流、Agent 角色、环境、安全阈均可 YAML 描述，可版本控制
5. **Scorecard 审计** — 验证 AI 输出质量，含节点漂移检测、命令真实执行校验、端到端测试
6. **环境即配置** — 不同角色按需配置 Docker 镜像/工具链/环境变量

### 1.3 与现有系统关系

```
现有系统                        扩展
────────────────────────────────────────────
WorkflowNode (30 种)         → + Orchestrator 变体
SubGraph (容器子图)          → + DynamicSubGraph（运行时生成）
AgentRole (8 种)             → Coordinator Role 增强 + 环境配置
Plan/Phase/Task              → + GraphPlan（图拓扑计划）
NodeExecutionRecord          → + StructuredHandover 字段
AuditEntry                   → + Scorecard 记录
JSON 序列化                  → + YAML 一等公民
```

---

## 2. 现有能力评估

### ✅ 已有能力（直接复用）

| 能力 | 位置 | 说明 |
|------|------|------|
| DAG 执行引擎 | `rt-workflow/engine.rs` | Kahn + JoinSet，成熟的异步并发调度 |
| 容器 SubGraph | `harness/workflow_types.rs` | 6 种容器节点支持子图嵌入 |
| Agent 角色 | `rt-workflow/agent_roles.rs` | Coordinator 职责天然适合 Orchestrator |
| Plan 审批 | `agent_executor.rs: PlanCallbacks` | 生成→审批→执行模式成熟 |
| NodeDispatcher | `dispatcher.rs` | 注册式，新增执行器仅需几行 |
| AuditEntry | `harness/audit_trail.rs` | 审计框架已就绪 |
| YAML 依赖 | `serde_yaml 0.9` 已在 workspace | 零成本接入 |
| ContainerNode 前端 | `Nodes/ContainerNode.tsx` | 通用容器渲染，Orchestrator 可直接复用 |
| Loop 中断/恢复 | `loop_executor.rs` | checkpoint+interrupt 可复用为暂停审查 |
| SubWorkflow 递归 | `subworkflow_executor.rs` | 工作流嵌套已支持 |

### ❌ 需要新建的模块

| 模块 | 估算 | 优先级 |
|------|------|--------|
| OrchestratorNode 类型（Rust 变体 + TS） | 小 | **P0** |
| OrchestratorExecutor（任务分解+监控） | 大 | **P0** |
| StructuredHandover 6 项协议 | 中 | **P0** |
| DynamicSubGraph（运行时生成子 DAG） | 中 | **P0** |
| YAML 工作流导入导出 | 中 | P1 |
| Scorecard 审计引擎 + DB 表 | 大 | P1 |
| 环境配置（Docker/工具链） | 中 | P2 |
| 前端 OrchestratorNode UI | 中 | **P0** |
| 前端 Scorecard Panel | 中 | P1 |
| 前端 YAML Editor | 小 | P2 |
| 拓扑增强：扇出/打回/异步边 | 中 | P1 |

### 🔧 需要修改的现有模块

| 模块 | 修改内容 |
|------|---------|
| `WorkflowNode` 枚举 | 新增 `Orchestrator` 变体 + Deserialize + base() |
| `NodeOutput` | 新增可选 `handover` 字段 |
| `node_type_name()` | 新增 `orchestrator` 类型名 |
| `NODE_TYPE_MAP` (TS) | 新增 `orchestrator` 条目 |
| `WorkflowEditor.tsx` (TS) | 注册 OrchestratorNode 组件 |
| `Nodes/index.ts` (TS) | 导出 OrchestratorNode |

---

## 3. 架构设计

### 3.1 核心概念

```
┌─────────────────────────────────────────────────────────────┐
│                       Orchestrator                          │
│  ┌──────────┐  ┌────────────┐  ┌────────┐  ┌────────────┐  │
│  │ 任务分解  │→ │ Worker 分派 │→ │ 监控   │→ │ 结果汇总   │  │
│  │ (LLM/策略)│  │            │  │ +重规划 │  │ +Scorecard │  │
│  └──────────┘  └────────────┘  └────────┘  └────────────┘  │
│  输入: mission + YAML config                                │
│  不做: coding / tool 调用                                    │
└─────────────────────────────────────────────────────────────┘
         │ 输出: SubTask[]
         ▼
┌─────────────────────────────────────────────────────────────┐
│                      Worker Graph                           │
│  [Analyst] ──→ [Designer] ──→ [Developer] ──→ [Reviewer]   │
│  handover()     handover()       handover()     scorecard() │
│  每个 Worker: 干净上下文 + 6 项交接协议                        │
│  拓扑: 串行/并行/扇出/打回/异步                                │
└─────────────────────────────────────────────────────────────┘
```

### 3.2 结构化交接协议

Agent 间交接的六项信息，**缺一不可**，可被程序自动检查：

```yaml
handover:
  completed_work: "完成了数据清洗模块的重构"
  changes:                                 # 改了哪些文件
    - file_path: "src/data/cleaner.rs"
      change_type: refactor
      summary: "单线程改为并行流水线"
      lines_added: 120
      lines_removed: 85
  next_steps: "下一步需对接新数据源接口"
  remaining_issues: "极端大文件下可能有内存压力"
  dependencies: "需要上游先完成 schema 定义"
  validation_evidence: "通过全部 42 个测试，覆盖率 93%"
```

**设计**：每个 `NodeOutput` 增加可选 `handover` 字段，AgentExecutor 执行完毕后必须填写，缺项时 Scorecard 标记不通过。

### 3.3 Orchestrator 节点数据模型

```rust
OrchestratorNodeConfig {
    mission: String,                              // 高层任务
    strategy: OrchestrationStrategy,              // 编排策略
    worker_roles: Vec<AgentRoleAssignment>,        // Worker 角色
    max_parallel_workers: u32,                    // 最大并行
    max_replans: u32,                             // 最大重规划
    scorecard_config: Option<ScorecardConfig>,    // 审计配置
    environment: Option<EnvironmentConfig>,        // 环境配置
    safety_thresholds: Option<SafetyThresholds>,  // 安全阈值
    sub_graph: Option<SubGraph>,                  // 预定义子图
    enable_dynamic_subgraph: bool,                // 运行时生成子图
}
```

**编排策略**：

| 策略 | 描述 | 适用 |
|------|------|------|
| `Ordered` | 串行，一个接一个 | 有严格顺序依赖的任务 |
| `FanOut` | 并行扇出 | 独立子任务并行处理 |
| `Pipeline` | 阶段内并行，阶段间串行 | 开发流水线 |
| `Race` | 竞速，取最先完成的 | 方案探索/快速原型 |
| `Debate` | 多角度辩论后裁决 | 架构决策 |
| `Dynamic` | LLM 动态决定拓扑 | 结构不确定的任务 |

### 3.4 Scorecard 审计系统

对每个 Worker 节点输出进行多维度验证：

```rust
ScorecardReport {
    node_id, execution_id,
    overall_score: f64,             // 0.0~1.0
    checks: Vec<ScorecardCheck>,    // 单项检查
    passed: bool,                    // overall >= threshold
    failures: Vec<String>,
}
```

| 检查 | 描述 | 实现 |
|------|------|------|
| **Drift（偏移）** | 输出是否符合预期格式/范围 | JSON Schema 校验 + 关键词/正则匹配 |
| **Verification（验证）** | 命令是否真实执行 | 检查 audit_trail 的 input_hash 链 |
| **E2eTest（端到端）** | 能否通过预设测试 | 执行 e2e_test_command |
| **Completeness（完整性）** | 交接协议是否缺项 | 检查 HandoverProtocol 6 个必填字段 |

### 3.5 YAML 核心接口

完整工作流可用 YAML 定义，作为"源代码"版本控制：

```yaml
workflow:
  name: "重构数据清洗模块"
  version: "1.0.0"
topology:
  strategy: pipeline
  max_parallel: 3
agents:
  - role: "analyst"
    count: 1
    model: "doubao-seed-2-0"
    environment:
      toolchain: ["python3", "ruff"]
  - role: "developer"
    count: 2
    environment:
      docker_image: "rust:1.80"
nodes:
  - id: "analyze"
    type: agent
    agent_role: "analyst"
    config: { system_prompt: "分析代码结构", output_var: "report" }
edges:
  - source: "analyze"
    target: "implement"
    edge_type: direct
environment:
  working_dir: "/workspace/project"
  env_vars: { RUST_LOG: "debug" }
safety:
  max_consecutive_failures: 3
  allow_fs_write: true
```

### 3.6 拓扑增强

在已有 `EdgeType` 基础上新增 6 种增强边：

```rust
enum EnhancedEdgeType {
    FanOut,        // 1:N 扇出分发
    Converge,      // N:1 汇聚合并
    BounceBack,    // 下游打回上游（审核不通过）
    AsyncDelivery, // 异步投递（不阻塞执行流）
    Retry,         // 业务级重试
    DataOnly,      // 仅传数据不传控制流
}
```

**BounceBack（打回）** 是最复杂的拓扑模式：

```
[Developer] ──→ [Reviewer]
    ↑               │
    └─ bounceBack ──┘   (审核不通过，打回)
```

复用 LoopExecutor 的 `interrupt_after_each` 模式实现。

---

## 4. 数据流

```
用户输入 "重构数据清洗模块"
  │
  ▼
Orchestrator.receive(mission)
  │
  ├─ decompose_mission()
  │   → SubTask[] = [
  │       { id: "analyze", role: "analyst", deps: [] },
  │       { id: "design",  role: "designer", deps: ["analyze"] },
  │       { id: "code",    role: "developer", deps: ["design"] },
  │       { id: "review",  role: "reviewer", deps: ["code"] },
  │     ]
  │
  ├─ build_subgraph()
  │   → SubGraph { nodes: [Agent 节点], edges: [依赖边] }
  │     // 每个 Agent 节点已注入 system_prompt + handover 模板
  │
  ├─ inject_to_context()
  │   → context.variables["sub_tasks"] = ...
  │   → context.variables["orchestrator_plan"] = ...
  │
  └─ engine.execute(dynamic_subgraph)   // 复用现有 DAG 引擎
      │
      ├─ [analyze] → handover = { completed_work, changes, ... }
      ├─ [design]  → 读取 handover → 设计 → handover
      ├─ [code]    → 读取 handover → 编码 + 测试 → handover
      └─ [review]  → Scorecard 检查 → 报告
```

---

## 5. 实施计划（三阶段）

### Phase 1 — 核心数据结构 + Orchestrator 节点（P0，2-3天）

**目标**：在工作流中放置 Orchestrator 节点并执行静态子图

| 任务 | 文件 | 预估 |
|------|------|------|
| Rust: 新增 `orchestrator` 模块（HandoverProtocol, OrchestratorNodeConfig, ScorecardConfig 等） | `harness/src/orchestrator.rs` | 400行 |
| Rust: `WorkflowNode::Orchestrator` 变体 + Deserialize | `harness/workflow_types.rs` | 20行 |
| Rust: `OrchestratorExecutor` | `rt-workflow/.../executors/orchestrator_executor.rs` | 300行 |
| Rust: 注册到 `NodeDispatcher` | `dispatcher.rs` + `executors/mod.rs` | 10行 |
| TS: `OrchestratorNode` 类型 + `NODE_TYPE_MAP` | `workflow.types.ts` | 30行 |
| TS: `OrchestratorNode` 组件（复用 `ContainerNode`） | `Nodes/OrchestratorNode.tsx` | 60行 |
| TS: 注册到 `WorkflowEditor.tsx` + `Nodes/index.ts` | 2 文件 | 5行 |
| TS: 属性面板 | `PropertyPanel/OrchestratorPropertyPanel.tsx` | 200行 |

**交付**：可拖入 Orchestrator 节点，配置 mission + strategy，运行时输出子任务分解结果。

### Phase 2 — 结构化交接 + 动态子图 + Scorecard（P0→P1，2-3天）

**目标**：Agent 间结构化交接，动态子图，Scorecard 审计

| 任务 | 预估 |
|------|------|
| Rust: `StructuredHandover` 完整类型 + `NodeOutput.handover` | 80行 |
| Rust: `DynamicSubGraph` 生成器 | 200行 |
| Rust: `ScorecardEngine` + DB 表 | 400行 |
| Rust: Tauri 命令（scorecard_report 等） | 100行 |
| Rust: AgentExecutor 自动填写 handover | 40行 |

**交付**：Agent 自动填交接协议，Orchestrator 动态生成子 DAG，Scorecard 审计通过 DB 验证。

### Phase 3 — YAML 接口 + 环境 + 前端（P1→P2，2-3天）

**目标**：YAML 导入导出 + 环境隔离 + 完整前端面板

| 任务 | 预估 |
|------|------|
| Rust: YAML 工作流导出/导入 | 80行 |
| Rust: 环境配置解析（Dockerfile/工具链） | 100行 |
| TS: Scorecard Panel | 250行 |
| TS: YAML Editor | 200行 |
| TS: 策略选择 UI | 150行 |
| 文档 | 200行 |

**总计**：~1800 行 Rust + ~1000 行 TS，分散在 ~12 个新文件

---

## 6. 关键决策点

| 决策 | 选项 | **推荐** | 理由 |
|------|------|---------|------|
| Orchestrator 节点类型 | Agent vs **容器** | **容器** | 不做编码，复用 SubGraph，与 Parallel/Swarm 同类 |
| 交接协议传递 | NodeOutput vs EventBus | **NodeOutput + handover** | 与现有数据流一致 |
| 子图生成时机 | 静态 vs 动态 | **先静态后动态** | 静态子图让用户可预配置 |
| YAML 角色 | 转换层 vs **一等公民** | **一等公民** | 与代码同版本管理 |
| Scorecard 阻断 | 硬阻断 vs 可配置 | **可配置** | 默认 blocking + 可选 non-blocking |

---

## 7. 风险

| 风险 | 等级 | 缓解 |
|------|------|------|
| Orchestrator 的 LLM 分解决策可能不准 | **高** | 结构化策略模板 + 人工审批兜底 |
| 动态子图与现有 DAG 引擎兼容性 | 中 | 从预定义子图开始 |
| Scorecard 误报 | 中 | 可调节阈值 + non-blocking 模式 |
| YAML/JSON 双表示不一致 | 中 | 始终单向转换 YAML→JSON |
| Docker 环境隔离复杂度 | 高 | P2 优先级，暂不做 Docker |

---

## 8. 成功条件

系统可用性的四个前提：

1. **有参照物** — 标准答案或已知正确实现可作对比
2. **定义好的协议** — YAML 接口稳定、交接协议格式固定
3. **端到端测试** — 每个 Worker 输出可用测试验证
4. **可回滚** — 失败的变更不影响主分支

---

*以上是完整设计方案，请评审。确认方案后我再开始逐步实现。*
