// SPDX-License-Identifier: AGPL-3.0-only
use axagent_harness::types::GatewayKey;
use axagent_harness::{ProviderAdapter, ProviderRequestContext};
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;

use crate::auth::AuthenticatedKey;
use crate::handlers::error::{
    error_response, provider_type_to_str, record_log, resolve_hermes_provider_context,
};
use crate::server::GatewayAppState;

// ── /api/jobs* 系列 18 个 handler 的公共样板 ──────────────────────────────
//
// 原先每个 handler 都逐字重复三段样板（合计 ~40 行/处）：
//   ① 前置解析 provider → ctx → adapter
//   ② 成功 / 失败两条路径的 `record_log!` 访问日志
//   ③ 成功响应的 `Response::builder()` 构造
// 收敛后每处只剩 1 行调用。**行为逐字保持**：method / path / status / 错误文案 /
// trait 方法名均未改动（声明集合 diff 见 `convert-jobs-handlers.mjs`）。

/// 公共前置：定位 Hermes/OpenClaw provider → 取 `ProviderRequestContext` → 取 adapter。
///
/// 失败时返回**已构造好的错误响应**，调用方 `Err(resp) => return resp` 即可。
///
/// 注意返回的是 **owned `Arc<dyn ProviderAdapter>`**（`provider_registry.get()` 本身
/// 返回 owned Arc），不是引用 —— 原先误写成 `&Arc<...>` 会触发 E0308。
async fn jobs_ctx(
    state: &GatewayAppState,
) -> Result<(Arc<dyn ProviderAdapter>, ProviderRequestContext, String), axum::response::Response> {
    let (provider, ctx) =
        resolve_hermes_provider_context(&state.adapter, &*state.provider_registry).await?;
    let adapter = match state.provider_registry.get(provider_type_to_str(&provider.provider_type)) {
        Some(a) => a,
        None => {
            return Err(error_response(StatusCode::BAD_GATEWAY, "No adapter available"));
        },
    };
    Ok((adapter, ctx, provider.id))
}

/// 记录一次 /api/jobs* 访问日志。成功与失败路径只有 `status` 不同，故合并为一个函数。
async fn jobs_log(
    state: &GatewayAppState,
    key: &GatewayKey,
    method: &str,
    path: &str,
    provider_id: &str,
    start_time: Instant,
    status: i32,
) {
    record_log!(
        &state.adapter,
        key,
        method,
        path,
        None,
        provider_id,
        status,
        start_time.elapsed().as_millis() as i64,
        0,
        0,
        None
    );
}

/// 成功响应：`application/json` + 指定状态码；body 构造失败时降级为 500 JSON。
fn jobs_ok_json(status: StatusCode, body: String) -> axum::response::Response {
    axum::response::Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(body.into())
        .unwrap_or_else(|_| {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to build response")
        })
}

/// GET /api/jobs — list all jobs from Hermes/OpenClaw gateway
pub async fn list_jobs(
    State(state): State<GatewayAppState>,
    Extension(auth): Extension<AuthenticatedKey>,
) -> impl IntoResponse {
    let AuthenticatedKey(gateway_key) = auth;
    let start_time = Instant::now();

    let (adapter, ctx, provider_id) = match jobs_ctx(&state).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match adapter.list_jobs(&ctx).await {
        Ok(response_body) => {
            jobs_log(&state, &gateway_key, "GET", "/api/jobs", &provider_id, start_time, 200).await;

            jobs_ok_json(StatusCode::OK, response_body)
        },
        Err(e) => {
            jobs_log(&state, &gateway_key, "GET", "/api/jobs", &provider_id, start_time, 500).await;

            error_response(StatusCode::BAD_GATEWAY, &format!("Failed to list jobs: {}", e))
        },
    }
}

