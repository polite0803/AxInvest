> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# WorkflowOptimizer 算法方案

> 版本：v1.0  ·  状态：Phase 1 已完成，Phase 2 进行中
> 所属模块：`axagent-trajectory / workflow_adapters / optimizer`
> Harness 契约：`axagent-harness / workflow_optimization.rs` + `workflow_reflection.rs`

---

## 1. 设计目标

`WorkflowOptimizer` 是 trajectory crate 的核心能力之一，与 `WorkflowReflector`、`WorkflowEvolver` 共同构成"反思-优化-进化"三层闭环。

它的职责：

1. **输入**：一次或多次 workflow 执行后的 `Reflection`（包含 `WorkflowReflectionMetadata`、节点耗时统计、瓶颈分析、失败分类等）。
2. **分析**：识别工作流模板的结构性/配置性/提示词级缺陷。
3. **输出**：一组 `WorkflowSuggestion`（可执行的优化建议），供上层（Agent 自驱动、前端展示、Evolver 自动应用）消费。
4. **不修改模板本体**——优化是"建议驱动"的，`apply_suggestions` 才真正把建议写回模板。

---

## 2. Harness 契约（关键类型）

### 2.1 WorkflowSuggestion

```rust
pub struct WorkflowSuggestion {
    pub id: String,
    pub category: SuggestionCategory,
    pub priority: SuggestionPriority,
    pub target_node_id: Option<String>,
    pub description: String,
    pub proposed_change: ProposedChange,
    pub confidence: f32,           // 0.0 - 1.0
    pub estimated_impact: Option<f32>, // 0.0 - 1.0
}
```

### 2.2 SuggestionCategory（7 种）

| 枚举值 | 说明 |
|---|---|
| `NodeConfig` | 节点配置修改（非 retry/timeout 类） |
| `NodeReplacement` | 节点类型替换 |
| `EdgeRewire` | 边重连 |
| `PromptRefine` | Agent prompt 精炼/追加 |
| `ErrorHandling` | 错误处理增强（try/catch） |
| `VariableMisconfig` | 变量配置错误 |
| `ResourceTuning` | retry、timeout、并发度等资源/时序调优 |

### 2.3 SuggestionPriority

`Critical > High > Medium > Low`

### 2.4 ProposedChange（8 种）

```rust
pub enum ProposedChange {
    UpdateConfig { node_id, patch },           // ✅ Phase 1 已实现生成 + 应用
    ReplaceNode { node_id, new_type, new_config }, // Phase 2
    AddNode { after, node },                   // Phase 2
    RemoveNode { node_id },                    // ✅ Phase 1 已实现
    RewireEdge { from, to, new_target },       // ✅ Phase 1 已实现
    RefinePrompt { node_id, new_prompt },      // ✅ Phase 1 已实现生成，Phase 2 补 apply
    TuneRetry { node_id, max_attempts, backoff_ms }, // ✅ Phase 1 已实现
}
```

### 2.5 BottleneckReason（瓶颈分类）

来自 `workflow_reflection::BottleneckReason`，Phase 1 实现了 `HighLatency`，Phase 2 扩展：

| 枚举值 | 说明 | Phase |
|---|---|---|
| `HighLatency` | 节点平均耗时超过阈值 | 1 ✅ |
| `HighErrorRate` | 节点失败率超过阈值 | 2 |
| `ToolFailure` | 工具调用失败模式 | 2 |
| `LLMQuality` | LLM 输出质量低导致重试 | 2 |
| `ContextOverflow` | 上下文溢出/截断 | 3 |
| `RedundantComputation` | 重复计算/可缓存 | 3 |

---

## 3. 核心算法

### 3.1 BottleneckDetector

**单次 reflection → 单条/多条瓶颈建议**。

流程：

```
Reflection.metadata.bottlenecks
    └─ 每个 Bottleneck { node_id, reason, impact_score, detail }
        └─ match reason:
            HighLatency   → ProposedChange::UpdateConfig  ← retry + timeout
            HighErrorRate → ProposedChange::TuneRetry
            ToolFailure   → ProposedChange::ErrorHandling
            LLMQuality   → ProposedChange::RefinePrompt
            ...
```

Phase 1 HighLatency 生成的 patch（非占位符，真正可应用）：

