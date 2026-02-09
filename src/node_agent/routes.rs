use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};

use super::process::{ProcessManager, ProcessStatus};

pub struct AppState {
    pub manager: ProcessManager,
    pub start_time: std::time::Instant,
    pub agent_name: String,
}

#[derive(Debug, Serialize)]
pub struct AgentStatus {
    pub agent_name: String,
    pub uptime_secs: u64,
    pub processes: Vec<ProcessStatus>,
}

#[derive(Debug, Serialize)]
pub struct ApiResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    pub tail: Option<usize>,
}

pub fn build_router(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/api/status", get(get_status))
        .route("/api/processes/:name", get(get_process))
        .route("/api/processes/:name/start", post(start_process))
        .route("/api/processes/:name/stop", post(stop_process))
        .route("/api/processes/:name/restart", post(restart_process))
        .route("/api/processes/:name/logs", get(get_logs))
        .route("/api/start-all", post(start_all))
        .route("/api/stop-all", post(stop_all))
        .layer(cors)
        .with_state(state)
}

async fn get_status(State(state): State<Arc<AppState>>) -> Json<AgentStatus> {
    let processes = state.manager.all_status().await;
    Json(AgentStatus {
        agent_name: state.agent_name.clone(),
        uptime_secs: state.start_time.elapsed().as_secs(),
        processes,
    })
}

async fn get_process(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<ProcessStatus>, StatusCode> {
    state
        .manager
        .get_status(&name)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn start_process(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> (StatusCode, Json<ApiResponse>) {
    match state.manager.start(&name).await {
        Ok(()) => (
            StatusCode::OK,
            Json(ApiResponse {
                success: true,
                message: format!("{} started", name),
            }),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                success: false,
                message: e,
            }),
        ),
    }
}

async fn stop_process(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> (StatusCode, Json<ApiResponse>) {
    match state.manager.stop(&name).await {
        Ok(()) => (
            StatusCode::OK,
            Json(ApiResponse {
                success: true,
                message: format!("{} stopped", name),
            }),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                success: false,
                message: e,
            }),
        ),
    }
}

async fn restart_process(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> (StatusCode, Json<ApiResponse>) {
    match state.manager.restart(&name).await {
        Ok(()) => (
            StatusCode::OK,
            Json(ApiResponse {
                success: true,
                message: format!("{} restarted", name),
            }),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                success: false,
                message: e,
            }),
        ),
    }
}

async fn start_all(State(state): State<Arc<AppState>>) -> Json<ApiResponse> {
    state.manager.start_all().await;
    Json(ApiResponse {
        success: true,
        message: "All processes started".into(),
    })
}

async fn stop_all(State(state): State<Arc<AppState>>) -> Json<ApiResponse> {
    state.manager.stop_all().await;
    Json(ApiResponse {
        success: true,
        message: "All processes stopped".into(),
    })
}

async fn get_logs(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(query): Query<LogQuery>,
) -> Result<Json<super::process::LogResponse>, StatusCode> {
    state
        .manager
        .get_logs(&name, query.tail)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}
