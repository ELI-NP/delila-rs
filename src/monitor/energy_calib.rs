//! Energy calibration for the DISPLAY (TODO 83): `E_keV = kev_per_adc * ADC + offset_kev`.
//!
//! The coefficients are worked out outside delila-rs and arrive as a table of
//! four columns, `module channel kev_per_adc offset_kev`. Histograms stay in ADC:
//! this module only holds the table for the REST API, and the web UI maps the axes.
//! Nothing here touches the histogram task or anything recorded.
//!
//! `tools/root_sink/common/energy_calib.hpp` reads the same format; both are tested
//! on the files in `tools/root_sink/common/fixtures/energy_calib/`.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use tracing::{error, info};

/// One channel's straight line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearCalib {
    pub kev_per_adc: f64,
    pub offset_kev: f64,
}

/// Coefficients by (module, channel). A channel not listed has none.
pub type CalibTable = BTreeMap<(u8, u8), LinearCalib>;

/// Digits only, 0..=255. `str::parse::<u8>` would also take "+1".
fn parse_id(token: &str) -> Option<u8> {
    if token.is_empty() || !token.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    token.parse().ok()
}

/// Parse the whole table. Any bad line rejects the file, and every problem is
/// reported at once with its line number, so that one reload fixes them all.
pub fn parse_energy_calib(text: &str, source: &str) -> Result<CalibTable, Vec<String>> {
    let mut table = CalibTable::new();
    let mut first_line: HashMap<(u8, u8), usize> = HashMap::new();
    let mut errors = Vec::new();

    for (index, raw) in text.lines().enumerate() {
        let lineno = index + 1;
        let where_ = format!("{source}:{lineno}");
        let line = raw.split('#').next().unwrap_or("");
        let columns: Vec<&str> = line.split_whitespace().collect();
        if columns.is_empty() {
            continue;
        }
        if columns.len() != 4 {
            errors.push(format!(
                "{where_}: expected 'module channel kev_per_adc offset_kev', got {} column(s)",
                columns.len()
            ));
            continue;
        }

        let module = parse_id(columns[0]);
        if module.is_none() {
            errors.push(format!(
                "{where_}: module '{}' is not an integer 0..255",
                columns[0]
            ));
        }
        let channel = parse_id(columns[1]);
        if channel.is_none() {
            errors.push(format!(
                "{where_}: channel '{}' is not an integer 0..255",
                columns[1]
            ));
        }
        let kev_per_adc = columns[2].parse::<f64>().ok();
        let slope_ok = kev_per_adc.is_some_and(|v| v.is_finite() && v > 0.0);
        if !slope_ok {
            errors.push(format!(
                "{where_}: kev_per_adc '{}' must be a finite number > 0",
                columns[2]
            ));
        }
        let offset_kev = columns[3].parse::<f64>().ok();
        let offset_ok = offset_kev.is_some_and(f64::is_finite);
        if !offset_ok {
            errors.push(format!(
                "{where_}: offset_kev '{}' must be a finite number",
                columns[3]
            ));
        }

        let (Some(module), Some(channel), Some(kev_per_adc), Some(offset_kev)) =
            (module, channel, kev_per_adc, offset_kev)
        else {
            continue;
        };
        if !slope_ok || !offset_ok {
            continue;
        }
        if let Some(first) = first_line.get(&(module, channel)) {
            errors.push(format!(
                "{where_}: module {module} channel {channel} is already listed on line {first}"
            ));
            continue;
        }
        first_line.insert((module, channel), lineno);
        table.insert(
            (module, channel),
            LinearCalib {
                kev_per_adc,
                offset_kev,
            },
        );
    }

    if errors.is_empty() {
        Ok(table)
    } else {
        Err(errors)
    }
}

/// State of the table in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CalibStatus {
    /// No file configured.
    None,
    /// The latest read succeeded.
    Ok,
    /// The latest read failed; the previous table is still in force.
    Stale,
    /// No read has succeeded yet, so there is no table.
    Error,
}

/// One row of [`CalibResponse`].
#[derive(Debug, Clone, Serialize)]
pub struct CalibEntry {
    pub module: u8,
    pub channel: u8,
    pub kev_per_adc: f64,
    pub offset_kev: f64,
}

