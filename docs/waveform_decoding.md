# Waveform Decoding Specification

## Design Principle

**All probes output exactly `total_samples` points** (= raw sample count from hardware).

This ensures all probe arrays have identical length and can be overlaid on the same
x-axis without alignment issues. The x-axis index corresponds to the sample index;
multiply by `time_step_ns` to obtain physical time.

## Firmware Comparison

| | PSD1 (DPP-PSD, DIG1) | PHA1 (DPP-PHA, DIG1) | PSD2 (DPP-PSD, DIG2) |
|---|---|---|---|
| Word size | 32-bit LE | 32-bit LE | 64-bit BE |
| Samples/word | 2 | 2 | 2 |
| Analog probes | 1 (14-bit signed) | 1 (14-bit signed) | 2 (AP1 + AP2 per sample) |
| Digital probes | 2 (DP1, DP2) | 2 (DP, Trigger) | 4 (DP1-DP4) |
| Dual trace | Yes (interleaved) | Yes (interleaved) | N/A (always both) |

## PSD1/PHA1 Word Format (32-bit)

```
Bit layout per word:
  [15:0]  = Lower sample (2N)
    [13:0]  = Analog value (14-bit signed)
    [14]    = Digital Probe 1 (PSD1: DP1, PHA1: DP)
    [15]    = Digital Probe 2 (PSD1: DP2, PHA1: Trigger flag)
  [31:16] = Upper sample (2N+1)
    [29:16] = Analog value (14-bit signed)
    [30]    = Digital Probe 1
    [31]    = Digital Probe 2
```

### Record Length Calculation

```
num_samples_wave = channel_header_bits[15:0]  (= actual_samples / 8)
total_words      = num_samples_wave * 4
total_samples    = num_samples_wave * 8       (= total_words * 2)
```

Example: DT5730B (500 MHz, 2 ns/sample), record length 1024 ns:
- actual samples = 512
- num_samples_wave = 64
- total_words = 256
- total_samples = 512

### Single Trace Mode (DT=0)

All analog values go to `analog_probe1`. No duplication.

Per word: 2 pushes to AP1, 2 pushes to each DP.

```
Output:
  analog_probe1:  [s0, s1, s2, s3, ...]     = total_samples points
  digital_probe1: [dp0, dp1, dp2, dp3, ...]  = total_samples points
  digital_probe2: [dp0, dp1, dp2, dp3, ...]  = total_samples points
```

### Dual Trace Mode (DT=1)

CAEN DPP-PSD/PHA firmware packs dual-trace samples in reverse VTrace order:
- Even samples (lower half of word, s1) = **VTrace 1** → `analog_probe2`
- Odd samples (upper half of word, s2) = **VTrace 0** → `analog_probe1`

Each analog probe has half the unique samples (total_samples / 2).
To align with digital probes (which have all total_samples), each analog
sample is duplicated once (sample-and-hold: the ADC value is held constant
for the time bin where the other probe was being sampled).

Per word: 2 pushes to AP1 (from odd/s2), 2 pushes to AP2 (from even/s1), 2 pushes to each DP.

```
Output:
  analog_probe1:  [s1, s1, s3, s3, ...]     = total_samples points (VTrace 0, 256 unique x 2)
  analog_probe2:  [s0, s0, s2, s2, ...]     = total_samples points (VTrace 1, 256 unique x 2)
  digital_probe1: [dp0, dp1, dp2, dp3, ...]  = total_samples points
  digital_probe2: [dp0, dp1, dp2, dp3, ...]  = total_samples points
```

This duplication is physically correct: in dual trace mode, the ADC alternates
between two analog inputs at half the sampling rate per input. The sample-and-hold
representation accurately reflects that the analog value does not change between
consecutive samples of the same probe.

## PSD2 Word Format (64-bit)

```
Each 64-bit word contains 2 x 32-bit samples.
Per 32-bit sample:
  [13:0]  = Analog Probe 1 (14-bit signed)
  [14]    = Digital Probe 1
  [15]    = Digital Probe 2
  [29:16] = Analog Probe 2 (14-bit signed)
  [30]    = Digital Probe 3
  [31]    = Digital Probe 4
```

PSD2 has no dual trace issue because both analog probes are packed into every
sample. All probes naturally have the same number of points without duplication.

## Source Files

- `src/reader/decoder/psd1.rs` — PSD1 (DPP-PSD, x725/x730) decoder
- `src/reader/decoder/pha1.rs` — PHA1 (DPP-PHA, x725/x730) decoder
- `src/reader/decoder/psd2.rs` — PSD2 (DPP-PSD, x2745/VX2745) decoder
- `src/reader/decoder/common.rs` — Shared `Waveform` and `EventData` structs
