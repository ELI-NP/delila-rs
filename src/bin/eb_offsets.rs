//! `eb_offsets` — resolve and print a flat per-channel time-offset table
//! from an EB `timeSettings.json` file (SPEC § 4.3.4).
//!
//! Usage:
//!
//! ```text
//! eb_offsets <timeSettings.json> [--sort module|abs|depth] [--csv]
//! ```
//!
//! Output columns: `module, channel, absolute_offset_ns, depth, root_mod, root_ch`.

use std::path::PathBuf;
use std::process::ExitCode;

use delila_rs::event_builder::time_offsets::{ResolvedRow, TimeOffsetsFile};

#[derive(Debug, Clone, Copy)]
enum SortKey {
    /// (module, channel) ascending — default
    Channel,
    /// Absolute offset ascending
    Absolute,
    /// Depth ascending, then (module, channel)
    Depth,
}

fn parse_sort(s: &str) -> Result<SortKey, String> {
    match s.to_ascii_lowercase().as_str() {
        "module" | "channel" | "ch" => Ok(SortKey::Channel),
        "abs" | "absolute" | "offset" => Ok(SortKey::Absolute),
        "depth" => Ok(SortKey::Depth),
        other => Err(format!("unknown sort key: {other}")),
    }
}

fn print_usage() {
    eprintln!(
        "Usage: eb_offsets <timeSettings.json> [--sort module|abs|depth] [--csv]\n\
         \n\
         Resolves the offset tree in timeSettings.json and prints a flat\n\
         per-channel table of absolute offsets.\n\
         \n\
         Convention: aligned_ts = raw_ts - absolute_offset_ns\n"
    );
}

fn parse_args() -> Result<(PathBuf, SortKey, bool), String> {
    let mut args = std::env::args().skip(1);
    let mut path: Option<PathBuf> = None;
    let mut sort = SortKey::Channel;
    let mut csv = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sort" => {
                let v = args
                    .next()
                    .ok_or_else(|| "--sort needs a value".to_string())?;
                sort = parse_sort(&v)?;
            }
            "--csv" => csv = true,
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown flag: {other}"));
            }
            other => {
                if path.is_some() {
                    return Err(format!("only one input path allowed, got extra: {other}"));
                }
                path = Some(PathBuf::from(other));
            }
        }
    }

    let path = path.ok_or_else(|| "missing input timeSettings.json path".to_string())?;
    Ok((path, sort, csv))
}

fn sort_rows(rows: &mut [ResolvedRow], key: SortKey) {
    match key {
        SortKey::Channel => rows.sort_by_key(|r| (r.module, r.channel)),
        SortKey::Absolute => {
            rows.sort_by(|a, b| {
                a.absolute_offset_ns
                    .partial_cmp(&b.absolute_offset_ns)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| (a.module, a.channel).cmp(&(b.module, b.channel)))
            });
        }
        SortKey::Depth => rows.sort_by_key(|r| (r.depth, r.module, r.channel)),
    }
}

fn print_csv(rows: &[ResolvedRow]) {
    println!("module,channel,absolute_offset_ns,depth,root_module,root_channel");
    for r in rows {
        println!(
            "{},{},{:.6},{},{},{}",
            r.module, r.channel, r.absolute_offset_ns, r.depth, r.root.0, r.root.1
        );
    }
}

fn print_table(rows: &[ResolvedRow]) {
    println!(
        "{:>4} {:>4}  {:>14}  {:>5}  {:>4} {:>4}",
        "mod", "ch", "abs_offset_ns", "depth", "rmod", "rch"
    );
    println!("{}", "-".repeat(50));
    for r in rows {
        println!(
            "{:>4} {:>4}  {:>14.3}  {:>5}  {:>4} {:>4}",
            r.module, r.channel, r.absolute_offset_ns, r.depth, r.root.0, r.root.1
        );
    }
}

fn main() -> ExitCode {
    let (path, sort, csv) = match parse_args() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: {e}\n");
            print_usage();
            return ExitCode::from(2);
        }
    };

    let file = match TimeOffsetsFile::load(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: failed to load {}: {e}", path.display());
            return ExitCode::from(1);
        }
    };

    let resolved = match file.resolve() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: failed to resolve: {e}");
            return ExitCode::from(1);
        }
    };

    for w in &resolved.warnings {
        eprintln!("warning: {w}");
    }
    if resolved.root_count() > 1 {
        eprintln!(
            "warning: {} disconnected timing domains — see `root_module/root_channel` columns",
            resolved.root_count()
        );
    }

    let mut rows: Vec<ResolvedRow> = resolved.iter().collect();
    sort_rows(&mut rows, sort);

    if csv {
        print_csv(&rows);
    } else {
        print_table(&rows);
    }

    ExitCode::SUCCESS
}
