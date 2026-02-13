//! Chunk-based event builder (v2)
//!
//! Legacy ELIFANT-Event と同じ「チャンク＋ソート＋Safe Horizon」方式。
//! ソート済みチャンクからイベントを構築する pure 関数を提供する。
//!
//! # アルゴリズム
//!
//! 1. Sorter がヒットを蓄積・ソートし、Safe Horizon で分割
//! 2. `build_events_from_chunk()` がソート済みチャンクからイベントを構築
//! 3. core 領域のトリガーのみ emit（境界イベントの重複・欠落を防ぐ）

use std::collections::{HashMap, HashSet};

use super::built_event::{BuiltEvent, EventHit};
use super::hit::Hit;

/// ソート済みチャンク
///
/// Sorter が蓄積したヒットをソートし、Safe Horizon で分割した結果。
/// `core_end` より前のトリガーのみイベントとして emit する。
#[derive(Debug, Clone)]
pub struct SortedChunk {
    /// ソート済みヒット（timestamp_ns 昇順）
    pub hits: Vec<Hit>,
    /// Core 領域の終端 [ns]
    /// この時刻以降のトリガーは次のチャンクで処理する。
    /// ただし coincidence window 内のヒットとして参照はされる。
    pub core_end: f64,
}

/// トリガー設定
///
/// イベントビルドに必要な全設定を保持する。
/// SliceBuilder の設定を簡潔に再構成したもの。
#[derive(Debug, Clone)]
pub struct TriggerConfig {
    /// トリガーチャンネル: (module, channel)
    pub triggers: HashSet<(u8, u8)>,
    /// パイルアップ優先度: (module, channel) -> priority (lower = higher)
    pub priorities: HashMap<(u8, u8), u32>,
    /// AC ペアマッピング: detector (mod, ch) -> AC (mod, ch)
    pub ac_pairs: HashMap<(u8, u8), (u8, u8)>,
    /// コインシデンスウィンドウ [ns]
    pub coincidence_window_ns: f64,
}

impl TriggerConfig {
    /// Check if a channel is a trigger
    #[inline]
    pub fn is_trigger(&self, module: u8, channel: u8) -> bool {
        self.triggers.contains(&(module, channel))
    }

    /// Get trigger priority (lower = higher priority, u32::MAX = not a trigger)
    #[inline]
    pub fn priority(&self, module: u8, channel: u8) -> u32 {
        self.priorities
            .get(&(module, channel))
            .copied()
            .unwrap_or(u32::MAX)
    }
}

