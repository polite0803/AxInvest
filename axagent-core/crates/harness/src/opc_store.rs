// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 站点内容接缝（`opc.store`）— DTO + Trait。
//!
//! 宿主把 OPC 的 `opc_*` 数据面经本接缝暴露给插件：读面九个 op（`list_blog_posts` 分页 + 列裁剪；
//! #36 批 1 的八条列表 op：发票 / 客户 / 项目 / 落地页 / 联系提交 / KPI / 内容资产 / 发布计划），
//! 写面 `publish_blog_post`（只 `Set` 三列）/ `delete_blog_post`（三态）/ `create_blog_post`（正文按字节设闸）。**缺行不是错误**：回 `NotFound`（走 payload、不占错误码）。闸只加写 op，读 op 不加。
//!
//! ## 为什么需要它
//!
//! OPC 域插件（`plugin-opc`）不持 `DatabaseConnection` —— 落库读写一律经宿主接缝
//! （该 crate 的 `Cargo.toml` 依赖规则原文）。而它替代的宿主命令 `opc_list_blog_posts`
//! 走的是 `DefaultSiteService::list_blog_posts`（`axagent-quant/crates/company-runtime/src/opc/site.rs`），
//! 那条服务在**实现层**、不在契约层，插件进程无法链接它 ⇒ 必须有一条 IPC 形态的数据面。
//! 命名取 `opc.store`（与 `jobs.store` / `trajectory.store` 同族）：OPC 的站点数据面
//! （博客 / 落地页 / 联系提交 / 排期）后续都挂在这一条接缝的**不同 op** 上，
//! 不再为每张表新起一条接缝。
//!
//! ## 视图裁剪：`content` 整列不过线
//!
//! `opc_blog_posts.content` 是 `column_type = "Text"` 的正文列（博客全文，单条可达数十 KB
//! 甚至更多），而现网两个消费面都不读它：
//! - `src/pages/opc/components/SitesTab.tsx` 的 `BlogPost` 声明里**没有** `content`
//!   （列表列只读 title / slug / published / viewCount / tags，键名与本视图逐字同名）；
//! - `src/pages/opc/components/DashboardTab.tsx` 只取 `posts.length` 当 KPI 计数。
//!
//! 要读正文应走宿主详情面（`SiteService::get_blog_post`，本接缝不承载），裁剪先例见
//! [`crate::analysis_record`] 模块头的「视图裁剪」一节与 `trajectory_store.rs` 的
//! 「`steps` / `rewards` 不过帧」。⚠ 与 `analysis.record` 不同的是：这里**连 SELECT 都没有**
//! —— 列裁剪做在 SQL 侧（`axagent-infra/crates/dao/src/repo/opc_blog_post.rs`），
//! 不存在「进了宿主内存但没进帧」的中间态。
//!
//! ## `tags` 的口径
//!
//! 落库列是 `tags_json`（缺省 `"[]"` 的 Text 列），本视图交**解码后的字符串数组**：
//! 与它替代的宿主 DTO（`company-runtime` 的 `BlogPost.tags`）同一口径 —— 解码失败按空数组
//! 处理（`unwrap_or_default`），**不是**错误。接缝上不给调用方原始 JSON 串，否则会催生
//! 第二份解码实现（禁区 12）。
//!
//! ## `total` 的口径
//!
//! `total` = 整表行数（与 `posts` **同一个集合**在 `limit` 之前的匹配总数，当前无过滤条件），
//! 照 [`crate::analysis_record::AnalysisRecordListOutcome`] 的 `total` 口径：出参是具名对象
//! 而不是裸数组，就是为了挂这个数 —— 插件的「取尽整表」分页循环靠它判断何时停，
//! 拿「本页不满」推断「没有下一页」在整页恰好填满时是错的。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::core_error::{AxAgentError, Result};

// ── 硬上限（契约级数字，单源）────────────────────────────────────────────
//
// ⚠ 宿主 provider 与 plugins 分派层**只准引用这里**，不得另建一套（先例
// [`crate::analysis_record::ANALYSIS_RECORD_LIST_LIMIT_RANGE`]）。

/// `list_blog_posts` 的 `limit` 合法闭区间。
///
/// 上界 200 与 `analysis.record` 同值、推导不同：博客行带 `excerpt`（摘要 Text 列），
/// 实测形态约 0.5–2 KB/条 ⇒ 200 条一帧 ≈ 100–400 KB，仍在帧预算内；再往上放就要么
/// 逼调用方分页、要么让单帧过大。下界 1：`limit = 0` 没有合法语义，「不想要数据」应当不调用。
///
/// ⚠ 它**比被本接缝替代的那条宿主命令更严**：`opc_list_blog_posts` 一次返回整表、无上限
/// （同进程 IPC，不过帧）。插件侧要拿整表必须按本页 `total` 分页取尽（见 `plugin-opc`）。
pub const OPC_BLOG_POST_LIST_LIMIT_RANGE: (usize, usize) = (1, 200);

/// `list_blog_posts` 不传 `limit` 时的缺省页大小。
///
/// 定 50 的理由与 `analysis.record` 同一条：现网消费方的缺省档取的都是 50
/// （[`crate::analysis_record::ANALYSIS_RECORD_LIST_DEFAULT_LIMIT`]），不另拍一个数。
pub const OPC_BLOG_POST_LIST_DEFAULT_LIMIT: usize = 50;

// 区间自洽 + 缺省值落在区间内（任一写错，`list_blog_posts` 会在宿主侧恒拒或恒夹取）。
const _: () = assert!(OPC_BLOG_POST_LIST_LIMIT_RANGE.0 <= OPC_BLOG_POST_LIST_LIMIT_RANGE.1);
const _: () = assert!(
    OPC_BLOG_POST_LIST_DEFAULT_LIMIT >= OPC_BLOG_POST_LIST_LIMIT_RANGE.0
        && OPC_BLOG_POST_LIST_DEFAULT_LIMIT <= OPC_BLOG_POST_LIST_LIMIT_RANGE.1
);

// ── DTO（跨边界视图；宿主实体不直接过线）────────────────────────────────

/// 一条博客文章的**列表视图**（`opc_blog_posts` 的列裁剪投影，不含 `content`）。
///
/// 时间口径只有一个单位：epoch **秒**（`i64`），provider 不做二次换算，也不发 `chrono` 字符串。
/// ⚠ 初版误写「毫秒」：写入侧统一用 `now_ts()`（秒），逐字段实证见 `publish_blog_post` 方法头。
///
/// 字段 `None` 的语义逐条写死：
/// - `publishedAt`：`None` = **尚未发布**（草稿）。落库列可空，发布动作才写入
///   ⇒ 不得把 `None` 读成 0，也不得回退到 `createdAt`；
/// - `published`：落库是 `i32` 哨兵列，视图按「非 0 即已发布」收敛成 `bool`
///   （与它替代的宿主 DTO `BlogPost.published` 同一条判据）。`published == false` 时
///   `publishedAt` 正常应为 `None`，但**两者不是同一判据**（历史行存在只改标志未写时刻的
///   形态），消费方要判「是否发布」只认 `published`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlogPostView {
    /// 文章 id（详情查询与发布动作的唯一句柄）。
    pub id: String,
    pub title: String,
    /// URL slug：对外路由键，表上带 `unique` ⇒ 同名不可并存。
    pub slug: String,
    /// 摘要（`excerpt` Text 列）。列表页展示用它，**不用**正文。
    pub excerpt: String,
    /// 标签（`tags_json` 解码结果；解码失败 = 空数组）。
    pub tags: Vec<String>,
    pub published: bool,
    pub published_at: Option<i64>,
    /// 浏览量。保持实体列宽 `i64`（对齐 PG bigint），不在接缝上收窄成 `u32`。
    pub view_count: i64,
    /// 创建时刻（epoch 秒）。
    pub created_at: i64,
    /// 最近一次写库时刻（epoch 秒）。
    pub updated_at: i64,
}

