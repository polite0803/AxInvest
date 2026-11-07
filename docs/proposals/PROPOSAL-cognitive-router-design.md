> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# 认知编排器（Cognitive Router）设计方案

> **版本**: v0.2 (Draft)  
> **日期**: 2026-08-13  
> **状态**: 设计阶段（V2 — 分层路由架构）

---

## 0. 核心架构：三层分层路由体系

### 0.1 设计总览

```
用户输入
    │
    ▼
┌─────────────────────────────────────────────────────────────────┐
│  第一层：分层路由树（L1 → L2 → L3）                              │
│  ─────────────────────────────────────────                       │
│  L1 域层（约10个）          L2 能力簇层（每域约20个）   L3 具体工作流 │
│  ┌──────────────────┐      ┌─────────────────────┐     ┌──────────┐│
│  │ 交易域            │─────▶│  订单修改            │────▶│  改单价   ││
│  │ 物流域            │      │  退款处理            │     │  改数量   ││
│  │ 营销域            │      │  发票管理            │     │  改地址   ││
│  │ 客服域            │      │  ...                 │     │  ...     ││
│  │ 财务域            │      └─────────────────────┘     └──────────┘│
│  │ ... (共10域)      │                                              │
│  └──────────────────┘                                              │
│                                                                    │
│  Prompt 只注入 L1 + L2 信息（约200节点），绝不注入 L3               │
│  单次 Prompt ≈ 几百 Token，不随总量增长                            │
└───────────────────────────┬─────────────────────────────────────┘
                            │ L2 命中
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│  第二层：检索增强路由（RAR）                                      │
│  ─────────────────────────────────────────                       │
│  1. 用户输入 → 向量检索 Top-5 相似工作流                          │
│  2. Prompt 动态注入这 5 个工作流的名称和参数                      │
│  3. LLM 必须且只能从这 5 个里选 → 5选1 选择题                    │
│                                                                    │
│  效果：2000+ 工作流 → 恒定 5 选 1，准确率 >90%                    │
└───────────────────────────┬─────────────────────────────────────┘
                            │ Top-5 候选
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│  第三层：工作流图谱（Graph of Workflows）                         │
│  ─────────────────────────────────────────                       │
│  所有工作流组织成有向无环图（DAG）                                │
│  路由 Prompt 只存邻接表（路径标识）：                             │
│  "当前节点：交易域_退款。下游：补发优惠券、转人工工单"              │
│                                                                    │
│  模型输出"路径地址"（如 /trade/refund/auto）                       │
│  而非自然语言描述 → 确定性硬编码路径                               │
└─────────────────────────────────────────────────────────────────┘
```

### 0.2 与现有基础设施的映射

