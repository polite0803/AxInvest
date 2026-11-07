> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# 能力发现（Capability Discovery）设计方案

> 版本: v1.0  
> 日期: 2026-08-13  
> 状态: 设计评审通过，进入实现阶段

---

## 一、设计目标

为 AxAgent 建立一套**通用的能力发现机制**，使智能体能够根据用户输入自动匹配最合适的工具、工作流、知识库、Agent 或技能，而无需硬编码路由逻辑。

### 核心价值

1. **零改 Prompt 扩展能力**：新增工作流/工具时只需补充元数据护照，路由代码一行不改
2. **语义级匹配**：从基于关键词的精确匹配升级到向量语义相似度匹配
3. **安全硬性闸门**：8 个维度的过滤机制，确保不会误匹配到不安全或不适用的能力
4. **主动式增强**：熔断降级、能力补全、热更新等主动式玩法

---

## 二、四层机制架构

```
用户输入
  │
  ▼
┌─────────────────────────────────────────────────────────┐
│ L1 注册层 (Registration)                                  │
│ 每个能力（工具/工作流/知识库/Agent/Skill）自带「数字护照」    │
│ 包含: 名称、入参Schema、所属域、成本、耗时、安全等级、          │
│       模态支持、规划复杂度、模型IQ要求、负面场景               │
└─────────────────────────────────────────────────────────┘
  │
  ▼
┌─────────────────────────────────────────────────────────┐
│ L2 索引层 (Indexing)                                      │
│ 护照信息向量化 + 标签化，存入本地向量库                       │
│ • 正向索引: description + tags → 语义向量                    │
│ • 负向索引: negative_scenarios → 负面向量（独立集合）         │
│ • 元数据存储: 完整 CapabilityPassportDto 作为 metadata       │
└─────────────────────────────────────────────────────────┘
  │
  ▼
┌─────────────────────────────────────────────────────────┐
│ L3 检索层 (Retrieval)                                     │
│ 混合检索: 向量相似度(语义) + BM25(关键词) + 标签硬匹配        │
│ 过滤: 负面场景排除（用户 query 与负面向量相似度 > 阈值则剔除）    │
│ 输出: Top-K 候选列表                                        │
└─────────────────────────────────────────────────────────┘
  │
  ▼
┌─────────────────────────────────────────────────────────┐
│ L4 排序与过滤层 (Ranking & Filtering)                       │
│ 8维硬性闸门 → 加权排序 → 模糊检测 → 最终命中                  │
│ 主动式增强: 熔断降级 / 能力补全 / 热更新                       │
└─────────────────────────────────────────────────────────┘
  │
  ▼
最终命中的能力
```

---

## 三、L4 排序层 — 8 维过滤机制

### 维度清单

| # | 维度 | 类型 | 说明 |
|---|------|------|------|
| 1 | 置信度 | 加权 | Top1 vs Top2 分差 < 0.1 时触发「模糊发现」 |
| 2 | 记忆/状态 | 加权 | 历史使用过的能力 +15% 权重（个性化提权） |
| 3 | 模态 | 闸门 | 图片+文字输入时剔除不支持多模态的工具 |
| 4 | 安全/合规 | 闸门 | 检测到 PII 时剔除未加密/无审计的能力 |
| 5 | 资源/成本 | 闸门 | 单次调用成本 >$0.05 时硬性剔除 |
| 6 | 交互策略 | 闸门 | 无屏设备剔除需要表格/图表输出的能力 |
| 7 | 规划复杂度 | 闸门 | SIMPLE 任务剔除复杂多步工作流 |
| 8 | 实验/灰度 | 闸门 | B 组用户仅检索 B 组能力 |

### 最终适配分公式

```
final_score = α × semantic + β × history − γ × (1 − speed) − δ × (1 − cost)
              + personalization_boost + exploration_boost

其中:
  semantic = 语义相似度 (0-1)
  history  = 历史成功率 (0-1)
  speed    = 耗时归一化分 (30s内=1.0, 120s以上=0.0)
  cost     = 成本归一化分 ($0.01内=1.0, $0.10以上=0.0)
  personalization_boost = 历史使用提权 (默认 +0.15)
  exploration_boost    = 冷启动探索提权 (默认 +0.20, 前100次调用内)

  α/β/γ/δ 由用户偏好动态调整 (默认: 0.4/0.3/0.15/0.15)
```

---

## 四、主动式新玩法

### 4.1 能力补全 (Proactive Suggestion)

场景: 用户问「怎么退货」，匹配到「退货工作流」，但系统发现订单状态是「已签收」，于是额外推荐「上门取件」能力。

实现: `CapabilityCompleter` trait，基于主命中能力 + 用户上下文 + 实体关联规则生成补全建议。

### 4.2 能力熔断 (Circuit Breaker)

场景: 「退款工作流」最近 5 次执行失败率 100%，即便语义匹配 Top1，强制降级为 Top2（如「转人工」）。

实现: `CapabilityCircuitBreaker` trait，订阅 `ToolUsageStats`，基于失败率动态调整可用性。

### 4.3 能力热更新 (Hot Swap)

场景: 新上线的工作流带「new」标签，前 100 次调用给予探索提权（+20%），每 10 秒刷新向量索引。

实现: `CapabilityHotSwapper` trait，后台定时刷新 + 冷启动探索机制。

---

## 五、接口设计

### 5.1 文件结构

