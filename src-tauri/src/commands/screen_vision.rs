// SPDX-License-Identifier: AGPL-3.0-only

//! 视觉命令面（无障碍树语义化阶段3）。
//!
//! 只保留 `analyze_image`（用户上传图片分析，独立能力）。
//! 原 5 条「截图 + 坐标 + 视觉定位」命令已被无障碍树语义命令取代。

use axagent_agent_macro::agent_command;
use tauri::State;

use crate::AppState;
use crate::commands::provider_ctx::{VisionContext, build_vision_context};

#[agent_command(domain = vision, safety = Safe, call_mode = StateInput, description = "分析图片")]
#[tauri::command]
pub async fn analyze_image(
    state: State<'_, AppState>,
    image_base64: String,
    task: String,
    provider_id: String,
    model_id: String,
) -> Result<axagent_agent::VisionResult, String> {
    let task_enum: axagent_agent::VisionTask = serde_json::from_str(&format!("\"{}\"", task))
        .map_err(|e| format!("Invalid vision task '{}': {}", task, e))?;

    let image_data = if let Some(stripped) = image_base64.strip_prefix("data:image/png;base64,") {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(stripped)
            .map_err(|e| format!("Failed to decode base64 image: {}", e))?
    } else {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(&image_base64)
            .map_err(|e| format!("Failed to decode base64 image: {}", e))?
    };

    let VisionContext { adapter, ctx } =
        build_vision_context(state.harness.db(), state.harness.master_key(), &provider_id).await?;

    let pipeline = axagent_agent::VisionPipeline::new(adapter, ctx, model_id);

    pipeline
        .analyze(&image_data, task_enum)
        .await
        .map_err(|e| format!("Image analysis failed: {}", e))
}
