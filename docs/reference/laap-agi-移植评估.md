> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# laap-AGI 认知分析报告

> 目标仓库：https://github.com/lorryjovens-hub/laap-AGI
> 分析日期：2026-09-04
> 结论先行：**不建议移植代码，建议只吸收 3 个思想点，用 Rust 原生实现。**

---

## 一、仓库本质判定

LAAP（Living Agent Application Protocol，代号 Aris）自我定位为「零 LLM 认知架构 / 数字生命体的心灵」，README 宣称 25+ 认知模块、不调用一次 LLM 即实现觉知、记忆、推理、想象、成长。

读核心源码后的判定：**这是一个以概念演示为目标的项目，大量模块用数学符号包装简单启发式规则。**

判定依据不是风格偏好，是可验证的代码事实：

| # | 代码事实 | 位置 | 性质 |
|---|---------|------|------|
| 1 | 预测误差 = 正弦波，代码中不存在任何"预测→对照→求差"环节 | `psi_core/engine.py` `_select_attention()` | 致命 |
| 2 | 因果方向由变量名字典序决定（`if var < nb`），与观测数据无关 | `laap/agi/causal.py` `CausalDiscovery.discover()` Step 3 | 致命 |
| 3 | 「量子振幅放大」= `score ** 0.7` 幂函数；「叠加态」= numpy 归一化向量余弦相似度 | `laap/agi/causal.py` `QuantumCausalStore.predict_effect()` | 夸大 |
| 4 | 需求更新 = 指数衰减回基线 `v*0.98 + 0.5*0.02` + 硬编码中文关键词词表（"帮我"/"爱你"/"为什么"） | `psi_core/engine.py` `_update_needs()` | 夸大 |
| 5 | 反事实情绪 = 情绪字符串查表 × 固定系数 0.8 | `laap/agi/causal.py` `CounterfactualEmotion.compute()` | 夸大 |
| 6 | do-calculus 干预 = 改实体状态后重查，比较的是置信度而非概率分布 | `laap/agi/causal.py` `UnifiedCausalEngine.intervene()` | 错误 |

第 1 条单独就足以定性。「预测误差」是整套架构的学习信号源头——如果这个信号是 `sin(cycle)` 生成的，下游所有"学习"都没有信息论意义上的输入。

第 2 条同理：因果发现的全部价值在于**定向**（谁导致谁）。这里定向由字母表决定，`A→B` 成立只因为 `"A" < "B"`。

### 代码规模说明

`laap/agi/causal.py` 共 1623 行，但其中约 40% 是 dataclass 定义 + `to_dict()` 序列化 + 文件末尾的 `test()` / `test_p1_1()` 测试函数。真实推理逻辑的密度远低于行数暗示。

---

## 二、逐模块可用性评估

| LAAP 模块 | 宣称 | 实际 | 对 AxAgent 的价值 |
|-----------|------|------|------------------|
| PSI Core 5 维需求 | 需求动力学驱动注意力 | 指数衰减 + 关键词词表 | △ 思想可借鉴，代码无用 |
| prediction_error | 预测误差学习信号 | sin 波 | ✗ 无价值 |
| QuantumCausalStore | 量子因果编码 | 余弦相似度检索 | ✗ 无价值（search crate 已有向量检索） |
| CausalRule / 物理规则 | 条件→效应模拟 | 5 条硬编码物理常识（倒水/掉落/沸腾） | ✗ 无价值 |
| CausalDiscovery (PC) | 从数据发现因果结构 | 偏相关实现可信，定向有 bug | △ 可参考骨架，需重写定向 |
| **CausalBond** | 贝叶斯因果键 | **增量更新逻辑正确、简洁** | ✓ **值得吸收** |
| **TemporalCausalLink** | 时间因果链 A—[Δt]→B | **BFS + 累计延迟，实现正确** | ✓ **值得吸收** |
| **MultiFactorRule** | 多因素 AND/OR/XOR/WEIGHTED | **激活计算正确** | ✓ **值得吸收** |
| CounterfactualEmotion | 反事实情感 | 字符串查表 | ✗ 无价值 |
| EmotionEngine | 激素 + 镜像神经元 | if/elif 查表 | ✗ 无价值 |
| LongFormSynthesizer | Markov 展开生成长文 | 未验证质量 | ✗ 不建议 |
| Subconscious V12.5 | 马尔可夫-量子直觉 | 概念性 | ✗ 无价值 |

### 2.1 「意识能力」宣称的代码级审查（补遗）

README 的核心卖点是"让 agent 具备意识能力"，实现集中在 `laap/agi/` 四个模块。逐一看代码：