/// POST /api/jobs — create a new job
pub async fn create_job(
    State(state): State<GatewayAppState>,
    Extension(auth): Extension<AuthenticatedKey>,
    axum::extract::Json(job_data): axum::extract::Json<serde_json::Value>,
) -> impl IntoResponse {
    let AuthenticatedKey(gateway_key) = auth;
    let start_time = Instant::now();

    let (adapter, ctx, provider_id) = match jobs_ctx(&state).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let job_data_str = serde_json::to_string(&job_data)
        .unwrap_or_else(|e| format!("{{\"error\":\"Serialization failed: {}\"}}", e));

    match adapter.create_job(&ctx, &job_data_str).await {
        Ok(response_body) => {
            jobs_log(&state, &gateway_key, "POST", "/api/jobs", &provider_id, start_time, 201)
                .await;

            jobs_ok_json(StatusCode::CREATED, response_body)
        },
        Err(e) => {
            jobs_log(&state, &gateway_key, "POST", "/api/jobs", &provider_id, start_time, 500)
                .await;

            error_response(StatusCode::BAD_GATEWAY, &format!("Failed to create job: {}", e))
        },
    }
}

/// GET /api/jobs/{job_id} — get a specific job
pub async fn get_job(
    State(state): State<GatewayAppState>,
    Extension(auth): Extension<AuthenticatedKey>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let AuthenticatedKey(gateway_key) = auth;
    let start_time = Instant::now();

    let (adapter, ctx, provider_id) = match jobs_ctx(&state).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match adapter.get_job(&ctx, &job_id).await {
        Ok(response_body) => {
            jobs_log(
                &state,
                &gateway_key,
                "GET",
                &format!("/api/jobs/{}", job_id),
                &provider_id,
                start_time,
                200,
            )
            .await;

            jobs_ok_json(StatusCode::OK, response_body)
        },
        Err(e) => {
            jobs_log(
                &state,
                &gateway_key,
                "GET",
                &format!("/api/jobs/{}", job_id),
                &provider_id,
                start_time,
                500,
            )
            .await;

            error_response(StatusCode::BAD_GATEWAY, &format!("Failed to get job: {}", e))
        },
    }
}

/// PATCH /api/jobs/{job_id} — update a job
pub async fn update_job(
    State(state): State<GatewayAppState>,
    Extension(auth): Extension<AuthenticatedKey>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
    axum::extract::Json(job_data): axum::extract::Json<serde_json::Value>,
) -> impl IntoResponse {
    let AuthenticatedKey(gateway_key) = auth;
    let start_time = Instant::now();

    let (adapter, ctx, provider_id) = match jobs_ctx(&state).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let job_data_str = serde_json::to_string(&job_data)
        .unwrap_or_else(|e| format!("{{\"error\":\"Serialization failed: {}\"}}", e));

    match adapter.update_job(&ctx, &job_id, &job_data_str).await {
        Ok(response_body) => {
            jobs_log(
                &state,
                &gateway_key,
                "PATCH",
                &format!("/api/jobs/{}", job_id),
                &provider_id,
                start_time,
                200,
            )
            .await;

            jobs_ok_json(StatusCode::OK, response_body)
        },
        Err(e) => {
            jobs_log(
                &state,
                &gateway_key,
                "PATCH",
                &format!("/api/jobs/{}", job_id),
                &provider_id,
                start_time,
                500,
            )
            .await;

            error_response(StatusCode::BAD_GATEWAY, &format!("Failed to update job: {}", e))
        },
    }
}

/// DELETE /api/jobs/{job_id} — delete a job
pub async fn delete_job(
    State(state): State<GatewayAppState>,
    Extension(auth): Extension<AuthenticatedKey>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let AuthenticatedKey(gateway_key) = auth;
    let start_time = Instant::now();

    let (adapter, ctx, provider_id) = match jobs_ctx(&state).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match adapter.delete_job(&ctx, &job_id).await {
        Ok(_) => {
            jobs_log(
                &state,
                &gateway_key,
                "DELETE",
                &format!("/api/jobs/{}", job_id),
                &provider_id,
                start_time,
                200,
            )
            .await;

            Json(json!({ "deleted": true, "id": job_id })).into_response()
        },
        Err(e) => {
            jobs_log(
                &state,
                &gateway_key,
                "DELETE",
                &format!("/api/jobs/{}", job_id),
                &provider_id,
                start_time,
                500,
            )
            .await;

            error_response(StatusCode::BAD_GATEWAY, &format!("Failed to delete job: {}", e))
        },
    }
}

