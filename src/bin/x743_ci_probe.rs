//! V1743 Charge Mode (DPP-CI) API compatibility probe
//!
//! Systematically tests which CAENDigitizer API functions work in Charge Mode.
//! All tests are performed AFTER switching to Charge Mode via SetSAMAcquisitionMode(DPP_CI).
//!
//! Usage:
//!   cargo run --release --features x743 --bin x743_ci_probe -- [options]

use clap::Parser;
use delila_rs::reader::caen_legacy::ffi;
use delila_rs::reader::caen_legacy::*;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "x743_ci_probe", about = "V1743 Charge Mode API probe")]
struct Args {
    #[arg(long, default_value = "optical")]
    link_type: String,
    #[arg(long, default_value_t = 0)]
    link_num: u32,
    #[arg(long, default_value_t = 0)]
    conet_node: u32,
    #[arg(long, default_value = "0")]
    base_address: String,
}

/// Test result helper
fn test_api(name: &str, result: Result<(), DigitizerError>) {
    match result {
        Ok(()) => println!("  [OK]   {}", name),
        Err(e) => println!("  [FAIL] {}: {:?}", name, e),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let args = Args::parse();
    let link_type = match args.link_type.as_str() {
        "optical" | "opt" => ConnectionType::OpticalLink,
        "usb" => ConnectionType::USB,
        _ => {
            eprintln!("Unknown link type. Use 'optical' or 'usb'");
            std::process::exit(1);
        }
    };
    let base_address = if args.base_address.starts_with("0x") {
        u32::from_str_radix(&args.base_address[2..], 16)?
    } else {
        args.base_address.parse()?
    };

    println!("=== V1743 Charge Mode (DPP-CI) API Probe ===\n");

    // Open
    let handle = X743Handle::open(link_type, args.link_num, args.conet_node, base_address)?;
    if let Some(info) = handle.board_info() {
        println!("Board: {}, SN:{}, CH:{}, ADC:{}bit", info.model_name, info.serial_number, info.channels, info.adc_nbits);
        println!("ROC FW: {}, AMC FW: {}", info.roc_firmware, info.amc_firmware);
        println!("SAM Correction: {}", if info.sam_correction_loaded { "loaded" } else { "NOT loaded" });
        println!();
    }

    // Reset
    println!("--- Reset & Mode Switch ---");
    test_api("Reset", handle.reset());

    // Switch to Charge Mode
    test_api("SetSAMAcquisitionMode(DPP_CI)", handle.set_sam_acquisition_mode(SamAcquisitionMode::DppCI));
    println!();

    // === A: Basic config in Charge Mode ===
    println!("--- A: Basic Config (Charge Mode) ---");
    test_api("SetGroupEnableMask(0xFF)", handle.set_group_enable_mask(0xFF));
    test_api("SetRecordLength(256)", handle.set_record_length(256));
    test_api("SetSAMSamplingFrequency(3.2GHz)", handle.set_sam_sampling_frequency(SamFrequency::Ghz3_2));
    test_api("SetSAMCorrectionLevel(ALL)", handle.set_sam_correction_level(SamCorrectionLevel::All));
    for g in 0..8u32 {
        let r = handle.set_sam_post_trigger_size(g, 20);
        if g == 0 { test_api("SetSAMPostTriggerSize(0, 20)", r); }
        else if let Err(e) = r { println!("  [FAIL] SetSAMPostTriggerSize({}, 20): {:?}", g, e); }
    }
    test_api("SetMaxNumEventsBLT(100)", handle.set_max_num_events_blt(100));
    test_api("SetIOLevel(NIM)", handle.set_io_level(IOLevel::NIM));
    test_api("SetAcquisitionMode(SW_CONTROLLED)", handle.set_acquisition_mode(AcqMode::SWControlled));
    println!();

    // === B: Channel config in Charge Mode ===
    println!("--- B: Channel Config (Charge Mode) ---");
    test_api("SetChannelDCOffset(0, 0x7FFF)", handle.set_channel_dc_offset(0, 0x7FFF));
    test_api("SetChannelTriggerThreshold(0, 3000)", handle.set_channel_trigger_threshold(0, 3000));
    test_api("SetTriggerPolarity(0, FallingEdge)", handle.set_trigger_polarity(0, TriggerPolarity::FallingEdge));
    test_api("SetChannelPulsePolarity(0, Negative)", handle.set_channel_pulse_polarity(0, PulsePolarity::Negative));
    test_api("SetChannelSelfTrigger(AcqOnly, 0xFF)", handle.set_channel_self_trigger(TriggerMode::AcqOnly, 0xFF));
    println!();

    // === C: Trigger logic in Charge Mode ===
    println!("--- C: Trigger Logic (Charge Mode) ---");
    test_api("SetChannelPairTriggerLogic(0, 1, OR, 15)", handle.set_channel_pair_trigger_logic(0, 1, TriggerLogic::Or, 15));
    test_api("SetTriggerLogic(OR, 0)", handle.set_trigger_logic(TriggerLogic::Or, 0));
    test_api("SetSWTriggerMode(AcqOnly)", handle.set_sw_trigger_mode(TriggerMode::AcqOnly));
    test_api("SetExtTriggerInputMode(Disabled)", handle.set_ext_trigger_input_mode(TriggerMode::Disabled));
    // SAMTriggerCountVetoParam - call via FFI directly
    let ret = unsafe { ffi::CAEN_DGTZ_SetSAMTriggerCountVetoParam(handle.raw_handle(), 0, ffi::CAEN_DGTZ_EnaDis_t::CAEN_DGTZ_DISABLE, 0) };
    test_api("SetSAMTriggerCountVetoParam(0, Disable, 0)", DigitizerError::check(ret, "SetSAMTriggerCountVetoParam"));
    println!();

    // === D: SetDPPParameters — DPP_CI_Params_t vs DPP_X743_Params_t ===
    println!("--- D: SetDPPParameters Systematic Test ---");

    // D-1: DPP_CI_Params_t (V1720 系, 前回のテスト再現)
    {
        let mut params: ffi::CAEN_DGTZ_DPP_CI_Params_t = unsafe { std::mem::zeroed() };
        for i in 0..8 {
            params.thr[i] = 100;
            params.gate[i] = 50;
            params.pgate[i] = 5;
            params.csens[i] = 0;
            params.nsbl[i] = 2;
            params.tvaw[i] = 50;
            params.selft[i] = 0;
            params.trgc[i] = 1;
        }
        params.trgho = 100;
        test_api(
            "D-1: DPP_CI_Params_t, mask=0xFF",
            handle.set_dpp_parameters(0xFF, &mut params),
        );
    }

    // D-2: DPP_X743_Params_t (V1743 専用, zeroed)
    {
        let mut params: ffi::CAEN_DGTZ_DPP_X743_Params_t = unsafe { std::mem::zeroed() };
        let ret = unsafe {
            ffi::CAEN_DGTZ_SetDPPParameters(
                handle.raw_handle(),
                0xFF,
                &mut params as *mut ffi::CAEN_DGTZ_DPP_X743_Params_t as *mut std::ffi::c_void,
            )
        };
        test_api("D-2: DPP_X743_Params_t zeroed, mask=0xFF", DigitizerError::check(ret, "SetDPPParameters"));
    }

    // D-3: DPP_X743_Params_t with reasonable values
    {
        let mut params: ffi::CAEN_DGTZ_DPP_X743_Params_t = unsafe { std::mem::zeroed() };
        params.disableSuppressBaseline = ffi::CAEN_DGTZ_EnaDis_t::CAEN_DGTZ_DISABLE;
        for i in 0..16 {
            params.chargeLength[i] = 50;
            params.enableChargeThreshold[i] = ffi::CAEN_DGTZ_EnaDis_t::CAEN_DGTZ_DISABLE;
            params.chargeThreshold[i] = 0.0;
            params.startCell[i] = 0;
        }
        let ret = unsafe {
            ffi::CAEN_DGTZ_SetDPPParameters(
                handle.raw_handle(),
                0xFF,
                &mut params as *mut ffi::CAEN_DGTZ_DPP_X743_Params_t as *mut std::ffi::c_void,
            )
        };
        test_api("D-3: DPP_X743_Params_t with values, mask=0xFF", DigitizerError::check(ret, "SetDPPParameters"));
    }

    // D-4: DPP_X743_Params_t, mask=0x01 (group 0 only)
    {
        let mut params: ffi::CAEN_DGTZ_DPP_X743_Params_t = unsafe { std::mem::zeroed() };
        params.chargeLength[0] = 20;
        params.chargeLength[1] = 20;
        let ret = unsafe {
            ffi::CAEN_DGTZ_SetDPPParameters(
                handle.raw_handle(),
                0x01,
                &mut params as *mut ffi::CAEN_DGTZ_DPP_X743_Params_t as *mut std::ffi::c_void,
            )
        };
        test_api("D-4: DPP_X743_Params_t, mask=0x01", DigitizerError::check(ret, "SetDPPParameters"));
    }

    // D-5: DPP_X743_Params_t in Standard Mode
    println!("  --- Retry D-2 in Standard Mode ---");
    test_api("Switch to STANDARD", handle.set_sam_acquisition_mode(SamAcquisitionMode::Standard));
    {
        let mut params: ffi::CAEN_DGTZ_DPP_X743_Params_t = unsafe { std::mem::zeroed() };
        let ret = unsafe {
            ffi::CAEN_DGTZ_SetDPPParameters(
                handle.raw_handle(),
                0xFF,
                &mut params as *mut ffi::CAEN_DGTZ_DPP_X743_Params_t as *mut std::ffi::c_void,
            )
        };
        test_api("D-5: DPP_X743_Params_t zeroed in Standard, mask=0xFF", DigitizerError::check(ret, "SetDPPParameters"));
    }
    // Restore Charge Mode
    test_api("Restore DPP_CI mode", handle.set_sam_acquisition_mode(SamAcquisitionMode::DppCI));
    println!();

    // Other DPP APIs
    println!("--- D (cont): Other DPP APIs (Charge Mode) ---");
    test_api("SetDPPAcquisitionMode(List, EnergyAndTime)", handle.set_dpp_acquisition_mode(DppAcqMode::List, DppSaveParam::EnergyAndTime));
    test_api("SetDPPEventAggregation(0, 0)", handle.set_dpp_event_aggregation(0, 0));
    test_api("SetDPPPreTriggerSize(-1, 32)", handle.set_dpp_pre_trigger_size(-1, 32));

    let dpp_malloc_result = handle.malloc_dpp_events();
    match &dpp_malloc_result {
        Ok(_) => println!("  [OK]   MallocDPPEvents"),
        Err(e) => println!("  [FAIL] MallocDPPEvents: {:?}", e),
    }
    println!();

    // === E: Pulse Generator in Charge Mode ===
    println!("--- E: Pulse Generator (Charge Mode) ---");
    test_api("EnableSAMPulseGen(0, 0xFFFF, Cont)", handle.enable_sam_pulse_gen(0, 0xFFFF, SamPulseSource::Continuous));
    test_api("DisableSAMPulseGen(0)", handle.disable_sam_pulse_gen(0));
    println!();

    // === F: Data acquisition test in Charge Mode ===
    println!("--- F: Data Acquisition (Charge Mode + SW Trigger) ---");

    // Setup: SW trigger, self-trigger for pulse test
    let _ = handle.set_sw_trigger_mode(TriggerMode::AcqOnly);
    let _ = handle.set_ext_trigger_input_mode(TriggerMode::Disabled);
    let _ = handle.enable_sam_pulse_gen(0, 0xFFFF, SamPulseSource::Continuous);
    let _ = handle.set_channel_self_trigger(TriggerMode::AcqOnly, 0x01);

    let mut readout_buf = handle.malloc_readout_buffer()?;
    let mut event_buf = handle.allocate_event()?;

    let _ = handle.clear_data();
    test_api("SWStartAcquisition", handle.sw_start_acquisition());

    // Send SW triggers and try to read
    let mut total_events = 0u32;
    for i in 0..10 {
        let _ = handle.send_sw_trigger();
        std::thread::sleep(Duration::from_millis(100));

        match handle.read_data(&mut readout_buf) {
            Ok(0) => {}
            Ok(data_size) => {
                println!("  ReadData: {} bytes (iteration {})", data_size, i);

                // Standard readout path
                match handle.get_num_events(&readout_buf, data_size) {
                    Ok(num_events) => {
                        println!("  GetNumEvents: {}", num_events);
                        total_events += num_events;

                        for evt_idx in 0..num_events.min(3) {
                            match handle.get_event_info(&readout_buf, data_size, evt_idx) {
                                Ok((info, ptr)) => {
                                    if let Err(e) = handle.decode_event(ptr, &mut event_buf) {
                                        println!("  [FAIL] DecodeEvent: {:?}", e);
                                        continue;
                                    }
                                    let event = event_buf.event();
                                    println!("  Event {}: counter={}, time_tag={}, group_mask=0x{:02X}",
                                        evt_idx, info.EventCounter, info.TriggerTimeTag, info.ChannelMask);

                                    for g in 0..8usize {
                                        if event.GrPresent[g] == 0 { continue; }
                                        let group = &event.DataGroup[g];
                                        println!("    Group {}: ChSize={}, TDC={}, Charge={:.2}, Peak={:.2}, Baseline={:.2}",
                                            g, group.ChSize, group.TDC, group.Charge, group.Peak, group.Baseline);
                                    }
                                }
                                Err(e) => println!("  [FAIL] GetEventInfo: {:?}", e),
                            }
                        }
                    }
                    Err(e) => println!("  [FAIL] GetNumEvents: {:?}", e),
                }

                // DPP readout path (try)
                if let Ok(mut dpp_buf) = handle.malloc_dpp_events() {
                    match handle.get_dpp_events(&readout_buf, data_size, &mut dpp_buf) {
                        Ok(num_arr) => {
                            let total: u32 = num_arr.iter().sum();
                            println!("  GetDPPEvents: total={} (per-ch: {:?})", total, &num_arr[..8]);
                        }
                        Err(e) => println!("  [FAIL] GetDPPEvents: {:?}", e),
                    }
                }
            }
            Err(e) => {
                if i == 0 { println!("  ReadData error: {:?}", e); }
            }
        }
    }

    let _ = handle.sw_stop_acquisition();
    let _ = handle.disable_sam_pulse_gen(0);
    println!("  Total events: {}", total_events);
    println!();

    // === G: Register exploration ===
    println!("--- G: Register Exploration (Charge Mode) ---");
    let regs: &[(u32, &str)] = &[
        (0x8000, "Board Config"),
        (0x8004, "Board Config BIT SET"),
        (0x8008, "Board Config BIT CLR"),
        (0x800C, "Buffer Organization"),
        (0x8100, "Acquisition Control"),
        (0x8104, "Acquisition Status"),
        (0x810C, "Trigger Source Enable"),
        (0x8110, "Front Panel TRG-OUT Enable"),
        (0x8120, "Post Trigger / Charge?"),
        (0x8138, "Front Panel I/O Control"),
        (0x8140, "Board Info"),
        (0x8178, "Board Fail Status"),
        // Per-group registers (group 0)
        (0x1000, "Group 0: ???"),
        (0x1004, "Group 0: ???"),
        (0x1008, "Group 0: ???"),
        (0x100C, "Group 0: ???"),
        (0x1010, "Group 0: ???"),
        (0x1020, "Group 0: Threshold?"),
        (0x1024, "Group 0: ???"),
        (0x1028, "Group 0: ???"),
        (0x102C, "Group 0: ???"),
        (0x1030, "Group 0: ???"),
        (0x1034, "Group 0: ???"),
        (0x1038, "Group 0: ???"),
        (0x103C, "Group 0: ???"),
        (0x1040, "Group 0: ???"),
        (0x1080, "Group 0: Status?"),
        (0x1088, "Group 0: AMC FW?"),
        (0x1098, "Group 0: DC Offset?"),
    ];

    for &(addr, desc) in regs {
        match handle.read_register(addr) {
            Ok(val) => println!("  [0x{:04X}] {}: 0x{:08X} ({})", addr, desc, val, val),
            Err(_) => println!("  [0x{:04X}] {}: READ ERROR", addr, desc),
        }
    }
    println!();

    // Additional: scan for Charge Mode specific registers in group 0 range
    println!("--- Register Scan: Group 0 (0x1000-0x10FF) ---");
    for offset in (0x1000..=0x10FCu32).step_by(4) {
        match handle.read_register(offset) {
            Ok(val) if val != 0 && val != 0xFFFFFFFF => {
                println!("  [0x{:04X}] = 0x{:08X} ({})", offset, val, val);
            }
            _ => {}
        }
    }

    println!("\n=== Probe complete ===");
    Ok(())
}