| 模块 | 宣称 | 代码现实 |
|------|------|----------|
| `conscious.py` 意识流 | 第一人称体验流、感质（qualia） | GWT 竞争骨架（权值全为手写常量：`salience×0.4+urgency×0.3+novelty×0.2+emotional_weight×0.1`）；注意力焦点 = 关键词 if-else 链（含 "user"→USER）；**感质的情绪效价 = 37 个英文关键词计数**；自我叙事 = f-string 模板拼接；唤醒度/自我在场感 = 外部手动传入的浮点数 |
| `gw_workspace.py` 全局工作空间 | Baars 竞争-广播 | `activation × salience × recency_bonus` 排序取 top-4 广播，指数衰减。四模块中实现最干净，但本质是**优先级队列 + 发布订阅** |
| `self_model.py` 涌现自我模型 | 从经验涌现自我认知 | 统计聚合：成功率、EMA 自我效能（α=0.1 硬编码，失败钳制到 0.3）、置信度分桶校准曲线（0.1 粒度）；"自传"= significance>0.6 时写入事件列表；自我边界 = **子串匹配**（`can_i_affect` 互为子串→0.6）；含真实 bug：EXPERT 判定短路导致 MASTER 级不可达，`_agency_attributions` 只写不读 |
| `meta_cognitive.py` 元认知监控 | 偏差检测、循环推理识别 | 偏差检测 = 中英文关键词匹配（"我觉得"→情感偏差、"绝对"→过度自信、"上次"→锚定）；**循环推理检测是坏的**：任两个相隔 2 步的推理步骤含同一个英文字母即报循环（`for char in A..Z: if char in step1 and char in step3`），几乎必然误报；深度反思 = 拼 prompt 丢给 LLM |

**判定**：四个模块的共同模式 = **认知科学概念名（GWT / qualia / meta-cognition / self-model）+ 常规工程结构（竞争排序、统计聚合、发布订阅）+ 关键词词表与 f-string 模板填充**。

关键判据——"涌现"不存在：
1. 所有意识参数（唤醒度、自我在场感、显著度）都是外部手动传入，没有任何自主生成机制
2. qualia 在意识哲学中是"主观体验最小单位"的占位概念，用关键词计数实现情绪着色，与宣称的"主观体验"是概念错位而非"弱实现"
3. 自我叙事是模板拼接，"我知道我知道"（know_what_you_know）就是字典遍历

**其中唯一的真实工程洞察**：GWT 的竞争-广播结构对应一个真实的架构问题——当多个主动信号源（预测、提醒、建议）并发争夺系统注意力时，需要统一的显著性仲裁器而非各自为政。这个思想映射到 AxAgent = NudgeService 管线的多信号源仲裁（P3 遗留的 novelty 驱动 tick 落点问题）。当前 AxAgent 主动信号源仅 2-3 个，仲裁需求未到，先不建；信号源增多后再按此骨架实现（数据全部来自真实统计，不设手写权重）。

**对 AxAgent 的结论不变**：意识模块无可移植代码。校准思想（预测置信度 vs 实际结果的分桶统计）与 AxAgent 因果边层的置信度公式 `n/(n+3)` 同构，AxAgent 版本更简洁；偏差检测若未来要做，应基于真实统计（工具成功率 vs 模型自报置信度），而非关键词匹配。

---

## 三、关键发现：AxAgent 已有对标能力，且实现更扎实

这是本次分析最重要的结论。**LAAP 想做的事，AxAgent 大部分已经做了，而且做的是真版本。**

| LAAP 概念 | LAAP 实现 | AxAgent 现有实现 | 胜负 |
|-----------|----------|-----------------|------|
| 内在动机 / 需求驱动 | 5 维需求 + 关键词词表 | `crates/trajectory/src/intrinsic_reward.rs`（431 行）`NoveltyEstimator.compute_novelty()` = `1/(1+count)`、`ProgressTracker` | **AxAgent 胜**（基于新奇度与学习进展，有学术依据） |
| 预测误差 | `0.05 + 0.05*sin(cycle*0.1)` | `crates/trajectory/src/context_predictor.rs`（438 行）`predict()` 返回 `PredictionResult`，可算真实误差 `1 - confidence` | **AxAgent 胜**（有真实预测器，能算真误差） |
| 知识图谱 | 自研因果存储 | `knowledge_relations` 表 + `dao/src/knowledge_graph_provider.rs`（含 `relation_type_filters`） | 平手 |
| 记忆巩固 | 未实现 | `dream_consolidation.rs` | **AxAgent 胜** |
| 学习图 | 无 | `learning_graph.rs`（426 行）`GraphNode`/`GraphEdge`/`LearningGraph` | **AxAgent 胜** |
| 主动行为 | 无 | `proactive_mode.rs` + `nudge.rs` + `proactive_assistant.rs` | **AxAgent 胜** |
| **因果边（带方向与强度）** | CausalBond / TemporalLink | **无** | **LAAP 胜** |

