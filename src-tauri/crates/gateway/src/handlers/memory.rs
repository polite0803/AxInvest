// SPDX-License-Identifier: AGPL-3.0-only

//! 记忆外溢 HTTP handlers。
//! 路由全部挂在 `protected`（已走 auth_middleware）。
//!
//! [2026-09-03 已接线] 后端 = `GatewayAppState.memory_store`（`harness::memory::MemoryStore`
//! 接缝，主 crate wiring 层注入 `DaoMemoryStore`，见 src/gateway_memory_store.rs）。
//! 方法名与 DTO 字段已对齐 harness trait（search_memories / get_memory_tree /
//! submit_feedback / MemoryActionResultDto.error / MemoryFeedbackRequest.memory_id）。

use crate::server::GatewayAppState;
use axagent_harness::memory::{
    MemoryActionResultDto, MemoryAddRequest, MemoryFeedbackRequest, MemoryGroupedDto,
    MemorySearchItem, MemorySearchRequest, MemoryTreeItem, MemoryUpdateRequest,
};
use axum::Json;
use axum::extract::{Path, State};

/// POST /api/memory — 新增记忆
pub async fn add_memory(
    State(state): State<GatewayAppState>,
    Json(req): Json<MemoryAddRequest>,
) -> Json<MemoryActionResultDto> {
    match state.memory_store.add_memory(req).await {
        Ok(r) => Json(r),
        Err(e) => Json(MemoryActionResultDto { success: false, error: Some(e) }),
    }
}

/// POST /api/memory/search — 检索记忆
pub async fn search_memory(
    State(state): State<GatewayAppState>,
    Json(req): Json<MemorySearchRequest>,
) -> Json<Vec<MemorySearchItem>> {
    match state.memory_store.search_memories(req).await {
        Ok(r) => Json(r),
        Err(e) => {
            tracing::error!("memory search failed: {e}");
            Json(Vec::new())
        },
    }
}

/// GET /api/memory/tree — 记忆树（namespace → items）
pub async fn memory_tree(State(state): State<GatewayAppState>) -> Json<Vec<MemoryTreeItem>> {
    match state.memory_store.get_memory_tree().await {
        Ok(r) => Json(r),
        Err(e) => {
            tracing::error!("memory tree failed: {e}");
            Json(Vec::new())
        },
    }
}

/// GET /api/memory/working — working 层记忆内容
pub async fn memory_working(State(state): State<GatewayAppState>) -> Json<Option<String>> {
    match state.memory_store.get_working_memory().await {
        Ok(r) => Json(r),
        Err(e) => {
            tracing::error!("memory working failed: {e}");
            Json(None)
        },
    }
}

/// GET /api/memory/grouped — 按日期分组
pub async fn memory_grouped(State(state): State<GatewayAppState>) -> Json<Vec<MemoryGroupedDto>> {
    match state.memory_store.get_grouped_memories().await {
        Ok(r) => Json(r),
        Err(e) => {
            tracing::error!("memory grouped failed: {e}");
            Json(Vec::new())
        },
    }
}

/// POST /api/memory/{id}/feedback — 反馈（helpful → 晋升 tier）
pub async fn memory_feedback(
    State(state): State<GatewayAppState>,
    Path(id): Path<String>,
    Json(mut req): Json<MemoryFeedbackRequest>,
) -> Json<MemoryActionResultDto> {
    req.memory_id = id;
    match state.memory_store.submit_feedback(req).await {
        Ok(r) => Json(r),
        Err(e) => Json(MemoryActionResultDto { success: false, error: Some(e) }),
    }
}

/// PATCH /api/memory/{id} — 更新内容/重要度/标签
pub async fn update_memory(
    State(state): State<GatewayAppState>,
    Path(id): Path<String>,
    Json(mut req): Json<MemoryUpdateRequest>,
) -> Json<MemoryActionResultDto> {
    req.id = id;
    match state.memory_store.update_memory(req).await {
        Ok(r) => Json(r),
        Err(e) => Json(MemoryActionResultDto { success: false, error: Some(e) }),
    }
}

/// DELETE /api/memory/{id} — 删除记忆
pub async fn delete_memory_handler(
    State(state): State<GatewayAppState>,
    Path(id): Path<String>,
) -> Json<MemoryActionResultDto> {
    match state.memory_store.delete_memory(&id).await {
        Ok(r) => Json(r),
        Err(e) => Json(MemoryActionResultDto { success: false, error: Some(e) }),
    }
}
