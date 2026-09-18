// SPDX-License-Identifier: AGPL-3.0-only

//! 能力域覆盖层 repository —— 「域」的用户可改子集（启用/停用 + 追加别名）持久化层。
//!
//! # 职责边界
//!
//! 本层只做两件事：**读写 `capability_domain_overrides` 表**，
//! 以及**把读到的全表灌进 harness 的运行时覆盖层**（[`load_into_runtime`]）。
//!
//! 它**不做**取值合法性校验（那是命令层的职责，见下）也不**不**碰内置声明
//! （`DOMAIN_NODES` 是编译期契约，覆盖层改不到）。
//!
//! # 为什么 `load_into_runtime` 对坏行「告警 + 跳过」而不是报错
//!
//! 覆盖层是**可选配置**：一行 `domain` 解析不出枚举（手工改库 / 旧版本残留）不应该
//! 让应用起不来 —— 那会把「一个域没生效」升级成「整个应用白屏」。
//! 但**也不能静默**：跳过时按行 `tracing::warn!` 打出原始值，并汇总条数，
//! 使「我的覆盖没生效」在日志里可见（判据：降级必须可见）。
//!
//! # 与 `capability_policy` 的区别
//!
//! 同目录的 `capability_policy` 是**排除型过滤器规则**（作用于候选能力列表）；
//! 本模块是**域自身**的启用状态与别名（作用于 L1 prompt / L1 路由 / 能力过滤闸门）。

use sea_orm::*;

use axagent_entities::capability_domain_overrides;
use axagent_harness::core_error::Result;
use axagent_harness::util_fns::now_ts;
use axagent_harness::{CapabilityDomain, DomainOverride, apply_domain_overrides};

/// 一行覆盖的**原始形态**（`domain` 仍是字符串，未解析成枚举）。
///
/// 刻意保留字符串形态：命令层需要把「库里有这行」与「这行解析得出来」两件事分开报告
/// （UI 要能显示「有一行坏数据」而不是假装它不存在）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainOverrideRow {
    pub domain: String,
    pub enabled: bool,
    pub extra_aliases: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 读**全表**（含被停用的行 —— 停用本身就是覆盖，必须读出来）。