/// ソート済みチャンクからイベントを構築する (pure, 副作用なし)
///
/// # アルゴリズム
///
/// 1. ソート済みヒットを順にスキャン
/// 2. トリガーチャンネルを発見したら:
///    - `ts >= core_end` → スキップ (unsafe 領域、次チャンクで処理)
///    - backward scan で prior trigger チェック (pile-up rejection)
///    - `partition_point` で ±coincidence_window 内のヒット範囲を特定
///    - coincident hits をスキャンして AC 判定
///    - BuiltEvent を emit
///
/// # Arguments
/// * `chunk` - ソート済みチャンク
/// * `config` - トリガー設定
///
/// # Returns
/// 構築されたイベントのベクタ（trigger_time 昇順）
pub fn build_events_from_chunk(chunk: &SortedChunk, config: &TriggerConfig) -> Vec<BuiltEvent> {
    let hits = &chunk.hits;
    if hits.is_empty() {
        return Vec::new();
    }

    let window = config.coincidence_window_ns;
    let mut events = Vec::new();

    for (idx, hit) in hits.iter().enumerate() {
        // Skip non-trigger hits
        if !config.is_trigger(hit.module, hit.channel) {
            continue;
        }

        // Skip triggers in unsafe region (processed in next chunk)
        if hit.timestamp_ns >= chunk.core_end {
            continue;
        }

        // Check for prior trigger within coincidence window (pile-up rejection)
        if has_prior_trigger(hits, idx, config) {
            continue;
        }

        // Build event from this trigger
        let trigger_time = hit.timestamp_ns;
        let window_start = trigger_time - window;
        let window_end = trigger_time + window;

        // Binary search for coincidence window boundaries
        let range_start = hits.partition_point(|h| h.timestamp_ns < window_start);
        let range_end = hits.partition_point(|h| h.timestamp_ns <= window_end);

        // Collect channels present in window (for AC detection)
        let channels_in_window: HashSet<(u8, u8)> = hits[range_start..range_end]
            .iter()
            .map(|h| (h.module, h.channel))
            .collect();

        // Build event hits
        let mut event_hits = Vec::with_capacity(range_end - range_start);
        for h in &hits[range_start..range_end] {
            let with_ac = config
                .ac_pairs
                .get(&(h.module, h.channel))
                .is_some_and(|ac| channels_in_window.contains(ac));

            event_hits.push(EventHit {
                module: h.module,
                channel: h.channel,
                energy: h.energy,
                energy_short: h.energy_short,
                relative_time: h.timestamp_ns - trigger_time,
                with_ac,
            });
        }

        if !event_hits.is_empty() {
            events.push(BuiltEvent {
                event_id: 0, // Assigned by caller
                trigger_time,
                trigger_module: hit.module,
                trigger_channel: hit.channel,
                hits: event_hits,
            });
        }
    }

    events
}

/// Check if there's a prior trigger within coincidence window (backward scan)
///
/// A "prior" trigger is one that:
/// - Is within coincidence_window before the current trigger
/// - Has equal or higher priority (lower priority value)
///
/// If such a trigger exists, the current trigger is suppressed (pile-up rejection).
fn has_prior_trigger(hits: &[Hit], trigger_idx: usize, config: &TriggerConfig) -> bool {
    let trigger = &hits[trigger_idx];
    let trigger_time = trigger.timestamp_ns;
    let trigger_priority = config.priority(trigger.module, trigger.channel);
    let window_start = trigger_time - config.coincidence_window_ns;

    // Backward scan from trigger
    for i in (0..trigger_idx).rev() {
        let h = &hits[i];
        if h.timestamp_ns < window_start {
            break;
        }
        if config.is_trigger(h.module, h.channel) {
            let other_priority = config.priority(h.module, h.channel);
            if other_priority <= trigger_priority {
                return true;
            }
        }
    }
    false
}

/// ヒットバッファをソートし、Safe Horizon で分割する
///
/// バッファの所有権を取り、ソート後に Safe Horizon で分割する。
/// データ不足の場合はバッファをそのまま返す（ソート済み）。
///
/// # Arguments
/// * `buffer` - 未ソートのヒットバッファ（move される）
/// * `safe_horizon_ns` - Safe Horizon [ns] (典型値: 50_000_000.0 = 50ms)
///
/// # Returns
/// * `Ok((chunk, retained))` - チャンクと次回に持ち越すヒット
/// * `Err(buffer)` - データ不足（バッファをそのまま返す、ソート済み）
pub fn sort_and_split(
    mut buffer: Vec<Hit>,
    safe_horizon_ns: f64,
) -> Result<(SortedChunk, Vec<Hit>), Vec<Hit>> {
    if buffer.is_empty() {
        return Err(buffer);
    }

    // Sort by timestamp
    buffer.sort_unstable_by(|a, b| a.timestamp_ns.total_cmp(&b.timestamp_ns));

    let earliest = buffer.first().unwrap().timestamp_ns;
    let latest = buffer.last().unwrap().timestamp_ns;
    let core_end = latest - safe_horizon_ns;

    // Not enough data spread for safe extraction
    if core_end <= earliest {
        return Err(buffer);
    }

    // Split at core_end: retained = hits with ts >= core_end
    let split_idx = buffer.partition_point(|h| h.timestamp_ns < core_end);
    let retained = buffer[split_idx..].to_vec();

    // The entire buffer becomes the chunk (O(1) — we already have it by value)
    let chunk = SortedChunk {
        hits: buffer,
        core_end,
    };

    Ok((chunk, retained))
}

