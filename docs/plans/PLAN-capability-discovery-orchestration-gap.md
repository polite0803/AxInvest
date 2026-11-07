> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# 能力发现 × 认知编排 架构差距分析与实施方案

> 状态：已评审，开始实施（2026-08-30）
> 范围：AxAgent 能力发现系统（harness/tools）+ 认知编排（cognitive_query + L1/L2/L3 DAG）+ 暴露层（agent/mod.rs）
> 结论摘要：发现系统骨架完备，但存在 1 个排序数据断裂（P0）、1 个执行断链（P0）、4 类对象缺失（P1/P2）

---

## 一、差距分析结论（已代码核实）

### 1.1 分层架构

| 你的规范 | AxAgent 现状 | 差距 |
|---|---|---|
| 应用层（工作流、插件） | `CapabilityKind::Workflow`；插件是 `CapabilitySource::Plugin` 护照 | 插件无动态展开层，静态注册即完成 |
| 任务层（技能、模板、工具链） | 只有 `Skill` | **Template、Toolchain 完全缺失** |
| 原子层（工具） | `Tool` | 有，且 P5 保护强制非 Workflow 降级 Delegate |
| 元信息层（策略、能力组、知识片段、别名） | `KnowledgeBase` + L2 簇（近似能力组） | **策略非对象化、知识片段/别名完全缺失** |
| 自上而下逐层降级 | L1/L2/L3 是路由树；检索是平铺混合召回（向量0.6+关键词0.2+标签0.2） | 有"先窄后精"骨架，但**无"高层命中不降级"的粒度降级语义**，Skill/Tool 检索时平级 |

### 1.2 动态发现流程

| 步骤 | 现状 | 差距 |
|---|---|---|
| 语义解析 | 文本直接 embed | **无意图向量 + 关键实体抽取** |
| 分层检索 | L1 规则+LLM → L2 簇 → L3 RAR + 图谱路由 | 结构完备；命令层 RAR 默认关，主 DAG 内开 |
| 策略过滤 | 8 维闸门 + FilterContext + negative_scenarios + 熔断 | ✅ 符合"策略=后置过滤器" |
| 关联扩展 | 无 | **完全缺失**（无 Skill 上下游、无 Toolchain、无组合扩展） |
| 排序输出 | α语义+β历史成功率+γ耗时+δ成本+个性化+探索 | 公式完备，**但数据源断裂（见 1.4）** |

### 1.3 10 类对象覆盖

| 对象 | 状态 | 说明 |
|---|---|---|
| Workflow | ✅ 基本符合 | 标签+语义匹配，>0.90 直发，认知层不拆编排 |
| Skill | ⚠️ 部分 | 有语义+β+权限；**缺前提条件匹配** |
| Template | ❌ 缺失 | 无占位符匹配 + 实例化提示 |
| Toolchain | ❌ 缺失 | 无固定顺序工具组合对象 |
| Tool | ⚠️ 部分 | 兜底有；**缺附带建议组合路径** |
| Plugin | ⚠️ 近似 | 护照化展开符合；但静态注册非动态展开 |
| Policy | ⚠️ 近似 | 8 维闸门符合理念；硬编码非可配置规则 |
| 知识片段 | ❌ 缺失 | 护照只有 description，无随附知识 |
| Alias | ❌ 缺失 | 无"发邮件→mail_send"映射 |
| 能力组 | ⚠️ 近似 | L2 簇近似；静态规则非动态注册 |

### 1.4 🔴 隐藏 Bug：反馈闭环断裂（P0）

- 护照 `stats` 注册后**从未被写回**（state.rs 全部 `Default::default()`）
- 执行反馈只写 `workflow_tool` / `workflow_execution_stats` / trajectory / telemetry，无一处更新护照 stats
- 后果：`stats.total_calls` 恒 0 → `total_calls < 10` 恒真 → **探索提权对所有能力生效** → β/γ/δ/探索四维排序全部失真
- 根因之二：护照存内存 + 向量库 metadata，不落业务表，重启即丢

### 1.5 🔴 执行断链：暴露模式差异未处理（P0）

| 维度 | 被动模式（execution_mode=None） | 主动模式（execution_mode=Some） |
|---|---|---|
| built-in 工具 | 按 active_domains 全量塞 | **空**（注释承诺"由能力发现路径注入"，未实现） |
| 技能 | enabled_skill_ids 全量注入 | **空** |
| Tauri 命令桥 | 全量（~120 命令） | **空** |
| MCP | 会话启用即注入 | 会话启用即注入（唯一漏网） |

