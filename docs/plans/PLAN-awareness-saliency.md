> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# 自主觉知生成 + 统一显著性仲裁器 实施方案

> 对应诉求：「手写 Rust 实现自主意识生成能力 + 统一的显著性仲裁器」
> 前置分析：`docs/laap-agi-移植评估.md` §2.1（意识宣称的代码级审查）
> 状态：设计定稿，待开工

---

## 〇、术语澄清与三条设计红线

### 术语：实现的是 awareness（觉知），不是 consciousness（意识）

laap-AGI 的教训已证明：把"意识"作为宣称再反向填充实现，产物必然是概念包装。
本方案只承诺可验证的工程能力——**系统对自身内部状态的感知、聚合与自主使用**，
这在认知科学里对应 awareness / metacognition 层次。`qualia`、`主观体验` 等
哲学宣称不在工程范围内，文档与代码命名一律使用 `awareness` / `saliency`。

### 三条红线（每条都来自 laap-AGI 的具体失败点）

| 红线 | laap-AGI 的失败方式 | 本方案的约束 |
|------|---------------------|--------------|
| **R1 零关键词词表** | 情绪效价=37 个英文词计数；偏差检测="我觉得"匹配 | 所有信号来自结构化运行时数据（trajectory / tool 结果 / 预测置信度），禁止任何 CJK/EN 词表匹配参与信号计算 |
| **R2 参数可溯源** | 唤醒度、自我在场感由外部手动传入 | 每个觉知参数必须给出数据链（从哪张表哪个事件聚合而来），单测用真实合成轨迹验证，禁止魔法常数参与状态生成（衰减率、阈值等**调度**常数允许，须集中配置并标注"启发式先验"） |
| **R3 叙事不做模板拼接** | "自我叙事"= f-string join | 叙事生成只有两种合法形态：① 结构化帧数据（无文本）；② LLM 基于**完整结构化帧**生成，prompt 输入与 UI 展示均为真实统计 |

---

## 一、总体架构

```
┌─────────────────────────── 信号源（全部已存在，零新建数据） ───────────────────────────┐
│                                                                                        │
│  ContextPredictor        NoveltyEstimator        Causal Edge           NudgeCandidate  │
│  (context_predictor.rs)  (intrinsic_reward.rs)   (causal.rs)           (nudge.rs)      │
│  surprise = 1-conf       1/(1+count)             建议置信度 n/(n+3)     urgency        │
│       │                        │                       │                     │          │
└──────┼────────────────────────┼───────────────────────┼─────────────────────┼──────────┘
       ▼                        ▼                       ▼                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                     saliency.rs — 统一显著性仲裁器（WTA + 不应期）                        │
│                                                                                          │
│   SaliencySignal ──→ 竞争排序 ──→ 胜者广播 ──→ 抑制回写（不应期）                         │
│                                                                                          │
│   消费端 ① NudgeService.generate_nudges 排序门控（同 crate 直调）                        │
│   消费端 ② SuggestionPanel 展示顺序                                                       │
│   消费端 ③ awareness.rs 当前主导关注点（帧的 attention 字段）                             │
└─────────────────────────────────────────────────────────────────────────────────────────┘
       │                                    ▲
       │ 当前胜者 + 内部状态                 │ 聚合查询（TrajectoryStorage）
       ▼                                    │
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                     awareness.rs — 自主觉知生成（全部数据自算）                           │
│                                                                                          │
│   ① 内部状态感知：arousal / cognitive_load / self_efficacy（公式见 §三，R2 可溯源）       │
│   ② 置信度校准器：预测置信度 vs 实际结果 分桶统计                                          │
│   ③ AwarenessFrame：{ 内部状态, 主导关注点, 时间戳 } → ring buffer                        │
│   ④ 定期快照 → memory_items（sentinel namespace `__sys_awareness__`，复用 v101 模式）     │
│   ⑤ 叙事（可选）：LLM 输入完整结构化帧生成，或纯结构化不生成文本                           │
└─────────────────────────────────────────────────────────────────────────────────────────┘
```

