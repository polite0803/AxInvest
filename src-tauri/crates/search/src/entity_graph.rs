// SPDX-License-Identifier: AGPL-3.0-only

//! `EntityGraphProvider` 的注入点（由 wiring 层调用 `set_entity_graph_provider`）。
//!
//! # 为什么需要这个模块
//!
//! `RAGPipeline` 的第 4 阶段（图增强检索）在 `entity_graph_provider` 为 `None` 时
//! **直接跳过**，而此前 `rag.rs` 里的调用点恒传 `None` ⇒ 该分支永不进入，
//! 也就是「Graph RAG 能力定义了但永不执行」。
//!
//! 根因是依赖方向：本 crate 不依赖 `dao`，拿不到 `dao::knowledge_graph_provider`
//! 的实现。因此与 `sources::set_sources` / `axagent_tools::parser::set_parser`
//! **同构**，由上层 wiring 注入。
//!
//! # 默认行为
//!
//! 未注入 ⇒ `entity_graph_provider()` 返回 `None` ⇒ 管线跳过图增强检索，
//! **与接线前的运行行为完全一致**。是否启用由 wiring 层决定（当前受设置里
//! `ragPipelineConfig.entityGraph.enabled` 控制，默认关闭）。
//!
//! # 可运行期替换（2026-09-15 修）
//!
//! 原实现用 `OnceLock`（与 `sources::set_sources` 同构），语义是「首次注入生效、
//! 之后忽略」⇒ **改设置必须重启应用**，而 UI 上「配置了却不即时生效」最容易被当成 bug。
//! 现改为 `RwLock<Option<..>>`：设置保存后 wiring 层重新调用本模块即可即时生效，
//! 关闭开关时调 `clear_entity_graph_provider`。

// SAFETY: 此处 std::sync::RwLock 不跨 await 使用，临界区内仅同步操作。
#[allow(clippy::disallowed_types)]
use std::sync::{Arc, RwLock};

use axagent_harness::EntityGraphProvider;

/// 进程内单例。`RwLock::new` 是 const，故可作 `static` 初始化。
///
/// 每次检索在进入时 `read().clone()` 取一次 `Arc` 快照 ⇒ 正在执行的检索不会中途
/// 被换成另一个实现，这一点与原 `OnceLock` 的保证等价。
// SAFETY: 此处 std::sync::RwLock 不跨 await 使用，临界区内仅同步操作。
#[allow(clippy::disallowed_types)]
static ENTITY_GRAPH_PROVIDER: RwLock<Option<Arc<dyn EntityGraphProvider>>> = RwLock::new(None);

/// 注入 / **替换**实体图谱提供者。
///
/// 由 wiring 层在启动时调用，并在「设置面板保存 RAG 配置后」重新调用
/// （`enabled = true` 注入新实例，`enabled = false` 走 [`clear_entity_graph_provider`]），
/// 因此开关无需重启应用。
///
/// `Err(poisoned)` 时取回内部值继续写：本模块的临界区只有一次赋值，不可能留下
/// 半更新状态，故中毒不影响正确性（也就不必把它升级成 panic）。
pub fn set_entity_graph_provider(provider: Arc<dyn EntityGraphProvider>) {
    let mut guard = ENTITY_GRAPH_PROVIDER.write().unwrap_or_else(|e| e.into_inner());
    *guard = Some(provider);
}

/// 清除已注入的提供者 ⇒ 关闭图增强检索（运行期关闭开关的路径）。
///
/// 只把槽位置回 `None`，**不销毁**正在被检索持有的 `Arc`（它们持的是快照）。
pub fn clear_entity_graph_provider() {
    let mut guard = ENTITY_GRAPH_PROVIDER.write().unwrap_or_else(|e| e.into_inner());
    *guard = None;
}

/// 取已注入的实体图谱提供者；未注入时返回 `None`（管线跳过图增强检索）。
pub fn entity_graph_provider() -> Option<Arc<dyn EntityGraphProvider>> {
    ENTITY_GRAPH_PROVIDER.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// 是否已注入。
///
/// # 这是「开关是否打开」的**唯一**运行期判据
///
/// 注入与否由 wiring 层按全局设置 `ragPipelineConfig.entityGraph.enabled` 决定
/// （`src/init/services.rs`），因此本函数等价于「用户有没有打开图增强检索开关」。
/// `rag.rs::fold_entity_graph_context` 用它区分「图谱零命中」与「功能未开启」两种情况：
/// 前者要告警（多半是 kb_id 域不匹配或图谱未构建），后者必须静默。
pub fn is_entity_graph_enabled() -> bool {
    ENTITY_GRAPH_PROVIDER.read().unwrap_or_else(|e| e.into_inner()).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_harness::test_support::NoopEntityGraphProvider;

    /// 注入点是**进程内单例**，下面两个用例会改同一份全局状态。
    /// 若不串行化，`cargo test` 的默认多线程会把「A set / B clear / A assert」交错，
    /// 表现为随机失败（假红）。故两者共用本锁。
    // SAFETY: 此处 std::sync::Mutex 不跨 await 使用（纯同步测试锁，测试模块内无
    //         async 上下文），临界区内仅同步操作。属 clippy.toml 允许的例外。
    #[allow(clippy::disallowed_types)]
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 注入契约：`set` 之后必须变为「已注入」。
    ///
    /// 这是本模块唯一的对外承诺（`is_entity_graph_enabled` 是运行期开关判据），
    /// 故必须有一个**真能失败**的用例来钉住它 —— 原先那个用例是
    /// `if !is_entity_graph_enabled() { assert!(entity_graph_provider().is_none()) }`，
    /// 条件与断言其实是同一件事（`is_none()` ⟺ `!is_some()` ⟺ `!enabled()`），
    /// 属恒真式，永远不可能报错，等于没有测试。
    #[test]
    fn set_provider_turns_enabled_on() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let before = is_entity_graph_enabled();
        set_entity_graph_provider(Arc::new(NoopEntityGraphProvider));
        assert!(is_entity_graph_enabled(), "注入后必须变为已启用");

        if !before {
            // 本次调用是首次注入 ⇒ 可以断言「注入真的发生了」
            assert!(entity_graph_provider().is_some(), "注入后必须能取回 provider");
        }
    }

    /// **热更新契约**：清除后必须真的关闭、且能再次注入。
    ///
    /// # 为什么必须钉
    ///
    /// `OnceLock` 时代做不到「先开 → 关 → 再开」：首次 `set` 之后所有后续 `set` 都是
    /// no-op，也没有「清除」入口 ⇒ 用户改设置必须重启。本用例用「关掉再打开」把这条
    /// 能力钉死；若哪天有人把实现改回 `OnceLock`（或忘了给 `clear` 加写锁），这里会红。
    #[test]
    fn provider_can_be_cleared_and_reinjected() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        set_entity_graph_provider(Arc::new(NoopEntityGraphProvider));
        assert!(is_entity_graph_enabled(), "前置：注入后应为启用");

        clear_entity_graph_provider();
        assert!(!is_entity_graph_enabled(), "清除后必须变为未启用");
        assert!(entity_graph_provider().is_none(), "清除后必须取不回 provider");

        set_entity_graph_provider(Arc::new(NoopEntityGraphProvider));
        assert!(is_entity_graph_enabled(), "清除后必须能重新注入（热更新开启开关的路径）");
    }
}