/// `list_blog_posts` 的返回：具名对象（不返裸数组），行 + `limit` 之前的匹配总数。
///
/// 理由同 `analysis.record`：裸数组没地方挂 `total`，插件侧就只能拿「本页不满」猜
/// 「没有下一页」，而整页恰好填满时那个推断是错的。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlogPostListOutcome {
    /// 本页文章（宿主按 `createdAt` **降序**，同刻并列按 `id` 升序 —— 分页第二排序键，
    /// 见 dao 的 `opc_blog_post.rs`）。
    pub posts: Vec<BlogPostView>,
    /// `limit` **之前**的匹配总数（当前无过滤条件 ⇒ 整表行数）。
    pub total: u64,
}

// ── Trait ───────────────────────────────────────────────────────────────

/// OPC 数据面提供者（宿主独占 `opc_*` 表：九个读 op + 三个写 op，清单见模块头）。
///
/// 实现方：宿主 wiring 层（`src-tauri/src/init/opc_store_service.rs`，只调 dao，不直连库）。
/// 默认实现返回「该提供者不支持此数据面」，便于外部实现只覆盖关心的 op
/// （与 `AnalysisRecordProvider` / `TrajectoryStoreProvider` 同款缺省）。
///
/// **权限闸只加在写 op 上**：`worker.rs` 的 `required_permission_for` 里 `publish_blog_post`
/// 要 `write`，而 `list_blog_posts` 必须仍是 `None`（给读缝加闸会让只读插件被无端拒掉）。
#[async_trait]
pub trait OpcStoreProvider: Send + Sync {
    /// 博客文章列表（`list_blog_posts` op）。
    ///
    /// - `limit = None` ⇒ 取 [`OPC_BLOG_POST_LIST_DEFAULT_LIMIT`]；显式值越出
    ///   [`OPC_BLOG_POST_LIST_LIMIT_RANGE`] ⇒ `Err(Validation)` 拒收（**不静默夹取**：
    ///   夹取会让插件拿到的条数与它以为的不同，分页据此算错）；
    /// - `offset = None` ⇒ `0`；
    /// - 排序与列裁剪钉在 SQL 侧（dao），provider 不做内存过滤；
    /// - 空表 / 越界 offset ⇒ `posts` 空数组、`total` 照常返回，**不是错误**。
    async fn list_blog_posts(
        &self,
        _limit: Option<usize>,
        _offset: Option<usize>,
    ) -> Result<BlogPostListOutcome> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::list_blog_posts 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 发布博客文章（`publish_blog_post` op）—— 本接缝**第一个写 op**，要 `write` 权限。
    ///
    /// 入参只有 `id`，**完全不碰 `content`**：写侧只 `Set` `published` / `published_at` /
    /// `updated_at` 三列。逐字段口径抄自现写体
    /// `axagent-quant/crates/company-runtime/src/opc/site.rs` 的 `publish_blog_post`：
    /// 时刻取 `crate::util_fns::now_ts()` —— epoch **秒**（该函数体是
    /// `chrono::Utc::now().timestamp()`），不是 `now_ms()`。⚠ 抄错单位的后果不是「数值难看」
    /// 而是**静默查空**：`opc::data_service` 的 KPI 按 `published_at BETWEEN from AND to` 计数，
    /// 而 `from` / `to` 由 `TimeRange` 给出、同为秒（该处注释已逐字核对）⇒ 写成毫秒会让这一行
    /// 永远落在窗口之外，看板的「发布篇数」静默少一篇。
    ///
    /// - `id` 空 / 全空白 ⇒ `Err(Validation)`（provider 侧的闸，在 IO 之前）；
    /// - 行不存在 ⇒ `Ok(PublishBlogPostOutcome::NotFound)`，**不是错误**（判据见该枚举）；
    /// - 对已发布的行重复调用 ⇒ 仍然 `Ok(Published)`，并把 `published_at` 刷成本次时刻
    ///   （现写体就是无条件 `Set`；本 op 不改写成「已发布则跳过」，否则两条写通道行为分叉）。
    async fn publish_blog_post(&self, _id: String) -> Result<PublishBlogPostOutcome> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::publish_blog_post 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 删除博客文章（`delete_blog_post` op）—— 写 op，要 `write` 权限。
    ///
    /// 三态出参见 [`RemoveBlogPostOutcome`]（与 `jobs.store` 的 `remove` 同族）：
    /// - `id` 空 / 全空白 ⇒ `Err(Validation)`（provider 侧闸，在 IO **之前**）；
    /// - 受保护 id ⇒ `Ok(Protected)` 且**一条 DELETE 都不发**（先判保护、再删）；
    /// - 行不存在 ⇒ `Ok(NotFound)` —— **不是错误**：宿主 service 按 `rows_affected == 0` 判，
    ///   不许回「删成功」；
    /// - 命中 ⇒ `Ok(Removed)`。
    ///
    /// ⚠ 「删不存在的行也回成功」是本仓踩过的形态（裸 `Entity::delete_by_id().exec()` 不检
    /// `rows_affected`）：那会让前端把一次空操作读成真删除。本 op 只接受**可判别**结果。
    async fn delete_blog_post(&self, _id: String) -> Result<RemoveBlogPostOutcome> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::delete_blog_post 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 新建博客文章（`create_blog_post` op）—— 写 op，要 `write` 权限。
    ///
    /// 入参是既有领域 DTO [`CreateBlogPostInput`]（本批从
    /// `axagent-quant/crates/company-runtime/src/opc/site.rs` 上收：它要跨进程过帧，
    /// 而插件进程链接不到实现层 crate ⇒ 权威定义必须在契约层，实现层保留 `pub use` 垫片）。
    ///
    /// - `input.content` 按 **UTF-8 字节**计：超 [`OPC_BLOG_POST_CONTENT_MAX_BYTES`] ⇒
    ///   `Err(Validation)`，**不截断**（截断等于让调用方以为整篇都进库了）；
    /// - `input.slug` 空 / 全空白 ⇒ `Err(Validation)`（它是对外路由键，表上 `unique`）；
    ///   `title` / `excerpt` / `content` 的空串**不拒**（先落草稿、后补正文是合法流程）；
    /// - 落库走宿主既有 `SiteService::create_blog_post`（**不是**本 trait 里重写一遍 SQL），
    ///   回创建后的[`BlogPostView`]（列裁剪同读面：`content` 不过线，调用方要正文走详情面）。
    ///
    /// 为什么出参是视图而不是裸 id：前端拿到 id 与拿到整行是同一批字段（#180 ④ 的教训 ——
    /// 让前端自己 `randomUUID()` 造 id 会落下悬空引用），而视图已经是接缝唯一的线格式。
    async fn create_blog_post(&self, _input: CreateBlogPostInput) -> Result<BlogPostView> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::create_blog_post 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    // ── 只读查询 op（#36 批 1）────────────────────────────────────────
    //
    // 八个列表 op 的判据一致：空表 ⇒ `Ok(vec![])`（**不是错误**，同 `list_blog_posts` 的
    // 空页口径）；过滤 / 排序 / 列裁剪都钉在实现层查询里（provider 不做内存过滤）。
    // 这些查询的 `limit` 是「宿主工具的历史缺省」，不参与插件侧分页算术 ⇒ 本批不另设
    // 区间闸（与 `list_blog_posts` 的 `OPC_BLOG_POST_LIST_LIMIT_RANGE` 不同源，勿混）。
    /// 发票列表（`list_invoices` op）。`status` 是枚举串，非法值按「不过滤」处理
    /// （与迁移前宿主工具 `OpcListInvoices` 的 `from_str(...).ok()` 行为逐字一致）。
    async fn list_invoices(&self, _query: InvoiceListQuery) -> Result<Vec<InvoiceSummaryView>> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::list_invoices 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 客户列表（`list_customers` op）。`status` 非法串按「不过滤」处理；`customerType`
    /// 遵**域枚举**的既有语义（`CustomerType::from_str` 未知串落 `Unknown`），迁移前宿主
    /// 工具就是 `from_str(...).ok()` ⇒ 无这一档的两侧语义差。
    ///
    /// 入参**没有** `tags`：域服务 `list_customers` 今天只认 status / customerType / search /
    /// limit / offset（`CustomerFilter.tags` 虽在结构体里但没有任何分支消费它），迁移前宿主
    /// 工具也恒传 `None`。把一个不生效的过滤键摆上契约 = 对调用方宣称「筛过了」——要开这个
    /// 能力须同批补域服务分支，不在迁移里顺手加。
    async fn list_customers(&self, _query: CustomerListQuery) -> Result<Vec<CustomerSummaryView>> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::list_customers 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 项目列表（`list_projects` op）。里程碑完成数由 provider 按明细现算
    /// （视图只带两个计数，不带里程碑明细 —— 列表面不扩列）。
    async fn list_projects(&self, _query: ProjectListQuery) -> Result<Vec<ProjectSummaryView>> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::list_projects 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 落地页列表（`list_landing_pages` op）。
    async fn list_landing_pages(&self) -> Result<Vec<LandingPageSummaryView>> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::list_landing_pages 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 联系表单提交列表（`list_contacts` op）。
    async fn list_contacts(&self) -> Result<Vec<ContactSummaryView>> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::list_contacts 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// KPI 记录列表（`list_kpis` op）。`period` 为空表示不过滤；跨域包语义沿用
    /// 迁移前 `list_kpis_all` 的口径（`domainPackId` 为空串 = 未标注，**不是**某个域包）。
    async fn list_kpis(&self, _query: KpiListQuery) -> Result<Vec<KpiSummaryView>> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::list_kpis 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 内容资产列表（`list_content_assets` op）。`body` 不过线（列裁剪同博客 `content`）。
    async fn list_content_assets(&self) -> Result<Vec<ContentAssetSummaryView>> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::list_content_assets 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 发布计划列表（`list_publish_schedules` op）。
    async fn list_publish_schedules(&self) -> Result<Vec<PublishScheduleSummaryView>> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::list_publish_schedules 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    // ── 写 op（#36 批 2：id / 枚举形状）────────────────────────────────
    //
    // 四条写 op 判定一致（与 blog 三件同族）：
    // ① 空 id ⇒ `Err(Validation)`（provider 侧闸，在 IO **之前**）；
    // ② 行不存在 ⇒ 走 **payload**（`NotFound` / `"not_found"`），不占错误码；
    // ③ 其余失败才是 `Err(Provider)`（分派层落 `SEAM_CALL_FAILED`）。

