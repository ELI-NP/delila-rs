//! Monitor layout persistence (GET/PUT as opaque JSON)

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};

use super::{ApiResponse, AppState};

/// Get the saved monitor layout
pub(super) async fn get_monitor_layout(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let layout = state.monitor_layout.read().await;
    Json(layout.clone())
}

/// Save monitor layout to memory and disk
pub(super) async fn save_monitor_layout(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<ApiResponse>) {
    // Update in-memory state
    {
        let mut layout = state.monitor_layout.write().await;
        *layout = body.clone();
    }

    // Persist to disk
    let file_path = state.config_dir.join("monitor_layout.json");
    if let Some(parent) = file_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!("Failed to create directory for monitor layout: {e}");
        }
    }

    match serde_json::to_string_pretty(&body) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&file_path, json) {
                tracing::warn!(
                    "Failed to write monitor layout to {}: {e}",
                    file_path.display()
                );
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse::error(format!("Failed to save layout: {e}"))),
                );
            }
            tracing::debug!("Monitor layout saved to {}", file_path.display());
        }
        Err(e) => {
            tracing::warn!("Failed to serialize monitor layout: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::error(format!(
                    "Failed to serialize layout: {e}"
                ))),
            );
        }
    }

    (StatusCode::OK, Json(ApiResponse::success("Layout saved")))
}

/// Load monitor layout from disk (returns default empty object if not found)
pub fn load_monitor_layout(config_dir: &std::path::Path) -> serde_json::Value {
    let file_path = config_dir.join("monitor_layout.json");
    match std::fs::read_to_string(&file_path) {
        Ok(contents) => match serde_json::from_str(&contents) {
            Ok(value) => {
                tracing::info!("Loaded monitor layout from {}", file_path.display());
                value
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to parse monitor layout from {}: {e}",
                    file_path.display()
                );
                serde_json::json!({})
            }
        },
        Err(_) => {
            tracing::debug!("No monitor layout file at {}", file_path.display());
            serde_json::json!({})
        }
    }
}
