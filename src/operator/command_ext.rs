//! Operator-side `CommandHandlerExt` shim.
//!
//! R-P7 (Phase 1 refactor sprint 2026-Q2). The other 6 pipeline components
//! (Reader, Merger, Recorder, Monitor, DataSink, Emulator) all implement
//! [`CommandHandlerExt`] so the `command_task.rs` plumbing handles their
//! Configure / Arm / Start / Stop / Reset / GetStatus uniformly. Operator
//! sits outside that loop — it's the REST front-end *sending* those
//! commands rather than receiving them — so historically it skipped the
//! trait entirely (audit: 6/9 implementers, Operator missing).
//!
//! Phase 1 lands a **stateless shim** that satisfies the trait surface so
//! the Phase 3 [R-P8 `ComponentRunner`] can enumerate Operator as the 7th
//! component without special-casing. The actual integration (reading
//! `current_run` / `tuneup_mode` from `AppState` snapshots, replacing the
//! direct `RwLock` fan-out in `routes/status.rs`) is deferred to Phase 2
//! R-P3 (`AppState` RwLock → DashMap/RCU read cache) — that refactor is
//! where lock contention actually matters.
//!
//! ## Why stateless
//!
//! `AppState` is held under `Arc<RwLock<...>>` and the trait methods are
//! sync. Wiring the actual snapshot here would force `block_on` from a
//! sync context, which the axum runtime forbids. A stateless shim keeps
//! the surface honest (no fake state, no panics) until Phase 2 R-P3
//! reshapes `AppState` so reads can happen sync-cheap.
//!
//! [R-P8 `ComponentRunner`]: ../../docs/component_architecture.md

use crate::common::state::CommandHandlerExt;
use crate::common::ComponentMetrics;

/// Stateless `CommandHandlerExt` impl for Operator.
///
/// Identifies Operator as a pipeline component for trait-driven enumeration.
/// All hooks return their default (no-op) implementations — Operator does
/// not directly process Configure / Arm / Start / Stop transitions; it
/// orchestrates them in the other components via REST + ZMQ REQ.
#[derive(Debug, Default, Clone, Copy)]
pub struct OperatorCommandExt;

impl OperatorCommandExt {
    pub const fn new() -> Self {
        Self
    }
}

impl CommandHandlerExt for OperatorCommandExt {
    fn component_name(&self) -> &'static str {
        "Operator"
    }

    /// Operator does not export DAQ-pipeline metrics (event count / bytes).
    /// REST traffic counters could be added in Phase 2 R-P3 when AppState
    /// gains a snapshot path; for now the trait reports None.
    fn get_metrics(&self) -> Option<ComponentMetrics> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_reports_its_name() {
        let ext = OperatorCommandExt::new();
        assert_eq!(ext.component_name(), "Operator");
    }

    #[test]
    fn operator_reports_no_metrics() {
        let ext = OperatorCommandExt::new();
        assert!(ext.get_metrics().is_none());
    }

    /// Default `effective_state` should be a passthrough — Operator has no
    /// hardware state, only the software target state from AppState.
    #[test]
    fn operator_effective_state_passes_through() {
        use crate::common::ComponentState;
        let ext = OperatorCommandExt::new();
        for s in [
            ComponentState::Idle,
            ComponentState::Configured,
            ComponentState::Armed,
            ComponentState::Running,
            ComponentState::Error,
        ] {
            assert_eq!(ext.effective_state(s), s);
        }
    }
}
