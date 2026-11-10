//! The end-of-run marker (EOS) of a Reader whose digitizer was lost mid-run
//! (TODO 85 §4; pf-qaq run 70, 2026-11-08).
//!
//! The read loop sends the Stop signal (the decode pipeline then publishes the
//! EOS) from its "stop the acquisition" step, which needs a live connection
//! that is acquiring. When reads keep failing for 30 s the loop drops the
//! connection and reports Error, so the Operator's Stop later finds nothing to
//! stop and no EOS goes out. EOS-driven sinks (nwall_sink, root_sink) wait for
//! an EOS from every source that sent data in the run, so their run file was
//! never finalized. A board that is gone has no more data for the run, so its
//! EOS is still true: [`OwedEos`] keeps track of it and says when to send it.
//!
//! Never mid-run: the Rust Recorder closes its run file on the first EOS of
//! the run, so an EOS while the other boards still record would lose their
//! data. The owed EOS goes out only after the Operator stopped the run (target
//! Configured or Idle), and [`OWED_EOS_DELAY`] later: the live boards drain
//! their buffers first (up to 1 s), so this EOS is never the one that closes
//! the Recorder's file before their last data.
//!
//! The decode pipeline stamps an EOS with the Reader's current run number,
//! which the next Start changes. So the owed EOS goes out at once when the
//! Operator already arms the next run, and is abandoned (with an error) when
//! the next run has started.

use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tracing::{error, info, warn};

use super::ReadLoopOutput;
use crate::common::ComponentState;

/// How long after the Operator's Stop the owed EOS waits (see module docs).
pub(crate) const OWED_EOS_DELAY: Duration = Duration::from_secs(2);

/// What the read loop should do with the owed EOS now.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OwedEosAction {
    Wait,
    Send { run: u32 },
    Abandon { run: u32, current_run: u32 },
}

/// The run whose EOS this Reader still has to send.
#[derive(Debug, Default)]
pub(crate) struct OwedEos {
    run: Option<u32>,
    due: Option<Instant>,
}

impl OwedEos {
    /// The hardware started acquiring for `run`: its EOS is owed from now on.
    pub(crate) fn run_started(&mut self, run: u32) {
        self.run = Some(run);
        self.due = None;
    }

    /// The Stop signal (and so the EOS) was sent: nothing is owed.
    pub(crate) fn stop_sent(&mut self) {
        self.run = None;
        self.due = None;
    }

    /// Called on every read-loop pass, after the normal stop step (which
    /// clears the owed EOS whenever the connection is still there).
    pub(crate) fn poll(
        &mut self,
        target: ComponentState,
        current_run: u32,
        now: Instant,
    ) -> OwedEosAction {
        let Some(run) = self.run else {
            return OwedEosAction::Wait;
        };
        if current_run != run {
            self.stop_sent();
            return OwedEosAction::Abandon { run, current_run };
        }
        match target {
            ComponentState::Configured | ComponentState::Idle => {
                let due = *self.due.get_or_insert(now + OWED_EOS_DELAY);
                if now < due {
                    return OwedEosAction::Wait;
                }
            }
            // The Operator is arming the next run: send before its Start
            // changes the run number.
            ComponentState::Armed => {}
            // Still this run (the Operator has not stopped it): never mid-run.
            ComponentState::Running | ComponentState::Error => return OwedEosAction::Wait,
        }
        self.stop_sent();
        OwedEosAction::Send { run }
    }
}

/// Log what [`OwedEos::poll`] decided and send the Stop signal when it says so.
pub(crate) fn act_on(action: OwedEosAction, tx: &mpsc::Sender<ReadLoopOutput>) {
    match action {
        OwedEosAction::Wait => {}
        OwedEosAction::Send { run } => {
            warn!(
                run,
                "Sending the end-of-run marker (EOS) for run {run} after the digitizer was \
                 lost mid-run: this board's data for the run ends at the read failure"
            );
            send_stop_signal(tx);
        }
        OwedEosAction::Abandon { run, current_run } => error!(
            run,
            current_run,
            "The end-of-run marker (EOS) of run {run} was never sent: the digitizer was lost \
             mid-run and run {current_run} started before run {run} was stopped. EOS-driven \
             sinks (nwall_sink, root_sink) leave run {run}'s file unfinalized"
        ),
    }
}