    /// 变更发票状态（`transition_invoice` op）—— 写 op，要 `write` 权限。
    ///
    /// - `target` 是枚举串（`draft` / `sent` / `paid` / `overdue` / `cancelled` / `refunded`）：
    ///   非法串 ⇒ `Err(Validation)`（迁移前宿主工具落的就是 `invalid_input`）；
    /// - 行不存在 ⇒ `Ok(TransitionInvoiceOutcome::NotFound)`；
    /// - **非法状态转换**（域服务 `InvalidStateTransition`）⇒ `Err(Provider)`：迁移前宿主
    ///   工具把它包成 `execution_failed`，本批不改判；
    /// - 成功 ⇒ `Transitioned(视图)`（列裁剪同读面，`line_items` 不过线）。
    async fn transition_invoice(
        &self,
        _id: String,
        _target: String,
    ) -> Result<TransitionInvoiceOutcome> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::transition_invoice 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 更新内容资产（`update_content_asset` op）—— 写 op，要 `write` 权限。
    ///
    /// 入参是 [`UpdateContentAssetInput`]（本批从实现层上收：它要跨进程过帧）：五个字段
    /// 全 `Option`，`None` = 该列**不动**（不是清空 —— 清空要显式传空串，与迁移前
    /// `SiteService::update_content_asset` 的 `if let Some(..)` 分支逐字同义）。
    ///
    /// - 行不存在 ⇒ `Ok(UpdateContentAssetOutcome::NotFound)`；
    /// - 成功 ⇒ `Updated(视图)`（`body` 不过线，同 `list_content_assets` 的列裁剪）。
    async fn update_content_asset(
        &self,
        _id: String,
        _input: UpdateContentAssetInput,
    ) -> Result<UpdateContentAssetOutcome> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::update_content_asset 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 删除内容资产（`delete_content_asset` op）—— 写 op，要 `write` 权限。
    ///
    /// 两态出参见 [`RemoveContentAssetOutcome`]：缺行 ⇒ `NotFound`（**不是错误**，
    /// 判据同 [`RemoveBlogPostOutcome`]）。本表无「受保护 id」概念（域服务没有该分支）。
    async fn delete_content_asset(&self, _id: String) -> Result<RemoveContentAssetOutcome> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::delete_content_asset 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 取消发布计划（`cancel_publish_schedule` op）—— 写 op，要 `write` 权限。
    ///
    /// 语义是**硬删**（域服务 `delete_publish_schedule` 走 `delete_by_id`），不是把状态
    /// 改成 `cancelled` —— 与迁移前宿主工具 `OpcCancelPublishSchedule` 逐字一致。
    async fn cancel_publish_schedule(&self, _id: String) -> Result<RemovePublishScheduleOutcome> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::cancel_publish_schedule 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    // ── 写 op（#36 批 3：创建 / 流程动作）────────────────────────────
    //
    // 八个写 op 的判据一致：出参是创建后的**列裁剪视图**（先例 `create_blog_post` ——
    // 让调用方直接拿到 id 句柄，不另造「裸 id」形态）；失败才 `Err(Provider)`。
    // 入参缺省值（货币 / 备注 / 描述 / 行项目数量）由 provider 补，DTO 用 `Option`
    // 表达「调用方没给」—— 不把「没给」与「显式传空串」读成同一件事。

    /// 新建发票（`create_invoice` op）—— 写 op，要 `write` 权限。
    ///
    /// 行项目的 `total` 由 provider 按 `quantity × unitPrice × (1 + taxRate)` 现算
    /// （与迁移前宿主工具 `OpcCreateInvoice` 同一公式，搬到这里是为了让两条通道只有一处算法）。
    async fn create_invoice(&self, _input: CreateInvoiceRequest) -> Result<InvoiceSummaryView> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::create_invoice 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 新建客户（`create_customer` op）—— 写 op，要 `write` 权限。
    ///
    /// `customerType` / `source` 是**串**：provider 按域枚举的既有语义翻译
    /// （`consumer` / `business` 以外的输入落 `Unknown`，五个已知来源以外的落 `Other`），
    /// 与迁移前宿主工具的解码逐字一致。
    async fn create_customer(&self, _input: CreateCustomerRequest) -> Result<CustomerSummaryView> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::create_customer 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 新建项目（`create_project` op）—— 写 op，要 `write` 权限。
    async fn create_project(&self, _input: CreateProjectRequest) -> Result<ProjectSummaryView> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::create_project 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 给项目加里程碑（`add_milestone` op）—— 写 op，要 `write` 权限。
    ///
    /// 里程碑 id 由 provider 生成（`util_fns::gen_id()`），状态恒为 `Pending` ——
    /// 与迁移前宿主工具一致；调用方不提供 id（前端历史上有自己造 id 的写法，
    /// 那会让「谁生成的 id」出现两个答案）。
    async fn add_milestone(&self, _input: AddMilestoneRequest) -> Result<ProjectSummaryView> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::add_milestone 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 新建落地页（`create_landing_page` op）—— 写 op，要 `write` 权限。
    async fn create_landing_page(
        &self,
        _input: CreateLandingPageRequest,
    ) -> Result<LandingPageSummaryView> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::create_landing_page 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 财务报表（`get_financial_report` op）—— **读 op**，不设权限闸。
    ///
    /// 报告与投资建议是同一份读数的两半（建议由报告派生），故在一次调用里一并返回：
    /// 分成两个 op 会让插件侧多一次帧往返，且中间态毫无独立意义。
    async fn get_financial_report(&self, _period: String) -> Result<FinancialReportView> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::get_financial_report 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 新建内容资产（`create_content_asset` op）—— 写 op，要 `write` 权限。
    async fn create_content_asset(
        &self,
        _input: CreateContentAssetRequest,
    ) -> Result<ContentAssetSummaryView> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::create_content_asset 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 新建发布计划（`create_publish_schedule` op）—— 写 op，要 `write` 权限。
    async fn create_publish_schedule(
        &self,
        _input: CreatePublishScheduleRequest,
    ) -> Result<PublishScheduleSummaryView> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::create_publish_schedule 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 处理所有到期发布计划（`process_due_schedules` op）—— 写 op，要 `write` 权限
    /// （它会把状态改成 `published` 并回写时刻）。
    ///
    /// 空集 ⇒ `Ok(vec![])`（「没有到期的」是正常读数，不是错误）。
    async fn process_due_schedules(&self) -> Result<Vec<PublishScheduleSummaryView>> {
        Err(AxAgentError::Provider(
            "OpcStoreProvider::process_due_schedules 未实现（该提供者不支持此数据面）".into(),
        ))
    }
}

