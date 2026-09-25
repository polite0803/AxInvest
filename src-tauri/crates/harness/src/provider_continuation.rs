// SPDX-License-Identifier: AGPL-3.0-only

//! 会话级 Responses API 续写状态 —— `previous_response_id` + 消息水位。
//!
//! ## 为什么需要
//!
//! OpenAI Responses API 允许用 `previous_response_id` 把上一轮的响应接在链上，
//! 本轮只发**增量 items**，服务端复用已缓存的上下文（codex 侧即如此，
//! 见 `AUDIT-codex-parity-gap-2026-09-24.md` §4.3）。要发增量就必须知道两件事：
//!
//! 1. 上一轮的 response id（服务端生成，只能从响应里取回）；
//! 2. 本地历史里**已被该 response 链覆盖到第几条**（水位）。
//!
//! ## 分层与职责边界
//!
//! - **provider 侧**（`axagent-providers`）只做一件事：拿到 response id 后
//!   [`ContinuationStore::note_response_id`] 登记（provider 不知道本地历史条数）。
//! - **调用点侧**（`axagent-agent` 的 `AxAgentApiClient`）读水位、裁增量、
//!   并在本轮成功后 [`ContinuationStore::record`] 把 id 与水位**一次配对写入**。
//!   两段写入分开是为了避免「provider 猜水位」——它看不到未裁剪的完整历史。
//! - 键为会话 id（`ProviderRequestContext::conversation`）。批处理型调用点不绑定
//!   该字段 ⇒ 既不读也不写，状态表里不会出现它们的任何条目。
//!
//! ## 失效面
//!
//! 历史被压缩 / 回退（`covered >= 当前历史条数`）时链已不可信，调用点必须
//! [`ContinuationStore::clear`] 后整段重发，否则会静默丢上下文。
//!
//! ## 锁的选择
//!
//! 状态表同时被同步调用点（`ApiClient::stream` 是同步 trait 方法）与异步
//! provider 任务访问，故用 std 互斥锁。**临界区内只有 HashMap 读写，绝不跨
//! await 持有**，符合 `clippy.toml` 头部约定的合法例外。

#![allow(clippy::disallowed_types)]

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// 单个会话的续写状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinuationState {
    /// 上一轮服务端返回的 response id（下一轮经 `previous_response_id` 回传）。
    pub response_id: String,
    /// 已被该 response 链覆盖的**历史消息条数**（不含系统提示）。
    pub covered_messages: usize,
}

#[derive(Debug, Default)]
struct Inner {
    states: HashMap<String, ContinuationState>,
    /// 本轮刚从 provider 取回的 response id（尚未与水位配对）。
    pending_response_id: HashMap<String, String>,
}

/// 进程级续写状态表。
#[derive(Debug)]
pub struct ContinuationStore {
    inner: Mutex<Inner>,
}

impl ContinuationStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 读取会话续写状态；无状态或已清除时返回 `None`。
    pub fn get(&self, conversation_id: &str) -> Option<ContinuationState> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).states.get(conversation_id).cloned()
    }

    /// 一次配对写入「response id + 覆盖到的历史条数」（调用点在本轮成功后调用）。
    pub fn record(
        &self,
        conversation_id: &str,
        response_id: impl Into<String>,
        covered_messages: usize,
    ) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.pending_response_id.remove(conversation_id);
        inner.states.insert(
            conversation_id.to_string(),
            ContinuationState { response_id: response_id.into(), covered_messages },
        );
    }

    /// provider 侧登记本轮 response id（水位稍后由调用点补上）。
    pub fn note_response_id(&self, conversation_id: &str, response_id: impl Into<String>) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending_response_id
            .insert(conversation_id.to_string(), response_id.into());
    }

    /// 取出并清除待配对的 response id。
    pub fn take_response_id(&self, conversation_id: &str) -> Option<String> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending_response_id
            .remove(conversation_id)
    }

    /// 清除会话续写状态（压缩 / 回退 / 显式重置）。
    pub fn clear(&self, conversation_id: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.states.remove(conversation_id);
        inner.pending_response_id.remove(conversation_id);
    }
}

impl Default for ContinuationStore {
    fn default() -> Self {
        Self { inner: Mutex::new(Inner::default()) }
    }
}

/// 续写是否可用：水位必须**非零且严格落后于当前历史**。
///
/// - `covered == 0`：没有可续写的链；
/// - `covered >= history_len`：本轮无可发增量（历史被压缩 / 回退，或同一轮重试），
///   继续发空增量会让服务端接到空 input ⇒ 调用点须整段重发。
pub fn can_continue(state: &ContinuationState, history_len: usize) -> bool {
    state.covered_messages > 0 && state.covered_messages < history_len
}

/// 进程级单例状态表。
pub fn global() -> &'static ContinuationStore {
    static STORE: OnceLock<ContinuationStore> = OnceLock::new();
    STORE.get_or_init(ContinuationStore::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(covered: usize) -> ContinuationState {
        ContinuationState { response_id: "resp_1".to_string(), covered_messages: covered }
    }

    #[test]
    fn record_then_get_roundtrips_and_clears_pending() {
        let store = ContinuationStore::new();
        store.note_response_id("c1", "resp_1");
        store.record("c1", "resp_1", 4);

        assert_eq!(store.get("c1"), Some(state(4)));
        // record 已消费 pending，不应残留
        assert_eq!(store.take_response_id("c1"), None);
    }

    #[test]
    fn clear_removes_both_state_and_pending() {
        let store = ContinuationStore::new();
        store.note_response_id("c1", "resp_1");
        store.record("c1", "resp_1", 2);
        store.note_response_id("c1", "resp_2");

        store.clear("c1");

        assert_eq!(store.get("c1"), None);
        assert_eq!(store.take_response_id("c1"), None);
    }

    #[test]
    fn stores_are_keyed_by_conversation() {
        let store = ContinuationStore::new();
        store.record("c1", "resp_1", 1);
        store.record("c2", "resp_2", 9);

        assert_eq!(store.get("c1").map(|s| s.response_id), Some("resp_1".to_string()));
        assert_eq!(store.get("c2").map(|s| s.covered_messages), Some(9));
        assert_eq!(store.get("missing"), None);
    }

    #[test]
    fn can_continue_requires_nonzero_and_strictly_behind() {
        assert!(can_continue(&state(2), 5), "水位落后于历史 ⇒ 可续写");
        assert!(!can_continue(&state(0), 5), "水位为 0 ⇒ 无可续写链");
        assert!(!can_continue(&state(5), 5), "水位与历史齐平 ⇒ 本轮无可发增量");
        assert!(!can_continue(&state(7), 5), "水位超前（压缩/回退）⇒ 链已失效");
    }
}
