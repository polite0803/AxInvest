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
    SiteService,
};
use axagent_dao::repo::opc_blog_post::{BlogPostListRow, count_blog_posts, list_blog_posts};
use axagent_harness::DatabaseConnection;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::opc_store::{
    BlogPostListOutcome, BlogPostView, ContactSummaryView, ContentAssetSummaryView,
    CreateBlogPostInput, CustomerListQuery, CustomerSummaryView, InvoiceListQuery,
    InvoiceSummaryView, KpiListQuery, KpiSummaryView, LandingPageSummaryView,
    OPC_BLOG_POST_CONTENT_MAX_BYTES, OPC_BLOG_POST_LIST_DEFAULT_LIMIT,
    OPC_BLOG_POST_LIST_LIMIT_RANGE, OpcStoreProvider, ProjectListQuery, ProjectSummaryView,
    PublishBlogPostOutcome, PublishScheduleSummaryView, RemoveBlogPostOutcome,
};
use tokio::sync::RwLock;

/// `opc.store` 的宿主实现。
pub struct HostOpcStore {
    /// 与 `AppState.harness.db()` **同一个**连接池句柄（`DatabaseConnection` 是 Clone 的句柄，
    /// 不是第二套连接语义）。
    db: DatabaseConnection,
    /// 受保护 id 名单（内置站点内容）：`delete_blog_post` 对它们回 `Protected` 且**不删除**。
    ///
    /// 名单机制照 `jobs.store` 的三件套（`axagent-runtime/crates/runtime-core/src/cron_job.rs`
    /// 的 `protect_ids` / `is_protected` + `src-tauri/src/init/state.rs` 启动时从种子模块常量登记）。
    /// ⚠ OPC 侧今天**没有种子内容**（`opc_blog_posts` 无 INSERT 种子路径，`init/` 下也没有
    /// OPC 种子模块）⇒ 默认是**空表**，`Protected` 分支由单测经 [`Self::protect_ids`] 注入取证。
    /// 将来有种子内容时在 wiring 同段登记，名单常量放种子模块 —— 不在本层拍一份字面量。
    protected: RwLock<HashSet<String>>,
}

impl HostOpcStore {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db, protected: RwLock::new(HashSet::new()) }
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
        Ok(rows.into_iter().map(project_summary_view).collect())
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
}

// ── #36 批 2 写 op 的依赖 ─────────────────────────────────────────────────
//
// 这两个 `use` **刻意放在 impl 之后**：本文件 `impl OpcStoreProvider for HostOpcStore` 那一行
// 被登记表按行号钉住（见 `scripts/capability-register.json` 的 impl 溯源段），往上方的
// `use axagent_harness::opc_store::{…}` 列表里加名字会让它换行、impl 行整体下移。
// 模块级 `use` 在任意 item 位置都生效，故这里只是位置选择，不是作用域限制。
use axagent_harness::opc_store::{
    RemoveContentAssetOutcome, RemovePublishScheduleOutcome, TransitionInvoiceOutcome,
    UpdateContentAssetInput, UpdateContentAssetOutcome,
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
        status: c.status.as_str().to_string(),
        total_revenue: c.total_revenue,
        created_at: c.created_at,
    }
}

/// 里程碑只带两个计数（完成数是现算的，判据同迁移前 `OpcListProjects`：
/// `m.status == MilestoneStatus::Completed`）。
fn project_summary_view(p: Project) -> ProjectSummaryView {
    let milestone_total = p.milestones.len();
    let milestone_completed =
        p.milestones.iter().filter(|m| m.status == MilestoneStatus::Completed).count();
    ProjectSummaryView {
        id: p.id,
        title: p.title,
        status: p.status.as_str().to_string(),
        budget: p.budget,
        currency: p.currency,
        milestone_total,
        milestone_completed,
        created_at: p.created_at,
    }
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

        let project = serde_json::to_value(project_summary_view(Project {
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
                "title",
                "status",
                "budget",
                "currency",
                "milestoneTotal",
                "milestoneCompleted",
                "createdAt",
            ],
        );
        assert_eq!(project["milestoneTotal"], 2);
        assert_eq!(project["milestoneCompleted"], 1, "只数 `Completed`，不数 in_progress");
        assert!(project.get("milestones").is_none(), "里程碑明细不过线");

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
