// SPDX-License-Identifier: AGPL-3.0-only

//! `opc_blog_posts` 的**列表读面 + 发布写面** —— SQL 边界的唯一实现。
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
//!
//! ## 写面的单位口径：`set_blog_post_published` 落 epoch **秒**
//!
//! 三个 `Set` 逐字段抄现写体 `axagent-quant/crates/company-runtime/src/opc/site.rs` 的
//! `publish_blog_post`（同一条 SQL 位点；迁出中途两条写通道并存的裁定见 #144 那条先例）：
//! 时刻取 `axagent_harness::util_fns::now_ts()`，其函数体是
//! `chrono::Utc::now().timestamp()` ⇒ **秒**，不是 `now_ms()`（那个是运行时侧的毫秒统一）。
//!
//! 判据不是审美而是**同列同单位**：`company-runtime` 的 `data_service` 按
//! `published_at BETWEEN from AND to` 数「本周期发布了几篇」，而 `from` / `to` 由 `TimeRange`
//! 给出、同为秒（该处注释已逐字核对过）。这里写成毫秒 ⇒ 新发布的行永远落在窗口之外，
//! 看板上的发布篇数**静默少一篇**（没有报错，只有数不对）。

use axagent_entities::opc_blog_posts;
use axagent_harness::core_error::Result;
use axagent_harness::util_fns::now_ts;
use sea_orm::{
    ActiveModelTrait, DatabaseConnection, EntityTrait, FromQueryResult, PaginatorTrait, QueryOrder,
    QuerySelect, Set,
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

/// 发布（写面）：把 `id` 那行置为已发布 —— 只 `Set` 三列，**既不读也不写 `content`**。
///
/// 返回值是**可判别结果**而不是错误：`true` = 命中并已 UPDATE；`false` = `id` 不在表里，
/// 一条 UPDATE 都不发（先 `find_by_id` 再改，正是为了「缺行不发写」这条）。接缝侧落成
/// [`axagent_harness::PublishBlogPostOutcome::NotFound`]。
///
/// ⚠ 与同族删除路径的对照：`src-tauri/src/commands/opc.rs` 里那两条裸
/// `Entity::delete_by_id(..).exec(..)` 不检 `rows_affected` ⇒ 删不存在的 id 也回成功，
/// 与本函数刻意做的「缺行可判别」正好相反 —— 那两条属收口项（PLAN §115.2 已记），不属本批。
///
/// 不做「已发布则跳过」：现写体就是无条件 `Set`，重复发布会把 `published_at` / `updated_at`
/// 刷成本次时刻。两条写通道（本接缝与 `site.rs` 的 service 方法）必须逐字同行为，
/// 否则「这次发布走的是哪条路」会改变读数。
pub async fn set_blog_post_published(db: &DatabaseConnection, id: &str) -> Result<bool> {
    let Some(entity) = opc_blog_posts::Entity::find_by_id(id).one(db).await? else {
        return Ok(false);
    };
    // 单位：epoch 秒（判据见模块头「写面的单位口径」一节）。
    let now = now_ts();
    let mut am: opc_blog_posts::ActiveModel = entity.into();
    am.published = Set(1);
    am.published_at = Set(Some(now));
    am.updated_at = Set(now);
    am.update(db).await?;
    Ok(true)
}

// ── 测试 ────────────────────────────────────────────────────────────────
//
// 内联测试 mod 追加在文件末尾（`items_after_test_module` 只在 clippy 下暴露）。
//
// 三件纯函数事（行收敛：哨兵列 → bool、`tags_json` → 数组）+ **一条真库端到端**：
// 写面 [`set_blog_post_published`] 的命中 / 缺行两态，以及「落库时刻是秒级量级」这条单位判据。
//
// 真库为什么必须开：单位抄错在类型层面完全合法（两侧都是 `i64`），编译器与序列化都抓不到，
// 而它的代价是 KPI 窗口静默少一篇（判据见模块头）—— 只有把行真写进 `opc_blog_posts`
// 再读回来，才量得出 `published_at` 到底是秒还是毫秒。建表走 `create_test_pool` 的
// 生产同一条 `initialize_schema`，不手连 `sqlite::memory:`（池 >1 时每连接各一个内存库，
// 建表与写入会落在不同库上 —— 该坑的原文在 `repo/opc_demand.rs` 的同名用例里）。

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

    /// 真库端到端：发布写面的**命中 / 缺行**两态 + 「只动三列」+ **秒级量级**的单位判据。
    ///
    /// 四件事各挡一类静默缺陷：
    /// - 缺行返回 `false` 且**整表行数不变** —— 挡「不检结果就回成功」（同族删除路径的既存债）；
    /// - `content` / `slug` / `created_at` 读回原值 —— 挡「顺手把整行 ActiveModel 重写一遍」
    ///   把正文列或对外路由键改掉；
    /// - `published_at` 落在秒级区间 —— 毫秒（≥1e12）会直接越界，这正是 `data_service`
    ///   的 KPI 窗口静默少一篇的成因；
    /// - 重复发布仍然 `true` —— 挡「已发布就跳过」这种把同一 op 变成两种读数的改写。
    #[tokio::test]
    async fn set_blog_post_published_hits_and_misses_against_real_db() {
        let handle = crate::db::create_test_pool().await.expect("测试库应能建起来");
        let db = handle.conn.clone();
        // 预置一条草稿行：正文列给非空值，才好验证「写面不碰正文」；created_at 用历史秒值。
        let seeded = 1_700_000_000_i64;
        let draft = opc_blog_posts::ActiveModel {
            id: Set("post-publish-e2e".to_string()),
            title: Set("端到端草稿".to_string()),
            slug: Set("publish-e2e".to_string()),
            excerpt: Set("摘要".to_string()),
            content: Set("正文正文正文".to_string()),
            tags_json: Set(r#"["opc"]"#.to_string()),
            published: Set(0),
            published_at: Set(None),
            view_count: Set(0),
            created_at: Set(seeded),
            updated_at: Set(seeded),
        };
        draft.insert(&db).await.expect("预置草稿行应能插入");

        // ① 命中：回 true，且只有那三列变了。
        assert!(
            set_blog_post_published(&db, "post-publish-e2e").await.unwrap(),
            "命中已存在的 id 必须回 true"
        );
        let row = opc_blog_posts::Entity::find_by_id("post-publish-e2e")
            .one(&db)
            .await
            .unwrap()
            .expect("命中路径之后行必须还在");
        assert_eq!(row.published, 1, "published 哨兵必须置 1");
        assert!(row.published_at.is_some(), "发布动作必须写 published_at（草稿态本是 NULL）");
        assert!(row.updated_at > seeded, "updated_at 必须被刷成本次时刻，实际 {}", row.updated_at);
        assert_eq!(row.content, "正文正文正文", "写面不得碰正文列");
        assert_eq!(row.slug, "publish-e2e", "slug 是对外路由键（表上 unique），发布不改它");
        assert_eq!(row.created_at, seeded, "created_at 不归本 op 管");
        assert_eq!(row.title, "端到端草稿", "标题原样");

        // ② 单位口径：秒级量级。当前 epoch 秒 ~1.7e9，毫秒 ~1.7e12 ⇒ 上界 1e11 能机械区分。
        let now_seconds = row.published_at.expect("已发布必有发布时刻");
        assert!(
            (1_000_000_000..100_000_000_000).contains(&now_seconds),
            "published_at 必须是 epoch **秒**（毫秒量级 ≥1e12 会越出本区间）：{now_seconds}"
        );
        assert_eq!(
            row.updated_at, now_seconds,
            "两列必须同一个 now（本函数只取一次时钟；现写体是两次 now_ts()，秒粒度下跨秒会不等）"
        );

        // ③ 缺行：回 false，且**一条 UPDATE / INSERT 都不发**（整表行数仍为 1）。
        assert!(
            !set_blog_post_published(&db, "post-does-not-exist").await.unwrap(),
            "缺行必须回 false（接缝侧落成 NotFound，不是错误码）"
        );
        assert_eq!(
            opc_blog_posts::Entity::find().count(&db).await.unwrap(),
            1,
            "缺行路径不得写入任何行"
        );

        // ④ 重复发布：仍然 true，且时刻再次落到「本次」（现写体是无条件 Set，不跳过）。
        assert!(
            set_blog_post_published(&db, "post-publish-e2e").await.unwrap(),
            "重复发布仍须 true"
        );
    }
}
