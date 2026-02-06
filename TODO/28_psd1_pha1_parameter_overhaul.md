# PSD1/PHA1 Parameter Support Overhaul

**Status: COMPLETED**
**Priority: HIGH**
**Created: 2026-02-06**

## Problem Summary

PSD1/PHA1 board parameters use incorrect DevTree paths, causing CAEN errors:
```
Some parameters failed to apply: ["/par/startsource", "/par/gpiomode", "/par/globaltriggersource"]
```

### Root Cause
The backend sends PSD2-style paths for all firmware types:
- `/par/startsource` → should be `/par/startmode` for PSD1/PHA1
- `/par/gpiomode` → should be `/par/out_selection` for PSD1/PHA1
- `/par/globaltriggersource` → does NOT exist for PSD1/PHA1

## Implementation Tasks

### Phase 1: Critical Path Fixes (Backend)

- [x] **1.1** Fix `add_board_parameters()` in `src/config/digitizer.rs`
  - Use firmware-specific paths for start_source, gpio_mode
  - Skip global_trigger_source for PSD1/PHA1
  - Location: ~line 720-790

- [x] **1.2** Add unit tests for PSD1 parameter paths
  ```rust
  #[test]
  fn test_psd1_board_params_use_correct_paths()
  ```

### Phase 2: Add Missing Board Parameters

- [x] **2.1** Add new fields to `BoardConfig` struct (~line 148)
  - `ext_trigger_enable: Option<String>`
  - `sw_trigger_enable: Option<String>`
  - `io_level: Option<String>`
  - `ext_clock: Option<String>`
  - `start_delay: Option<u32>`
  - `extras_enabled: Option<String>`
  - `event_aggregation: Option<u32>`
  - `coinc_trgout: Option<u32>`

- [x] **2.2** Add board parameter mapping in `add_board_parameters()`

### Phase 3: Add Missing Channel Parameters

- [x] **3.1** Add new fields to `ChannelConfig` struct (~line 212)
  - `trigger_latency: Option<String>`
  - `coinc_mask: Option<u32>`
  - `coinc_operation: Option<String>`
  - `coinc_majority_level: Option<u32>`
  - `coinc_trgext: Option<String>`
  - `coinc_trgsw: Option<String>`
  - `pileup_gap: Option<u32>`

- [x] **3.2** Update `add_channel_params()` PSD1 arm (~line 1070)

- [x] **3.3** Update `get_channel_config()` merge macro

- [x] **3.4** Update `set_in_run_param_names()` for PSD1

### Phase 4: Frontend Type Updates

- [x] **4.1** Update `web/operator-ui/src/app/models/types.ts`
  - Add new fields to `BoardConfig` interface
  - Add new fields to `ChannelConfig` interface

### Phase 5: Frontend Parameter Definitions

- [x] **5.1** Update `web/operator-ui/src/app/models/channel-params.ts`
  - Update `PSD1_TRIGGER_PARAMS` with trigger_latency
  - Update `PSD1_COINCIDENCE_PARAMS` with extended coincidence params
  - Add pileup_gap to appropriate category

- [x] **5.2** Update `CHANNEL_PARAM_KEYS` in `digitizer.service.ts`

### Phase 6: UI Updates

- [x] **6.1** Update Board tab in `digitizer-settings.component.ts`
  - Hide Global Trigger Source for PSD1/PHA1
  - Add PSD1-specific board controls
  - Rename GPO Mode to Output Selection for PSD1

## Files to Modify

| File | Changes |
|------|---------|
| `src/config/digitizer.rs` | Board/channel param paths, new fields |
| `web/operator-ui/src/app/models/types.ts` | TypeScript interfaces |
| `web/operator-ui/src/app/models/channel-params.ts` | PSD1 param definitions |
| `web/operator-ui/src/app/services/digitizer.service.ts` | CHANNEL_PARAM_KEYS |
| `web/operator-ui/src/app/components/digitizer-settings/digitizer-settings.component.ts` | Board tab UI |

## Verification

### Build & Test
```bash
# Rust tests
cargo test --lib

# Clippy
cargo clippy -- -D warnings

# Frontend build
cd web/operator-ui && ng build
```

### Hardware Test
```bash
# Deploy to remote
./scripts/deploy_reader.sh config/config_psd1_test.toml --build

# Start DAQ
./scripts/start_daq.sh config/config_psd1_test.toml --no-mongo

# Check for errors
ssh aogaki@172.18.4.147 "grep -i 'failed\|error' ~/WorkSpace/delila-rs/logs/remote/reader_0.log"

# Verify triggers work
curl -X POST http://localhost:8080/api/configure
curl -X POST http://localhost:8080/api/start
```

## Reference

- [docs/psd1_pha1_parameter_reference.md](../docs/psd1_pha1_parameter_reference.md) - Complete parameter reference
- [docs/devtree_examples/dt5730b_psd1_sn990.json](../docs/devtree_examples/dt5730b_psd1_sn990.json) - PSD1 DevTree
- [docs/devtree_examples/dt5730b_pha1_sn990.json](../docs/devtree_examples/dt5730b_pha1_sn990.json) - PHA1 DevTree
- [docs/compass_devtree_mapping.md](../docs/compass_devtree_mapping.md) - CoMPASS mapping

## Notes

- All timing parameters use nanoseconds in config, converted to samples at CAEN level
- PSD1 uses 500 MHz sampling (2 ns/sample)
- Waveform params are board-level but should be accessible from channel settings for Tune Up mode convenience