/// Body of `GET /api/calibration`.
#[derive(Debug, Clone, Serialize)]
pub struct CalibResponse {
    pub configured: bool,
    pub path: Option<String>,
    pub status: CalibStatus,
    pub unit: &'static str,
    pub loaded_at: Option<String>,
    pub entries: Vec<CalibEntry>,
    pub errors: Vec<String>,
}

/// The table the Monitor serves, re-read at start-up, at every run start and on request.
#[derive(Debug, Clone)]
pub struct ActiveCalib {
    path: Option<String>,
    table: CalibTable,
    status: CalibStatus,
    loaded_at: Option<String>,
    errors: Vec<String>,
}

impl ActiveCalib {
    /// `path = None` means no calibration is configured. Nothing is read until
    /// the first [`ActiveCalib::reload`].
    pub fn new(path: Option<String>) -> Self {
        Self {
            path,
            table: CalibTable::new(),
            status: CalibStatus::None,
            loaded_at: None,
            errors: Vec::new(),
        }
    }

    pub fn is_configured(&self) -> bool {
        self.path.is_some()
    }

    pub fn status(&self) -> CalibStatus {
        self.status
    }

    pub fn lookup(&self, module: u8, channel: u8) -> Option<LinearCalib> {
        self.table.get(&(module, channel)).copied()
    }

