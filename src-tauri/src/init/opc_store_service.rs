// SPDX-License-Identifier: AGPL-3.0-only

//! `opc.store` 接缝的**宿主侧实现**（wiring 层，只调 dao，不直连库）。
//!
//! 契约在 `axagent-core/crates/harness/src/opc_store.rs`，现下两个 op：读面
//! `list_blog_posts` + 写面 `publish_blog_post`。本文件只做两件事：入参闸（`limit` /
//! `offset` / `id`）与「dao 行 → 契约视图」的逐字段映射（写面没有视图，只有两态枚举）。
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
//! [`blog_post_view`] 是唯一映射点：`content`（博客正文）在 **SQL 侧**就没被 SELECT，
//! 到这里的行本来就没有该列 ⇒ 不存在「进了宿主内存但没进帧」的中间态。
//! `published` 哨兵收敛与 `tags_json` 解码同样在 dao 做过，本层不再补第二次。
//!
//! ## 闸（越界拒收，不静默夹取）
//!
//! - `limit`：`None` ⇒ [`OPC_BLOG_POST_LIST_DEFAULT_LIMIT`]；显式值越出
//!   [`OPC_BLOG_POST_LIST_LIMIT_RANGE`] ⇒ `Err(Validation)`。插件侧的 `limit` 参与
//!   「取尽整表」的分页算术，夹取后「我给 500 却只回 200」会让它把下一页 offset 算错
//!   （同 `analysis_record_service.rs` 的 `resolve_limit` 判据）；
//! - `offset`：`None` ⇒ `0`；无上限（越界 offset 就是一次空页，`total` 照常返回）。
//! - `id`（写 op）：空 / 全空白 ⇒ `Err(Validation)`；缺 id 属入参非法，与「id 合法但表里没有这篇」
//!   是两件事 —— 后者回 `NotFound` 正常值。三处闸都在 IO **之前**：非法参数不该换来一次主键查询。

use async_trait::async_trait;
use axagent_dao::repo::opc_blog_post::{BlogPostListRow, count_blog_posts, list_blog_posts};
use axagent_harness::DatabaseConnection;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::opc_store::{
    BlogPostListOutcome, BlogPostView, OPC_BLOG_POST_LIST_DEFAULT_LIMIT,
    OPC_BLOG_POST_LIST_LIMIT_RANGE, OpcStoreProvider, PublishBlogPostOutcome,
};

/// `opc.store` 的宿主实现。
pub struct HostOpcStore {
    /// 与 `AppState.harness.db()` **同一个**连接池句柄（`DatabaseConnection` 是 Clone 的句柄，
    /// 不是第二套连接语义）。
    db: DatabaseConnection,
}

impl HostOpcStore {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
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
        require_publish_id(&id)?;
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
}

/// 写 op 的入参闸：`id` 空 / 全空白 ⇒ `Err(Validation)`。
///
/// 为什么在 provider 而不是分派层：读 op 的两个可选参数闸也在 provider（分派层
/// 「只透传契约 DTO、一个数字都不写」的分工）。两处各判一半 ⇒ 迟早有一处漏。
fn require_publish_id(id: &str) -> Result<()> {
    if id.trim().is_empty() {
        return Err(AxAgentError::Validation(
            "opc.store::publish_blog_post 的 id 不能为空（缺 id 属入参非法，不是「没有这篇」）"
                .into(),
        ));
    }
    Ok(())
}

// ── 测试 ──────────────────────────────────────────────────────────────────
//
// 内联测试 mod 追加在文件末尾（`items_after_test_module` 只在 clippy 下暴露）。
//
// 只测本文件自己的三件事：`limit` 闸、`id` 闸、映射键集合。dao 的行收敛（哨兵 / `tags_json`）
// 与**发布写面的真库端到端**（命中 / 缺行 / 秒级量级）都在 `repo/opc_blog_post.rs` 测 ——
// 本层不重复开库，只保证「闸 + 翻译」这两件本层职责不静默漂移。

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
    #[test]
    fn publish_id_gate_rejects_blank_but_does_not_trim() {
        for bad in ["", " ", "  ", "\t", "\n", "\u{00a0}"] {
            let err = require_publish_id(bad).expect_err("空白 id 必须拒收");
            assert!(matches!(err, AxAgentError::Validation(_)), "应落 Validation：{err:?}");
        }
        // 只判「有没有内容」，不替调用方 trim 出第二个值：带前后空格的 id 原样交给 dao。
        assert!(require_publish_id(" post-1 ").is_ok(), "非空白即放行，本层不改写 id");
        assert!(require_publish_id("post-1").is_ok());
    }
}
