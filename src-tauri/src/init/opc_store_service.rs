// SPDX-License-Identifier: AGPL-3.0-only

//! `opc.store` 接缝的**宿主侧实现**（wiring 层，只调 dao，不直连库）。
//!
//! 契约在 `axagent-core/crates/harness/src/opc_store.rs`，现下十二个 op：读面
//! `list_blog_posts` + 八条列表读 op（发票 / 客户 / 项目 / 落地页 / 联系提交 / KPI /
//! 内容资产 / 发布计划），写面 `publish_blog_post` / `delete_blog_post`（三态）/
//! `create_blog_post`（正文按字节设闸）。本文件只做两件事：入参闸（`limit` / `offset` /
//! `id` / `content` / `slug`）与「dao 行 / 领域 DTO → 契约视图」的逐字段映射
//! （publish / delete 没有视图，只有两态 / 三态枚举）。
//!
//! ## 为什么不在这层写 SQL
//!
//! 分层纪律：列清单、排序键、分页量都属**存储契约**，钉在 dao
//! （`axagent-infra/crates/dao/src/repo/opc_blog_post.rs`）。provider 里再拼一份
//! `Entity::find()` 就会出现两份列清单 —— 新增列只会被加进其中一份，裁剪口径静默漂移
//! （先例判据见 `repo/stock_analysis_rerun_queue.rs` 模块头）。
//! 与 `init/analysis_record_service.rs` 的差别要说清：那份为了 `COUNT(*)` 直接在
//! wiring 层用了 `Entity::find().count()`；本批把计数一并下沉 dao（`count_blog_posts`），
//! 因为这行的接缝**只需要** dao 的两个函数，没有「为一次计数去动既有查询函数」的约束。
//!
//! ## 视图裁剪点唯一
//!
//! 每张表一个映射函数（[`blog_post_view`] + 八个 `*_summary_view`），一个字段只在一处
//! 决定「过不过线」：`content`（博客正文）在 **SQL 侧**就没被 SELECT，到这里的行本来就
//! 没有该列；八条列表视图不带 `body` / `message` / `lineItems` / `milestones` 明细
//! （只带两个计数）。`published` 哨兵收敛与 `tags_json` 解码在 dao 做过，本层不再补第二次。
//!
//! ## 闸（越界拒收，不静默夹取）
//!
//! - `limit`：`None` ⇒ [`OPC_BLOG_POST_LIST_DEFAULT_LIMIT`]；显式值越出
//!   [`OPC_BLOG_POST_LIST_LIMIT_RANGE`] ⇒ `Err(Validation)`。插件侧的 `limit` 参与
//!   「取尽整表」的分页算术，夹取后「我给 500 却只回 200」会让它把下一页 offset 算错
//!   （同 `analysis_record_service.rs` 的 `resolve_limit` 判据）；
//! - `offset`：`None` ⇒ `0`；无上限（越界 offset 就是一次空页，`total` 照常返回）。
//! - `id`（三个写 op 的 publish / delete）：空 / 全空白 ⇒ `Err(Validation)`；缺 id 属入参非法，
//!   与「id 合法但表里没有这篇」是两件事 —— 后者回 `NotFound` 正常值。
//! - `content`（create）：超 [`OPC_BLOG_POST_CONTENT_MAX_BYTES`]（**UTF-8 字节**）⇒
//!   `Err(Validation)`，不截断；`slug` 空 / 全空白同样拒（它是对外路由键，表上 `unique`）。
//!
//! 四类闸都在 IO **之前**：非法参数不该换来一次主键查询 / INSERT。

use std::collections::HashSet;

use async_trait::async_trait;
use axagent_company_runtime::opc::{
    AnalyticsService, ContactSubmission, ContentAsset, ContentAssetService, Customer,
    CustomerFilter, CustomerService, CustomerStatus, CustomerType, DefaultAnalyticsService,
    DefaultCustomerService, DefaultInvoiceService, DefaultProjectService, DefaultSiteService,
    Invoice, InvoiceFilter, InvoiceService, InvoiceStatus, KpiRecord, LandingPage, MilestoneStatus,
    Project, ProjectFilter, ProjectService, ProjectStatus, PublishSchedule, PublishScheduleService,
    RevenueRecord, SiteService,
};
use axagent_dao::repo::opc_blog_post::{BlogPostListRow, count_blog_posts, list_blog_posts};
use axagent_dao::repo::{opc_delivery, opc_demand};
use axagent_harness::DatabaseConnection;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::opc_store::{
    BlogPostListOutcome, BlogPostView, ContactSummaryView, ContentAssetSummaryView,
    CreateBlogPostInput, CustomerListQuery, CustomerSummaryView, DashboardSummaryView,
    InvoiceListQuery, InvoiceSummaryView, KpiListQuery, KpiSummaryView, LandingPageSummaryView,
    OPC_BLOG_POST_CONTENT_MAX_BYTES, OPC_BLOG_POST_LIST_DEFAULT_LIMIT,
    OPC_BLOG_POST_LIST_LIMIT_RANGE, OpcStoreProvider, ProjectListQuery, ProjectSummaryView,
    PublishBlogPostOutcome, PublishScheduleSummaryView, RemoveBlogPostOutcome, RevenueRecordView,
};
use axagent_harness::types::opc_demand::{
    CreateInvoiceFromLeadInput, DeliveryInvoiceDto, DeliverySummary,
};
use axagent_harness::util_fns::now_ts;
use tokio::sync::RwLock;

/// `opc.store` 的宿主实现。
pub struct HostOpcStore {
    /// 与 `AppState.harness.db()` **同一个**连接池句柄（`DatabaseConnection` 是 Clone 的句柄，
    /// 不是第二套连接语义）。
    db: DatabaseConnection,
    /// 需求实现链的宿主服务（#264 批 3）：`convert_lead_to_workflow` 回灌能力索引用
    /// indexer、`run_lead_workflow` 启动执行用 WorkEngine。测试只跑表读写 op，故不装配；
    /// 生产由 `state.rs` 经 [`Self::with_demand_pipeline`] 装配（缺装配的调用回 `Provider` 错）。
    demand: Option<DemandPipeline>,
    /// 受保护 id 名单（内置站点内容）：`delete_blog_post` 对它们回 `Protected` 且**不删除**。
    ///
    /// 名单机制照 `jobs.store` 的三件套（`axagent-runtime/crates/runtime-core/src/cron_job.rs`
    /// 的 `protect_ids` / `is_protected` + `src-tauri/src/init/state.rs` 启动时从种子模块常量登记）。
    /// ⚠ OPC 侧今天**没有种子内容**（`opc_blog_posts` 无 INSERT 种子路径，`init/` 下也没有
    /// OPC 种子模块）⇒ 默认是**空表**，`Protected` 分支由单测经 [`Self::protect_ids`] 注入取证。
    /// 将来有种子内容时在 wiring 同段登记，名单常量放种子模块 —— 不在本层拍一份字面量。
    protected: RwLock<HashSet<String>>,
}

/// 需求实现链的宿主服务句柄（#264 批 3）。
struct DemandPipeline {
    work_engine: std::sync::Arc<axagent_runtime::work_engine::WorkEngine>,
    capability_indexer: std::sync::Arc<dyn axagent_harness::CapabilityIndexer>,
}

impl HostOpcStore {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db, demand: None, protected: RwLock::new(HashSet::new()) }
    }

    /// 装配需求实现链的两项宿主服务（生产唯一装配口，`state.rs` 调用）。
    pub fn with_demand_pipeline(
        mut self,
        work_engine: std::sync::Arc<axagent_runtime::work_engine::WorkEngine>,
        capability_indexer: std::sync::Arc<dyn axagent_harness::CapabilityIndexer>,
    ) -> Self {
        self.demand = Some(DemandPipeline { work_engine, capability_indexer });
        self
    }

    /// 取需求实现链句柄；未装配即回 `Provider` 错（本层 fail-closed，不回退到任何默认值）。
    fn demand_pipeline(&self, op: &str) -> Result<&DemandPipeline> {
        self.demand.as_ref().ok_or_else(|| {
            AxAgentError::Provider(format!(
                "opc.store::{op} 未装配需求实现管线（宿主 wiring 缺 with_demand_pipeline）"
            ))
        })
    }

    /// 登记受保护 id（幂等、只增不减；判据同 `CronJobStore::protect_ids`）。
    pub async fn protect_ids(&self, ids: impl IntoIterator<Item = String>) {
        self.protected.write().await.extend(ids);
    }

    /// 该 id 是否受保护（`delete_blog_post` 据此回 `Protected` 且不发 DELETE）。
    async fn is_protected(&self, id: &str) -> bool {
        self.protected.read().await.contains(id)
    }
}

// ── 纯函数：闸与映射（本文件全部语义的落点）─────────────────────────────────

/// `limit` 闸：越出 [`OPC_BLOG_POST_LIST_LIMIT_RANGE`] ⇒ `Err(Validation)`，**不夹取**。
fn resolve_limit(limit: Option<usize>) -> Result<usize> {
    let effective = limit.unwrap_or(OPC_BLOG_POST_LIST_DEFAULT_LIMIT);
    if (OPC_BLOG_POST_LIST_LIMIT_RANGE.0..=OPC_BLOG_POST_LIST_LIMIT_RANGE.1).contains(&effective) {
        return Ok(effective);
    }
    Err(AxAgentError::Validation(format!(
        "opc.store::list_blog_posts 的 limit={effective} 越出契约区间 {}..={}（越界拒收，不静默夹取：\
         插件按 limit 算分页 offset，夹取会让下一页落在错误位置）",
        OPC_BLOG_POST_LIST_LIMIT_RANGE.0, OPC_BLOG_POST_LIST_LIMIT_RANGE.1
    )))
}

