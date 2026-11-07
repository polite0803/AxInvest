> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# Hermes-Agent 借鉴功能实施计划

> **日期**: 2026-07-11  
> **前提**: 聚焦桌面端，VPS 部署已搁置  
> **优先级**: P0 → P1 → P2，每个优先级内按实施成本排序

---

## 现有代码基础评估

实施前对现有代码的盘点——基础设施已大量存在，核心工作是"接线 + 前端可视化"。

| 模块 | 后端 (Rust) | 前端 (React) | 不足 |
|------|:-----------:|:------------:|------|
| Nudge/Reminder | ✅ `PeriodicNudge`, `ClosedLoopService`, `ReminderManager`, `ProactiveAssistant` | ✅ `useProactiveStore` | 未接入 agent 推理循环；用户无感知 |
| 学习图/技能可视化 | ✅ `SkillProposalService`, `skill_manager`, `trajectory/insight` | ✅ SkillsPage, MemoryPage | 无"学习图"节点/边可视化组件 |
| 技能自动创建 | ✅ `SkillProposalService`, `AutoToolCreator`, `skill_evolution` | ✅ SkillProposalPanel | 未自动触发，需手动确认 |
| 记忆写回 Mirror | ❌ 无 | ❌ 无 | 需要新建 |
| 会话搜索 UI | ✅ FTS5 + 向量搜索 | ✅ 搜索框存在 | 缺乏跨会话语义搜索 UI |

---

## P0 — 立即实施（按成本排序）

### P0-1：Nudge 机制注入 ReAct 循环 — ✅ 已完成（2026-07-11，方案 B）

> **目标**: 在每轮对话前注入"记忆提醒"上下文块，让 agent 能引用长期记忆

**实施方案**（方案 B — 扩展现有 ConversationRuntimeHost trait）：

1. **harness**: 在 `ConversationRuntimeHost` trait 添加 `set_nudge_lines(&mut self, Vec<String>)`
   - 默认空实现（`{}`），保持向后兼容
   - Box<T> blanket impl 添加转发

2. **runtime-core**: `ConversationRuntime` 实现
   - 新增 `nudge_lines: Vec<String>` 字段
   - 在 `run_turn()` 的 LLM 调用前注入到 system_prompt
   - 格式：`<memory_context>...</memory_context>` 块

3. **与现有代码的关系**
   - `commands/agent/mod.rs` 已有静态 nudge 注入到系统提示（`build_system_prompt_with_context`）
   - 新 runtime 级注入与之互补，未来可在每次迭代前注入不同的 nudge 上下文
   - 当前：静态路径处理 nudge → 无需迁移

**已修改文件**:
- `src-tauri/crates/harness/src/runtime_types/conversation.rs`
- `src-tauri/crates/runtime-core/src/conversation.rs`

**下一步**: 前端 nudge 通知组件（Chat 页面顶部"记忆提醒"横幅）— 等待确认是否需要

---

### P0-2：学习可见性面板 — 前端"学习图"页面（~4 天）

> **目标**: 新增 `/learning-graph` 页面，展示技能节点、记忆卡片、技能-记忆关联边

**现状**:
- ✅ SkillsPage 已存在（技能列表/创建/市场/统计）
- ✅ MemoryPage 已存在（记忆设置）
- ✅ `trajectory/insight.rs` 有 `LearningInsight`, `LearningInsightSystem`
- ✅ `trajectory/skill.rs` 有 `SkillProposal` 数据
- ❌ **缺失**: 节点/边关系模型、图可视化组件、前端渲染引擎

**实施步骤**:

1. **后端新增 `LearningGraphService`**（`trajectory/src/learning_graph.rs`）
   - 数据模型：
     ```rust
     struct GraphNode { id, label, kind: "skill"|"memory"|"insight", category, timestamp, use_count, state }
     struct GraphEdge { source, target, weight: f64, relation_type: String }
     struct LearningGraph { nodes: Vec<GraphNode>, edges: Vec<GraphEdge>, stats: GraphStats }
     ```
   - 数据来源：
     - 技能节点 → `skill_manager` + usage stats
     - 记忆节点 → `auto_memory` + `.workbuddy/memory/MEMORY.md`
     - 关联边 → lexical overlap (类似 Hermes 方法)
   - Tauri command: `get_learning_graph` → `Result<LearningGraph>`

2. **新增 Tauri 命令**
   - 在 `src/commands/` 新增 `commands/learning_graph.rs`
   - 注册到 `commands/mod.rs` + `lib.rs`

