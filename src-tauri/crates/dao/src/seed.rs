// SPDX-License-Identifier: AGPL-3.0-only
//! **持久初始行**（sentinel）的播种处 —— 「表建好之后，还必须存在的那几行」。
//!
//! ## 为什么必须有这个模块（P6 删迁移时暴露出来的唯一真缺口）
//!
//! 版本化迁移清空后，声明式引擎（`reconcile::apply::bootstrap_schema`）成为**唯一**
//! 建表来源。但「建表」不等于「库可用」：有两行数据是**跨 crate 的 DB 协议**，
//! 缺了它们库结构完好而功能静默失效。
//!
//! 这两行原先由 `migrations/v101_consolidate_knowledge_memory.rs` 的 PHASE 1 / PHASE 5
//! 写入。删掉那条迁移若不补回来，**全新库**就会出现下述空洞（存量库无影响，它们早已写入）：
//!
//! | 行 | 写入端 | 缺失后果 |
//! |---|---|---|
//! | `knowledge_bases.__sys_trajectory__` | `crates/trajectory/src/storage.rs`（写 `knowledge_entities` / `knowledge_relations` 时按其 id 归属） | 实体图按该 id 过滤 ⇒ **恒返空**，不报错 |
//! | `memory_namespaces.__sys_trajectory_memory__` | 同上（轨迹记忆的归属命名空间） | 记忆检索按该 id 过滤 ⇒ **恒返空**，不报错 |
//!
//! 「静默返空」这个失败形态是权威表述，见 `crates/harness/src/constants.rs:421-432`
//! 的模块文档（它也是这两个 id 的**唯一定义处**，值不在本文件重复书写）。
//!
//! ## 归属边界：什么该放这里，什么**不该**
//!
//! - ✅ **放这里**：全新库从零建起时也**必须**存在的行 —— 也就是「初始状态」的一部分。
//! - ❌ **不放这里**：存量库的一次性数据修复（回填、改名、类型转换、旧表搬运）。
//!   那些是**历史事件**：任何在用的库都已在其迁移轨道上执行过；全新库没有那些历史数据，
//!   执行它们是空操作。把它们搬进启动路径只会让每次启动都跑一遍「针对不存在的遗留形态」
//!   的逻辑，属于把历史事件伪装成不变量。
//!
//! ## 两条实现纪律
//!
//! 1. **用实体 `ActiveModel` 而不是手写 INSERT 文本**。手写 SQL 要自己维护「列集/类型/
//!    方言分支」三份知识，而列集已经在实体里了 —— 实体加了列，编译器会直接拦下漏填的
//!    构造；手写文本则会在运行期才报「列不存在」。历史教训：v101 曾在 PG 分支把
//!    `knowledge_bases.enabled`（INTEGER 列）写成布尔字面量 `FALSE`，PG 拒绝，
//!    错误冒泡成「数据库初始化失败」使应用**完全无法启动**（2026-09-12 生产实证）。
//! 2. **方言由连接自己决定，不在这里分支**。`on_conflict_do_nothing()` 由 sea-query 按
//!    连接 backend 渲染成 PG 的 `ON CONFLICT DO NOTHING` 或 SQLite 的等价形态 ⇒
//!    本函数**没有** `if is_pg` 这类分支，也就不存在「只在一个方言上被测过」的路径。
//!    用户在设置里选 SQLite 还是 PostgreSQL，两边走的是同一份代码。

use sea_orm::{DatabaseConnection, DbErr, EntityTrait, Set};

use axagent_entities::{knowledge_bases, memory_namespaces};

/// 播种持久初始行。**幂等**：可每轮启动调用。
///
/// 幂等性靠 `ON CONFLICT DO NOTHING`（主键冲突即跳过），不是靠「先查再插」——
/// 后者在并发启动下会撞唯一约束，而本函数可能与另一个进程同时启动。
pub async fn ensure_sentinels(db: &DatabaseConnection) -> Result<(), DbErr> {
    ensure_trajectory_knowledge_base(db).await?;
    ensure_trajectory_memory_namespace(db).await?;
    ensure_memory_reflow_knowledge_base(db).await?;
    Ok(())
}

