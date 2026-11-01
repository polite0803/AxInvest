# 文字创作工作流叙事结构增强方案

> **状态**: 规划中
> **创建日期**: 2026-08-10
> **关联模块**: `content_media` (OPC 行业适配器), `rt-workflow` (工作流引擎), `axagent-harness` (数据契约)
> **目标**: 将文字创作工作流中的“叙事结构设计”（角色弧线、交汇点、伏笔网络）从“Prompt驱动”升级为“结构化、数据化、引擎化”的工程能力。

---

## 1. 背景与现状

当前文字创作工作流（`workflow-cm-literary-creation`）已具备基础的“生成式写作流水线”（元认知→大纲→循环写章→评审→归档），但其核心叙事逻辑（弧线、交汇点、伏笔）仅作为 Agent 的 Prompt 文本描述存在，缺乏：
- **结构化定义**：无统一的数据模型存储叙事结构。
- **动态控制**：无法在执行过程中根据反馈调整弧线或伏笔。
- **可视化交互**：用户无法直观设计和调整故事结构。

---

## 2. 核心增强思路

1. **结构化数据**：在 `axagent-harness` 中定义权威的叙事结构 DTO，实现类型安全。
2. **节点化驱动**：将叙事结构作为核心变量注入工作流节点，新增结构映射、校验、调整节点。
3. **动态自修复**：复用现有反思进化机制，实现结构的动态调整与优化。
4. **可视化交互**：新增前端组件，提供叙事结构的可视化设计面板。

---

## 3. 详细设计

### 3.1 奠定基础：结构化数据模型 (Phase 1)

在 `axagent-harness`（核心契约层）中定义叙事结构的权威数据类型，确保前后端、执行引擎和 Agent 之间的类型安全与复用。

**建议新增的 DTO 定义 (Rust)**:

```rust
// src-tauri/crates/harness/src/narrative.rs (建议新建)

/// 叙事结构总览
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrativeStructure {
    pub arcs: Vec<NarrativeArc>,
    pub confluences: Vec<ConfluencePoint>,
    pub foreshadows: Vec<Foreshadow>,
}

/// 角色/主题弧线
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrativeArc {
    pub id: String,
    pub arc_type: ArcType,
    pub subject: String,               // 弧线主体（如主角、“自由”主题）
    pub want: String,                 // 外部目标
    pub need: String,                 // 内部缺失
    pub stages: Vec<ArcStage>,       // 弧线的关键阶段
    pub current_progress: f32,        // 当前推进度 (0-100)
}

/// 交汇点 (多条线索/弧线在此汇聚、冲突或转折)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfluencePoint {
    pub id: String,
    pub trigger_chapter: u32,         // 触发章节
    pub confluence_type: ConfluenceType,
    pub involved_arcs: Vec<String>,
    pub involved_foreshadows: Vec<String>,
    pub impact: String,
}

/// 伏笔 (追踪“埋设”与“回收”的完整生命周期)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Foreshadow {
    pub id: String,
    pub setup_chapter: u32,           // 埋设章节
    pub payoff_chapter: Option<u32>,  // 回收章节
    pub status: ForeshadowStatus,
    pub description: String,
    pub payoff_description: Option<String>,
    pub related_arcs: Vec<String>,
}

// --- 枚举类型 ---
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ArcType {
    Transformative,  // 转换型
    Steadfast,       // 坚定型
    Flat,            // 扁平型
    Tragic,          // 悲剧型
    Comedic,         // 喜剧型
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConfluenceType {
    ConflictBurst,   // 冲突爆发
    RevealTruth,     // 真相揭示
    ShiftPerspective, // 视角转换
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ForeshadowStatus {
    Setup,       // 已埋设
    Payoff,      // 已回收
    Abandoned,   // 已废弃
}
```

**架构合规性**:
- 类型定义在 `harness`，作为权威来源。
- 前端 `src/types` 需同步更新 TypeScript 定义。
- `rt-workflow` (Consumer) 仅依赖 `harness` 读取这些类型，不得绕过。

---

### 3.2 工作流节点改造：从“文本生成”到“结构驱动” (Phase 1-2)

改造 `workflow-cm-literary-creation` 模板，将叙事结构作为**核心驱动变量**注入。

#### A. 扩展 `lc-conceive` (创作元认知装配)
**原功能**: 仅生成世界观、冲突轴
**增强后**: 调用 `academic-narratologist` Agent，基于主题生成**完整的结构化叙事骨架**

- **输入**: 用户主题
- **输出**: 完整的 `NarrativeStructure` JSON，存入 `lc-structure` 变量。
- **Prompt 改造**: 要求 Agent 输出结构化数据。