3. **前端新增 LearningGraphPage**
   - `src/pages/LearningGraphPage.tsx`
   - 使用 **vis-network** 或 **react-force-graph-2d** 渲染节点/边图
   - 左侧节点列表（可按 category 筛选）
   - 右侧图面板（带缩放/拖拽）
   - 点击节点弹出详情（技能内容/记忆卡片）

4. **路由注册**
   - 在 route 配置中加入 `/learning-graph`
   - 在主菜单（侧栏）新增"学习图"入口

**依赖**: 
- `npm install vis-network` 或 `react-force-graph-2d`
- 后端 `trajectory` crate（已在依赖中）

---

### P0-3：技能自动创建闭环（~3 天）

> **目标**: 复杂任务完成后自动生成 skill 提案，推送到前端通知用户确认

**现状**:
- ✅ `SkillProposalService` 已存在（`analyze_and_propose` 方法 + 触发逻辑）
- ✅ `AutoToolCreator` 可自动生成工具代码
- ✅ `SkillProposalPanel` 前端组件已存在（`src/components/chat/SkillProposalPanel.tsx`）
- ❌ **缺失**: 后端触发时机（`run_turn_with_tools` 完成后未调用 proposal）、前端自动弹出

**实施步骤**:

1. **后端自动触发**
   - 在 `session_manager.rs` 的 `run_turn_with_tools` 成功返回后：
     - 如果 `trajectory_service` 存在
     - 调用 `trajectory_service.propose_skill_from_turn(session_id, turn_summary)`
   - 逻辑：检查该 turn 的工具调用次数 > 3 + 成功 → 自动生成 proposal

2. **推送 proposal 到前端**
   - 通过 Tauri event 发送 `skill-proposal` 事件
   - 数据结构：`{ id, name, description, suggested_tools[], confidence }`

3. **前端自动弹出提示**
   - 监听 `skill-proposal` 事件
   - 在 Chat 页面右下角弹出通知气泡
   - 点击后打开 `SkillProposalPanel` 模态框

4. **SkillProposalPanel 增强**
   - 显示 LLM 生成的代码片段预览
   - "接受" → 创建 skill 并跳转到 SkillsPage
   - "修改" → 打开 FrontendEditorModal
   - "忽略" → 标记 dismissed

**关键代码位置**:
- 后端触发点: `session_manager.rs:700` — `run_turn` 成功后
- 现有提案组件: `src/components/chat/SkillProposalPanel.tsx`
- Tauri event: `app_handle.emit("skill-proposal", payload)`

---

## P1 — 中期实施计划

### P1-1：记忆写回 Mirror 机制（~2 天）

> 当内置 memory 工具写入时，自动同步到 search crate 的向量索引

**步骤**:
1. 在 `session_manager.rs` 的处理 tool_result 逻辑中增加 hook
2. 检测 `memory` 工具的 add/replace 操作
3. 同步调用 `search` crate 的 `index_memory` 写入 FTS5 + 向量存储
4. 前端无感，但后续搜索可命中

### P1-2：用户建模升级为"偏好漂移追踪"（~3 天）

**步骤**:
1. 在 `trajectory/preference_learner.rs` 增加时间维度
2. 记录每次偏好变化的 timestamp 和上下文
3. 新增 `preference_history` 表，支持"上周喜欢 X，这周喜欢 Y"的查询
4. 在 `user_profile` 中增加 `preference_timeline` 字段
5. 前端 LearningGraphPage 增加"偏好变化"时间线视图

### P1-3：Cron 自然语言定义（~2 天）

**步骤**:
1. 在 `rt-workflow` 中增加 `nl_to_cron` LLM 调用
2. 输入："每天早上 9 点发日报" → 输出 cron 表达式
3. 前端新增"自然语言定时"输入框

---

## P2 — 可选改进

### P2-1：流量/信用追踪仪表盘（~3 天）
### P2-2：上下文压缩按钮（~1 天）
### P2-3：使用统计面板 /insights（~2 天）
### P2-4：斜杠命令快捷方式（~2 天）

---

## 实施路线图

```
Week 1    P0-1 Nudge 注入 ReAct 循环         ← 当前
          P0-3 技能自动创建闭环

Week 2    P0-2 学习可见性面板 (LearningGraph)

Week 3    P1-1 记忆写回 Mirror
          P1-2 用户建模升级

Week 4    P1-3 Cron 自然语言定义
          P2 可选改进
```

**建议即刻开始的第一项**: **P0-1 Nudge 机制**。它代码量最小（~2 天）、用户感知最直接、基础设施已就绪。