/// dao 行 → 契约视图（**唯一**映射点：字段改名在这里，列裁剪在 SQL 侧）。
///
/// 逐字段一一对应，没有派生 / 换算（时间是 epoch 毫秒，两侧同单位）。
fn blog_post_view(row: BlogPostListRow) -> BlogPostView {
    BlogPostView {
        id: row.id,
        title: row.title,
        slug: row.slug,
        excerpt: row.excerpt,
        tags: row.tags,
        published: row.published,
        published_at: row.published_at,
        view_count: row.view_count,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

// ── Trait 实现 ────────────────────────────────────────────────────────────

#[async_trait]
impl OpcStoreProvider for HostOpcStore {
    /// 列表：一页裁剪行 + `limit` 之前的匹配总数（整表行数）。
    async fn list_blog_posts(
        &self,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<BlogPostListOutcome> {
        // 闸在 IO 之前。
        let limit = resolve_limit(limit)?;
        let offset = offset.unwrap_or(0);

        let rows = list_blog_posts(&self.db, limit, offset).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::list_blog_posts 取博客列表失败: {e}"))
        })?;
        let total = count_blog_posts(&self.db).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::list_blog_posts 计数失败: {e}"))
        })?;

        Ok(BlogPostListOutcome { posts: rows.into_iter().map(blog_post_view).collect(), total })
    }

    /// 发布（写 op）：闸 → dao → 两态枚举。**没有 SQL、没有 `sea_orm` 导入**（分层纪律见模块头）。
    ///
    /// 「缺行」在这里是 `Ok(NotFound)` 而不是错误 —— 判据钉在契约的
    /// [`PublishBlogPostOutcome`]，本层只负责把它从 dao 的 `bool` 翻译成契约枚举，
    /// 不在这里再判一次「行在不在」。
    async fn publish_blog_post(&self, id: String) -> Result<PublishBlogPostOutcome> {
        // 闸在 IO 之前（空 id 不该换来一次主键查询）。
        require_id(&id, "publish_blog_post")?;
        // dao 写函数**全限定调用**：本文件的 `impl` 那一行被登记表按行号钉住，
        // 往上方 `use axagent_dao::...` 那一条里加名字会让它换行、impl 行整体下移 ⇒
        // 行中性改法就是把新符号留在这里（`use` 列表只加了契约枚举，宽度够）。
        let found = axagent_dao::repo::opc_blog_post::set_blog_post_published(&self.db, &id)
            .await
            .map_err(|e| {
                AxAgentError::Provider(format!("opc.store::publish_blog_post 发布失败: {e}"))
            })?;
        Ok(if found {
            PublishBlogPostOutcome::Published
        } else {
            PublishBlogPostOutcome::NotFound
        })
    }

    /// 删除（写 op）：**先判保护、再删**，缺行 / 受保护都走 payload（三态可判别）。
    ///
    /// 两个判据的关键点：
    /// 1. **不裸删**：落库走既有 `SiteService::delete_blog_post`（那条已按
    ///    `rows_affected == 0 ⇒ OpcError::NotFound` 收口，见 `opc/site.rs`）——
    ///    本层不许再出现 `Entity::delete_by_id(..).exec(..)` 不检行数的旧形态；
    /// 2. **保护先于删除**：受保护 id 连一次 DELETE 都不发（`jobs.store` 的 `remove` 同一条
    ///    判据），`Protected` 与 `NotFound` 是两种读数，顺序颠倒会把「不许删」报成「没有这篇」。
    async fn delete_blog_post(&self, id: String) -> Result<RemoveBlogPostOutcome> {
        // 闸在 IO 之前（空 id 不该换来一次主键删除）。
        require_id(&id, "delete_blog_post")?;
        // ① 先判保护：命中即返回，行数据原样（IO 零触达）。
        if self.is_protected(&id).await {
            return Ok(RemoveBlogPostOutcome::Protected);
        }
        // ② 再删：走既有 service（`rows_affected == 0` 已收敛成 `OpcError::NotFound`）。
        let svc = DefaultSiteService::new(self.db.clone());
        match SiteService::delete_blog_post(&svc, &id).await {
            Ok(()) => Ok(RemoveBlogPostOutcome::Removed),
            // 缺行是**正常结果**，不占错误码（判据见契约 `RemoveBlogPostOutcome`）。
            Err(axagent_company_runtime::opc::OpcError::NotFound(_)) => {
                Ok(RemoveBlogPostOutcome::NotFound)
            },
            Err(e) => {
                Err(AxAgentError::Provider(format!("opc.store::delete_blog_post 删除失败: {e}")))
            },
        }
    }

    /// 新建（写 op）：正文按**字节**设闸 → 落既有 service → 回列裁剪视图。
    ///
    /// 闸的落点与 `knowledge.vector` 的 `index_text` 同一条分工：预算数字住在契约层
    /// （[`OPC_BLOG_POST_CONTENT_MAX_BYTES`]），判定住 provider（IO 之前），**不截断**
    /// —— 静默截断会让调用方以为整篇都进库了。
    ///
    /// ⚠ 不拿 `MAX_FRAME_LEN`（64 MiB）当预算：那是帧层能装多少字节，不是本 op 的成本口径。
    async fn create_blog_post(&self, input: CreateBlogPostInput) -> Result<BlogPostView> {
        // 闸在 IO 之前：超预算 / 空 slug 都不该换来一次 INSERT。
        check_blog_post_content_budget(&input.content)?;
        check_blog_post_slug(&input.slug)?;
        let svc = DefaultSiteService::new(self.db.clone());
        let created = SiteService::create_blog_post(&svc, input).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::create_blog_post 创建失败: {e}"))
        })?;
        // 领域 DTO → 接缝视图的唯一换算点在 `company-runtime`（`From<BlogPost> for BlogPostView`），
        // 本层不再列第二份字段名单（禁区 12）。
        Ok(BlogPostView::from(created))
    }

    // ── 只读查询 op（#36 批 1）────────────────────────────────────────
    //
    // 判据三条，八个 op 一致：
    // ① 过滤 / 排序钉在 `company-runtime` 的既有 service（本层不写 SQL、不重列字段名单）；
    // ② 枚举位是**串**：非法值 `.parse().ok()` 后丢弃 = 「不过滤」，与迁移前宿主工具
    //    `OpcList*` 的 `from_str(..).ok()` 逐字一致（迁移不顺手改判，见契约模块注释）；
    // ③ 空表 ⇒ `Ok(vec![])`（不是错误），视图映射是唯一裁剪点。

    async fn list_invoices(&self, query: InvoiceListQuery) -> Result<Vec<InvoiceSummaryView>> {
        let svc = DefaultInvoiceService::new(self.db.clone());
        let rows = svc
            .list_invoices(InvoiceFilter {
                status: query.status.as_deref().and_then(|s| s.parse::<InvoiceStatus>().ok()),
                customer_id: query.customer_id,
                date_from: query.date_from,
                date_to: query.date_to,
                limit: query.limit,
                offset: query.offset,
            })
            .await
            .map_err(|e| {
                AxAgentError::Provider(format!("opc.store::list_invoices 取发票列表失败: {e}"))
            })?;
        Ok(rows.into_iter().map(invoice_summary_view).collect())
    }

    async fn list_customers(&self, query: CustomerListQuery) -> Result<Vec<CustomerSummaryView>> {
        let svc = DefaultCustomerService::new(self.db.clone());
        // 存量回填：未分类（unknown）客户按 company 归类为 business / consumer，幂等。
        // 宿主同名命令 `opc_list_customers` 退役后本入口是唯一触发点（迁移时保持行为等价）。
        if let Err(e) = svc.backfill_customer_types().await {
            tracing::warn!(
                "[opc.store::list_customers] 存量客户类型回填失败（忽略，继续查询）: {e}"
            );
        }
        let rows = svc
            .list_customers(CustomerFilter {
                status: query.status.as_deref().and_then(|s| s.parse::<CustomerStatus>().ok()),
                customer_type: query
                    .customer_type
                    .as_deref()
                    .and_then(|s| s.parse::<CustomerType>().ok()),
                search: query.search,
                // `CustomerFilter.tags` 恒 `None`：域服务没有 tags 分支（同迁移前宿主工具），
                // 契约 DTO 也因此不提供这个键 —— 见 `opc_store.rs` 里 `list_customers` 的说明。
                tags: None,
                limit: query.limit,
                offset: query.offset,
            })
            .await
            .map_err(|e| {
                AxAgentError::Provider(format!("opc.store::list_customers 取客户列表失败: {e}"))
            })?;
        Ok(rows.into_iter().map(customer_summary_view).collect())
    }

    async fn list_projects(&self, query: ProjectListQuery) -> Result<Vec<ProjectSummaryView>> {
        let svc = DefaultProjectService::new(self.db.clone());
        let rows = svc
            .list_projects(ProjectFilter {
                status: query.status.as_deref().and_then(|s| s.parse::<ProjectStatus>().ok()),
                customer_id: query.customer_id,
                search: query.search,
                limit: query.limit,
                offset: query.offset,
            })
            .await
            .map_err(|e| {
                AxAgentError::Provider(format!("opc.store::list_projects 取项目列表失败: {e}"))
            })?;
        Ok(rows.iter().map(project_summary_view).collect())
    }

    async fn list_landing_pages(&self) -> Result<Vec<LandingPageSummaryView>> {
        let svc = DefaultSiteService::new(self.db.clone());
        let rows = SiteService::list_landing_pages(&svc).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::list_landing_pages 取落地页失败: {e}"))
        })?;
        Ok(rows.into_iter().map(landing_page_summary_view).collect())
    }

    async fn list_contacts(&self) -> Result<Vec<ContactSummaryView>> {
        let svc = DefaultSiteService::new(self.db.clone());
        let rows = SiteService::list_contacts(&svc).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::list_contacts 取联系表单失败: {e}"))
        })?;
        Ok(rows.into_iter().map(contact_summary_view).collect())
    }

    async fn list_kpis(&self, query: KpiListQuery) -> Result<Vec<KpiSummaryView>> {
        let svc = DefaultAnalyticsService::new(self.db.clone());
        // 跨域包读（`list_kpis_all`）：本接缝的调用方不携带域包标识，与迁移前
        // `OpcListKpis` 同一条口径；每行都带 `domain_pack_id`（空串 = 未标注）。
        let rows = svc.list_kpis_all(query.period, query.limit).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::list_kpis 取 KPI 记录失败: {e}"))
        })?;
        Ok(rows.into_iter().map(kpi_summary_view).collect())
    }

    async fn list_content_assets(&self) -> Result<Vec<ContentAssetSummaryView>> {
        let svc = DefaultSiteService::new(self.db.clone());
        // 全限定调用：`DefaultSiteService` 同时实现 `SiteService` / `ContentAssetService` /
        // `PublishScheduleService`，同名方法在导入多个 trait 时会有歧义。
        let rows = ContentAssetService::list_content_assets(&svc).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::list_content_assets 取内容资产失败: {e}"))
        })?;
        Ok(rows.into_iter().map(content_asset_summary_view).collect())
    }

    async fn list_publish_schedules(&self) -> Result<Vec<PublishScheduleSummaryView>> {
        let svc = DefaultSiteService::new(self.db.clone());
        let rows = PublishScheduleService::list_publish_schedules(&svc).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::list_publish_schedules 取发布计划失败: {e}"))
        })?;
        Ok(rows.into_iter().map(publish_schedule_summary_view).collect())
    }

    /// 看板总览（读 op，不设权限闸）。
    ///
    /// 与迁移前宿主命令 `opc_get_dashboard_summary` 同一口径：直接调域服务的
    /// `get_dashboard_summary_overview`（整表聚合，无 limit）。**不要**改用 `list_*` 自行加总 ——
    /// 那些 op 是「一页行」，带 `limit` 会静默少计。
    async fn dashboard_summary(&self) -> Result<DashboardSummaryView> {
        let svc = DefaultAnalyticsService::new(self.db.clone());
        let summary = svc.get_dashboard_summary_overview().await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::dashboard_summary 取看板总览失败: {e}"))
        })?;
        Ok(DashboardSummaryView {
            total_revenue: summary.total_revenue,
            total_invoices: summary.total_invoices,
            active_projects: summary.active_projects,
            total_customers: summary.total_customers,
            recent_kpis: summary.recent_kpis.into_iter().map(kpi_summary_view).collect(),
            revenue_trend: summary.revenue_trend.into_iter().map(revenue_record_view).collect(),
        })
    }

    // ── 交付域五 op（#264 批 1）────────────────────────────────────────
    //
    // 与迁移前宿主命令模块（`src-tauri/src/commands/opc_delivery.rs`，#264 批 1 已删除）
    // 逐句同源：底层只调 dao
    // （`repo::opc_delivery` / `repo::opc_demand`），本层不写 SQL。
    // 交付账本只列 `lead_id IS NOT NULL` 的交付来源行 —— 这条过滤在 dao 的
    // `list_invoices` 里，本层不重复（域包发票与交付发票共用 `opc_invoices` 表）。

    async fn list_delivery_invoices(
        &self,
        status: Option<String>,
    ) -> Result<Vec<DeliveryInvoiceDto>> {
        opc_delivery::list_invoices(&self.db, status).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::list_delivery_invoices 取交付发票失败: {e}"))
        })
    }

    async fn create_invoice_from_lead(
        &self,
        lead_id: String,
        input: CreateInvoiceFromLeadInput,
    ) -> Result<DeliveryInvoiceDto> {
        let lead = opc_demand::get_lead(&self.db, &lead_id).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::create_invoice_from_lead 取线索失败: {e}"))
        })?;
        if lead.status != "won" {
            return Err(AxAgentError::Validation(format!(
                "仅 won 线索可开票，当前状态: {}（线索 {lead_id}）",
                lead.status
            )));
        }

        // 幂等：已有发票直接返回，不重复开
        let existing =
            opc_delivery::list_invoices_by_lead(&self.db, &lead_id).await.map_err(|e| {
                AxAgentError::Provider(format!("opc.store::create_invoice_from_lead 查重失败: {e}"))
            })?;
        if let Some(inv) = existing.first() {
            return Ok(inv.clone());
        }

        let amount = input.amount.or(lead.budget_max).or(lead.budget_min).unwrap_or(0.0);
        let filled = CreateInvoiceFromLeadInput {
            title: input.title.or(Some(lead.title.clone())),
            amount: Some(amount),
            currency: Some(
                input
                    .currency
                    .or_else(|| {
                        if lead.budget_currency.is_empty() {
                            None
                        } else {
                            Some(lead.budget_currency.clone())
                        }
                    })
                    .unwrap_or_else(|| "CNY".to_string()),
            ),
            notes: input.notes,
        };

        opc_delivery::create_invoice(
            &self.db,
            &lead_id,
            lead.linked_workflow_id.clone(),
            &filled,
            now_ts(),
        )
        .await
        .map_err(|e| {
            AxAgentError::Provider(format!("opc.store::create_invoice_from_lead 开票失败: {e}"))
        })
    }

    async fn update_delivery_invoice_status(
        &self,
        invoice_id: String,
        status: String,
    ) -> Result<DeliveryInvoiceDto> {
        // 枚举串在 IO 之前校验：非法值不该换来一次主键查询（同 transition_invoice）。
        if !matches!(status.as_str(), "draft" | "sent" | "paid") {
            return Err(AxAgentError::Validation(format!(
                "非法发票状态: {status}（合法值 draft/sent/paid）"
            )));
        }
        opc_delivery::update_invoice_status(&self.db, &invoice_id, &status, now_ts()).await.map_err(
            |e| {
                AxAgentError::Provider(format!(
                    "opc.store::update_delivery_invoice_status 推进失败: {e}"
                ))
            },
        )
    }

    async fn delete_delivery_invoice(&self, id: String) -> Result<()> {
        require_id(&id, "delete_delivery_invoice")?;
        opc_delivery::delete_invoice(&self.db, &id).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::delete_delivery_invoice 删除失败: {e}"))
        })
    }

    async fn delivery_summary(&self) -> Result<DeliverySummary> {
        // 线索侧统计：个人公司量级（几十~几百条）全量拉取无压力，不值得上聚合 SQL
        // （与迁移前宿主命令同一判断）。
        let leads = opc_demand::list_leads(&self.db, 10_000, None, None).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::delivery_summary 取线索失败: {e}"))
        })?;
        let won_leads = leads.iter().filter(|l| l.status == "won").count() as u32;
        let active_leads = leads.iter().filter(|l| l.status != "lost").count() as u32;

        let mut summary =
            opc_delivery::delivery_summary(&self.db, won_leads, active_leads).await.map_err(
                |e| AxAgentError::Provider(format!("opc.store::delivery_summary 汇总失败: {e}")),
            )?;

        // 空库兜底：发票表为零时给 0 转化率而非 NaN（active=0 时 dao 已处理）。
        if summary.active_leads == 0 {
            summary.conversion_rate = 0.0;
        }
        Ok(summary)
    }

    // ── 写 op（#36 批 2：id / 枚举形状）────────────────────────────────
    //
    // 四条判据一致：空 id ⇒ `Err(Validation)`（IO **之前**）；行不存在 ⇒ payload
    // （`NotFound` / `"not_found"`，**不占错误码**）；其余失败才是 `Err(Provider)`。
    // 出参视图一律走既有 `*_summary_view` 映射 —— 列裁剪只有一个落点。

    async fn transition_invoice(
        &self,
        id: String,
        target: String,
    ) -> Result<TransitionInvoiceOutcome> {
        require_id(&id, "transition_invoice")?;
        // 枚举串在 IO 之前解析：非法值不该换来一次主键查询。
        let target = InvoiceStatus::from_str(&target).map_err(|_| {
            AxAgentError::Validation(format!(
                "opc.store::transition_invoice 的 target=`{target}` 不是合法发票状态"
            ))
        })?;
        let svc = DefaultInvoiceService::new(self.db.clone());
        match InvoiceService::transition_status(&svc, &id, target).await {
            Ok(inv) => Ok(TransitionInvoiceOutcome::Transitioned(invoice_summary_view(inv))),
            Err(axagent_company_runtime::opc::OpcError::NotFound(_)) => {
                Ok(TransitionInvoiceOutcome::NotFound)
            },
            // 非法状态转换（`InvalidStateTransition`）与库错误都落 Provider：迁移前宿主工具
            // 把它们包成 `execution_failed`，本批不改判。
            Err(e) => {
                Err(AxAgentError::Provider(format!("opc.store::transition_invoice 变更失败: {e}")))
            },
        }
    }

    async fn update_content_asset(
        &self,
        id: String,
        input: UpdateContentAssetInput,
    ) -> Result<UpdateContentAssetOutcome> {
        require_id(&id, "update_content_asset")?;
        let svc = DefaultSiteService::new(self.db.clone());
        // 全限定调用：`DefaultSiteService` 同时实现三个服务 trait，同名方法有歧义。
        match ContentAssetService::update_content_asset(&svc, &id, input).await {
            Ok(asset) => Ok(UpdateContentAssetOutcome::Updated(content_asset_summary_view(asset))),
            Err(axagent_company_runtime::opc::OpcError::NotFound(_)) => {
                Ok(UpdateContentAssetOutcome::NotFound)
            },
            Err(e) => Err(AxAgentError::Provider(format!(
                "opc.store::update_content_asset 更新失败: {e}"
            ))),
        }
    }

    async fn delete_content_asset(&self, id: String) -> Result<RemoveContentAssetOutcome> {
        require_id(&id, "delete_content_asset")?;
        let svc = DefaultSiteService::new(self.db.clone());
        match ContentAssetService::delete_content_asset(&svc, &id).await {
            Ok(()) => Ok(RemoveContentAssetOutcome::Removed),
            Err(axagent_company_runtime::opc::OpcError::NotFound(_)) => {
                Ok(RemoveContentAssetOutcome::NotFound)
            },
            Err(e) => Err(AxAgentError::Provider(format!(
                "opc.store::delete_content_asset 删除失败: {e}"
            ))),
        }
    }

    async fn cancel_publish_schedule(&self, id: String) -> Result<RemovePublishScheduleOutcome> {
        require_id(&id, "cancel_publish_schedule")?;
        let svc = DefaultSiteService::new(self.db.clone());
        // 语义是**硬删**（域服务 `delete_publish_schedule` 走 `delete_by_id`），不是状态位改写。
        match PublishScheduleService::delete_publish_schedule(&svc, &id).await {
            Ok(()) => Ok(RemovePublishScheduleOutcome::Removed),
            Err(axagent_company_runtime::opc::OpcError::NotFound(_)) => {
                Ok(RemovePublishScheduleOutcome::NotFound)
            },
            Err(e) => Err(AxAgentError::Provider(format!(
                "opc.store::cancel_publish_schedule 取消失败: {e}"
            ))),
        }
    }

    // ── 写 op（#36 批 3：创建 / 流程动作）────────────────────────────
    //
    // 判据三条，八个写 op 一致：
    // ① 缺省值（货币 `CNY` / 空串 / 行项目数量 1）在这里补 —— 契约 DTO 用 `Option`
    //    表达「调用方没给」，到域服务入参才落成具体值（与迁移前宿主工具逐字一致）；
    // ② 枚举串翻译也在本层（接缝不认识实现层枚举），未知值按域服务的既有兜底语义；
    // ③ 出参一律走既有 `*_summary_view` 映射 —— 列裁剪只有一个落点。

    async fn create_invoice(&self, request: CreateInvoiceRequest) -> Result<InvoiceSummaryView> {
        let svc = DefaultInvoiceService::new(self.db.clone());
        let line_items = request
            .line_items
            .into_iter()
            .map(|item| {
                let quantity = item.quantity.unwrap_or(1.0);
                let unit_price = item.unit_price.unwrap_or(0.0);
                let tax_rate = item.tax_rate.unwrap_or(0.0);
                InvoiceLineItem {
                    description: item.description.unwrap_or_default(),
                    quantity,
                    unit_price,
                    tax_rate,
                    // 合计由宿主现算：与迁移前宿主工具同一公式，调用方不自报 total。
                    total: quantity * unit_price * (1.0 + tax_rate),
                }
            })
            .collect();
        let input = CreateInvoiceInput {
            customer_id: request.customer_id,
            line_items,
            currency: request.currency.unwrap_or_else(|| "CNY".to_string()),
            due_at: request.due_at,
            notes: request.notes.unwrap_or_default(),
        };
        let inv = InvoiceService::create_invoice(&svc, input).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::create_invoice 创建失败: {e}"))
        })?;
        Ok(invoice_summary_view(inv))
    }

    async fn create_customer(&self, request: CreateCustomerRequest) -> Result<CustomerSummaryView> {
        let svc = DefaultCustomerService::new(self.db.clone());
        // 来源与类型的解码语义逐字抄迁移前宿主工具：五个已知来源原样映射，其余落
        // `Other(原串)`；类型只看小写后的 `consumer` / `business`，其余落 `Unknown`。
        let source = request.source.map(|s| match s.as_str() {
            "Referral" => CustomerSource::Referral,
            "Website" => CustomerSource::Website,
            "SocialMedia" => CustomerSource::SocialMedia,
            "Marketplace" => CustomerSource::Marketplace,
            "Direct" => CustomerSource::Direct,
            other => CustomerSource::Other(other.to_string()),
        });
        let customer_type = request
            .customer_type
            .map(|s| s.to_lowercase())
            .map(|s| match s.as_str() {
                "consumer" => CustomerType::Consumer,
                "business" => CustomerType::Business,
                _ => CustomerType::Unknown,
            })
            .unwrap_or(CustomerType::Unknown);
        let input = CreateCustomerInput {
            name: request.name,
            email: request.email,
            phone: request.phone,
            company: request.company,
            customer_type,
            country: request.country,
            region: request.region,
            city: request.city,
            address: request.address,
            latitude: request.latitude,
            longitude: request.longitude,
            source,
            // 新建不带标签：域服务的入参位存在，但迁移前工具恒传空数组。
            tags: Vec::new(),
            notes: request.notes.unwrap_or_default(),
        };
        let c = CustomerService::create_customer(&svc, input).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::create_customer 创建失败: {e}"))
        })?;
        Ok(customer_summary_view(c))
    }

    async fn create_project(&self, request: CreateProjectRequest) -> Result<ProjectSummaryView> {
        let svc = DefaultProjectService::new(self.db.clone());
        let input = CreateProjectInput {
            customer_id: request.customer_id,
            title: request.title,
            description: request.description.unwrap_or_default(),
            budget: request.budget,
            currency: request.currency.unwrap_or_else(|| "CNY".to_string()),
            deadline: request.deadline,
            notes: request.notes.unwrap_or_default(),
        };
        let p = ProjectService::create_project(&svc, input).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::create_project 创建失败: {e}"))
        })?;
        Ok(project_summary_view(&p))
    }

    async fn add_milestone(&self, request: AddMilestoneRequest) -> Result<ProjectSummaryView> {
        require_id(&request.project_id, "add_milestone")?;
        let svc = DefaultProjectService::new(self.db.clone());
        let milestone = Milestone {
            id: axagent_harness::util_fns::gen_id(),
            title: request.title,
            description: request.description.unwrap_or_default(),
            due_at: request.due_at,
            completed_at: None,
            status: MilestoneStatus::Pending,
        };
        let p = ProjectService::add_milestone(&svc, &request.project_id, milestone).await.map_err(
            |e| AxAgentError::Provider(format!("opc.store::add_milestone 添加失败: {e}")),
        )?;
        Ok(project_summary_view(&p))
    }

    async fn create_landing_page(
        &self,
        request: CreateLandingPageRequest,
    ) -> Result<LandingPageSummaryView> {
        let svc = DefaultSiteService::new(self.db.clone());
        let input = CreateLandingPageInput {
            title: request.title,
            slug: request.slug,
            description: request.description.unwrap_or_default(),
            content: request.content,
        };
        let lp = SiteService::create_landing_page(&svc, input).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::create_landing_page 创建失败: {e}"))
        })?;
        Ok(landing_page_summary_view(lp))
    }

    async fn get_financial_report(&self, period: String) -> Result<FinancialReportView> {
        let svc = DefaultFinanceService::new(self.db.clone());
        let report = FinanceService::get_financial_report(&svc, &period).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::get_financial_report 生成报表失败: {e}"))
        })?;
        // 建议由报告派生，读数成对返回 —— 分两个 op 只会多一次帧往返。
        let advice = FinanceService::get_investment_advice(&svc, &report).await;
        Ok(FinancialReportView {
            period: report.period,
            total_revenue: report.total_revenue,
            pending_revenue: report.pending_revenue,
            refunded_amount: report.refunded_amount,
            active_projects: report.active_projects,
            new_customers: report.new_customers,
            net_profit: report.net_profit,
            investable_amount: report.investable_amount,
            churned_customers: report.churned_customers,
            pretax_revenue: report.pretax_revenue,
            risk_level: advice.risk_level,
            recommended_amount: advice.recommended_amount,
            advice: advice.advice,
        })
    }

    async fn create_content_asset(
        &self,
        request: CreateContentAssetRequest,
    ) -> Result<ContentAssetSummaryView> {
        let svc = DefaultSiteService::new(self.db.clone());
        let input = CreateContentAssetInput {
            title: request.title,
            content_type: request.content_type,
            body: request.body,
            tags: request.tags.unwrap_or_default(),
        };
        let asset = ContentAssetService::create_content_asset(&svc, input).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::create_content_asset 创建失败: {e}"))
        })?;
        Ok(content_asset_summary_view(asset))
    }

    async fn create_publish_schedule(
        &self,
        request: CreatePublishScheduleRequest,
    ) -> Result<PublishScheduleSummaryView> {
        let svc = DefaultSiteService::new(self.db.clone());
        let input = CreatePublishScheduleInput {
            content_ref_type: request.content_ref_type,
            content_ref_id: request.content_ref_id,
            scheduled_at: request.scheduled_at,
        };
        let schedule =
            PublishScheduleService::create_publish_schedule(&svc, input).await.map_err(|e| {
                AxAgentError::Provider(format!("opc.store::create_publish_schedule 创建失败: {e}"))
            })?;
        Ok(publish_schedule_summary_view(schedule))
    }

    async fn process_due_schedules(&self) -> Result<Vec<PublishScheduleSummaryView>> {
        let svc = DefaultSiteService::new(self.db.clone());
        let rows = PublishScheduleService::process_due_schedules(&svc).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::process_due_schedules 处理失败: {e}"))
        })?;
        Ok(rows.into_iter().map(publish_schedule_summary_view).collect())
    }

    // ── #32 前端刀：客户编辑 / 客户删除 / 发票删除 ─────────────────────────
    //
    // 前端 OPC 页面（InvoicesTab / CustomersTab）迁插件通道的缺口补齐：域服务 trait
    // 本就有这三个方法（`CustomerService::{update,delete}_customer`、
    // `InvoiceService::delete_invoice`），接缝此前没透。缺行 = `Err(Provider)`（域服务
    // `NotFound` 透传；不造「两态 Outcome」，域服务没有那个分支）。

    async fn update_customer(
        &self,
        id: String,
        request: UpdateCustomerRequest,
    ) -> Result<CustomerSummaryView> {
        require_id(&id, "update_customer")?;
        let svc = DefaultCustomerService::new(self.db.clone());
        // `customerType` 串解码同 `create_customer`（小写后 consumer/business 之外落 Unknown）。
        let customer_type =
            request.customer_type.map(|s| s.to_lowercase()).map(|s| match s.as_str() {
                "consumer" => CustomerType::Consumer,
                "business" => CustomerType::Business,
                _ => CustomerType::Unknown,
            });
        // `name` / `email` 在域包是单层 `Option<String>`（其余可空列是 `Option<Option<T>>`
        // 双层），而接缝 DTO 统一双层承载「缺席=不动 / null=清空」语义。线上 `Some(None)`
        // 不可达（JSON null 落外层），故 `flatten()` 后 `None` = 不动，语义等价。
        let input = UpdateCustomerInput {
            name: request.name.flatten(),
            email: request.email.flatten(),
            phone: request.phone,
            company: request.company,
            customer_type,
            country: request.country,
            region: request.region,
            city: request.city,
            address: request.address,
            latitude: request.latitude,
            longitude: request.longitude,
            tags: None,
            notes: request.notes,
        };
        let c = CustomerService::update_customer(&svc, &id, input).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::update_customer 更新失败: {e}"))
        })?;
        Ok(customer_summary_view(c))
    }

    async fn delete_customer(&self, id: String) -> Result<()> {
        require_id(&id, "delete_customer")?;
        let svc = DefaultCustomerService::new(self.db.clone());
        CustomerService::delete_customer(&svc, &id).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::delete_customer 删除失败: {e}"))
        })
    }

    async fn delete_invoice(&self, id: String) -> Result<()> {
        require_id(&id, "delete_invoice")?;
        let svc = DefaultInvoiceService::new(self.db.clone());
        InvoiceService::delete_invoice(&svc, &id)
            .await
            .map_err(|e| AxAgentError::Provider(format!("opc.store::delete_invoice 删除失败: {e}")))
    }

    // ── #32 批 B：项目详情 / 更新 / 删除 / 里程碑完成 ─────────────────────
    //
    // 四个 op 与批 A 三条同型：空 id ⇒ `Err(Validation)`（`require_id`，在 IO **之前**）；
    // 缺行 ⇒ 域服务 `NotFound` 透传成 `Err(Provider)`（不借「两态 Outcome」表达缺行）。

    async fn get_project(&self, id: String) -> Result<ProjectDetailView> {
        require_id(&id, "get_project")?;
        let svc = DefaultProjectService::new(self.db.clone());
        let p = ProjectService::get_project(&svc, &id).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::get_project 取项目失败: {e}"))
        })?;
        Ok(project_detail_view(p))
    }

    async fn update_project(
        &self,
        id: String,
        request: UpdateProjectRequest,
    ) -> Result<ProjectDetailView> {
        require_id(&id, "update_project")?;
        let svc = DefaultProjectService::new(self.db.clone());
        // `title` / `description` / `notes` 在域包是单层 `Option<String>`（`budget` /
        // `deadline` 才是双层）⇒ `flatten()` 口径同 `update_customer`：线上 `Some(None)`
        // 不可达，`None` = 不动。
        let input = UpdateProjectInput {
            title: request.title.flatten(),
            description: request.description.flatten(),
            budget: request.budget,
            deadline: request.deadline,
            notes: request.notes.flatten(),
        };
        let p = ProjectService::update_project(&svc, &id, input).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::update_project 更新失败: {e}"))
        })?;
        Ok(project_detail_view(p))
    }

    async fn delete_project(&self, id: String) -> Result<()> {
        require_id(&id, "delete_project")?;
        let svc = DefaultProjectService::new(self.db.clone());
        ProjectService::delete_project(&svc, &id)
            .await
            .map_err(|e| AxAgentError::Provider(format!("opc.store::delete_project 删除失败: {e}")))
    }

    async fn complete_milestone(
        &self,
        project_id: String,
        milestone_id: String,
    ) -> Result<ProjectDetailView> {
        // 两个 id 都设闸：里程碑 id 空串会让域服务在明细里空转一圈再回 NotFound
        // （那是「这一行不存在」的读数，与「入参非法」不是同一档）。
        require_id(&project_id, "complete_milestone")?;
        require_id(&milestone_id, "complete_milestone")?;
        let svc = DefaultProjectService::new(self.db.clone());
        let p =
            ProjectService::complete_milestone(&svc, &project_id, &milestone_id).await.map_err(
                |e| AxAgentError::Provider(format!("opc.store::complete_milestone 完成失败: {e}")),
            )?;
        Ok(project_detail_view(p))
    }

    async fn publish_landing_page(&self, id: String) -> Result<LandingPageSummaryView> {
        require_id(&id, "publish_landing_page")?;
        let svc = DefaultSiteService::new(self.db.clone());
        let lp = SiteService::publish_landing_page(&svc, &id).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::publish_landing_page 发布失败: {e}"))
        })?;
        Ok(landing_page_summary_view(lp))
    }

    async fn delete_landing_page(&self, id: String) -> Result<()> {
        require_id(&id, "delete_landing_page")?;
        let svc = DefaultSiteService::new(self.db.clone());
        SiteService::delete_landing_page(&svc, &id).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::delete_landing_page 删除失败: {e}"))
        })
    }

    async fn mark_contact_read(&self, id: String) -> Result<()> {
        require_id(&id, "mark_contact_read")?;
        let svc = DefaultSiteService::new(self.db.clone());
        SiteService::mark_contact_read(&svc, &id).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::mark_contact_read 标记失败: {e}"))
        })
    }

    // ── 需求发现八 op（#264 批 2）──────────────────────────────────────
    //
    // 实现体与迁移前 `commands::opc_demand_discovery` 逐句同源（该模块已整文件删除）：
    // 平台 CRUD 走 `dao::repo::opc_demand`，扫描/评估/策略复用 `axagent_tools` 的
    // `opc_demand_scan` 与 `marketplace_scanner` —— 不在此处新写一份扫描逻辑。

    async fn list_platforms(&self) -> Result<Vec<DemandPlatform>> {
        opc_demand::seed_default_platforms_if_empty(&self.db).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::list_platforms 种默认平台失败: {e}"))
        })?;
        opc_demand::list_platforms(&self.db).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::list_platforms 取平台失败: {e}"))
        })
    }

    async fn save_platform(&self, input: SaveDemandPlatformInput) -> Result<DemandPlatform> {
        opc_demand::save_platform(&self.db, input)
            .await
            .map_err(|e| AxAgentError::Provider(format!("opc.store::save_platform 保存失败: {e}")))
    }

    async fn delete_platform(&self, id: String) -> Result<()> {
        require_id(&id, "delete_platform")?;
        opc_demand::delete_platform(&self.db, &id).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::delete_platform 删除失败: {e}"))
        })
    }

    async fn list_leads(
        &self,
        limit: Option<u64>,
        min_score: Option<f64>,
        status: Option<String>,
    ) -> Result<Vec<DemandLeadDto>> {
        opc_demand::list_leads(&self.db, limit.unwrap_or(100).min(500), min_score, status)
            .await
            .map_err(|e| AxAgentError::Provider(format!("opc.store::list_leads 取线索失败: {e}")))
    }

    async fn get_scan_policy(&self) -> Result<ScanPolicy> {
        opc_demand_scan::load_scan_policy(&self.db).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::get_scan_policy 读策略失败: {e}"))
        })
    }

    async fn save_scan_policy(&self, policy: ScanPolicy) -> Result<ScanPolicy> {
        let normalized = policy.normalized();
        let json = serde_json::to_string(&normalized).map_err(|e| {
            AxAgentError::Provider(format!("opc.store::save_scan_policy 序列化失败: {e}"))
        })?;
        axagent_dao::repo::settings::set_setting(&self.db, SCAN_POLICY_SETTING_KEY, &json)
            .await
            .map_err(|e| {
                AxAgentError::Provider(format!("opc.store::save_scan_policy 写入失败: {e}"))
            })?;
        tracing::info!(
            concurrency = normalized.concurrency,
            rate_limit = normalized.rate_limit_per_min,
            retry_max = normalized.retry_max,
            dedup_window_hours = normalized.dedup_window_hours,
            "[opc_demand] 扫描策略已保存"
        );
        Ok(normalized)
    }

    async fn discover_and_evaluate_leads(&self, query: String) -> Result<DiscoverLeadsSummary> {
        let query = query.trim().to_string();
        if query.is_empty() {
            return Err(AxAgentError::Validation("query 不能为空".into()));
        }
        opc_demand_scan::run_discovery_scan(&self.db, &query, &[]).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::discover_and_evaluate_leads 扫描失败: {e}"))
        })
    }

    async fn create_lead(&self, input: SaveDemandLeadInput) -> Result<DemandLeadDto> {
        let title = input.title.trim().to_string();
        let description = input.description.trim().to_string();
        if title.is_empty() || description.is_empty() {
            return Err(AxAgentError::Validation("title 与 description 不能为空".into()));
        }

        let policy = opc_demand_scan::load_scan_policy(&self.db).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::create_lead 读策略失败: {e}"))
        })?;

        // URL 清洗：空串归一为 None（否则 new_from_raw 会存出 Some("")）
        let source_url =
            input.source_url.clone().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());

        let raw = RawLead {
            platform: MANUAL_PLATFORM.to_string(),
            title,
            description,
            url: source_url.clone().unwrap_or_default(),
            price_text: None, // 手动预算走结构化字段，不走价格文本解析
            contact: input.contact_name.clone(),
            contact_email: input.contact_email.clone(),
            contact_phone: input.contact_phone.clone(),
            snapshot: serde_json::json!({ "source": "manual" }),
        };
        let mut lead = DemandLead::new_from_raw(raw);
        // 手动填写的预算覆盖自动解析（用户没填的字段保留自动提取结果）
        if input.budget_min.is_some() || input.budget_max.is_some() {
            lead.budget_min = input.budget_min;
            lead.budget_max = input.budget_max;
        }
        if let Some(currency) =
            input.budget_currency.as_deref().map(str::trim).filter(|c| !c.is_empty())
        {
            lead.budget_currency = currency.to_string();
        }
        lead.source_url = source_url;

        let evaluation = evaluate_lead(&lead);
        let evaluated = EvaluatedDemandLead { lead, evaluation };
        opc_demand::create_manual_lead(
            &self.db,
            opc_demand_scan::evaluated_to_row(&evaluated),
            policy.dedup_window_secs(),
        )
        .await
        .map_err(|e| AxAgentError::Provider(format!("opc.store::create_lead 补录失败: {e}")))
    }

    // ── #264 批 3：需求实现三 op（迁移前 `commands::opc_demand_implement` 的命令体）──

    /// 与迁移前命令逐句同源：dao 判定合法迁移 / 终态幂等，错误原样上抛（保留
    /// `Validation` / `NotFound` 的类别，不在这里塌成 `Provider`）。
    async fn update_lead_status(&self, lead_id: String, status: String) -> Result<DemandLeadDto> {
        opc_demand::update_lead_status(&self.db, &lead_id, &status).await
    }

    /// 与迁移前命令逐句同源：读线索 → 生成模板 → 写模板表 → 回灌能力索引 → 回写线索。
    ///
    /// 「回灌索引」必须在模板落库之后、回写线索之前 —— 迁移前注释已说明：不回灌则
    /// 刚生成的模板在本会话内不可路由（`sync_passport` 只在索引失败时 warn，不回滚）。
    async fn convert_lead_to_workflow(&self, lead_id: String) -> Result<WorkflowTemplateData> {
        let pipeline = self.demand_pipeline("convert_lead_to_workflow")?;
        let lead = opc_demand::get_lead(&self.db, &lead_id).await?;

        if let Some(wid) = &lead.linked_workflow_id {
            return Err(AxAgentError::Validation(format!("线索已转化，工作流 ID: {wid}")));
        }

        let template = generate_lead_workflow_template(&lead);
        let active_model = build_active_model_from_data(&template);
        insert_workflow_template(&self.db, active_model).await.map_err(|e| {
            AxAgentError::Provider(format!("opc.store::convert_lead_to_workflow 创建模板失败: {e}"))
        })?;

        crate::commands::workflow_template::sync_passport(
            pipeline.capability_indexer.as_ref(),
            &template,
        )
        .await;

        opc_demand::link_lead_to_workflow(&self.db, &lead_id, &template.id).await?;

        tracing::info!(
            lead_id = %lead_id,
            workflow_id = %template.id,
            "[opc_demand] 线索已转化为工作流"
        );
        Ok(template)
    }

    /// 与迁移前命令逐句同源：前置 = 线索已转化；执行输入为需求简报（与 LLM 节点 prompt
    /// 同源）；启动成功后回写 `implemented_at`，出参为执行 id。
    async fn run_lead_workflow(&self, lead_id: String) -> Result<String> {
        let pipeline = self.demand_pipeline("run_lead_workflow")?;
        let lead = opc_demand::get_lead(&self.db, &lead_id).await?;

        let workflow_id = lead.linked_workflow_id.clone().ok_or_else(|| {
            AxAgentError::Validation(
                "线索尚未转化为工作流，请先调用 opc_convert_lead_to_workflow".into(),
            )
        })?;

        let execution_id = pipeline
            .work_engine
            .start_workflow(
                &workflow_id,
                serde_json::json!({ "lead_brief": build_lead_brief(&lead) }),
                None,
            )
            .await
            .map_err(|e| {
                AxAgentError::Provider(format!("opc.store::run_lead_workflow 启动执行失败: {e}"))
            })?;

        opc_demand::mark_lead_implemented(&self.db, &lead_id).await?;

        tracing::info!(
            lead_id = %lead_id,
            workflow_id = %workflow_id,
            execution_id = %execution_id,
            "[opc_demand] 线索实现工作流已启动"
        );
        Ok(execution_id)
    }

    // ── #264 批 4：领域工作流只读三 op ────────────────────────────────────
    //
    // 迁移前是 `commands::opc_workflows::domain_workflows` 的三条只读命令（该文件已删除）。
    // 前缀筛选下沉 dao，领域/标签过滤与摘要映射仍在本层：出参逐字段保持迁移前语义。

    async fn list_domain_workflows(
        &self,
        domain_id: Option<String>,
        tag: Option<String>,
    ) -> Result<Vec<DomainWorkflowSummaryView>> {
        let all = axagent_dao::repo::workflow_template::list_domain_workflow_templates(&self.db)
            .await
            .map_err(|e| {
                AxAgentError::Provider(format!(
                    "opc.store::list_domain_workflows 查询领域工作流失败: {e}"
                ))
            })?;

        let summaries = all
            .iter()
            .filter(|wf| {
                if let Some(ref did) = domain_id {
                    if extract_domain_from_id(&wf.id) != did.as_str() {
                        return false;
                    }
                }
                if let Some(ref t) = tag {
                    if !parse_tags(&wf.tags).iter().any(|tag_item| tag_item == t) {
                        return false;
                    }
                }
                true
            })
            .map(|wf| DomainWorkflowSummaryView {
                id: wf.id.clone(),
                name: wf.name.clone(),
                description: wf.description.clone().unwrap_or_default(),
                icon: wf.icon.clone(),
                tags: parse_tags(&wf.tags),
                domain: extract_domain_from_id(&wf.id).to_string(),
                step_count: parse_nodes_len(&wf.nodes).unwrap_or(0),
            })
            .collect();

        Ok(summaries)
    }

    async fn get_domain_workflow(
        &self,
        workflow_id: String,
    ) -> Result<Option<WorkflowTemplateResponse>> {
        let model =
            axagent_dao::repo::workflow_template::get_workflow_template(&self.db, &workflow_id)
                .await
                .map_err(|e| {
                    AxAgentError::Provider(format!(
                        "opc.store::get_domain_workflow 查询工作流失败: {e}"
                    ))
                })?;
        model.map(|m| model_to_domain_response(m).map_err(AxAgentError::Provider)).transpose()
    }

    async fn list_domains(&self) -> Result<Vec<DomainEntryView>> {
        let all = axagent_dao::repo::workflow_template::list_domain_workflow_templates(&self.db)
            .await
            .map_err(|e| {
                AxAgentError::Provider(format!("opc.store::list_domains 查询领域失败: {e}"))
            })?;

        let mut domains: Vec<DomainEntryView> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for wf in &all {
            let domain_id = extract_domain_from_id(&wf.id).to_string();
            if seen.insert(domain_id.clone()) {
                domains
                    .push(DomainEntryView { name: domain_id_to_name(&domain_id), id: domain_id });
            }
        }

        domains.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(domains)
    }

    // ── #264 批 5：交付面六 op ────────────────────────────────────
    //
    // 迁移前是 `commands::demand_discovery` 的六条交付命令（该文件余发现面五条）。存储
    // 触点全在 dao（本文件无 `sea_orm` 导入）；dao 的错误已带分类（NotFound / Db）⇒
    // 与批 3 的 `update_lead_status` 同模式**原样上抛**，不在本层包 Provider 抹掉分类。
    // execute 的后台块保持迁移前的 `tauri::async_runtime::spawn`；后台回写失败从静默
    // `ok()` 改为 warn 日志（行为面不变，主流程不感知）。

    async fn execute_demand_workflow(
        &self,
        lead_id: String,
        workflow_template_id: Option<String>,
    ) -> Result<DeliveryRecordView> {
        let pipeline = self.demand_pipeline("execute_demand_workflow")?;
        let now = chrono::Utc::now().timestamp();

        let lead = opc_demand::get_lead(&self.db, &lead_id).await?;
        let template_id = workflow_template_id.unwrap_or_else(|| {
            lead.linked_workflow_id
                .clone()
                .unwrap_or_else(|| "default_demand_delivery".to_string())
        });

        let saved = opc_delivery::create_delivery(
            &self.db,
            &lead_id,
            &format!("交付: {}", lead.title),
            &template_id,
            lead.description.clone(),
            now,
        )
        .await?;

        // 真正调用工作流引擎下发（异步后台执行，不阻塞交付记录创建）—— 逐句同源迁移前命令。
        let engine = std::sync::Arc::clone(&pipeline.work_engine);
        let db = self.db.clone();
        let delivery_id = saved.id.clone();
        use axagent_harness::workflow_types::Variable;
        use axagent_rt_workflow::work_engine::RunOptions;

        let variables = vec![
            Variable {
                name: "lead_id".into(),
                var_type: "string".into(),
                value: serde_json::Value::String(lead_id.clone()),
                description: Some("需求线索 ID".into()),
                is_secret: false,
            },
            Variable {
                name: "delivery_id".into(),
                var_type: "string".into(),
                value: serde_json::Value::String(delivery_id.clone()),
                description: Some("交付记录 ID".into()),
                is_secret: false,
            },
            Variable {
                name: "demand_title".into(),
                var_type: "string".into(),
                value: serde_json::Value::String(saved.title.clone()),
                description: Some("需求标题".into()),
                is_secret: false,
            },
            Variable {
                name: "demand_description".into(),
                var_type: "string".into(),
                value: serde_json::Value::String(saved.description.clone()),
                description: Some("需求描述".into()),
                is_secret: false,
            },
        ];

        let opts = RunOptions {
            max_concurrent: 2,
            step_timeout: std::time::Duration::from_secs(300),
            tool_timeout: std::time::Duration::from_secs(60),
            variables: Some(variables),
            progress_callback: None,
            ..Default::default()
        };

        let wf_id = template_id;
        tauri::async_runtime::spawn(async move {
            let result = engine.run_workflow(&wf_id, opts).await;
            let task_now = chrono::Utc::now().timestamp();
            let outcome = match result {
                Ok(wf) => {
                    opc_delivery::mark_delivery_completed(
                        &db,
                        &delivery_id,
                        task_now,
                        &format!("工作流已完成，节点数: {}", wf.nodes.len()),
                    )
                    .await
                },
                Err(e) => {
                    opc_delivery::mark_delivery_failed(
                        &db,
                        &delivery_id,
                        task_now,
                        &serde_json::json!([{ "workflow": e.to_string() }]).to_string(),
                    )
                    .await
                },
            };
            if let Err(update_err) = outcome {
                tracing::warn!(
                    delivery_id = %delivery_id,
                    error = %update_err,
                    "[opc_delivery] 后台回写交付行失败"
                );
            }
        });

        Ok(saved)
    }

    async fn list_deliveries(
        &self,
        status: Option<String>,
        lead_id: Option<String>,
    ) -> Result<Vec<DeliveryRecordView>> {
        opc_delivery::list_deliveries(&self.db, status.as_deref(), lead_id.as_deref()).await
    }

    async fn get_delivery(&self, delivery_id: String) -> Result<DeliveryRecordView> {
        opc_delivery::get_delivery(&self.db, &delivery_id).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_delivery(
        &self,
        delivery_id: String,
        status: String,
        progress: Option<f64>,
        result_summary: Option<String>,
        deliverables: Option<serde_json::Value>,
        errors: Option<serde_json::Value>,
    ) -> Result<DeliveryRecordView> {
        let now = chrono::Utc::now().timestamp();
        opc_delivery::update_delivery(
            &self.db,
            &delivery_id,
            &status,
            progress,
            result_summary,
            deliverables,
            errors,
            now,
        )
        .await
    }

    async fn retry_delivery(&self, delivery_id: String) -> Result<DeliveryRecordView> {
        let now = chrono::Utc::now().timestamp();
        opc_delivery::reset_delivery(&self.db, &delivery_id, now).await
    }

    async fn cancel_delivery(&self, delivery_id: String) -> Result<DeliveryRecordView> {
        let now = chrono::Utc::now().timestamp();
        opc_delivery::cancel_delivery(&self.db, &delivery_id, now).await
    }

    // ── #264 批 6：能力发现面五 op（迁移前 `commands::demand_discovery` 发现段命令体）──

    /// 与迁移前 `opc_scan_capabilities` 命令逐句同源：遍历能力索引护照按载体分组
    /// 组装清单，系统专用护照（`is_system_only`）不进业务清单。
    async fn scan_capabilities(&self) -> Result<axagent_harness::capability::CapabilityInventory> {
        use axagent_company_runtime::opc::capability::CapabilitySource;
        use axagent_harness::CapabilityKind;

        let pipeline = self.demand_pipeline("scan_capabilities")?;
        let now = chrono::Utc::now().timestamp();

        let mut tools: Vec<_> = Vec::new();
        let mut skills: Vec<_> = Vec::new();
        let mut mcp_tools: Vec<_> = Vec::new();
        let mut workflows: Vec<_> = Vec::new();
        let mut agents: Vec<_> = Vec::new();

        // 从上游能力索引读取全部护照，按类型分组（来源与 kind 保持一致）
        let ids = pipeline.capability_indexer.list_capability_ids().await;
        for id in ids {
            if let Some(p) = pipeline.capability_indexer.get_passport(&id).await {
                if p.visibility.is_system_only() {
                    continue;
                }
                let source = match p.kind {
                    CapabilityKind::Skill => CapabilitySource::Skill,
                    CapabilityKind::Workflow => CapabilitySource::Workflow,
                    CapabilityKind::Tool if p.capability_id.starts_with("mcp:") => {
                        CapabilitySource::McpTool
                    },
                    CapabilityKind::Tool => CapabilitySource::Tool,
                    CapabilityKind::Agent => CapabilitySource::Agent,
                    _ => continue,
                };
                let entry = passport_to_entry(&p, source.clone());
                match source {
                    CapabilitySource::Tool => tools.push(entry),
                    CapabilitySource::Skill => skills.push(entry),
                    CapabilitySource::McpTool => mcp_tools.push(entry),
                    CapabilitySource::Workflow => workflows.push(entry),
                    CapabilitySource::Agent => agents.push(entry),
                }
            }
        }

        let mut inv = axagent_harness::capability::CapabilityInventory {
            tools,
            skills,
            mcp_tools,
            workflows,
            agents,
            scanned_at: now,
            total_count: 0,
        };
        inv.recalc_count();
        Ok(inv)
    }

    /// 与迁移前 `opc_proactive_evaluate_and_save_leads` + `run_proactive_scan_pipeline`
    /// 逐句同源：关键词提取已下沉 dao（`extract_demand_discovery_queries`），逐关键词
    /// 调扫描核心并聚合统计。单关键词失败不中断整轮（warn 后记入 query_stats.error）。
    async fn proactive_evaluate_leads(&self) -> Result<ProactiveScanSummary> {
        let queries = wf_template_repo::extract_demand_discovery_queries(&self.db).await?;

        let mut total_scanned = 0u32;
        let mut total_saved = 0u32;
        let mut high_value_count = 0u32;
        let mut query_stats = Vec::new();
        for query in &queries {
            match run_discovery_scan(&self.db, query, &[]).await {
                Ok(summary) => {
                    total_scanned += summary.total_scanned;
                    total_saved += summary.total_saved;
                    high_value_count += summary.high_value_count;
                    query_stats.push(QueryStat {
                        query: query.clone(),
                        scanned: summary.total_scanned,
                        saved: summary.total_saved,
                        refreshed: Some(summary.total_refreshed),
                        error: None,
                    });
                },
                Err(e) => {
                    tracing::warn!("[proactive_scan] 关键词 '{query}' 扫描失败: {e}");
                    query_stats.push(QueryStat {
                        query: query.clone(),
                        scanned: 0,
                        saved: 0,
                        refreshed: None,
                        error: Some(e.to_string()),
                    });
                },
            }
        }

        Ok(ProactiveScanSummary {
            total_queries: queries.len(),
            total_scanned,
            total_saved,
            high_value_count,
            query_stats,
        })
    }

    /// 与迁移前 `opc_list_capability_gaps` 命令逐句同源：状态过滤 + 创建时间降序。
    async fn list_capability_gaps(&self, status: Option<String>) -> Result<Vec<CapabilityGapView>> {
        opc_capability_gap::list_gaps(&self.db, status).await
    }

    /// 与迁移前 `opc_close_capability_gap` 命令逐句同源：缺行报 NotFound。
    async fn close_capability_gap(&self, gap_id: String) -> Result<CapabilityGapView> {
        let now = chrono::Utc::now().timestamp();
        opc_capability_gap::close_gap(&self.db, &gap_id, now).await
    }

    /// 与迁移前 `opc_analyze_capability_gaps` 命令逐句同源：统计未转化需求中未被
    /// 现有能力覆盖的高频关键词（频率 ≥2），Top 5 自动创建 `proactive` 缺口行。
    async fn analyze_capability_gaps(&self) -> Result<AnalyzeGapsSummary> {
        use std::collections::HashMap;

        let pipeline = self.demand_pipeline("analyze_capability_gaps")?;
        let now = chrono::Utc::now().timestamp();

        // 1) 扫描现有能力库（复用上游能力索引的能力护照）
        let mut capability_keywords: Vec<String> = Vec::new();
        for id in pipeline.capability_indexer.list_capability_ids().await {
            if let Some(p) = pipeline.capability_indexer.get_passport(&id).await {
                if p.visibility.is_system_only() {
                    continue;
                }
                capability_keywords.push(
                    format!("{} {} {}", p.name, p.description, p.kind.as_str()).to_lowercase(),
                );
            }
        }

        // 2) 统计高价值需求中的高频关键词（读 A 链路 opc_demand_leads；won/lost 为
        //    销售终态，不参与统计）
        let all_leads = opc_demand::list_leads(&self.db, 10_000, Some(50.0), None).await.map_err(
            |e| AxAgentError::Provider(format!("opc.store::analyze_capability_gaps {e}")),
        )?;
        let leads: Vec<_> =
            all_leads.iter().filter(|l| l.status != "won" && l.status != "lost").collect();

        let mut keyword_freq: HashMap<String, (usize, f64)> = HashMap::new(); // (出现次数, 累计评分)
        for lead in &leads {
            let text = format!("{} {}", lead.title, lead.description).to_lowercase();
            let score = lead.commercial_value_score;

            // 简单分词：按空格和常见标点
            for word in
                text.split(|c: char| c.is_whitespace() || "，。！？、；：\"'（）【】".contains(c))
            {
                let trimmed = word.trim();
                if trimmed.len() >= 2 && trimmed.len() <= 10 {
                    let entry = keyword_freq.entry(trimmed.to_string()).or_insert((0, 0.0));
                    entry.0 += 1;
                    entry.1 += score;
                }
            }
        }

        // 3) 识别未被现有能力覆盖的高频关键词
        let mut missing_keywords: Vec<(String, usize, f64)> = Vec::new();
        for (keyword, (freq, total_score)) in &keyword_freq {
            let covered = capability_keywords.iter().any(|ck| ck.contains(keyword));
            if !covered && *freq >= 2 {
                missing_keywords.push((keyword.clone(), *freq, *total_score));
            }
        }

        // 按频率排序（降序）
        missing_keywords.sort_by_key(|a| std::cmp::Reverse(a.1));

        // 4) 基于配置的领域关键词分析覆盖情况
        let domain_queries = wf_template_repo::extract_demand_discovery_queries(&self.db).await?;
        let mut domain_coverage = Vec::new();
        for domain_kw in &domain_queries {
            let domain_kw_lower = domain_kw.to_lowercase();
            let covered = capability_keywords.iter().any(|ck| ck.contains(&domain_kw_lower));
            let demand_count = leads
                .iter()
                .filter(|l| {
                    let text = format!("{} {}", l.title, l.description).to_lowercase();
                    text.contains(&domain_kw_lower)
                })
                .count();

            domain_coverage.push(DomainCoverage {
                domain: domain_kw.clone(),
                covered,
                demand_count,
            });
        }

        // 5) 自动创建高优先级缺口（Top 5 高频缺失；单条失败不中断整轮）
        let mut auto_created: Vec<String> = Vec::new();
        if missing_keywords.len() >= 2 {
            for (keyword, freq, _) in missing_keywords.iter().take(5) {
                let gap_id = format!("gap-auto-{}", uuid::Uuid::new_v4().simple());
                let priority = if *freq >= 5 {
                    1
                } else if *freq >= 3 {
                    2
                } else {
                    3
                };

                let view = CapabilityGapView {
                    id: gap_id,
                    lead_id: None,
                    title: format!("[主动分析] 高频缺失能力: {keyword}"),
                    description: format!(
                        "关键词 '{keyword}' 在 {freq} 条高价值需求中出现，但现有能力库未覆盖。建议新增对应能力。"
                    ),
                    missing_capability: keyword.clone(),
                    gap_type: "proactive".to_string(),
                    suggested_action: format!(
                        "针对 '{keyword}' 领域新增工具/技能/工作流模板，或扫描市场平台获取该领域需求详情"
                    ),
                    priority,
                    status: "open".to_string(),
                    created_at: now,
                    updated_at: now,
                    closed_at: None,
                };
                match opc_capability_gap::insert_gap(&self.db, &view).await {
                    Ok(()) => auto_created.push(keyword.clone()),
                    Err(e) => {
                        tracing::warn!("[analyze_capability_gaps] 自动建缺口 '{keyword}' 失败: {e}")
                    },
                }
            }
        }

        // 6) 返回分析结果
        Ok(AnalyzeGapsSummary {
            total_leads_analyzed: leads.len(),
            total_capabilities: capability_keywords.len(),
            missing_keywords_count: missing_keywords.len(),
            top_missing_keywords: missing_keywords
                .iter()
                .take(10)
                .map(|(k, f, s)| TopMissingKeyword {
                    keyword: k.clone(),
                    frequency: *f,
                    total_score: *s,
                })
                .collect(),
            domain_coverage,
            auto_created_gaps: auto_created,
        })
    }
}

