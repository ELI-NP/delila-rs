pub mod config;
pub mod process;
pub mod routes;

use std::sync::Arc;

use axum::Router;

pub use config::AgentFileConfig;

pub fn build(config: AgentFileConfig) -> (Arc<routes::AppState>, Router) {
    let manager = process::ProcessManager::new(config.process, config.agent.log_buffer_lines);

    let state = Arc::new(routes::AppState {
        manager,
        start_time: std::time::Instant::now(),
        agent_name: config.agent.name,
    });

    let router = routes::build_router(Arc::clone(&state));
    (state, router)
}
