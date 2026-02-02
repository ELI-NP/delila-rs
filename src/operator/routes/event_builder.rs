//! Event Builder configuration API endpoints

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::event_builder::{ChannelConfig, L2Settings, TimeCalibration};

use super::super::{ApiResponse, EventBuilderConfigDocument};
use super::AppState;

/// Query parameters for listing configs
#[derive(Debug, Deserialize)]
pub struct ListConfigsQuery {
    pub exp_name: String,
}

/// Query parameters for config history
#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    20
}

/// Request body for creating/updating a config
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateConfigRequest {
    /// Config name
    pub name: String,
    /// Experiment name
    pub exp_name: String,
    /// Optional description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Channel settings
    #[schema(value_type = Vec<Vec<Object>>)]
    pub ch_settings: ChannelConfig,
    /// Time calibration
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub time_settings: Option<TimeCalibration>,
    /// L2 filter settings
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Vec<Object>>)]
    pub l2_settings: Option<L2Settings>,
    /// Coincidence window [ns]
    #[serde(default = "default_coincidence_window")]
    pub coincidence_window_ns: f64,
    /// Slice duration [ns]
    #[serde(default = "default_slice_duration")]
    pub slice_duration_ns: f64,
}

fn default_coincidence_window() -> f64 {
    500.0
}

fn default_slice_duration() -> f64 {
    10_000_000.0
}

/// Request body for updating ch_settings only
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateChSettingsRequest {
    #[schema(value_type = Vec<Vec<Object>>)]
    pub ch_settings: ChannelConfig,
}

/// Request body for updating time_settings only
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateTimeSettingsRequest {
    #[schema(value_type = Object)]
    pub time_settings: TimeCalibration,
}

/// Request body for updating l2_settings only
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateL2SettingsRequest {
    #[schema(value_type = Vec<Object>)]
    pub l2_settings: L2Settings,
}

/// Request body for restore version
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RestoreVersionRequest {
    pub version: u32,
}

/// Config history item (simplified)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConfigHistoryItem {
    pub version: u32,
    #[schema(value_type = String, format = "date-time")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub created_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub is_current: bool,
}

impl From<EventBuilderConfigDocument> for ConfigHistoryItem {
    fn from(doc: EventBuilderConfigDocument) -> Self {
        Self {
            version: doc.version,
            created_at: doc.created_at,
            created_by: doc.created_by,
            description: doc.description,
            is_current: doc.is_current,
        }
    }
}

/// List all experiments with Event Builder configs
#[utoipa::path(
    get,
    path = "/api/event-builder/experiments",
    tag = "Event Builder",
    responses(
        (status = 200, description = "List of experiment names", body = Vec<String>),
        (status = 503, description = "MongoDB not available", body = ApiResponse)
    )
)]
pub(super) async fn list_experiments(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<String>>, (StatusCode, Json<ApiResponse>)> {
    let repo = state.event_builder_repo.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse::error(
                "MongoDB not configured for Event Builder",
            )),
        )
    })?;

    let experiments = repo.list_experiments().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::error(format!(
                "Failed to list experiments: {}",
                e
            ))),
        )
    })?;

    Ok(Json(experiments))
}

/// List Event Builder configurations for an experiment
#[utoipa::path(
    get,
    path = "/api/event-builder/configs",
    tag = "Event Builder",
    params(
        ("exp_name" = String, Query, description = "Experiment name")
    ),
    responses(
        (status = 200, description = "List of configurations", body = Vec<EventBuilderConfigDocument>),
        (status = 503, description = "MongoDB not available", body = ApiResponse)
    )
)]
pub(super) async fn list_configs(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListConfigsQuery>,
) -> Result<Json<Vec<EventBuilderConfigDocument>>, (StatusCode, Json<ApiResponse>)> {
    let repo = state.event_builder_repo.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse::error(
                "MongoDB not configured for Event Builder",
            )),
        )
    })?;

    let configs = repo.list_configs(&query.exp_name).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::error(format!("Failed to list configs: {}", e))),
        )
    })?;

    Ok(Json(configs))
}

/// Get a specific Event Builder configuration
#[utoipa::path(
    get,
    path = "/api/event-builder/configs/{exp_name}/{name}",
    tag = "Event Builder",
    params(
        ("exp_name" = String, Path, description = "Experiment name"),
        ("name" = String, Path, description = "Configuration name")
    ),
    responses(
        (status = 200, description = "Configuration", body = EventBuilderConfigDocument),
        (status = 404, description = "Config not found", body = ApiResponse),
        (status = 503, description = "MongoDB not available", body = ApiResponse)
    )
)]
pub(super) async fn get_config(
    State(state): State<Arc<AppState>>,
    Path((exp_name, name)): Path<(String, String)>,
) -> Result<Json<EventBuilderConfigDocument>, (StatusCode, Json<ApiResponse>)> {
    let repo = state.event_builder_repo.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse::error(
                "MongoDB not configured for Event Builder",
            )),
        )
    })?;

    let config = repo
        .get_current_config(&name, &exp_name)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::error(format!("Failed to get config: {}", e))),
            )
        })?;

    config.map(Json).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::error(format!(
                "Config not found: {}:{}",
                exp_name, name
            ))),
        )
    })
}