/// Flush: 全データをチャンクとして返す (EOS 時に使用)
///
/// Safe Horizon を無視して全データを返す。core_end = f64::MAX で全トリガーを emit。
pub fn sort_and_flush(mut buffer: Vec<Hit>) -> Option<SortedChunk> {
    if buffer.is_empty() {
        return None;
    }

    buffer.sort_unstable_by(|a, b| a.timestamp_ns.total_cmp(&b.timestamp_ns));

    Some(SortedChunk {
        hits: buffer,
        core_end: f64::MAX,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hit(module: u8, channel: u8, ts: f64) -> Hit {
        Hit::new(module, channel, 1000, 500, ts)
    }

    fn simple_config() -> TriggerConfig {
        let mut triggers = HashSet::new();
        triggers.insert((0, 0));
        let mut priorities = HashMap::new();
        priorities.insert((0, 0), 0);
        TriggerConfig {
            triggers,
            priorities,
            ac_pairs: HashMap::new(),
            coincidence_window_ns: 500.0,
        }
    }

    // ======================================================================
    // build_events_from_chunk tests
    // ======================================================================

    #[test]
    fn test_empty_chunk() {
        let chunk = SortedChunk {
            hits: vec![],
            core_end: 1000.0,
        };
        let events = build_events_from_chunk(&chunk, &simple_config());
        assert!(events.is_empty());
    }

    #[test]
    fn test_no_trigger_hits() {
        let config = simple_config(); // only (0,0) is trigger
        let chunk = SortedChunk {
            hits: vec![
                make_hit(1, 0, 100.0), // not trigger
                make_hit(1, 1, 200.0), // not trigger
            ],
            core_end: 1000.0,
        };
        let events = build_events_from_chunk(&chunk, &config);
        assert!(events.is_empty());
    }

    #[test]
    fn test_single_trigger() {
        let config = simple_config(); // (0,0) trigger, 500ns window
        let chunk = SortedChunk {
            hits: vec![
                make_hit(0, 0, 1000.0), // trigger
                make_hit(1, 0, 1100.0), // coincident
                make_hit(1, 1, 1200.0), // coincident
            ],
            core_end: 5000.0,
        };
        let events = build_events_from_chunk(&chunk, &config);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].trigger_module, 0);
        assert_eq!(events[0].trigger_channel, 0);
        assert_eq!(events[0].hits.len(), 3); // trigger + 2 coincident
    }

    #[test]
    fn test_multiple_separated_triggers() {
        let config = simple_config(); // 500ns window
        let chunk = SortedChunk {
            hits: vec![
                make_hit(0, 0, 1000.0),  // trigger 1
                make_hit(1, 0, 1100.0),  // coincident with T1
                make_hit(0, 0, 5000.0),  // trigger 2 (well separated)
                make_hit(1, 1, 5100.0),  // coincident with T2
                make_hit(0, 0, 10000.0), // trigger 3
            ],
            core_end: 20000.0,
        };
        let events = build_events_from_chunk(&chunk, &config);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].hits.len(), 2);
        assert_eq!(events[1].hits.len(), 2);
        assert_eq!(events[2].hits.len(), 1);
    }

    #[test]
    fn test_pileup_rejection() {
        // Two triggers within coincidence window — lower priority is suppressed
        let mut triggers = HashSet::new();
        triggers.insert((0, 0));
        triggers.insert((0, 1));
        let mut priorities = HashMap::new();
        priorities.insert((0, 0), 0); // highest priority
        priorities.insert((0, 1), 1); // lower priority
        let config = TriggerConfig {
            triggers,
            priorities,
            ac_pairs: HashMap::new(),
            coincidence_window_ns: 500.0,
        };

        let chunk = SortedChunk {
            hits: vec![
                make_hit(0, 0, 1000.0), // trigger (priority 0)
                make_hit(0, 1, 1200.0), // trigger (priority 1) — within window of (0,0), suppressed
                make_hit(1, 0, 1100.0), // coincident
            ],
            core_end: 5000.0,
        };
        let events = build_events_from_chunk(&chunk, &config);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].trigger_module, 0);
        assert_eq!(events[0].trigger_channel, 0);
    }

    #[test]
    fn test_pileup_equal_priority() {
        // Two triggers with same priority — earlier one wins
        let mut triggers = HashSet::new();
        triggers.insert((0, 0));
        triggers.insert((0, 1));
        let mut priorities = HashMap::new();
        priorities.insert((0, 0), 0);
        priorities.insert((0, 1), 0); // same priority
        let config = TriggerConfig {
            triggers,
            priorities,
            ac_pairs: HashMap::new(),
            coincidence_window_ns: 500.0,
        };

        let chunk = SortedChunk {
            hits: vec![
                make_hit(0, 0, 1000.0), // first trigger
                make_hit(0, 1, 1200.0), // second trigger — suppressed (same priority, prior exists)
            ],
            core_end: 5000.0,
        };
        let events = build_events_from_chunk(&chunk, &config);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].trigger_channel, 0);
    }

    #[test]
    fn test_higher_priority_later_not_suppressed() {
        // Higher priority trigger after lower priority — NOT suppressed
        let mut triggers = HashSet::new();
        triggers.insert((0, 0));
        triggers.insert((0, 1));
        let mut priorities = HashMap::new();
        priorities.insert((0, 0), 1); // lower priority
        priorities.insert((0, 1), 0); // higher priority
        let config = TriggerConfig {
            triggers,
            priorities,
            ac_pairs: HashMap::new(),
            coincidence_window_ns: 500.0,
        };

        let chunk = SortedChunk {
            hits: vec![
                make_hit(0, 0, 1000.0), // trigger (priority 1)
                make_hit(0, 1, 1200.0), // trigger (priority 0) — NOT suppressed, it's higher priority
            ],
            core_end: 5000.0,
        };
        let events = build_events_from_chunk(&chunk, &config);
        // Both should fire: (0,0) first, then (0,1) is higher priority so not suppressed
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_core_end_boundary() {
        let config = simple_config(); // 500ns window
        let chunk = SortedChunk {
            hits: vec![
                make_hit(0, 0, 900.0),  // trigger — in core (< 1000)
                make_hit(0, 0, 1000.0), // trigger — at core_end boundary (>= 1000, skip)
                make_hit(0, 0, 1100.0), // trigger — after core_end (>= 1000, skip)
                make_hit(1, 0, 1050.0), // coincident (can be referenced by core trigger)
            ],
            core_end: 1000.0,
        };
        let events = build_events_from_chunk(&chunk, &config);
        // Only the trigger at 900 should fire
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].trigger_time, 900.0);
        // Coincident hits at 1050 are within 500ns window of 900 → included
        assert!(events[0].hits.len() >= 2);
    }

    #[test]
    fn test_coincidence_window_boundaries() {
        let config = simple_config(); // 500ns window
        let chunk = SortedChunk {
            hits: vec![
                make_hit(1, 0, 400.0),  // too early (500 ns before 1000)
                make_hit(1, 1, 500.0),  // at window start (1000 - 500 = 500)
                make_hit(0, 0, 1000.0), // trigger
                make_hit(1, 2, 1500.0), // at window end (1000 + 500 = 1500)
                make_hit(1, 3, 1501.0), // just outside window
            ],
            core_end: 5000.0,
        };
        let events = build_events_from_chunk(&chunk, &config);
        assert_eq!(events.len(), 1);
        // Window: [500, 1500] — hits at 500, 1000, 1500 should be included
        // Hit at 400 is outside (< 500), hit at 1501 is outside (> 1500)
        assert_eq!(events[0].hits.len(), 3);
    }

    #[test]
    fn test_ac_detection() {
        let mut config = simple_config();
        config.ac_pairs.insert((0, 0), (0, 1)); // detector (0,0) has AC at (0,1)

        let chunk = SortedChunk {
            hits: vec![
                make_hit(0, 0, 1000.0), // trigger + detector
                make_hit(0, 1, 1050.0), // AC hit
            ],
            core_end: 5000.0,
        };
        let events = build_events_from_chunk(&chunk, &config);
        assert_eq!(events.len(), 1);

        // The detector hit (0,0) should have with_ac = true
        let det_hit = events[0]
            .hits
            .iter()
            .find(|h| h.module == 0 && h.channel == 0)
            .unwrap();
        assert!(det_hit.with_ac);

        // The AC hit (0,1) should have with_ac = false (no AC pair defined for it)
        let ac_hit = events[0]
            .hits
            .iter()
            .find(|h| h.module == 0 && h.channel == 1)
            .unwrap();
        assert!(!ac_hit.with_ac);
    }

    #[test]
    fn test_ac_not_in_window() {
        let mut config = simple_config();
        config.ac_pairs.insert((0, 0), (0, 1));

        let chunk = SortedChunk {
            hits: vec![
                make_hit(0, 0, 1000.0), // trigger + detector
                make_hit(0, 1, 2000.0), // AC hit — too far away
            ],
            core_end: 5000.0,
        };
        let events = build_events_from_chunk(&chunk, &config);
        assert_eq!(events.len(), 1);

        // AC is outside window → with_ac = false
        let det_hit = events[0]
            .hits
            .iter()
            .find(|h| h.module == 0 && h.channel == 0)
            .unwrap();
        assert!(!det_hit.with_ac);
    }

    #[test]
    fn test_relative_time_calculation() {
        let config = simple_config();
        let chunk = SortedChunk {
            hits: vec![
                make_hit(1, 0, 900.0),  // -100 ns relative
                make_hit(0, 0, 1000.0), // trigger (relative_time = 0)
                make_hit(1, 1, 1150.0), // +150 ns relative
            ],
            core_end: 5000.0,
        };
        let events = build_events_from_chunk(&chunk, &config);
        assert_eq!(events.len(), 1);

        let hit_before = events[0]
            .hits
            .iter()
            .find(|h| h.module == 1 && h.channel == 0)
            .unwrap();
        assert!((hit_before.relative_time - (-100.0)).abs() < 0.01);

        let trigger_hit = events[0]
            .hits
            .iter()
            .find(|h| h.module == 0 && h.channel == 0)
            .unwrap();
        assert!((trigger_hit.relative_time - 0.0).abs() < 0.01);

        let hit_after = events[0]
            .hits
            .iter()
            .find(|h| h.module == 1 && h.channel == 1)
            .unwrap();
        assert!((hit_after.relative_time - 150.0).abs() < 0.01);
    }

    #[test]
    fn test_event_id_not_assigned() {
        // build_events_from_chunk assigns event_id = 0, caller is responsible for IDs
        let config = simple_config();
        let chunk = SortedChunk {
            hits: vec![make_hit(0, 0, 1000.0), make_hit(0, 0, 5000.0)],
            core_end: 10000.0,
        };
        let events = build_events_from_chunk(&chunk, &config);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_id, 0);
        assert_eq!(events[1].event_id, 0);
    }

    // ======================================================================
    // sort_and_split tests
    // ======================================================================

    #[test]
    fn test_sort_and_split_empty() {
        let result = sort_and_split(vec![], 50_000_000.0);
        assert!(result.is_err());
    }

    #[test]
    fn test_sort_and_split_insufficient_data() {
        // All data within safe_horizon — nothing safe to extract
        let hits = vec![
            make_hit(0, 0, 1000.0),
            make_hit(0, 1, 2000.0),
            make_hit(1, 0, 3000.0),
        ];
        // safe_horizon = 50ms = 50_000_000 ns, data span = 2000 ns
        let result = sort_and_split(hits, 50_000_000.0);
        assert!(result.is_err());
    }

    #[test]
    fn test_sort_and_split_sorts_correctly() {
        let hits = vec![
            make_hit(0, 0, 300.0),
            make_hit(0, 1, 100.0),
            make_hit(1, 0, 200.0),
        ];
        // Use tiny safe_horizon to force split
        let result = sort_and_split(hits, 50.0);
        assert!(result.is_ok());
        let (chunk, _) = result.unwrap();

        // Should be sorted
        for i in 1..chunk.hits.len() {
            assert!(chunk.hits[i].timestamp_ns >= chunk.hits[i - 1].timestamp_ns);
        }
    }

    #[test]
    fn test_sort_and_split_retained_data() {
        // Create hits spanning 100ms
        let hits = vec![
            make_hit(0, 0, 0.0),
            make_hit(0, 1, 30_000_000.0),  // 30ms
            make_hit(1, 0, 60_000_000.0),  // 60ms
            make_hit(1, 1, 100_000_000.0), // 100ms
        ];
        let result = sort_and_split(hits, 50_000_000.0); // 50ms safe horizon
        assert!(result.is_ok());
        let (chunk, retained) = result.unwrap();

        // core_end = 100ms - 50ms = 50ms
        assert!((chunk.core_end - 50_000_000.0).abs() < 0.01);

        // Retained should contain hits with ts >= core_end (60ms and 100ms)
        assert_eq!(retained.len(), 2);
        assert!((retained[0].timestamp_ns - 60_000_000.0).abs() < 0.01);
        assert!((retained[1].timestamp_ns - 100_000_000.0).abs() < 0.01);

        // Chunk should contain ALL hits (for coincidence reference)
        assert_eq!(chunk.hits.len(), 4);
    }

    #[test]
    fn test_sort_and_split_core_end_correct() {
        let hits = vec![
            make_hit(0, 0, 0.0),
            make_hit(0, 1, 200_000_000.0), // 200ms
        ];
        let result = sort_and_split(hits, 50_000_000.0);
        assert!(result.is_ok());
        let (chunk, retained) = result.unwrap();

        // core_end = 200ms - 50ms = 150ms
        assert!((chunk.core_end - 150_000_000.0).abs() < 0.01);

        // Only the 200ms hit should be retained
        assert_eq!(retained.len(), 1);
        assert!((retained[0].timestamp_ns - 200_000_000.0).abs() < 0.01);
    }

    // ======================================================================
    // sort_and_flush tests
    // ======================================================================

    #[test]
    fn test_sort_and_flush_empty() {
        let result = sort_and_flush(vec![]);
        assert!(result.is_none());
    }

    #[test]
    fn test_sort_and_flush_sorts_and_returns_all() {
        let hits = vec![
            make_hit(0, 0, 300.0),
            make_hit(0, 1, 100.0),
            make_hit(1, 0, 200.0),
        ];
        let chunk = sort_and_flush(hits).unwrap();

        assert_eq!(chunk.hits.len(), 3);
        assert_eq!(chunk.core_end, f64::MAX);
        assert_eq!(chunk.hits[0].timestamp_ns, 100.0);
        assert_eq!(chunk.hits[1].timestamp_ns, 200.0);
        assert_eq!(chunk.hits[2].timestamp_ns, 300.0);
    }

    // ======================================================================
    // Integration: sort_and_split + build_events_from_chunk
    // ======================================================================

    #[test]
    fn test_pipeline_integration() {
        let config = simple_config(); // (0,0) trigger, 500ns window

        // Simulate accumulated hits spanning 200ms
        let mut hits = Vec::new();
        for i in 0..20 {
            let base = (i as f64) * 10_000_000.0; // every 10ms
            hits.push(make_hit(0, 0, base)); // trigger
            hits.push(make_hit(1, 0, base + 100.0)); // coincident
        }
        // Shuffle to simulate network disorder
        use rand::seq::SliceRandom;
        hits.shuffle(&mut rand::thread_rng());

        let total_triggers = 20;

        // First split
        let (chunk1, retained) = sort_and_split(hits, 50_000_000.0).unwrap();
        let events1 = build_events_from_chunk(&chunk1, &config);

        // Simulate more data arriving (none in this test)
        // Flush the retained data
        let chunk2 = sort_and_flush(retained).unwrap();
        let events2 = build_events_from_chunk(&chunk2, &config);

        let total_events = events1.len() + events2.len();
        assert_eq!(
            total_events, total_triggers,
            "Should recover all {} triggers, got {}",
            total_triggers, total_events
        );

        // Verify all events have exactly 2 hits (trigger + coincident)
        for event in events1.iter().chain(events2.iter()) {
            assert_eq!(event.hits.len(), 2);
        }
    }

    /// Offline verification with real ROOT data
    ///
    /// Reads ELIFANT test data, shuffles within 30ms chunks,
    /// runs sort_and_split → build_events_from_chunk pipeline,
    /// and compares with SliceBuilder (offline reference).
    ///
    /// Run: cargo test --features root --lib chunk_builder::tests::test_offline_root_data -- --ignored --nocapture
    #[test]
    #[ignore]
    #[cfg(feature = "root")]
    fn test_offline_root_data() {
        use super::super::root_io::read_hits_from_root;
        use super::super::slice_builder::SliceBuilder;
        use rand::seq::SliceRandom;
        use std::path::Path;
        use std::time::Instant;

        let path =
            Path::new("/Users/aogaki/WorkSpace/ELIFANT2025/p91Zr/data/run0113_0000_p_91Zr.root");
        if !path.exists() {
            eprintln!("Test data not found: {}", path.display());
            return;
        }

        // Read hits
        let t0 = Instant::now();
        let all_hits = read_hits_from_root(path, "ELIADE_Tree").unwrap();
        eprintln!("Read {} hits in {:?}", all_hits.len(), t0.elapsed());

        // --- Reference: SliceBuilder (offline, single-pass) ---
        let t1 = Instant::now();
        let mut ref_builder = SliceBuilder::new(10_000_000.0, 100.0); // 10ms slice, 100ns window
        ref_builder.add_trigger(0, 0, 0); // Same trigger as used in tests
        let ref_events = ref_builder.build_events(all_hits.clone());
        eprintln!(
            "SliceBuilder: {} events in {:?}",
            ref_events.len(),
            t1.elapsed()
        );

        // --- Chunk pipeline: simulate shuffled input ---
        let t2 = Instant::now();

        // Build trigger config matching the SliceBuilder
        let config = TriggerConfig {
            triggers: {
                let mut s = HashSet::new();
                s.insert((0, 0));
                s
            },
            priorities: {
                let mut m = HashMap::new();
                m.insert((0, 0), 0);
                m
            },
            ac_pairs: HashMap::new(),
            coincidence_window_ns: 100.0,
        };

        // Shuffle hits within 30ms chunks (simulate network disorder)
        let mut shuffled_hits = all_hits;
        shuffled_hits.sort_by(|a, b| a.timestamp_ns.total_cmp(&b.timestamp_ns));

        let mut rng = rand::thread_rng();
        let chunk_size_ns = 30_000_000.0; // 30ms
        let mut chunk_start = shuffled_hits.first().unwrap().timestamp_ns;
        let mut start_idx = 0;
        for i in 0..shuffled_hits.len() {
            if shuffled_hits[i].timestamp_ns > chunk_start + chunk_size_ns {
                shuffled_hits[start_idx..i].shuffle(&mut rng);
                chunk_start = shuffled_hits[i].timestamp_ns;
                start_idx = i;
            }
        }
        shuffled_hits[start_idx..].shuffle(&mut rng);

        // Process with sort_and_split pipeline
        let safe_horizon_ns = 50_000_000.0; // 50ms
        let batch_size = 500_000; // 500K hits per accumulation
        let mut buffer = Vec::new();
        let mut total_events = 0usize;
        let mut chunk_count = 0usize;

        for hit in shuffled_hits {
            buffer.push(hit);
            if buffer.len() >= batch_size {
                match sort_and_split(buffer, safe_horizon_ns) {
                    Ok((chunk, retained)) => {
                        let events = build_events_from_chunk(&chunk, &config);
                        total_events += events.len();
                        chunk_count += 1;
                        buffer = retained;
                    }
                    Err(returned) => buffer = returned,
                }
            }
        }

        // Flush remaining
        if let Some(chunk) = sort_and_flush(buffer) {
            let events = build_events_from_chunk(&chunk, &config);
            total_events += events.len();
            chunk_count += 1;
        }

        eprintln!(
            "Chunk pipeline: {} events in {} chunks, {:?}",
            total_events,
            chunk_count,
            t2.elapsed()
        );

        // Compare results
        eprintln!("SliceBuilder events: {}", ref_events.len());
        eprintln!("Chunk pipeline events: {}", total_events);

        // Allow small discrepancy due to boundary effects with shuffled data
        // but they should be very close
        let diff = (total_events as i64 - ref_events.len() as i64).unsigned_abs();
        let tolerance = (ref_events.len() as f64 * 0.001) as u64; // 0.1% tolerance
        assert!(
            diff <= tolerance,
            "Event count difference {} exceeds tolerance {} (ref={}, chunk={})",
            diff,
            tolerance,
            ref_events.len(),
            total_events,
        );
    }

    #[test]
    fn test_pipeline_no_boundary_loss() {
        // Place triggers near the core_end boundary to verify no events are lost
        let config = simple_config(); // 500ns window

        let hits = vec![
            make_hit(0, 0, 49_999_000.0),  // just before core_end (50ms - 1µs)
            make_hit(1, 0, 49_999_100.0),  // coincident
            make_hit(0, 0, 50_001_000.0),  // just after core_end (50ms + 1µs)
            make_hit(1, 1, 50_001_100.0),  // coincident
            make_hit(0, 0, 100_000_000.0), // far future (to set time range)
        ];

        let (chunk, retained) = sort_and_split(hits.clone(), 50_000_000.0).unwrap();
        // core_end = 100ms - 50ms = 50ms
        let events1 = build_events_from_chunk(&chunk, &config);

        // The trigger at 49.999ms should be in events1 (before core_end)
        assert!(
            events1
                .iter()
                .any(|e| (e.trigger_time - 49_999_000.0).abs() < 0.01),
            "Trigger at 49.999ms should be in first chunk"
        );

        // The trigger at 50.001ms should NOT be in events1 (at/after core_end)
        assert!(
            !events1
                .iter()
                .any(|e| (e.trigger_time - 50_001_000.0).abs() < 0.01),
            "Trigger at 50.001ms should NOT be in first chunk"
        );

        // Flush retained — should contain the 50.001ms trigger
        let chunk2 = sort_and_flush(retained).unwrap();
        let events2 = build_events_from_chunk(&chunk2, &config);
        assert!(
            events2
                .iter()
                .any(|e| (e.trigger_time - 50_001_000.0).abs() < 0.01),
            "Trigger at 50.001ms should be in flushed chunk"
        );

        // Total: all 3 triggers accounted for
        let total = events1.len() + events2.len();
        assert_eq!(total, 3);
    }
}
