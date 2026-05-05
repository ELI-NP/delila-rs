//! Parameter validation using DevTree metadata.
//!
//! Provides snap-to-step rounding (CoMPASS-style) and parameter validation
//! against DevTree constraints (min, max, increment, allowed_values).

use super::handle::ParamInfo;

/// Result of validating a single parameter value
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidateResult {
    /// Validated (possibly adjusted) value string
    pub value: String,
    /// Whether the value was adjusted from the original
    pub adjusted: bool,
    /// Human-readable message describing the adjustment (if any)
    pub message: Option<String>,
}

/// Status of a parameter apply operation
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum ParamApplyStatus {
    /// Value accepted as-is after DevTree validation
    Ok,
    /// Value was snapped to nearest valid step
    Adjusted,
    /// Hardware rejected the value
    Failed,
    /// Parameter not found in DevTree, set_value failed
    Skipped,
    /// Parameter not found in DevTree, set_value succeeded — value was
    /// passed straight to FW without min/max/step validation. Pre-2026-05-04
    /// this was silently counted as `Ok` and helped hide a CamelCase /
    /// lowercase cache-key bug for months. Always inspect these.
    NoCache,
}

/// Result of applying a single parameter
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParamApplyResult {
    /// DevTree parameter path (e.g., "/par/ch_gate")
    pub path: String,
    /// Original value before validation
    pub original_value: String,
    /// Value actually sent to hardware
    pub applied_value: String,
    /// Status of the operation
    pub status: ParamApplyStatus,
    /// Human-readable message
    pub message: Option<String>,
}

/// Aggregate result of applying a full configuration
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ApplyConfigResult {
    /// Total number of parameters processed
    pub total: usize,
    /// Number applied without adjustment
    pub ok: usize,
    /// Number adjusted to valid step/range
    pub adjusted: usize,
    /// Number that failed on hardware
    pub failed: usize,
    /// Number skipped (not in DevTree, set_value failed)
    pub skipped: usize,
    /// Number applied without DevTree validation (set_value succeeded but
    /// no cache entry was found — typically range-expanded paths or
    /// hand-crafted paths like `dt_ext_clock`). These are loud-by-default
    /// in the apply log so a regression of the 2026-05-04 case-insensitive
    /// lookup bug is visible — back then every CamelCase write missed the
    /// cache and silently bypassed validation.
    #[serde(default)]
    pub no_cache: usize,
    /// Per-parameter details
    pub details: Vec<ParamApplyResult>,
}

/// Snap a numeric value to the nearest valid step (CoMPASS-style round-to-nearest).
///
/// Formula: `round((value - min) / increment) * increment + min`, clamped to [min, max].
///
/// # Arguments
/// * `value` - Input value to snap
/// * `min` - Minimum allowed value
/// * `max` - Maximum allowed value
/// * `increment` - Step size (0 or negative means no snapping, just clamp)
pub fn snap_to_step(value: f64, min: f64, max: f64, increment: f64) -> f64 {
    if increment <= 0.0 {
        return value.clamp(min, max);
    }
    let steps = ((value - min) / increment).round();
    let snapped = steps * increment + min;
    snapped.clamp(min, max)
}

/// Format a snapped value as a string, matching the precision of the increment.
///
/// Integer increments (2, 8, 16) produce integer strings ("104").
/// Float increments (0.1, 0.001) produce formatted strings ("50.3").
fn format_snapped_value(value: f64, increment: f64) -> String {
    // Determine if increment is integer-valued
    if increment >= 1.0 && increment.fract() == 0.0 {
        // Integer step: cast to avoid dirty floats like "104.00000000000001"
        format!("{}", value.round() as i64)
    } else if increment > 0.0 {
        // Float step: determine decimal places from increment
        let decimal_places = decimal_places_of(increment);
        format!("{:.prec$}", value, prec = decimal_places)
    } else {
        format!("{}", value)
    }
}

/// Count the number of decimal places in a float (e.g., 0.1 → 1, 0.001 → 3)
fn decimal_places_of(value: f64) -> usize {
    let s = format!("{}", value);
    match s.find('.') {
        Some(pos) => s.len() - pos - 1,
        None => 0,
    }
}

/// Validate and possibly adjust a parameter value against DevTree metadata.
///
/// For NUMBER type: snaps to nearest valid step, clamps to [min, max].
/// For STRING type: checks against allowed_values (pass-through if not in list).
pub fn validate_param(value: &str, info: &ParamInfo) -> ValidateResult {
    match info.datatype.as_str() {
        "NUMBER" => validate_numeric(value, info),
        "STRING" => validate_enum(value, info),
        _ => ValidateResult {
            value: value.to_string(),
            adjusted: false,
            message: None,
        },
    }
}

