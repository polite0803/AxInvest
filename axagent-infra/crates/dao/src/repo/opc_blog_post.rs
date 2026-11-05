// SPDX-License-Identifier: AGPL-3.0-only

//! `opc_blog_posts` 的**列表读面** —— SQL 边界的唯一实现。
//!
//! ## 为什么在 dao 而不是宿主 provider 里直接查
//!
//! `opc.store` 接缝的宿主 provider 落在 `src-tauri/src/init/opc_store_service.rs`，但
//! 「取哪几列 / 按什么排 / 取多少行」是**存储契约**，不是装配逻辑：写在 provider 里就等于
//! 让 wiring 层持有第二份列清单（本仓的分层纪律是「复杂查询下沉 dao，provider 只调 dao 函数」，
//! 同 `repo/stock_analysis_rerun_queue.rs`、`repo/trade.rs`（#114）的做法）。
//!
//! ## 列裁剪做在 SQL 侧
//!
//! 实体是宽表：`content` 是 `column_type = "Text"` 的**博客正文**列。列表消费方
//! （OPC 站点页 / 仪表盘 KPI）不读正文，接缝视图也不带它 ⇒ 这里用 `select_only()` 逐列
//! 点名，**不 `Entity::find()` 取整行**。先例理由同 `stock_analysis_rerun_queue.rs` 的
//! 「绝不能拉整行」：多拉一列大列就是纯浪费，且随历史积累线性恶化。
//!
//! ## 排序是本模块的契约，不是调用方的偏好
//!
//! `(created_at DESC, id ASC)`。`id` 这一档**不是装饰**：接缝按 `LIMIT/OFFSET` 分页，
//! 而 `created_at` 会撞（批量灌入的行常同一毫秒），无第二排序键时 SQLite 对同值行的返回
//! 顺序不保证稳定 ⇒ 同一 offset 在两次取页之间可能跳到另一行，表现为「插件取尽整表」时
//! 重复行或漏行。全键有序（`id` 是主键）⇒ 分页结果确定。
//!
//! 主序仍与它替代的宿主查询（`company-runtime` 的 `DefaultSiteService::list_blog_posts`）
//! 逐字一致：`created_at` 降序 = 最新在前。

use axagent_entities::opc_blog_posts;
use axagent_harness::core_error::Result;
use sea_orm::{
    DatabaseConnection, EntityTrait, FromQueryResult, PaginatorTrait, QueryOrder, QuerySelect,
};

