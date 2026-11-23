//! Per-source batch sequence check, shared by the Merger and the Recorder.
//!
//! Every source numbers its batches from 0 at Start without gaps: a Reader
//! still publishes empty batches, and a decode failure leaves a gap on
//! purpose (`reader/parallel_decode.rs`). The Merger forwards the bytes
//! unchanged, so the Recorder sees the same numbers. A gap anywhere between
//! a source and the Recorder (a lost ZMQ connection, a decode failure) shows
//! up here. TODO 58 M18 (Merger), TODO 89 (Recorder).

/// Per-source statistics with sequence tracking
#[derive(Debug, Default, Clone)]
pub struct SourceStats {
    pub last_sequence: Option<u64>,
    pub total_batches: u64,
    pub restart_count: u32,
    pub gaps_detected: u64,
    pub total_gap_size: u64,
}

/// What `SourceStats::update` observed about a batch's sequence number
/// (TODO 58 M18 — the call site must surface `Gap`/`Restart`, they mean
/// batches were lost or a source restarted mid-run).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeqEvent {
    /// Sequence advanced normally (or first batch from this source).
    Ok,
    /// `gap` batches are missing between the last and this sequence number.
    Gap(u64),
    /// Sequence number jumped far backwards — source restart.
    Restart,
}

impl SourceStats {
    pub fn update(&mut self, seq: u64) -> SeqEvent {
        let event = if let Some(last) = self.last_sequence {
            if seq < last.saturating_sub(100) {
                self.restart_count += 1;
                SeqEvent::Restart
            } else {
                let expected = last + 1;
                if seq > expected {
                    let gap = seq - expected;
                    self.gaps_detected += 1;
                    self.total_gap_size += gap;
                    SeqEvent::Gap(gap)
                } else {
                    SeqEvent::Ok
                }
            }
        } else {
            SeqEvent::Ok
        };

        self.last_sequence = Some(seq);
        self.total_batches += 1;
        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_stats_update() {
        let mut stats = SourceStats::default();

        assert_eq!(stats.update(0), SeqEvent::Ok);
        assert_eq!(stats.total_batches, 1);

        assert_eq!(stats.update(1), SeqEvent::Ok);
        assert_eq!(stats.total_batches, 2);

        // Gap detection
        assert_eq!(stats.update(200), SeqEvent::Gap(198));
        assert_eq!(stats.total_batches, 3);
        assert_eq!(stats.gaps_detected, 1);
        assert_eq!(stats.total_gap_size, 198);

        // Restart detection
        assert_eq!(stats.update(0), SeqEvent::Restart);
        assert_eq!(stats.restart_count, 1);
    }

    #[test]
    fn source_stats_no_gap_sequential() {
        let mut stats = SourceStats::default();
        for i in 0..100 {
            stats.update(i);
        }
        assert_eq!(stats.total_batches, 100);
        assert_eq!(stats.gaps_detected, 0);
        assert_eq!(stats.total_gap_size, 0);
        assert_eq!(stats.restart_count, 0);
    }

    #[test]
    fn source_stats_multiple_gaps() {
        let mut stats = SourceStats::default();
        stats.update(0);
        stats.update(5); // gap of 4
        stats.update(10); // gap of 4
        stats.update(100); // gap of 89
        assert_eq!(stats.gaps_detected, 3);
        assert_eq!(stats.total_gap_size, 4 + 4 + 89);
    }
}
