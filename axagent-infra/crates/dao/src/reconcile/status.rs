// SPDX-License-Identifier: AGPL-3.0-only

//! `reconcile::status` —— 引擎视角的**结构状态探测（只读）**。
//!
//! ## 它回答的问题与 [`super::apply`] 不同
//!
//! | | 回答 | 载体 |
//! |---|---|---|
//! | `apply::*` | 「**这轮做了什么**」（执行了几条、跳过几条、被哪道闸拦下） | `ApplyOutcome` |
//! | 本模块 | 「**还差什么、谁会去修**」（离声明形态还差多少条、各由谁负责） | [`EngineStatus`] |
//!
//! **为什么不复用 `ApplyOutcome`**：它只有在**跑过一遍之后**才存在，而「打开设置页
//! 看一眼状态」这个诉求不能以「先跑一遍」为代价 —— 那等于「为了知道缺什么，先把能补的
//! 补了」，观测动作变成了变更动作。而且 `ApplyOutcome` 的字段全是「已发生」的计数
//! （executed / deferred / skipped），它对「本该执行却没执行的那些还差多少」只能给出
//! 一个混合桶（`unsupported()` 把方言限制与渲染真错混在一起），分不出「等引擎自动补」
//! 与「必须人工处理」—— 而这两者对用户是**完全不同的两句话**。
//!
//! ## 为什么也不走 dry-run
//!
//! `apply::run(..., dry_run = true)` 看起来是现成的只读路径，但它**不是**只读：
//! ① 它开头就 `ensure_meta_tables`（建元表）；② 结尾无条件 `write_audit`（写审计行）。
//! 一个「看一眼状态」的观测动作不该在库里留痕 —— 否则「用户只是打开了设置页」与
//! 「用户点了修复」在库里的痕迹无法区分（审计表里会多出一轮什么都没做的 run，
//! 而审计行的存在本身就是「这轮有人在动 schema」的证据）。本模块因此**自己构造 plan
//! 并逐条渲染**，全链路只走 `dialect_of` → `load_orphan_whitelist` → `introspect::read`
//! → `expected::build` → `diff_with` → `render` 这六个纯读 / 纯计算入口。
//!
//! ## 三个桶各自的「谁来修」
//!
//! | 字段 | 含义 | 谁来修 |
//! |---|---|---|
//! | `pending_apply` | 纯新增 **且** 本方言渲染得出 DDL | **引擎自动**：下次启动 `bootstrap_schema` 就补上 |
//! | `pending_unsupported` | 纯新增 **但** 本方言没有原生 DDL（SQLite 的 `SET DEFAULT` / `ADD FK`） | **暂时没人**：须等「重建表 12 步流程」（PLAN §3.1）。每轮 diff 都会重现、不影响使用、不报错 ⇒ 是**已知限制**，不是故障 |
//! | `pending_manual` | 非纯新增（收缩 / 改类型）**或** 渲染失败 | **人工**：启动期刻意不动（`additive_only` 白名单），须走离线全量 apply（探针） |
//!
//! ⚠ 因此 `pending_apply` 才是「这个库会在下次启动被自动补齐」这句承诺的量化形态；
//! 它同时是「bootstrap 与状态探测用的是同一份 plan 口径」的跨模块判据 ——
//! 两处将来漂移（有人改了 `retain_purely_additive` 或 `is_purely_additive`），
//! `probe_agrees_with_bootstrap_on_fresh_db` 会红。
//!
//! ## 前置条件（真依赖，不是风格问题）
//!
//! [`super::safety::load_orphan_whitelist`] 直接 `SELECT` 元表，**不会**替你建表
//! （建表是写动作）。故 `probe` 要求库里已有元表 —— 凡走过一次
//! `db::initialize_schema`（→ `bootstrap_schema` → `cycle` → `ensure_meta_tables`）
//! 的库都满足。未走过的库会得到 `Err(表不存在)`，而不是「白名单为空」这个**假答案**
//! （后者会让本该需要人工处理的孤儿表被少算）。

use sea_orm::{DatabaseConnection, DbErr};

/// 引擎视角的结构状态（**只读**：不执行任何 DDL、不写审计、不建元表）。
///
/// 三个桶**互斥且完备**：`pending_apply + pending_unsupported + pending_manual == plan.changes.len()`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineStatus {
    /// 方言：`"sqlite"` / `"postgres"`。
    pub dialect: String,
    /// 声明侧（`expected::build`）的表数。
    pub tables_expected: usize,
    /// 实况侧（`introspect::read`）的表数。
    pub tables_actual: usize,
    /// **引擎下次启动会自动补上**的条数：纯新增 **且** 本方言能渲染出 DDL。
    pub pending_apply: usize,
    /// 纯新增 **但** 本方言没有原生 DDL 的条数（SQLite 的 `SET DEFAULT` / `ADD FK`）。
    /// 属已知限制：每一轮 diff 都会重现、不影响使用、不报错。
    pub pending_unsupported: usize,
    /// **启动期刻意不动**的条数：非纯新增（收缩 / 改类型）**或** 渲染失败。
    pub pending_manual: usize,
    /// 声明漂移条数（advisory：只有表达式文本差异，**不产出任何 DDL**）。
    ///
    /// PG 上结构性非零（`'x'` → `'x'::text`、`true` → `TRUE` 之类），**不代表结构有问题**。
    pub advisories: usize,
    /// 异常备注（正常为空）。目前只有一种：非方言原因的渲染失败。
    ///
    /// 那在当前 plan 构造路径下不可达（`diff` 产出的 payload 与 kind 恒匹配），
    /// 真出现就是渲染器 bug ⇒ 必须可见。
    pub notes: Vec<String>,
}