**结论：移植 LAAP 在 6/7 个维度上是降级。真正的增量只有一个——因果边。**

---

## 四、落地方案

### 4.1 唯一推荐落地项：因果边层（Causal Edge Layer）

**核心洞察：不需要新建表。** `knowledge_relations` 已经具备全部所需字段：

```rust
// crates/entities/src/knowledge_relations.rs:10-27
pub relation_type: String,      // 复用：新增 "causes" 取值
pub weight: f64,                // 复用：因果强度
pub properties: Option<Json>,   // 复用：存 confidence / observations / delay
pub source_type: String,        // 复用：标记 "causal_observation"
pub source_id: String,          // 复用：指向 trajectory id
```

`properties` JSONB 载荷设计：

```json
{
  "confidence": 0.62,
  "observations": 14,
  "positive": 11,
  "delay_mean_ms": 1800,
  "delay_std_ms": 400,
  "evidence_refs": ["traj_01H...", "traj_01H..."]
}
```

#### 实施步骤

**Step 1 — 新增模块 `crates/trajectory/src/causal.rs`**

```rust
/// 因果边观测更新（增量贝叶斯，移植 CausalBond 的核心 3 行逻辑）
pub fn observe_causal_edge(
    conn: &DatabaseConnection,
    kb_id: &str,
    cause_entity: &str,
    effect_entity: &str,
    matched: bool,
    observed_delay_ms: Option<i64>,
) -> Result<CausalEdgeStats, TrajectoryError> {
    // 1. upsert knowledge_relations (relation_type="causes")
    // 2. weight += (target - weight) * (1 / observations)   // target = 0.9 | 0.1
    // 3. confidence = min(0.99, n / (n + 3))
    // 4. delay_mean_ms 滚动平均；delay_std_ms 用 Welford 在线算法
}
```

**Step 2 — 时间链预测**

```rust
/// BFS 遍历 causes 边，返回带累计延迟的因果链
pub fn predict_causal_chain(
    conn: &DatabaseConnection,
    from_entity: &str,
    max_depth: usize,
    min_confidence: f64,   // 剪枝阈值，建议 0.35
) -> Result<Vec<CausalChain>, TrajectoryError>
```

**Step 3 — 埋点（3 处，都是已有写入点）**

| 埋点位置 | 观测内容 | 因果语义 |
|---------|---------|---------|
| `crates/trajectory/src/trajectory_impl.rs` 工具调用完成后 | tool_call → outcome（成功/失败） | 「用工具 X 处理 Y 类任务」→「成功/失败」 |
| `crates/trajectory/src/context_predictor.rs` `predict()` 结果落库时 | 预测 → 实际 | 「上下文特征 C」→「下一步动作 A」，延迟 = 实际间隔 |
| `crates/trajectory/src/rl.rs` 奖励信号落地时 | action → reward | 「动作 A 在状态 S」→「获得奖励 R」 |

**Step 4 — 消费点（让因果边真正被用起来）**

| 消费点 | 用途 |
|--------|------|
| `crates/trajectory/src/task_prefetcher.rs` | 用 `delay_mean_ms` 决定预取时机：若 A→B 平均延迟 1.8s，在 A 发生后 1.5s 启动 B 的预热 |
| `crates/trajectory/src/suggestion_engine.rs` | 因果链置信度 > 阈值时，主动建议下一步 |
| `dao/src/knowledge_graph_provider.rs` | `relation_type_filters` 增加 `"causes"` 过滤分支 |

#### Feature flag

按项目惯例加入统一开关，默认关闭：

```rust
// crates/harness 或 config
pub causal_edges_enabled: bool,   // 默认 false
```

---

### 4.2 可选落地项：需求驱动的主动行为调度

现在的 `proactive_mode.rs` 是**空闲计时器**——只看 `last_user_input` 和 `MAX_TICKS_PER_MINUTE: u64 = 6`，到点就注入 tick。

可以升级为**真实信号驱动**，且 AxAgent 已有全部输入：

```
预测误差 = 1.0 - context_predictor.predict(features).confidence
新奇度   = intrinsic_reward.compute_novelty(state_key)

触发条件：novelty > 0.6 && prediction_error > 0.5
抑制条件：novelty < 0.2（重复场景，别打扰用户）
```

这比 LAAP 的做法强在：AxAgent 的预测误差是从**真实预测器**算出来的，不是 sin 波。