fn validate_numeric(value: &str, info: &ParamInfo) -> ValidateResult {
    let val: f64 = match value.parse() {
        Ok(v) => v,
        Err(_) => {
            return ValidateResult {
                value: value.to_string(),
                adjusted: false,
                message: Some(format!("cannot parse '{}' as number", value)),
            };
        }
    };

    let min = info
        .min_value
        .as_ref()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(f64::MIN);
    let max = info
        .max_value
        .as_ref()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(f64::MAX);
    let increment = info
        .increment
        .as_ref()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);

    let snapped = snap_to_step(val, min, max, increment);
    let snapped_str = format_snapped_value(snapped, increment);

    // Check if adjusted (use epsilon for float comparison)
    let adjusted = (snapped - val).abs() > 1e-9;
    let message = if adjusted {
        Some(format!(
            "{} → {} (step={}, range=[{}, {}])",
            value, snapped_str, increment, min, max
        ))
    } else {
        None
    };

    ValidateResult {
        value: snapped_str,
        adjusted,
        message,
    }
}

fn validate_enum(value: &str, info: &ParamInfo) -> ValidateResult {
    if !info.allowed_values.is_empty() && !info.allowed_values.contains(&value.to_string()) {
        // Log but don't modify — hardware is the authority for enum validation
        ValidateResult {
            value: value.to_string(),
            adjusted: false,
            message: Some(format!(
                "'{}' not in allowed values: {:?}",
                value, info.allowed_values
            )),
        }
    } else {
        ValidateResult {
            value: value.to_string(),
            adjusted: false,
            message: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // snap_to_step tests
    // =========================================================================

    #[test]
    fn test_snap_to_step_round_up() {
        // 101ns with step=8, min=0 → round(101/8)=round(12.625)=13 → 13*8=104
        assert_eq!(snap_to_step(101.0, 0.0, 524280.0, 8.0), 104.0);
    }

    #[test]
    fn test_snap_to_step_round_down() {
        // 41ns with step=8, min=40 → round((41-40)/8)=round(0.125)=0 → 0*8+40=40
        assert_eq!(snap_to_step(41.0, 40.0, 2016.0, 8.0), 40.0);
    }

    #[test]
    fn test_snap_to_step_exact_value() {
        // 104ns with step=8, min=0 → already aligned
        assert_eq!(snap_to_step(104.0, 0.0, 524280.0, 8.0), 104.0);
    }

    #[test]
    fn test_snap_to_step_step_2() {
        // 101ns with step=2, min=4 → round((101-4)/2)=round(48.5)=49 → 49*2+4=102
        // Note: Rust f64::round() rounds half away from zero, so 48.5 → 49
        assert_eq!(snap_to_step(101.0, 4.0, 32766.0, 2.0), 102.0);
    }

    #[test]
    fn test_snap_to_step_step_16() {
        // 1001ns with step=16, min=16 → round((1001-16)/16)=round(61.5625)=62 → 62*16+16=1008
        assert_eq!(snap_to_step(1001.0, 16.0, 131056.0, 16.0), 1008.0);
    }

    #[test]
    fn test_snap_to_step_clamp_min() {
        // -10 with min=0 → clamped to 0
        assert_eq!(snap_to_step(-10.0, 0.0, 100.0, 1.0), 0.0);
    }

    #[test]
    fn test_snap_to_step_clamp_max() {
        // 200000 with max=524280 → within range, snap to step
        // But 600000 with max=524280 → clamped
        assert_eq!(snap_to_step(600000.0, 0.0, 524280.0, 8.0), 524280.0);
    }

    #[test]
    fn test_snap_to_step_float_increment() {
        // 50.35 with step=0.1, min=0.0 → round(50.35/0.1)=round(503.5)=504 → 504*0.1=50.4
        let result = snap_to_step(50.35, 0.0, 100.0, 0.1);
        assert!((result - 50.4).abs() < 1e-9);
    }

    #[test]
    fn test_snap_to_step_float_exact() {
        // 50.3 with step=0.1 → already aligned
        let result = snap_to_step(50.3, 0.0, 100.0, 0.1);
        assert!((result - 50.3).abs() < 1e-9);
    }

    #[test]
    fn test_snap_to_step_zero_increment() {
        // increment=0 → just clamp, no snapping
        assert_eq!(snap_to_step(50.7, 0.0, 100.0, 0.0), 50.7);
        assert_eq!(snap_to_step(150.0, 0.0, 100.0, 0.0), 100.0);
    }

    #[test]
    fn test_snap_to_step_min_equals_value() {
        // Value exactly at min
        assert_eq!(snap_to_step(40.0, 40.0, 2016.0, 8.0), 40.0);
    }

    #[test]
    fn test_snap_to_step_max_boundary() {
        // Value at max that's aligned
        assert_eq!(snap_to_step(2016.0, 40.0, 2016.0, 8.0), 2016.0);
    }

    #[test]
    fn test_snap_to_step_midpoint() {
        // Exactly midpoint: 44 with step=8, min=40 → round((44-40)/8)=round(0.5)=1 → 48
        // Rust round() uses "round half away from zero"
        assert_eq!(snap_to_step(44.0, 40.0, 2016.0, 8.0), 48.0);
    }

    // =========================================================================
    // format_snapped_value tests
    // =========================================================================

    #[test]
    fn test_format_integer_step() {
        assert_eq!(format_snapped_value(104.0, 8.0), "104");
        assert_eq!(format_snapped_value(40.0, 8.0), "40");
        assert_eq!(format_snapped_value(1008.0, 16.0), "1008");
    }

    #[test]
    fn test_format_float_step() {
        assert_eq!(format_snapped_value(50.3, 0.1), "50.3");
        assert_eq!(format_snapped_value(0.0, 0.1), "0.0");
        assert_eq!(format_snapped_value(100.0, 0.1), "100.0");
    }

    #[test]
    fn test_format_fine_float_step() {
        assert_eq!(format_snapped_value(50.001, 0.001), "50.001");
    }

    // =========================================================================
    // validate_param tests
    // =========================================================================

    fn make_numeric_info(min: &str, max: &str, increment: &str, expuom: i32) -> ParamInfo {
        ParamInfo {
            name: "test_param".to_string(),
            datatype: "NUMBER".to_string(),
            access_mode: "READ_WRITE".to_string(),
            setinrun: true,
            min_value: Some(min.to_string()),
            max_value: Some(max.to_string()),
            allowed_values: vec![],
            unit: Some("s".to_string()),
            increment: Some(increment.to_string()),
            default_value: None,
            expuom: Some(expuom),
        }
    }

    #[test]
    fn test_validate_numeric_no_adjustment() {
        let info = make_numeric_info("0", "524280", "8", -9);
        let result = validate_param("104", &info);
        assert!(!result.adjusted);
        assert_eq!(result.value, "104");
    }

    #[test]
    fn test_validate_numeric_adjusted() {
        let info = make_numeric_info("0", "524280", "8", -9);
        let result = validate_param("101", &info);
        assert!(result.adjusted);
        assert_eq!(result.value, "104");
        assert!(result.message.is_some());
        assert!(result.message.unwrap().contains("101"));
    }

    #[test]
    fn test_validate_numeric_float_step() {
        let info = make_numeric_info("0.0", "100.0", "0.1", 0);
        let result = validate_param("50.35", &info);
        assert!(result.adjusted);
        assert_eq!(result.value, "50.4");
    }

    #[test]
    fn test_validate_numeric_out_of_range() {
        let info = make_numeric_info("0", "100", "1", 0);
        let result = validate_param("150", &info);
        assert!(result.adjusted);
        assert_eq!(result.value, "100");
    }

    #[test]
    fn test_validate_numeric_invalid_string() {
        let info = make_numeric_info("0", "100", "1", 0);
        let result = validate_param("abc", &info);
        assert!(!result.adjusted);
        assert_eq!(result.value, "abc");
        assert!(result.message.unwrap().contains("cannot parse"));
    }

    #[test]
    fn test_validate_enum_valid() {
        let info = ParamInfo {
            name: "polarity".to_string(),
            datatype: "STRING".to_string(),
            access_mode: "READ_WRITE".to_string(),
            setinrun: false,
            min_value: None,
            max_value: None,
            allowed_values: vec!["Positive".to_string(), "Negative".to_string()],
            unit: None,
            increment: None,
            default_value: None,
            expuom: None,
        };
        let result = validate_param("Positive", &info);
        assert!(!result.adjusted);
        assert!(result.message.is_none());
    }

    #[test]
    fn test_validate_enum_invalid() {
        let info = ParamInfo {
            name: "polarity".to_string(),
            datatype: "STRING".to_string(),
            access_mode: "READ_WRITE".to_string(),
            setinrun: false,
            min_value: None,
            max_value: None,
            allowed_values: vec!["Positive".to_string(), "Negative".to_string()],
            unit: None,
            increment: None,
            default_value: None,
            expuom: None,
        };
        let result = validate_param("Unknown", &info);
        assert!(!result.adjusted); // Pass through, don't modify
        assert_eq!(result.value, "Unknown");
        assert!(result.message.unwrap().contains("not in allowed values"));
    }

    // =========================================================================
    // Real DevTree parameter validation (PSD1 DT5730B values)
    // =========================================================================

    #[test]
    fn test_validate_psd1_ch_pretrg() {
        // ch_pretrg: min=40, max=2016, increment=8, expuom=-9
        let info = make_numeric_info("40", "2016", "8", -9);
        let r = validate_param("101", &info);
        assert!(r.adjusted);
        assert_eq!(r.value, "104");
    }

    #[test]
    fn test_validate_psd1_ch_gate() {
        // ch_gate: min=4, max=32766, increment=2, expuom=-9
        let info = make_numeric_info("4", "32766", "2", -9);
        let r = validate_param("301", &info);
        assert!(r.adjusted);
        assert_eq!(r.value, "302");
    }

    #[test]
    fn test_validate_psd1_reclen() {
        // reclen: min=16, max=131056, increment=16, expuom=-9
        let info = make_numeric_info("16", "131056", "16", -9);
        let r = validate_param("1000", &info);
        assert!(r.adjusted);
        assert_eq!(r.value, "1008");
    }

    #[test]
    fn test_validate_psd1_dc_offset() {
        // ch_dcoffset: min=0.0, max=100.0, increment=0.1, expuom=0
        let info = make_numeric_info("0.0", "100.0", "0.1", 0);
        let r = validate_param("20.0", &info);
        assert!(!r.adjusted);
        assert_eq!(r.value, "20.0");
    }
}