/// Create or update an Event Builder configuration
#[utoipa::path(
    post,
    path = "/api/event-builder/configs",
    tag = "Event Builder",
    request_body = CreateConfigRequest,
    responses(
        (status = 200, description = "Configuration saved", body = EventBuilderConfigDocument),
        (status = 503, description = "MongoDB not available", body = ApiResponse)
    )
)]
pub(super) async fn create_config(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateConfigRequest>,
) -> Result<Json<EventBuilderConfigDocument>, (StatusCode, Json<ApiResponse>)> {
    let repo = state.event_builder_repo.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse::error(
                "MongoDB not configured for Event Builder",
            )),
        )
    })?;

    let doc = EventBuilderConfigDocument {
        id: None,
        name: request.name,
        exp_name: request.exp_name,
        version: 0, // Will be set by repository
        created_at: chrono::Utc::now(),
        created_by: String::new(), // Will be set by repository
        description: request.description,
        is_current: true,
        ch_settings: request.ch_settings,
        time_settings: request.time_settings,
        l2_settings: request.l2_settings,
        coincidence_window_ns: request.coincidence_window_ns,
        slice_duration_ns: request.slice_duration_ns,
    };

    let saved = repo.save_config(doc, "api").await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::error(format!("Failed to save config: {}", e))),
        )
    })?;

    Ok(Json(saved))
}

/// Update chSettings for a configuration
#[utoipa::path(
    put,
    path = "/api/event-builder/configs/{exp_name}/{name}/ch-settings",
    tag = "Event Builder",
    params(
        ("exp_name" = String, Path, description = "Experiment name"),
        ("name" = String, Path, description = "Configuration name")
    ),
    request_body = UpdateChSettingsRequest,
    responses(
        (status = 200, description = "chSettings updated", body = EventBuilderConfigDocument),
        (status = 404, description = "Config not found", body = ApiResponse),
        (status = 503, description = "MongoDB not available", body = ApiResponse)
    )
)]
pub(super) async fn update_ch_settings(
    State(state): State<Arc<AppState>>,
    Path((exp_name, name)): Path<(String, String)>,
    Json(request): Json<UpdateChSettingsRequest>,
) -> Result<Json<EventBuilderConfigDocument>, (StatusCode, Json<ApiResponse>)> {
    let repo = state.event_builder_repo.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse::error(
                "MongoDB not configured for Event Builder",
            )),
        )
    })?;

    let updated = repo
        .update_ch_settings(&name, &exp_name, request.ch_settings, "api")
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::error(format!(
                    "Failed to update chSettings: {}",
                    e
                ))),
            )
        })?;

    Ok(Json(updated))
}

/// Update timeSettings for a configuration
#[utoipa::path(
    put,
    path = "/api/event-builder/configs/{exp_name}/{name}/time-settings",
    tag = "Event Builder",
    params(
        ("exp_name" = String, Path, description = "Experiment name"),
        ("name" = String, Path, description = "Configuration name")
    ),
    request_body = UpdateTimeSettingsRequest,
    responses(
        (status = 200, description = "timeSettings updated", body = EventBuilderConfigDocument),
        (status = 404, description = "Config not found", body = ApiResponse),
        (status = 503, description = "MongoDB not available", body = ApiResponse)
    )
)]
pub(super) async fn update_time_settings(
    State(state): State<Arc<AppState>>,
    Path((exp_name, name)): Path<(String, String)>,
    Json(request): Json<UpdateTimeSettingsRequest>,
) -> Result<Json<EventBuilderConfigDocument>, (StatusCode, Json<ApiResponse>)> {
    let repo = state.event_builder_repo.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse::error(
                "MongoDB not configured for Event Builder",
            )),
        )
    })?;

    let updated = repo
        .update_time_settings(&name, &exp_name, request.time_settings, "api")
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::error(format!(
                    "Failed to update timeSettings: {}",
                    e
                ))),
            )
        })?;

    Ok(Json(updated))
}