| 分层方案能力 | 现有代码基础 | 差距评估 |
|-------------|------------|---------|
| L1 域层 | [capability.rs#L51-L64](file:///d:/OneManager/AxAgent/src-tauri/crates/harness/src/capability.rs#L51-L64) `CapabilityDomain` 已有 9 个域（core/devops/invest/data_analysis/content_creation 等） | 🟢 已具备：扩展域枚举即可 |
| L2 能力簇层 | [capability.rs#L248-L290](file:///d:/OneManager/AxAgent/src-tauri/crates/harness/src/capability.rs#L248-L290) `CapabilityPassport` trait 已有 `domain()` + `tags()` + `description()` | 🟡 中等：需新增 `sub_category()` 方法 |
| L3 具体工作流 | [workflow_template.rs](file:///d:/OneManager/AxAgent/src-tauri/crates/entities/src/workflow_template.rs) `workflow_templates` 表已有 `tags` 字段 | 🟢 已具备 |
| RAR 向量检索 | [capability_indexer_impl.rs](file:///d:/OneManager/AxAgent/src-tauri/crates/tools/src/capability_indexer_impl.rs#L30-L88) 已有 `VectorStore` + embedding + metadata 索引 | 🟢 已具备：扩展检索范围即可 |
| 工作流图谱 | **执行层已具备**：[SubWorkflowExecutor](file:///d:/OneManager/AxAgent/src-tauri/crates/rt-workflow/src/work_engine/executors/subworkflow_executor.rs#L60-L108) 支持子工作流嵌套执行；**分析层缺失**：现有 DAG 引擎仅能分析单个工作流内部拓扑，无法跨工作流进行全局依赖分析 | 🟡 中等：需新增**元数据层**的跨工作流 DAG 索引（用于路径规划、影响分析），但**执行层**可直接复用 SubWorkflowNode 机制 |

---

## 1. 第一层：分层路由树

### 1.1 三级数据模型扩展

```rust
/// 路由层级枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RouteLevel {
    /// L1: 域层（约10个，对应 CapabilityDomain）
    Domain,
    /// L2: 能力簇层（每域约20个）
    Cluster,
    /// L3: 具体工作流（每簇约10个）
    Workflow,
}

/// 扩展 CapabilityPassport trait，增加分层方法
pub trait CapabilityPassport: Send + Sync {
    // ... 现有方法 ...

    /// 所属 L2 能力簇（如 "refund"、"order_modify"）
    fn sub_category(&self) -> &str {
        ""
    }

    /// 父级域（L1），格式："domain:cluster"
    fn route_path(&self) -> String {
        format!("{}:{}", self.domain().as_str(), self.sub_category())
    }
}
```

### 1.2 数据库扩展

```sql
-- L2 能力簇注册表（新增）
CREATE TABLE capability_clusters (
    id TEXT PRIMARY KEY,
    domain TEXT NOT NULL,           -- L1 域："invest" | "ecommerce" | ...
    cluster_name TEXT NOT NULL,      -- L2 簇："refund" | "order_modify" | ...
    display_name TEXT,
    description TEXT,
    tags TEXT,                       -- JSON: ["退款", "售后"]
    priority INTEGER DEFAULT 0,
    is_active INTEGER DEFAULT 1,
    created_at INTEGER,
    updated_at INTEGER,
    UNIQUE(domain, cluster_name)
);

-- 工作流模板表扩展（workflow_templates 新增字段）
ALTER TABLE workflow_templates ADD COLUMN cluster_id TEXT;
ALTER TABLE workflow_templates ADD COLUMN route_path TEXT;  -- 冗余索引："invest:stock_analysis"

-- 路由规则表（支持三级优先级）
CREATE TABLE routing_rules (
    id TEXT PRIMARY KEY,
    rule_name TEXT NOT NULL,
    level TEXT NOT NULL,            -- "L1" | "L2" | "L3"
    rule_type TEXT NOT NULL,         -- "exact" | "regex" | "keyword" | "rhai"
    pattern TEXT NOT NULL,
    target_path TEXT NOT NULL,       -- "invest:stock_analysis" | "invest"
    priority INTEGER DEFAULT 0,
    is_active INTEGER DEFAULT 1,
    created_at INTEGER,
    updated_at INTEGER
);
```

### 1.3 路由 Prompt 策略

**L1 路由 Prompt（域层选择）**：

```
你是一个业务路由器。根据用户输入判断其所属的业务域。

## 可用业务域列表
1. invest — 金融投资：股票分析、基金推荐、风险评估
2. ecommerce — 电商运营：订单管理、退款处理、营销推广
3. content_creation — 内容创作：小说写作、文章生成、翻译
4. data_analysis — 数据分析：报表生成、数据可视化、趋势预测
5. customer_service — 客服支持：工单处理、FAQ 查询、满意度调查
... (共约 10 个域)

## 用户输入
{{user_input}}

## 输出格式
{
  "domain": "invest",
  "confidence": 0.95,
  "reasoning": "用户提到'分析301302股票'，属于金融投资域"
}
```

**L2 路由 Prompt（簇层选择，注入该域下约 20 个簇）**：

```
你是一个业务路由器。在"invest"域下选择最合适的能力簇。

## invest 域下的可用能力簇
1. stock_analysis — 股票技术/基本面分析
2. fund_recommendation — 基金推荐与对比
3. risk_assessment — 投资风险评估
4. market_sentiment — 市场情绪分析
5. portfolio_optimization — 投资组合优化
... (共约 20 个簇)

## 用户输入
{{user_input}}

## 输出格式
{
  "cluster": "stock_analysis",
  "confidence": 0.92,
  "extracted_params": {
    "stock_code": "301302"
  }
}
```

**关键约束**：Prompt 中**绝不**注入 L3 具体工作流列表，避免 token 膨胀。

### 1.4 子工作流嵌入策略

现有引擎支持两种方式组织子工作流，各有适用场景：

#### 方式 1：子工作流嵌入（推荐）

使用 `SubWorkflowNode` 节点将子工作流**直接嵌入**主路由工作流中：

```typescript
// SubWorkflowNodeConfig 使用示例
{
  node_id: "security_gateway",
  type: "subWorkflow",
  config: {
    sub_workflow_id: "wf_security_gateway",  // 引用子工作流
    input_mapping: {                          // 父→子 变量映射
      "user_input": "input",
      "conversation_id": "conv_id"
    },
    output_var: "security_result",           // 子工作流输出变量名
    is_async: false,                          // 同步执行（阻塞主流程）
    subGraph: {                               // 可选：子图定义
      nodes: [...],
      edges: [...]
    }
  }
}
```

**优势**：
- 编辑器内直接展开子图，可视化调试
- 变量映射清晰，父工作流变量 → 子工作流入参
- 支持同步/异步执行
- 子工作流可独立版本管理

**适用场景**：
- 安全网关、快速匹配等需要嵌入主流程的子模块
- 需要在编辑器中可视化调试的复杂逻辑

#### 方式 2：独立工作流模板

将子工作流注册为独立模板，通过 `sub_workflow_id` 引用：

```rust
// 后端通过 SubWorkflowCallback 执行子工作流
let result = cb(
    sub_workflow_id.to_string(),
    parent_execution_id.to_string(),
    input_mapping,  // 映射后的变量
).await?;
```

**优势**：
- 子工作流可被多个父工作流复用
- 独立部署、版本管理
- 支持跨项目共享

**适用场景**：
- 通用能力（如发送邮件、数据格式化）
- 需要跨多个路由场景复用的模块

#### 推荐组合

路由编排器采用**混合策略**：

```
cognitive_router (主工作流)
├── subWorkflow: security_gateway     // 嵌入方式（需要可视化调试）
├── subWorkflow: fast_matcher         // 嵌入方式（需要快速迭代）
├── subWorkflow: intelligent_analyzer // 嵌入方式（复杂 LLM 逻辑）
├── subWorkflow: decision_maker       // 嵌入方式
└── subWorkflow: resource_scheduler   // 嵌入方式
    │
    └── 内部引用独立工作流模板（如 RAR 检索器、图谱查询器）
```

---

## 2. 第二层：检索增强路由（RAR）

### 2.1 架构设计

```
用户输入 "请分析301302这支股票"
    │
    ▼
┌─────────────────────────────────────────────────────────┐
│  Step 1: 向量检索                                        │
│  ──────────────                                         │
│  输入 embedding → VectorStore.search(                   │
│      collection: "workflow_capabilities",               │
│      filter: { domain: "invest", cluster: "stock_analysis" }, │
│      top_k: 5                                           │
│  )                                                      │
│                                                         │
│  返回 Top-5 候选工作流：                                 │
│  [wf_stock_tech, wf_stock_fundamental, wf_stock_news,    │
│   wf_stock_risk, wf_stock_competitor]                   │
└───────────────────────────┬─────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────┐
│  Step 2: 动态 Few-shot 注入                              │
│  ────────────────────────                               │
│  Prompt 中只注入这 5 个工作流：                          │
│                                                         │
│  ## 候选项（必须且只能从以下中选择一个）                   │
│  1. wf_stock_tech — 技术面分析（K线、均线、MACD）         │
│     输入参数：stock_code, period                         │
│  2. wf_stock_fundamental — 基本面分析（PE、ROE、业绩）   │
│     输入参数：stock_code, quarter                        │
│  3. wf_stock_news — 舆情分析（新闻、公告、研报）          │
│     输入参数：stock_code, time_range                     │
│  4. wf_stock_risk — 风险评估（波动率、最大回撤）         │
│     输入参数：stock_code, risk_level                     │
│  5. wf_stock_competitor — 同行对比分析                   │
│     输入参数：stock_code, peer_code                      │
│                                                         │
│  ## 用户输入                                             │
│  {{user_input}}                                         │
│                                                         │
│  要求：必须且只能从上述 5 个选项中选择一个最匹配的工作流。 │
└───────────────────────────┬─────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────┐
│  Step 3: LLM 路由决策                                    │
│  ─────────────────                                       │
│  LLM 输出：                                             │
│  {                                                      │
│    "workflow_id": "wf_stock_tech",                       │
│    "confidence": 0.96,                                   │
│    "extracted_params": {                                 │
│      "stock_code": "301302",                             │
│      "period": "2024"                                    │
│    }                                                    │
│  }                                                      │
└─────────────────────────────────────────────────────────┘
```

### 2.2 现有代码复用

| RAR 步骤 | 现有实现 | 代码位置 |
|---------|---------|---------|
| 向量生成 | `EmbeddingProvider` | [capability_indexer_impl.rs#L74-L88](file:///d:/OneManager/AxAgent/src-tauri/crates/tools/src/capability_indexer_impl.rs#L74-L88) |
| 向量检索 | `VectorStore.search_with_filter` | [hybrid_search.rs#L142-L179](file:///d:/OneManager/AxAgent/src-tauri/crates/search/src/hybrid_search.rs#L142-L179) |
| 元数据过滤 | `CapabilityIndexerImpl.metadata` | [capability_indexer_impl.rs#L30-L36](file:///d:/OneManager/AxAgent/src-tauri/crates/tools/src/capability_indexer_impl.rs#L30-L36) |
| 混合检索 | `hybrid_search_with_filter` | [hybrid_search.rs#L142-L179](file:///d:/OneManager/AxAgent/src-tauri/crates/search/src/hybrid_search.rs#L142-L179) |

### 2.3 RAR 检索实现

```rust
/// RAR 检索器：面向路由场景的 Top-K 检索
pub struct RarRouter {
    vector_store: Arc<VectorStore>,
    capability_indexer: Arc<CapabilityIndexerImpl>,
    embedding_provider: Arc<dyn EmbeddingProvider>,
}

impl RarRouter {
    /// 在指定域+簇下检索 Top-K 工作流
    pub async fn search_top_k(
        &self,
        user_input: &str,
        domain: &str,
        cluster: Option<&str>,
        top_k: usize,
    ) -> Result<Vec<RarCandidate>, RarError> {
        // 1. 生成用户输入的 embedding
        let input_embedding = self.embedding_provider
            .embed(user_input)
            .await?;

        // 2. 构建过滤条件（基于 L1/L2 层级）
        let mut filter = serde_json::Map::new();
        filter.insert("domain".into(), json!(domain));
        if let Some(c) = cluster {
            filter.insert("cluster".into(), json!(c));
        }

        // 3. 向量检索
        let results = self.vector_store
            .search_with_filter(
                &input_embedding,
                top_k as u64,
                Some(filter),
                None,
            )
            .await?;

        // 4. 映射为候选列表
        let candidates = results
            .into_iter()
            .filter_map(|r| {
                let passport = self.capability_indexer
                    .get_metadata(&r.document_id)?;
                Some(RarCandidate {
                    workflow_id: r.document_id,
                    name: passport.name,
                    description: passport.description,
                    input_schema: passport.input_schema,
                    tags: passport.tags,
                    score: r.score,
                })
            })
            .collect();

        Ok(candidates)
    }
}

#[derive(Debug, Serialize)]
pub struct RarCandidate {
    pub workflow_id: String,
    pub name: String,
    pub description: String,
    pub input_schema: Option<serde_json::Value>,
    pub tags: Vec<String>,
    pub score: f64,
}
```

### 2.4 Prompt 模板生成

```rust
/// 生成 RAR 动态注入的 Prompt
pub fn build_rar_prompt(
    candidates: &[RarCandidate],
    user_input: &str,
) -> String {
    let mut prompt = String::new();

    prompt.push_str("你是一个工作流路由器。从以下候选项中选择最合适的一个。\n\n");
    prompt.push_str("## 候选项（必须且只能选择一个）\n");

    for (i, c) in candidates.iter().enumerate() {
        let idx = i + 1;
        prompt.push_str(&format!("{}. {} — {}\n", idx, c.name, c.description));
        if let Some(schema) = &c.input_schema {
            if let Some(props) = schema.get("properties") {
                let params: Vec<String> = props.as_object()
                    .map(|m| m.keys().map(|k| k.as_str()).collect())
                    .unwrap_or_default();
                prompt.push_str(&format!("   输入参数：{}\n", params.join(", ")));
            }
        }
    }

    prompt.push_str("\n## 用户输入\n");
    prompt.push_str(user_input);
    prompt.push_str("\n\n要求：必须且只能从上述候选项中选择。输出 JSON 格式。");

    prompt
}
```

---

## 3. 第三层：工作流图谱（Graph of Workflows）

### 3.0 关键概念：执行层 vs 分析层

现有引擎已具备**执行层**的子工作流嵌套能力，但本章节的图谱聚焦于**分析层**的元数据索引：

| 维度 | 执行层（现有能力） | 分析层（本章节新增） |
|------|-------------------|---------------------|
| **目标** | 运行时嵌套执行子工作流 | 设计时全局路径规划与影响分析 |
| **机制** | `SubWorkflowNode` + `SubWorkflowExecutor` | 元数据图谱 + 邻接表索引 |
| **示例** | 主工作流调用安全网关子工作流 | 路由时从"交易域"定位到"退款处理"路径 |
| **生命周期** | 运行时创建，执行完销毁 | 持久化存储，工作流变更时同步更新 |

**核心价值**：图谱让路由模型输出**确定性路径地址**（如 `/trade/refund/auto`），而非自然语言描述，大幅降低 LLM 记忆负担和错判概率。

### 3.1 DAG 数据模型

```rust
/// 工作流图谱节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowGraphNode {
    /// 路径标识（确定性 ID）
    pub path: String,            /// 如 "invest/stock_analysis"
    pub display_name: String,
    pub level: RouteLevel,       /// L1 | L2 | L3
    pub domain: String,
    pub cluster: Option<String>,
    pub workflow_id: Option<String>,  /// L3 节点关联实际工作流
}

/// 工作流图谱边
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowGraphEdge {
    pub from: String,   /// 源节点 path
    pub to: String,     /// 目标节点 path
    pub edge_type: EdgeType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EdgeType {
    /// 层级关系：域 → 簇 → 工作流
    Hierarchy,
    /// 跳转关系：退款 → 补发优惠券
    Transition,
    /// 兜底关系：失败 → 转人工
    Fallback,
}

/// 工作流图谱
pub struct WorkflowGraph {
    nodes: HashMap<String, WorkflowGraphNode>,
    edges: Vec<WorkflowGraphEdge>,
    adjacency: HashMap<String, Vec<String>>,  /// 邻接表
}

impl WorkflowGraph {
    /// 获取指定节点的下游可达节点
    pub fn get_neighbors(&self, path: &str) -> Vec<&WorkflowGraphNode> {
        self.adjacency
            .get(path)
            .map(|neighbors| {
                neighbors.iter().filter_map(|p| self.nodes.get(p)).collect()
            })
            .unwrap_or_default()
    }

    /// 生成 LLM 友好的图谱摘要（仅邻接表）
    pub fn to_adjacency_summary(&self, current_path: &str) -> String {
        let neighbors = self.get_neighbors(current_path);
        if neighbors.is_empty() {
            return format!("当前节点：{}。无下游可达节点。", current_path);
        }

        let neighbor_list: Vec<String> = neighbors
            .iter()
            .map(|n| n.display_name.clone())
            .collect();

        format!(
            "当前节点：{}。下游可达节点：{}。",
            current_path,
            neighbor_list.join("、")
        )
    }
}
```

### 3.2 图谱数据库

```sql
-- 工作流图谱节点表
CREATE TABLE workflow_graph_nodes (
    path TEXT PRIMARY KEY,            -- "invest/stock_analysis"
    display_name TEXT NOT NULL,
    level TEXT NOT NULL,              -- "L1" | "L2" | "L3"
    domain TEXT NOT NULL,
    cluster TEXT,
    workflow_id TEXT,
    is_active INTEGER DEFAULT 1,
    created_at INTEGER,
    updated_at INTEGER
);

-- 工作流图谱边表
CREATE TABLE workflow_graph_edges (
    id TEXT PRIMARY KEY,
    from_path TEXT NOT NULL,
    to_path TEXT NOT NULL,
    edge_type TEXT NOT NULL,           -- "hierarchy" | "transition" | "fallback"
    priority INTEGER DEFAULT 0,
    created_at INTEGER,
    FOREIGN KEY (from_path) REFERENCES workflow_graph_nodes(path),
    FOREIGN KEY (to_path) REFERENCES workflow_graph_nodes(path)
);

CREATE INDEX idx_graph_from ON workflow_graph_edges(from_path);
CREATE INDEX idx_graph_to ON workflow_graph_edges(to_path);
```

### 3.3 图谱路由 Prompt

**关键设计**：路由 Prompt 只存邻接表，不存自然语言描述。模型输出路径地址而非文字。

```
你是一个工作流路由器。根据当前节点和下游可达节点，选择最佳路径。

## 当前节点
invest/stock_analysis

## 下游可达节点
1. invest/stock_analysis/tech — 技术面分析
2. invest/stock_analysis/fundamental — 基本面分析
3. invest/stock_analysis/news — 舆情分析
4. invest/stock_analysis/risk — 风险评估
5. invest/stock_analysis/competitor — 同行对比

## 用户输入
请分析301302的技术面走势

## 输出格式（只能输出路径地址）
{
  "path": "invest/stock_analysis/tech",
  "confidence": 0.98
}
```

### 3.4 图谱维护流程

```
工作流创建/更新
    │
    ▼
┌─────────────────────┐
│  图谱同步器          │
│  (新增/更新节点)     │
└─────────┬───────────┘
          │
          ▼
┌─────────────────────┐
│  边关系自动推导       │
│  ─────────────      │
│  1. L1→L2: 域下    │
│     所有簇自动关联   │
│  2. L2→L3: 簇下    │
│     所有工作流关联   │
│  3. 手动边: 工作流  │
│     创建者指定跳转关系│
└─────────────────────┘
```

---

## 4. 三层协作流程

### 4.1 完整路由决策流程

```
用户输入: "请分析301302这支股票的技术面"
    │
    ▼
┌─────────────────────────────────────────────────────────────────┐
│  Layer 1: 分层路由树                                             │
│  ────────────────────                                            │
│                                                                   │
│  Step 1.1: L1 域路由                                              │
│  ├── 规则匹配: 含"股票" → 命中 CapabilityDomain::Invest           │
│  ├── 置信度: 1.0（规则命中）                                      │
│  └── 输出: domain = "invest"                                     │
│                                                                   │
│  Step 1.2: L2 簇路由                                              │
│  ├── 在 invest 域下，regex "\d{6}" → 命中 stock_analysis         │
│  ├── 置信度: 0.95                                                │
│  └── 输出: cluster = "stock_analysis"                            │
│                                                                   │
│  Step 1.3: L3 预检索（不注入 Prompt）                             │
│  ├── 查询 stock_analysis 簇下所有工作流（约10个）                  │
│  └── 不直接选择，交给 Layer 2                                     │
└───────────────────────────┬─────────────────────────────────────┘
                            │ domain="invest", cluster="stock_analysis"
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│  Layer 2: 检索增强路由 (RAR)                                      │
│  ──────────────────────────                                       │
│                                                                   │
│  Step 2.1: 向量检索 Top-5                                         │
│  ├── 输入 embedding("请分析301302这支股票的技术面")                │
│  ├── 过滤: domain="invest", cluster="stock_analysis"              │
│  ├── 返回: [tech, fundamental, news, risk, competitor]           │
│  └── score: [0.92, 0.85, 0.78, 0.72, 0.68]                     │
│                                                                   │
│  Step 2.2: 动态 Few-shot Prompt 注入                              │
│  ├── 只注入 Top-5 的 name + description + input_schema           │
│  ├── 总 token ≈ 200（恒定，不随总量增长）                          │
│  └── 输出: 5选1 选择题                                            │
│                                                                   │
│  Step 2.3: LLM 选择                                               │
│  ├── LLM 输出: workflow_id = "wf_stock_tech", confidence = 0.94   │
│  └── 提取参数: stock_code = "301302"                             │
└───────────────────────────┬─────────────────────────────────────┘
                            │ workflow_id = "wf_stock_tech"
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│  Layer 3: 工作流图谱 (Graph)                                      │
│  ──────────────────────────                                       │
│                                                                   │
│  Step 3.1: 查询图谱路径                                           │
│  ├── 命中节点: invest/stock_analysis/tech                        │
│  ├── 检查下游邻接: {fallback: "invest/stock_analysis/fundamental"} │
│  └── 标记兜底路径（技术面失败时走基本面）                          │
│                                                                   │
│  Step 3.2: 输出路由决策                                           │
│  └── 返回: {                                                      │
│        route_path: "invest/stock_analysis/tech",                  │
│        workflow_id: "wf_stock_tech",                              │
│        fallback_path: "invest/stock_analysis/fundamental",        │
│        extracted_params: { stock_code: "301302" }                 │
│      }                                                            │
└───────────────────────────┬─────────────────────────────────────┘
                            │
                            ▼
                    执行目标工作流
```

### 4.2 Token 预算分析

| 层级 | Prompt 内容 | 估算 Token | 说明 |
|------|-----------|-----------|------|
| L1 域路由 | 约 10 个域的描述 | ~500 | 固定，不随工作流数量增长 |
| L2 簇路由 | 该域下约 20 个簇的描述 | ~800 | 固定，不随工作流数量增长 |
| L3 RAR 检索 | Top-5 候选的 name + schema | ~300 | 恒定 5 选 1 |
| 图谱路由 | 邻接表 + 路径信息 | ~200 | 仅路径标识 |
| **合计** | | **~1800** | 恒定值，< 2K |

**对比**：平铺 2000 工作流 → Prompt 约 50K+ Token → LLM 无法稳定输出

---

## 5. 路由决策输出结构（V2 更新）

```rust
/// 路由决策输出（三层架构版）
#[derive(Debug, Serialize, Deserialize)]
pub struct RoutingDecisionV2 {
    // ── 核心决策 ──
    pub decision: RouteDecision,
    pub confidence: f64,

    // ── 三层路径 ──
    pub route_path: RoutePath,
    pub extracted_params: HashMap<String, serde_json::Value>,

    // ── L2 RAR 信息 ──
    pub rar_candidates: Vec<RarCandidate>,  // Top-5 候选
    pub rar_scores: Vec<f64>,                 // 各候选向量得分

    // ── 图谱信息 ──
    pub graph_info: GraphRoutingInfo,

    // ── 资源与安全 ──
    pub memory_plan: MemoryPlan,
    pub resource_plan: ResourcePlan,
    pub security_flags: SecurityFlags,
    pub trace: TraceContext,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RoutePath {
    pub l1_domain: String,              /// "invest"
    pub l2_cluster: String,            /// "stock_analysis"
    pub l3_workflow: Option<String>,    /// "tech" (可选，RAR 未命中时为 None)
    pub full_path: String,              /// "invest/stock_analysis/tech"
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GraphRoutingInfo {
    pub current_node: String,
    pub downstream_nodes: Vec<String>,
    pub fallback_path: Option<String>,
}
```

---

## 6. 风险评估与缓解（V2 更新）

### 6.1 架构风险

| 风险 | 等级 | 缓解策略 |
|------|------|---------|
| L1/L2 层级定义争议 | 🟡 中 | 域和簇的划分由产品团队评审，提供迁移工具 |
| RAR 检索质量依赖 embedding | 🟡 中 | 支持多 embedding 后端切换 + 负向场景过滤 |
| 图谱维护复杂度 | 🟡 中 | L1→L2→L3 边自动推导，手动边仅限跳转关系 |
| 三层延迟叠加 | 🟡 中 | L1/L2 纯规则（<1ms），L3 RAR 检索（~100ms），图谱查询（<5ms） |
| 过度工程化 | 🟡 中 | 允许跳过任意层：L1 直接到 L3 或 L2 不经过 RAR |

### 6.2 安全性

```
安全防护嵌入每层：
├── L1 域路由层
│   └── 越狱检测（阻断攻击输入）
├── L2 RAR 检索层
│   └── 负向场景过滤（排除不匹配的候选）
└── 图谱路由层
    └── 合规路径校验（禁止访问受限路径）
```

### 6.3 元能力隔离架构（防自指悖论）

> **核心原则**：用户只能发现业务能力，绝不能发现系统自身的"神经系统"。路由工作流是"大脑"，能力发现是"眼睛"——眼睛可以看向世界，但永远不该试图"发现"大脑本身。

#### 问题：自指悖论

如果编排器（Cognitive Router）被注册到常规能力索引中，会导致：
- 编排器路由到"编排器"自身 → **无限递归**
- 编排器被其他编排器路由 → **层级混乱**
- 工作流发现机制暴露系统核心 → **安全风险**

#### 五层隔离防线

```
┌─────────────────────────────────────────────────────────────────┐
│                        五层隔离架构                              │
│                                                                 │
│  Layer 1: 注册层 — 双注册表物理隔离                              │
│  ┌─────────────────────┐    ┌─────────────────────┐            │
│  │  Business Registry  │    │  System Registry    │            │
│  │  (面向用户)          │    │  (面向运维/架构师)    │            │
│  │                     │    │                     │            │
│  │  • 查订单、退货款    │    │  • 路由编排器        │            │
│  │  • 发优惠券、查物流  │    │  • Model-Fallback    │            │
│  │  • 所有业务工作流    │    │  • Token预算切分器   │            │
│  │                     │    │  • 能力索引更新器    │            │
│  │  ✅ 挂载到能力发现   │    │  ✅ 不挂载到发现机制  │            │
│  └─────────────────────┘    └─────────────────────┘            │
│                                                                 │
│  Layer 2: 检索层 — visibility 字段硬性过滤                      │
│  ┌─────────────────────────────────────────────────────┐      │
│  │  能力元数据护照（Capability Passport）                 │      │
│  │  {                                                    │      │
│  │    "capability_id": "router_workflow_v4",            │      │
│  │    "visibility": "SYSTEM_ONLY",  // PUBLIC | SYSTEM_ONLY | HIDDEN │
│  │    "allowed_caller_roles": ["admin", "orchestrator"] │      │
│  │  }                                                    │      │
│  │                                                       │      │
│  │  🔒 检索闸门：visibility == 'SYSTEM_ONLY' → 直接跳过 │      │
│  └─────────────────────────────────────────────────────┘      │
│                                                                 │
│  Layer 3: 路由层 — 自指熔断代码级保护                            │
│  ┌─────────────────────────────────────────────────────┐      │
│  │  死刑判决逻辑（Self-reference Circuit Breaker）        │      │
│  │                                                       │      │
│  │  if selected.id == "ROUTER_WORKFLOW"                  │      │
│  │     or "ORCHESTRATOR" in selected.tags:               │      │
│  │      return execute_fallback("系统内部能力不对外开放") │      │
│  │                                                       │      │
│  │  🔒 这是防止无限递归的最后一道物理锁                     │      │
│  └─────────────────────────────────────────────────────┘      │
│                                                                 │
│  Layer 4: 执行层 — 系统能力走特权管道                            │
│  ┌─────────────────────────────────────────────────────┐      │
│  │  内部管理API（/admin/system/*）                        │      │
│  │  ├── mTLS 服务间认证                                   │      │
│  │  ├── 不接入面向用户的智能体网关                         │      │
│  │  ├── 仅后台定时任务/运维命令行可调用                     │      │
│  │  └── 彻底物理隔绝                                      │      │
│  └─────────────────────────────────────────────────────┘      │
│                                                                 │
│  Layer 5: 自增长处理 — 动态更新不污染业务库                      │
│  ┌─────────────────────────────────────────────────────┐      │
│  │  ✅ 正确做法：                                        │      │
│  │  • 路由表更新 → 仅修改 System Registry                 │      │
│  │  • Prompt配置更新 → 静默更新配置中心                   │      │
│  │                                                       │      │
│  │  ❌ 禁止做法：                                        │      │
│  │  • 把更新后的路由表塞回业务向量库                       │      │
│  │  • 让路由表作为"业务能力"被用户检索到                   │      │
│  └─────────────────────────────────────────────────────┘      │
└─────────────────────────────────────────────────────────────────┘
```

---

#### Layer 1: 注册层 — 双注册表物理隔离

从源头拆分，建立两个独立注册表：

```rust
/// 注册表类型枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryType {
    /// 业务能力注册表：面向用户，能力发现的搜索范围
    Business,
    /// 系统能力注册表：面向运维/架构师，不暴露给用户
    System,
}

/// 双注册表管理器
pub struct DualRegistry {
    /// 业务向量索引（用户可发现）
    business_index: VectorStore,
    /// 系统配置中心（不建索引，仅存储元数据）
    system_config: SystemConfigCenter,
}

impl DualRegistry {
    /// 注册业务能力（必须通过此接口）
    pub async fn register_business(
        &self,
        passport: &CapabilityPassport,
    ) -> Result<(), RegistryError> {
        // 强制校验：系统能力禁止注册到业务库
        if passport.visibility == Visibility::SystemOnly {
            return Err(RegistryError::SystemCapabilityBlocked);
        }
        self.business_index.upsert(passport).await
    }
    
    /// 注册系统能力（仅编排器/内部服务可调用）
    pub async fn register_system(
        &self,
        passport: &CapabilityPassport,
    ) -> Result<(), RegistryError> {
        // 系统能力存储到配置中心，不建向量索引
        self.system_config.store(passport).await
    }
    
    /// 能力发现（仅查询业务注册表）
    pub async fn discover(
        &self,
        query: &str,
        filter: &FilterContext,
    ) -> Result<Vec<CapabilityPassport>> {
        // 物理隔离：直接查业务索引，系统能力根本不在此集合
        let candidates = self.business_index.search(query, filter).await?;
        Ok(candidates)
    }
}
```

**注册表归属清单**：

| 能力类型 | 是否可被用户发现 | 调用者身份 | 存储位置 |
|---------|----------------|----------|---------|
| 业务工作流（查订单/退款） | ✅ 是 | 终端用户 | 业务向量索引（公开） |
| 路由编排器 | ❌ 否（绝对禁止） | 网关层（硬编码调用） | 系统配置中心（不建索引） |
| 模型降级控制器 | ❌ 否 | 后台守护进程 | 系统配置中心 |
| 能力发现/索引更新器 | ❌ 否 | 后台定时任务 | 系统配置中心 |
| Token预算切分器 | ❌ 否 | 网关前置拦截器 | 代码静态配置 |

---

#### Layer 2: 检索层 — visibility 字段硬性过滤

在能力护照中增加绝对过滤字段，检索闸门第一条规则直接跳过：

```rust
/// 能力可见性枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Visibility {
    /// 公开可发现（业务能力）
    Public,
    /// 系统内部专用（编排器、降级控制器等）
    SystemOnly,
    /// 已废弃/隐藏（不参与检索）
    Hidden,
}

impl Visibility {
    pub fn is_discoverable(&self) -> bool {
        matches!(self, Visibility::Public)
    }
}

/// 能力调用权限
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallerPermissions {
    /// 允许调用的角色列表
    pub allowed_roles: Vec<String>,
    /// 是否允许终端用户调用
    pub allow_end_user: bool,
}

/// 完整的能力护照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityPassport {
    pub capability_id: String,
    pub name: String,
    pub description: String,
    
    /// 🔒 核心隔离字段：可见性
    pub visibility: Visibility,
    
    /// 🔒 调用权限控制
    pub caller_permissions: CallerPermissions,
    
    /// 业务域
    pub domain: CapabilityDomain,
    /// 安全等级
    pub security_level: SecurityLevel,
    // ... 其他字段
}

/// 检索闸门实现（8维过滤的第一维）
impl CapabilityFilter {
    pub fn apply(&self, passport: &CapabilityPassport) -> FilterResult {
        // 🔴 第一闸门：visibility 硬性过滤
        if passport.visibility == Visibility::SystemOnly {
            return FilterResult::Reject(RejectReason::SystemOnly);
        }
        if passport.visibility == Visibility::Hidden {
            return FilterResult::Reject(RejectReason::Hidden);
        }
        
        // 继续其他维度过滤...
        // self.check_security_level(passport)?;
        // self.check_caller_permissions(passport)?;
        // ...
        
        FilterResult::Accept
    }
}
```

**检索流程伪代码**：
```python
def discover_capabilities(user_input, filter_context):
    candidates = vector_store.search(
        collection="business_capabilities",  # 物理隔离：仅查业务集合
        query=user_input,
        top_k=50,
    )
    
    results = []
    for candidate in candidates:
        # 🔒 第一道闸门：visibility 过滤
        if candidate.visibility == "SYSTEM_ONLY":
            continue  # 绝对跳过，不参与任何相似度计算
        if candidate.visibility == "HIDDEN":
            continue
        
        # 第二道闸门：安全等级检查
        if candidate.security_level == "RESTRICTED":
            if not filter_context.user_has_permission("admin"):
                continue
        
        # 通过所有闸门
        results.append(candidate)
    
    return results[:5]  # 最终 Top-5
```

---

#### Layer 3: 路由层 — 自指熔断保护

即使前两层都漏了，在最终决策层加上"死刑判决"逻辑：

```rust
/// 自指熔断保护器
pub struct SelfReferenceCircuitBreaker {
    /// 绝对禁止的能力ID前缀
    forbidden_id_patterns: Vec<String>,
    /// 绝对禁止的标签
    forbidden_tags: Vec<String>,
}

impl SelfReferenceCircuitBreaker {
    pub fn new() -> Self {
        Self {
            forbidden_id_patterns: vec![
                "router".to_string(),
                "orchestrator".to_string(),
                "cognitive_router".to_string(),
            ],
            forbidden_tags: vec![
                "system".to_string(),
                "orchestrator".to_string(),
                "meta".to_string(),
            ],
        }
    }
    
    /// 熔断检查：返回 true 表示应该阻断
    pub fn should_block(&self, candidate: &RarCandidate) -> bool {
        let id_lower = candidate.id.to_lowercase();
        
        // 规则1：ID 包含禁止模式
        for pattern in &self.forbidden_id_patterns {
            if id_lower.contains(pattern) {
                tracing::warn!(
                    capability_id = %candidate.id,
                    reason = "forbidden_id_pattern",
                    "🔒 自指熔断触发：ID匹配禁止模式"
                );
                return true;
            }
        }
        
        // 规则2：标签包含禁止标记
        if let Some(tags) = &candidate.tags {
            for tag in tags {
                if self.forbidden_tags.contains(&tag.to_lowercase()) {
                    tracing::warn!(
                        capability_id = %candidate.id,
                        tag = %tag,
                        reason = "forbidden_tag",
                        "🔒 自指熔断触发：标签匹配禁止标记"
                    );
                    return true;
                }
            }
        }
        
        false
    }
}

/// 路由决策流程中的熔断检查
impl CognitiveRouter {
    pub async fn route(&self, user_input: &str) -> Result<RoutingDecision> {
        // ... 前两层检索 ...
        
        // Layer 3: 自指熔断检查
        for candidate in &rar_candidates {
            if self.circuit_breaker.should_block(candidate) {
                // 强制降级到安全兜底
                return Ok(RoutingDecision::fallback(
                    "系统内部能力不对外开放，请重新描述您的业务需求"
                ));
            }
        }
        
        // 正常路由流程...
        Ok(RoutingDecision::normal(selected_workflow))
    }
}
```

---

#### Layer 4: 执行层 — 特权管道

系统能力通过独立的内部管理 API 端点执行，物理隔绝：

```rust
/// 系统能力执行特权管道
pub struct SystemPrivilegedPipeline {
    /// mTLS 认证器
    mtls_authenticator: MtlsAuthenticator,
    /// 审计日志
    audit_logger: AuditLogger,
}

impl SystemPrivilegedPipeline {
    /// 系统能力执行（仅内部服务可调用）
    pub async fn execute_system_capability(
        &self,
        capability_id: &str,
        input: serde_json::Value,
        caller: &CallerIdentity,
    ) -> Result<ExecutionResult, SystemPipelineError> {
        // 🔒 mTLS 认证
        self.mtls_authenticator.verify(caller)?;
        
        // 🔒 角色权限检查
        if !caller.has_role("orchestrator") && !caller.has_role("admin") {
            return Err(SystemPipelineError::AccessDenied);
        }
        
        // 🔒 审计日志
        self.audit_logger.log_system_call(capability_id, caller, &input);
        
        // 执行系统能力
        let result = self.execute_internal(capability_id, input).await?;
        
        Ok(result)
    }
    
    /// 系统能力列表（仅管理界面可见）
    pub async fn list_system_capabilities(
        &self,
        caller: &CallerIdentity,
    ) -> Result<Vec<SystemCapabilityInfo>, SystemPipelineError> {
        self.mtls_authenticator.verify(caller)?;
        if !caller.has_role("admin") {
            return Err(SystemPipelineError::AccessDenied);
        }
        // 返回系统能力清单...
    }
}
```

---

#### Layer 5: 自增长处理 — 动态更新不污染

路由工作流自我更新时，严格隔离业务库：

```rust
/// 路由表自更新管理器
pub struct RouterSelfUpdateManager {
    system_config: SystemConfigCenter,
    business_registry: DualRegistry,
}

impl RouterSelfUpdateManager {
    /// 路由表自更新（仅修改系统配置中心）
    pub async fn update_routing_table(
        &self,
        new_rules: Vec<RoutingRule>,
    ) -> Result<(), UpdateError> {
        // ✅ 正确做法：仅修改 System Registry
        self.system_config.update_routing_rules(new_rules).await?;
        
        // ✅ 更新 Prompt 配置（静默）
        self.system_config.update_prompt_template(
            "cognitive_router_system_prompt",
            self.build_updated_prompt(),
        ).await?;
        
        // ✅ 记录版本
        self.system_config.record_version("routing_table", new_rules.len()).await?;
        
        // ❌ 绝对禁止：把路由表塞回业务向量库
        // self.business_registry.register_business(router_capability)  ← 禁止！
        
        Ok(())
    }
    
    /// 置信度阈值自优化
    pub async fn optimize_confidence_threshold(
        &self,
        new_threshold: f64,
        performance_metrics: &PerformanceMetrics,
    ) -> Result<(), UpdateError> {
        // 更新配置（仅系统配置中心）
        self.system_config.update_config(
            "router_confidence_threshold",
            serde_json::json!({
                "value": new_threshold,
                "updated_at": chrono::Utc::now(),
                "based_on_metrics": performance_metrics,
            }),
        ).await?;
        
        tracing::info!(
            new_threshold,
            "路由置信度阈值已自优化（仅系统配置中心）"
        );
        Ok(())
    }
}
```

---

#### 隔离验证清单

| 验证项 | 测试方法 | 通过标准 |
|--------|----------|----------|
| 业务检索不返回系统能力 | 用用户输入检索所有系统能力关键词 | 0 条结果 |
| 编排器不会被自身路由 | 触发编排器路由流程，检查是否命中自身 | 熔断触发，返回兜底 |
| 系统能力仅内部可调用 | 用普通用户凭证调用系统 API | 403 Access Denied |
| 路由表更新不污染业务库 | 更新路由表后，检索业务库 | 业务库数量不变 |
| mTLS 认证强制 | 用无证书请求系统 API | 401 Certificate Required |

---

*文档版本 v0.3 — 三层分层路由架构 + 五层元能力隔离防线*

---

## 7. 实施路径（V3 更新）

### Phase 0: 五层隔离架构 + 数据模型（1 周）

**隔离基础层**：
- [ ] Layer 1: 注册层 — 实现 `DualRegistry` 双注册表管理器
  - `register_business()` 接口（业务能力注册，含 visibility 校验）
  - `register_system()` 接口（系统能力注册，存储到配置中心）
  - `discover()` 接口（仅查询业务注册表）
- [ ] Layer 2: 检索层 — 新增 `Visibility` 枚举字段
  - `CapabilityPassport` 新增 `visibility` 和 `caller_permissions` 字段
  - `CapabilityFilter` 新增 visibility 硬性过滤闸门（第一维）
- [ ] Layer 3: 路由层 — 实现 `SelfReferenceCircuitBreaker` 自指熔断保护器
- [ ] Layer 4: 执行层 — 定义 `SystemPrivilegedPipeline` 特权管道接口
- [ ] Layer 5: 自增长处理 — 实现 `RouterSelfUpdateManager` 自更新管理器

**数据模型**：
- [ ] 扩展 `CapabilityDomain` 枚举（新增业务域）
- [ ] 新增 `capability_clusters` 表 + CRUD
- [ ] 新增 `workflow_graph_nodes/edges` 表
- [ ] `CapabilityPassport` trait 新增 `sub_category()` 方法
- [ ] `workflow_templates` 表新增 `cluster_id`、`route_path` 字段

### Phase 1: 分层路由树（2 周）

- [ ] L1 域路由规则实现（纯规则 + LLM 兜底）
- [ ] L2 簇路由规则实现
- [ ] Prompt 模板引擎（分层注入，token 限制）
- [ ] 路由规则管理 UI（编辑器中配置 L1/L2 规则）
- [ ] 编排器注册到 System Registry（验证注册隔离）

### Phase 2: RAR 检索增强（2 周）

- [ ] `RarRouter` 实现（复用现有 VectorStore）
- [ ] Top-K 候选过滤 + visibility 硬性过滤 + 负向场景排除
- [ ] 动态 Few-shot Prompt 生成
- [ ] RAR 效果离线评测（准确率 > 90% 目标）
- [ ] **隔离验证**：RAR 检索 100% 不返回 SystemOnly 能力

### Phase 3: 工作流图谱（2 周）

- [ ] `WorkflowGraph` DAG 结构实现
- [ ] 图谱同步器（工作流创建/更新时自动维护）
- [ ] 邻接表 API（`get_neighbors`、`to_adjacency_summary`）
- [ ] 图谱路由 LLM 实现
- [ ] 路径校验：禁止输出系统路径

### Phase 4: 集成与优化（1 周）

- [ ] 三层协作流程串联
- [ ] 自指熔断集成到路由决策流程
- [ ] 端到端路由延迟优化
- [ ] **五层隔离全面验证**：
  - 业务检索不返回系统能力（0 条结果）
  - 编排器不会被自身路由（熔断触发）
  - 系统能力仅内部可调用（403 拦截）
  - 路由表更新不污染业务库（数量不变）
- [ ] A/B 实验框架
- [ ] 完整验收测试

---

## 8. 验收标准（V3 更新）

| Phase | 功能验收指标 | 五层隔离验收指标 |
|-------|-------------|-----------------|
| Phase 0 | 数据库迁移通过，CRUD 接口可用 | 双注册表接口工作正常，visibility 字段正确过滤 |
| Phase 1 | L1 域路由准确率 > 95%，L2 簇路由准确率 > 90% | 编排器不会被 L1/L2 路由命中（Layer 1 验证） |
| Phase 2 | RAR Top-5 命中率 > 92%，最终选择准确率 > 90% | RAR 检索 100% 不返回 SystemOnly 能力（Layer 2 验证） |
| Phase 3 | 图谱路径覆盖率 > 98%，邻接表查询 < 1ms | 自指熔断正确触发，返回兜底（Layer 3 验证） |
| Phase 4 | 端到端路由延迟 < 500ms（LLM 路径），< 10ms（规则路径） | 系统 API 仅内部可调用，自增长不污染（Layer 4-5 验证） |

**隔离专项测试用例**：

| 测试场景 | 测试方法 | 预期结果 |
|---------|----------|----------|
| 业务能力发现 | 用"路由编排器"等关键词检索 | 返回 0 条结果 |
| 自指熔断触发 | 模拟编排器被 LLM 选中 | 熔断触发，返回兜底消息 |
| 越权访问拦截 | 用普通用户 token 调用系统 API | 403 Access Denied |
| mTLS 认证强制 | 无证书请求系统特权管道 | 401 Certificate Required |
| 路由表自更新 | 更新路由规则后检查业务库 | 业务库记录数不变 |
| 递归深度保护 | 模拟 11 层嵌套调用 | 第 11 层返回错误 |

---

## 9. 参考资料

- [capability.rs](file:///d:/OneManager/AxAgent/src-tauri/crates/harness/src/capability.rs) — CapabilityDomain、SecurityLevel、CapabilityPassport 定义
- [capability_filter.rs](file:///d:/OneManager/AxAgent/src-tauri/crates/harness/src/capability_filter.rs) — 8 维过滤闸门实现
- [capability_indexer_impl.rs](file:///d:/OneManager/AxAgent/src-tauri/crates/tools/src/capability_indexer_impl.rs) — 能力向量索引实现
- [hybrid_search.rs](file:///d:/OneManager/AxAgent/src-tauri/crates/search/src/hybrid_search.rs) — 混合检索实现
- [workflow_template.rs](file:///d:/OneManager/AxAgent/src-tauri/crates/entities/src/workflow_template.rs) — 工作流模板实体
- [subworkflow_executor.rs](file:///d:/OneManager/AxAgent/src-tauri/crates/rt-workflow/src/work_engine/executors/subworkflow_executor.rs) — 子工作流执行器
- [switch_executor.rs](file:///d:/OneManager/AxAgent/src-tauri/crates/rt-workflow/src/work_engine/executors/switch_executor.rs) — Switch 节点执行器
- [llm_classifier_executor.rs](file:///d:/OneManager/AxAgent/src-tauri/crates/rt-workflow/src/work_engine/executors/llm_classifier_executor.rs) — LLM 分类节点
- [dag_store.rs](file:///d:/OneManager/AxAgent/src-tauri/crates/rt-workflow/src/work_engine/dag_store.rs) — DAG 拓扑计算

---

*文档版本 v0.3 — 三层分层路由架构 + 五层元能力隔离防线*
