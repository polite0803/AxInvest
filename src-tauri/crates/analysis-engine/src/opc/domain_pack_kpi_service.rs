//! 域包 KPI 计算服务 — 迁移自 OpcDomainPackAdapter::compute_kpis
//!
//! 替代域包适配器中的 compute_kpis() 方法，保留动态业务逻辑。

use std::sync::Arc;

use super::analytics::{KpiDefinition, KpiValue};
use super::data_service::{OpcDataService, TimeRange};
use super::error::OpcResult;
use super::invoice::InvoiceStatus;
use super::project::ProjectStatus;

/// 会计 KPI 计算
pub async fn compute_accounting_kpis(
    data_service: &Arc<dyn OpcDataService>,
    time_range: &TimeRange,
) -> OpcResult<Vec<KpiValue>> {
    let (from, to) = (time_range.start, time_range.end);
    let now = chrono::Utc::now().timestamp();

    let revenue =
        data_service.aggregate_invoice_amounts(&[InvoiceStatus::Paid], from, to).await?.total;
    let outstanding = data_service
        .count_invoices(&[InvoiceStatus::Sent, InvoiceStatus::Overdue], from, to)
        .await? as f64;
    let total = data_service.count_invoices(&[], from, to).await? as f64;

    let collection_rate = if total > 0.0 {
        let paid = data_service.count_invoices(&[InvoiceStatus::Paid], from, to).await? as f64;
        paid / total
    } else {
        0.0
    };

    Ok(vec![
        KpiValue {
            key: "total_revenue".to_string(),
            value: revenue,
            target: None,
            unit: Some("元".to_string()),
            timestamp: now,
            ..Default::default()
        },
        KpiValue {
            key: "outstanding_invoices".to_string(),
            value: outstanding,
            target: None,
            unit: Some("张".to_string()),
            timestamp: now,
            ..Default::default()
        },
        KpiValue {
            key: "collection_rate".to_string(),
            value: collection_rate * 100.0,
            target: None,
            unit: Some("%".to_string()),
            timestamp: now,
            ..Default::default()
        },
    ])
}

/// 金融投资 KPI 计算
pub async fn compute_finance_invest_kpis(
    data_service: &Arc<dyn OpcDataService>,
    time_range: &TimeRange,
) -> OpcResult<Vec<KpiValue>> {
    let now = chrono::Utc::now().timestamp();
    let (from, to) = (time_range.start, time_range.end);

    let portfolio_value =
        data_service.aggregate_project_budgets(&[ProjectStatus::Active], from, to).await?.total;
    let orders = data_service.count_invoices(&[], from, to).await? as f64;

    Ok(vec![
        KpiValue {
            key: "portfolio_value".to_string(),
            value: portfolio_value,
            target: None,
            unit: Some("元".to_string()),
            timestamp: now,
            ..Default::default()
        },
        KpiValue {
            key: "transaction_count".to_string(),
            value: orders,
            target: None,
            unit: Some("笔".to_string()),
            timestamp: now,
            ..Default::default()
        },
    ])
}

/// 软件研发 KPI 计算
pub async fn compute_software_dev_kpis(
    data_service: &Arc<dyn OpcDataService>,
    time_range: &TimeRange,
) -> OpcResult<Vec<KpiValue>> {
    let now = chrono::Utc::now().timestamp();
    let (from, to) = (time_range.start, time_range.end);

    let completed =
        data_service.count_projects(&[ProjectStatus::Completed], from, to).await? as f64;
    let total = data_service.count_projects(&[], from, to).await? as f64;
    let active = data_service.count_projects(&[ProjectStatus::Active], from, to).await? as f64;

    let completion_rate = if total > 0.0 {
        completed / total * 100.0
    } else {
        0.0
    };

    Ok(vec![
        KpiValue {
            key: "completed_projects".to_string(),
            value: completed,
            target: None,
            unit: Some("个".to_string()),
            timestamp: now,
            ..Default::default()
        },
        KpiValue {
            key: "active_projects".to_string(),
            value: active,
            target: None,
            unit: Some("个".to_string()),
            timestamp: now,
            ..Default::default()
        },
        KpiValue {
            key: "completion_rate".to_string(),
            value: completion_rate,
            target: None,
            unit: Some("%".to_string()),
            timestamp: now,
            ..Default::default()
        },
    ])
}

