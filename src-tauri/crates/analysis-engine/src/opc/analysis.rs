// OPC 域包分析/决策层
// 对齐 stock-analysis 的分析引擎，实现域包隔离的分析回合、决策和风控

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use axagent_harness::self_improving_loop::{
    NextAction, RoundEvaluation, RoundResult, RoundStep, SelfImprovingRound,
};

use super::data_service::OpcDataService;
use super::error::OpcResult;

// ── OpcCapabilityPackDecision ────────────────────────────────────────

/// 域包分析决策
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpcCapabilityPackDecision {
    pub domain_pack_id: String,
    pub decision_type: DecisionType,
    pub summary: String,
    pub confidence: f64,
    pub kpis: Vec<super::analytics::KpiValue>,
    pub recommendations: Vec<String>,
    pub risk_level: RiskLevel,
}

/// 决策类型。
///
/// `rename_all = "snake_case"`：前端 `types.ts` 的值域与 `DomainComponents.tsx`
/// 的比对均为小写，且 `snake_case` 对单词变体（`low`/`high` 等）与 `lowercase`
/// 输出一致、对多词变体（`performance_review`）可读性更好，故两个枚举统一用
/// `snake_case`。取值域未被持久化（无表列 / 无 JSON 列 / 无缓存文件），
/// 因此无需 `alias` 兼容旧 PascalCase 值。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionType {
    PerformanceReview,
    KpiAnalysis,
    TrendForecast,
    RiskAssessment,
    StrategicPlanning,
}

/// 风险等级（取值域 `low` / `medium` / `high` / `critical`，与风险门控判定一致）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

// ── 创作类 KPI 的风控管辖范围 ───────────────────────────────────

/// 域包 → 受风控管辖的 KPI 键集（顺序照 `runtime.yaml` 的 `kpi_definitions`）。
///
/// **管辖范围 = 该域包声明的全部 7 个键**。管辖 ≠ 生效：一个受管键是否真的参与判定，
/// 取决于它是否在 `runtime.yaml` 里声明了 `risk.min` / `risk.max`
/// （见 [`DeclaredKpiThreshold::is_effective`]）。当前只有 `word_count` /
/// `completion_rate` 声明了阈值 ⇒ `Critical` 判据的分母 = 2。
///
/// **为什么必须按域包限定**：`completion_rate` 在多个域包中同名
/// （content_media / security / project_management / design / geospatial /
/// game_dev / software_dev），裸键名匹配会**跨域包误伤**。形态照抄
/// `capability_pack_kpi_service::KPI_SOURCE_REGISTRY`：新增域包只加一行，不改分支逻辑。
///
/// **为什么 5 个受管键刻意不声明阈值**（阈值必须可举证，宁缺勿造；未声明 ⇒ 不参与
/// 判定，仅 `warn` 留痕）：
/// - `content_count` / `page_views`：计算源是聚合查询（`count_blog_posts` /
///   `sum_blog_post_views`），**空结果也是 `Available` 且值为 0** ⇒ 任何 `min` 都会把
///   「本月还没发内容」判成违规（假报）；
/// - `conversion_rate`：`ContactConversionRate` = 联系数 / 浏览数 × 100，取值域随业务
///   量浮动，代码内没有任何权威下限/上限可引；
/// - `content_engagement`：恒 `NoDataSource`（`capability_pack_kpi_service.rs` 的
///   `CONTENT_MEDIA_KPI_SOURCES`），声明阈值也永远不会生效 —— 留着只会制造
///   「配了却没跑」的错觉；
/// - `revision_rounds`：产出被代码限死为 {0, 1}，任何界限都无依据（详见 runtime.yaml）。
pub const RISK_SCOPED_KPI_KEYS: &[(&str, &[&str])] = &[(
    "content_media",
    &[
        "content_count",
        "page_views",
        "conversion_rate",
        "content_engagement",
        "word_count",
        "completion_rate",
        "revision_rounds",
    ],
)];