/// POST /api/jobs/{job_id}/pause — pause a job
pub async fn pause_job(
    State(state): State<GatewayAppState>,
    Extension(auth): Extension<AuthenticatedKey>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let AuthenticatedKey(gateway_key) = auth;
    let start_time = Instant::now();

    let (adapter, ctx, provider_id) = match jobs_ctx(&state).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match adapter.pause_job(&ctx, &job_id).await {
        Ok(_) => {
            jobs_log(
                &state,
                &gateway_key,
                "POST",
                &format!("/api/jobs/{}/pause", job_id),
                &provider_id,
                start_time,
                200,
            )
            .await;

            Json(json!({ "paused": true, "id": job_id })).into_response()
        },
        Err(e) => {
            jobs_log(
                &state,
                &gateway_key,
                "POST",
                &format!("/api/jobs/{}/pause", job_id),
                &provider_id,
                start_time,
                500,
            )
            .await;

            error_response(StatusCode::BAD_GATEWAY, &format!("Failed to pause job: {}", e))
        },
    }
}

/// POST /api/jobs/{job_id}/resume — resume a job
pub async fn resume_job(
    State(state): State<GatewayAppState>,
    Extension(auth): Extension<AuthenticatedKey>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let AuthenticatedKey(gateway_key) = auth;
    let start_time = Instant::now();

    let (adapter, ctx, provider_id) = match jobs_ctx(&state).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match adapter.resume_job(&ctx, &job_id).await {
        Ok(_) => {
            jobs_log(
                &state,
                &gateway_key,
                "POST",
                &format!("/api/jobs/{}/resume", job_id),
                &provider_id,
                start_time,
                200,
            )
            .await;

            Json(json!({ "resumed": true, "id": job_id })).into_response()
        },
        Err(e) => {
            jobs_log(
                &state,
                &gateway_key,
                "POST",
                &format!("/api/jobs/{}/resume", job_id),
                &provider_id,
                start_time,
                500,
            )
            .await;

            error_response(StatusCode::BAD_GATEWAY, &format!("Failed to resume job: {}", e))
        },
    }
}

/// POST /api/jobs/{job_id}/run — trigger/run a job immediately
pub async fn trigger_job(
    State(state): State<GatewayAppState>,
    Extension(auth): Extension<AuthenticatedKey>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let AuthenticatedKey(gateway_key) = auth;
    let start_time = Instant::now();

    let (adapter, ctx, provider_id) = match jobs_ctx(&state).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match adapter.trigger_job(&ctx, &job_id).await {
        Ok(_) => {
            jobs_log(
                &state,
                &gateway_key,
                "POST",
                &format!("/api/jobs/{}/run", job_id),
                &provider_id,
                start_time,
                200,
            )
            .await;

            Json(json!({ "triggered": true, "id": job_id })).into_response()
        },
        Err(e) => {
            jobs_log(
                &state,
                &gateway_key,
                "POST",
                &format!("/api/jobs/{}/run", job_id),
                &provider_id,
                start_time,
                500,
            )
            .await;

            error_response(StatusCode::BAD_GATEWAY, &format!("Failed to trigger job: {}", e))
        },
    }
}