/// Update L2Settings for a configuration
#[utoipa::path(
    put,
    path = "/api/event-builder/configs/{exp_name}/{name}/l2-settings",
    tag = "Event Builder",
    params(
        ("exp_name" = String, Path, description = "Experiment name"),
        ("name" = String, Path, description = "Configuration name")
    ),
    request_body = UpdateL2SettingsRequest,
    responses(
        (status = 200, description = "L2Settings updated", body = EventBuilderConfigDocument),
        (status = 404, description = "Config not found", body = ApiResponse),
        (status = 503, description = "MongoDB not available", body = ApiResponse)
    )
)]
pub(super) async fn update_l2_settings(
    State(state): State<Arc<AppState>>,
    Path((exp_name, name)): Path<(String, String)>,
    Json(request): Json<UpdateL2SettingsRequest>,
) -> Result<Json<EventBuilderConfigDocument>, (StatusCode, Json<ApiResponse>)> {
    let repo = state.event_builder_repo.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse::error(
                "MongoDB not configured for Event Builder",
            )),
        )
    })?;

    let updated = repo
        .update_l2_settings(&name, &exp_name, request.l2_settings, "api")
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::error(format!(
                    "Failed to update L2Settings: {}",
                    e
                ))),
            )
        })?;

    Ok(Json(updated))
}

/// Get version history for a configuration
#[utoipa::path(
    get,
    path = "/api/event-builder/configs/{exp_name}/{name}/history",
    tag = "Event Builder",
    params(
        ("exp_name" = String, Path, description = "Experiment name"),
        ("name" = String, Path, description = "Configuration name"),
        ("limit" = Option<i64>, Query, description = "Max versions to return (default: 20)")
    ),
    responses(
        (status = 200, description = "Version history", body = Vec<ConfigHistoryItem>),
        (status = 503, description = "MongoDB not available", body = ApiResponse)
    )
)]
pub(super) async fn get_config_history(
    State(state): State<Arc<AppState>>,
    Path((exp_name, name)): Path<(String, String)>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Vec<ConfigHistoryItem>>, (StatusCode, Json<ApiResponse>)> {
    let repo = state.event_builder_repo.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse::error(
                "MongoDB not configured for Event Builder",
            )),
        )
    })?;

    let history = repo
        .get_config_history(&name, &exp_name, query.limit)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::error(format!("Failed to get history: {}", e))),
            )
        })?;

    Ok(Json(history.into_iter().map(Into::into).collect()))
}

/// Restore a specific version of a configuration
#[utoipa::path(
    post,
    path = "/api/event-builder/configs/{exp_name}/{name}/restore",
    tag = "Event Builder",
    params(
        ("exp_name" = String, Path, description = "Experiment name"),
        ("name" = String, Path, description = "Configuration name")
    ),
    request_body = RestoreVersionRequest,
    responses(
        (status = 200, description = "Version restored", body = EventBuilderConfigDocument),
        (status = 404, description = "Version not found", body = ApiResponse),
        (status = 503, description = "MongoDB not available", body = ApiResponse)
    )
)]
pub(super) async fn restore_version(
    State(state): State<Arc<AppState>>,
    Path((exp_name, name)): Path<(String, String)>,
    Json(request): Json<RestoreVersionRequest>,
) -> Result<Json<EventBuilderConfigDocument>, (StatusCode, Json<ApiResponse>)> {
    let repo = state.event_builder_repo.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse::error(
                "MongoDB not configured for Event Builder",
            )),
        )
    })?;

    let restored = repo
        .restore_version(&name, &exp_name, request.version)
        .await
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiResponse::error(format!(
                    "Failed to restore version: {}",
                    e
                ))),
            )
        })?;

    Ok(Json(restored))
}

/// Delete a configuration (all versions)
#[utoipa::path(
    delete,
    path = "/api/event-builder/configs/{exp_name}/{name}",
    tag = "Event Builder",
    params(
        ("exp_name" = String, Path, description = "Experiment name"),
        ("name" = String, Path, description = "Configuration name")
    ),
    responses(
        (status = 200, description = "Configuration deleted", body = ApiResponse),
        (status = 503, description = "MongoDB not available", body = ApiResponse)
    )
)]
pub(super) async fn delete_config(
    State(state): State<Arc<AppState>>,
    Path((exp_name, name)): Path<(String, String)>,
) -> (StatusCode, Json<ApiResponse>) {
    let repo = match state.event_builder_repo.as_ref() {
        Some(r) => r,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ApiResponse::error(
                    "MongoDB not configured for Event Builder",
                )),
            );
        }
    };

    match repo.delete_config(&name, &exp_name).await {
        Ok(count) => (
            StatusCode::OK,
            Json(ApiResponse::success(format!(
                "Deleted {} version(s) of {}:{}",
                count, exp_name, name
            ))),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::error(format!(
                "Failed to delete config: {}",
                e
            ))),
        ),
    }
}
