use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "stock_reflections")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub stock_code: String,
    pub stock_name: String,
    /// 原始分析的 ID（关联 stock_analyses.id）
    #[sea_orm(indexed)]
    pub original_analysis_id: String,
    /// as-of 时间（原始分析日期，YYYY-MM-DD）
    pub as_of_date: String,
    /// 后见信息时间（校验/反思触发日期，YYYY-MM-DD）
    pub hindsight_date: String,
    /// 反思触发时的置信度阈值
    pub min_confidence_threshold: i32,
    /// 反思深度：light | deep（deep 会详述 reasoning chain）
    pub reflection_depth: String,
    /// 本次反思**复盘的持有周期档**（〇-B v2 第 4 条：一次反思 = 一个周期）。
    ///
    /// 值域 = `axagent_harness::holding_period::Period::as_str()`；由反思链路按
    /// 「原分析的主档」盖章写入（不是 LLM 自报）。
    ///
    /// 为什么必须落库：`lesson` / 参数建议都只在**本档**有效，无此列时
    /// 教训与档位参数建议会被后续任意档的分析共同引用 ⇒ 跨周期污染。
    /// `NULL` 语义 = 本列引入前的记录，复盘档未知（读侧**不得**当作某档使用）。
    #[sea_orm(indexed)]
    pub horizon: Option<String>,
    /// 实际走势描述，如 "30天跌-8.3% → 失败"
    pub actual_outcome: String,
    // ── v008 升级（借鉴 TradingAgents 反思机制）──
    // 4 个结构化 outcome 变量:C3 借鉴。让 LLM 反思时直接引用"持仓 30 天跌
    // 8%,相对沪深 300 超额 -2.1%"这样的硬数字,避免 LLM 脑补。
    /// 原始收益率(%)。None=回退到 actual_outcome 自然语言。
    pub raw_return: Option<f64>,
    /// 相对基准的超额收益(%)。None=未算 alpha。
    pub alpha_return: Option<f64>,
    /// 实际持有天数。None=未到反思点(pending row)。
    pub holding_days: Option<i32>,
    /// #10 P7 妖股标签（**按档**挂在逐档反思行上，与 `horizon` 同一行同一窗口）。
    ///
    /// 口径 = **分析价 → 反思价的原始涨跌幅**（`MarketSnapshot::price_change_pct`：入场=分析日之后
    /// 首个交易日开盘价，退出=反思时点最新收盘价）。两条**不是**本判据：档内逐日收盘累计
    /// （`mover_recall::window_cum_gain_pct`）量的不是「从分析到反思」；`net_return_pct` 扣了双边
    /// 成本，而阈值是给涨幅定的（超短 10%/短 20%/中 30%/长 40%，唯一权威 = 面板变量
    /// `mover_gain_*`，见 `mover_recall::DEFAULT_GAIN_THRESHOLDS`）。
    ///
    /// 取值（**NULL 不复用**，每种缺席各占一句，避免「拿不到」冒充「算过且没达标」）：
    /// - `"mover"` / `"normal"` = 窗口已满且判据可用，达标 / 未达标；
    /// - `"window_incomplete"` = 持有期未满 ⇒ **不判定**（面板留空并注明未满，不拿当前价冒充到期价）；
    /// - `"no_market_data"` = 到反思点了但该档行情快照不可得；
    /// - `"rule_unavailable"` = 该档阈值判据不可用（变量缺失/非正，或变量表读取失败 —— 两者都由
    ///   日志分述，列上都是「无从判定」，都不得写成 `normal`）；
    /// - `NULL` = 本列引入前的存量行，或该行还没走到反思收尾（pending/running/failed）。
    pub mover_label: Option<String>,
    /// 基准名称,如"沪深300"/"中证500"。None=未指定。
    pub benchmark_name: Option<String>,
    // 3 个 C2 借鉴短文本输出。
    /// 反思判定:correct / partial / wrong 三选一
    pub verdict: Option<String>,
    /// 反思中引用的关键 alpha/信号
    pub alpha_cited: Option<String>,
    /// ≤200 字符、≤2 句简短总结(C1 强制短文本)
    pub lesson_summary: Option<String>,
    /// 反思摘要：错因
    pub what_went_wrong: Option<String>,
    /// 反思摘要：被忽视的信号（JSON 数组字符串）
    pub missed_signals: Option<String>,
    /// 反思摘要：改进建议
    pub fix_for_future: Option<String>,
    /// 反思 agent 输出的参数调整建议（params_suggestion JSON 数组字符串）
    pub parameter_suggestions_json: Option<String>,
    /// 四周期反思结果 JSON（schemaVersion=1）；NULL 表示历史单周期记录。
    pub horizon_results_json: Option<String>,
    /// portfolio-manager 完整输出 JSON
    pub decision_json: Option<String>,
    /// 工作流完整结果（用于追溯）
    pub blackboard_snapshot: Option<String>,
    /// 本行所**复盘的那条分析**所属的算法代际（= `stock_analyses.template_version` 的建点副本）。
    ///
    /// 为什么是「溯源戳」而不是第二份权威：权威仍是 `stock_analyses.template_version`
    /// （单一写入口径见 `stock_workflow/core.rs` 的 `template_version: Set(Some(loaded.version))`），
    /// 这里在建 pending 行那一刻把**被复盘那条**的代际抄下来。抄的理由与 `horizon` 同一条：
    /// 错题本的读侧（注入与统计）要按代筛样，而反思行与分析行的两跳 join 会把
    /// 「分析已重跑、代际已变」的中间态算进历史样本 —— 归属必须在事实发生的那刻钉住。
    ///
    /// NULL 的两种来源（读侧不得混为一谈，也不得当成「第 0 代」）：
    /// ① 本列引入前的存量行；② 对话直执行通道 —— 那条链不经过 `workflow_templates`，
    /// 其分析行的 `template_version` 本身就是显式 NULL（`hooks.rs` 的 A4 注释已裁定）。
    /// 值域 = `workflow_templates.version` 整数，不得另造口径。
    pub template_version: Option<i32>,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::ActiveValue;

    #[test]
    fn horizon_results_json_is_a_nullable_persistence_column() {
        assert_eq!(Column::HorizonResultsJson.as_str(), "horizon_results_json");
        let active =
            ActiveModel { horizon_results_json: ActiveValue::Set(None), ..Default::default() };
        assert!(matches!(active.horizon_results_json, ActiveValue::Set(None)));
    }
}