/// GET /api/jobs/{job_id}/runs — list runs for a job
pub async fn list_runs(
    State(state): State<GatewayAppState>,
    Extension(auth): Extension<AuthenticatedKey>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let AuthenticatedKey(gateway_key) = auth;
    let start_time = Instant::now();

    let (adapter, ctx, provider_id) = match jobs_ctx(&state).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match adapter.list_runs(&ctx, &job_id).await {
        Ok(response_body) => {
            jobs_log(
                &state,
                &gateway_key,
                "GET",
                &format!("/api/jobs/{}/runs", job_id),
                &provider_id,
                start_time,
                200,
            )
            .await;

            jobs_ok_json(StatusCode::OK, response_body)
        },
        Err(e) => {
            jobs_log(
                &state,
                &gateway_key,
                "GET",
                &format!("/api/jobs/{}/runs", job_id),
                &provider_id,
                start_time,
                500,
            )
            .await;
            error_response(StatusCode::BAD_GATEWAY, &format!("Failed to list runs: {}", e))
        },
    }
}

/// POST /api/jobs/{job_id}/runs — trigger a new run
pub async fn trigger_run(
    State(state): State<GatewayAppState>,
    Extension(auth): Extension<AuthenticatedKey>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
    axum::extract::Json(params): axum::extract::Json<serde_json::Value>,
) -> impl IntoResponse {
    let AuthenticatedKey(gateway_key) = auth;
    let start_time = Instant::now();

    let (adapter, ctx, provider_id) = match jobs_ctx(&state).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let params_str = serde_json::to_string(&params)
        .unwrap_or_else(|e| format!("{{\"error\":\"Serialization failed: {}\"}}", e));

    match adapter.trigger_run(&ctx, &job_id, Some(&params_str)).await {
        Ok(response_body) => {
            jobs_log(
                &state,
                &gateway_key,
                "POST",
                &format!("/api/jobs/{}/runs", job_id),
                &provider_id,
                start_time,
                201,
            )
            .await;

            jobs_ok_json(StatusCode::CREATED, response_body)
        },
        Err(e) => {
            jobs_log(
                &state,
                &gateway_key,
                "POST",
                &format!("/api/jobs/{}/runs", job_id),
                &provider_id,
                start_time,
                500,
            )
            .await;
            error_response(StatusCode::BAD_GATEWAY, &format!("Failed to trigger run: {}", e))
        },
    }
}

/// GET /api/jobs/{job_id}/runs/{run_id} — get a specific run
pub async fn get_run(
    State(state): State<GatewayAppState>,
    Extension(auth): Extension<AuthenticatedKey>,
    axum::extract::Path((job_id, run_id)): axum::extract::Path<(String, String)>,
) -> impl IntoResponse {
    let AuthenticatedKey(gateway_key) = auth;
    let start_time = Instant::now();

    let (adapter, ctx, provider_id) = match jobs_ctx(&state).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match adapter.get_run(&ctx, &job_id, &run_id).await {
        Ok(response_body) => {
            jobs_log(
                &state,
                &gateway_key,
                "GET",
                &format!("/api/jobs/{}/runs/{}", job_id, run_id),
                &provider_id,
                start_time,
                200,
            )
            .await;

            jobs_ok_json(StatusCode::OK, response_body)
        },
        Err(e) => {
            jobs_log(
                &state,
                &gateway_key,
                "GET",
                &format!("/api/jobs/{}/runs/{}", job_id, run_id),
                &provider_id,
                start_time,
                500,
            )
            .await;
            error_response(StatusCode::BAD_GATEWAY, &format!("Failed to get run: {}", e))
        },
    }
}

