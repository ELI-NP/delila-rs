use delila_rs::config::digitizer::DigitizerConfig;
use std::fs;
fn main() {
    let txt = fs::read_to_string("config/digitizers/amax_56.json").unwrap();
    let cfg: DigitizerConfig = serde_json::from_str(&txt).unwrap();
    println!("num_channels = {}", cfg.num_channels);
    println!(
        "channel_defaults.enabled = {:?}",
        cfg.channel_defaults.enabled
    );
    println!(
        "channel_defaults.polarity = {:?}",
        cfg.channel_defaults.polarity
    );
    println!(
        "channel_defaults.dc_offset = {:?}",
        cfg.channel_defaults.dc_offset
    );
    let amax_cfg = cfg.channel_defaults.amax.as_ref();
    println!(
        "channel_defaults.amax.thrs = {:?}",
        amax_cfg.and_then(|a| a.thrs)
    );
    println!(
        "channel_defaults.amax.polarity = {:?}",
        amax_cfg.and_then(|a| a.polarity)
    );
    println!();
    let params = cfg.to_caen_parameters();
    println!("to_caen_parameters().len() = {}", params.len());
    for p in &params {
        println!("  {}  =  {}", p.path, p.value);
    }
}