```json
{
  "base": {
    "retry": {
      "enabled": true,
      "maxRetries": 2,
      "backoffType": "exponential",
      "baseDelayMs": 1000,
      "maxDelayMs": 30000
    },
    "timeout": 120000
  }
}
```

### 3.2 PromptRefiner

**跨多次 reflection → 一条 prompt 精炼建议**。

触发条件（冷启动保护）：`正例数量 (quality_score ≥ 7) ≥ 2`

流程：

```
reflections[]
    ├─ 正例 = { r | r.quality_score ≥ 7 }
    ├─ 反例 = { r | r.quality_score < 7 }
    ├─ 统计正例 reusable_patterns + overall_summary → HashMap<keyword, freq>
    ├─ 统计反例 error_patterns + overall_summary → HashMap<keyword, freq>
    ├─ 过滤 freq ≥ 2，正例 top-5，反例 top-3
    └─ 生成 RefinePrompt {
          new_prompt = "\n\n# 历史成功经验\n\n根据历史 N 次成功执行(≥7分),特别关注:kw1、kw2...\n避免常见失误:err1、err2..."
       }
```

置信度公式：

```
confidence = clamp(0.4 + positive_count * 0.05 + avg_top_freq * 0.02, 0.4, 0.85)
```

### 3.3 suggest_batch 去重

同 `(target_node_id, category)` 只保留优先级最高的一条——避免多次 reflection 产生重复建议。

---

## 4. 建议应用（apply_one 分支覆盖矩阵）

| ProposedChange | 生成 | 应用 | 实现位置 |
|---|---|---|---|
| UpdateConfig | ✅ Phase 1 | ✅ Phase 1 `apply_update_config` | `optimizer.rs` |
| RemoveNode | ✅ Phase 0 | ✅ Phase 1 | `optimizer.rs` |
| RewireEdge | ✅ Phase 0 | ✅ Phase 1 | `optimizer.rs` |
| TuneRetry | ✅ Phase 1 | ✅ Phase 1（同 UpdateConfig 路径） | `optimizer.rs` |
| RefinePrompt | ✅ Phase 1 | **Phase 2** | 待实现 |
| AddNode | Phase 2 | **Phase 2** | 待实现 |
| ReplaceNode | Phase 2 | **Phase 2** | 待实现 |

### 4.1 apply_update_config（Phase 1 已实现）

解析 `patch.base` 下的三个字段写回目标节点的 `NodeBase`：

- `base.retry` → `serde_json::from_value::<RetryConfig>` → `node.base.retry`
- `base.timeout`（u64）→ `node.base.timeout = Some(timeout)`
- `base.enabled`（bool）→ `node.base.enabled`

Phase 2 扩展字段：`base.concurrency`、`base.input_mapping`、`base.output_mapping`。

### 4.2 apply_refine_prompt（Phase 2 待实现）

```
遍历 template.nodes 找到 target_node_id
    └─ match node:
        WorkflowNode::Agent(agent) → agent.system_prompt.push_str(new_prompt)
        其他 NodeType               → warn + skip
```

追加格式：`\n\n# 历史成功经验\n\n` + `new_prompt`，保证和现有 prompt 之间有清晰分隔。

### 4.3 apply_add_node（Phase 2 待实现）

```
ProposedChange::AddNode { after, node }
    ├─ 反序列化 node JSON → WorkflowNode
    ├─ 插入 template.nodes（after 节点之后）
    └─ 插入边：
         - 原 after → next 边 改为 after → new_node
         - 新增 new_node → next 边
```

### 4.4 apply_replace_node（Phase 2 待实现）

```
ProposedChange::ReplaceNode { node_id, new_type, new_config }
    ├─ 反序列化 new_config + new_type → WorkflowNode
    └─ 替换 template.nodes 中匹配 node_id 的节点（保留 node_id 和边连接）
```

### 4.5 apply_suggestions 版本号递增

Phase 1 已实现：`template.version += 1` + `updated_at = now()`。

---

## 5. 目录架构