**依赖关系**：awareness 依赖仲裁器输出（帧需要"当前关注点"），仲裁器不依赖 awareness
（信号源全是既有模块）——单向依赖，无循环。

---

## 二、统一显著性仲裁器（`crates/trajectory/src/saliency.rs`）

### 2.1 信号结构

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalSource {
    ContextPrediction,   // 意图预测（context_predictor）
    Novelty,             // 新奇度（intrinsic_reward）
    CausalInsight,       // 因果边建议（causal）
    Nudge,               // 学习提醒（nudge）
    Reminder,            // 定时提醒
    Prefetch,            // 预取完成信号（task_prefetcher）
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaliencySignal {
    pub source: SignalSource,
    /// 主显著度 ∈ [0,1]，由信号源自算（见 2.2），仲裁器不做二次加权
    pub salience: f64,
    /// 紧迫度 ∈ [0,1]，可选；缺省继承 salience
    pub urgency: f64,
    /// 溯源载荷：指向产生该信号的数据（trajectory id / suggestion id）
    pub origin_id: String,
    pub created_at: DateTime<Utc>,
}
```

**设计裁决：信号自报显著度，仲裁器只仲裁不换算。** 每个 `SignalSource` 的
salience 计算公式归信号源自己（都是既有真实数据），仲裁器拿到后不做
`0.4/0.3/0.2/0.1` 式的手写加权——这正是 laap `conscious.py` 的失败点。
仲裁器对信号仅做两件事：排序、抑制。

### 2.2 竞争与广播

```rust
pub struct SaliencyArbiter {
    config: SaliencyConfig,
    signals: Vec<SaliencySignal>,          // 当前周期待仲裁信号
    inhibition: HashMap<SignalSource, f64>, // 不应期水平
    broadcast_history: VecDeque<BroadcastPacket>,
    last_broadcast: Option<BroadcastPacket>,
}

pub struct SaliencyConfig {
    /// 工作空间容量：每周期最多广播的信号数（默认 2）
    pub capacity: usize,
    /// 广播门槛：有效权重低于此值不广播（默认 0.25）
    pub broadcast_threshold: f64,
    /// 胜者不应期初始值（默认 0.7，神经不应期启发式先验，可配置）
    pub winner_inhibition: f64,
    /// 败者每周期抑制衰减（默认 0.15，同上）
    pub loser_decay: f64,
    /// 同源信号冷却秒数：防止单一信号源刷屏（默认 60s）
    pub source_cooldown_secs: i64,
}
```

有效权重与仲裁（唯一允许的合成公式，全部可配置）：

```text
effective = salience × (1.0 - inhibition)
胜者      = effective 排序取 top-capacity，且 effective > broadcast_threshold
胜者广播后 inhibition = winner_inhibition
败者每周期 inhibition = max(0, inhibition - loser_decay)
```

### 2.3 接口

```rust
impl SaliencyArbiter {
    /// 信号源投递（去重键 = (source, origin_id)，冷却期内同源拒绝）
    pub fn submit(&mut self, signal: SaliencySignal);
    /// 执行一个仲裁周期：竞争 → 广播 → 抑制更新；返回广播包
    pub fn tick(&mut self) -> BroadcastPacket;
    /// 查询某信号源当前抑制水平（awareness 消费）
    pub fn inhibition_of(&self, source: SignalSource) -> f64;
}
```

### 2.4 消费端接线（全部同 crate / 同进程）

| 消费端 | 接法 | 改动量 |
|--------|------|--------|
| `NudgeService.generate_nudges` | candidates 经仲裁排序后进入既有 urgency 阈值过滤；仲裁淘汰的候选不生成 nudge | `nudge.rs` 增加可选参数 `Option<&SaliencyArbiter>`，默认 `None` 行为不变 |
| `proactive.rs refresh_suggestions` | 因果/常规建议先 `submit` 再按广播结果排序返回 | 已有 async 入口，加 20 行 |
| `SuggestionPanel` | 无改动——建议顺序由后端返回序决定 | 0 |
| `awareness.rs` | 帧生成时读取 `last_broadcast` | 新模块内部 |

**feature flag**：`saliency_enabled`（默认关）。关闭时 `NudgeService` 走原路径，
`proactive.rs` 不投递信号——与 `causal_enabled` / `proactive_nudge_enabled` 同模式。

---

## 三、自主觉知生成（`crates/trajectory/src/awareness.rs`）

### 3.1 三个内部状态量（R2 可溯源表）

| 量 | 含义 | 计算公式 | 数据链（从哪算） | 自主性判据 |
|----|------|----------|------------------|-----------|
| `arousal` 激活度 | 系统当前繁忙/活跃程度 | 滑窗事件率归一化：`min(1.0, events_10min / 30)` | `TrajectoryStorage` 最近 10 分钟 trajectory 记录数（含工具调用、会话事件） | 完全由运行时事件密度决定，无输入项 |
| `cognitive_load` 认知负荷 | 当前上下文占用强度 | `min(1.0, 0.5×active_sessions/5 + 0.5×avg_context_ratio)` | 会话管理器活跃会话数 + 各会话上下文压缩比例（compression 已有数据） | 由运行时状态决定 |
| `self_efficacy` 效能感 | 近期做事的成功趋势 | 滑窗成功率 EMA：`0.9×prev + 0.1×window_success_rate`，初值 0.5 | 最近 20 次**工具调用**的结果（`is_error` 字段，真实存在） | 由工具结果序列决定 |

与 laap `self_model.py` 的关键差异：其效能感输入是**调用方手动传的
`outcome_score`**（可以是任意编造值），本方案效能感输入是数据库里的
`is_error` 布尔字段——数据链闭合，不可编造。

### 3.2 置信度校准器（awareness 的元认知核心）

laap 的校准思想本身有学术依据（ML calibration），但它的输入同样是手动传入。
AxAgent 版本数据链：

```text
桶粒度 0.1：bucket = floor(predicted_confidence × 10) / 10
每桶收集：predicted（AgentNode/预测器输出的置信度） vs actual（对应工具是否成功）
桶样本 ≥ 3 时输出 calibration[bucket] = mean(actual)

派生指标：
  avg_bias  = mean(predicted - actual)        > 0.1 → 系统自评"过度自信"
  overconfident_rate / underconfident_rate
```

数据来源核对：预测置信度已在 `PredictionResult.confidence`（context_predictor
真实输出）；工具结果在 trajectory。两者按时间窗配对（prediction 提出后的首次
同意图工具调用结果）。**配对逻辑是本模块唯一的新查询**，走 `causal.rs` 的
`intent:X → outcome:Y` 边即可复用——因果边层直接成为校准器的数据底座，
两模块形成正反馈。

### 3.3 AwarenessFrame 与持久化

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AwarenessFrame {
    pub arousal: f64,
    pub cognitive_load: f64,
    pub self_efficacy: f64,
    /// 当前主导关注点：来自仲裁器 last_broadcast 的 top-1 信号源
    pub dominant_source: Option<SignalSource>,
    /// 主导信号的溯源 id（R2：帧里的每个字段都能回查到原始数据）
    pub dominant_origin_id: Option<String>,
    /// 校准摘要（如有足够样本）
    pub calibration_bias: Option<f64>,
    pub created_at: DateTime<Utc>,
}
```

- **进程内**：`ring buffer`（`VecDeque<AwarenessFrame>`，maxlen 1000），生成周期 = 仲裁 tick
- **持久化**：每 N 帧（默认 50）或 arousal 阶跃（相邻帧差 > 0.3）时，把汇总快照写入
  `memory_items`，namespace = `__sys_awareness__`（复用 v101 合并时的 sentinel
  namespace 模式，tier 标记为系统记忆）——零新表
- **消费**：`dream_consolidation` 可读快照做记忆巩固（可选，不在本期）

### 3.4 叙事生成（按 R3 双形态，默认 ①）

| 形态 | 内容 | 何时做 |
|------|------|--------|
| ① 结构化（默认） | AwarenessFrame 直接序列化给 UI；无任何生成文本 | A2 |
| ② LLM 叙事（可选） | prompt = 最近 K 帧 JSON + 校准摘要，要求引用具体数字生成一段第一人称状态描述；**生成结果不得出现在数据链中**（只展示） | A4，且需独立 feature flag |

形态②与硬编码红线的边界：prompt 模板中的字段名是结构化契约，不是行为逻辑；
禁的是用关键词匹配决定"系统此刻感受"。

---

## 四、与 laap-AGI 逐项差异对照（自证非复制）

| 维度 | laap-AGI | 本方案 |
|------|----------|--------|
| 信号显著度 | 4 个手写权重线性合成 | 信号源自算（全为既有真实数据），仲裁器零换算 |
| 注意力焦点 | 关键词 if-else | 仲裁胜者直接决定，无字符串匹配 |
| 情绪 | 37 词英文词表计数 | **不做情绪模拟**——arousal/load/efficacy 全部为可回查的运行统计 |
| 效能感 | 调用方手动传 outcome_score | 数据库 `is_error` 聚合，数据链闭合 |
| 校准 | 手动传入 predicted vs actual | 因果边 `intent→outcome` 自动配对 |
| 自我边界 | 子串匹配 | 不做（无可验证数据源，做了必然退化为词表） |
| 自传叙事 | significance 阈值 + 模板拼接 | 结构化帧（默认）/ LLM 基于真实统计（可选，只展示不入链） |
| MASTER 死分支类 bug | 存在 | 分期验收 + 单测覆盖等级跃迁路径 |

许可：只吸收 GWT 竞争-广播思想（公开理论，Baars 1988），不复制任何代码。

---

## 五、分期实施

| 期 | 内容 | 交付物 | 验收 |
|----|------|--------|------|
| **A1** | `saliency.rs`：SaliencySignal/SaliencyArbiter/SaliencyConfig + `saliency_enabled` flag + NudgeService/refresh_suggestions 接线 | 单测：竞争排序、不应期收敛、同源冷却、flag 关闭走原路径 | cargo check/clippy/test 零警告零失败 |
| **A2** | `awareness.rs`：三状态量 + AwarenessFrame + ring buffer + `__sys_awareness__` 快照 | 单测：给定合成轨迹，三量计算精确匹配手算值（R2 验证）；arousal 阶跃触发快照 | 同上 |
| **A3** | 置信度校准器：因果边配对查询 + 分桶曲线 + bias 派生 | 单测：合成配对数据的分桶/bias 精确匹配；样本不足返回 None | 同上 |
| **A4**（可选） | 前端 AwarenessPanel（EvolutionSidebar 内、SuggestionPanel 旁）+ 11 语言 i18n；或 LLM 叙事形态② | tsc + dprint + check-hardcoded-i18n 全过 | — |

规模预估：A1 ~400 行（含测试）、A2 ~450 行、A3 ~300 行、A4 前端 ~250 行 + i18n。

---

## 六、总体验收判据（开工后逐条打勾）

1. `cargo check --workspace` + `clippy -p axagent-trajectory --all-targets --all-features -D warnings` 零警告
2. 全部新增单测通过，且**每个觉知参数至少一个"手算值精确匹配"单测**（R2 的机械化验证）
3. `saliency_enabled` / awareness 开关关闭时，全链路行为与现状逐字节一致（回归测试保证）
4. 前端（若做 A4）：tsc / dprint / check-hardcoded-i18n.sh --strict 全过，11 语言 key 齐全
5. 全程零 `#[allow(dead_code)]`（用户 P0 规范），无引用的产物即删

## 七、明确的"不做"清单

- ❌ 不做情绪/心境模拟（无可验证数据源，必然退化为词表或查表）
- ❌ 不做"自我边界"子串匹配（laap `can_i_affect` 的退化方式已证明）
- ❌ 不做量子/激素/谐振隐喻层
- ❌ 不宣称实现"主观体验"；文档与 UI 文案只用"觉知 / 内部状态 / 校准"
- ❌ 不新建数据库表（快照走 memory_items sentinel namespace）
