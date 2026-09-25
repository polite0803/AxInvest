use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "stock_analyses")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub stock_code: String,
    pub stock_name: String,
    pub analysis_date: String,
    pub provider_id: String,
    pub conversation_id: String,
    #[sea_orm(indexed)]
    pub status: String,
    pub decision_action: Option<String>,
    /// 决策持仓状态轴（与 `decision_action` **正交**，v228 引入）：
    /// `EMPTY` / `OPENING` / `HOLDING` / `TRIMMING`。
    ///
    /// 「持有 vs 观望」此前是同一中性档因仓位有无被单向互改的两个名字，
    /// 使 `decision_action` 同时承载「方向强度」与「持仓状态」两个维度。
    /// 本列把持仓状态拆为独立轴。
    ///
    /// `NULL` 语义 = 该记录产生于本字段引入之前，**采集时点没有这个信息** ——
    /// 消费端应按 `decision_position_pct` 自行派生展示，**不得**读成 `EMPTY`
    /// （迁移有意不回填，见 `v228_stock_analyses_decision_position_state`）。
    pub decision_position_state: Option<String>,
    pub decision_position_pct: Option<f64>,
    pub decision_reasoning: Option<String>,
    pub decision_json: Option<String>,
    /// 四周期价位映射（阶段1，PROPOSAL-stock-decision-four-horizon.md）：
    /// 同一决策保留单一 `decision_action`/仓位/时间维度语义，但目标价/止损按四周期
    /// 各给一组绝对价，序列化为 JSON 字符串（与 `decision_json` 同 text 形态）。
    ///
    /// 值形态：`{"ultra_short":{...},"short":{...},"mid":{...},"long":{...}}`，每组含
    /// `stopLossPct`/`takeProfitPct`/`expectedHoldingDays`/`targetPrice`/`stopLoss`。
    /// `NULL` = 该记录产生于本字段引入之前的采集时点，**无此信息**；消费端按主档位
    /// `decision_json` 的 `targetPrice`/`stopLoss` 回退，**不得**读成空映射。
    pub horizon_price_map: Option<String>,
    /// 四周期独立决策（阶段2，PROPOSAL-stock-decision-four-horizon.md）：
    /// 与 `horizon_price_map` 并存的独立决策轴。`horizon_price_map` 只存价位映射，
    /// 本列存每组**完整决策**（action/verdict/positionPct/confidence/stopLossPct/
    /// takeProfitPct/expectedHoldingDays，ultra_short 另含 confLowerBound），序列化为
    /// JSON 字符串。
    ///
    /// 值形态：`{"ultra_short":{...},"short":{...},"mid":{...},"long":{...}}`。
    /// `NULL` = 该记录产生于本字段引入之前的采集时点，**无此信息**；消费端按主档位
    /// `decision_action`/`decision_json` 回退，**不得**读成空映射。
    pub horizon_decisions: Option<String>,
    pub blackboard_snapshot: Option<String>,
    pub config_id: Option<String>,
    /// Time-travel mode: 'live' | 'replay' | 'ab_test'
    #[sea_orm(default_value = "live")]
    pub analysis_kind: String,
    /// Time-travel mode: replay 模式的数据截止日 (YYYY-MM-DD)
    pub as_of_date: Option<String>,
    /// 时间维度: "ultra_short" | "short" | "mid" | "long"
    pub decision_time_horizon: Option<String>,
    /// 期望持有天数（交易日）
    pub decision_expected_holding_days: Option<i64>,
    /// 决策所用 LLM 的版本标识（用于复现实验）
    pub model_version: Option<String>,
    /// 生成该决策时的**工作流模板版本**（取 `workflow_templates.version` 的当时值）。
    ///
    /// 为什么必须落库：模板版本是**决策公式的合法代理** —— `portfolio-mgr.rhai` 等
    /// 脚本经 `include_str!` 嵌入模板，改公式必升 `TEMPLATE_VERSION`
    /// （`seed_stock_analysis.rs` 的版本史注释已立此规矩）。没有这一列时，
    /// 用现行公式复算历史决策会**系统性偏高 4.5pt**（2026-09-18 实测，见
    /// `portfolio-mgr.rhai` 复算注释），使「离线复算」失去意义。
    ///
    /// `NULL` 语义 = 该记录产生于本列引入之前，**采集时点没有这个信息** ——
    /// 复算器见到 NULL 必须声明「公式版本未知」，**不得**默认按当前版本复算。
    /// （与 `decision_position_state` 同约定：引擎只做纯新增、不做 DML，故有意不回填。）
    pub template_version: Option<i32>,
    /// 生成该记录的**工作流模板 id**（完整分析链 `"stock-analysis"` / 快速 JEV 链
    /// `"stock-analysis-fast"`）。判据值即 `workflow_templates.id` 的字面量。
    ///
    /// 为什么必须落库：`template_version` **不能**承担这个职责 —— 它是
    /// `workflow_templates.version` 的当时值，而各模板的 version **各自独立计数**
    /// （实测同库中完整链为 81、快速链为 1），两个数字不可比，用它区分链路是巧合而非判据。
    /// 没有这一列的后果（2026-09-24 实证，300642）：快速链记录与完整链记录在
    /// `stock_analyses` 里形态完全一致（`analysis_kind` 同为 `"live"`），
    /// 「最近分析」查询按 `created_at DESC` 取到的是**更晚写入的快速链记录**，
    /// 于是界面上表现为「快速分析覆盖了完整分析」的完整数据质量结论（D 级被 F 级顶替）。
    ///
    /// `NULL` 语义 = 该记录产生于本列引入之前，**采集时点没有这个信息** ——
    /// 读取侧不得据此推断链路，应按「未知」处理。
    ///
    /// 存量行**刻意不写迁移回填**：本仓 schema 由 entity 驱动（`bootstrap_schema` 负责建列），
    /// 而 `run_migrations` 在它**之前**执行 ⇒ 迁移里引用本列必然失败（列尚不存在）。
    /// 确证为快速链的历史记录（判据 `blackboard_snapshot::jsonb ? 'j-winner'`）
    /// 由一次性 SQL 手工回填，不进版本化迁移。
    pub template_id: Option<String>,
    /// 关联到 L2 disk-cache 的快照 ID
    pub data_snapshot_id: Option<String>,
    /// 决策校验结果：pending / win / loss
    #[sea_orm(indexed)]
    pub outcome: Option<String>,
    /// LLM 决策 JSON（方案 D 双向并存：trader 节点的 `{stance, positionPct, confidence}`）
    ///
    /// ⚠ 原写 `#[sea_orm(default_value = "NULL")]`（2026-09-16 修正）：那渲染成
    /// `DEFAULT 'NULL'`，而本列是 `text`（**不是** `json`）⇒ 落库的会是字面量字符串
    /// `"NULL"`，读取端按 JSON 解析会失败（`serde_json` 只认小写 `null`）。
    /// 「默认为 NULL」的语义就是**不写 DEFAULT 子句**。
    pub llm_decision_json: Option<String>,
    /// 版本化分析：重跑分析时指向原始分析记录的 ID，实现"同一股票多个时间版本"。
    /// 首次分析为 NULL；重跑时指向被重跑的原始记录 ID。
    ///
    /// ⚠ 原写 `#[sea_orm(default_value = "NULL", indexed)]`（2026-09-16 修正）：本列是
    /// `text`，照字面量写会让「首次分析」拿到**字符串 `"NULL"`** —— 一个看起来非空、
    /// 实则指向不存在分析的悬挂引用，与本注释声明的语义正好相反。
    #[sea_orm(indexed)]
    pub parent_analysis_id: Option<String>,
    /// 交易意图审核状态: pending / reviewed / executed / expired / rejected
    #[sea_orm(default_value = "pending", indexed)]
    pub trade_intent_status: String,
    /// 交易意图来源: analysis / conditional_order / quant_signal / portfolio_monitor
    #[sea_orm(indexed)]
    pub trade_intent_source: Option<String>,
    /// 来源关联 ID（分析ID / 条件单ID / 信号ID）
    pub trade_intent_source_ref_id: Option<String>,
    /// 审核时间（ms）
    pub trade_intent_reviewed_at: Option<i64>,
    /// 审核人
    pub trade_intent_reviewed_by: Option<String>,
    /// 审核备注
    pub trade_intent_review_notes: Option<String>,
    /// 关联的实际交易 ID（执行后关联到 trades 表）
    pub trade_intent_actual_trade_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