    /// Read the file again. A file that cannot be used never empties the table:
    /// the previous one stays in force and the status says so. Returns whether
    /// the read succeeded.
    pub fn reload(&mut self) -> bool {
        let Some(path) = self.path.clone() else {
            return false;
        };
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| vec![format!("cannot read {path}: {e}")])
            .and_then(|text| parse_energy_calib(&text, &path));
        match parsed {
            Ok(table) => {
                self.table = table;
                self.status = CalibStatus::Ok;
                self.loaded_at = Some(chrono::Utc::now().to_rfc3339());
                self.errors.clear();
                info!(
                    path = %path,
                    channels = self.table.len(),
                    "Energy calibration loaded (display only, histograms stay in ADC)"
                );
                true
            }
            Err(errors) => {
                let first_load = self.loaded_at.is_none();
                self.status = if first_load {
                    CalibStatus::Error
                } else {
                    CalibStatus::Stale
                };
                error!(
                    path = %path,
                    errors = ?errors,
                    "Energy calibration not usable: {}",
                    if first_load {
                        "display stays in ADC"
                    } else {
                        "STALE, previous table kept"
                    }
                );
                self.errors = errors;
                false
            }
        }
    }

    pub fn response(&self) -> CalibResponse {
        CalibResponse {
            configured: self.is_configured(),
            path: self.path.clone(),
            status: self.status,
            unit: "keV",
            loaded_at: self.loaded_at.clone(),
            entries: self
                .table
                .iter()
                .map(|(&(module, channel), c)| CalibEntry {
                    module,
                    channel,
                    kev_per_adc: c.kev_per_adc,
                    offset_kev: c.offset_kev,
                })
                .collect(),
            errors: self.errors.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &str = "tools/root_sink/common/fixtures/energy_calib";

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{FIXTURES}/{name}")).expect("fixture")
    }

    fn line(kev_per_adc: f64, offset_kev: f64) -> LinearCalib {
        LinearCalib {
            kev_per_adc,
            offset_kev,
        }
    }

    #[test]
    fn good_table_reads_all_rows() {
        let t = parse_energy_calib(&fixture("good.txt"), "f").expect("valid");
        assert_eq!(t.len(), 3);
        assert_eq!(t[&(0, 1)], line(0.2218, 3.1));
        assert_eq!(t[&(5, 0)], line(0.3012, -12.4));
        assert_eq!(t[&(5, 1)], line(0.25, 0.0));
        assert!(!t.contains_key(&(1, 1)));
    }

    #[test]
    fn crlf_lines_are_fine() {
        let t = parse_energy_calib(&fixture("good_crlf.txt"), "f").expect("valid");
        assert_eq!(t.len(), 2);
        assert_eq!(t[&(0, 1)], line(0.5, -10.0));
    }

    #[test]
    fn comments_only_is_an_empty_table() {
        let t = parse_energy_calib(&fixture("comments_only.txt"), "f").expect("valid");
        assert!(t.is_empty());
        assert!(parse_energy_calib("", "f").expect("valid").is_empty());
    }

    #[test]
    fn wrong_column_counts_are_rejected_with_line_numbers() {
        let e = parse_energy_calib(&fixture("bad_columns.txt"), "f").expect_err("invalid");
        assert_eq!(e.len(), 3, "{e:?}");
        assert!(e[0].starts_with("f:1:"), "{e:?}");
        assert!(e[1].starts_with("f:2:"), "{e:?}");
        assert!(e[2].starts_with("f:3:"), "{e:?}");
    }

    #[test]
    fn bad_values_are_all_reported() {
        let e = parse_energy_calib(&fixture("bad_values.txt"), "f").expect_err("invalid");
        assert_eq!(e.len(), 12, "{e:?}");
        assert!(e[0].contains("kev_per_adc '0'"), "{e:?}");
        assert!(e[1].contains("kev_per_adc '-0.5'"), "{e:?}");
        assert!(e[5].contains("offset_kev 'inf'"), "{e:?}");
        assert!(e[7].contains("module '256'"), "{e:?}");
        assert!(e[10].contains("channel '256'"), "{e:?}");
        assert!(e[11].contains("kev_per_adc '0x10'"), "{e:?}");
    }

    #[test]
    fn duplicates_name_both_lines() {
        let e = parse_energy_calib(&fixture("bad_duplicate.txt"), "f").expect_err("invalid");
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].starts_with("f:4:"), "{e:?}");
        assert!(e[0].contains("line 1"), "{e:?}");
    }

    fn write(dir: &tempfile::TempDir, text: &str) -> String {
        let path = dir.path().join("calib.txt");
        std::fs::write(&path, text).expect("write");
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn no_file_configured() {
        let mut a = ActiveCalib::new(None);
        assert!(!a.reload());
        let r = a.response();
        assert!(!r.configured);
        assert_eq!(r.status, CalibStatus::None);
        assert!(r.entries.is_empty());
    }

    #[test]
    fn first_read_failing_leaves_an_error_and_no_table() {
        let mut a = ActiveCalib::new(Some("/nonexistent/calib.txt".into()));
        assert!(!a.reload());
        let r = a.response();
        assert!(r.configured);
        assert_eq!(r.status, CalibStatus::Error);
        assert!(r.entries.is_empty());
        assert!(r.errors[0].contains("cannot read"), "{:?}", r.errors);
    }

    #[test]
    fn a_bad_reload_keeps_the_previous_table() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write(&dir, "0 1 0.5 -10\n");
        let mut a = ActiveCalib::new(Some(path.clone()));
        assert!(a.reload());
        assert_eq!(a.status(), CalibStatus::Ok);
        assert_eq!(a.lookup(0, 1), Some(line(0.5, -10.0)));

        // Invalid: previous table kept.
        write(&dir, "0 1 -1 0\n");
        assert!(!a.reload());
        assert_eq!(a.status(), CalibStatus::Stale);
        assert_eq!(a.lookup(0, 1), Some(line(0.5, -10.0)));
        assert_eq!(a.response().errors.len(), 1);

        // Unreadable: previous table kept.
        std::fs::remove_file(&path).expect("remove");
        assert!(!a.reload());
        assert_eq!(a.status(), CalibStatus::Stale);
        assert_eq!(a.lookup(0, 1), Some(line(0.5, -10.0)));

        // Valid again: adopted, stale cleared.
        write(&dir, "0 2 2 1\n");
        assert!(a.reload());
        assert_eq!(a.status(), CalibStatus::Ok);
        assert_eq!(a.lookup(0, 1), None);
        assert_eq!(a.lookup(0, 2), Some(line(2.0, 1.0)));
        assert!(a.response().errors.is_empty());
    }

    #[test]
    fn response_json_shape() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write(&dir, "5 0 0.25 -1.5\n");
        let mut a = ActiveCalib::new(Some(path.clone()));
        assert!(a.reload());
        let v = serde_json::to_value(a.response()).expect("json");
        assert_eq!(v["configured"], true);
        assert_eq!(v["path"], path);
        assert_eq!(v["status"], "ok");
        assert_eq!(v["unit"], "keV");
        assert!(v["loaded_at"].is_string());
        assert_eq!(v["entries"][0]["module"], 5);
        assert_eq!(v["entries"][0]["channel"], 0);
        assert_eq!(v["entries"][0]["kev_per_adc"], 0.25);
        assert_eq!(v["entries"][0]["offset_kev"], -1.5);
        assert_eq!(v["errors"].as_array().map(Vec::len), Some(0));
    }
}