/// POST /api/jobs/{job_id}/runs/{run_id}/cancel — cancel a run
pub async fn cancel_run(
    State(state): State<GatewayAppState>,
    Extension(auth): Extension<AuthenticatedKey>,
    axum::extract::Path((job_id, run_id)): axum::extract::Path<(String, String)>,
) -> impl IntoResponse {
    let AuthenticatedKey(gateway_key) = auth;
    let start_time = Instant::now();

    let (adapter, ctx, provider_id) = match jobs_ctx(&state).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match adapter.cancel_run(&ctx, &job_id, &run_id).await {
        Ok(_) => {
            jobs_log(
                &state,
                &gateway_key,
                "POST",
                &format!("/api/jobs/{}/runs/{}/cancel", job_id, run_id),
                &provider_id,
                start_time,
                200,
            )
            .await;
            Json(json!({ "cancelled": true, "job_id": job_id, "run_id": run_id })).into_response()
        },
        Err(e) => {
            jobs_log(
                &state,
                &gateway_key,
                "POST",
                &format!("/api/jobs/{}/runs/{}/cancel", job_id, run_id),
                &provider_id,
                start_time,
                500,
            )
            .await;
            error_response(StatusCode::BAD_GATEWAY, &format!("Failed to cancel run: {}", e))
        },
    }
}

/// GET /api/jobs/{job_id}/runs/{run_id}/logs — get run logs
pub async fn get_run_logs(
    State(state): State<GatewayAppState>,
    Extension(auth): Extension<AuthenticatedKey>,
    axum::extract::Path((job_id, run_id)): axum::extract::Path<(String, String)>,
) -> impl IntoResponse {
    let AuthenticatedKey(gateway_key) = auth;
    let start_time = Instant::now();

    let (adapter, ctx, provider_id) = match jobs_ctx(&state).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match adapter.get_run_logs(&ctx, &job_id, &run_id).await {
        Ok(response_body) => {
            jobs_log(
                &state,
                &gateway_key,
                "GET",
                &format!("/api/jobs/{}/runs/{}/logs", job_id, run_id),
                &provider_id,
                start_time,
                200,
            )
            .await;

            jobs_ok_json(StatusCode::OK, response_body)
        },
        Err(e) => {
            jobs_log(
                &state,
                &gateway_key,
                "GET",
                &format!("/api/jobs/{}/runs/{}/logs", job_id, run_id),
                &provider_id,
                start_time,
                500,
            )
            .await;
            error_response(StatusCode::BAD_GATEWAY, &format!("Failed to get run logs: {}", e))
        },
    }
}

/// POST /api/jobs/{job_id}/runs/{run_id}/retry — retry a run
pub async fn retry_run(
    State(state): State<GatewayAppState>,
    Extension(auth): Extension<AuthenticatedKey>,
    axum::extract::Path((job_id, run_id)): axum::extract::Path<(String, String)>,
) -> impl IntoResponse {
    let AuthenticatedKey(gateway_key) = auth;
    let start_time = Instant::now();

    let (adapter, ctx, provider_id) = match jobs_ctx(&state).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match adapter.retry_run(&ctx, &job_id, &run_id).await {
        Ok(response_body) => {
            jobs_log(
                &state,
                &gateway_key,
                "POST",
                &format!("/api/jobs/{}/runs/{}/retry", job_id, run_id),
                &provider_id,
                start_time,
                200,
            )
            .await;

            jobs_ok_json(StatusCode::OK, response_body)
        },
        Err(e) => {
            jobs_log(
                &state,
                &gateway_key,
                "POST",
                &format!("/api/jobs/{}/runs/{}/retry", job_id, run_id),
                &provider_id,
                start_time,
                500,
            )
            .await;
            error_response(StatusCode::BAD_GATEWAY, &format!("Failed to retry run: {}", e))
        },
    }
}

/// GET /api/jobs/{job_id}/schedule — get job schedule
pub async fn get_job_schedule(
    State(state): State<GatewayAppState>,
    Extension(auth): Extension<AuthenticatedKey>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let AuthenticatedKey(gateway_key) = auth;
    let start_time = Instant::now();

    let (adapter, ctx, provider_id) = match jobs_ctx(&state).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match adapter.get_job_schedule(&ctx, &job_id).await {
        Ok(response_body) => {
            jobs_log(
                &state,
                &gateway_key,
                "GET",
                &format!("/api/jobs/{}/schedule", job_id),
                &provider_id,
                start_time,
                200,
            )
            .await;

            jobs_ok_json(StatusCode::OK, response_body)
        },
        Err(e) => {
            jobs_log(
                &state,
                &gateway_key,
                "GET",
                &format!("/api/jobs/{}/schedule", job_id),
                &provider_id,
                start_time,
                500,
            )
            .await;
            error_response(StatusCode::BAD_GATEWAY, &format!("Failed to get job schedule: {}", e))
        },
    }
}