// ── 写 op 的出参（刻意放在 trait **之后**）──────────────────────────────────
//
// 本文件 `pub trait OpcStoreProvider` 那一行被登记表按**行号**钉住（见
// `scripts/capability-register.json` 的 `k1PubTraitCount` 溯源段与 PLAN §115.1 的接缝面表）
// ⇒ 新增 DTO 一律追加到 trait 之后；trait 之前的改动（模块头、`BlogPostView` 的时间一节、
// trait 文档）必须逐条**行中性**，否则那个行号整体移位、登记表读数全烂。

/// `publish_blog_post` 的结果：**缺行是正常结果，不是错误**。
///
/// 为什么不借错误码表达「id 不存在」：现写体 `site.rs` 用的是 `OpcError::NotFound`，但那条路
/// 只在同进程 IPC 上；接缝上「这一行不在表里」对调用方是**业务读数**，与
/// [`crate::jobs_store::RemoveJobOutcome::NotFound`] 同一档判据 —— 错误码留给真故障
/// （SQL 失败 / 未实现 / 入参非法）。升成错误码会让插件把「没有这篇」与「库连不上」读成同一件事。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublishBlogPostOutcome {
    /// 命中并已写入三列（含「本来就是已发布状态」的重复发布）。
    Published,
    /// `id` 不在表里：**未发起任何 UPDATE**，行数据原样。
    NotFound,
}

/// `delete_blog_post` 的结果：区分「已删除」「不存在」「受保护未删除」。
///
/// 三态与 [`crate::jobs_store::RemoveJobOutcome`] **同族同口径**（那边叫 `RemoveJobOutcome`
/// —— 命名规则是「表/资源名 + `Remove…Outcome`」，故本接缝落 `RemoveBlogPostOutcome`；
/// 序列化形态逐字对齐：`#[serde(rename_all = "snake_case")]` ⇒ `"removed"` / `"not_found"` /
/// `"protected"`，插件侧逐字透传，不做任何包装）。
///
/// 为什么 `NotFound` / `Protected` 都走 payload 而不是错误码：它们对调用方都是**业务读数**
/// （「这篇本来就不在」「这篇不许删」），与「库连不上 / 未实现」不是同一档。升成错误码会让
/// 插件把三种事读成一件（判据同 [`PublishBlogPostOutcome`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoveBlogPostOutcome {
    /// 命中并已删除（`rows_affected == 1`）。
    Removed,
    /// `id` 不在表里：**未发起任何 DELETE**。
    NotFound,
    /// 受保护 id（内置站点内容）：**未删除**，行数据原样。
    Protected,
}

/// `create_blog_post` 可提交的正文上限（**字节**，UTF-8 计）。
///
/// 与 [`crate::knowledge_vector::KNOWLEDGE_CONTENT_MAX_BYTES`] **同值同口径**（64 KiB）：
/// 那条是「一次 embedding 调用的成本防线」，这条是「一次写库 + 一次回读的帧体积防线」，
/// 判据都是**字节**而不是字符 —— 中文正文的字符数只有字节数的三分之一，按字符判会放行
/// 三倍于预算的正文。
///
/// 为什么**不**复用 `MAX_FRAME_LEN`（64 MiB）：那是**帧层**上限（长度前缀能装下多少），
/// 不是 op 预算 —— 拿它当预算等于「一帧塞 64 MiB 正文」，宿主侧写库与回读都会被拖住，
/// 而调用方从帧层永远收不到「这篇太大」的明确拒绝（只能撞到一个巨大的超时）。
///
/// 超限 ⇒ provider 回 `Err(Validation)`（分派层落 `SEAM_CALL_INVALID_ARGS`），**不截断**。
pub const OPC_BLOG_POST_CONTENT_MAX_BYTES: usize = 64 * 1024;

/// `create_blog_post` 的入参（`opc.store` 写面唯一携带大字段的 DTO）。
///
/// 权威定义在本模块（**不是**实现层）：它要跨进程过帧，而插件进程按依赖规则链接不到
/// `company-runtime` 的实现层。实现层
/// （`axagent-quant/crates/company-runtime/src/opc/site.rs`）保留 `pub use` 垫片，
/// 既有 `SiteService::create_blog_post` 的签名与所有调用方**零改动**（禁区 12 单源）。
///
/// 字段与 `opc_blog_posts` 的落库列一一对应：
/// - `slug`：对外路由键，表上 `unique` ⇒ 空 / 全空白由 provider 拒收（重复由 DB 唯一约束拒）；
/// - `content`：正文，**唯一**按字节设闸的字段（见 [`OPC_BLOG_POST_CONTENT_MAX_BYTES`]）；
/// - `tags`：落库是 `tags_json`，编码在实现层做一次（不在接缝上给调用方第二个形态）。
///
/// ⚠ `rename_all = "camelCase"` 是禁区 13 的硬要求。本 DTO 的五个字段名恰好都是单词
/// （`title` / `slug` / `excerpt` / `content` / `tags`）⇒ 注解**不改**线格式，上收前后
/// 前端 `invoke("opc_create_blog_post", { input })` 的载荷逐字兼容。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBlogPostInput {
    pub title: String,
    pub slug: String,
    pub excerpt: String,
    pub content: String,
    pub tags: Vec<String>,
}

// ── 只读查询 op 的入参（#36 批 1）──────────────────────────────────────────
//
// 四个查询 DTO 的字段与实现层 `*Filter` 逐项对应，但枚举位一律收 **串**：接缝层不认识
// 枚举（插件进程链接不到实现层），非法串由 provider 按「不过滤」丢弃 —— 这是迁移前宿主
// 工具的原语义，不在迁移中顺手改判（改成拒收会让「传了个旧枚举值」从静默忽略变成硬错）。

/// `list_invoices` 的过滤条件。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InvoiceListQuery {
    /// 状态枚举串（`draft` / `sent` / `paid` / `overdue` / `cancelled` / `refunded`）。
    pub status: Option<String>,
    pub customer_id: Option<String>,
    pub date_from: Option<i64>,
    pub date_to: Option<i64>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// `list_customers` 的过滤条件。
