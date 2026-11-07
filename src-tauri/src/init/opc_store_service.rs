// SPDX-License-Identifier: AGPL-3.0-only

//! `opc.store` 接缝的**宿主侧实现**（wiring 层，只调 dao，不直连库）。
//!
//! 契约在 `axagent-core/crates/harness/src/opc_store.rs`，现下四个 op：读面
//! `list_blog_posts`，写面 `publish_blog_post` / `delete_blog_post`（三态）/
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
//! - `id`（三个写 op 的 publish / delete）：空 / 全空白 ⇒ `Err(Validation)`；缺 id 属入参非法，
//!   与「id 合法但表里没有这篇」是两件事 —— 后者回 `NotFound` 正常值。
//! - `content`（create）：超 [`OPC_BLOG_POST_CONTENT_MAX_BYTES`]（**UTF-8 字节**）⇒
//!   `Err(Validation)`，不截断；`slug` 空 / 全空白同样拒（它是对外路由键，表上 `unique`）。
//!
//! 四类闸都在 IO **之前**：非法参数不该换来一次主键查询 / INSERT。

use std::collections::HashSet;

use async_trait::async_trait;
use axagent_company_runtime::opc::{DefaultSiteService, SiteService};
use axagent_dao::repo::opc_blog_post::{BlogPostListRow, count_blog_posts, list_blog_posts};
use axagent_harness::DatabaseConnection;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::opc_store::{
    BlogPostListOutcome, BlogPostView, CreateBlogPostInput, OPC_BLOG_POST_CONTENT_MAX_BYTES,
    OPC_BLOG_POST_LIST_DEFAULT_LIMIT, OPC_BLOG_POST_LIST_LIMIT_RANGE, OpcStoreProvider,
    PublishBlogPostOutcome, RemoveBlogPostOutcome,
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
        require_blog_post_id(&id, "publish_blog_post")?;
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
        require_blog_post_id(&id, "delete_blog_post")?;
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
}

/// 写 op 的 `id` 闸：空 / 全空白 ⇒ `Err(Validation)`（`op` 只用于文案定位）。
///
/// 为什么在 provider 而不是分派层：读 op 的两个可选参数闸也在 provider（分派层
/// 「只透传契约 DTO、一个数字都不写」的分工）。两处各判一半 ⇒ 迟早有一处漏。
fn require_blog_post_id(id: &str, op: &str) -> Result<()> {
    if id.trim().is_empty() {
        return Err(AxAgentError::Validation(format!(
            "opc.store::{op} 的 id 不能为空（缺 id 属入参非法，不是「没有这篇」）"
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
                let err = require_blog_post_id(bad, op).expect_err("空白 id 必须拒收");
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
        assert!(
            require_blog_post_id(" post-1 ", "delete_blog_post").is_ok(),
            "非空白即放行，本层不改写 id"
        );
        assert!(require_blog_post_id("post-1", "delete_blog_post").is_ok());
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
}