根因：
1. 护照元数据与工具定义是两份独立快照，`RarCandidate` 无 tool_ref/schema，命中后无法反向定位真实工具
2. 技能双身份未区分：被动靠会话勾选、主动靠语义检索，两套触发互不同步
3. 暴露粒度失控：被动"按域全量塞"，无"按任务选子集"
4. 护照热更新漂移：启动快照 vs 注册表热更新

### 1.6 交互边界

- 任务拆解：只有 TaskShape 形态分类，无子目标拆解（Plan 模式由 agent planner 做，不在边界内）
- 逐项发现：一次性 Top-K，无"每子目标逐项发现"
- 路径选择/执行调用：✅ 符合
- 参数填充：ParameterExtract 部分实现
- 结果反馈：轨迹+stats+进化闭环设计存在，**数据管道断裂（1.4）**

---

## 二、实施方案（四阶段）

### Phase 1 — 修复反馈闭环（P0，正确性，先行）

目标：让执行结果真正回流到排序器。

| 步骤 | 改动 | 文件 |
|---|---|---|
| 1.1 | `CapabilityIndexer` trait 增加 `update_stats(capability_id, ExecutionDelta)` | harness/src/capability_indexer.rs |
| 1.2 | `CapabilityIndexerImpl` 实现：更新内存护照 + 向量库 metadata | tools/src/capability_indexer_impl.rs |
| 1.3 | stats 落 DB（新表 `capability_stats`），启动时合并进护照 | dao repo + state.rs 启动合并 |
| 1.4 | 三个写回接线点：skill 执行完成、workflow 执行完成、agent 执行完成 | skill_execution.rs / rt-workflow engine / cognitive.rs |

验收：执行 1 次 workflow + 1 次 skill 后 `capability_get_stats` 的 total_calls > 0；探索提权随调用衰减；β 开始区分能力。

### Phase 1.5 — 暴露模式显式化 + 按需注入闭环（P0，执行断链）

目标：主动模式下"发现的能力"真正可执行。

1. 护照加 `exposure: Auto | OnDemand | Managed`（与 kind/domain 正交）
   - Auto：被动全量 + 主动命中注入（轻量 built-in 工具、常用技能）
   - OnDemand：仅发现命中时注入（Tauri 命令桥、重型工具、MCP）
   - Managed：永不自动暴露，仅参与路由（system_*）
2. 护照加 `tool_ref: Option<{toolName, registryKey}>`；新增 `AgentQueryRequest.extra_tools`
3. cognitive.rs 执行分支：命中 → 解析 tool_ref → 从 UnifiedToolRegistry 取 ChatTool → 注入 extra_tools → agent/mod.rs 按名追加（复用现有 extra_names 注入逻辑 mod.rs:1463）
4. 技能双身份解耦：护照加 `exposure_source`（session_enabled / discovery_hit / role_recommended）
5. 被动模式可选进阶：active_domains 全量 → RAR Top-K 子集注入

### Phase 2 — Alias + Toolchain（P1）

- Alias：护照加 `aliases: Vec<String>`；retriever 开头加别名映射，命中直接进候选 + 语义加权
- Toolchain：`CapabilityKind` 加 Toolchain；护照加 `steps: Vec<String>`；命中返回顺序组合，执行失败短路（第一阶段只返回组合，第二阶段接执行器）

### Phase 3 — Template / 策略对象化 / 关联扩展（P2）

- Template：新 kind + `placeholders`；命中只提示"可实例化为 Skill"
- 策略对象化：8 维闸门抽成可注册规则（YAML/DB `capability_policies`），替代硬编码维度
- 关联扩展：护照加 upstream/downstream，检索后一跳扩展（复用 WorkflowGraph 边）

---

## 三、影响面与风险

**影响面**：harness（capability.rs / capability_indexer.rs / retriever.rs / router.rs）、tools（3 个 impl）、src（state.rs / cognitive.rs / capability.rs / agent/mod.rs / skill_execution.rs / work_engine.rs）、前端（capability.ts / CapabilityDiscoveryPanel.tsx）。Phase 1.3 需新 migration。

**风险**：
1. `CapabilityKind` 加枚举值 → 全项目 match 分支编译期强制更新，可控但面广
2. stats 写回点选错 → 数据污染排序（接线点做来源标记）
3. 别名撞名 → 全部进候选由排序裁决，可接受
4. 护照不持久化 → stats 落 DB 解决重启归零

**执行顺序**：Phase 1 → Phase 1.5 → Phase 2 → Phase 3（前序是后序的正确性基础）

---

## 四、落地复盘（2026-08-30 追加）

### 4.1 已落地（全部编译/测试/typecheck 通过）