// ── #264 批 4 领域工作流三 op 的依赖（同样放在 impl 之后，理由同上）──────────
use axagent_harness::opc_store::{DeliveryRecordView, DomainEntryView, DomainWorkflowSummaryView};
use axagent_harness::workflow_types::WorkflowTemplateResponse;

/// 从工作流 ID 提取领域标识：`wf-acd-xxx` → `acd`（无法判断回 `unknown`）。
fn extract_domain_from_id(id: &str) -> &str {
    if let Some(rest) = id.strip_prefix("wf-") {
        if let Some(domain) = rest.split('-').next() {
            return domain;
        }
    }
    "unknown"
}

/// 领域 ID → 中文名（迁移前命令的固定映射表，逐字搬移）。
fn domain_id_to_name(id: &str) -> String {
    match id {
        "academic" => "学术研究".to_string(),
        "design" => "设计与创意".to_string(),
        "engineering" => "工程与开发".to_string(),
        "finance" => "财务与会计".to_string(),
        "gamedev" => "游戏开发".to_string(),
        "gis" => "地理信息系统".to_string(),
        "marketing" => "市场营销".to_string(),
        "paidmedia" => "付费媒体".to_string(),
        "pm" => "项目管理".to_string(),
        "product" => "产品管理".to_string(),
        "sales" => "销售与商务".to_string(),
        "security" => "安全与合规".to_string(),
        "spatial" => "空间计算".to_string(),
        "specialized" => "专业服务".to_string(),
        "strategy" => "战略规划".to_string(),
        "support" => "客户支持".to_string(),
        "testing" => "测试与质量".to_string(),
        _ => id.to_string(),
    }
}