/// 列表读面的一行 —— 接缝视图需要的**全部**列，刻意不含 `content`。
///
/// 刻意不给 `opc_blog_posts::Model`：那等于把实体类型重新漏回上层，下一个调用方就会
/// 顺手去读正文列（禁区 12 的「视图裁剪点唯一」在类型层面就没有防线了）。
///
/// `tags` 在这里解码：`tags_json` 是**落库形态**（Text 列存 JSON 数组串），解码属存储知识
/// （同 `opc_delivery.rs` 的 `invoice_from_entity` 那一档职责）。解码失败按空数组处理，
/// **不**报错 —— 与被它替代的宿主 DTO 转换（`company-runtime` 的 `blog_entity_to_dto`）
/// 同一口径；那条转换仍服务未迁出的写命令，两处解码的收敛属 OPC 整域迁出批，不属本批。
///
/// ⚠ 刻意**不**derive `FromQueryResult`：那一格只长在 [`BlogPostListRaw`] 上（列类型与
/// 字段一一对应才解得了），本结构带 `Vec<String>` / `bool`，不是「一行 SQL」的形状。
#[derive(Debug, Clone)]
pub struct BlogPostListRow {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub excerpt: String,
    /// `tags_json` 的解码结果（畸形 / 空 ⇒ 空数组）。
    pub tags: Vec<String>,
    /// 落库是 `i32` 哨兵列，这里按「非 0 即真」收敛。
    pub published: bool,
    pub published_at: Option<i64>,
    pub view_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 中间形态：`FromQueryResult` 只能按**列名**落字段，`published` / `tags` 的收敛在它之后做。
#[derive(Debug, FromQueryResult)]
struct BlogPostListRaw {
    id: String,
    title: String,
    slug: String,
    excerpt: String,
    tags_json: String,
    published: i32,
    published_at: Option<i64>,
    view_count: i64,
    created_at: i64,
    updated_at: i64,
}

impl From<BlogPostListRaw> for BlogPostListRow {
    fn from(raw: BlogPostListRaw) -> Self {
        Self {
            id: raw.id,
            title: raw.title,
            slug: raw.slug,
            excerpt: raw.excerpt,
            tags: serde_json::from_str(&raw.tags_json).unwrap_or_default(),
            published: raw.published != 0,
            published_at: raw.published_at,
            view_count: raw.view_count,
            created_at: raw.created_at,
            updated_at: raw.updated_at,
        }
    }
}

/// 列表：按 `(created_at DESC, id ASC)` 取一页裁剪行。
///
/// `limit` / `offset` 已由调用方（接缝 provider）按契约区间校验过；这里**不再夹取**，
/// 也不设第二套上限数字 —— 闸住一次就够，两处夹取必然漂移（同 `analysis_record_service.rs`
/// 的 `resolve_limit` 判据）。
pub async fn list_blog_posts(
    db: &DatabaseConnection,
    limit: usize,
    offset: usize,
) -> Result<Vec<BlogPostListRow>> {
    let rows = opc_blog_posts::Entity::find()
        .select_only()
        .column(opc_blog_posts::Column::Id)
        .column(opc_blog_posts::Column::Title)
        .column(opc_blog_posts::Column::Slug)
        .column(opc_blog_posts::Column::Excerpt)
        .column(opc_blog_posts::Column::TagsJson)
        .column(opc_blog_posts::Column::Published)
        .column(opc_blog_posts::Column::PublishedAt)
        .column(opc_blog_posts::Column::ViewCount)
        .column(opc_blog_posts::Column::CreatedAt)
        .column(opc_blog_posts::Column::UpdatedAt)
        .order_by_desc(opc_blog_posts::Column::CreatedAt)
        .order_by_asc(opc_blog_posts::Column::Id)
        .limit(limit as u64)
        .offset(offset as u64)
        .into_model::<BlogPostListRaw>()
        .all(db)
        .await?;
    Ok(rows.into_iter().map(BlogPostListRow::from).collect())
}

/// 整表行数（接缝出参里 `total` 的唯一来源）。
///
/// 不能拿「本页 `len()`」顶：那会把「最后一页不满」读成「总共就这么多」，
/// 而插件的取尽循环正是按 `total` 判断何时停的。
pub async fn count_blog_posts(db: &DatabaseConnection) -> Result<u64> {
    Ok(opc_blog_posts::Entity::find().count(db).await?)
}

// ── 测试 ────────────────────────────────────────────────────────────────
//
// 内联测试 mod 追加在文件末尾（`items_after_test_module` 只在 clippy 下暴露）。
//
// 这里只测本文件自己的两件纯函数事：行收敛（哨兵列 → bool、`tags_json` → 数组）。
// 真库端到端（列裁剪是否真的没 SELECT `content`、`ORDER BY id` 是否落进 SQL）属
// 需要预置 `opc_blog_posts` 行的跨批项，本批不开。

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_row() -> BlogPostListRaw {
        BlogPostListRaw {
            id: "post-1".into(),
            title: "第一篇".into(),
            slug: "first-post".into(),
            excerpt: "摘要".into(),
            tags_json: r#"["opc","blog"]"#.into(),
            published: 1,
            published_at: Some(1_700_000_000_000),
            view_count: 42,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_060_000,
        }
    }

    #[test]
    fn row_converges_sentinel_and_tags_json() {
        let row = BlogPostListRow::from(raw_row());
        assert!(row.published, "非 0 哨兵 ⇒ true");
        assert_eq!(row.tags, vec!["opc".to_string(), "blog".to_string()]);
        assert_eq!(row.view_count, 42);
        assert_eq!(row.published_at, Some(1_700_000_000_000));
    }

    /// 哨兵与时刻是两个独立列：历史行可以「标志已改回草稿但时刻还留着」，这里都不加工，
    /// 原样上抛（消费方判「是否发布」只认 `published`，判据见契约模块头）。
    #[test]
    fn draft_sentinel_zero_is_false_and_published_at_is_not_rewritten() {
        let mut raw = raw_row();
        raw.published = 0;
        let row = BlogPostListRow::from(raw);
        assert!(!row.published, "0 ⇒ false（草稿）");
        assert_eq!(row.published_at, Some(1_700_000_000_000), "不替调用方清时刻列");
    }

    /// 畸形 / 空 `tags_json` ⇒ 空数组，**不是**错误（与被替代的宿主 DTO 转换同口径）。
    #[test]
    fn malformed_tags_json_degrades_to_empty_vec() {
        for bad in ["", "not-json", "{}", "null", "[1,2"] {
            let mut raw = raw_row();
            raw.tags_json = bad.into();
            let row = BlogPostListRow::from(raw);
            assert!(row.tags.is_empty(), "`tags_json = {bad:?}` 应落空数组，实际 {:?}", row.tags);
        }
    }
}