| Phase | 内容 | 验证 |
|---|---|---|
| P1 反馈闭环 | capability_stats 表（v127）+ 3 写回接线点 + indexer 读时合并 | dao 测试 5/5 |
| P1.5 暴露模式 | 护照 exposure/tool_ref + extra_tools 注入闭环（修复执行断链） | cargo check ✅ |
| P2 Alias+Toolchain | aliases 字段 + 检索加权；Toolchain kind + steps | typecheck ✅ |
| P3 Template/策略/扩展 | placeholders + capability_policies 表（v128）+ filter 集成；upstream/downstream 一跳扩展 | dao 测试 2/2 |
| 遗留① | 主动模式技能按需注入（extra_skills 通道 + handler 注册） | cargo check ✅ |
| 遗留② | Toolchain 组合展开注入（steps → extra_tools） | typecheck ✅ |
| 遗留③ | cognitive_decompose_task（RuleBased 拆解 + 逐项 discover） | fmt/dprint ✅ |

### 4.2 落地 vs 需求差距（逐块）

**分层架构**：
- 覆盖 9/10 对象（知识片段未做）；但**"自上而下检索、高层命中不降级"的粒度分层语义未实现**——检索仍是平铺混合召回，kind 只是过滤字段。保留了原有 P5 保护（非 Workflow clamp Delegate），非规范的粒度降级。

**动态发现流程**：
- 策略过滤 ✅（8 维 + 可注册策略）、关联扩展 ✅（一跳）、排序输出 ✅（数据源修复）
- **意图向量 + 关键实体抽取未做**（{{target_ip}} 占位符无人解析，Template placeholders 是静态元数据）

**10 类对象**：覆盖 8 类（Alias/Toolchain/Template/Policy 新增；Workflow/Plugin/能力组原有）；**未做**：知识片段、Tool 组合路径建议、Skill 前提条件。**半覆盖**：Template 实例化交互提示未接、Toolchain 确定性执行器未做（走 Delegate）、策略规则仅排除型子集。

**交互边界**：6 环节覆盖 5 个（任务拆解用 RuleBased 规则模板，语义拆解受 AGENTS.md 运行时边界禁止）；参数填充未动（与实体抽取缺失联动）。

**五原则**：符合 3 个（模板不执行 / 策略是过滤器 / 插件展开后返回）；**未实现**：分层检索优先上层、能力和信息分离（知识片段缺失）。

### 4.3 差距优先级（后续实施依据）

| 优先级 | 项 | 状态 |
|---|---|---|
| P0 | **分层检索降级**（kind 从过滤字段升级为检索层级：Workflow→Task→Atomic 逐层尝试，高层命中即止） | ✅ 已实施（2026-08-30） |
| P1 | 实体抽取（接 Template placeholders）+ Skill preconditions（接过滤闸门） | ✅ 已实施（2026-08-30） |
| P2 | 知识片段（attached_snippets）+ Tool 组合建议（机制已具备待数据）+ Toolchain 确定性执行器 | ✅ 已实施（2026-08-30） |

### 4.4 优先级实施记录（2026-08-30 追加）

**P0 分层检索降级**：
- harness：`CapabilityLayer`（App=Workflow / Task=Skill+Template+Toolchain / Atomic=Tool+Agent+KB）+ `CapabilityCandidate.layer` + `CapabilityLayer::from_kind`
- tools：`apply_layer_gating`——逐层判定，高层 top1 `retrieval_score ≥ 0.45` 即命中该层（返回层内 top_k，不降级），原子层无条件兜底
- 前端：CapabilityLayer 类型

**P1 实体抽取 + 前提条件**：
- `entity_extractor.rs`（harness）：纯正则提取（ip/date_range/number/email/url/id），5 个单测通过；`CapabilityDiscoveryResult.extractedEntities`，主能力为 Template 时按 placeholders 提取
- preconditions：护照加 `preconditions` 字段 + `FilterContext.satisfied_conditions/conditions_checked` + 过滤维度十 `Preconditions`（默认未启用检查不误杀）

**P2 知识片段 + Toolchain 执行器**：
- 护照加 `attached_snippets: Vec<KnowledgeSnippet{key,content}>`（能力与信息分离载体）
- `cognitive_execute_toolchain` 命令：按 steps 顺序经 `tool_ref → UnifiedToolRegistry::execute`（集成权限/Hook/沙箱）执行，步骤间透传输出，失败短路；错误码 `COGNITIVE_TOOLCHAIN_EXEC_FAILED`
- Tool 组合建议：机制已具备（upstream/downstream + expand_dependencies），待注册数据

**验证**：cargo check 零警告；entity_extractor 单测 5/5；typecheck 通过（本次改动文件）；cargo fmt/dprint 通过。踩坑：ExtractedEntity 与 knowledge_graph 既有类型撞名（改名 CapabilityEntity）；r-string 不能含双引号。