///
/// 字段与 `CustomerFilter` 的**生效**分支逐项对应：`tags` 不在其中（域服务不消费它 ⇒ 摆上
/// 契约只会静默不筛，见 trait 侧 `list_customers` 的说明）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CustomerListQuery {
    pub status: Option<String>,
    pub customer_type: Option<String>,
    pub search: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// `list_projects` 的过滤条件。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectListQuery {
    pub status: Option<String>,
    pub customer_id: Option<String>,
    pub search: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// `list_kpis` 的过滤条件。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KpiListQuery {
    pub period: Option<String>,
    pub limit: Option<u32>,
}

// ── 只读查询 op 的出参视图（列裁剪后的唯一线格式）─────────────────────────
//
// 全部 `rename_all = "camelCase"`（禁区 13）。**不带**大字段：博客/内容资产的正文
// （`content` / `body`）、联系表单的留言（`message`）与发票行明细（`lineItems`）都不过线
// —— 它们要么是列表工具从不展示的字段，要么会让一帧体积与整表行数相乘。

/// 发票摘要视图。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceSummaryView {
    pub id: String,
    pub customer_id: String,
    pub invoice_number: String,
    pub status: String,
    pub total: f64,
    pub currency: String,
    pub created_at: i64,
}

/// 客户摘要视图。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomerSummaryView {
    pub id: String,
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub status: String,
    pub total_revenue: f64,
    pub created_at: i64,
}

/// 项目摘要视图（里程碑只带两个计数，不带明细）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummaryView {
    pub id: String,
    pub title: String,
    pub status: String,
    pub budget: Option<f64>,
    pub currency: String,
    pub milestone_total: usize,
    pub milestone_completed: usize,
    pub created_at: i64,
}

/// 落地页摘要视图。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LandingPageSummaryView {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub published: bool,
    pub created_at: i64,
}

/// 联系表单摘要视图（`message` 不过线）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactSummaryView {
    pub id: String,
    pub name: String,
    pub email: String,
    pub source: String,
    pub read: bool,
    pub created_at: i64,
}

/// KPI 记录摘要视图。`domain_pack_id` 为空串表示「未标注」（存量行语义，见 `KpiRecord`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KpiSummaryView {
    pub id: String,
    pub domain_pack_id: String,
    pub name: String,
    pub value: f64,
    pub unit: String,
    pub period: String,
    pub recorded_at: i64,
}

/// 内容资产摘要视图（`body` 不过线）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentAssetSummaryView {
    pub id: String,
    pub title: String,
    pub content_type: String,
    pub status: String,
    pub tags: Vec<String>,
    pub created_at: i64,
}

/// 发布计划摘要视图。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishScheduleSummaryView {
    pub id: String,
    pub content_ref_type: String,
    pub content_ref_id: String,
    pub status: String,
    pub scheduled_at: i64,
    pub published_at: Option<i64>,
}

// ── 写 op（#36 批 2：id / 枚举形状）──────────────────────────────────────
//
// 出参一律是「业务读数 + 数据」的外部标记枚举（`{"transitioned": {…}}` / `"not_found"`），
// 与 [`PublishBlogPostOutcome`] / [`RemoveBlogPostOutcome`] 同一套判据：缺行不是错误。
// 带数据的两个变体**不** derive `Copy`（视图不是标量），插件侧逐字透传，不做包装。

/// `transition_invoice` 的结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionInvoiceOutcome {
    /// 已转换，回转换后的发票视图（与 `list_invoices` 的行同形）。
    Transitioned(InvoiceSummaryView),
    /// `id` 不在表里：**未发起任何 UPDATE**。
    NotFound,
}

/// `update_content_asset` 的结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateContentAssetOutcome {
    /// 已更新，回更新后的视图（`body` 不过线，同列表口径）。
    Updated(ContentAssetSummaryView),
    /// `id` 不在表里：**未发起任何 UPDATE**。
    NotFound,
}

/// `delete_content_asset` 的结果（两态 —— 内容资产没有「受保护 id」这一档）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoveContentAssetOutcome {
    /// 命中并已删除（`rows_affected == 1`）。
    Removed,
    /// `id` 不在表里：**未发起任何 DELETE**。
    NotFound,
}

/// `cancel_publish_schedule` 的结果（两态：本 op 是**硬删**，不是状态位改写）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemovePublishScheduleOutcome {
    /// 命中并已删除（`rows_affected == 1`）。
    Removed,
    /// `id` 不在表里：**未发起任何 DELETE**。
    NotFound,
}

/// `update_content_asset` 的入参：五个字段全 `Option`，`None` = 该列不动。
///
/// 权威定义在本模块（**不是**实现层）：它要跨进程过帧，而插件进程按依赖规则链接不到
/// `company-runtime` 的实现层。实现层
/// （`axagent-quant/crates/company-runtime/src/opc/content_asset.rs`）保留 `pub use` 垫片，
/// 既有 `ContentAssetService::update_content_asset` 的签名与所有调用方**零改动**（禁区 12）。
///
/// ⚠ `rename_all = "camelCase"` 是禁区 13 的硬要求 ⇒ `contentType` 是本 DTO 唯一被改名的
/// 键；宿主 Tauri 命令 `opc_update_content_asset` 的前端调用方同批改为 `contentType`
/// （迁移前后两条通道的线格式就此统一）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateContentAssetInput {
    pub title: Option<String>,
    pub content_type: Option<String>,
    pub body: Option<String>,
    pub tags: Option<Vec<String>>,
    pub status: Option<String>,
}

// ── 写 op 的入参（#36 批 3：创建 / 流程动作）────────────────────────────────
//
// 九个 DTO 全部是本模块自有定义（不在实现层）：它们要跨进程过帧，而插件进程按依赖
// 规则链接不到 `company-runtime`。与实现层入参的**两处刻意差异**：
// ① 枚举位收**串**（`customerType` / `source`）—— 接缝层不认识实现层的枚举类型，
//    翻译（含「未知值兜底」语义）由 provider 做；
// ② 可缺省字段用 `Option` 表达「调用方没给」，缺省值（`CNY` / 空串 / 数量 1）由
//    provider 补 —— 不把「没给」与「显式传空串」读成同一件事。

/// `create_invoice` 的一行明细（线格式 camelCase）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InvoiceLineItemInput {
    pub description: Option<String>,
    pub quantity: Option<f64>,
    pub unit_price: Option<f64>,
    pub tax_rate: Option<f64>,
}

/// `create_invoice` 的入参。
///
/// 行项目的 `total` **不在本 DTO 上**：它由 provider 按
/// `quantity × unitPrice × (1 + taxRate)` 现算 —— 与迁移前宿主工具同一公式，且不让
/// 调用方自己报一个可能与三列对不上的合计。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CreateInvoiceRequest {
    pub customer_id: String,
    pub line_items: Vec<InvoiceLineItemInput>,
    /// 缺省 `CNY`（provider 补）。
    pub currency: Option<String>,
    pub due_at: Option<i64>,
    /// 缺省空串（provider 补）。
    pub notes: Option<String>,
}

/// `create_customer` 的入参。
///
/// `customer_type` / `source` 是串：provider 按域枚举的既有解码语义翻译（未知值分别落
/// `Unknown` / `Other`），保持迁移前宿主工具的行为。`tags` 刻意不在其中 —— 迁移前工具
/// 恒传空数组，域服务也没有「新建时带标签」的入参位。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CreateCustomerRequest {
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub customer_type: Option<String>,
    pub country: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    pub address: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub source: Option<String>,
    /// 缺省空串（provider 补）。
    pub notes: Option<String>,
}

/// `create_project` 的入参。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CreateProjectRequest {
    pub customer_id: Option<String>,
    pub title: String,
    /// 缺省空串（provider 补）。
    pub description: Option<String>,
    pub budget: Option<f64>,
    /// 缺省 `CNY`（provider 补）。
    pub currency: Option<String>,
    pub deadline: Option<i64>,
    /// 缺省空串（provider 补）。
    pub notes: Option<String>,
}