/// PUT /api/jobs/{job_id}/schedule — update job schedule
pub async fn update_job_schedule(
    State(state): State<GatewayAppState>,
    Extension(auth): Extension<AuthenticatedKey>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
    axum::extract::Json(schedule): axum::extract::Json<serde_json::Value>,
) -> impl IntoResponse {
    let AuthenticatedKey(gateway_key) = auth;
    let start_time = Instant::now();

    let (adapter, ctx, provider_id) = match jobs_ctx(&state).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let schedule_str = serde_json::to_string(&schedule)
        .unwrap_or_else(|e| format!("{{\"error\":\"Serialization failed: {}\"}}", e));

    match adapter.update_job_schedule(&ctx, &job_id, &schedule_str).await {
        Ok(response_body) => {
            jobs_log(
                &state,
                &gateway_key,
                "PUT",
                &format!("/api/jobs/{}/schedule", job_id),
                &provider_id,
                start_time,
                200,
            )
            .await;

            jobs_ok_json(StatusCode::OK, response_body)
        },
        Err(e) => {
            jobs_log(
                &state,
                &gateway_key,
                "PUT",
                &format!("/api/jobs/{}/schedule", job_id),
                &provider_id,
                start_time,
                500,
            )
            .await;
            error_response(
                StatusCode::BAD_GATEWAY,
                &format!("Failed to update job schedule: {}", e),
            )
        },
    }
}

/// POST /api/jobs/{job_id}/enable — enable a job
pub async fn enable_job(
    State(state): State<GatewayAppState>,
    Extension(auth): Extension<AuthenticatedKey>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let AuthenticatedKey(gateway_key) = auth;
    let start_time = Instant::now();

    let (adapter, ctx, provider_id) = match jobs_ctx(&state).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match adapter.enable_job(&ctx, &job_id).await {
        Ok(_) => {
            jobs_log(
                &state,
                &gateway_key,
                "POST",
                &format!("/api/jobs/{}/enable", job_id),
                &provider_id,
                start_time,
                200,
            )
            .await;
            Json(json!({ "enabled": true, "id": job_id })).into_response()
        },
        Err(e) => {
            jobs_log(
                &state,
                &gateway_key,
                "POST",
                &format!("/api/jobs/{}/enable", job_id),
                &provider_id,
                start_time,
                500,
            )
            .await;
            error_response(StatusCode::BAD_GATEWAY, &format!("Failed to enable job: {}", e))
        },
    }
}

/// POST /api/jobs/{job_id}/disable — disable a job
pub async fn disable_job(
    State(state): State<GatewayAppState>,
    Extension(auth): Extension<AuthenticatedKey>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let AuthenticatedKey(gateway_key) = auth;
    let start_time = Instant::now();

    let (adapter, ctx, provider_id) = match jobs_ctx(&state).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match adapter.disable_job(&ctx, &job_id).await {
        Ok(_) => {
            jobs_log(
                &state,
                &gateway_key,
                "POST",
                &format!("/api/jobs/{}/disable", job_id),
                &provider_id,
                start_time,
                200,
            )
            .await;
            Json(json!({ "disabled": true, "id": job_id })).into_response()
        },
        Err(e) => {
            jobs_log(
                &state,
                &gateway_key,
                "POST",
                &format!("/api/jobs/{}/disable", job_id),
                &provider_id,
                start_time,
                500,
            )
            .await;
            error_response(StatusCode::BAD_GATEWAY, &format!("Failed to disable job: {}", e))
        },
    }
}