#### B. 新增 `lc-structure-injector` (结构注入节点)
在 `lc-outline`（大纲拆章）之前，新增一个节点，用于**将叙事结构映射到具体章节**。

- **节点ID**: `lc-structure-injector`
- **作用**: 为大纲生成提供精确的叙事约束（如：第5章推进弧线，第12章触发交汇点）。
- **输出**: `lc-chapter-structure` (键值对，Key为章节号，Value为该章的结构指令)

#### C. 改造 `lc-draft-chapter` (单章创作循环)
在每章创作前，**强制注入该章的叙事结构指令**。

- **输入 Mapping**: 增加 `("chapter_structure", "lc-chapter-structure.{chapter}")`，动态注入当前章的结构约束。
- **Prompt 改造**: 增加**强制性结构约束**（如：必须推进弧线X，埋设伏笔Y）。

#### D. 新增 `lc-structure-checker` (结构校验节点)
在每章创作后，新增一个校验节点，**自动检查叙事结构的遵循情况**。

- **节点ID**: `lc-structure-checker`
- **作用**: 对比实际生成内容与结构指令，输出偏差报告。
- **输出**: `{ "compliance": 85, "deviations": [] }`

---

### 3.3 动态执行控制：结构的自修复与进化 (Phase 2)

利用项目已有的**动态任务拆解**和**反思进化**机制，实现叙事结构的**动态调整**。

#### A. 结构动态调整节点 `lc-structure-adapter`
在 `lc-draft-loop` (逐章循环) 的 `body_steps` 中，增加一个调整节点。

- **触发条件**: 如果 `lc-structure-checker` 报告偏差较大（`compliance < 70`）。
- **逻辑**: 自动调整后续章节的结构，例如将无法埋设的伏笔延后。
- **操作**: 更新 `lc-chapter-structure` 变量。

#### B. 扩展 KPI 与反思维度
在行业适配器 (`content_media.rs`) 和反思配置 (`learning.yaml`) 中，新增叙事结构相关指标。

- **新增 KPI**:
  - `arc_completion` (弧线完成度)
  - `foreshadow_recovery_rate` (伏笔回收率)
  - `structure_compliance` (结构遵循率)
- **新增反思维度**: `structure_compliance` (评估叙事结构遵循情况)

---

### 3.4 可视化与交互：用户可控的结构设计 (Phase 3)

为前端添加**叙事结构可视化面板**，让用户可以直观地设计和调整结构。

#### A. 新增 React 组件
在 `src/components/workflow/` 下新增：
- `NarrativeStructurePanel.tsx`: 主面板，汇总展示所有结构元素。
- `ArcTimeline.tsx`: 可视化角色弧线的阶段推进。
- `ForeshadowGraph.tsx`: 图结构展示伏笔的埋设与回收关系。

#### B. 集成到工作流编辑器
在工作流画布侧边栏添加「叙事结构」标签页，允许用户：
- 手动创建/编辑弧线、交汇点、伏笔。
- 拖拽调整伏笔的埋设/回收章节。
- 预览结构调整对后续章节的影响。

---

## 4. 实施路线图

| 阶段 | 任务 | 依赖 | 优先级 |
|------|------|------|--------|
| **Phase 1** | 1. 定义 `axagent-harness` DTO<br>2. 扩展 `lc-conceive` 输出结构<br>3. 改造 `lc-draft-chapter` 注入约束 | 无 | 🔴 高 |
| **Phase 2** | 1. 新增 `lc-structure-injector` / `checker` 节点<br>2. 实现结构动态调整逻辑<br>3. 扩展 KPI 和反思机制 | Phase 1 | 🔴 高 |
| **Phase 3** | 1. 新增前端可视化组件<br>2. 集成到工作流编辑器<br>3. 实现用户手动调整功能 | Phase 2 | 🟡 中 |
| **Phase 4** | 1. 新增 `narrative_structure` 数据表<br>2. 实现结构的跨会话保存与恢复 | Phase 3 | 🟢 低 |

---

## 5. 架构合规性检查

- ✅ **数据模型定义在 `harness`**：Consumer (`rt-workflow`) 仅依赖 `harness`，符合“依赖方向铁律”。
- ✅ **复用现有节点类型**：复用 Agent、Condition、Loop 节点，未新增自定义节点类型。
- ✅ **复用现有 Agent**：复用 `academic-narratologist` Agent 的能力，未创建重复 Agent。
- ✅ **复用反思机制**：扩展 `learning.yaml` 的维度，而非重写反思逻辑。
- ✅ **动态工作流**：通过变量注入和循环实现动态编排，符合动态性要求。

---

*文档结束*