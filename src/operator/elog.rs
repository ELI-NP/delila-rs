//! ELOG electronic logbook auto-posting
//!
//! Posts a Run Summary entry to ELOG on Run Stop via HTTP POST.

use crate::config::ElogConfig;
use crate::operator::RunStats;

/// Post a run summary to ELOG.
///
/// Uses the ELOG HTTP submission API (multipart form POST with `cmd=Submit` in body).
/// Errors are logged but never propagate — ELOG must not block the DAQ.
pub async fn post_run_summary(
    config: &ElogConfig,
    run_number: i32,
    exp_name: &str,
    comment: &str,
    duration_secs: i64,
    stats: &RunStats,
) {
    let hours = duration_secs / 3600;
    let minutes = (duration_secs % 3600) / 60;
    let secs = duration_secs % 60;

    let text = format!(
        "Run #{run_number} completed\n\
         \n\
         Experiment: {exp_name}\n\
         Duration: {hours:02}:{minutes:02}:{secs:02}\n\
         Total events: {events}\n\
         Total bytes: {bytes}\n\
         Average rate: {rate:.1} evt/s\n\
         \n\
         Comment: {comment}",
        events = stats.total_events,
        bytes = format_bytes(stats.total_bytes),
        rate = stats.average_rate,
        comment = if comment.is_empty() { "-" } else { comment },
    );

    let subject = format!("Run #{run_number} - {exp_name}");

    let url = format!("{}/{}/", config.url.trim_end_matches('/'), config.logbook);

    let form = reqwest::multipart::Form::new()
        .text("cmd", "Submit")
        .text("Author", config.author.clone())
        .text("Type", "Run Summary")
        .text("Category", "DAQ")
        .text("Subject", subject)
        .text("Run_Number", run_number.to_string())
        .text("Text", text);

    let client = reqwest::Client::new();
    match client.post(&url).multipart(form).send().await {
        Ok(resp) if resp.status().is_success() || resp.status().is_redirection() => {
            tracing::info!(
                "ELOG: Posted run #{run_number} summary to {}",
                config.logbook
            );
        }
        Ok(resp) => {
            tracing::warn!(
                "ELOG: Unexpected status {} posting run #{run_number}",
                resp.status()
            );
        }
        Err(e) => {
            tracing::warn!("ELOG: Failed to post run #{run_number}: {e}");
        }
    }
}

fn format_bytes(bytes: i64) -> String {
    const KB: i64 = 1024;
    const MB: i64 = 1024 * 1024;
    const GB: i64 = 1024 * 1024 * 1024;
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}