⚠️ 注意：此项属于增强现有功能，不是引入新能力，优先级低于 4.1。

---

### 4.3 明确不做的部分

| 不做的部分 | 理由 |
|-----------|------|
| 移植任何 Python 代码 | AxAgent 是 Rust/Tauri 技术栈，引入 Python 子进程不可接受 |
| 量子因果引擎 | 本质是余弦相似度，search crate 已有更成熟的向量检索 |
| 情绪 / 情感 / 激素系统 | 字符串查表，对任务型 Agent 无实际收益 |
| PC 算法因果发现 | 定向逻辑有 bug；且 AxAgent 缺少密集数值观测数据场景，PC 算法无用武之地 |
| LongFormSynthesizer / PaperEngine | Markov 展开生成内容，质量不可控 |
| 整套 PSI 五维需求 | AxAgent 的 intrinsic_reward 已在做同一件事，且更扎实 |

---

## 五、许可风险（必须注意）

LAAP 采用**分层许可**，不是简单的 Apache 2.0：

| 部分 | 许可 | 影响 |
|------|------|------|
| `psi_core/`（Python PSI fallback） | Apache 2.0 | 可自由使用 |
| `aris_brain/`、`laap/agi/`、`laap_brain/`、`mcp_server/` | **BSL 1.1** | **源码可见，非生产免费；生产使用需商业授权；2030-07-15 自动转 Apache 2.0** |
| Rust PSI Core | 商业授权 only | 不可用 |
| `laap-enterprise/` | 商业授权 only | 不可用 |
| `docs/` 论文与架构图 | CC BY-SA 4.0 | 署名 + 相同方式共享 |

⚠️ 仓库根目录 `LICENSE` 曾于 2026-08-06 更新为 **AGPL-3.0** 标准文本，与 README 描述的 Apache 2.0 / BSL 1.1 分层许可存在冲突。使用前必须以仓库最新 LICENSE 系列文件为准。

**对本项目的建议**：只吸收思想（因果边的贝叶斯更新公式、时间延迟链、多因素算子），**不复制任何源码**。这三个点都是教科书级的标准算法，独立实现无许可风险。若复制 `causal.py` 代码，则落入 BSL 1.1 范围，生产使用需商业授权。

---

## 六、价值评估

### 如果落地 4.1（因果边层）

| 价值维度 | 具体收益 | 量级 |
|---------|---------|------|
| 工具选择优化 | 从「历史上用过」升级为「用 A 处理 Y 类任务成功率高且耗时低」，可按预测延迟与成功率排序工具 | 中 |
| 预取时机精确化 | `task_prefetcher` 从固定提前量改为 `delay_mean_ms` 驱动，减少无效预取 | 中 |
| 失败归因 | 工具失败时沿因果链回溯高置信度前驱，定位真正原因而非最后一步 | 中高 |
| 建议质量 | `suggestion_engine` 的建议有因果依据，不再是纯模式匹配 | 中 |
| 可解释性 | 能回答「为什么推荐这个」——因为 A→B 置信度 0.78，观测 14 次 | 高 |
| 碳硅差异 | 知识图谱目前只有"相关"边，加入"因果"边是语义层级跃迁 | 高 |

### 收益风险比

| 项 | 评估 |
|----|------|
| 实施成本 | 低——复用现有表，无新 migration，约 1 个新模块 + 3 处埋点 + 3 处消费 |
| 架构风险 | 低——`relation_type="causes"` 是纯增量，不影响现有 `contains`/`follows` 边 |
| 性能风险 | 低——BFS 深度限 5，边数按 `min_confidence` 剪枝 |
| 收益 | 中高——补上 AxAgent 唯一的语义缺口 |
| **结论** | **值得做** |

### 如果落地 4.2（需求驱动调度）

| 项 | 评估 |
|----|------|
| 实施成本 | 低——两个信号源都已存在 |
| 收益 | 中——减少无效打断，提升主动性时机质量 |
| **结论** | **可做，但排 4.1 之后** |

---

## 七、一句话总结

LAAP 是一个**包装过度、实现薄弱**的概念演示项目：核心学习信号是 sin 波，因果方向由字母表决定，「量子」是余弦相似度。它的 25+ 模块里，AxAgent 已经以更扎实的方式实现了其中绝大部分（内在动机、上下文预测、记忆巩固、学习图、主动行为）。

但它有一个 AxAgent 确实没有的东西：**带方向、强度、时间延迟和置信度的因果边**。这是唯一值得吸收的增量，且可以零新表落在现有 `knowledge_relations` 上，成本很低。

**建议：不移植代码，只取因果边这一个点，用 Rust 原生实现，约 1 个新模块即可。**