pub async fn list_overrides(db: &DatabaseConnection) -> Result<Vec<DomainOverrideRow>> {
    let rows = capability_domain_overrides::Entity::find().all(db).await?;
    Ok(rows
        .into_iter()
        .map(|r| DomainOverrideRow {
            domain: r.domain,
            enabled: r.enabled,
            extra_aliases: decode_aliases(&r.extra_aliases),
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect())
}

/// 按域读一行覆盖（`None` = 该域无覆盖 = 走内置默认）。
pub async fn get_override(
    db: &DatabaseConnection,
    domain: &str,
) -> Result<Option<DomainOverrideRow>> {
    let row = capability_domain_overrides::Entity::find_by_id(domain).one(db).await?;
    Ok(row.map(|r| DomainOverrideRow {
        domain: r.domain,
        enabled: r.enabled,
        extra_aliases: decode_aliases(&r.extra_aliases),
        created_at: r.created_at,
        updated_at: r.updated_at,
    }))
}

/// 写入/更新一行覆盖（upsert，幂等）。
///
/// `created_at` 在冲突时不更新（保留首次写入时间），`updated_at` 每次刷新。
pub async fn upsert_override(
    db: &DatabaseConnection,
    domain: &str,
    enabled: bool,
    extra_aliases: &[String],
) -> Result<()> {
    let now = now_ts();
    let aliases_json = serde_json::to_string(extra_aliases).unwrap_or_else(|_| "[]".to_string());
    let am = capability_domain_overrides::ActiveModel {
        domain: Set(domain.to_string()),
        enabled: Set(enabled),
        extra_aliases: Set(aliases_json),
        created_at: Set(now),
        updated_at: Set(now),
    };
    let _ = capability_domain_overrides::Entity::insert(am)
        .on_conflict(
            sea_query::OnConflict::column(capability_domain_overrides::Column::Domain)
                .update_columns([
                    capability_domain_overrides::Column::Enabled,
                    capability_domain_overrides::Column::ExtraAliases,
                    capability_domain_overrides::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(db)
        .await?;
    Ok(())
}

/// 删除一行覆盖（该域回到**内置默认**）。
///
/// 返回是否真的删掉了一行：UI 用它区分「重置成功」与「本来就是默认值」。
pub async fn delete_override(db: &DatabaseConnection, domain: &str) -> Result<bool> {
    let res = capability_domain_overrides::Entity::delete_by_id(domain).exec(db).await?;
    Ok(res.rows_affected > 0)
}

/// 读全表并**解析**成覆盖项；返回 `(可用覆盖项, 无法解析的域字符串)`。
///
/// 拆成「原始 + 解析」两段是刻意的：命令层要能回答「有一行坏数据」，
/// 而不是假装它不存在（坏行不表现为「已覆盖」，但也不该无声无息）。
pub async fn list_parsed(db: &DatabaseConnection) -> Result<(Vec<DomainOverride>, Vec<String>)> {
    let rows = list_overrides(db).await?;
    let mut parsed = Vec::with_capacity(rows.len());
    let mut unknown = Vec::new();
    for row in rows {
        match row.domain.parse::<CapabilityDomain>() {
            Ok(domain) => parsed.push(DomainOverride::new(domain, row.enabled, row.extra_aliases)),
            Err(_) => unknown.push(row.domain),
        }
    }
    Ok((parsed, unknown))
}

/// 读全表并灌进 harness 运行时覆盖层；返回**成功应用**的条数。
///
/// 启动路径调用它一次（`init/database.rs` / `init/state.rs`），
/// 写命令在落库后调用它以让改动**立刻生效**（不必重启）。
///
/// ⚠ 这是**整表替换**语义（[`apply_domain_overrides`]）：即便全部行都是坏行，
/// 也会把覆盖层清空 → 回到内置默认，而不是保留上一次的旧覆盖。
/// 「读失败」与「读到一堆坏行」是两件事，前者返回 `Err` 且**不动**运行时状态。
pub async fn load_into_runtime(db: &DatabaseConnection) -> Result<usize> {
    let (applied, unknown) = list_parsed(db).await?;

    if !unknown.is_empty() {
        // 可见地降级：逐行点名 + 汇总（判据：静默跳过 = 用户永远查不出「为什么没生效」）
        for d in &unknown {
            tracing::warn!(domain = %d, "capability_domain_overrides 存在无法解析的域行，已跳过");
        }
        tracing::warn!(
            skipped = unknown.len(),
            "能力域覆盖层部分行被跳过（这些域将按内置默认处理）"
        );
    }

    let count = applied.len();
    apply_domain_overrides(applied);
    Ok(count)
}

/// **不触碰全局状态**地把「库里的覆盖」渲染成合并视图所需的形状。
///
/// 与 [`load_into_runtime`] 的分工：读命令不该顺手改全局运行时状态
/// （那是副作用，且会让并发请求看到别人触发的刷新），故它取本函数的返回值
/// 交给 `*_with` 纯函数族渲染 —— 两处用**同一套判据**，不产生第二份「启用语义」。
pub async fn snapshot_for_view(
    db: &DatabaseConnection,
) -> Result<(Vec<DomainOverride>, Vec<String>)> {
    list_parsed(db).await
}

/// 解码 `extra_aliases` JSON 数组；坏 JSON 返回空 vec（不 panic）。
///
/// 坏 JSON 与坏 `domain` 的处理刻意不同：`domain` 坏 ⇒ 整行无意义（跳过）；
/// `extra_aliases` 坏 ⇒ 该行的 **`enabled` 仍然有效**，只是没有追加别名 ⇒
/// 只降级这一行的别名部分，不要让整个「停用」决定一起失效。
fn decode_aliases(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_harness::{
        clear_domain_overrides, enabled_domains, is_domain_enabled, l1_classifier_domain_list,
        resolve_enabled_domain,
    };
    /// 建库：走**与生产同一条**建表链（`db::initialize_schema`）。
    ///
    /// ⚠ 2026-09-16 改判：此前这里是 `Database::connect("sqlite::memory:")` +
    /// `migrations::run_migrations`。版本化迁移清空后 `run_migrations` 变成**空操作**
    /// ⇒ 表一张都建不出来，本文件所有用例集体报 `no such table`。
    /// 换 `create_test_pool` 的理由不是「它更好用」，而是**建表来源只能有一个**：
    /// 测试要自己另起一套建库方式，它验证的就不是用户真拿到的那条链。
    async fn setup() -> DatabaseConnection {
        let handle = crate::db::create_test_pool().await.expect("测试库应可建立");
        // `DatabaseConnection` 是 Arc 句柄：`handle` 在 setup 返回时析构，但克隆体仍
        // 持有连接池（`DbHandle` 无 `Drop` 实现，且池的 `min_connections=1`）。
        handle.conn.clone()
    }

    /// 覆盖层是**进程级全局**状态 ⇒ 触碰它的用例必须串行（与 harness 侧同款约定）。
    ///
    /// # 为什么是 `tokio::sync::Mutex`（而不是 `std::sync::Mutex` + `#[allow]`）
    ///
    /// 本模块的用例全是 `#[tokio::test]`，且**必须**在持锁期间 `await`（断言对象是
    /// DB 往返 + `route(...).await` 的结果，而覆盖层在整个 `await` 期间不许被别人改）。
    /// std guard 跨 await 会同时踩两条 `-D warnings` 下的 error：
    ///   - `clippy::disallowed_types`（本仓 `clippy.toml` 禁用 `std::sync::{Mutex, …}`）；
    ///   - `clippy::await_holding_lock`（「this `MutexGuard` is held across an await point」）。
    ///
    /// 换异步锁 ⇒ 两条都不触发 ⇒ **本模块不需要任何 `#[allow]` 豁免**。
    ///
    /// ⚠ 前车之鉴：我曾在这里写「`std::sync::Mutex` 不跨 await」的 SAFETY 豁免注释，
    /// 而事实恰好相反 —— clippy 当场抓住。**豁免注释是承诺，不是排版**。
    static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// 统一入口：**前置清空** + 返回锁（异步，无中毒语义）。
    async fn case() -> tokio::sync::MutexGuard<'static, ()> {
        let g = TEST_LOCK.lock().await;
        clear_domain_overrides();
        g
    }

    /// 无行 ⇒ 无覆盖 ⇒ 全部启用（覆盖层语义的地基）。
    #[tokio::test]
    async fn empty_table_means_no_override() {
        let _g = case().await;
        let db = setup().await;

        assert!(list_overrides(&db).await.expect("读表应成功").is_empty());
        let applied = load_into_runtime(&db).await.expect("加载应成功");
        assert_eq!(applied, 0);
        assert_eq!(enabled_domains().len(), 9, "无覆盖时 9 个域全部启用");
    }

    /// **核心链路**：写库 → 加载 → 运行时行为真的变了（消费端①②的前提）。
    #[tokio::test]
    async fn upsert_then_load_changes_runtime_behavior() {
        let _g = case().await;
        let db = setup().await;

        // 逗号切分（不依赖分隔符两侧空格的具体写法：别把「格式化细节」写成契约）。
        let in_list = |list: &str, d: &str| list.split(',').any(|s| s.trim() == d);

        // **对照组**：覆盖层尚未加载时（`case()` 已清空全局），内置默认下必须有 finance。
        // 没有这一条，「加载后不含 finance」这个断言在「清单恒为空」的退化实现下也会绿。
        assert!(
            in_list(&l1_classifier_domain_list(), "finance"),
            "对照组失败：内置默认下 finance 应在 L1 域清单里，实际清单 = {}",
            l1_classifier_domain_list()
        );

        upsert_override(&db, "finance", false, &[]).await.expect("写入应成功");
        // 幂等：同域重复写入走 on_conflict 更新，不报错
        upsert_override(&db, "finance", false, &["股票分析".to_string()])
            .await
            .expect("重复写入应成功（upsert）");

        let applied = load_into_runtime(&db).await.expect("加载应成功");
        assert_eq!(applied, 1);

        assert!(!is_domain_enabled(CapabilityDomain::Finance), "停用应生效");
        assert_eq!(enabled_domains().len(), 8);
        assert_eq!(resolve_enabled_domain("finance"), None, "停用域不得从入口解析出来");
        assert_eq!(
            resolve_enabled_domain("股票分析"),
            None,
            "域已停用 ⇒ 其追加别名同样不得解析出来"
        );

        // **消费端①**：L1 分类器 prompt 的域清单必须跟着变 ——
        // 这是「真实写入 → load → 某条代码路径行为真的变了」的**同一条测试内**闭环
        // （PLAN-domain-single-source.md §9.3：验收标准不是「有 UI」，而是「行为真的变了」）。
        assert!(
            !in_list(&l1_classifier_domain_list(), "finance"),
            "停用后 L1 域清单里不得再有 finance，实际清单 = {}（覆盖层没接到分类器 prompt 这一消费端）",
            l1_classifier_domain_list()
        );
        // 停用一个域不应连带影响别的域（防「一停全停」的退化实现也蒙混过关）。
        assert!(
            in_list(&l1_classifier_domain_list(), "data_analysis"),
            "停用 finance 不应牵连 data_analysis，实际清单 = {}",
            l1_classifier_domain_list()
        );

        clear_domain_overrides();
    }

    /// 追加别名生效；删除该行后回到内置默认（含别名一并消失）。
    #[tokio::test]
    async fn extra_aliases_round_trip_and_delete_restores_default() {
        let _g = case().await;
        let db = setup().await;

        upsert_override(&db, "finance", true, &["选股".to_string()]).await.expect("写入应成功");
        load_into_runtime(&db).await.expect("加载应成功");
        assert_eq!(resolve_enabled_domain("选股"), Some(CapabilityDomain::Finance));

        let row = get_override(&db, "finance").await.expect("读取应成功").expect("应有一行");
        assert_eq!(row.extra_aliases, vec!["选股".to_string()]);

        assert!(delete_override(&db, "finance").await.expect("删除应成功"), "应删掉一行");
        assert!(
            !delete_override(&db, "finance").await.expect("删除应成功"),
            "再删应返回 false（本来就没有）"
        );

        load_into_runtime(&db).await.expect("加载应成功");
        assert_eq!(resolve_enabled_domain("选股"), None, "删除覆盖后追加别名应失效");
        assert_eq!(enabled_domains().len(), 9, "删除覆盖后回到全部启用");
    }

    /// 坏行（`domain` 解析不出枚举）⇒ 跳过 + 其余行照常生效，**不报错、不阻塞启动**。
    ///
    /// 这条用例是「手工改库 / 旧版本残留」的防回归：坏行不得让覆盖层整体失效，
    /// 也不得让应用起不来。
    #[tokio::test]
    async fn unparseable_domain_row_is_skipped_but_others_apply() {
        use sea_orm::ConnectionTrait;
        let _g = case().await;
        let db = setup().await;

        db.execute_unprepared(
            "INSERT INTO capability_domain_overrides \
             (domain, enabled, extra_aliases, created_at, updated_at) \
             VALUES ('not_a_real_domain', 0, '[]', 1, 1)",
        )
        .await
        .expect("写入应成功");
        upsert_override(&db, "automation", false, &[]).await.expect("写入应成功");

        let applied = load_into_runtime(&db).await.expect("坏行不应让加载报错");
        assert_eq!(applied, 1, "只有 automation 一行可解析");
        assert!(!is_domain_enabled(CapabilityDomain::Automation), "好行必须照常生效");
        assert_eq!(enabled_domains().len(), 8);
    }

    /// `extra_aliases` 坏 JSON ⇒ 只降级别名部分，该行的 `enabled` 仍然生效。
    #[tokio::test]
    async fn broken_aliases_json_keeps_enabled_effect() {
        use sea_orm::ConnectionTrait;
        let _g = case().await;
        let db = setup().await;

        db.execute_unprepared(
            "INSERT INTO capability_domain_overrides \
             (domain, enabled, extra_aliases, created_at, updated_at) \
             VALUES ('finance', 0, 'not-json', 1, 1)",
        )
        .await
        .expect("写入应成功");

        load_into_runtime(&db).await.expect("加载应成功");
        assert!(
            !is_domain_enabled(CapabilityDomain::Finance),
            "别名 JSON 坏掉不应让「停用」决定一起失效"
        );
        assert!(effective_aliases_has_no_extra(&db).await, "坏 JSON 应按空别名处理");
    }

    async fn effective_aliases_has_no_extra(db: &DatabaseConnection) -> bool {
        let row = get_override(db, "finance").await.expect("读取应成功").expect("应有一行");
        row.extra_aliases.is_empty()
    }

    /// 不可停用的域：即便库里有 `enabled = false` 的行，读侧也仍按启用处理（同一判据兜底）。
    #[tokio::test]
    async fn general_row_cannot_disable_it() {
        let _g = case().await;
        let db = setup().await;

        upsert_override(&db, "general", false, &[]).await.expect("写入应成功");
        load_into_runtime(&db).await.expect("加载应成功");

        assert!(
            is_domain_enabled(CapabilityDomain::General),
            "general 是唯一兜底域，读侧必须无视停用行"
        );
        assert_eq!(resolve_enabled_domain("general"), Some(CapabilityDomain::General));
        assert_eq!(enabled_domains().len(), 9);
    }
}
