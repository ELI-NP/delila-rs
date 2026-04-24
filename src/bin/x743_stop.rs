//! V1743 cleanup utility: opens the digitizer, issues SWStopAcquisition +
//! ClearData, reports Board Fail Status, and closes. Use to return hardware
//! to a clean Stopped state if a previous program exited without stopping.
//!
//! Usage: x743_stop [--link-type optical|usb] [--link-num N] [--conet-node N]

use clap::Parser;
use delila_rs::reader::caen_legacy::*;
use tracing::{info, warn};

#[derive(Parser)]
#[command(name = "x743_stop", about = "V1743 stop/clear utility")]
struct Args {
    #[arg(long, default_value = "optical")]
    link_type: String,
    #[arg(long, default_value_t = 0)]
    link_num: u32,
    #[arg(long, default_value_t = 0)]
    conet_node: u32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let link_type = match args.link_type.as_str() {
        "optical" => ConnectionType::OpticalLink,
        "usb" => ConnectionType::USB,
        other => return Err(format!("bad link_type '{}'", other).into()),
    };

    let h = X743Handle::open(link_type, args.link_num, args.conet_node, 0)?;

    // Best-effort stop — proceed even on error, so we don't leave hw half-touched.
    match h.sw_stop_acquisition() {
        Ok(()) => info!("SWStopAcquisition OK"),
        Err(e) => warn!("SWStopAcquisition failed: {}", e),
    }

    match h.clear_data() {
        Ok(()) => info!("ClearData OK"),
        Err(e) => warn!("ClearData failed: {}", e),
    }

    match h.read_register(0x8178) {
        Ok(s) => info!("Board Fail Status (0x8178) = 0x{:08X}", s),
        Err(e) => warn!("read 0x8178 failed: {}", e),
    }

    // Drop closes the handle. Acquisition state has been set to stopped above.
    Ok(())
}
