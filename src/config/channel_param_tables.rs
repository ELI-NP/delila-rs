//! Per-firmware channel-parameter tables for [`super::digitizer::ChannelConfig`]
//! → CAEN DevTree mapping.
//!
//! Replaces the 4 hand-written `if let Some(...)` branches inside
//! `DigitizerConfig::add_channel_params` (R-C1, 2026-05-06). Each table is a
//! flat slice of `(devtree_path, accessor)` entries; the accessor returns
//! `Option<String>` so the loop only emits a CAEN parameter when the field
//! is `Some`. Non-capturing closures coerce to `fn` pointers, so the slices
//! live in `const` storage with no runtime cost.
//!
//! # Why per-FW (not cross-FW shared)
//!
//! PSD2/AMax and PHA2 share a lot of "Input / Coincidence / Waveform"
//! DevTree names, but the original branches deliberately kept them per-FW
//! so an accidentally-set field for the wrong FW (e.g. `baseline_avg` on a
//! PHA2 channel) would be silently dropped rather than pushed and rejected
//! by FELib. Preserving that behavior is part of the "no behavior change"
//! contract for R-C1, so each FW gets its own table — duplication remains
//! at the table level but is cheap to maintain (one line per field) and is
//! visible in one place rather than scattered across a 500-line `match`.

use super::digitizer::ChannelConfig;

/// Pull `Option<String>` from a `ChannelConfig` field. The runtime loop
/// pushes a CAEN parameter only when this returns `Some`.
pub type ChannelParamAccessor = fn(&ChannelConfig) -> Option<String>;