/// `add_milestone` 的入参（里程碑 id 与初始状态由 provider 补）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AddMilestoneRequest {
    pub project_id: String,
    pub title: String,
    /// 缺省空串（provider 补）。
    pub description: Option<String>,
    pub due_at: Option<i64>,
}

/// `create_landing_page` 的入参。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CreateLandingPageRequest {
    pub title: String,
    pub slug: String,
    /// 缺省空串（provider 补）。
    pub description: Option<String>,
    pub content: String,
}

/// `create_content_asset` 的入参（`contentType` 是 camelCase 线格式，禁区 13）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CreateContentAssetRequest {
    pub title: String,
    pub content_type: String,
    pub body: String,
    /// 缺省空数组（provider 补）。
    pub tags: Option<Vec<String>>,
}

/// `create_publish_schedule` 的入参。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CreatePublishScheduleRequest {
    pub content_ref_type: String,
    pub content_ref_id: String,
    pub scheduled_at: i64,
}

/// `get_financial_report` 的出参：报告 + 派生建议（一次调用内成对返回）。
///
/// 字段与实现层 `FinancialReport` + `InvestmentAdvice` 逐项对应（`rename_all =
/// "camelCase"`）。它是**读**的结果，不带 `[u8]` 之类的大字段，也不裁剪
/// —— 报表的全部指标正是调用方要渲染的东西。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinancialReportView {
    pub period: String,
    pub total_revenue: f64,
    pub pending_revenue: f64,
    pub refunded_amount: f64,
    pub active_projects: u32,
    pub new_customers: u32,
    pub net_profit: f64,
    pub investable_amount: f64,
    pub churned_customers: u32,
    pub pretax_revenue: f64,
    pub risk_level: String,
    pub recommended_amount: f64,
    pub advice: String,
}