/// Send the Stop signal to the decode pipeline, which publishes the EOS.
/// Retries for 3 s on a full channel: a one-shot try_send on the bounded
/// channel silently loses the Stop, and thus the EOS (TODO 58 H1).
pub(crate) fn send_stop_signal(tx: &mpsc::Sender<ReadLoopOutput>) {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut signal = ReadLoopOutput::Stop;
    loop {
        match tx.try_send(signal) {
            Ok(()) => {
                info!("Stop signal sent to decode pipeline");
                return;
            }
            Err(mpsc::error::TrySendError::Full(returned)) => {
                if Instant::now() > deadline {
                    error!("Failed to send Stop signal: channel full for 3s");
                    return;
                }
                signal = returned;
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                warn!("Decode channel closed, Stop signal not needed");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owed(run: u32) -> OwedEos {
        let mut o = OwedEos::default();
        o.run_started(run);
        o
    }

    #[test]
    fn nothing_is_owed_before_a_run_starts() {
        let mut o = OwedEos::default();
        let now = Instant::now();
        assert_eq!(
            o.poll(ComponentState::Configured, 0, now),
            OwedEosAction::Wait
        );
        assert_eq!(
            o.poll(ComponentState::Configured, 0, now + OWED_EOS_DELAY),
            OwedEosAction::Wait
        );
    }

    #[test]
    fn never_sent_while_the_run_goes_on() {
        // The board was lost mid-run: the read loop itself sets Error, the
        // Operator has not stopped the run (the Recorder closes on any EOS).
        let mut o = owed(70);
        let now = Instant::now();
        for dt in [0, 10, 3600] {
            let t = now + Duration::from_secs(dt);
            assert_eq!(o.poll(ComponentState::Error, 70, t), OwedEosAction::Wait);
            assert_eq!(o.poll(ComponentState::Running, 70, t), OwedEosAction::Wait);
        }
    }

    #[test]
    fn sent_once_after_the_stop_and_the_delay() {
        let mut o = owed(70);
        let stop = Instant::now();
        assert_eq!(
            o.poll(ComponentState::Configured, 70, stop),
            OwedEosAction::Wait
        );
        assert_eq!(
            o.poll(ComponentState::Configured, 70, stop + OWED_EOS_DELAY / 2),
            OwedEosAction::Wait
        );
        assert_eq!(
            o.poll(ComponentState::Configured, 70, stop + OWED_EOS_DELAY),
            OwedEosAction::Send { run: 70 }
        );
        assert_eq!(
            o.poll(ComponentState::Configured, 70, stop + OWED_EOS_DELAY * 2),
            OwedEosAction::Wait
        );
    }

    #[test]
    fn a_reset_counts_as_a_stop() {
        let mut o = owed(70);
        let stop = Instant::now();
        assert_eq!(o.poll(ComponentState::Idle, 70, stop), OwedEosAction::Wait);
        assert_eq!(
            o.poll(ComponentState::Idle, 70, stop + OWED_EOS_DELAY),
            OwedEosAction::Send { run: 70 }
        );
    }

    #[test]
    fn sent_at_once_when_the_next_run_is_being_armed() {
        // The read loop may not see the short Configured step (it can sit in
        // a reconnect attempt): Armed means the Operator moved on.
        let mut o = owed(70);
        assert_eq!(
            o.poll(ComponentState::Armed, 70, Instant::now()),
            OwedEosAction::Send { run: 70 }
        );
    }

    #[test]
    fn abandoned_once_the_next_run_has_started() {
        // The pipeline would stamp the EOS with run 71 and close run 71's
        // Recorder file: never send it.
        let mut o = owed(70);
        assert_eq!(
            o.poll(ComponentState::Running, 71, Instant::now()),
            OwedEosAction::Abandon {
                run: 70,
                current_run: 71
            }
        );
        assert_eq!(
            o.poll(
                ComponentState::Configured,
                71,
                Instant::now() + OWED_EOS_DELAY
            ),
            OwedEosAction::Wait
        );
    }

    #[test]
    fn the_normal_stop_clears_it() {
        let mut o = owed(70);
        o.stop_sent();
        let now = Instant::now();
        assert_eq!(
            o.poll(ComponentState::Configured, 70, now),
            OwedEosAction::Wait
        );
        assert_eq!(
            o.poll(ComponentState::Configured, 70, now + OWED_EOS_DELAY),
            OwedEosAction::Wait
        );
    }

    #[test]
    fn stop_signal_reaches_the_pipeline() {
        let (tx, mut rx) = mpsc::channel(1);
        send_stop_signal(&tx);
        assert!(matches!(rx.try_recv(), Ok(ReadLoopOutput::Stop)));
    }

    #[test]
    fn stop_signal_on_a_closed_channel_returns() {
        let (tx, rx) = mpsc::channel::<ReadLoopOutput>(1);
        drop(rx);
        send_stop_signal(&tx);
    }
}