/// 按域包取受风控管辖的 KPI 键集。域包 id 按下划线归一（`content-media` ≡
/// `content_media`）；未登记域包返回**空集**（不参与创作类风控）。
pub fn risk_scoped_kpi_keys(domain_pack_id: &str) -> &'static [&'static str] {
    let id = domain_pack_id.replace('-', "_");
    RISK_SCOPED_KPI_KEYS
        .iter()
        .find(|(ind, _)| *ind == id.as_str())
        .map(|(_, keys)| *keys)
        .unwrap_or(&[])
}

/// 域包 `runtime.yaml` 为单个 KPI 声明的风控阈值。
///
/// 由命令层从域包解析后注入 —— `analysis-engine` **不读 YAML**，维持与 KPI
/// 元数据同一条边界（YAML 管声明，Rust 管判定，唯一契约是 `key`）。
#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredKpiThreshold {
    pub key: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

impl DeclaredKpiThreshold {
    /// 是否至少声明了一个界限。`min` / `max` 都缺 ⇒ 该条声明无效，视同未声明。
    pub fn is_effective(&self) -> bool {
        self.min.is_some() || self.max.is_some()
    }
}

// ── OpcRiskGate ─────────────────────────────────────────────────

/// 域包风控门控（对齐 position_limits）。
///
/// 判定分两类：
/// 1. **通用规则**（既有）：`expense_ratio` 上限 / `customer_satisfaction` 下限，
///    阈值仍为下方两个字段的既有取值（**本次不动**，属既有状态）；
/// 2. **域包声明规则**：`runtime.yaml` 为受管键声明的 `min` / `max`，受管键集见
///    [`RISK_SCOPED_KPI_KEYS`]。
///
/// 两条硬约束：
/// - **只有 `availability == Available` 的 KPI 参与判定**（见 [`Self::check`]）；
/// - **未声明阈值 = 不启用**，不得用默认值兜底（默认值就是硬编码的第二种形态），
///   但构造时会 `warn` 留痕。
#[derive(Debug, Clone)]
pub struct OpcRiskGate {
    domain_pack_id: String,
    max_monthly_expense_ratio: f64,
    min_customer_satisfaction: f64,
    /// 域包声明的阈值；未列出的受管键不参与判定。
    declared: Vec<DeclaredKpiThreshold>,
}

impl OpcRiskGate {
    /// 不注入任何域包声明阈值（仅通用规则生效）。
    pub fn new(domain_pack_id: &str) -> Self {
        Self::with_declared_thresholds(domain_pack_id, Vec::new())
    }

    /// 注入域包 `runtime.yaml` 声明的阈值。
    pub fn with_declared_thresholds(
        domain_pack_id: &str,
        declared: Vec<DeclaredKpiThreshold>,
    ) -> Self {
        let gate = Self {
            domain_pack_id: domain_pack_id.to_string(),
            max_monthly_expense_ratio: 0.7,
            min_customer_satisfaction: 3.5,
            declared,
        };
        gate.warn_undeclared_scoped_keys();
        gate
    }

    /// 受管键集内「未声明」或「声明无效」的键 ⇒ `warn` 留痕。
    ///
    /// 「未声明 = 不启用」是唯一不会伪造的语义（阈值必须有据）；但一段被声明为
    /// 受管却整段不生效的配置必须可观测，否则又是一处静默口。
    ///
    /// 一次构造**只打一行**（列出全部未声明键）：门控在仪表盘路径上按请求构造
    /// （见 `commands::opc_capability_pack_runtime::get_dashboard`），7 键里 5 键默认无阈值，
    /// 逐键各打一行会把一次面板刷新变成 5 行日志。
    fn warn_undeclared_scoped_keys(&self) {
        let undeclared: Vec<&str> = risk_scoped_kpi_keys(&self.domain_pack_id)
            .iter()
            .copied()
            .filter(|key| !self.declared.iter().any(|d| d.key == *key && d.is_effective()))
            .collect();
        if undeclared.is_empty() {
            return;
        }
        tracing::warn!(
            capability_pack = %self.domain_pack_id,
            kpis = ?undeclared,
            "[opc-risk-gate] 这些 KPI 属于本域包受管键集，但 runtime.yaml 未声明 risk 阈值 \
             ⇒ 本次不参与风控（未声明 = 不启用）"
        );
    }