/// 轨迹实体知识库的哨兵行。
async fn ensure_trajectory_knowledge_base(db: &DatabaseConnection) -> Result<(), DbErr> {
    use axagent_harness::constants::sentinel::{TRAJECTORY_KB_ENABLED, TRAJECTORY_KB_ID};

    // ⚠ `enabled` 必走 `TRAJECTORY_KB_ENABLED`（`i32`），不要内联字面量。
    //   该列是 INTEGER：PG **拒绝**布尔字面量并直接令应用起不来；
    //   用 `i32` 常量后「写错成布尔」在类型层即不可能（见常量文档）。
    // ⚠ `description` 显式给值而不是留给默认 —— 这一行会出现在知识库列表里，
    //   空描述会让用户面对一个不知用途的系统 KB。
    let row = knowledge_bases::ActiveModel {
        id: Set(TRAJECTORY_KB_ID.to_owned()),
        name: Set("System Trajectory Entities".to_owned()),
        description: Set(Some("Auto-extracted entities from conversation trajectories".to_owned())),
        enabled: Set(TRAJECTORY_KB_ENABLED),
        ..Default::default()
    };

    // 冲突时 `DO NOTHING` 的语义就是「已存在即保持原样」：这一行是**协议常量**的载体，
    // 不该被后续启动用本文件的字面量覆盖回去（否则改这里的文案就等于改线上数据）。
    knowledge_bases::Entity::insert(row)
        .on_conflict_do_nothing()
        .exec_without_returning(db)
        .await?;
    Ok(())
}

/// 轨迹记忆命名空间的哨兵行。
async fn ensure_trajectory_memory_namespace(db: &DatabaseConnection) -> Result<(), DbErr> {
    use axagent_harness::constants::sentinel::{TRAJECTORY_MEM_NS_ID, TRAJECTORY_MEM_NS_NAME};

    // ⚠ `scope` 显式写 `"system"`：实体层的默认值是 `"global"`（那是**用户**命名空间的默认），
    //   而本行是系统内部命名空间。漏写会把它混进用户可见的 global 列表。
    let row = memory_namespaces::ActiveModel {
        id: Set(TRAJECTORY_MEM_NS_ID.to_owned()),
        name: Set(TRAJECTORY_MEM_NS_NAME.to_owned()),
        scope: Set("system".to_owned()),
        sort_order: Set(0),
        ..Default::default()
    };

    memory_namespaces::Entity::insert(row)
        .on_conflict_do_nothing()
        .exec_without_returning(db)
        .await?;
    Ok(())
}