/// 解析 `tags` JSON 列（解码失败 = 空数组，与迁移前同一降级）。
fn parse_tags(tags: &Option<String>) -> Vec<String> {
    tags.as_ref().and_then(|s| serde_json::from_str::<Vec<String>>(s).ok()).unwrap_or_default()
}

/// 解析 `nodes` JSON 列并取节点数（解码失败回 `None`，调用方降级为 0）。
fn parse_nodes_len(nodes_json: &str) -> Option<usize> {
    serde_json::from_str::<Vec<serde_json::Value>>(nodes_json).ok().map(|v| v.len())
}

/// DB Model → `WorkflowTemplateResponse`（迁移前命令的映射体，逐字搬移）。
///
/// 不复用 dao 的 `workflow_template_response_from_model`：那份把 `tool_defs` 固定为 `None`、
/// 且 nodes/edges 解析失败时降级为空数组；迁移前命令要保留 `tool_defs` 且解析失败即错，
/// 语义不同，合并会静默改变出参。
fn model_to_domain_response(
    m: axagent_entities::workflow_template::Model,
) -> std::result::Result<WorkflowTemplateResponse, String> {
    let nodes: Vec<axagent_harness::workflow_types::WorkflowNode> =
        serde_json::from_str(&m.nodes).map_err(|e| format!("解析 nodes: {e}"))?;
    let edges: Vec<axagent_harness::workflow_types::WorkflowEdge> =
        serde_json::from_str(&m.edges).map_err(|e| format!("解析 edges: {e}"))?;
    let tags: Vec<String> =
        m.tags.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default();
    let is_system = m.is_preset && tags.iter().any(|t| t == "cognitive_router");
    let variables: Vec<axagent_harness::workflow_types::Variable> =
        m.variables.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default();
    let tool_defs: Option<Vec<axagent_harness::workflow_types::RhaiToolDef>> =
        m.tool_defs.as_deref().and_then(|s| serde_json::from_str(s).ok());
    let trigger_config: Option<axagent_harness::workflow_types::TriggerConfig> =
        m.trigger_config.as_deref().and_then(|s| serde_json::from_str(s).ok());
    let input_schema: Option<axagent_harness::workflow_types::JsonSchema> =
        m.input_schema.as_deref().and_then(|s| serde_json::from_str(s).ok());
    let output_schema: Option<axagent_harness::workflow_types::JsonSchema> =
        m.output_schema.as_deref().and_then(|s| serde_json::from_str(s).ok());
    let error_config: Option<axagent_harness::workflow_types::ErrorConfig> =
        m.error_config.as_deref().and_then(|s| serde_json::from_str(s).ok());

    Ok(WorkflowTemplateResponse {
        id: m.id,
        name: m.name,
        description: m.description,
        icon: m.icon,
        cluster_id: m.cluster_id,
        route_path: m.route_path,
        tags,
        version: m.version,
        is_preset: m.is_preset,
        is_editable: m.is_editable,
        is_public: m.is_public,
        is_system,
        trigger_config,
        nodes,
        edges,
        input_schema,
        output_schema,
        variables,
        error_config,
        tool_defs,
        mission_hash: m.mission_hash,
        created_at: m.created_at,
        updated_at: m.updated_at,
    })
}