/// 只读探测一次。
///
/// 取数序列与 [`super::apply::cycle`] **逐字一致**（同一份 plan 口径），只是不执行：
/// 差异之处只有「不 `ensure_meta_tables`、不收窄成纯新增、不跑 `apply`」。
///
/// ⚠ 刻意**不**调 `plan.retain_purely_additive()`：那是 bootstrap 的「执行收窄」动作，
/// 而这里要报的是**离声明有多远**（全量 diff），收窄后的数字会给用户一个偏小的缺口 ——
/// 尤其会把「需人工处理的收缩类变更」整个抹掉，而那正是最需要被看见的一类。
pub async fn probe(db: &DatabaseConnection) -> Result<EngineStatus, DbErr> {
    let dialect = super::apply::dialect_of(db)?;

    // 白名单要在 diff **之前**读（与 `cycle` 的顺序决策 2 一致）。它只影响「孤儿是否被
    // DROP」那一侧，而孤儿全是破坏性变更 ⇒ 只影响 `pending_manual` 的计数。
    // 已过期的白名单项不在 `active` 里 ⇒ 其孤儿表会被计入 `pending_manual`，
    // 这正是「它需要人工决定」的正确表达。
    let wl = super::safety::load_orphan_whitelist(db, super::safety::now_epoch()).await?;

    let actual = super::introspect::read(db).await?;
    let want = super::expected::build(dialect)?;
    let p = super::plan::diff_with(
        &want,
        &actual,
        dialect,
        &super::plan::PlanOptions { orphan_whitelist: wl.active },
    );

    let mut pending_apply = 0usize;
    let mut pending_unsupported = 0usize;
    let mut pending_manual = 0usize;
    let mut notes: Vec<String> = Vec::new();

    for c in &p.changes {
        // 非纯新增（收缩 / 改类型 / 改名 / SET NOT NULL）一律「启动期不动」⇒ 人工。
        // 不细分到「哪一类」：分的依据在 `is_purely_additive` 的白名单里，
        // 这里再抄一份分类就会多出一处会漂移的真相源。
        if !c.kind.is_purely_additive() {
            pending_manual += 1;
            continue;
        }
        match super::render::render(c, dialect) {
            Ok(_) => pending_apply += 1,
            // 方言能力不足：已知限制，不是缺陷 ⇒ 单列一桶，服务端与 UI 都不该报错。
            Err(super::render::RenderError::UnsupportedDialect { .. }) => pending_unsupported += 1,
            // 其余渲染失败（payload 与 kind 不匹配 / 标识符为空 / 类型名非法 …）：
            // 在当前 `diff` → `render` 的构造路径下不可达，真出现就是渲染器 bug。
            // 归人工桶**并且**留字面理由 —— 只加计数的话，UI 上会看到一个
            // 「待人工处理 N 条」而日志里找不到任何线索。
            Err(e) => {
                pending_manual += 1;
                notes.push(format!("{} {} 渲染失败：{e}", c.kind.as_str(), c.object));
            },
        }
    }

    Ok(EngineStatus {
        // 与 `model.rs` 里 `impl Serialize for Dialect` 的指纹形态（`"sqlite"` /
        // `"postgres"`）逐字一致。用 `Debug` 小写化得到它，而不是新加
        // `Dialect::as_str()`：后者会让「方言 → 字符串」的映射在本 crate 里出现
        // 第三份副本（`Debug` 派生、`Serialize` 实现、`as_str()`），三份各自腐烂。
        dialect: format!("{dialect:?}").to_lowercase(),
        tables_expected: want.tables.len(),
        tables_actual: actual.tables.len(),
        pending_apply,
        pending_unsupported,
        pending_manual,
        advisories: p.advisories.len(),
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 照抄 `probe` 的取数序列，只为拿到「全量 diff 的变更条数」这个基数。
    ///
    /// 刻意**不复用** `probe`（不调它也不从它内部取数）：判据要能独立证伪
    /// `probe` 的分桶逻辑 —— 若两边共用同一份取数代码，那么「桶之和 == 变更数」
    /// 这条断言就退化成 `x == x`，永远绿。
    async fn full_diff(db: &DatabaseConnection) -> super::super::plan::Plan {
        let dialect = super::super::apply::dialect_of(db).expect("测试：方言应可解析");
        let wl = super::super::safety::load_orphan_whitelist(db, super::super::safety::now_epoch())
            .await
            .expect("测试：白名单应可读");
        let actual = super::super::introspect::read(db).await.expect("测试：实况应可读");
        let want = super::super::expected::build(dialect).expect("测试：期望应可算");
        super::super::plan::diff_with(
            &want,
            &actual,
            dialect,
            &super::super::plan::PlanOptions { orphan_whitelist: wl.active },
        )
    }

    /// 三个桶**互斥且完备** —— 它们必须无重复、无遗漏地覆盖全量 diff。
    ///
    /// 完备性会红的两种真实情形：① 有人给分桶加了 `continue` 却忘了计数（某类变更
    /// 从所有桶里消失，UI 报出的缺口偏小）；② 有人新增了一个 `ChangeKind` 并把它
    /// 归进两个桶。
    #[tokio::test]
    async fn buckets_sum_to_change_count() {
        let handle = crate::db::create_test_pool().await.expect("测试库应可建立");
        let db = &handle.conn;

        let st = probe(db).await.expect("测试：只读探测应成功");
        let p = full_diff(db).await;
        let changes = p.changes.len();

        println!(
            "[probe] dialect={} tables_expected={} tables_actual={} \
             pending_apply={} pending_unsupported={} pending_manual={} advisories={} notes={:?} \
             | 全量 diff: changes={} advisories={:?}",
            st.dialect,
            st.tables_expected,
            st.tables_actual,
            st.pending_apply,
            st.pending_unsupported,
            st.pending_manual,
            st.advisories,
            st.notes,
            changes,
            // 打进输出是为了**取证**：advisory 是 UI 会显示的内容，只报条数的话
            // 「为什么一个全新库有 1 条 advisory」只能靠猜。
            p.advisories
        );

        assert_eq!(
            st.pending_apply + st.pending_unsupported + st.pending_manual,
            changes,
            "三个桶必须互斥且完备（桶之和 == 全量 diff 条数）：桶和 {} vs 变更数 {}",
            st.pending_apply + st.pending_unsupported + st.pending_manual,
            changes
        );
        assert_eq!(
            st.advisories,
            p.advisories.len(),
            "`advisories` 必须原样透传 plan 的诊断条数（它是 UI 上「声明漂移」那一行的唯一来源）"
        );
        // 反向断言：证明探测**真的读到了库**，而不是对着一个空模型下判据
        // （纪律：统计量恰为 0 时先怀疑测量工具）。表数是「读到东西」的最直接证据，
        // 且它与分桶逻辑无关 —— 就算分桶全错，这条也应当为真。
        assert!(
            st.tables_actual > 100,
            "实况只读到 {} 张表 —— 这不像一个走完 `initialize_schema` 的库，\
             更像是 introspect 空手而归（此时上面那条分桶断言没有意义）",
            st.tables_actual
        );
        assert!(st.tables_expected > 0, "声明侧一张表都没有，期望模型构造失败了");
    }

    /// **跨模块一致性判据**：启动期 bootstrap 跑完之后，「下次启动会自动补的条数」必须是 0。
    ///
    /// `create_test_pool` 走的是与生产**同一条** `initialize_schema`（含
    /// `bootstrap_schema`，纯新增），所以此刻库里已经没有任何「引擎够得着却没做」的
    /// 变更。若这条红了，只可能是两件事之一：
    ///
    /// 1. **bootstrap 与状态探测的口径漂移了** —— 例如有人改了
    ///    `retain_purely_additive` / `is_purely_additive`，于是两边对「什么算纯新增」
    ///    的理解不一致：一条变更在 bootstrap 侧被跳过、在探测侧被算进 `pending_apply`
    ///    （或反过来）。那时 UI 会永远显示「有 N 条待自动补齐」而每次启动都不消失；
    /// 2. `bootstrap_schema` 真的没把纯新增做完（渲染/执行失败被静默降级）。
    ///
    /// ⚠ 只断言 `pending_apply == 0`，**不**断言总数为 0：`pending_unsupported` 允许非零
    /// —— 那是 SQLite 表达不了的几类（`SET DEFAULT` / `ADD FK` …），每一轮都会重现且
    /// 永远不会被执行，在它上面要求「归零」等于要求引擎做不到的事。
    #[tokio::test]
    async fn probe_agrees_with_bootstrap_on_fresh_db() {
        let handle = crate::db::create_test_pool().await.expect("测试库应可建立");
        let db = &handle.conn;

        let st = probe(db).await.expect("测试：只读探测应成功");

        println!(
            "[probe/bootstrap 一致性] dialect={} tables_expected={} tables_actual={} \
             pending_apply={} pending_unsupported={} pending_manual={} advisories={} notes={:?}",
            st.dialect,
            st.tables_expected,
            st.tables_actual,
            st.pending_apply,
            st.pending_unsupported,
            st.pending_manual,
            st.advisories,
            st.notes
        );

        assert_eq!(
            st.pending_apply, 0,
            "bootstrap 刚跑完（纯新增），却仍报 {} 条「下次启动会自动补」—— \
             说明启动期收敛与状态探测用的不是同一份口径，或 bootstrap 的纯新增没做完。\
             （`pending_unsupported` 允许非零，见本测试文档）",
            st.pending_apply
        );
    }
}