// ── 测试 ────────────────────────────────────────────────────────────────
//
// 内联测试 mod 追加在文件末尾（`items_after_test_module` 只在 clippy 下暴露）。

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// 把序列化出的 JSON 键集合钉死：本接缝**没有**登记表门禁覆盖，测试就是唯一的形态锁。
    fn keys_of(value: &Value) -> Vec<String> {
        let mut keys: Vec<String> =
            value.as_object().expect("DTO 必须序列化成对象").keys().cloned().collect();
        keys.sort();
        keys
    }

    fn expect_keys(value: &Value, expected: &[&str]) {
        let mut want: Vec<String> = expected.iter().map(|s| (*s).to_string()).collect();
        want.sort();
        assert_eq!(keys_of(value), want, "键集合漂移：{value:?}");
    }

    fn sample_view() -> BlogPostView {
        BlogPostView {
            id: "post-1".into(),
            title: "OPC 的第一篇博客".into(),
            slug: "opc-first-post".into(),
            excerpt: "摘要若干字".into(),
            tags: vec!["opc".into(), "blog".into()],
            published: true,
            published_at: Some(1_700_000_000_000),
            view_count: 42,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_060_000,
        }
    }

    /// 行键集合逐字钉住：10 个键，全部 camelCase。
    #[test]
    fn blog_post_view_key_set_is_exactly_ten_camel_case() {
        let val = serde_json::to_value(sample_view()).expect("序列化应成功");
        expect_keys(
            &val,
            &[
                "id",
                "title",
                "slug",
                "excerpt",
                "tags",
                "published",
                "publishedAt",
                "viewCount",
                "createdAt",
                "updatedAt",
            ],
        );
        // snake_case 残留一律不许（全站 camelCase 约定，禁区 13）
        for bad in ["published_at", "view_count", "created_at", "updated_at"] {
            assert!(val.get(bad).is_none(), "不得残留 snake_case 键 {bad}");
        }
    }

    /// 模块头「视图裁剪」的形态锁：正文列哪天被加回来，本用例即红。
    #[test]
    fn blog_post_view_carries_no_bulk_column() {
        let val = serde_json::to_value(sample_view()).expect("序列化应成功");
        for forbidden in ["content", "tagsJson", "tags_json"] {
            assert!(val.get(forbidden).is_none(), "{forbidden} 属正文/落库原列，不过线");
        }
    }

    /// 时间只有一个口径：整数 epoch，不出 `Date` 字符串、不自造 `*Ms` / `*Iso` 第二套键名。
    ///
    /// ⚠ 本用例锁的是**形状**（只有一个时间字段族、没有第二套口径），**不是单位** ——
    /// 夹具里的数字是任意整数，单位（epoch 秒）的实证在 dao 的真库用例
    /// `repo/opc_blog_post.rs::set_blog_post_published_hits_and_misses_against_real_db`
    /// （那里量得出写进库的值落在秒级区间）。
    #[test]
    fn blog_post_view_time_is_a_bare_epoch_integer_only() {
        let val = serde_json::to_value(sample_view()).expect("序列化应成功");
        assert_eq!(val["createdAt"], json!(1_700_000_000_000_i64));
        assert_eq!(val["publishedAt"], json!(1_700_000_000_000_i64));
        for bad in ["createdAtMs", "publishedAtMs", "createdAtIso", "publishedAtIso"] {
            assert!(val.get(bad).is_none(), "不得造第二套时间口径 {bad}");
        }
    }

    /// 草稿的 `publishedAt = None` 不得伪装成 0，`view_count` 的 0 是合法读数。
    #[test]
    fn draft_fields_stay_null_not_zero() {
        let mut view = sample_view();
        view.published = false;
        view.published_at = None;
        view.view_count = 0;
        view.tags.clear();
        let val = serde_json::to_value(&view).expect("序列化应成功");
        assert_eq!(val["publishedAt"], json!(null), "未发布不得回退成 createdAt 或 0");
        assert_eq!(val["published"], json!(false));
        assert_eq!(val["viewCount"], json!(0), "零浏览是合法读数，不是缺失");
        assert_eq!(val["tags"], json!([]), "无标签 = 空数组");
    }

    #[test]
    fn blog_post_view_roundtrip() {
        let back: BlogPostView =
            serde_json::from_value(serde_json::to_value(sample_view()).expect("序列化应成功"))
                .expect("往返应成立");
        assert_eq!(back.id, "post-1");
        assert_eq!(back.slug, "opc-first-post");
        assert_eq!(back.tags, vec!["opc".to_string(), "blog".to_string()]);
        assert_eq!(back.view_count, 42);
        assert_eq!(back.published_at, Some(1_700_000_000_000));
    }

    /// 出参是具名对象：`{posts, total}`，不是裸数组（裸数组没地方挂 `total`）。
    #[test]
    fn list_outcome_is_named_object() {
        let empty = BlogPostListOutcome { posts: Vec::new(), total: 0 };
        let val = serde_json::to_value(&empty).expect("序列化应成功");
        expect_keys(&val, &["posts", "total"]);
        assert_eq!(val["posts"], json!([]), "空表 = 空数组，不是错误");
        assert_eq!(val["total"], json!(0), "空库的 0 是合法读数");

        let one = BlogPostListOutcome { posts: vec![sample_view()], total: 137 };
        let val = serde_json::to_value(&one).expect("序列化应成功");
        assert_eq!(val["total"], json!(137));
        assert_eq!(val["posts"].as_array().expect("应为数组").len(), 1);
        let back: BlogPostListOutcome = serde_json::from_value(val).expect("往返应成立");
        assert_eq!(back.total, 137);
        assert_eq!(back.posts[0].id, "post-1");
    }

    /// 数字住在契约层：宿主 provider 与分派层只准引用这两个常量。
    #[test]
    fn limits_live_in_the_contract_layer() {
        assert_eq!(OPC_BLOG_POST_LIST_LIMIT_RANGE, (1, 200));
        assert_eq!(OPC_BLOG_POST_LIST_DEFAULT_LIMIT, 50);
        // 区间自洽性由模块里的 `const _: () = assert!(…)` 在编译期钉住，
        // 故此处不写常量对常量的 assert!（会撞 clippy::assertions_on_constants）。
    }

    /// 未覆盖任何方法的实现：吃默认实现 ⇒ `Err(Provider)`。
    struct BareStore;
    impl OpcStoreProvider for BareStore {}

    #[test]
    fn default_method_is_unimplemented() {
        let provider = BareStore;
        let err = futures::executor::block_on(provider.list_blog_posts(Some(10), None))
            .expect_err("默认实现必须是错误");
        match err {
            AxAgentError::Provider(message) => {
                assert!(
                    message.contains("OpcStoreProvider::list_blog_posts"),
                    "文案须点明 trait 与方法：{message}"
                );
            },
            other => panic!("默认实现应返回 AxAgentError::Provider，实际 {other:?}"),
        }
    }

    #[test]
    fn trait_is_object_safe() {
        // 注册进 capability_registry 靠 `Arc<dyn OpcStoreProvider>`，对象安全是前提
        let provider: std::sync::Arc<dyn OpcStoreProvider> = std::sync::Arc::new(BareStore);
        assert!(futures::executor::block_on(provider.list_blog_posts(None, None)).is_err());
    }

    /// 写 op 的默认体：与 `list_blog_posts` 同款 `Err(Provider)`，且文案必须点明 trait + 方法。
    ///
    /// 落点分层判据：「未实现」是**契约缺省**，不是入参非法 ⇒ 在分派层落成
    /// `PLUGIN_CALL_FAILED`（见 `worker.rs` 的 `seam_provider_error`），不许升成参数错误。
    #[test]
    fn publish_default_method_is_unimplemented() {
        let provider = BareStore;
        let err = futures::executor::block_on(provider.publish_blog_post("post-1".to_string()))
            .expect_err("默认实现必须是错误");
        match err {
            AxAgentError::Provider(message) => {
                assert!(
                    message.contains("OpcStoreProvider::publish_blog_post"),
                    "文案须点明 trait 与方法：{message}"
                );
            },
            other => panic!("默认实现应返回 AxAgentError::Provider，实际 {other:?}"),
        }
    }

    /// 写 op 也要能经 `Arc<dyn OpcStoreProvider>` 调到：入参是**owned `String`**（不是 `&str`），
    /// 对象安全对它同样成立 —— 宿主注册表存的就是 dyn 形式，这条不测就会到 wiring 层才炸。
    #[test]
    fn publish_is_reachable_through_the_dyn_object() {
        let provider: std::sync::Arc<dyn OpcStoreProvider> = std::sync::Arc::new(BareStore);
        let err = futures::executor::block_on(provider.publish_blog_post("post-1".to_string()))
            .expect_err("默认体必须是错误");
        assert!(matches!(err, AxAgentError::Provider(_)), "实际 {err:?}");
    }

    /// 线格式逐字钉住：`"published"` / `"not_found"` 两个 snake_case 串，不是对象也不是数字。
    ///
    /// 这是插件出口的唯一形态锁 —— `NotFound` 走 payload（可判别的**正常值**）而不借错误码，
    /// 取值漂移会让插件侧「发布成功 / 没有这篇」的判别静默失效（禁区 13：接缝出参就这一套名）。
    #[test]
    fn publish_outcome_wire_format_is_two_snake_case_strings() {
        assert_eq!(
            serde_json::to_value(PublishBlogPostOutcome::Published).expect("可序列化"),
            json!("published")
        );
        assert_eq!(
            serde_json::to_value(PublishBlogPostOutcome::NotFound).expect("可序列化"),
            json!("not_found")
        );
        for (raw, want) in [
            (r#""published""#, PublishBlogPostOutcome::Published),
            (r#""not_found""#, PublishBlogPostOutcome::NotFound),
        ] {
            let parsed: PublishBlogPostOutcome =
                serde_json::from_str(raw).expect("线格式应能解回枚举");
            assert_eq!(parsed, want, "解回值应逐字对应：{raw}");
        }
        // 大写首字母 / camelCase / 对象包裹 / 兄弟枚举的取值：一律不许被静默接受。
        for bad in
            [r#""Published""#, r#""notFound""#, r#"{"state":"published"}"#, r#""removed""#, "null"]
        {
            assert!(
                serde_json::from_str::<PublishBlogPostOutcome>(bad).is_err(),
                "非法线格式必须报错：{bad}"
            );
        }
    }

    /// 两个状态必须可判等、可 Copy：分派层与 provider 各自都要「取一次、比一次」，
    /// 不是移动语义的话会在两处之间来回 clone（同一读数的两份拷贝是重复定义的开端）。
    #[test]
    fn publish_outcome_is_copy_and_comparable() {
        let outcome = PublishBlogPostOutcome::Published;
        let copied = outcome;
        assert_eq!(outcome, copied);
        assert_ne!(outcome, PublishBlogPostOutcome::NotFound);
        assert_ne!(PublishBlogPostOutcome::Published, PublishBlogPostOutcome::NotFound);
    }

    // ── 本批新增：`delete_blog_post` / `create_blog_post` 的形态锁 ─────────────

    /// 默认体：与 publish 同款 `Err(Provider)`，且文案必须点明 trait + 方法。
    ///
    /// 落点分层判据同 publish：这是**契约缺省**，不是入参非法 ⇒ 分派层落
    /// `PLUGIN_CALL_FAILED`，不许升成参数错误，也不许被伪装成 `"not_found"`。
    #[test]
    fn delete_and_create_default_methods_are_unimplemented() {
        let provider = BareStore;
        let err = futures::executor::block_on(provider.delete_blog_post("post-1".to_string()))
            .expect_err("默认实现必须是错误");
        match err {
            AxAgentError::Provider(message) => {
                assert!(
                    message.contains("OpcStoreProvider::delete_blog_post"),
                    "文案须点明 trait 与方法：{message}"
                );
            },
            other => panic!("默认实现应返回 AxAgentError::Provider，实际 {other:?}"),
        }

        let err = futures::executor::block_on(provider.create_blog_post(sample_input()))
            .expect_err("默认实现必须是错误");
        match err {
            AxAgentError::Provider(message) => {
                assert!(
                    message.contains("OpcStoreProvider::create_blog_post"),
                    "文案须点明 trait 与方法：{message}"
                );
            },
            other => panic!("默认实现应返回 AxAgentError::Provider，实际 {other:?}"),
        }
    }

    fn sample_input() -> CreateBlogPostInput {
        CreateBlogPostInput {
            title: "OPC 的第一篇博客".into(),
            slug: "opc-first-post".into(),
            excerpt: "摘要若干字".into(),
            content: "正文".into(),
            tags: vec!["opc".into(), "blog".into()],
        }
    }

    /// 线格式逐字钉住：三个 snake_case 串（与 `jobs.store` 的 `remove` 同一套取值）。
    ///
    /// 取值漂移会让插件侧「删掉了 / 本来没有 / 不许删」的判别静默失效（禁区 13：接缝出参
    /// 就这一套名）。兄弟枚举的取值（`"published"` / `"removed"` 混用）一律不许被接受。
    #[test]
    fn remove_outcome_wire_format_is_three_snake_case_strings() {
        assert_eq!(
            serde_json::to_value(RemoveBlogPostOutcome::Removed).expect("可序列化"),
            json!("removed")
        );
        assert_eq!(
            serde_json::to_value(RemoveBlogPostOutcome::NotFound).expect("可序列化"),
            json!("not_found")
        );
        assert_eq!(
            serde_json::to_value(RemoveBlogPostOutcome::Protected).expect("可序列化"),
            json!("protected")
        );
        for (raw, want) in [
            (r#""removed""#, RemoveBlogPostOutcome::Removed),
            (r#""not_found""#, RemoveBlogPostOutcome::NotFound),
            (r#""protected""#, RemoveBlogPostOutcome::Protected),
        ] {
            let parsed: RemoveBlogPostOutcome =
                serde_json::from_str(raw).expect("线格式应能解回枚举");
            assert_eq!(parsed, want, "解回值应逐字对应：{raw}");
        }
        for bad in
            [r#""Removed""#, r#""notFound""#, r#"{"state":"removed"}"#, r#""published""#, "null"]
        {
            assert!(
                serde_json::from_str::<RemoveBlogPostOutcome>(bad).is_err(),
                "非法线格式必须报错：{bad}"
            );
        }
        // 三态两两可判别（分派层与 provider 都要「取一次、比一次」）。
        assert_ne!(RemoveBlogPostOutcome::Removed, RemoveBlogPostOutcome::NotFound);
        assert_ne!(RemoveBlogPostOutcome::Removed, RemoveBlogPostOutcome::Protected);
        assert_ne!(RemoveBlogPostOutcome::NotFound, RemoveBlogPostOutcome::Protected);
    }

    /// 入参 DTO 的线格式：五个 camelCase 键（本 DTO 五个字段名都是单词，camelCase 与
    /// snake_case 同形 ⇒ **上收不改载荷**），且没有 `content` 之外的派生键。
    #[test]
    fn create_input_key_set_is_exactly_five_words() {
        let val = serde_json::to_value(sample_input()).expect("序列化应成功");
        expect_keys(&val, &["title", "slug", "excerpt", "content", "tags"]);
        let back: CreateBlogPostInput = serde_json::from_value(val).expect("往返应成立");
        assert_eq!(back.tags, vec!["opc".to_string(), "blog".to_string()]);
        assert_eq!(back.content, "正文");
    }

    /// 字节预算住在契约层：值就是 64 KiB，按 **UTF-8 字节** 而不是字符计。
    ///
    /// 本用例只钉常量与「中文字符数 ≪ 字节数」这条量纲事实 —— 真正的边界判定
    /// （`±1` 字节）在宿主 provider 侧（`init/opc_store_service.rs` 的
    /// `check_content_budget`），那里才拿得到 `Err(Validation)`。
    #[test]
    fn content_budget_is_64_kib_and_measured_in_bytes() {
        assert_eq!(OPC_BLOG_POST_CONTENT_MAX_BYTES, 64 * 1024);
        // 一个中文字 = 3 字节（UTF-8）⇒ 字符数与字节数**不是**同一个数。
        let cjk = "文";
        assert_eq!(cjk.len(), 3, "中文字符按 3 字节计");
        assert_eq!(cjk.chars().count(), 1);
        // 预算下界必须是正数：0 会把所有正文都拒掉，等于把写面悄悄关死。
        // ⚠ 这一条由常量本身保证，不写成 `assert!(CONST > 0)`（会撞 clippy::assertions_on_constants）。
    }

    /// 八个只读视图的线格式键集合（camelCase，禁区 13）与「大字段不过线」一起钉死。
    #[test]
    fn readonly_view_key_sets_are_camel_case_and_cropped() {
        let invoice = InvoiceSummaryView {
            id: "inv-1".into(),
            customer_id: "cus-1".into(),
            invoice_number: "INV-001".into(),
            status: "paid".into(),
            total: 100.0,
            currency: "CNY".into(),
            created_at: 1,
        };
        expect_keys(
            &serde_json::to_value(&invoice).expect("可序列化"),
            &["id", "customerId", "invoiceNumber", "status", "total", "currency", "createdAt"],
        );

        let customer = CustomerSummaryView {
            id: "cus-1".into(),
            name: "张三".into(),
            email: "a@b.c".into(),
            phone: None,
            company: Some("ACME".into()),
            status: "active".into(),
            total_revenue: 10.0,
            created_at: 1,
        };
        expect_keys(
            &serde_json::to_value(&customer).expect("可序列化"),
            &["id", "name", "email", "phone", "company", "status", "totalRevenue", "createdAt"],
        );

        let project = ProjectSummaryView {
            id: "prj-1".into(),
            title: "项目".into(),
            status: "active".into(),
            budget: None,
            currency: "CNY".into(),
            milestone_total: 3,
            milestone_completed: 1,
            created_at: 1,
        };
        expect_keys(
            &serde_json::to_value(&project).expect("可序列化"),
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

        let landing = LandingPageSummaryView {
            id: "lp-1".into(),
            title: "页".into(),
            slug: "page".into(),
            published: false,
            created_at: 1,
        };
        expect_keys(
            &serde_json::to_value(&landing).expect("可序列化"),
            &["id", "title", "slug", "published", "createdAt"],
        );

        let contact = ContactSummaryView {
            id: "ct-1".into(),
            name: "李四".into(),
            email: "x@y.z".into(),
            source: "website".into(),
            read: false,
            created_at: 1,
        };
        // `message` 不过线（列裁剪）。
        expect_keys(
            &serde_json::to_value(&contact).expect("可序列化"),
            &["id", "name", "email", "source", "read", "createdAt"],
        );

        let kpi = KpiSummaryView {
            id: "kpi-1".into(),
            domain_pack_id: "sales_growth".into(),
            name: "营收".into(),
            value: 1.0,
            unit: "CNY".into(),
            period: "2026-11".into(),
            recorded_at: 1,
        };
        expect_keys(
            &serde_json::to_value(&kpi).expect("可序列化"),
            &["id", "domainPackId", "name", "value", "unit", "period", "recordedAt"],
        );

        let asset = ContentAssetSummaryView {
            id: "ca-1".into(),
            title: "资产".into(),
            content_type: "article".into(),
            status: "draft".into(),
            tags: vec!["opc".into()],
            created_at: 1,
        };
        // `body` 不过线（列裁剪）。
        expect_keys(
            &serde_json::to_value(&asset).expect("可序列化"),
            &["id", "title", "contentType", "status", "tags", "createdAt"],
        );

        let schedule = PublishScheduleSummaryView {
            id: "ps-1".into(),
            content_ref_type: "blog_post".into(),
            content_ref_id: "post-1".into(),
            status: "pending".into(),
            scheduled_at: 1,
            published_at: None,
        };
        expect_keys(
            &serde_json::to_value(&schedule).expect("可序列化"),
            &["id", "contentRefType", "contentRefId", "status", "scheduledAt", "publishedAt"],
        );
    }

    /// 查询 DTO 的线格式：camelCase 键可解，缺键走 `Default`（`#[serde(default)]`）。
    #[test]
    fn list_query_dtos_are_camel_case_and_defaultable() {
        let invoice: InvoiceListQuery = serde_json::from_value(json!({
            "customerId": "cus-1",
            "dateFrom": 10,
            "limit": 5
        }))
        .expect("camelCase 键应可解");
        assert_eq!(invoice.customer_id.as_deref(), Some("cus-1"));
        assert_eq!(invoice.date_from, Some(10));
        assert_eq!(invoice.limit, Some(5));
        assert_eq!(invoice.offset, None);

        let empty: CustomerListQuery = serde_json::from_str("{}").expect("空对象走 Default");
        assert!(empty.status.is_none() && empty.customer_type.is_none());

        let project: ProjectListQuery = serde_json::from_value(json!({
            "customerId": "cus-2",
            "search": "网站"
        }))
        .expect("camelCase 键应可解");
        assert_eq!(project.customer_id.as_deref(), Some("cus-2"));

        let kpi: KpiListQuery =
            serde_json::from_value(json!({ "period": "2026-11", "limit": 20 })).expect("键应可解");
        assert_eq!(kpi.period.as_deref(), Some("2026-11"));
        assert_eq!(kpi.limit, Some(20));
    }
}