/// 手动补录平台的固定 platform 标识（与扫描器内置平台标识不冲突）
const MANUAL_PLATFORM: &str = "manual";

// ── #264 批 3 需求实现三 op 的依赖（同样放在 impl 之后，理由同上）──────────
use axagent_dao::repo::workflow_template::{
    build_active_model_from_data, insert_workflow_template,
};
use axagent_harness::workflow_types::WorkflowTemplateData;

/// 模板描述/标题的字节截断上限（防线索标题超长撑爆列表 UI；迁移前命令同值）。
const MAX_TEXT_BYTES: usize = 120;

/// 从线索生成最小可用的实现工作流模板（Trigger + LLM + End）—— 迁移前命令体逐字搬移。
fn generate_lead_workflow_template(lead: &DemandLeadDto) -> WorkflowTemplateData {
    use axagent_harness::capability::Visibility;
    use axagent_harness::workflow_types::{
        EdgeType, EndNode, EndNodeConfig, LLMNode, LLMNodeConfig, Position, RetryConfig,
        TriggerConfig, TriggerNode, TriggerType, WorkflowEdge, WorkflowNode, WorkflowNodeBase,
    };
    use chrono::Utc;

    let now = Utc::now().timestamp_millis();
    let brief = build_lead_brief(lead);
    let title = axagent_harness::util_fns::truncate_to_char_boundary(&lead.title, MAX_TEXT_BYTES);

    let trigger_base = WorkflowNodeBase {
        id: "trigger".to_string(),
        title: "开始".to_string(),
        description: Some("用户触发".to_string()),
        position: Position { x: 100.0, y: 80.0 },
        retry: RetryConfig::default(),
        timeout: None,
        enabled: true,
        parent_id: None,
        compensation: None,
        continue_on_fail: false,
    };

    let llm_base = WorkflowNodeBase {
        id: "llm_implement".to_string(),
        title: format!("需求实现: {title}"),
        description: Some(
            axagent_harness::util_fns::truncate_to_char_boundary(&lead.description, MAX_TEXT_BYTES)
                .to_string(),
        ),
        position: Position { x: 400.0, y: 80.0 },
        retry: RetryConfig::default(),
        timeout: None,
        enabled: true,
        parent_id: None,
        compensation: None,
        continue_on_fail: false,
    };

    let end_base = WorkflowNodeBase {
        id: "end".to_string(),
        title: "结束".to_string(),
        description: Some("输出实施方案".to_string()),
        position: Position { x: 700.0, y: 80.0 },
        retry: RetryConfig::default(),
        timeout: None,
        enabled: true,
        parent_id: None,
        compensation: None,
        continue_on_fail: false,
    };

    let llm_config = LLMNodeConfig {
        model: String::new(),
        prompt: format!(
            "你是 OPC（一人公司）的需求实现助手。请针对以下需求线索产出实施方案，\
             包含：①需求理解与范围边界 ②实施方案（分步骤）③交付物清单 \
             ④工作量与报价建议 ⑤风险与前提。只使用需求简报中的事实，不得编造。\n\n{brief}"
        ),
        messages: None,
        temperature: Some(0.7),
        max_tokens: Some(2048),
        tools: None,
        functions: None,
        consistency_check: None,
        max_context_tokens: None,
        reserved_output_tokens: None,
    };

    WorkflowTemplateData {
        hooks_config: None,
        id: format!("demand:lead:{}", lead.id),
        name: format!("需求实现: {title}"),
        description: Some(
            axagent_harness::util_fns::truncate_to_char_boundary(&lead.description, MAX_TEXT_BYTES)
                .to_string(),
        ),
        icon: "target".to_string(),
        tags: vec!["opc_demand".to_string(), "demand_implement".to_string()],
        version: 1,
        is_preset: false,
        is_editable: true,
        is_public: false,
        visibility: Visibility::Public,
        trigger_config: Some(TriggerConfig {
            trigger_type: TriggerType::Manual,
            config: serde_json::json!({}),
        }),
        nodes: vec![
            WorkflowNode::Trigger(TriggerNode {
                base: trigger_base,
                config: TriggerConfig {
                    trigger_type: TriggerType::Manual,
                    config: serde_json::json!({}),
                },
            }),
            WorkflowNode::Llm(LLMNode { base: llm_base, config: llm_config }),
            WorkflowNode::End(EndNode {
                base: end_base,
                config: EndNodeConfig { output_var: None },
            }),
        ],
        edges: vec![
            WorkflowEdge {
                id: "e1".to_string(),
                source: "trigger".to_string(),
                source_handle: None,
                target: "llm_implement".to_string(),
                target_handle: None,
                edge_type: EdgeType::Direct,
                label: None,
            },
            WorkflowEdge {
                id: "e2".to_string(),
                source: "llm_implement".to_string(),
                source_handle: None,
                target: "end".to_string(),
                target_handle: None,
                edge_type: EdgeType::Direct,
                label: None,
            },
        ],
        input_schema: None,
        output_schema: None,
        variables: vec![],
        error_config: None,
        error_workflow_id: None,
        tool_defs: vec![],
        mission_hash: None,
        cluster_id: Some("demand_implement".to_string()),
        route_path: Some(format!("/automation/demand/{}", lead.id)),
        created_at: now,
        updated_at: now,
    }
}

