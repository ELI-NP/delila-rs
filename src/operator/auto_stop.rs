//! Notice of a run the Operator stopped on its own (TODO 84 §8.3).
//!
//! The backlog autostop (TODO 68) and the run guard (TODO 69) stop a run
//! without anyone pressing Stop. Their log lines are easy to miss, so the
//! Operator keeps one notice — what stopped which run, why, and whether the
//! queued data was written — and reports it in `/api/status` until the next
//! run starts. The UI shows it as a banner.

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use utoipa::ToSchema;

/// Why the Operator stopped the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AutoStopCause {
    /// A pipeline component's queue crossed its hard watermark: the input rate
    /// is higher than the recording path can write (TODO 68).
    RateTooHigh,
    /// Free disk space fell below the hard watermark (TODO 69).
    DiskLow,
    /// A file write failed (TODO 69).
    WriteFailure,
}

/// How far the automatic stop has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AutoStopPhase {
    /// Sources are being stopped; for a drain-first stop the queued data is
    /// still being written.
    Stopping,
    /// The run is stopped (for a drain-first stop: every queued event written).
    Stopped,
    /// The stop finished with a problem (e.g. drain incomplete); see `message`.
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct AutoStopNotice {
    pub cause: AutoStopCause,
    pub phase: AutoStopPhase,
    /// The run that was stopped (None if no run number was known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_number: Option<i32>,
    /// What triggered it, e.g. "Recorder: 21.0 GB waiting to be written".
    pub reason: String,
    /// Unix time in ms when the stop began (also identifies the notice).
    pub started_ms: i64,
    /// Result of the stop, empty while it is in progress.
    #[serde(default)]
    pub message: String,
}

/// Record that an automatic stop of `run_number` has begun.
pub async fn begin(
    slot: &RwLock<Option<AutoStopNotice>>,
    run_number: Option<i32>,
    cause: AutoStopCause,
    reason: &str,
) {
    *slot.write().await = Some(AutoStopNotice {
        cause,
        phase: AutoStopPhase::Stopping,
        run_number,
        reason: reason.to_string(),
        started_ms: chrono::Utc::now().timestamp_millis(),
        message: String::new(),
    });
}

/// Record how the automatic stop ended.
pub async fn finish(slot: &RwLock<Option<AutoStopNotice>>, success: bool, message: &str) {
    if let Some(n) = slot.write().await.as_mut() {
        n.phase = if success {
            AutoStopPhase::Stopped
        } else {
            AutoStopPhase::Incomplete
        };
        n.message = message.to_string();
    }
}

/// "21.0 GB" for a queue size in bytes.
pub fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn begin_then_finish_records_the_outcome() {
        let slot = RwLock::new(None);
        begin(
            &slot,
            Some(45),
            AutoStopCause::RateTooHigh,
            "Recorder: 21.0 GB",
        )
        .await;
        let n = slot.read().await.clone().expect("notice");
        assert_eq!(n.phase, AutoStopPhase::Stopping);
        assert_eq!(n.run_number, Some(45));
        assert!(n.started_ms > 0);

        finish(&slot, true, "Drain-first stop complete").await;
        let n = slot.read().await.clone().expect("notice");
        assert_eq!(n.phase, AutoStopPhase::Stopped);
        assert_eq!(n.message, "Drain-first stop complete");

        finish(&slot, false, "DRAIN INCOMPLETE").await;
        assert_eq!(
            slot.read().await.as_ref().map(|n| n.phase),
            Some(AutoStopPhase::Incomplete)
        );
    }

    #[tokio::test]
    async fn finish_without_a_notice_does_nothing() {
        let slot = RwLock::new(None);
        finish(&slot, true, "x").await;
        assert!(slot.read().await.is_none());
    }

    #[test]
    fn serializes_in_snake_case_for_the_ui() {
        let n = AutoStopNotice {
            cause: AutoStopCause::RateTooHigh,
            phase: AutoStopPhase::Stopping,
            run_number: Some(7),
            reason: "r".into(),
            started_ms: 1,
            message: String::new(),
        };
        let json = serde_json::to_string(&n).expect("json");
        assert!(json.contains("\"cause\":\"rate_too_high\""), "{json}");
        assert!(json.contains("\"phase\":\"stopping\""), "{json}");
        assert_eq!(gb(21_036_435_473), "21.0 GB");
    }
}
