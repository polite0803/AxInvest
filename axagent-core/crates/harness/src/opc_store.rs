// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 站点内容只读接缝（`opc.store`）— DTO + Trait。
//!
//! 宿主把 `opc_blog_posts` 表的**列表读面**经本接缝暴露给插件侧；插件经
//! `call_seam("opc.store", "list_blog_posts", { limit?, offset? })` 按页读取。首批
//! **只有 `list_blog_posts` 一个 op，且是读面**（无写 op、无权限闸）。
//!
//! ## 为什么需要它
//!
//! OPC 域插件（`plugin-opc`）不持 `DatabaseConnection` —— 落库读写一律经宿主接缝
//! （该 crate 的 `Cargo.toml` 依赖规则原文）。而它替代的宿主命令 `opc_list_blog_posts`
//! 走的是 `DefaultSiteService::list_blog_posts`（`axagent-quant/crates/company-runtime/src/opc/site.rs`），
//! 那条服务在**实现层**、不在契约层，插件进程无法链接它 ⇒ 必须有一条 IPC 形态的读面。
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
/// 时间口径只有一个单位：epoch **毫秒**（`i64`），与宿主列 `created_at` / `updated_at` /
/// `published_at` 同单位，不做二次换算，也不发 `chrono` 字符串。
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
    /// 创建时刻（epoch 毫秒）。
    pub created_at: i64,
    /// 最近一次写库时刻（epoch 毫秒）。
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

/// OPC 站点内容提供者（宿主独占 `opc_*` 表读面，首批只有 `list_blog_posts`）。
///
/// 实现方：宿主 wiring 层（`src-tauri/src/init/opc_store_service.rs`，只调 dao，不直连库）。
/// 默认实现返回「该提供者不支持此数据面」，便于外部实现只覆盖关心的 op
/// （与 `AnalysisRecordProvider` / `TrajectoryStoreProvider` 同款缺省）。
///
/// 首批无写 op ⇒ 本 trait 没有需要 `write` 权限的方法，`worker.rs` 的
/// `required_permission_for` 也**一条都不许加**（多登记一条会让只读插件被无端拒掉）。
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

    /// 时间只有 epoch 毫秒一个单位：不要出 `Date` 字符串、也不要把毫秒当秒。
    #[test]
    fn blog_post_view_time_is_epoch_millis_only() {
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
}
