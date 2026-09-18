// SPDX-License-Identifier: AGPL-3.0-only

use serde::{Deserialize, Serialize};

// ── 分析指标类型（stock-analysis 域） ──────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum MetricType {
    #[default]
    Counter,
    Count,
    Gauge,
    Histogram,
    Rate,
    Ratio,
    Currency,
    Percentage,
    Boolean,
}

impl std::fmt::Display for MetricType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetricType::Counter => write!(f, "counter"),
            MetricType::Count => write!(f, "count"),
            MetricType::Gauge => write!(f, "gauge"),
            MetricType::Histogram => write!(f, "histogram"),
            MetricType::Rate => write!(f, "rate"),
            MetricType::Ratio => write!(f, "ratio"),
            MetricType::Currency => write!(f, "currency"),
            MetricType::Percentage => write!(f, "percentage"),
            MetricType::Boolean => write!(f, "boolean"),
        }
    }
}

impl std::str::FromStr for MetricType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "counter" => Ok(MetricType::Counter),
            "count" => Ok(MetricType::Count),
            "gauge" => Ok(MetricType::Gauge),
            "histogram" => Ok(MetricType::Histogram),
            "rate" => Ok(MetricType::Rate),
            "ratio" => Ok(MetricType::Ratio),
            "currency" => Ok(MetricType::Currency),
            "percentage" => Ok(MetricType::Percentage),
            "boolean" => Ok(MetricType::Boolean),
            _ => Err(format!("Unknown metric type: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KpiDefinition {
    pub key: String,
    pub name: String,
    pub description: String,
    pub metric_type: MetricType,
    pub target: Option<f64>,
    pub unit: Option<String>,
    pub formula: Option<String>,
    pub calculation_rule: Option<String>,
}

impl KpiDefinition {
    pub fn new(
        key: impl Into<String>,
        name: impl Into<String>,
        unit: impl Into<String>,
        metric_type: MetricType,
    ) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            description: String::new(),
            metric_type,
            target: None,
            unit: Some(unit.into()),
            formula: None,
            calculation_rule: None,
        }
    }

    pub fn with_target(mut self, target: f64) -> Self {
        self.target = Some(target);
        self
    }

    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }
}

/// KPI 值的取数状态。
///
/// **缺失必须显式**：本项目禁止用 `0` 冒充「无数据」。`KpiValue::value` 只在
/// `Available` 时才是真实值；`Empty` / `NoDataSource` 下 `value` 恒为 0.0，
/// 仅是占位，消费方（前端 / 决策摘要）必须读本字段决定如何呈现。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KpiAvailability {
    /// 有真实值（含「真实统计结果恰为 0」，如时间窗内确实没有文章）。
    #[default]
    Available,
    /// 计算源已接入，但当前窗口内没有记录（如工作流尚未产出、浏览量为 0 导致比率无从计算）。
    Empty,
    /// 计算逻辑未接入数据源（如互动数据无存储）。值无意义。
    NoDataSource,
}

impl std::fmt::Display for KpiAvailability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KpiAvailability::Available => write!(f, "available"),
            KpiAvailability::Empty => write!(f, "empty"),
            KpiAvailability::NoDataSource => write!(f, "no_data_source"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KpiValue {
    pub key: String,
    /// 展示 ID（缺省等于 `key`）。由 runtime.yaml 元数据补全。
    #[serde(default)]
    pub id: String,
    /// 展示名称。由 runtime.yaml 元数据补全（缺省回退 `key`）。
    #[serde(default)]
    pub name: String,
    pub value: f64,
    pub target: Option<f64>,
    pub unit: Option<String>,
    pub timestamp: i64,
    /// 取数状态；默认 `available` 保持旧 JSON 兼容。
    #[serde(default)]
    pub availability: KpiAvailability,
    /// 非 `available` 时的原因说明（供前端 tooltip / 报告追溯）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartConfig {
    pub chart_type: ChartType,
    pub title: String,
    pub x_axis_label: Option<String>,
    pub y_axis_label: Option<String>,
    pub data_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum ChartType {
    #[default]
    Line,
    Bar,
    Pie,
    Area,
    Scatter,
    Table,
    Metric,
}

impl std::fmt::Display for ChartType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChartType::Line => write!(f, "line"),
            ChartType::Bar => write!(f, "bar"),
            ChartType::Pie => write!(f, "pie"),
            ChartType::Area => write!(f, "area"),
            ChartType::Scatter => write!(f, "scatter"),
            ChartType::Table => write!(f, "table"),
            ChartType::Metric => write!(f, "metric"),
        }
    }
}