/// KPI 计算源 —— Rust 侧声明的「该 key 从哪张表、怎么聚合」。
///
/// **元数据（name / unit / metric_type）不在这里**：那部分以域包
/// `runtime.yaml` 的 `kpi_definitions` 为唯一权威（见
/// `crate::opc::domain_pack::runtime_schema`）。
/// 两侧唯一的契约是 `key`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KpiSource {
    /// `COUNT(opc_blog_posts WHERE published=1 AND published_at ∈ [from,to])`
    BlogPostCount,
    /// `SUM(opc_blog_posts.view_count)`，同过滤条件
    BlogPostViews,
    /// `COUNT(opc_contact_submissions.created_at ∈ [from,to]) / page_views × 100`
    ContactConversionRate,
    /// 读取工作流落库的最新一条 KPI（`opc_kpi_records.name == key`，按 recorded_at 倒序）。
    /// 写端：模板 post_exec 钩子（见 `commands::opc_workflow_kpi_hook`）。
    RecordedKpi(&'static str),
    /// 计算逻辑已声明但数据源未接入：**显式缺失**，不得用 0 冒充真实值。
    NoDataSource(&'static str),
}

/// `content_media` KPI 计算源注册表：key → 计算源。
///
/// 新增 KPI = runtime.yaml 加一条元数据 + 本表加一行。
pub const CONTENT_MEDIA_KPI_SOURCES: &[(&str, KpiSource)] = &[
    ("content_count", KpiSource::BlogPostCount),
    ("page_views", KpiSource::BlogPostViews),
    ("conversion_rate", KpiSource::ContactConversionRate),
    (
        "content_engagement",
        KpiSource::NoDataSource(
            "无互动数据存储：opc_blog_posts 仅 view_count，无 like/comment/share 列或互动表",
        ),
    ),
    ("word_count", KpiSource::RecordedKpi("word_count")),
    ("completion_rate", KpiSource::RecordedKpi("completion_rate")),
    ("revision_rounds", KpiSource::RecordedKpi("revision_rounds")),
];

/// 域包 → KPI 计算源注册表（新增域包只在此表加一行，不改任何分支逻辑）。
pub const KPI_SOURCE_REGISTRY: &[(&str, &[(&str, KpiSource)])] =
    &[("content_media", CONTENT_MEDIA_KPI_SOURCES)];

/// 按 `(domain_pack_id, key)` 查计算源。域包 id 按下划线归一（`content-media` ≡ `content_media`）。
pub fn kpi_source(domain_pack_id: &str, key: &str) -> Option<KpiSource> {
    let id = domain_pack_id.replace('-', "_");
    KPI_SOURCE_REGISTRY
        .iter()
        .find(|(ind, _)| *ind == id.as_str())
        .and_then(|(_, table)| table.iter().find(|(k, _)| *k == key).map(|(_, s)| *s))
}

/// 执行单个计算源，返回 `(值, 取数状态, 说明)`。
///
/// 缺失必须是**显式状态**：`Empty` / `NoDataSource` 下返回值恒为 0.0，仅作占位，
/// 消费方必须读 `availability`（前端渲染为「—」，决策摘要渲染为「未接数据源」）。
///
/// `domain_pack_id` 是 `KpiSource::RecordedKpi` 的**必要**入参：那条路读的是
/// `opc_kpi_records`（多域包共用一张表），缺了它就会读到别的域包的同名 KPI。
/// 其余计算源（`opc_blog_posts` 等）目前没有域包列，暂不使用该参数，
/// 但接口上保留 —— 参数由 `compute_kpis` 一路上传，将来那些表加域包列时
/// 不必再改一次调用链。
async fn resolve_kpi_value(
    data_service: &Arc<dyn OpcDataService>,
    domain_pack_id: &str,
    source: KpiSource,
    time_range: &TimeRange,
) -> OpcResult<(f64, super::analytics::KpiAvailability, Option<String>)> {
    use super::analytics::KpiAvailability;

    let (from, to) = (time_range.start, time_range.end);

    Ok(match source {
        KpiSource::BlogPostCount => (
            data_service.count_blog_posts(from, to).await? as f64,
            KpiAvailability::Available,
            None,
        ),
        KpiSource::BlogPostViews => {
            (data_service.sum_blog_post_views(from, to).await?, KpiAvailability::Available, None)
        },
        KpiSource::ContactConversionRate => {
            let contacts = data_service.count_contacts(from, to).await? as f64;
            let views = data_service.sum_blog_post_views(from, to).await?;
            if views > 0.0 {
                (contacts / views * 100.0, KpiAvailability::Available, None)
            } else {
                (
                    0.0,
                    KpiAvailability::Empty,
                    Some("时间窗内博客浏览量为 0，转化率无从计算".to_string()),
                )
            }
        },
        KpiSource::RecordedKpi(name) => {
            match data_service.latest_kpi(domain_pack_id, name, time_range).await? {
                Some(v) => (v, KpiAvailability::Available, None),
                None => (
                    0.0,
                    KpiAvailability::Empty,
                    Some(format!(
                        "opc_kpi_records 中暂无 {name} 记录（域包 {domain_pack_id}、时间窗 [{from}, {to}]；\
                         工作流产出经 post_exec 钩子入库后可见）"
                    )),
                ),
            }
        },
        KpiSource::NoDataSource(reason) => {
            (0.0, KpiAvailability::NoDataSource, Some(reason.to_string()))
        },
    })
}

/// 内容媒体 KPI 计算 —— **按 key 查注册表**逐项取数（无 if-domain_pack 特例）。
///
/// unit / name 不在此处硬编码：由 runtime.yaml 元数据在展示层补全。
pub async fn compute_content_media_kpis(
    data_service: &Arc<dyn OpcDataService>,
    domain_pack_id: &str,
    time_range: &TimeRange,
) -> OpcResult<Vec<KpiValue>> {
    let now = chrono::Utc::now().timestamp();

    let mut out = Vec::with_capacity(CONTENT_MEDIA_KPI_SOURCES.len());
    for (key, source) in CONTENT_MEDIA_KPI_SOURCES {
        let (value, availability, note) =
            resolve_kpi_value(data_service, domain_pack_id, *source, time_range).await?;
        if !matches!(availability, super::analytics::KpiAvailability::Available) {
            tracing::debug!(
                "[opc-kpi] content_media.{key} 无可用数据: {availability} ({})",
                note.as_deref().unwrap_or("-")
            );
        }
        out.push(KpiValue {
            key: (*key).to_string(),
            value,
            target: None,
            unit: None,
            timestamp: now,
            availability,
            note,
            ..Default::default()
        });
    }
    Ok(out)
}

/// 电子商务 KPI 计算
pub async fn compute_ecommerce_kpis(
    data_service: &Arc<dyn OpcDataService>,
    time_range: &TimeRange,
) -> OpcResult<Vec<KpiValue>> {
    let now = chrono::Utc::now().timestamp();
    let (from, to) = (time_range.start, time_range.end);

    let gmv = data_service.aggregate_invoice_amounts(&[InvoiceStatus::Paid], from, to).await?.total;
    let orders = data_service
        .count_invoices(&[InvoiceStatus::Paid, InvoiceStatus::Sent], from, to)
        .await? as f64;

    Ok(vec![
        KpiValue {
            key: "total_gmv".to_string(),
            value: gmv,
            target: None,
            unit: Some("元".to_string()),
            timestamp: now,
            ..Default::default()
        },
        KpiValue {
            key: "order_count".to_string(),
            value: orders,
            target: None,
            unit: Some("笔".to_string()),
            timestamp: now,
            ..Default::default()
        },
    ])
}

/// 通用 KPI 计算（无特殊逻辑的域包）
pub async fn compute_generic_kpis(
    data_service: &Arc<dyn OpcDataService>,
    time_range: &TimeRange,
    _kpi_definitions: &[KpiDefinition],
) -> OpcResult<Vec<KpiValue>> {
    let now = chrono::Utc::now().timestamp();
    let (from, to) = (time_range.start, time_range.end);

    // 使用通用指标：客户数、项目数、发票总额
    let customer_count = data_service.count_customers(&[], from, to).await? as f64;
    let project_count = data_service.count_projects(&[], from, to).await? as f64;
    let invoice_total = data_service.aggregate_invoice_amounts(&[], from, to).await?.total;

    Ok(vec![
        KpiValue {
            key: "customer_count".to_string(),
            value: customer_count,
            target: None,
            unit: Some("个".to_string()),
            timestamp: now,
            ..Default::default()
        },
        KpiValue {
            key: "project_count".to_string(),
            value: project_count,
            target: None,
            unit: Some("个".to_string()),
            timestamp: now,
            ..Default::default()
        },
        KpiValue {
            key: "invoice_total".to_string(),
            value: invoice_total,
            target: None,
            unit: Some("元".to_string()),
            timestamp: now,
            ..Default::default()
        },
    ])
}

/// 计算域包 KPI（统一入口）
pub async fn compute_kpis(
    domain_pack_id: &str,
    data_service: &Arc<dyn OpcDataService>,
    time_range: &TimeRange,
) -> OpcResult<Vec<KpiValue>> {
    match domain_pack_id.replace('-', "_").as_str() {
        "accounting" => compute_accounting_kpis(data_service, time_range).await,
        "finance_invest" => compute_finance_invest_kpis(data_service, time_range).await,
        "software_dev" => compute_software_dev_kpis(data_service, time_range).await,
        "content_media" => {
            compute_content_media_kpis(data_service, domain_pack_id, time_range).await
        },
        "ecommerce" => compute_ecommerce_kpis(data_service, time_range).await,
        _ => {
            let config = super::domain_pack_config::get_config(domain_pack_id);
            let definitions = config.map(|c| c.kpi_definitions).unwrap_or_default();
            compute_generic_kpis(data_service, time_range, &definitions).await
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opc::analytics::KpiAvailability;
    use crate::opc::data_service::MockDataService;

    /// 注册表与 runtime.yaml 的 key 词表必须一一对应（本次改造的核心契约）。
    ///
    /// runtime.yaml（content_media）声明 7 个 key；注册表少注册一个 ⇔ 该 KPI 在
    /// 仪表盘上永远显示「未接数据源」，且没有任何编译期报错 —— 故用测试钉住。
    #[test]
    fn content_media_registry_covers_all_seven_declared_keys() {
        let mut keys: Vec<&str> = CONTENT_MEDIA_KPI_SOURCES.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys.len(), 7, "runtime.yaml 声明 7 个 KPI，注册表必须全部登记");
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 7, "注册表 key 不得重复（重复会被静默覆盖）");
        for expected in [
            "content_count",
            "page_views",
            "conversion_rate",
            "content_engagement",
            "word_count",
            "completion_rate",
            "revision_rounds",
        ] {
            assert!(
                CONTENT_MEDIA_KPI_SOURCES.iter().any(|(k, _)| *k == expected),
                "runtime.yaml 声明的 key 未注册计算源: {expected}"
            );
        }
    }

    #[test]
    fn kpi_source_lookup_normalizes_domain_pack_id_and_isolates_domain_packs() {
        assert!(matches!(
            kpi_source("content_media", "word_count"),
            Some(KpiSource::RecordedKpi("word_count"))
        ));
        // 连字符形式等价
        assert!(matches!(
            kpi_source("content-media", "page_views"),
            Some(KpiSource::BlogPostViews)
        ));
        // 跨域包不得串号：accounting 的 key 不得命中 content_media 注册表
        assert!(kpi_source("accounting", "invoice_count").is_none());
        assert!(kpi_source("content_media", "no_such_kpi").is_none());
    }

    /// 缺失必须显式：注册表里的 7 个 key 全部有产出，且未接数据源的 3 个
    /// 标记为 `NoDataSource`（值仅为占位），未落库的 3 个标记为 `Empty`。
    #[tokio::test]
    async fn content_media_kpis_mark_missing_sources_explicitly() {
        let ds: Arc<dyn OpcDataService> = Arc::new(MockDataService::default());
        let kpis =
            compute_content_media_kpis(&ds, "content_media", &TimeRange::days(30)).await.unwrap();
        assert_eq!(kpis.len(), 7);

        let find = |key: &str| kpis.iter().find(|k| k.key == key).expect(key).clone();

        // 有真实数据源的 3 项
        assert_eq!(find("content_count").value, 25.0);
        assert_eq!(find("content_count").availability, KpiAvailability::Available);
        assert_eq!(find("page_views").value, 12500.0);
        assert_eq!(find("page_views").availability, KpiAvailability::Available);
        // 转化率：Mock 无联系表单提交 ⇒ 真实值为 0（Available，不是「缺失」）
        let conv = find("conversion_rate");
        assert_eq!(conv.value, 0.0);
        assert_eq!(conv.availability, KpiAvailability::Available);

        // 未接数据源：必须显式标记，且带原因
        let engagement = find("content_engagement");
        assert_eq!(engagement.availability, KpiAvailability::NoDataSource);
        assert!(engagement.note.as_deref().unwrap_or_default().contains("互动"));

        // 依赖落库的 3 项：Mock 无记录 ⇒ Empty（不得把占位 0 当真实值）
        for key in ["word_count", "completion_rate", "revision_rounds"] {
            let k = find(key);
            assert_eq!(k.availability, KpiAvailability::Empty, "{key} 应为「暂无数据」");
            assert_eq!(k.value, 0.0);
            assert!(k.note.is_some(), "{key} 缺失时必须带原因说明");
        }
    }

    /// 计算侧不再硬编码 unit —— 元数据由 runtime.yaml 权威提供。
    #[tokio::test]
    async fn content_media_kpis_do_not_hardcode_units() {
        let ds: Arc<dyn OpcDataService> = Arc::new(MockDataService::default());
        let kpis =
            compute_content_media_kpis(&ds, "content_media", &TimeRange::days(30)).await.unwrap();
        assert!(
            kpis.iter().all(|k| k.unit.is_none()),
            "unit 必须由 runtime.yaml 元数据补全，计算层不得硬编码"
        );
    }
}