/// 构建需求简报（转化 prompt 与执行输入共用，保证单一事实源）—— 迁移前命令体逐字搬移。
fn build_lead_brief(lead: &DemandLeadDto) -> String {
    let budget = match (lead.budget_min, lead.budget_max) {
        (Some(min), Some(max)) => {
            format!("{} {} – {} {}", min, lead.budget_currency, max, lead.budget_currency)
        },
        (Some(v), None) | (None, Some(v)) => format!("{v} {}", lead.budget_currency),
        _ => "未提供".to_string(),
    };
    format!(
        "【需求简报】\n标题: {}\n描述: {}\n来源平台: {}\n需求类型: {}\n预算: {}\n商业价值分: {:.0}（{}）\n原文链接: {}\n线索 ID: {}",
        lead.title,
        if lead.description.is_empty() {
            "（无描述）"
        } else {
            &lead.description
        },
        lead.platform,
        lead.demand_type,
        budget,
        lead.commercial_value_score,
        lead.opportunity_level,
        lead.source_url.as_deref().unwrap_or("（无）"),
        lead.id,
    )
}

// ── #264 批 2 需求发现八 op 的依赖 ────────────────────────────────────────
//
// 同下面两段：放在 `impl` 之后以保持 `impl OpcStoreProvider for HostOpcStore` 行号不动
// （该行被登记表钉住）。模块级 `use` 在任意 item 位置都生效。
use axagent_harness::types::opc_demand::{
    DemandLeadDto, DemandPlatform, DiscoverLeadsSummary, SCAN_POLICY_SETTING_KEY,
    SaveDemandLeadInput, SaveDemandPlatformInput, ScanPolicy,
};
use axagent_tools::tools::marketplace_scanner::{
    DemandLead, EvaluatedDemandLead, RawLead, evaluate_lead,
};
use axagent_tools::tools::opc_demand_scan;