```
crates/harness/src/
├── workflow_optimization.rs    # WorkflowOptimizer trait + 类型契约
└── workflow_reflection.rs       # Reflection + BottleneckReason + FailureCategory

crates/trajectory/src/workflow_adapters/
├── mod.rs                       # 模块声明 + pub re-export
├── optimizer.rs                 # WorkflowOptimizerImpl（Phase 1/2 核心实现）
├── reflector.rs                 # WorkflowReflectorImpl
└── evolver.rs                   # WorkflowEvolverImpl
```

依赖层级：

```
axagent-trajectory  →  axagent-harness（consumer，唯一依赖）
axagent-harness     →  零 axagent-* 依赖（foundation）
```

**铁律遵守**：Optimizer 不越过 harness 直接依赖 providers、dao、entities、storage 等实现层 crate。

---

## 6. 实施路线图

### Phase 1 ✅ 已完成

- [x] BottleneckDetector HighLatency → UpdateConfig（真正 retry + timeout patch）
- [x] apply_one UpdateConfig 分支（解析 JSON patch 写回 RetryConfig / timeout / enabled）
- [x] PromptRefiner（正例/反例高频词提取，RefinePrompt 建议生成）
- [x] suggest_batch 去重
- [x] apply_suggestions 版本号 +1
- [x] cargo check ✅ | clippy ✅ | fmt ✅ | 341 tests passed ✅

### Phase 2 进行中

- [ ] RefinePrompt 的 apply_one 分支（修改 Agent 节点 system_prompt，追加 `\n\n# 历史成功经验`）
- [ ] AddNode 分支（构造 WorkflowNode + 插入 + 边重连）
- [ ] ReplaceNode 分支（替换节点类型/配置，保留 node_id 和边）
- [ ] BottleneckReason 扩展：HighErrorRate / ToolFailure / LLMQuality
- [ ] cargo check + clippy + 测试

### Phase 3 规划

- [ ] ContextOverflow / RedundantComputation 场景
- [ ] apply_update_config 扩展：concurrency / input_mapping / output_mapping
- [ ] RefinePrompt 的"清理"机制（同模板多次 apply 时去重历史经验段落）
- [ ] 与前端集成：WorkflowOptimizer 建议通过 Tauri 命令暴露给前端展示

---

## 7. 质量门禁（每 Phase 必过）

| 检查项 | 命令 | 阻塞 |
|---|---|---|
| 类型检查 | `cargo check` | ✅ |
| Lint | `cargo clippy -- -D warnings` | ✅ |
| 格式化 | `cargo fmt --check` | ✅ |
| 单元测试 | `__TAURI_WORKSPACE__=true cargo test -p axagent-trajectory --lib` | ✅ |
| 全量 check | `cargo check`（workspace 根） | Phase 末必过 |

---

## 8. 设计决策记录

### 8.1 为什么用 JSON patch 而不是 Typed DTO？

**UpdateConfig 的 patch 是开放结构**——Phase 1 只处理 `base.retry / base.timeout / base.enabled`，Phase 2 要扩展 `concurrency / input_mapping / output_mapping`，Phase 3 可能有更多。如果在 harness 定义全量 Typed DTO，每加一个字段都要改三个 crate 的类型，不符合开放-封闭原则。

选择：patch 用 `serde_json::Value`，`apply_one` 里按字段名分支解析。新增字段只改 trajectory 一个 crate。

### 8.2 PromptRefiner 为什么用停用词表而不是 TF-IDF？

工程复杂度 vs 收益平衡。PromptRefiner 的输入是 `reusable_patterns`（已经是 LLM 提取的结构化模式）+ `overall_summary`（自由文本）。在这个场景下：

- 停用词 + 词频统计的有效率 ≈ TF-IDF（因为正例文本高度同质）
- 实现量 50 行 vs 引入 tokenizer crate（数百 KB 依赖）
- 正例 ≥ 2 的冷启动保护已经把"随机噪声"降到极低

如果 Phase 3 发现精度不够，再考虑引入 `rapidfuzz` 或 `jieba`。

### 8.3 去重为什么用线性扫描不用 HashSet？

`SuggestionCategory`（Phase 1）没有实现 `Hash` trait，且建议数量通常 ≤ 10 条。线性扫描复杂度 O(n²)，n≤10 时完全可忽略。如果未来建议量上升到 50+，给 `SuggestionCategory` 加 `#[derive(Hash)]` 是两行代码的事。