/// Memory → 知识图谱回流所用 KB 的哨兵行。
///
/// ## 为什么它也属于「初始状态」（即为什么放本模块而不是别处）
///
/// 回流路径（`repo::knowledge_graph::reflow_memory_to_knowledge`）把 `knowledge_entities`
/// 的 `knowledge_base_id` 指向本行。该列有指向 `knowledge_bases.id` 的**外键**
/// ⇒ 本行缺失时那条路径**一行也写不进去**，且失败会被吞（不报错、单测也可能过）。
/// 这与上面两行的失败形态**完全同族**：库结构完好，功能静默失效。
///
/// ⚠ 存量库无影响（若它已用过回流路径，本行早已由旧迁移写入）；
/// 本函数保证的是**全新库**从零建起时也具备该行。
async fn ensure_memory_reflow_knowledge_base(db: &DatabaseConnection) -> Result<(), DbErr> {
    use axagent_harness::constants::sentinel::{
        MEMORY_REFLOW_KB_ENABLED, MEMORY_REFLOW_KB_ID, MEMORY_REFLOW_KB_NAME,
    };

    // ⚠ `enabled` 必走常量（`i32`），不要内联字面量 —— 该列是 INTEGER，
    //   PG 拒绝布尔字面量并直接令应用起不来（见 `constants.rs` 的 `TRAJECTORY_KB_ENABLED`）。
    // ⚠ `description` 显式给值：这一行会出现在知识库列表里，
    //   空描述会让用户面对一个不知用途的系统 KB。
    let row = knowledge_bases::ActiveModel {
        id: Set(MEMORY_REFLOW_KB_ID.to_owned()),
        name: Set(MEMORY_REFLOW_KB_NAME.to_owned()),
        description: Set(Some(
            "Auto-reflowed high-importance memory items (system-internal)".to_owned(),
        )),
        enabled: Set(MEMORY_REFLOW_KB_ENABLED),
        ..Default::default()
    };

    knowledge_bases::Entity::insert(row)
        .on_conflict_do_nothing()
        .exec_without_returning(db)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_harness::constants::sentinel::{
        MEMORY_REFLOW_KB_ENABLED, MEMORY_REFLOW_KB_ID, TRAJECTORY_KB_ENABLED, TRAJECTORY_KB_ID,
        TRAJECTORY_MEM_NS_ID,
    };
    use sea_orm::PaginatorTrait;

    /// 启动链上真的播种了 —— 「建表之后这几行必须存在」是启动期的**不变量**。
    ///
    /// ⚠ 本条与 [`ensure_sentinels_restores_deleted_rows`] 是**两条不同的判据**，不能合并：
    /// - 本条的失败形态是「种子函数写对了但没人调用」（接线断了）；
    /// - 那条的失败形态是「有人调用但函数本身没写对」（实现错了）。
    /// 只留一条时，另一半永远查不出来。
    ///
    /// ⚠ 有效性边界（诚实登记）：在 `MIGRATIONS` 尚未清空的过渡期，`v101` 自己也会插入
    /// 同样的两行 ⇒ 本条对「种子接线是死的」**暂时不具区分力**。它转为可判别是在
    /// `MIGRATIONS` 置空那一刻 —— 那之前请以另一条判据为准。
    #[tokio::test]
    async fn create_test_pool_yields_sentinels() {
        let handle = crate::db::create_test_pool().await.expect("测试库应可建立");
        let db = &handle.conn;

        let kb = knowledge_bases::Entity::find_by_id(TRAJECTORY_KB_ID)
            .one(db)
            .await
            .expect("查询应成功")
            .expect("建库之后哨兵 KB 行必须已存在");
        assert_eq!(
            kb.enabled, TRAJECTORY_KB_ENABLED,
            "enabled 与常量不符 —— 该列是 INTEGER，两侧必须是同一个 i32"
        );

        let ns = memory_namespaces::Entity::find_by_id(TRAJECTORY_MEM_NS_ID)
            .one(db)
            .await
            .expect("查询应成功")
            .expect("建库之后哨兵命名空间行必须已存在");
        assert_eq!(ns.scope, "system", "哨兵命名空间 scope 必须是 system 而非实体默认的 global");

        // ⚠ 回流 KB 与上面两个哨兵**同样是启动期不变量**：回流路径写 `knowledge_entities`
        //   的 `knowledge_base_id` 时，外键指向本行 ⇒ 缺行 = 静默空转（一行都进不去）。
        let reflow = knowledge_bases::Entity::find_by_id(MEMORY_REFLOW_KB_ID)
            .one(db)
            .await
            .expect("查询应成功")
            .expect("建库之后回流哨兵 KB 行必须已存在");
        assert_eq!(reflow.enabled, MEMORY_REFLOW_KB_ENABLED, "回流 KB 的 enabled 与常量不符");
    }

    /// 播种函数本身：**删掉能补回来**（真的会写），且**重复调用不多写**（幂等）。
    ///
    /// 为什么先删再补，而不是「直接跑一次看有没有行」：
    /// `create_test_pool` 已经把这两行建好了 ⇒ 直接跑一次时「有行」这个观测对
    /// 「种子函数是空壳」同样成立（假绿）。**先破坏再修复**才把被测对象圈定在本函数上。
    ///
    /// 为什么要断言「第二次调用行数不变」：`DO NOTHING` 若被写成普通 INSERT，
    /// 第一次仍会成功 —— 只在第二次暴露。而本函数每轮启动都跑，
    /// 「第二次」不是边缘场景，而是每次冷启动之后的每次启动。
    #[tokio::test]
    async fn ensure_sentinels_restores_deleted_rows() {
        let handle = crate::db::create_test_pool().await.expect("测试库应可建立");
        let db = &handle.conn;

        // 破坏：把**所有**哨兵行都删掉。此后「行存在」只可能来自本函数。
        // ⚠ 本删除段必须与 `ensure_sentinels` 的播种集合**逐个对齐**：
        //   `count_sentinels` 是**全表计数** ⇒ 漏删一行会让下面 `== 0` 的前提断言直接失败
        //   （这正是不对齐时会暴露的地方，不是靠运气）。
        knowledge_bases::Entity::delete_by_id(TRAJECTORY_KB_ID).exec(db).await.expect("删除应成功");
        knowledge_bases::Entity::delete_by_id(MEMORY_REFLOW_KB_ID)
            .exec(db)
            .await
            .expect("删除应成功");
        memory_namespaces::Entity::delete_by_id(TRAJECTORY_MEM_NS_ID)
            .exec(db)
            .await
            .expect("删除应成功");
        assert_eq!(count_sentinels(db).await, 0, "前提：删除后应真的没有哨兵行");

        ensure_sentinels(db).await.expect("播种应成功");
        let after_first = count_sentinels(db).await;
        assert_eq!(after_first, 3, "播种后应恰好补回 3 行（2 个 KB + 1 个命名空间）");

        ensure_sentinels(db).await.expect("二次播种应成功（幂等）");
        assert_eq!(
            count_sentinels(db).await,
            after_first,
            "二次播种改变了行数 ⇒ 幂等性破了（会每轮启动重复插入）"
        );

        // 补回来的值也必须对，而不是「补了两行随便什么」。
        let kb = knowledge_bases::Entity::find_by_id(TRAJECTORY_KB_ID)
            .one(db)
            .await
            .expect("查询应成功")
            .expect("KB 行应已补回");
        assert_eq!(kb.enabled, TRAJECTORY_KB_ENABLED, "补回的 enabled 与常量不符");
        assert!(!kb.name.is_empty(), "补回的 name 不得为空（空名会让系统 KB 在列表里不可辨认）");

        let reflow = knowledge_bases::Entity::find_by_id(MEMORY_REFLOW_KB_ID)
            .one(db)
            .await
            .expect("查询应成功")
            .expect("回流哨兵 KB 行应已补回");
        assert_eq!(
            reflow.enabled, MEMORY_REFLOW_KB_ENABLED,
            "回流 KB 的 enabled 与常量不符 —— 回流路径的外键依赖这一行真实存在"
        );
        assert!(
            !reflow.name.is_empty(),
            "补回的 name 不得为空（空名会让回流 KB 在列表里不可辨认）"
        );

        let ns = memory_namespaces::Entity::find_by_id(TRAJECTORY_MEM_NS_ID)
            .one(db)
            .await
            .expect("查询应成功")
            .expect("命名空间行应已补回");
        assert_eq!(ns.scope, "system", "补回的 scope 必须是 system");
    }

    /// 哨兵行计数（KB 列表里的**全部**行 + 命名空间表里的**全部**行）。
    ///
    /// 用「全表计数」而非「按 id 计数」是刻意的：按 id 计数对「多插了一条别的行」
    /// 完全不敏感，而全表计数能抓住它。
    async fn count_sentinels(db: &DatabaseConnection) -> u64 {
        let kb = knowledge_bases::Entity::find().count(db).await.expect("计数应成功");
        let ns = memory_namespaces::Entity::find().count(db).await.expect("计数应成功");
        kb + ns
    }
}