// ── #36 批 2 写 op 的依赖 ─────────────────────────────────────────────────
//
// 这两个 `use` **刻意放在 impl 之后**：本文件 `impl OpcStoreProvider for HostOpcStore` 那一行
// 被登记表按行号钉住（见 `scripts/capability-register.json` 的 impl 溯源段），往上方的
// `use axagent_harness::opc_store::{…}` 列表里加名字会让它换行、impl 行整体下移。
// 模块级 `use` 在任意 item 位置都生效，故这里只是位置选择，不是作用域限制。
use axagent_harness::opc_store::{
    AddMilestoneRequest, CreateContentAssetRequest, CreateCustomerRequest, CreateInvoiceRequest,
    CreateLandingPageRequest, CreateProjectRequest, CreatePublishScheduleRequest,
    FinancialReportView, MilestoneView, ProjectDetailView, RemoveContentAssetOutcome,
    RemovePublishScheduleOutcome, TransitionInvoiceOutcome, UpdateContentAssetInput,
    UpdateContentAssetOutcome, UpdateCustomerRequest, UpdateProjectRequest,
};
// #36 批 3 的域入参 / 枚举 / 服务：同样放在 impl 之后（理由同上 —— impl 行被登记表钉住）。
use axagent_company_runtime::opc::{
    CreateContentAssetInput, CreateCustomerInput, CreateInvoiceInput, CreateLandingPageInput,
    CreateProjectInput, CreatePublishScheduleInput, CustomerSource, DefaultFinanceService,
    FinanceService, InvoiceLineItem, Milestone, UpdateCustomerInput, UpdateProjectInput,
};
use std::str::FromStr;

/// 写 op 的 `id` 闸：空 / 全空白 ⇒ `Err(Validation)`（`op` 只用于文案定位）。
///
/// 服务全部 id 形态的写 op：blog 三件 + #36 批 2 的四条（发票 / 内容资产 ×2 / 发布计划）。
///
/// 为什么在 provider 而不是分派层：读 op 的两个可选参数闸也在 provider（分派层
/// 「只透传契约 DTO、一个数字都不写」的分工）。两处各判一半 ⇒ 迟早有一处漏。
fn require_id(id: &str, op: &str) -> Result<()> {
    if id.trim().is_empty() {
        return Err(AxAgentError::Validation(format!(
            "opc.store::{op} 的 id 不能为空（缺 id 属入参非法，不是「这一行不存在」）"
        )));
    }
    Ok(())
}

/// 正文预算闸：超 [`OPC_BLOG_POST_CONTENT_MAX_BYTES`] ⇒ `Err(Validation)`，**不截断**。
///
/// 按 **UTF-8 字节**计（与常量的口径逐字一致）：中文正文的字符数远小于字节数，
/// 按字符判会放行三倍于预算的正文（判据抄 `knowledge_vector_service.rs` 的
/// `check_content_budget`，那是同一类闸的先例）。
fn check_blog_post_content_budget(content: &str) -> Result<()> {
    let bytes = content.len();
    if bytes > OPC_BLOG_POST_CONTENT_MAX_BYTES {
        return Err(AxAgentError::Validation(format!(
            "opc.store::create_blog_post 的 content 是 {bytes} 字节，越出契约上限 \
             {OPC_BLOG_POST_CONTENT_MAX_BYTES} 字节（超限拒收不截断：静默截断会让调用方以为整篇都进库了）"
        )));
    }
    Ok(())
}

/// `slug` 闸：空 / 全空白 ⇒ `Err(Validation)`。
///
/// `slug` 是**对外路由键**（表上 `unique`）：空白 slug 会落一行对外不可达、且第二行必然撞
/// 唯一约束的内容。落库前先拒，比让调用方去解一条 SQLite 唯一约束报错更可归因。
fn check_blog_post_slug(slug: &str) -> Result<()> {
    if slug.trim().is_empty() {
        return Err(AxAgentError::Validation(
            "opc.store::create_blog_post 的 slug 不能为空（它是对外路由键，表上 unique）".into(),
        ));
    }
    Ok(())
}

// ── 测试 ──────────────────────────────────────────────────────────────────
//
// 内联测试 mod 追加在文件末尾（`items_after_test_module` 只在 clippy 下暴露）。
//
// 纯函数面：`limit` 闸、`id` / 正文预算 / `slug` 闸、映射键集合。
// **删除三态的真库端到端**（`Removed` / `NotFound` / `Protected`，以及「受保护的行一行不动」）
// 在本层开一次真库 —— 那三档是 #180-①② 的核心判据，只验枚举串证不了「先判保护、再删」的落点；
// 发布写面的命中 / 缺行 / 秒级量级仍归 `repo/opc_blog_post.rs`。

// ── 只读视图映射（唯一裁剪点）──────────────────────────────────────────────
//
// 八个函数一一对应八个 `*SummaryView`：字段改名与裁剪只在这里发生，和 `blog_post_view`
// 同一条分工。**不裁剪的字段**（`content` / `body` / `message` / `lineItems`）在此处
// 不出现 —— 它们要么列表工具从不展示，要么会让一帧体积与整表行数相乘。

fn invoice_summary_view(inv: Invoice) -> InvoiceSummaryView {
    InvoiceSummaryView {
        id: inv.id,
        customer_id: inv.customer_id,
        invoice_number: inv.invoice_number,
        status: inv.status.as_str().to_string(),
        total: inv.total,
        currency: inv.currency,
        due_at: inv.due_at,
        created_at: inv.created_at,
    }
}

fn customer_summary_view(c: Customer) -> CustomerSummaryView {
    CustomerSummaryView {
        id: c.id,
        name: c.name,
        email: c.email,
        phone: c.phone,
        company: c.company,
        customer_type: c.customer_type.as_str().to_string(),
        country: c.country,
        region: c.region,
        city: c.city,
        address: c.address,
        latitude: c.latitude,
        longitude: c.longitude,
        source: c.source.map(|s| opc_source_to_str(&s)),
        notes: c.notes,
        status: c.status.as_str().to_string(),
        total_revenue: c.total_revenue,
        invoice_count: c.invoice_count,
        created_at: c.created_at,
    }
}

/// `CustomerSource` → 线格式串（小蛇形；`Other` 原串透传）。域包 `source_to_str` 是
/// 私有 fn（`customer.rs` 内部用），这里按同一判据本地写一份 —— 两处形状由前端筛选器
/// 的字面值（`referral` / `website` / …）钉住。
fn opc_source_to_str(s: &CustomerSource) -> String {
    match s {
        CustomerSource::Referral => "referral".into(),
        CustomerSource::Website => "website".into(),
        CustomerSource::SocialMedia => "social_media".into(),
        CustomerSource::Marketplace => "marketplace".into(),
        CustomerSource::Direct => "direct".into(),
        CustomerSource::Other(v) => v.clone(),
    }
}

/// 里程碑只带两个计数（完成数是现算的，判据同迁移前 `OpcListProjects`：
/// `m.status == MilestoneStatus::Completed`）。
///
/// 入参取引用：详情视图要在同一份读数上算摘要 + 明细（`project_detail_view`），
/// 按值收会让那个映射函数被迫把九个字段再抄一遍（禁区 12）。
fn project_summary_view(p: &Project) -> ProjectSummaryView {
    let milestone_total = p.milestones.len();
    let milestone_completed =
        p.milestones.iter().filter(|m| m.status == MilestoneStatus::Completed).count();
    ProjectSummaryView {
        id: p.id.clone(),
        customer_id: p.customer_id.clone(),
        title: p.title.clone(),
        status: p.status.as_str().to_string(),
        budget: p.budget,
        currency: p.currency.clone(),
        deadline: p.deadline,
        milestone_total,
        milestone_completed,
        created_at: p.created_at,
    }
}

/// 里程碑 → 详情行（状态落枚举串，与库里的落库串同形）。
///
/// 域包 `MilestoneStatus` 只有 `Serialize`（`snake_case`），本函数据同一判据本地映射 ——
/// 不让「视图串」和「落库串」各漂一套（前端按 `completed` 判可点 / 已完成）。
fn project_milestone_view(m: Milestone) -> MilestoneView {
    let status = match m.status {
        MilestoneStatus::Pending => "pending",
        MilestoneStatus::InProgress => "in_progress",
        MilestoneStatus::Completed => "completed",
        MilestoneStatus::Skipped => "skipped",
    };
    MilestoneView {
        id: m.id,
        title: m.title,
        description: m.description,
        due_at: m.due_at,
        completed_at: m.completed_at,
        status: status.to_string(),
    }
}

/// 项目详情视图：摘要（复用 [`project_summary_view`]，计数同一口径）+ 详情三列。
fn project_detail_view(p: Project) -> ProjectDetailView {
    let summary = project_summary_view(&p);
    let milestones = p.milestones.into_iter().map(project_milestone_view).collect();
    ProjectDetailView { summary, description: p.description, notes: p.notes, milestones }
}

fn landing_page_summary_view(p: LandingPage) -> LandingPageSummaryView {
    LandingPageSummaryView {
        id: p.id,
        title: p.title,
        slug: p.slug,
        published: p.published,
        created_at: p.created_at,
    }
}

fn contact_summary_view(c: ContactSubmission) -> ContactSummaryView {
    ContactSummaryView {
        id: c.id,
        name: c.name,
        email: c.email,
        source: c.source,
        read: c.read,
        created_at: c.created_at,
    }
}

fn kpi_summary_view(k: KpiRecord) -> KpiSummaryView {
    KpiSummaryView {
        id: k.id,
        domain_pack_id: k.domain_pack_id,
        name: k.name,
        value: k.value,
        unit: k.unit,
        period: k.period,
        recorded_at: k.recorded_at,
    }
}

fn revenue_record_view(r: RevenueRecord) -> RevenueRecordView {
    RevenueRecordView {
        id: r.id,
        amount: r.amount,
        currency: r.currency,
        category: r.category,
        description: r.description,
        recorded_at: r.recorded_at,
    }
}

fn content_asset_summary_view(a: ContentAsset) -> ContentAssetSummaryView {
    ContentAssetSummaryView {
        id: a.id,
        title: a.title,
        content_type: a.content_type,
        status: a.status,
        tags: a.tags,
        created_at: a.created_at,
    }
}

fn publish_schedule_summary_view(s: PublishSchedule) -> PublishScheduleSummaryView {
    PublishScheduleSummaryView {
        id: s.id,
        content_ref_type: s.content_ref_type,
        content_ref_id: s.content_ref_id,
        status: s.status,
        scheduled_at: s.scheduled_at,
        published_at: s.published_at,
    }
}

// ── #264 批 6：能力发现面五 op 的依赖 ─────────────────────────────────────
//
// 同前几批的理由：`impl OpcStoreProvider for HostOpcStore` 行被登记表钉住，use 列表
// 不再往上加名字；本块依赖集中在 impl 之后声明。`extract_domain_queries` / 缺口
// CRUD 已下沉 dao（`repo::workflow_template` / `repo::opc_capability_gap`），扫描核心
// 复用 `axagent_tools::opc_demand_scan`（与订阅定时扫描同源），能力护照复用
// `demand_pipeline.capability_indexer`（缺装配回 Provider 错，fail-closed）。
use axagent_dao::repo::{opc_capability_gap, workflow_template as wf_template_repo};
use axagent_harness::opc_store::{
    AnalyzeGapsSummary, CapabilityGapView, DomainCoverage, ProactiveScanSummary, QueryStat,
    TopMissingKeyword,
};
use axagent_tools::tools::opc_demand_scan::run_discovery_scan;