impl std::str::FromStr for ChartType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "line" => Ok(ChartType::Line),
            "bar" => Ok(ChartType::Bar),
            "pie" => Ok(ChartType::Pie),
            "area" => Ok(ChartType::Area),
            "scatter" => Ok(ChartType::Scatter),
            "table" => Ok(ChartType::Table),
            "metric" => Ok(ChartType::Metric),
            _ => Err(format!("Unknown chart type: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardConfig {
    pub id: String,
    pub name: String,
    pub description: String,
    pub layout: Vec<DashboardWidget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardWidget {
    pub id: String,
    pub title: String,
    pub widget_type: ChartType,
    pub data_key: String,
    pub size: WidgetSize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum WidgetSize {
    Small,
    #[default]
    Medium,
    Large,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportConfig {
    pub id: String,
    pub name: String,
    pub template: String,
    pub sections: Vec<ReportSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportSection {
    pub id: String,
    pub title: String,
    pub type_: ReportSectionType,
    pub data_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum ReportSectionType {
    #[default]
    Summary,
    Detail,
    Chart,
    Table,
    Text,
}

// ── OPC 分析仪表盘服务 ──────────────────────────────────────────

use async_trait::async_trait;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
};

use axagent_entities::{
    opc_customers, opc_invoices, opc_kpi_records, opc_projects, opc_revenue_records,
};
use axagent_harness::util_fns::{gen_id, now_ts};

use super::error::{OpcError, OpcResult};

/// KPI 记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KpiRecord {
    pub id: String,
    /// 域包归属（`config/opc/domain_packs/<id>`，下划线形式）。
    ///
    /// 消费方**必须**读本字段：`''` 表示「未标注」（v230 迁移对存量行的回填值），
    /// 不是任何域包的真实归属 —— 用它当域包会造出一个不存在的域包。
    pub domain_pack_id: String,
    pub name: String,
    pub value: f64,
    pub unit: String,
    pub period: String,
    pub recorded_at: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateKpiInput {
    /// 域包归属。**必填、无 `#[serde(default)]`** —— 缺键即反序列化报错。
    ///
    /// ## 为什么刻意不做空串兜底
    ///
    /// 兜底会让「调用方忘传」变成「静默写一行查不出来的记录」，与 v230 迁移摘掉
    /// 列 DEFAULT 的取舍直接矛盾（那边关掉的就是这条通道）。
    /// 域包归属只能由**运行上下文**注入（如 post_exec 钩子的 `ctx.input["domain_pack_id"]`），
    /// **不得由 LLM 自报**：自报的域包 id 要么是幻觉（落到不存在的域包桶），
    /// 要么是它猜的（落到别人的域包桶），两者都比「拒绝写入」更糟。
    pub domain_pack_id: String,
    pub name: String,
    pub value: f64,
    pub unit: String,
    /// 周期，唯一合法形态 `YYYY-MM`（`chrono` 的 `%Y-%m`）。
    /// 由 [`validate_period`] 在写入口强制（PG 侧另有 DB CHECK 兜底新写点）。
    pub period: String,
}

/// 收入记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevenueRecord {
    pub id: String,
    pub amount: f64,
    pub currency: String,
    pub category: String,
    pub description: String,
    pub recorded_at: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRevenueInput {
    pub amount: f64,
    pub currency: String,
    pub category: String,
    pub description: String,
}

/// 仪表盘摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSummary {
    pub total_revenue: f64,
    pub total_invoices: u32,
    pub active_projects: u32,
    pub total_customers: u32,
    pub recent_kpis: Vec<KpiRecord>,
    pub revenue_trend: Vec<RevenueRecord>,
}

// ── AnalyticsService Trait ─────────────────────────────────────

#[async_trait]
pub trait AnalyticsService: Send + Sync {
    async fn record_kpi(&self, input: CreateKpiInput) -> OpcResult<KpiRecord>;

    /// 读 `opc_kpi_records` 的**域包内**视图（R2）。
    ///
    /// `domain_pack_id` 必填而非 `Option`：`opc_kpi_records` 是多域包共用的一张表，
    /// 「不传域包」与「跨域包总览」是两件事，用一个 `Option` 混在一起会让
    /// 「忘传 = 跨域包」成为默认行为。需要跨域包的调用方请用 [`Self::list_kpis_all`]。
    async fn list_kpis(
        &self,
        domain_pack_id: &str,
        period: Option<String>,
        limit: Option<u32>,
    ) -> OpcResult<Vec<KpiRecord>>;

    /// **显式跨域包**列出 KPI 记录（不带域包参是它的语义，不是遗漏）。
    ///
    /// 唯一调用方是 LLM 工具 `OpcListKpis`（`tools/src/tools/opc.rs`）：
    /// 它的运行上下文不携带任何域包标识（`ToolContext.extra` 只有搜索配置与
    /// `vault_kb_id`），而「列出所有 KPI 记录」本身也不需要域包 —— 故给它一条
    /// 显式命名的跨域包通道，而不是让它去猜一个域包 id。
    /// 每行都带 `domain_pack_id`（见 [`KpiRecord::domain_pack_id`]）供消费方自行分组/标注。
    async fn list_kpis_all(
        &self,
        period: Option<String>,
        limit: Option<u32>,
    ) -> OpcResult<Vec<KpiRecord>>;

    async fn record_revenue(&self, input: CreateRevenueInput) -> OpcResult<RevenueRecord>;
    async fn list_revenue(
        &self,
        category: Option<String>,
        limit: Option<u32>,
    ) -> OpcResult<Vec<RevenueRecord>>;

    /// 域包范围内的仪表盘摘要。
    ///
    /// `domain_pack_id` 必填：KPI 段（`recent_kpis`）是域包相关的量，
    /// 跨域包混算本身就是串号来源；且风控档位（`risk_level`）按域包判定。
    async fn get_dashboard_summary(&self, domain_pack_id: &str) -> OpcResult<DashboardSummary>;

    /// **显式跨域包**总览（不带域包参是它的语义，不是遗漏）。
    ///
    /// 唯一调用方是 Tauri 命令 `opc_get_dashboard_summary` → 前端
    /// `src/pages/opc/components/DashboardTab.tsx`：那是 **OPC 首页的面板**
    /// （`src/pages/OpcPage.tsx:30` 的 dashboard 页签，组件不接收任何域包参数），
    /// 语义上就是「所有域包的概览」。让它走一条显式命名的函数，
    /// 比给一个 `Option<domain_pack_id>` 让它「默认跨域包」更难误用。
    /// 每行 KPI 都带 `domain_pack_id` 供前端标注来源。
    async fn get_dashboard_summary_overview(&self) -> OpcResult<DashboardSummary>;
}

/// 域包 id 归一到**唯一形态**（下划线），并拒绝空值。
///
/// ## 为什么必须归一（而不是各调用点各写各的）
///
/// 同一个域包的 id 在本仓同时以两种形态流通：
/// `config/opc/domain_packs/content_media/`（目录名、注册表键、本表的规范值）
/// 与前端/命令参数里的 `content-media`（连字符）。
/// 若不归一就过滤，两种写法会落进**两个不同的桶**
/// ⇒ 写入端用连字符、读取端用下划线时，域包仪表盘永远读不到自己刚写的值
/// （这正是「数据明明落了库却显示暂无数据」的形态）。
///
/// 归一真源只有这一处：写侧（post_exec 钩子、`opc_record_kpi` 命令）与
/// 读侧（`opc::data_service::DefaultDataService::latest_kpi` 等）**都必须**过这个函数。
/// 先例同 `domain_pack_kpi_service::kpi_source`（`:187` 的 `replace('-', "_")`）
/// 与 `opc_domain_pack_actions::opc_execute_workflow`（`:2494` 的「归一化域包 ID」）。
///
/// 空值直接 `Err`：`''` 在存量行里表示「未标注」（v230 迁移的回填值），
/// 它**不是**一个域包 id —— 允许它当查询条件会造出一个看起来合法的假域包桶。
pub fn canonical_domain_pack_id(domain_pack_id: &str) -> OpcResult<String> {
    let id = domain_pack_id.trim().replace('-', "_");
    if id.is_empty() {
        return Err(OpcError::Validation(
            "domain_pack_id 不能为空：'' 只表示存量行的「未标注」，不是一个域包".into(),
        ));
    }
    Ok(id)
}

/// 校验 `period` 的唯一合法形态：`YYYY-MM`（`chrono` 的 `%Y-%m`）。
///
/// ## 为什么 Rust 侧也校验（DB CHECK 之外）
///
/// v230 给 PG 加了 `CHECK (period ~ '^\d{4}-\d{2}$')`，但那条约束是 **PG-only**
/// （SQLite 无 `~` 操作符）。若只有 DB CHECK，「SQLite 测试通过」就不能代表
/// 写入口合法；两方言一致性会变成「只在生产才被发现」的假象。
/// 故写入口自身也强制 —— 两者是**同一条**契约在不同层的实现。
///
/// 与两处写点同源：钩子写 `chrono::Utc::now().format("%Y-%m")`；
/// `opc_record_kpi` 命令收到的 `period` 经此处校验后才落库。
pub fn validate_period(period: &str) -> OpcResult<()> {
    let bytes = period.as_bytes();
    let ok = bytes.len() == 7
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..].iter().all(u8::is_ascii_digit);
    // 月份额外卡 01–12：`2026-13` 满足 `\d{4}-\d{2}` 却不是合法月份。
    // DB CHECK 用的是同一条正则（结构），此处更严 —— 严的那侧是写入口，
    // 不会与 CHECK 冲突（CHECK 是下界，不是等号）。
    if !ok {
        return Err(OpcError::Validation(format!(
            "period 必须是 YYYY-MM（如 2026-09），实际为 {period:?}"
        )));
    }
    let month: u32 = period[5..].parse().unwrap_or(0);
    if !(1..=12).contains(&month) {
        return Err(OpcError::Validation(format!(
            "period 的月份必须在 01–12 之间，实际为 {period:?}"
        )));
    }
    Ok(())
}

/// Noop 实现
#[derive(Debug)]
pub struct NoopAnalyticsService;

#[async_trait]
impl AnalyticsService for NoopAnalyticsService {
    async fn record_kpi(&self, _: CreateKpiInput) -> OpcResult<KpiRecord> {
        Err(OpcError::NotFound("AnalyticsService not implemented".into()))
    }
    async fn list_kpis(
        &self,
        _domain_pack_id: &str,
        _period: Option<String>,
        _limit: Option<u32>,
    ) -> OpcResult<Vec<KpiRecord>> {
        Ok(Vec::new())
    }
    async fn list_kpis_all(
        &self,
        _period: Option<String>,
        _limit: Option<u32>,
    ) -> OpcResult<Vec<KpiRecord>> {
        Ok(Vec::new())
    }
    async fn record_revenue(&self, _: CreateRevenueInput) -> OpcResult<RevenueRecord> {
        Err(OpcError::NotFound("AnalyticsService not implemented".into()))
    }
    async fn list_revenue(
        &self,
        _: Option<String>,
        _: Option<u32>,
    ) -> OpcResult<Vec<RevenueRecord>> {
        Ok(Vec::new())
    }
    async fn get_dashboard_summary(&self, _domain_pack_id: &str) -> OpcResult<DashboardSummary> {
        Ok(DashboardSummary {
            total_revenue: 0.0,
            total_invoices: 0,
            active_projects: 0,
            total_customers: 0,
            recent_kpis: Vec::new(),
            revenue_trend: Vec::new(),
        })
    }
    async fn get_dashboard_summary_overview(&self) -> OpcResult<DashboardSummary> {
        Ok(DashboardSummary {
            total_revenue: 0.0,
            total_invoices: 0,
            active_projects: 0,
            total_customers: 0,
            recent_kpis: Vec::new(),
            revenue_trend: Vec::new(),
        })
    }
}

// ── Entity ↔ DTO 转换 ──────────────────────────────────────────

fn kpi_entity_to_dto(e: opc_kpi_records::Model) -> KpiRecord {
    KpiRecord {
        id: e.id,
        domain_pack_id: e.domain_pack_id,
        name: e.name,
        value: e.value,
        unit: e.unit,
        period: e.period,
        recorded_at: e.recorded_at,
        created_at: e.created_at,
    }
}

fn revenue_entity_to_dto(e: opc_revenue_records::Model) -> RevenueRecord {
    RevenueRecord {
        id: e.id,
        amount: e.amount,
        currency: e.currency,
        category: e.category,
        description: e.description,
        recorded_at: e.recorded_at,
        created_at: e.created_at,
    }
}

// ── DefaultAnalyticsService (SeaORM) ──────────────────────────

/// 默认分析服务实现
pub struct DefaultAnalyticsService {
    pub db: DatabaseConnection,
}

impl DefaultAnalyticsService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// 仪表盘摘要的公共实现。
    ///
    /// `domain_pack: Option<&str>` **只在本文件内部使用** —— 对外暴露的是两个
    /// 显式命名的函数（[`AnalyticsService::get_dashboard_summary`] 带域包、
    /// [`AnalyticsService::get_dashboard_summary_overview`] 跨域包），
    /// 免得「忘传 = 跨域包」成为默认行为。
    ///
    /// 非 KPI 段（收入/发票/项目/客户）不按域包过滤：那几张表（`opc_invoices` /
    /// `opc_projects` / `opc_customers`）本例没有域包列，这个口径**不由本次变更决定**
    /// —— 本次只收敛 KPI 段（`recent_kpis`），不顺手改别的表的语义。
    async fn load_dashboard(&self, domain_pack: Option<&str>) -> OpcResult<DashboardSummary> {
        let invoices = opc_invoices::Entity::find()
            .all(&self.db)
            .await
            .map_err(|e| OpcError::Database(e.to_string()))?;
        let customers = opc_customers::Entity::find()
            .all(&self.db)
            .await
            .map_err(|e| OpcError::Database(e.to_string()))?;
        let projects = opc_projects::Entity::find()
            .all(&self.db)
            .await
            .map_err(|e| OpcError::Database(e.to_string()))?;
        let mut kpi_query =
            opc_kpi_records::Entity::find().order_by_desc(opc_kpi_records::Column::RecordedAt);
        if let Some(ind) = domain_pack {
            kpi_query = kpi_query
                .filter(opc_kpi_records::Column::DomainPackId.eq(canonical_domain_pack_id(ind)?));
        }
        let kpis = kpi_query.all(&self.db).await.map_err(|e| OpcError::Database(e.to_string()))?;
        let revenue = opc_revenue_records::Entity::find()
            .order_by_desc(opc_revenue_records::Column::RecordedAt)
            .all(&self.db)
            .await
            .map_err(|e| OpcError::Database(e.to_string()))?;

        let total_revenue: f64 =
            invoices.iter().filter(|i| i.status == "paid").map(|i| i.total.unwrap_or(0.0)).sum();
        let total_invoices = invoices.len() as u32;
        let active_projects =
            projects.iter().filter(|p| p.status == "active" || p.status == "planning").count()
                as u32;
        let total_customers = customers.len() as u32;

        Ok(DashboardSummary {
            total_revenue,
            total_invoices,
            active_projects,
            total_customers,
            recent_kpis: kpis.into_iter().map(kpi_entity_to_dto).collect(),
            revenue_trend: revenue.into_iter().map(revenue_entity_to_dto).collect(),
        })
    }
}

#[async_trait]
impl AnalyticsService for DefaultAnalyticsService {
    async fn record_kpi(&self, input: CreateKpiInput) -> OpcResult<KpiRecord> {
        // 两个写入口校验都在落库前、且在同一个地方：
        // 1. 域包必填且归一到唯一形态（否则同一个域包会有两个桶）；
        // 2. period 必须 YYYY-MM（DB CHECK 是 PG-only 的下界兜底，见 `validate_period`）。
        let domain_pack_id = canonical_domain_pack_id(&input.domain_pack_id)?;
        validate_period(&input.period)?;
        let now = now_ts();
        let am = opc_kpi_records::ActiveModel {
            id: Set(gen_id()),
            domain_pack_id: Set(domain_pack_id),
            name: Set(input.name),
            value: Set(input.value),
            unit: Set(input.unit),
            period: Set(input.period),
            recorded_at: Set(now),
            created_at: Set(now),
        };
        let entity = am.insert(&self.db).await.map_err(|e| OpcError::Database(e.to_string()))?;
        Ok(kpi_entity_to_dto(entity))
    }

    async fn list_kpis(
        &self,
        domain_pack_id: &str,
        period: Option<String>,
        _limit: Option<u32>,
    ) -> OpcResult<Vec<KpiRecord>> {
        let domain_pack_id = canonical_domain_pack_id(domain_pack_id)?;
        let mut query = opc_kpi_records::Entity::find()
            .filter(opc_kpi_records::Column::DomainPackId.eq(domain_pack_id))
            .order_by_desc(opc_kpi_records::Column::RecordedAt);
        if let Some(ref p) = period {
            validate_period(p)?;
            query = query.filter(opc_kpi_records::Column::Period.eq(p));
        }
        let entities = query.all(&self.db).await.map_err(|e| OpcError::Database(e.to_string()))?;
        Ok(entities.into_iter().map(kpi_entity_to_dto).collect())
    }

    async fn list_kpis_all(
        &self,
        period: Option<String>,
        _limit: Option<u32>,
    ) -> OpcResult<Vec<KpiRecord>> {
        let mut query =
            opc_kpi_records::Entity::find().order_by_desc(opc_kpi_records::Column::RecordedAt);
        if let Some(ref p) = period {
            validate_period(p)?;
            query = query.filter(opc_kpi_records::Column::Period.eq(p));
        }
        let entities = query.all(&self.db).await.map_err(|e| OpcError::Database(e.to_string()))?;
        Ok(entities.into_iter().map(kpi_entity_to_dto).collect())
    }

    async fn record_revenue(&self, input: CreateRevenueInput) -> OpcResult<RevenueRecord> {
        let now = now_ts();
        let am = opc_revenue_records::ActiveModel {
            id: Set(gen_id()),
            amount: Set(input.amount),
            currency: Set(input.currency),
            category: Set(input.category),
            description: Set(input.description),
            recorded_at: Set(now),
            created_at: Set(now),
        };
        let entity = am.insert(&self.db).await.map_err(|e| OpcError::Database(e.to_string()))?;
        Ok(revenue_entity_to_dto(entity))
    }

    async fn list_revenue(
        &self,
        category: Option<String>,
        _limit: Option<u32>,
    ) -> OpcResult<Vec<RevenueRecord>> {
        let mut query = opc_revenue_records::Entity::find()
            .order_by_desc(opc_revenue_records::Column::RecordedAt);
        if let Some(ref c) = category {
            query = query.filter(opc_revenue_records::Column::Category.eq(c));
        }
        let entities = query.all(&self.db).await.map_err(|e| OpcError::Database(e.to_string()))?;
        Ok(entities.into_iter().map(revenue_entity_to_dto).collect())
    }

    async fn get_dashboard_summary(&self, domain_pack_id: &str) -> OpcResult<DashboardSummary> {
        self.load_dashboard(Some(domain_pack_id)).await
    }

    async fn get_dashboard_summary_overview(&self) -> OpcResult<DashboardSummary> {
        self.load_dashboard(None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `period` 收敛到 `YYYY-MM`：合法值逐个放行。
    #[test]
    fn validate_period_accepts_yyyy_mm() {
        for ok in ["2026-01", "2026-09", "2026-12", "1999-10"] {
            assert!(validate_period(ok).is_ok(), "{ok} 应合法");
        }
    }

    /// **有区分力**：v210 起 `period` 是自由文本，历史写法（`2026-9` / `2026-09-01` /
    /// `2026年9月` / 空串）必须被拒 —— 否则「收敛到唯一形态」只是口号。
    #[test]
    fn validate_period_rejects_non_canonical_forms() {
        for bad in [
            "2026-9",
            "2026-009",
            "2026-09-01",
            "2026/09",
            "2026年09月",
            "",
            "2026-13",
            "2026-00",
            "abcd-ef",
        ] {
            assert!(validate_period(bad).is_err(), "{bad:?} 应被拒");
        }
    }

    /// 域包 id 归一：连字符与下划线必须落进**同一个桶**（否则写侧用一处、读侧用另一处，
    /// 数据明明落了库却读不到）。
    #[test]
    fn canonical_domain_pack_id_merges_hyphen_and_underscore() {
        assert_eq!(canonical_domain_pack_id("content-media").expect("应成功"), "content_media");
        assert_eq!(canonical_domain_pack_id("content_media").expect("应成功"), "content_media");
        assert_eq!(canonical_domain_pack_id("  content-media  ").expect("应成功"), "content_media");
    }

    /// 空值必须被拒：`''` 是存量行的「未标注」，不是一个域包。
    #[test]
    fn canonical_domain_pack_id_rejects_empty() {
        assert!(canonical_domain_pack_id("").is_err());
        assert!(canonical_domain_pack_id("   ").is_err());
    }
}