```
harness/src/
├── capability.rs           ← L1: CapabilityPassport trait + 数据类型
├── capability_indexer.rs   ← L2: CapabilityIndexer trait
├── capability_retriever.rs ← L3: CapabilityRetriever trait
├── capability_filter.rs    ← L4a: CapabilityFilter (8维闸门)
├── capability_ranker.rs    ← L4b: CapabilityRanker (加权排序)
├── capability_circuit.rs   ← 主动式: 熔断/补全/热更新
└── capability_router.rs   ← 编排层: 完整管线
```

### 5.2 核心 trait

| Trait | 文件 | 职责 | 实现层 |
|-------|------|------|--------|
| `CapabilityPassport` | capability.rs | 统一元数据契约 | tools (实现) |
| `CapabilityIndexer` | capability_indexer.rs | 向量化 + 标签化存储 | tools |
| `CapabilityRetriever` | capability_retriever.rs | 混合检索引擎 | tools |
| `CapabilityFilter` | capability_filter.rs | 8 维硬性闸门 | tools |
| `CapabilityRanker` | capability_ranker.rs | 加权排序 + 模糊检测 | tools |
| `CapabilityCircuitBreaker` | capability_circuit.rs | 熔断降级 | tools |
| `CapabilityCompleter` | capability_circuit.rs | 主动补全建议 | tools |
| `CapabilityHotSwapper` | capability_circuit.rs | 热更新 | tools |
| `CapabilityRouter` | capability_router.rs | 管线编排 | agent (消费) |

### 5.3 关键数据类型

- `CapabilityPassportDto`: 可序列化的能力护照 DTO，用于索引存储和前端展示
- `CapabilityQuery`: 检索请求参数（kind/domain/modality/tag 过滤）
- `FilterContext`: 过滤上下文（模态/PII/预算/设备/复杂度/实验组）
- `CapabilityCandidate`: 检索候选（含 semantic/keyword/tag/retrieval 四重分）
- `RankedCapability`: 排序结果（含最终分 + 提权 + 命中原因）
- `CapabilityDiscoveryResult`: 最终发现结果（主命中 + 备选 + 建议 + 熔断信息）

### 5.4 依赖方向（符合 Harness 架构铁律）

```
harness (foundation) → 零 axagent-* 依赖
  ↓
tools (implementor) → harness + entities
  ↓
agent (consumer) → harness only（通过 Arc<dyn CapabilityRouter> 消费）
```

---

## 六、实施计划

### Phase 1: 接口定义 + 数据类型（当前阶段）

- [x] 创建 `harness/src/capability.rs` — `CapabilityPassport` trait + 所有数据类型
- [x] 创建 `harness/src/capability_indexer.rs` — `CapabilityIndexer` trait
- [x] 创建 `harness/src/capability_retriever.rs` — `CapabilityRetriever` trait
- [x] 创建 `harness/src/capability_filter.rs` — `CapabilityFilter` trait + 8 维闸门
- [x] 创建 `harness/src/capability_ranker.rs` — `CapabilityRanker` trait + 加权公式
- [x] 创建 `harness/src/capability_circuit.rs` — 熔断/补全/热更新三个 trait
- [x] 创建 `harness/src/capability_router.rs` — `CapabilityRouter` trait + 默认实现
- [x] 在 `lib.rs` 中注册所有新模块
- [ ] `cargo check` 验证类型正确

### Phase 2: 索引层实现

- 实现 `CapabilityIndexer`（复用 `VectorStoreProvider` + `EmbeddingProvider`）
- 创建正向索引集合 `capabilities` 和负向索引集合 `capabilities_negative`
- 实现批量索引/删除/清空操作

### Phase 3: 检索层实现

- 实现 `CapabilityRetriever`（复用 `RAGProvider::hybrid_search`）
- 实现标签硬匹配过滤
- 实现负面场景排除逻辑

### Phase 4: 排序与过滤层实现

- 实现 8 维过滤闸门（`CapabilityFilter`）
- 实现加权排序器（`CapabilityRanker`）
- 实现模糊发现检测

### Phase 5: 主动式玩法实现

- 实现能力熔断器
- 实现能力补全器
- 实现热更新后台任务

### Phase 6: 集成与迁移

- `ToolInfo` / `WorkflowTemplate` / 其他实体实现 `CapabilityPassport`
- `DefaultCapabilityRouter` 接入主流程
- 前端消费 `CapabilityDiscoveryResult`
- 现有 `ToolRecommender` 降级为开发辅助工具

---

## 七、与现有系统的关系

| 现有组件 | 关系 |
|---------|------|
| `ToolInfo` (harness/tool.rs) | 实现 `CapabilityPassport`，扩展元数据 |
| `WorkflowTemplate` (entities) | 实现 `CapabilityPassport`，扩展元数据 |
| `VectorStoreProvider` (harness/rag_provider.rs) | 被 `CapabilityIndexer` 复用 |
| `RAGProvider::hybrid_search` | 被 `CapabilityRetriever` 复用 |
| `ToolUsageStats` (tools/stats.rs) | 被熔断器和排序器读取 |
| `CircuitBreaker` (harness/circuit_breaker.rs) | 被 `CapabilityCircuitBreaker` 扩展 |
| `ToolRecommender` (agent/tool_recommender/) | Phase 6 降级为开发辅助工具 |
| `ReasoningRouter` (agent/reasoning_router.rs) | Phase 6 被 `CapabilityRouter` 替代 |

---

## 八、设计亮点

1. **负面场景索引**: 独立向量集合，防止误匹配，业界首创
2. **硬性闸门 + 加权分离**: 安全约束不会被高分抵消
3. **动态 α/β/γ/δ**: 为个性化 RL 调参预留接口
4. **零侵入扩展**: 新增能力只需补充元数据，不改路由代码
5. **渐进式集成**: 与现有系统并行运行，逐步替换