/// `(devtree_path, accessor)` — entry in a per-FW parameter table.
pub type ChannelParamEntry = (&'static str, ChannelParamAccessor);

/// PSD1/PHA1 polarity maps "negative"/"positive" to CAEN register-style
/// enums. Anything else is passed through verbatim.
fn map_dig1_polarity(raw: &str) -> String {
    match raw.to_lowercase().as_str() {
        "negative" => "POLARITY_NEGATIVE".to_string(),
        "positive" => "POLARITY_POSITIVE".to_string(),
        _ => raw.to_string(),
    }
}

// ---------------------------------------------------------------------------
// PSD2 / AMax — 37 DevTree fields
// ---------------------------------------------------------------------------

pub const PSD2_AMAX_PARAMS: &[ChannelParamEntry] = &[
    // ---- Input ----
    ("ChEnable", |c| c.enabled.clone()),
    ("PulsePolarity", |c| c.polarity.clone()),
    ("DCOffset", |c| c.dc_offset.map(|v| v.to_string())),
    ("ChGain", |c| c.vga_gain.map(|v| v.to_string())),
    ("ADCInputBaselineAvg", |c| c.baseline_avg.clone()),
    ("AbsoluteBaseline", |c| {
        c.fixed_baseline.map(|v| v.to_string())
    }),
    ("ChRecordLengthT", |c| {
        c.record_length_ns.map(|v| v.to_string())
    }),
    ("ChPreTriggerT", |c| c.pre_trigger_ns.map(|v| v.to_string())),
    ("WaveDownSamplingFactor", |c| c.wave_downsampling.clone()),
    // ---- Trigger ----
    ("TriggerFilterSelection", |c| c.discriminator_mode.clone()),
    ("TriggerThr", |c| c.trigger_threshold.map(|v| v.to_string())),
    ("CFDDelayT", |c| c.cfd_delay_ns.map(|v| v.to_string())),
    ("CFDFraction", |c| c.cfd_fraction.clone()),
    ("TimeFilterRetriggerGuardT", |c| {
        c.trigger_holdoff_ns.map(|v| v.to_string())
    }),
    ("SmoothingFactor", |c| c.smoothing_factor.clone()),
    ("TimeFilterSmoothing", |c| c.time_filter_smoothing.clone()),
    ("EventTriggerSource", |c| c.event_trigger_source.clone()),
    ("WaveTriggerSource", |c| c.wave_trigger_source.clone()),
    // ---- Energy ----
    ("EnergyGain", |c| c.energy_coarse_gain.clone()),
    ("GateLongLengthT", |c| c.gate_long_ns.map(|v| v.to_string())),
    ("GateShortLengthT", |c| {
        c.gate_short_ns.map(|v| v.to_string())
    }),
    ("GateOffsetT", |c| c.gate_pre_ns.map(|v| v.to_string())),
    ("LongChargeIntegratorPedestal", |c| {
        c.charge_pedestal.map(|v| v.to_string())
    }),
    ("ShortChargeIntegratorPedestal", |c| {
        c.short_charge_pedestal.map(|v| v.to_string())
    }),
    ("ChargeSmoothing", |c| c.charge_smoothing.clone()),
    // ---- Coincidence ----
    ("ChannelsTriggerMask", |c| c.ch_trigger_mask.clone()),
    ("CoincidenceMask", |c| c.coincidence_mask.clone()),
    ("AntiCoincidenceMask", |c| c.anti_coincidence_mask.clone()),
    ("CoincidenceLengthT", |c| {
        c.coincidence_window_ns.map(|v| v.to_string())
    }),
    ("ChannelVetoSource", |c| c.ch_veto_source.clone()),
    ("ADCVetoWidth", |c| {
        c.ch_veto_width_ns.map(|v| v.to_string())
    }),
    ("EventSelector", |c| c.event_selector.clone()),
    // ---- Waveform ----
    ("WaveSaving", |c| c.wave_saving.clone()),
    ("WaveAnalogProbe0", |c| c.analog_probe_0.clone()),
    ("WaveAnalogProbe1", |c| c.analog_probe_1.clone()),
    ("WaveDigitalProbe0", |c| c.digital_probe_0.clone()),
    ("WaveDigitalProbe1", |c| c.digital_probe_1.clone()),
    ("WaveDigitalProbe2", |c| c.digital_probe_2.clone()),
    ("WaveDigitalProbe3", |c| c.digital_probe_3.clone()),
];

// ---------------------------------------------------------------------------
// PHA2 — 30 DevTree fields
// ---------------------------------------------------------------------------

pub const PHA2_PARAMS: &[ChannelParamEntry] = &[
    // ---- Input (DevTree paths shared with PSD2 but kept per-FW for safety) ----
    ("ChEnable", |c| c.enabled.clone()),
    ("PulsePolarity", |c| c.polarity.clone()),
    ("DCOffset", |c| c.dc_offset.map(|v| v.to_string())),
    ("ChGain", |c| c.vga_gain.map(|v| v.to_string())),
    ("ChRecordLengthT", |c| {
        c.record_length_ns.map(|v| v.to_string())
    }),
    ("ChPreTriggerT", |c| c.pre_trigger_ns.map(|v| v.to_string())),
    ("WaveDownSamplingFactor", |c| c.wave_downsampling.clone()),
    // ---- Trigger (PHA2 subset) ----
    ("TriggerThr", |c| c.trigger_threshold.map(|v| v.to_string())),
    ("EventTriggerSource", |c| c.event_trigger_source.clone()),
    ("WaveTriggerSource", |c| c.wave_trigger_source.clone()),
    // ---- Time filter (PHA2 specific) ----
    ("TimeFilterRiseTimeT", |c| {
        c.time_filter_rise_time_ns.map(|v| v.to_string())
    }),
    ("TimeFilterRetriggerGuardT", |c| {
        c.time_filter_retrigger_guard_ns.map(|v| v.to_string())
    }),
    // ---- Energy filter (PHA2 trapezoidal) ----
    ("EnergyFilterRiseTimeT", |c| {
        c.energy_filter_rise_time_ns.map(|v| v.to_string())
    }),
    ("EnergyFilterFlatTopT", |c| {
        c.energy_filter_flat_top_ns.map(|v| v.to_string())
    }),
    ("EnergyFilterPoleZeroT", |c| {
        c.energy_filter_pole_zero_ns.map(|v| v.to_string())
    }),
    ("EnergyFilterPeakingPosition", |c| {
        c.energy_filter_peaking_position.map(|v| v.to_string())
    }),
    ("EnergyFilterPeakingAvg", |c| {
        c.energy_filter_peaking_avg.clone()
    }),
    ("EnergyFilterBaselineAvg", |c| {
        c.energy_filter_baseline_avg.clone()
    }),
    ("EnergyFilterBaselineGuardT", |c| {
        c.energy_filter_baseline_guard_ns.map(|v| v.to_string())
    }),
    ("EnergyFilterPileupGuardT", |c| {
        c.energy_filter_pileup_guard_ns.map(|v| v.to_string())
    }),
    ("EnergyFilterFineGain", |c| {
        c.energy_filter_fine_gain.map(|v| v.to_string())
    }),
    ("EnergyFilterLFLimitation", |c| {
        c.energy_filter_lf_limitation.clone()
    }),
    // ---- Per-channel S_IN/GPI (PHA2 specific). Default None on most channels. ----
    ("SINFunction", |c| c.sin_function.clone()),
    ("GPIFunction", |c| c.gpi_function.clone()),
    // ---- Coincidence (DevTree paths shared with PSD2) ----
    ("ChannelsTriggerMask", |c| c.ch_trigger_mask.clone()),
    ("CoincidenceMask", |c| c.coincidence_mask.clone()),
    ("AntiCoincidenceMask", |c| c.anti_coincidence_mask.clone()),
    ("CoincidenceLengthT", |c| {
        c.coincidence_window_ns.map(|v| v.to_string())
    }),
    ("ChannelVetoSource", |c| c.ch_veto_source.clone()),
    ("ADCVetoWidth", |c| {
        c.ch_veto_width_ns.map(|v| v.to_string())
    }),
    ("EventSelector", |c| c.event_selector.clone()),
    // ---- Waveform (DevTree paths shared with PSD2) ----
    ("WaveSaving", |c| c.wave_saving.clone()),
    ("WaveAnalogProbe0", |c| c.analog_probe_0.clone()),
    ("WaveAnalogProbe1", |c| c.analog_probe_1.clone()),
    ("WaveDigitalProbe0", |c| c.digital_probe_0.clone()),
    ("WaveDigitalProbe1", |c| c.digital_probe_1.clone()),
    ("WaveDigitalProbe2", |c| c.digital_probe_2.clone()),
    ("WaveDigitalProbe3", |c| c.digital_probe_3.clone()),
];

// ---------------------------------------------------------------------------
// PSD1 — 28 DevTree fields (snake_case CAEN register style)
// ---------------------------------------------------------------------------

pub const PSD1_PARAMS: &[ChannelParamEntry] = &[
    // ---- Input ----
    ("ch_enabled", |c| c.enabled.clone()),
    ("ch_polarity", |c| {
        c.polarity.as_deref().map(map_dig1_polarity)
    }),
    ("ch_dcoffset", |c| c.dc_offset.map(|v| v.to_string())),
    ("ch_indyn", |c| c.input_dynamic.clone()),
    ("ch_bline_nsmean", |c| c.baseline_avg.clone()),
    ("ch_bline_fixed", |c| {
        c.fixed_baseline.map(|v| v.to_string())
    }),
    // DevTree expects nanoseconds directly (expuom: -9) for *_ns fields
    ("ch_pretrg", |c| c.pre_trigger_ns.map(|v| v.to_string())),
    // ---- Trigger ----
    ("ch_discr_mode", |c| c.discriminator_mode.clone()),
    ("ch_threshold", |c| {
        c.trigger_threshold.map(|v| v.to_string())
    }),
    ("ch_cfd_delay", |c| c.cfd_delay_ns.map(|v| v.to_string())),
    ("ch_cfd_fraction", |c| c.cfd_fraction.clone()),
    ("ch_cfd_smoothexp", |c| c.input_smoothing.clone()),
    ("ch_trg_holdoff", |c| {
        c.trigger_holdoff_ns.map(|v| v.to_string())
    }),
    ("ch_self_trg_enable", |c| c.self_trigger.clone()),
    ("ch_trg_global_gen", |c| c.global_trigger_gen.clone()),
    ("ch_out_propagate", |c| c.trigger_out_propagate.clone()),
    // ---- Energy ----
    ("ch_energy_cgain", |c| c.energy_coarse_gain.clone()),
    ("ch_gate", |c| c.gate_long_ns.map(|v| v.to_string())),
    ("ch_gateshort", |c| c.gate_short_ns.map(|v| v.to_string())),
    ("ch_gatepre", |c| c.gate_pre_ns.map(|v| v.to_string())),
    ("ch_pedestal_en", |c| c.charge_pedestal_en.clone()),
    // ---- Coincidence ----
    ("ch_trg_mode", |c| c.coincidence_mode.clone()),
    ("ch_veto_src", |c| c.ch_veto_source.clone()),
    ("ch_pur_en", |c| c.pileup_rejection.clone()),
    // ---- PSD1 Extended Coincidence ----
    ("ch_trg_latency", |c| c.trigger_latency.clone()),
    ("ch_coinc_mask", |c| c.coinc_mask.map(|v| v.to_string())),
    ("ch_coinc_operation", |c| c.coinc_operation.clone()),
    ("ch_coinc_majlev", |c| {
        c.coinc_majority_level.map(|v| v.to_string())
    }),
    ("ch_coinc_trgext", |c| c.coinc_trgext.clone()),
    ("ch_coinc_trgsw", |c| c.coinc_trgsw.clone()),
    ("ch_purgap", |c| c.pileup_gap.map(|v| v.to_string())),
    ("ch_pu_count_en", |c| c.pileup_counting_en.clone()),
];

// ---------------------------------------------------------------------------
// PHA1 — 26 DevTree fields
// ---------------------------------------------------------------------------

pub const PHA1_PARAMS: &[ChannelParamEntry] = &[
    // ---- Input ----
    ("ch_enabled", |c| c.enabled.clone()),
    ("ch_polarity", |c| {
        c.polarity.as_deref().map(map_dig1_polarity)
    }),
    ("ch_dcoffset", |c| c.dc_offset.map(|v| v.to_string())),
    ("ch_cgain", |c| c.coarse_gain.clone()),
    ("ch_bline_nsmean", |c| c.baseline_avg.clone()),
    ("ch_pretrg", |c| c.pre_trigger_ns.map(|v| v.to_string())),
    // ---- Trigger ----
    ("ch_threshold", |c| {
        c.trigger_threshold.map(|v| v.to_string())
    }),
    ("ch_trg_holdoff", |c| {
        c.trigger_holdoff_ns.map(|v| v.to_string())
    }),
    ("ch_rccr2_smooth", |c| c.fast_discr_smoothing.clone()),
    ("ch_rccr2_rise", |c| {
        c.input_rise_time_ns.map(|v| v.to_string())
    }),
    ("ch_self_trg_enable", |c| c.self_trigger.clone()),
    ("ch_trg_global_gen", |c| c.global_trigger_gen.clone()),
    ("ch_out_propagate", |c| c.trigger_out_propagate.clone()),
    // ---- Energy ----
    ("ch_trap_trise", |c| {
        c.trap_rise_time_ns.map(|v| v.to_string())
    }),
    ("ch_trap_tflat", |c| {
        c.trap_flat_top_ns.map(|v| v.to_string())
    }),
    ("ch_tdecay", |c| c.trap_pole_zero_ns.map(|v| v.to_string())),
    ("ch_trap_ftd", |c| c.peaking_time.map(|v| v.to_string())),
    ("ch_peak_nsmean", |c| c.peak_nsmean.clone()),
    ("ch_peak_holdoff", |c| {
        c.peak_holdoff_ns.map(|v| v.to_string())
    }),
    ("ch_fgain", |c| c.energy_fine_gain.map(|v| v.to_string())),
    // ---- Coincidence ----
    ("ch_trg_mode", |c| c.coincidence_mode.clone()),
    ("ch_veto_src", |c| c.ch_veto_source.clone()),
    // ---- PHA1 Extended Coincidence ----
    ("ch_trg_latency", |c| c.trigger_latency.clone()),
    ("ch_coinc_mask", |c| c.coinc_mask.map(|v| v.to_string())),
    ("ch_coinc_operation", |c| c.coinc_operation.clone()),
    ("ch_coinc_majlev", |c| {
        c.coinc_majority_level.map(|v| v.to_string())
    }),
    ("ch_coinc_trgext", |c| c.coinc_trgext.clone()),
    ("ch_coinc_trgsw", |c| c.coinc_trgsw.clone()),
    // ---- PHA1 Pileup ----
    ("ch_pu_flag_en", |c| c.pileup_flag_en.clone()),
];

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_dig1_polarity_known_values() {
        assert_eq!(map_dig1_polarity("negative"), "POLARITY_NEGATIVE");
        assert_eq!(map_dig1_polarity("Negative"), "POLARITY_NEGATIVE");
        assert_eq!(map_dig1_polarity("POSITIVE"), "POLARITY_POSITIVE");
    }

    #[test]
    fn map_dig1_polarity_passes_through_unknown() {
        // Already-mapped values stay identical (round-trip safety).
        assert_eq!(map_dig1_polarity("POLARITY_NEGATIVE"), "POLARITY_NEGATIVE");
        // Random unrecognised string passes verbatim.
        assert_eq!(map_dig1_polarity("ZeroCross"), "ZeroCross");
    }

    #[test]
    fn psd2_amax_table_is_non_empty() {
        assert!(PSD2_AMAX_PARAMS.len() >= 30);
    }

    #[test]
    fn pha2_table_is_non_empty() {
        assert!(PHA2_PARAMS.len() >= 25);
    }

    #[test]
    fn psd1_table_is_non_empty() {
        assert!(PSD1_PARAMS.len() >= 25);
    }

    #[test]
    fn pha1_table_is_non_empty() {
        assert!(PHA1_PARAMS.len() >= 25);
    }

    #[test]
    fn no_table_has_duplicate_devtree_paths() {
        // A duplicate would silently emit two CAEN parameters with the same
        // path — usually a copy-paste bug. Pin against it.
        for (name, table) in [
            ("PSD2_AMAX", PSD2_AMAX_PARAMS),
            ("PHA2", PHA2_PARAMS),
            ("PSD1", PSD1_PARAMS),
            ("PHA1", PHA1_PARAMS),
        ] {
            let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
            for (path, _) in table {
                assert!(
                    seen.insert(path),
                    "{} table has duplicate DevTree path: {}",
                    name,
                    path
                );
            }
        }
    }

    #[test]
    fn psd2_amax_accessor_round_trip_for_string_field() {
        let c = ChannelConfig {
            enabled: Some("True".to_string()),
            ..Default::default()
        };
        // Find the ChEnable accessor and call it.
        let (_, accessor) = PSD2_AMAX_PARAMS
            .iter()
            .find(|(name, _)| *name == "ChEnable")
            .expect("ChEnable in PSD2_AMAX_PARAMS");
        assert_eq!(accessor(&c), Some("True".to_string()));
    }

    #[test]
    fn psd2_amax_accessor_round_trip_for_numeric_field() {
        let c = ChannelConfig {
            dc_offset: Some(20.0),
            ..Default::default()
        };
        let (_, accessor) = PSD2_AMAX_PARAMS
            .iter()
            .find(|(name, _)| *name == "DCOffset")
            .expect("DCOffset in PSD2_AMAX_PARAMS");
        assert_eq!(accessor(&c), Some("20".to_string()));
    }

    #[test]
    fn psd2_amax_accessor_returns_none_when_field_missing() {
        let c = ChannelConfig::default();
        for (_, accessor) in PSD2_AMAX_PARAMS {
            // Default config has no fields set → every accessor returns None.
            assert!(accessor(&c).is_none());
        }
    }

    #[test]
    fn psd1_polarity_maps_lowercase_input_to_register_value() {
        let c = ChannelConfig {
            polarity: Some("negative".to_string()),
            ..Default::default()
        };
        let (_, accessor) = PSD1_PARAMS
            .iter()
            .find(|(name, _)| *name == "ch_polarity")
            .expect("ch_polarity in PSD1_PARAMS");
        assert_eq!(accessor(&c), Some("POLARITY_NEGATIVE".to_string()));
    }

    #[test]
    fn pha1_polarity_maps_lowercase_input_to_register_value() {
        let c = ChannelConfig {
            polarity: Some("positive".to_string()),
            ..Default::default()
        };
        let (_, accessor) = PHA1_PARAMS
            .iter()
            .find(|(name, _)| *name == "ch_polarity")
            .expect("ch_polarity in PHA1_PARAMS");
        assert_eq!(accessor(&c), Some("POLARITY_POSITIVE".to_string()));
    }
}