/// 能力护照 → 清单条目的载体分组（与迁移前 `opc_scan_capabilities` 命令体逐句同源）。
fn passport_to_entry(
    p: &axagent_harness::CapabilityPassportDto,
    source: axagent_company_runtime::opc::capability::CapabilitySource,
) -> axagent_harness::capability::CapabilityEntry {
    use axagent_harness::capability::CapabilityEntry;
    CapabilityEntry {
        id: p.capability_id.clone(),
        name: p.name.clone(),
        description: p.description.clone(),
        source,
        source_id: p.capability_id.clone(),
        capability_type: p.kind.as_str().to_string(),
        applicable_scenarios: Vec::new(),
        example_deliverables: Vec::new(),
        metadata: serde_json::json!({
            "enabled": p.enabled,
            "domain": p.domain.as_str(),
            "sub_category": p.sub_category,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn sample_row() -> BlogPostListRow {
        BlogPostListRow {
            id: "post-1".into(),
            title: "第一篇".into(),
            slug: "first-post".into(),
            excerpt: "摘要".into(),
            tags: vec!["opc".into(), "blog".into()],
            published: true,
            published_at: Some(1_700_000_000_000),
            view_count: 42,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_060_000,
        }
    }

    #[test]
    fn limit_gate_pins_the_contract_range() {
        let (low, high) = OPC_BLOG_POST_LIST_LIMIT_RANGE;
        assert_eq!(resolve_limit(Some(low)).unwrap(), low);
        assert_eq!(resolve_limit(Some(high)).unwrap(), high);
        for bad in [0, high + 1, usize::MAX] {
            let err = resolve_limit(Some(bad)).expect_err("越界应拒收，不该夹取");
            assert!(matches!(err, AxAgentError::Validation(_)), "越界应落成 Validation：{err:?}");
        }
        // 缺省取契约层那一份（不是本文件另抄的数字）。
        assert_eq!(resolve_limit(None).unwrap(), OPC_BLOG_POST_LIST_DEFAULT_LIMIT);
    }

    /// 键集合逐字钉住：多一个键 = 有大字段漏过线；少一个键 = 消费方（`SitesTab` 的列表列）
    /// 静默少一列。正文列 `content` 在 dao 的 SELECT 里就没有，这里再锁一次出参形态。
    #[test]
    fn blog_post_view_key_set_is_exactly_ten_camel_case() {
        let val: Value = serde_json::to_value(blog_post_view(sample_row())).expect("序列化应成功");
        let mut keys: Vec<String> = val.as_object().expect("应为对象").keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "createdAt",
                "excerpt",
                "id",
                "published",
                "publishedAt",
                "slug",
                "tags",
                "title",
                "updatedAt",
                "viewCount",
            ]
        );
        assert!(val.get("content").is_none(), "博客正文不过线");
    }

    #[test]
    fn blog_post_view_keeps_unknown_as_null() {
        let mut row = sample_row();
        row.published_at = None;
        row.tags.clear();
        let val = serde_json::to_value(blog_post_view(row)).expect("序列化应成功");
        assert_eq!(val["publishedAt"], json!(null), "未发布不得回退成 createdAt");
        assert_eq!(val["tags"], json!([]), "无标签 = 空数组，不是缺失");
    }

    /// 写 op 的 `id` 闸：空串与**纯空白**都拒（空白 id 走到 dao 是一次必然缺行的主键查询，
    /// 落成 `NotFound` 读数 —— 那是把参数错误伪装成业务结果）。合法值一律放行。
    ///
    /// 三个写 op（publish / delete）共用同一条闸，`op` 只进文案 ⇒ 两边不会各判一套。
    #[test]
    fn blog_post_id_gate_rejects_blank_but_does_not_trim() {
        for bad in ["", " ", "  ", "\t", "\n", "\u{00a0}"] {
            for op in ["publish_blog_post", "delete_blog_post"] {
                let err = require_id(bad, op).expect_err("空白 id 必须拒收");
                assert!(
                    matches!(err, AxAgentError::Validation(_)),
                    "应落 Validation（op={op}）：{err:?}"
                );
                if let AxAgentError::Validation(message) = err {
                    assert!(message.contains(op), "文案须点明是哪个 op：{message}");
                }
            }
        }
        // 只判「有没有内容」，不替调用方 trim 出第二个值：带前后空格的 id 原样交给 dao。
        assert!(require_id(" post-1 ", "delete_blog_post").is_ok(), "非空白即放行，本层不改写 id");
        assert!(require_id("post-1", "delete_blog_post").is_ok());
    }

    /// 正文预算闸的**边界**：`±1` 字节逐档验，且按字节而不是字符计。
    ///
    /// 三档对照（这也是本闸唯一的「不许放宽」面）：
    /// ① 恰好等于预算 ⇒ 放行（边界含等号，与 `>` 判据一致）；
    /// ② 预算 + 1 字节 ⇒ 拒（差一个字节也必须拒，否则预算就不是预算）；
    /// ③ 中文：字符数只有字节数的 1/3 ⇒ 「按字符判」会放行的串，本闸必须拒。
    #[test]
    fn content_budget_gate_is_exact_and_counts_bytes_not_chars() {
        let budget = OPC_BLOG_POST_CONTENT_MAX_BYTES;
        assert!(check_blog_post_content_budget(&"a".repeat(budget)).is_ok(), "恰好等于预算应放行");
        let err =
            check_blog_post_content_budget(&"a".repeat(budget + 1)).expect_err("超 1 字节必须拒收");
        assert!(matches!(err, AxAgentError::Validation(_)), "超限应落 Validation：{err:?}");
        if let AxAgentError::Validation(message) = err {
            assert!(message.contains(&(budget + 1).to_string()), "文案须给出实际字节数：{message}");
            assert!(message.contains(&budget.to_string()), "文案须给出契约上限：{message}");
        }

        // 中文按字节算：`budget` 个中文字 = 3 × budget 字节 ⇒ 字符数没超、字节数超了三倍。
        let cjk = "文".repeat(budget);
        assert_eq!(cjk.chars().count(), budget, "字符数恰好等于预算（按字符判会放行）");
        assert_eq!(cjk.len(), budget * 3, "字节数是三倍");
        assert!(
            matches!(check_blog_post_content_budget(&cjk), Err(AxAgentError::Validation(_))),
            "中文必须按字节判：字符数不超也算超限"
        );

        // 空正文是合法值（草稿可以先落行、正文后补）—— 闸只管上界。
        assert!(check_blog_post_content_budget("").is_ok());
    }

    /// `slug` 闸：空白拒、非空白放行（含前后空格：本层不改写，归一化在实现层做）。
    #[test]
    fn slug_gate_rejects_blank() {
        for bad in ["", " ", "\t", "\u{00a0}"] {
            assert!(
                matches!(check_blog_post_slug(bad), Err(AxAgentError::Validation(_))),
                "空白 slug 必须拒收：{bad:?}"
            );
        }
        assert!(check_blog_post_slug("first-post").is_ok());
        assert!(check_blog_post_slug(" 带空格 ").is_ok(), "归一化不属本层职责");
    }

    /// `delete_blog_post` 三态的真库端到端：`Removed` / `NotFound` / `Protected`。
    ///
    /// 三档缺一不可：
    /// ① `Removed` —— 真建一行再删，删后读不到（证明删的是真行，不是静默 no-op）；
    /// ② `NotFound` —— 同一个 id 再删一次（service 按 `rows_affected == 0` 判，不是假成功）；
    /// ③ `Protected` —— 名单里的 id **一行都不动**（行仍在，且结果不是 `NotFound`：
    ///    顺序颠倒会把「不许删」报成「没有这篇」）。
    #[tokio::test]
    async fn delete_reports_three_states_against_real_db() {
        let handle = axagent_dao::db::create_test_pool().await.expect("测试：创建连接池应成功");
        let store = HostOpcStore::new(handle.conn.clone());
        store.protect_ids(["keep-me".to_string()]).await;

        // 建一行（顺带取证 create op 的出参是列裁剪视图）。
        let created = store
            .create_blog_post(CreateBlogPostInput {
                title: "三态".into(),
                slug: "post-delete-three-states".into(),
                excerpt: "摘要".into(),
                content: "正文".into(),
                tags: vec!["opc".into()],
            })
            .await
            .expect("创建应成功");
        assert!(!created.id.is_empty(), "创建的出参必须带后端生成的 id");
        assert_eq!(created.slug, "post-delete-three-states");

        // ① 命中 ⇒ Removed，且删后真取不到。
        assert_eq!(
            store.delete_blog_post(created.id.clone()).await.expect("删除应成功"),
            RemoveBlogPostOutcome::Removed
        );
        let svc = DefaultSiteService::new(handle.conn.clone());
        assert!(
            SiteService::get_blog_post(&svc, &created.id).await.is_err(),
            "删后必须取不到（证明删的是真行）"
        );

        // ② 再删一次 ⇒ NotFound（不是错误，也不许又回 Removed）。
        assert_eq!(
            store.delete_blog_post(created.id).await.expect("缺行不是错误"),
            RemoveBlogPostOutcome::NotFound
        );

        // ③ 受保护 ⇒ Protected，且行仍在。
        let kept = store
            .create_blog_post(CreateBlogPostInput {
                title: "受保护".into(),
                slug: "post-protected".into(),
                excerpt: String::new(),
                content: String::new(),
                tags: Vec::new(),
            })
            .await
            .expect("创建应成功");
        store.protect_ids([kept.id.clone()]).await;
        assert_eq!(
            store.delete_blog_post(kept.id.clone()).await.expect("受保护不是错误"),
            RemoveBlogPostOutcome::Protected
        );
        assert!(
            SiteService::get_blog_post(&svc, &kept.id).await.is_ok(),
            "受保护的行必须一行都没动（先判保护、再删）"
        );
    }

    fn keys_of(value: &Value) -> Vec<String> {
        let mut keys: Vec<String> =
            value.as_object().expect("视图必须序列化成对象").keys().cloned().collect();
        keys.sort();
        keys
    }

    fn assert_keys(value: &Value, expected: &[&str]) {
        let mut want: Vec<String> = expected.iter().map(|s| (*s).to_string()).collect();
        want.sort();
        assert_eq!(keys_of(value), want, "键集合漂移：{value:?}");
    }

    /// 八个只读视图的**键集合**与**裁剪**：camelCase 逐字钉死，大字段一个都不许出现。
    ///
    /// 键集合是接缝线格式，插件侧逐字透传（禁区 13）⇒ 这里多一个键就是线格式扩容，
    /// 少一个键就是前端字段静默变 `undefined`。
    #[test]
    fn readonly_views_are_cropped_and_camel_case() {
        use axagent_company_runtime::opc::Milestone;

        let invoice = serde_json::to_value(invoice_summary_view(Invoice {
            id: "inv-1".into(),
            customer_id: "cus-1".into(),
            invoice_number: "INV-001".into(),
            status: InvoiceStatus::Paid,
            line_items: Vec::new(),
            subtotal: 100.0,
            tax_total: 6.0,
            total: 106.0,
            currency: "CNY".into(),
            issued_at: None,
            due_at: None,
            paid_at: None,
            notes: String::new(),
            created_at: 1,
            updated_at: 2,
        }))
        .expect("发票视图可序列化");
        assert_keys(
            &invoice,
            &["id", "customerId", "invoiceNumber", "status", "total", "currency", "createdAt"],
        );
        assert_eq!(invoice["status"], "paid", "枚举必须落 as_str 的小写串");
        assert!(invoice.get("lineItems").is_none(), "行明细不过线");

        let customer = serde_json::to_value(customer_summary_view(Customer {
            id: "cus-1".into(),
            name: "客户".into(),
            email: "a@b.c".into(),
            phone: None,
            company: None,
            customer_type: CustomerType::Consumer,
            country: None,
            region: None,
            city: None,
            address: None,
            latitude: None,
            longitude: None,
            source: None,
            tags: Vec::new(),
            notes: String::new(),
            total_revenue: 10.0,
            invoice_count: 1,
            status: CustomerStatus::Active,
            created_at: 1,
            updated_at: 2,
        }))
        .expect("客户视图可序列化");
        assert_keys(
            &customer,
            &["id", "name", "email", "phone", "company", "status", "totalRevenue", "createdAt"],
        );
        assert!(customer.get("notes").is_none(), "备注不过线");

        let project = serde_json::to_value(project_summary_view(&Project {
            id: "prj-1".into(),
            customer_id: None,
            title: "项目".into(),
            description: String::new(),
            status: ProjectStatus::Active,
            milestones: vec![
                Milestone {
                    id: "m1".into(),
                    title: "一".into(),
                    description: String::new(),
                    due_at: None,
                    completed_at: Some(1),
                    status: MilestoneStatus::Completed,
                },
                Milestone {
                    id: "m2".into(),
                    title: "二".into(),
                    description: String::new(),
                    due_at: None,
                    completed_at: None,
                    status: MilestoneStatus::Pending,
                },
            ],
            budget: Some(1.0),
            currency: "CNY".into(),
            started_at: None,
            deadline: None,
            completed_at: None,
            notes: String::new(),
            created_at: 1,
            updated_at: 2,
        }))
        .expect("项目视图可序列化");
        assert_keys(
            &project,
            &[
                "id",
                "customerId",
                "title",
                "status",
                "budget",
                "currency",
                "deadline",
                "milestoneTotal",
                "milestoneCompleted",
                "createdAt",
            ],
        );
        assert_eq!(project["milestoneTotal"], 2);
        assert_eq!(project["milestoneCompleted"], 1, "只数 `Completed`，不数 in_progress");
        assert!(project.get("milestones").is_none(), "里程碑明细不过线（明细只在详情面）");

        let landing = serde_json::to_value(landing_page_summary_view(LandingPage {
            id: "lp-1".into(),
            title: "落地页".into(),
            slug: "lp".into(),
            description: String::new(),
            content: "正文".into(),
            published: true,
            published_at: Some(1),
            created_at: 1,
            updated_at: 2,
        }))
        .expect("落地页视图可序列化");
        assert_keys(&landing, &["id", "title", "slug", "published", "createdAt"]);
        assert!(landing.get("content").is_none(), "正文不过线");

        let contact = serde_json::to_value(contact_summary_view(ContactSubmission {
            id: "ct-1".into(),
            name: "来访".into(),
            email: "a@b.c".into(),
            message: "留言".into(),
            source: "site".into(),
            read: false,
            created_at: 1,
        }))
        .expect("联系表单视图可序列化");
        assert_keys(&contact, &["id", "name", "email", "source", "read", "createdAt"]);
        assert!(contact.get("message").is_none(), "留言不过线");

        let kpi = serde_json::to_value(kpi_summary_view(KpiRecord {
            id: "kpi-1".into(),
            domain_pack_id: String::new(),
            name: "营收".into(),
            value: 1.5,
            unit: "CNY".into(),
            period: "2026-11".into(),
            recorded_at: 1,
            created_at: 1,
        }))
        .expect("KPI 视图可序列化");
        assert_keys(&kpi, &["id", "domainPackId", "name", "value", "unit", "period", "recordedAt"]);
        assert_eq!(kpi["domainPackId"], "", "空串 = 未标注，不许臆造域包");

        let asset = serde_json::to_value(content_asset_summary_view(ContentAsset {
            id: "ca-1".into(),
            title: "资产".into(),
            content_type: "article".into(),
            body: "正文".into(),
            tags: vec!["opc".into()],
            status: "published".into(),
            created_at: 1,
            updated_at: 2,
        }))
        .expect("内容资产视图可序列化");
        assert_keys(&asset, &["id", "title", "contentType", "status", "tags", "createdAt"]);
        assert!(asset.get("body").is_none(), "正文不过线");

        let schedule = serde_json::to_value(publish_schedule_summary_view(PublishSchedule {
            id: "ps-1".into(),
            content_ref_type: "blog".into(),
            content_ref_id: "post-1".into(),
            scheduled_at: 1,
            status: "pending".into(),
            published_at: None,
            created_at: 1,
            updated_at: 2,
        }))
        .expect("发布计划视图可序列化");
        assert_keys(
            &schedule,
            &["id", "contentRefType", "contentRefId", "status", "scheduledAt", "publishedAt"],
        );
    }
}