    /// 风控检查结果
    pub async fn check(&self, kpis: &[super::analytics::KpiValue]) -> OpcResult<RiskCheckResult> {
        use super::analytics::KpiAvailability;

        let mut violations = Vec::new();
        let scoped = risk_scoped_kpi_keys(&self.domain_pack_id);
        // 受管键上「命中声明阈值」产生的违规条数（`Critical` 判据的分子）。
        // 单独计数而**不**从 `violations` 里反查 `rule` 前缀：静态规则
        // （`expense_ratio` / `customer_satisfaction`）与声明规则是两组独立规则，
        // 不得混进同一个分子（混了就会把「费用率超限」算成「创作键违规」）。
        let mut scoped_hits = 0usize;

        for kpi in kpis {
            // 前置守卫（对**全部**规则统一生效）：只有 `Available` 才是真实值。
            // `Empty` / `NoDataSource` 下 `value` 是占位 0.0（见 `capability_pack_kpi_service`
            // 的 `resolve_kpi_value`），据此判定等于把「工作流还没跑过」判成
            // 「字数 0 < 下限」= 伪造数据。
            if kpi.availability != KpiAvailability::Available {
                continue;
            }

            match kpi.key.as_str() {
                "expense_ratio" if kpi.value > self.max_monthly_expense_ratio * 100.0 => {
                    violations.push(RiskViolation {
                        rule: "max_monthly_expense_ratio".to_string(),
                        current: kpi.value,
                        threshold: self.max_monthly_expense_ratio * 100.0,
                        message: format!(
                            "费用率 {:.1}% 超过阈值 {:.0}%",
                            kpi.value,
                            self.max_monthly_expense_ratio * 100.0
                        ),
                    });
                },
                "customer_satisfaction" if kpi.value < self.min_customer_satisfaction => {
                    violations.push(RiskViolation {
                        rule: "min_customer_satisfaction".to_string(),
                        current: kpi.value,
                        threshold: self.min_customer_satisfaction,
                        message: format!(
                            "客户满意度 {:.1} 低于阈值 {:.1}",
                            kpi.value, self.min_customer_satisfaction
                        ),
                    });
                },
                // 受管键：阈值**只能**来自域包声明，未声明则整条跳过。
                key if scoped.contains(&key) => {
                    let Some(d) = self.declared.iter().find(|d| d.key == key && d.is_effective())
                    else {
                        continue;
                    };
                    if let Some(min) = d.min.filter(|min| kpi.value < *min) {
                        violations.push(RiskViolation {
                            rule: format!("min_{key}"),
                            current: kpi.value,
                            threshold: min,
                            message: format!("{key} 当前 {:.1} 低于下限 {:.1}", kpi.value, min),
                        });
                        scoped_hits += 1;
                    }
                    if let Some(max) = d.max.filter(|max| kpi.value > *max) {
                        violations.push(RiskViolation {
                            rule: format!("max_{key}"),
                            current: kpi.value,
                            threshold: max,
                            message: format!("{key} 当前 {:.1} 超过上限 {:.1}", kpi.value, max),
                        });
                        scoped_hits += 1;
                    }
                },
                _ => {},
            }
        }

        // ── 等级映射 ─────────────────────────────────────────────
        // `Critical` = 「本域包受管键**全**违规」：
        //   分母 = 已声明生效阈值的受管键数（`effective_scoped`，今天 = 2）；
        //   分子 = 受管键上按声明阈值判出的违规条数（`scoped_hits`，今天最多 2）。
        // 两组规则独立计数：静态规则既不进分母也不进分子。
        // `effective_scoped == 0`（该域包没有任何生效的受管阈值）⇒ **不可 Critical**，
        // 否则「一个阈值都没配」会变成最高风险。其余：命中 1 条 ⇒ Medium，
        // ≥2 条但未覆盖全部受管键 ⇒ High。
        //
        // 注意：分子按**违规条数**而非**被违规的键数**计。当前每键最多声明 min/max
        // 各一 ⇒ 一个键最多贡献 1 条，两者等价；若将来同键同时声明 min 与 max 且双向
        // 越界，单个键可贡献 2 条，届时 `scoped_hits >= effective_scoped` 会比
        // 「全键违规」更宽（1 个键 2 条 ⇒ 2 个受管键的域包即判 Critical）。
        let effective_scoped = self
            .declared
            .iter()
            .filter(|d| d.is_effective() && scoped.contains(&d.key.as_str()))
            .count();
        let risk_level = if violations.is_empty() {
            RiskLevel::Low
        } else if effective_scoped > 0 && scoped_hits >= effective_scoped {
            RiskLevel::Critical
        } else if violations.len() == 1 {
            RiskLevel::Medium
        } else {
            RiskLevel::High
        };

        let passed = violations.is_empty();

        Ok(RiskCheckResult {
            domain_pack_id: self.domain_pack_id.clone(),
            risk_level,
            violations,
            passed,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskCheckResult {
    pub domain_pack_id: String,
    pub risk_level: RiskLevel,
    pub violations: Vec<RiskViolation>,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskViolation {
    pub rule: String,
    pub current: f64,
    pub threshold: f64,
    pub message: String,
}

// ── OpcCapabilityPackAnalysisRound ────────────────────────────────────

/// 域包分析回合（实现 SelfImprovingRound trait）
pub struct OpcCapabilityPackAnalysisRound {
    domain_pack_id: String,
    data_service: Arc<dyn OpcDataService>,
    risk_gate: OpcRiskGate,
}

impl OpcCapabilityPackAnalysisRound {
    /// 仅通用风控规则生效（不注入域包声明阈值）。
    pub fn new(domain_pack_id: String, data_service: Arc<dyn OpcDataService>) -> Self {
        Self::with_declared_thresholds(domain_pack_id, data_service, Vec::new())
    }

    /// 注入域包 `runtime.yaml` 声明的风控阈值。
    ///
    /// 阈值不可能由本 crate 自行取得：`analysis-engine` 不读 YAML，也不持有域包
    /// 目录 —— 由命令层解析后传入（见 `commands::opc_capability_pack_runtime`）。
    pub fn with_declared_thresholds(
        domain_pack_id: String,
        data_service: Arc<dyn OpcDataService>,
        declared: Vec<DeclaredKpiThreshold>,
    ) -> Self {
        let risk_gate = OpcRiskGate::with_declared_thresholds(&domain_pack_id, declared);
        Self { domain_pack_id, data_service, risk_gate }
    }

    /// 执行分析并返回决策
    pub async fn analyze(
        &self,
        time_range: &super::data_service::TimeRange,
    ) -> OpcResult<OpcCapabilityPackDecision> {
        let kpis = super::capability_pack_kpi_service::compute_kpis(
            &self.domain_pack_id,
            &self.data_service,
            time_range,
        )
        .await?;
        let risk_check = self.risk_gate.check(&kpis).await?;

        let recommendations = self.generate_recommendations(&kpis, &risk_check);

        let risk_level = risk_check.risk_level.clone();
        let confidence = self.calculate_confidence(&kpis);

        Ok(OpcCapabilityPackDecision {
            domain_pack_id: self.domain_pack_id.clone(),
            decision_type: DecisionType::PerformanceReview,
            summary: self.generate_summary(&kpis, &risk_check),
            confidence,
            kpis,
            recommendations,
            risk_level,
        })
    }

    fn generate_recommendations(
        &self,
        kpis: &[super::analytics::KpiValue],
        risk_check: &RiskCheckResult,
    ) -> Vec<String> {
        let mut recs = Vec::new();

        if !risk_check.passed {
            for violation in &risk_check.violations {
                recs.push(format!("⚠️ {}: {}", violation.rule, violation.message));
            }
        }

        for kpi in kpis {
            if let Some(target) = kpi.target {
                if kpi.value < target * 0.8 {
                    recs.push(format!(
                        "📈 {} 低于目标 80%（当前 {:.1}，目标 {:.1}）",
                        kpi.key, kpi.value, target
                    ));
                }
            }
        }

        if recs.is_empty() {
            recs.push("✅ 所有指标正常，建议保持当前策略".to_string());
        }

        recs
    }

    fn generate_summary(
        &self,
        kpis: &[super::analytics::KpiValue],
        risk_check: &RiskCheckResult,
    ) -> String {
        let kpi_summary: Vec<String> =
            kpis.iter().map(|k| format!("{}={}", k.key, kpi_value_display(k))).collect();

        let risk_summary = if risk_check.passed {
            "风控检查通过".to_string()
        } else {
            format!("发现 {} 个风控违规", risk_check.violations.len())
        };

        format!("域包「{}」分析：{}。{}", self.domain_pack_id, kpi_summary.join(", "), risk_summary)
    }

    fn calculate_confidence(&self, kpis: &[super::analytics::KpiValue]) -> f64 {
        if kpis.is_empty() {
            return 0.0;
        }
        let with_targets = kpis.iter().filter(|k| k.target.is_some()).count();
        if with_targets == 0 {
            return 0.5;
        }
        let on_track = kpis.iter().filter(|k| k.target.is_some_and(|t| k.value >= t * 0.8)).count();
        on_track as f64 / kpis.len() as f64
    }
}

/// KPI 值的摘要呈现。
///
/// 非 `available` 的 KPI **不得**把占位 0 写成真实值（本项目禁止伪造数据），
/// 必须显式标注「暂无数据 / 未接数据源」。
fn kpi_value_display(kpi: &super::analytics::KpiValue) -> String {
    match kpi.availability {
        super::analytics::KpiAvailability::Available => format!("{:.1}", kpi.value),
        super::analytics::KpiAvailability::Empty => "暂无数据".to_string(),
        super::analytics::KpiAvailability::NoDataSource => "未接数据源".to_string(),
    }
}

#[async_trait]
impl SelfImprovingRound for OpcCapabilityPackAnalysisRound {
    async fn execute_round(
        &mut self,
        _task: &str,
        _prev_evaluation: Option<&RoundEvaluation>,
    ) -> Result<RoundResult, Box<dyn std::error::Error + Send>> {
        let time_range = super::data_service::TimeRange::days(30);
        let decision =
            self.analyze(&time_range).await.map_err(|e| anyhow::anyhow!(e.to_string()))?;

        let output =
            serde_json::to_string_pretty(&decision).map_err(|e| anyhow::anyhow!(e.to_string()))?;

        let trace = vec![
            RoundStep {
                index: 0,
                kind: "analyze".to_string(),
                summary: format!("域包 {} KPI 分析完成", self.domain_pack_id),
                tokens_used: 0,
            },
            RoundStep {
                index: 1,
                kind: "risk_check".to_string(),
                summary: format!("风控检查：{:?}", decision.risk_level),
                tokens_used: 0,
            },
        ];

        Ok(RoundResult { round: 0, output, evaluation: None, trace })
    }

    async fn evaluate_round(
        &self,
        _task: &str,
        result: &RoundResult,
    ) -> Result<RoundEvaluation, Box<dyn std::error::Error + Send>> {
        let output_len = result.output.len();
        let has_content = output_len > 100;
        let score = if has_content { 0.8 } else { 0.3 };
        let confidence = if has_content { 0.9 } else { 0.4 };

        let mut gaps = Vec::new();
        if !has_content {
            gaps.push("输出内容不足".to_string());
        }

        let strengths =
            vec![format!("域包 {} 独立分析完成", self.domain_pack_id), "包含风控检查".to_string()];

        Ok(RoundEvaluation {
            score,
            confidence,
            gaps,
            strengths,
            raw_assessment: String::new(),
            next_direction: None,
        })
    }

    async fn decide_next(
        &self,
        _task: &str,
        _result: &RoundResult,
        evaluation: &RoundEvaluation,
    ) -> Result<NextAction, Box<dyn std::error::Error + Send>> {
        if evaluation.score >= 0.7 || evaluation.gaps.is_empty() {
            Ok(NextAction::Accept)
        } else {
            Ok(NextAction::Refine { direction: evaluation.gaps.join("; ") })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{risk_scoped_kpi_keys, DecisionType, DeclaredKpiThreshold, OpcRiskGate, RiskLevel};
    use crate::opc::analytics::{KpiAvailability, KpiValue};

    /// 跨语言契约夹具：本枚举的序列化值域必须与前端 `src/pages/opc/domains/types.ts`
    /// 第 210 行的联合字面量（`"high" | "medium" | "low"`）以及
    /// `DomainComponents.tsx` 的 `=== "high"` / `=== "medium"` 比对一致。
    ///
    /// 历史缺陷（F2）：无 `rename_all` 时输出 PascalCase `"Low"` / `"High"`，
    /// 前端比对恒 false ⇒ 风险提示永远落在绿色兜底分支。
    #[test]
    fn risk_level_serializes_snake_case() {
        for (variant, expected) in [
            (RiskLevel::Low, "low"),
            (RiskLevel::Medium, "medium"),
            (RiskLevel::High, "high"),
            (RiskLevel::Critical, "critical"),
        ] {
            assert_eq!(
                serde_json::to_value(&variant).expect("RiskLevel 应可序列化"),
                serde_json::json!(expected),
                "RiskLevel 序列化值与前端比对字面量不一致"
            );
        }
    }

    /// 同 `risk_level`：`decision_type` 目前在前端仅作原样展示（无值级 i18n），
    /// 但值域一经改动即为跨语言契约，故一并锁定。
    #[test]
    fn decision_type_serializes_snake_case() {
        for (variant, expected) in [
            (DecisionType::PerformanceReview, "performance_review"),
            (DecisionType::KpiAnalysis, "kpi_analysis"),
            (DecisionType::TrendForecast, "trend_forecast"),
            (DecisionType::RiskAssessment, "risk_assessment"),
            (DecisionType::StrategicPlanning, "strategic_planning"),
        ] {
            assert_eq!(
                serde_json::to_value(&variant).expect("DecisionType 应可序列化"),
                serde_json::json!(expected),
                "DecisionType 序列化值变化会破坏前端值域约定"
            );
        }
    }

    // ── F4：域包声明式风控（创作类 KPI）──────────────────────────

    fn kpi(key: &str, value: f64, availability: KpiAvailability) -> KpiValue {
        KpiValue { key: key.to_string(), value, availability, ..Default::default() }
    }

    /// `content_media` 已声明的阈值 —— 必须与
    /// `config/opc/domain_packs/content_media/runtime.yaml` 的 `risk` 段保持一致。
    ///
    /// **只含实际声明阈值的键**：`revision_rounds` 刻意不在列（实测只取 {0,1}，
    /// 任何阈值都无依据 ⇒ 未声明 ⇒ 不启用）。
    fn content_media_declared() -> Vec<DeclaredKpiThreshold> {
        vec![
            DeclaredKpiThreshold { key: "word_count".to_string(), min: Some(1000.0), max: None },
            DeclaredKpiThreshold {
                key: "completion_rate".to_string(),
                min: Some(100.0),
                max: None,
            },
        ]
    }

    /// 受管键全部 `Available` 且已声明的阈值全部越限 ⇒ **`Critical`**
    /// （受管键全违规）。
    ///
    /// ⚠️ 违规数**实测为 2 条**（`word_count` + `completion_rate`），不是 3 条：
    /// `revision_rounds` 属受管键但**未声明阈值**，按「未声明 = 不启用」不得参与
    /// 判定（本用例的第三个 KPI 即用来钉住这一点）。
    #[tokio::test]
    async fn content_media_creative_kpis_raise_risk() {
        let gate = OpcRiskGate::with_declared_thresholds("content_media", content_media_declared());
        let kpis = vec![
            kpi("word_count", 200.0, KpiAvailability::Available), // < 1000
            kpi("completion_rate", 0.0, KpiAvailability::Available), // < 100
            kpi("revision_rounds", 99.0, KpiAvailability::Available), // 受管但未声明
        ];

        let r = gate.check(&kpis).await.expect("风控检查不应失败");

        assert_eq!(
            r.violations.len(),
            2,
            "实测违规应为 2 条（受管且已声明的键各 1 条）：{:?}",
            r.violations
        );
        assert!(r.violations.iter().any(|v| v.rule == "min_word_count"));
        assert!(r.violations.iter().any(|v| v.rule == "min_completion_rate"));
        assert!(
            !r.violations.iter().any(|v| v.rule.contains("revision_rounds")),
            "未声明阈值的受管键不得参与判定"
        );
        assert_eq!(
            r.risk_level,
            RiskLevel::Critical,
            "2 条违规 = 已声明生效的全部受管键都违规 ⇒ Critical（旧门槛 >3 时恒不可达）"
        );
        assert!(!r.passed);
    }

    /// `Critical` 的分母是「已声明生效阈值的受管键数」：只命中其中一个键 ⇒ 不是
    /// 全违规 ⇒ **`Medium`**（旧门槛 `len == 1 ⇒ Medium` 在新语义下仍然成立）。
    #[tokio::test]
    async fn partial_scoped_violation_is_not_critical() {
        let gate = OpcRiskGate::with_declared_thresholds("content_media", content_media_declared());
        let kpis = vec![
            kpi("word_count", 200.0, KpiAvailability::Available), // < 1000 ⇒ 违规
            kpi("completion_rate", 100.0, KpiAvailability::Available), // = 100 ⇒ 达标
        ];

        let r = gate.check(&kpis).await.expect("风控检查不应失败");

        assert_eq!(r.violations.len(), 1, "只有 word_count 越限：{:?}", r.violations);
        assert_eq!(r.risk_level, RiskLevel::Medium, "受管键未全违规 ⇒ 不得 Critical");
        assert!(!r.passed);
    }

    /// **`effective_scoped == 0` ⇒ 不可 `Critical`**：一个受管阈值都没配时，静态规则
    /// 命中再多也只是 `High` —— 否则「没配阈值」会变成最高风险。
    #[tokio::test]
    async fn no_effective_scoped_threshold_never_critical() {
        // 不注入任何域包声明（`OpcRiskGate::new`）。
        let gate = OpcRiskGate::new("content_media");
        let kpis = vec![
            kpi("expense_ratio", 99.0, KpiAvailability::Available), // > 70 ⇒ 违规
            kpi("customer_satisfaction", 0.0, KpiAvailability::Available), // < 3.5 ⇒ 违规
        ];

        let r = gate.check(&kpis).await.expect("风控检查不应失败");

        assert_eq!(r.violations.len(), 2, "两条静态规则各 1 条：{:?}", r.violations);
        assert_eq!(
            r.risk_level,
            RiskLevel::High,
            "静态规则不进 Critical 分子/分母 ⇒ 分母为 0，最高只能 High"
        );
    }

    /// **变更①的守护**：`content_media` 的受管键集是 **7 键**（全部 KPI），但另外 5 键
    /// 未声明阈值 ⇒ 即使 `Available` 且值为 0 也不得判违规。
    ///
    /// `content_count` / `page_views` 的聚合源空结果也返回 `Available` + 0，一旦有人
    /// 「顺手」给它们补个 `min`，本用例会立刻变红。
    #[tokio::test]
    async fn all_seven_keys_scoped_but_undeclared_ones_never_rule() {
        assert_eq!(
            risk_scoped_kpi_keys("content_media").len(),
            7,
            "受管键集应覆盖 runtime.yaml 声明的全部 7 个键"
        );

        let gate = OpcRiskGate::with_declared_thresholds("content_media", content_media_declared());
        let kpis = vec![
            kpi("content_count", 0.0, KpiAvailability::Available),
            kpi("page_views", 0.0, KpiAvailability::Available),
            kpi("conversion_rate", 0.0, KpiAvailability::Available),
            kpi("content_engagement", 0.0, KpiAvailability::NoDataSource),
        ];

        let r = gate.check(&kpis).await.expect("风控检查不应失败");

        assert!(r.violations.is_empty(), "受管但未声明阈值的键不得参与判定：{:?}", r.violations);
        assert_eq!(r.risk_level, RiskLevel::Low);
        assert!(r.passed);
    }

    /// **防「把缺失当 0」回归（最重要）**：`Empty` / `NoDataSource` 下 `value` 是
    /// 占位 `0.0`，任何规则都不得据此判定 —— 否则「没跑过工作流」的用户会被判
    /// 「字数 0 < 下限」。
    #[tokio::test]
    async fn empty_kpis_never_violate() {
        let gate = OpcRiskGate::with_declared_thresholds("content_media", content_media_declared());
        let kpis = vec![
            kpi("word_count", 0.0, KpiAvailability::Empty),
            kpi("completion_rate", 0.0, KpiAvailability::Empty),
            // 对照组：通用规则同样不得读占位值（0.0 Available 时 customer_satisfaction
            // 必然违规，故此处若违规即证明守卫未对通用规则生效）
            kpi("expense_ratio", 0.0, KpiAvailability::Empty),
            kpi("customer_satisfaction", 0.0, KpiAvailability::Empty),
        ];

        let r = gate.check(&kpis).await.expect("风控检查不应失败");

        assert!(r.violations.is_empty(), "非 Available 一律不判，实际：{:?}", r.violations);
        assert_eq!(r.risk_level, RiskLevel::Low);
        assert!(r.passed);
    }

    /// **防跨域包误伤回归**：`completion_rate` 在多个域包中同名，且**语义不同** ——
    /// `compute_software_dev_kpis`（`capability_pack_kpi_service.rs:96-140`）也产出
    /// `completion_rate`，但它是 `completed / total × 100` 的项目完成比例（真值、
    /// `Available`），与 content_media 的「章节完成率」不是一回事。若守卫来自裸键名，
    /// software_dev 的 40% 会被 content_media 的 `min = 100` 判成违规。
    ///
    /// 即使命令层误把 content_media 的阈值下发给别的域包，也不得生效。
    #[tokio::test]
    async fn creative_rules_scoped_to_content_media() {
        // 域包 id 归一（content-media ≡ content_media）
        assert_eq!(risk_scoped_kpi_keys("content-media"), risk_scoped_kpi_keys("content_media"));
        assert!(!risk_scoped_kpi_keys("content_media").is_empty());

        for other in ["security", "software_dev", "project_management", "design"] {
            assert!(risk_scoped_kpi_keys(other).is_empty(), "{other} 不得进入创作类管辖集");

            let gate = OpcRiskGate::with_declared_thresholds(other, content_media_declared());
            let kpis = vec![
                kpi("completion_rate", 40.0, KpiAvailability::Available), // 40 < 100
                kpi("word_count", 10.0, KpiAvailability::Available),      // 10 < 1000
            ];
            let r = gate.check(&kpis).await.expect("风控检查不应失败");
            assert!(
                r.violations.is_empty(),
                "{other} 不得套用 content_media 的创作类规则：{:?}",
                r.violations
            );
            assert_eq!(r.risk_level, RiskLevel::Low);
            assert!(r.passed);
        }
    }

    /// 恒 `NoDataSource` 的键不得参与风控（纳入必然假报）。
    #[tokio::test]
    async fn content_engagement_never_violates() {
        let gate = OpcRiskGate::with_declared_thresholds("content_media", content_media_declared());
        let kpis = vec![kpi("content_engagement", 0.0, KpiAvailability::NoDataSource)];

        let r = gate.check(&kpis).await.expect("风控检查不应失败");

        assert!(r.violations.is_empty(), "实际：{:?}", r.violations);
        assert!(r.passed);
    }

    /// 「未声明 = 不启用」：受管键未声明、以及完全未注入声明时，均不得用默认值兜底。
    #[tokio::test]
    async fn undeclared_threshold_does_not_rule() {
        let kpis = vec![kpi("revision_rounds", 999.0, KpiAvailability::Available)];

        // 其余受管键已声明，但 revision_rounds 未声明
        let gate = OpcRiskGate::with_declared_thresholds("content_media", content_media_declared());
        let r = gate.check(&kpis).await.expect("风控检查不应失败");
        assert!(r.violations.is_empty(), "未声明 = 不启用，实际：{:?}", r.violations);

        // 完全未注入（既有构造方式）：不得回退成任何默认阈值
        let bare = OpcRiskGate::new("content_media");
        let r2 = bare.check(&kpis).await.expect("风控检查不应失败");
        assert!(r2.violations.is_empty());
        assert!(r2.passed);
        assert_eq!(r2.risk_level, RiskLevel::Low);
    }
}
