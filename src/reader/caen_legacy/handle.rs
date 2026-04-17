//! Safe RAII wrapper for CAENDigitizer handle
//!
//! X743Handle owns the digitizer connection and ensures proper cleanup on drop.
//! All CAENDigitizer API calls go through this handle.

use super::error::DigitizerError;
use super::ffi;
use crate::config::digitizer::{DigitizerConfig, X743Config};
use std::ffi::CStr;
use tracing::{debug, info, warn};

/// Maximum number of groups in V1743
pub const MAX_GROUPS: usize = 8;
/// Channels per group in V1743
pub const CHANNELS_PER_GROUP: usize = 2;
/// Maximum number of channels (groups × channels_per_group)
pub const MAX_CHANNELS: usize = MAX_GROUPS * CHANNELS_PER_GROUP;

/// Board information retrieved from the digitizer
#[derive(Debug, Clone)]
pub struct BoardInfo {
    pub model_name: String,
    pub model: u32,
    pub channels: u32,
    pub form_factor: u32,
    pub family_code: u32,
    pub roc_firmware: String,
    pub amc_firmware: String,
    pub serial_number: u32,
    pub adc_nbits: u32,
    pub sam_correction_loaded: bool,
}

/// Safe wrapper for CAENDigitizer handle (RAII)
///
/// Automatically closes the digitizer connection when dropped.
pub struct X743Handle {
    handle: i32,
    board_info: Option<BoardInfo>,
}

impl X743Handle {
    /// Open a connection to a V1743 digitizer
    ///
    /// # Arguments
    /// * `link_type` - Connection type (OpticalLink, USB, etc.)
    /// * `link_num` - Link number (port number for optical link)
    /// * `conet_node` - CONET node (daisy chain position, 0 for first)
    /// * `vme_base_address` - VME base address (0 for auto)
    pub fn open(
        link_type: ConnectionType,
        link_num: u32,
        conet_node: u32,
        vme_base_address: u32,
    ) -> Result<Self, DigitizerError> {
        let mut handle: i32 = -1;

        info!(
            "Opening V1743: link_type={:?}, link_num={}, conet_node={}, base=0x{:08X}",
            link_type, link_num, conet_node, vme_base_address
        );

        let ret = unsafe {
            ffi::CAEN_DGTZ_OpenDigitizer(
                link_type.to_ffi(),
                link_num as i32,
                conet_node as i32,
                vme_base_address,
                &mut handle,
            )
        };
        DigitizerError::check(ret, "OpenDigitizer")?;

        info!("V1743 opened successfully, handle={}", handle);

        let mut h = Self {
            handle,
            board_info: None,
        };

        // Immediately get board info to verify connection
        h.board_info = Some(h.get_board_info()?);

        if let Some(ref info) = h.board_info {
            info!(
                "V1743 Board: model={}, serial={}, channels={}, ROC_FW={}, AMC_FW={}, ADC={}bit, SAM_correction={}",
                info.model_name,
                info.serial_number,
                info.channels,
                info.roc_firmware,
                info.amc_firmware,
                info.adc_nbits,
                if info.sam_correction_loaded { "loaded" } else { "not loaded" },
            );
        }

        Ok(h)
    }

    /// Get the raw handle value (for FFI calls in submodules)
    pub fn raw_handle(&self) -> i32 {
        self.handle
    }

    /// Get cached board info
    pub fn board_info(&self) -> Option<&BoardInfo> {
        self.board_info.as_ref()
    }

    /// Query board information from the digitizer
    fn get_board_info(&self) -> Result<BoardInfo, DigitizerError> {
        let mut info: ffi::CAEN_DGTZ_BoardInfo_t = unsafe { std::mem::zeroed() };

        let ret = unsafe { ffi::CAEN_DGTZ_GetInfo(self.handle, &mut info) };
        DigitizerError::check(ret, "GetInfo")?;

        let model_name = unsafe { CStr::from_ptr(info.ModelName.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let roc_firmware = unsafe { CStr::from_ptr(info.ROC_FirmwareRel.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let amc_firmware = unsafe { CStr::from_ptr(info.AMC_FirmwareRel.as_ptr()) }
            .to_string_lossy()
            .into_owned();

        Ok(BoardInfo {
            model_name,
            model: info.Model,
            channels: info.Channels,
            form_factor: info.FormFactor,
            family_code: info.FamilyCode,
            roc_firmware,
            amc_firmware,
            serial_number: info.SerialNumber,
            adc_nbits: info.ADC_NBits,
            sam_correction_loaded: info.SAMCorrectionDataLoaded != 0,
        })
    }

    /// Reset the digitizer to factory defaults
    pub fn reset(&self) -> Result<(), DigitizerError> {
        info!("Resetting V1743 (handle={})", self.handle);
        let ret = unsafe { ffi::CAEN_DGTZ_Reset(self.handle) };
        DigitizerError::check(ret, "Reset")
    }

    /// Read a hardware register
    pub fn read_register(&self, address: u32) -> Result<u32, DigitizerError> {
        let mut data: u32 = 0;
        let ret = unsafe { ffi::CAEN_DGTZ_ReadRegister(self.handle, address, &mut data) };
        DigitizerError::check(ret, &format!("ReadRegister(0x{:08X})", address))?;
        Ok(data)
    }

    /// Write a hardware register
    pub fn write_register(&self, address: u32, data: u32) -> Result<(), DigitizerError> {
        let ret = unsafe { ffi::CAEN_DGTZ_WriteRegister(self.handle, address, data) };
        DigitizerError::check(
            ret,
            &format!("WriteRegister(0x{:08X}, 0x{:08X})", address, data),
        )
    }

    /// Set group enable mask
    pub fn set_group_enable_mask(&self, mask: u32) -> Result<(), DigitizerError> {
        debug!("SetGroupEnableMask: 0b{:08b}", mask);
        let ret = unsafe { ffi::CAEN_DGTZ_SetGroupEnableMask(self.handle, mask) };
        DigitizerError::check(ret, "SetGroupEnableMask")
    }

    /// Set SAM sampling frequency
    pub fn set_sam_sampling_frequency(
        &self,
        freq: SamFrequency,
    ) -> Result<(), DigitizerError> {
        debug!("SetSAMSamplingFrequency: {:?}", freq);
        let ret =
            unsafe { ffi::CAEN_DGTZ_SetSAMSamplingFrequency(self.handle, freq.to_ffi()) };
        DigitizerError::check(ret, "SetSAMSamplingFrequency")
    }

    /// Set SAM correction level
    pub fn set_sam_correction_level(
        &self,
        level: SamCorrectionLevel,
    ) -> Result<(), DigitizerError> {
        debug!("SetSAMCorrectionLevel: {:?}", level);
        let ret =
            unsafe { ffi::CAEN_DGTZ_SetSAMCorrectionLevel(self.handle, level.to_ffi()) };
        DigitizerError::check(ret, "SetSAMCorrectionLevel")
    }

    /// Set post-trigger size for a SAM group
    pub fn set_sam_post_trigger_size(
        &self,
        group: u32,
        value: u32,
    ) -> Result<(), DigitizerError> {
        debug!("SetSAMPostTriggerSize: group={}, value={}", group, value);
        let ret = unsafe {
            ffi::CAEN_DGTZ_SetSAMPostTriggerSize(self.handle, group as i32, value as u8)
        };
        DigitizerError::check(ret, &format!("SetSAMPostTriggerSize(group={})", group))
    }

    /// Set record length (number of samples, 16-1024 in steps of 16)
    pub fn set_record_length(&self, length: u32) -> Result<(), DigitizerError> {
        debug!("SetRecordLength: {}", length);
        let ret = unsafe { ffi::CAEN_DGTZ_SetRecordLength(self.handle, length) };
        DigitizerError::check(ret, "SetRecordLength")
    }

    /// Set channel DC offset (0-65535)
    pub fn set_channel_dc_offset(
        &self,
        channel: u32,
        offset: u32,
    ) -> Result<(), DigitizerError> {
        debug!("SetChannelDCOffset: ch={}, offset={}", channel, offset);
        let ret = unsafe { ffi::CAEN_DGTZ_SetChannelDCOffset(self.handle, channel, offset) };
        DigitizerError::check(ret, &format!("SetChannelDCOffset(ch={})", channel))
    }

    /// Set channel trigger threshold (0-65535)
    pub fn set_channel_trigger_threshold(
        &self,
        channel: u32,
        threshold: u32,
    ) -> Result<(), DigitizerError> {
        debug!(
            "SetChannelTriggerThreshold: ch={}, threshold={}",
            channel, threshold
        );
        let ret = unsafe {
            ffi::CAEN_DGTZ_SetChannelTriggerThreshold(self.handle, channel, threshold)
        };
        DigitizerError::check(ret, &format!("SetChannelTriggerThreshold(ch={})", channel))
    }

    /// Set trigger polarity for a channel
    pub fn set_trigger_polarity(
        &self,
        channel: u32,
        polarity: TriggerPolarity,
    ) -> Result<(), DigitizerError> {
        debug!("SetTriggerPolarity: ch={}, {:?}", channel, polarity);
        let ret = unsafe {
            ffi::CAEN_DGTZ_SetTriggerPolarity(self.handle, channel, polarity.to_ffi())
        };
        DigitizerError::check(ret, &format!("SetTriggerPolarity(ch={})", channel))
    }

    /// Set channel self-trigger mode
    pub fn set_channel_self_trigger(
        &self,
        mode: TriggerMode,
        channel_mask: u32,
    ) -> Result<(), DigitizerError> {
        debug!(
            "SetChannelSelfTrigger: mode={:?}, mask=0b{:016b}",
            mode, channel_mask
        );
        let ret = unsafe {
            ffi::CAEN_DGTZ_SetChannelSelfTrigger(self.handle, mode.to_ffi(), channel_mask)
        };
        DigitizerError::check(ret, "SetChannelSelfTrigger")
    }

    /// Set software trigger mode
    pub fn set_sw_trigger_mode(&self, mode: TriggerMode) -> Result<(), DigitizerError> {
        debug!("SetSWTriggerMode: {:?}", mode);
        let ret = unsafe { ffi::CAEN_DGTZ_SetSWTriggerMode(self.handle, mode.to_ffi()) };
        DigitizerError::check(ret, "SetSWTriggerMode")
    }

    /// Set external trigger input mode
    pub fn set_ext_trigger_input_mode(
        &self,
        mode: TriggerMode,
    ) -> Result<(), DigitizerError> {
        debug!("SetExtTriggerInputMode: {:?}", mode);
        let ret =
            unsafe { ffi::CAEN_DGTZ_SetExtTriggerInputMode(self.handle, mode.to_ffi()) };
        DigitizerError::check(ret, "SetExtTriggerInputMode")
    }

    /// Set I/O level (NIM or TTL)
    pub fn set_io_level(&self, level: IOLevel) -> Result<(), DigitizerError> {
        debug!("SetIOLevel: {:?}", level);
        let ret = unsafe { ffi::CAEN_DGTZ_SetIOLevel(self.handle, level.to_ffi()) };
        DigitizerError::check(ret, "SetIOLevel")
    }

    /// Set acquisition mode (SW controlled)
    pub fn set_acquisition_mode(&self, mode: AcqMode) -> Result<(), DigitizerError> {
        debug!("SetAcquisitionMode: {:?}", mode);
        let ret = unsafe { ffi::CAEN_DGTZ_SetAcquisitionMode(self.handle, mode.to_ffi()) };
        DigitizerError::check(ret, "SetAcquisitionMode")
    }

    /// Set max number of events per block transfer
    pub fn set_max_num_events_blt(&self, num: u32) -> Result<(), DigitizerError> {
        debug!("SetMaxNumEventsBLT: {}", num);
        let ret = unsafe { ffi::CAEN_DGTZ_SetMaxNumEventsBLT(self.handle, num) };
        DigitizerError::check(ret, "SetMaxNumEventsBLT")
    }

    /// Enable SAM pulse generator on a channel
    pub fn enable_sam_pulse_gen(
        &self,
        channel: u32,
        pulse_pattern: u16,
        source: SamPulseSource,
    ) -> Result<(), DigitizerError> {
        debug!(
            "EnableSAMPulseGen: ch={}, pattern=0x{:04X}, source={:?}",
            channel, pulse_pattern, source
        );
        let ret = unsafe {
            ffi::CAEN_DGTZ_EnableSAMPulseGen(
                self.handle,
                channel as i32,
                pulse_pattern,
                source.to_ffi(),
            )
        };
        DigitizerError::check(ret, &format!("EnableSAMPulseGen(ch={})", channel))
    }

    /// Disable SAM pulse generator on a channel
    pub fn disable_sam_pulse_gen(&self, channel: u32) -> Result<(), DigitizerError> {
        debug!("DisableSAMPulseGen: ch={}", channel);
        let ret =
            unsafe { ffi::CAEN_DGTZ_DisableSAMPulseGen(self.handle, channel as i32) };
        DigitizerError::check(ret, &format!("DisableSAMPulseGen(ch={})", channel))
    }

    /// Start acquisition
    pub fn sw_start_acquisition(&self) -> Result<(), DigitizerError> {
        info!("SWStartAcquisition (handle={})", self.handle);
        let ret = unsafe { ffi::CAEN_DGTZ_SWStartAcquisition(self.handle) };
        DigitizerError::check(ret, "SWStartAcquisition")
    }

    /// Stop acquisition
    pub fn sw_stop_acquisition(&self) -> Result<(), DigitizerError> {
        info!("SWStopAcquisition (handle={})", self.handle);
        let ret = unsafe { ffi::CAEN_DGTZ_SWStopAcquisition(self.handle) };
        DigitizerError::check(ret, "SWStopAcquisition")
    }

    /// Send a software trigger
    pub fn send_sw_trigger(&self) -> Result<(), DigitizerError> {
        let ret = unsafe { ffi::CAEN_DGTZ_SendSWtrigger(self.handle) };
        DigitizerError::check(ret, "SendSWtrigger")
    }

    /// Clear data buffers
    pub fn clear_data(&self) -> Result<(), DigitizerError> {
        let ret = unsafe { ffi::CAEN_DGTZ_ClearData(self.handle) };
        DigitizerError::check(ret, "ClearData")
    }

    /// Allocate a readout buffer (must be freed with free_readout_buffer)
    pub fn malloc_readout_buffer(&self) -> Result<ReadoutBuffer, DigitizerError> {
        let mut buffer: *mut std::os::raw::c_char = std::ptr::null_mut();
        let mut size: u32 = 0;

        let ret =
            unsafe { ffi::CAEN_DGTZ_MallocReadoutBuffer(self.handle, &mut buffer, &mut size) };
        DigitizerError::check(ret, "MallocReadoutBuffer")?;

        debug!("ReadoutBuffer allocated: {} bytes", size);

        Ok(ReadoutBuffer {
            buffer,
            allocated_size: size,
        })
    }

    /// Read data from the digitizer into a readout buffer
    pub fn read_data(&self, buf: &mut ReadoutBuffer) -> Result<u32, DigitizerError> {
        let mut data_size: u32 = 0;

        let ret = unsafe {
            ffi::CAEN_DGTZ_ReadData(
                self.handle,
                ffi::CAEN_DGTZ_ReadMode_t::CAEN_DGTZ_SLAVE_TERMINATED_READOUT_MBLT,
                buf.buffer,
                &mut data_size,
            )
        };
        DigitizerError::check(ret, "ReadData")?;

        Ok(data_size)
    }

    /// Get the number of events in a readout buffer
    pub fn get_num_events(
        &self,
        buf: &ReadoutBuffer,
        data_size: u32,
    ) -> Result<u32, DigitizerError> {
        let mut num_events: u32 = 0;

        let ret = unsafe {
            ffi::CAEN_DGTZ_GetNumEvents(
                self.handle,
                buf.buffer,
                data_size,
                &mut num_events,
            )
        };
        DigitizerError::check(ret, "GetNumEvents")?;

        Ok(num_events)
    }

    /// Get event info and pointer for a specific event index
    pub fn get_event_info(
        &self,
        buf: &ReadoutBuffer,
        data_size: u32,
        event_index: u32,
    ) -> Result<(ffi::CAEN_DGTZ_EventInfo_t, *mut std::os::raw::c_char), DigitizerError> {
        let mut event_info: ffi::CAEN_DGTZ_EventInfo_t = unsafe { std::mem::zeroed() };
        let mut event_ptr: *mut std::os::raw::c_char = std::ptr::null_mut();

        let ret = unsafe {
            ffi::CAEN_DGTZ_GetEventInfo(
                self.handle,
                buf.buffer,
                data_size,
                event_index as i32,
                &mut event_info,
                &mut event_ptr,
            )
        };
        DigitizerError::check(ret, &format!("GetEventInfo(idx={})", event_index))?;

        Ok((event_info, event_ptr))
    }

    /// Decode an event from a raw event pointer into X743_EVENT_t
    ///
    /// # Safety
    /// The event must have been previously allocated with allocate_event().
    pub fn decode_event(
        &self,
        event_ptr: *mut std::os::raw::c_char,
        event: &mut EventBuffer,
    ) -> Result<(), DigitizerError> {
        let ret = unsafe {
            ffi::CAEN_DGTZ_DecodeEvent(
                self.handle,
                event_ptr,
                &mut event.event as *mut *mut ffi::CAEN_DGTZ_X743_EVENT_t
                    as *mut *mut std::ffi::c_void,
            )
        };
        DigitizerError::check(ret, "DecodeEvent")
    }

    /// Allocate an event buffer for DecodeEvent
    pub fn allocate_event(&self) -> Result<EventBuffer, DigitizerError> {
        let mut event: *mut ffi::CAEN_DGTZ_X743_EVENT_t = std::ptr::null_mut();

        let ret = unsafe {
            ffi::CAEN_DGTZ_AllocateEvent(
                self.handle,
                &mut event as *mut *mut ffi::CAEN_DGTZ_X743_EVENT_t
                    as *mut *mut std::ffi::c_void,
            )
        };
        DigitizerError::check(ret, "AllocateEvent")?;

        Ok(EventBuffer {
            handle: self.handle,
            event,
        })
    }

    /// Apply Standard mode configuration from DigitizerConfig + X743Config.
    /// Follows WaveDemo ProgramBoard() sequence.
    pub fn apply_config_standard(&self, config: &DigitizerConfig) -> Result<(), DigitizerError> {
        let x743 = config
            .x743
            .as_ref()
            .ok_or_else(|| DigitizerError::new(-1, "Missing x743 config section"))?;

        info!("Applying V1743 configuration...");

        // 1. Reset
        self.reset()?;

        // 2. Group enable mask
        self.set_group_enable_mask(x743.group_enable_mask)?;

        // 3. SAM post-trigger size (per group)
        let num_groups = x743.group_enable_mask.count_ones();
        for g in 0..MAX_GROUPS as u32 {
            if x743.group_enable_mask & (1 << g) != 0 {
                self.set_sam_post_trigger_size(g, x743.post_trigger_size)?;
            }
        }
        debug!("Post-trigger size set for {} groups", num_groups);

        // 4. SAM sampling frequency
        let freq = parse_sam_frequency(&x743.sampling_frequency)?;
        self.set_sam_sampling_frequency(freq)?;

        // 5. Pulse generator
        if x743.pulse_gen_enabled {
            let source = parse_pulse_source(&x743.pulse_source)?;
            for ch in 0..MAX_CHANNELS as u32 {
                self.enable_sam_pulse_gen(ch, x743.pulse_pattern, source)?;
            }
        } else {
            for ch in 0..MAX_CHANNELS as u32 {
                self.disable_sam_pulse_gen(ch)?;
            }
        }

        // 6. Per-channel settings (threshold, dc_offset, polarity, self-trigger)
        self.apply_channel_config(config, x743)?;

        // 7. Trigger source
        match x743.trigger_source.as_str() {
            "software" | "sw" => {
                self.set_sw_trigger_mode(TriggerMode::AcqOnly)?;
                self.set_ext_trigger_input_mode(TriggerMode::Disabled)?;
            }
            "external" | "ext" => {
                self.set_sw_trigger_mode(TriggerMode::Disabled)?;
                self.set_ext_trigger_input_mode(TriggerMode::AcqOnly)?;
            }
            "self" => {
                // Self-trigger is configured per-channel above
                self.set_sw_trigger_mode(TriggerMode::Disabled)?;
                self.set_ext_trigger_input_mode(TriggerMode::Disabled)?;
            }
            "all" => {
                self.set_sw_trigger_mode(TriggerMode::AcqOnly)?;
                self.set_ext_trigger_input_mode(TriggerMode::AcqOnly)?;
            }
            other => {
                warn!("Unknown trigger source '{}', defaulting to external", other);
                self.set_sw_trigger_mode(TriggerMode::Disabled)?;
                self.set_ext_trigger_input_mode(TriggerMode::AcqOnly)?;
            }
        }

        // 8. SAM correction level
        let correction = parse_correction_level(&x743.correction_level)?;
        self.set_sam_correction_level(correction)?;

        // 9. Max events per BLT
        self.set_max_num_events_blt(x743.max_num_events_blt)?;

        // 10. Record length
        self.set_record_length(x743.record_length)?;

        // 11. I/O level
        let io = parse_io_level(&x743.io_level)?;
        self.set_io_level(io)?;

        // 12. Acquisition mode (always SW controlled for delila-rs)
        self.set_acquisition_mode(AcqMode::SWControlled)?;

        info!("V1743 configuration applied successfully");
        Ok(())
    }

    /// Apply per-channel settings from DigitizerConfig defaults + overrides.
    fn apply_channel_config(
        &self,
        config: &DigitizerConfig,
        x743: &X743Config,
    ) -> Result<(), DigitizerError> {
        let defaults = &config.channel_defaults;
        let mut self_trigger_mask: u32 = 0;

        for ch in 0..config.num_channels as u32 {
            let group = ch / CHANNELS_PER_GROUP as u32;
            // Skip if group is not enabled
            if x743.group_enable_mask & (1 << group) == 0 {
                continue;
            }

            let ch_config = config.channel_overrides.get(&(ch as u8));

            // DC Offset: convert percentage (0-100%) to DAC value (0-65535)
            let dc_offset_pct = ch_config
                .and_then(|c| c.dc_offset)
                .or(defaults.dc_offset)
                .unwrap_or(50.0);
            let dc_offset_dac = ((dc_offset_pct / 100.0) * 65535.0) as u32;
            self.set_channel_dc_offset(ch, dc_offset_dac)?;

            // Trigger threshold (raw DAC value, 0-65535)
            if let Some(threshold) = ch_config
                .and_then(|c| c.trigger_threshold)
                .or(defaults.trigger_threshold)
            {
                self.set_channel_trigger_threshold(ch, threshold)?;
            }

            // Trigger polarity
            let polarity_str = ch_config
                .and_then(|c| c.polarity.as_deref())
                .or(defaults.polarity.as_deref());
            if let Some(pol) = polarity_str {
                let polarity = match pol.to_lowercase().as_str() {
                    "positive" | "rising" | "risingedge" => TriggerPolarity::RisingEdge,
                    _ => TriggerPolarity::FallingEdge,
                };
                self.set_trigger_polarity(ch, polarity)?;
            }

            // Self-trigger mask
            let enabled = ch_config
                .and_then(|c| c.enabled.as_deref())
                .or(defaults.enabled.as_deref())
                .map(|s| s.eq_ignore_ascii_case("true"))
                .unwrap_or(true);
            let self_trig = ch_config
                .and_then(|c| c.self_trigger.as_deref())
                .or(defaults.self_trigger.as_deref())
                .map(|s| s.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            if enabled && self_trig {
                self_trigger_mask |= 1 << ch;
            }
        }

        // Apply self-trigger mask
        if self_trigger_mask != 0 {
            self.set_channel_self_trigger(TriggerMode::AcqOnly, self_trigger_mask)?;
        }

        Ok(())
    }

    /// Get device info as JSON (for Detect response)
    pub fn get_device_info_json(&self) -> Result<serde_json::Value, DigitizerError> {
        let info = self.board_info.as_ref().ok_or_else(|| {
            DigitizerError::new(-1, "Board info not available")
        })?;
        Ok(serde_json::json!({
            "model": info.model_name,
            "serial_number": info.serial_number.to_string(),
            "channels": info.channels,
            "adc_bits": info.adc_nbits,
            "roc_firmware": info.roc_firmware,
            "amc_firmware": info.amc_firmware,
            "form_factor": info.form_factor,
            "family_code": info.family_code,
            "sam_correction_loaded": info.sam_correction_loaded,
        }))
    }

    // ---- DPP-CI specific methods ----

    /// Set SAM acquisition mode (Standard waveform vs DPP-CI charge integration)
    pub fn set_sam_acquisition_mode(
        &self,
        mode: SamAcquisitionMode,
    ) -> Result<(), DigitizerError> {
        info!("SetSAMAcquisitionMode: {:?}", mode);
        let ret = unsafe {
            ffi::CAEN_DGTZ_SetSAMAcquisitionMode(self.handle, mode.to_ffi())
        };
        DigitizerError::check(ret, "SetSAMAcquisitionMode")
    }

    /// Set DPP parameters for all channels (DPP-CI mode)
    pub fn set_dpp_parameters(
        &self,
        channel_mask: u32,
        params: &mut ffi::CAEN_DGTZ_DPP_CI_Params_t,
    ) -> Result<(), DigitizerError> {
        info!("SetDPPParameters: channel_mask=0x{:04X}", channel_mask);
        let ret = unsafe {
            ffi::CAEN_DGTZ_SetDPPParameters(
                self.handle,
                channel_mask,
                params as *mut ffi::CAEN_DGTZ_DPP_CI_Params_t as *mut std::ffi::c_void,
            )
        };
        DigitizerError::check(ret, "SetDPPParameters")
    }

    /// Set pulse polarity for DPP firmware (per channel)
    pub fn set_channel_pulse_polarity(
        &self,
        channel: u32,
        polarity: PulsePolarity,
    ) -> Result<(), DigitizerError> {
        debug!("SetChannelPulsePolarity: ch={}, {:?}", channel, polarity);
        let ret = unsafe {
            ffi::CAEN_DGTZ_SetChannelPulsePolarity(self.handle, channel, polarity.to_ffi())
        };
        DigitizerError::check(ret, &format!("SetChannelPulsePolarity(ch={})", channel))
    }

    /// Set DPP acquisition mode (Oscilloscope/List/Mixed) and save parameter
    pub fn set_dpp_acquisition_mode(
        &self,
        mode: DppAcqMode,
        save: DppSaveParam,
    ) -> Result<(), DigitizerError> {
        info!("SetDPPAcquisitionMode: {:?}, {:?}", mode, save);
        let ret = unsafe {
            ffi::CAEN_DGTZ_SetDPPAcquisitionMode(self.handle, mode.to_ffi(), save.to_ffi())
        };
        DigitizerError::check(ret, "SetDPPAcquisitionMode")
    }

    /// Set DPP event aggregation parameters
    pub fn set_dpp_event_aggregation(
        &self,
        threshold: i32,
        maxsize: i32,
    ) -> Result<(), DigitizerError> {
        debug!(
            "SetDPPEventAggregation: threshold={}, maxsize={}",
            threshold, maxsize
        );
        let ret = unsafe {
            ffi::CAEN_DGTZ_SetDPPEventAggregation(self.handle, threshold, maxsize)
        };
        DigitizerError::check(ret, "SetDPPEventAggregation")
    }

    /// Set DPP pre-trigger size (channel=-1 for all channels)
    pub fn set_dpp_pre_trigger_size(
        &self,
        channel: i32,
        samples: u32,
    ) -> Result<(), DigitizerError> {
        debug!(
            "SetDPPPreTriggerSize: ch={}, samples={}",
            channel, samples
        );
        let ret = unsafe {
            ffi::CAEN_DGTZ_SetDPPPreTriggerSize(self.handle, channel, samples)
        };
        DigitizerError::check(ret, "SetDPPPreTriggerSize")
    }

    /// Set channel pair trigger logic (AND/OR with coincidence window)
    pub fn set_channel_pair_trigger_logic(
        &self,
        channel_a: u32,
        channel_b: u32,
        logic: TriggerLogic,
        coincidence_window: u16,
    ) -> Result<(), DigitizerError> {
        debug!(
            "SetChannelPairTriggerLogic: chA={}, chB={}, {:?}, window={}ns",
            channel_a, channel_b, logic, coincidence_window
        );
        let ret = unsafe {
            ffi::CAEN_DGTZ_SetChannelPairTriggerLogic(
                self.handle,
                channel_a,
                channel_b,
                logic.to_ffi(),
                coincidence_window,
            )
        };
        DigitizerError::check(ret, "SetChannelPairTriggerLogic")
    }

    /// Set board-level trigger logic (OR/AND/Majority)
    pub fn set_trigger_logic(
        &self,
        logic: TriggerLogic,
        majority_level: u32,
    ) -> Result<(), DigitizerError> {
        info!(
            "SetTriggerLogic: {:?}, majority_level={}",
            logic, majority_level
        );
        let ret = unsafe {
            ffi::CAEN_DGTZ_SetTriggerLogic(self.handle, logic.to_ffi(), majority_level)
        };
        DigitizerError::check(ret, "SetTriggerLogic")
    }

    /// Allocate DPP event buffers (per-channel matrix)
    pub fn malloc_dpp_events(&self) -> Result<DppEventBuffer, DigitizerError> {
        let mut events: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut size: u32 = 0;

        let ret = unsafe {
            ffi::CAEN_DGTZ_MallocDPPEvents(
                self.handle,
                &mut events as *mut *mut std::ffi::c_void,
                &mut size,
            )
        };
        DigitizerError::check(ret, "MallocDPPEvents")?;

        debug!("DPP event buffer allocated: {} bytes", size);

        Ok(DppEventBuffer {
            handle: self.handle,
            events,
            _allocated_size: size,
        })
    }

    /// Read DPP events from readout buffer. Returns per-channel event counts.
    pub fn get_dpp_events(
        &self,
        buf: &ReadoutBuffer,
        data_size: u32,
        dpp_buf: &mut DppEventBuffer,
    ) -> Result<Vec<u32>, DigitizerError> {
        let mut num_events = vec![0u32; MAX_CHANNELS];

        let ret = unsafe {
            ffi::CAEN_DGTZ_GetDPPEvents(
                self.handle,
                buf.buffer,
                data_size,
                &mut dpp_buf.events as *mut *mut std::ffi::c_void,
                num_events.as_mut_ptr(),
            )
        };
        DigitizerError::check(ret, "GetDPPEvents")?;

        Ok(num_events)
    }

    /// Apply DPP-CI configuration. Follows the programming sequence from
    /// docs/x743_dpp_ci_parameters.md Section 16.
    pub fn apply_config_dpp_ci(&self, config: &DigitizerConfig) -> Result<(), DigitizerError> {
        let x743 = config
            .x743
            .as_ref()
            .ok_or_else(|| DigitizerError::new(-1, "Missing x743 config section"))?;

        info!("Applying V1743 DPP-CI configuration...");

        // 1. Reset
        self.reset()?;

        // 2. Set SAM acquisition mode to DPP-CI
        info!("Setting SAM acquisition mode to DPP-CI...");
        match self.set_sam_acquisition_mode(SamAcquisitionMode::DppCI) {
            Ok(()) => info!("SetSAMAcquisitionMode(DPP_CI) succeeded"),
            Err(ref e) => {
                warn!("SetSAMAcquisitionMode(DPP_CI) failed: {}. This board's FW may not support DPP-CI mode. Falling back without mode switch.", e);
                // Continue without DPP-CI mode switch — some V1743 FW versions may not support it
            }
        }

        // 3. Build DPP_CI_Params_t and call SetDPPParameters
        let mut params: ffi::CAEN_DGTZ_DPP_CI_Params_t = unsafe { std::mem::zeroed() };
        let defaults = &config.channel_defaults;

        // Fill per-group arrays (DPP_CI_Params_t uses MAX_DPP_CI_CHANNEL_SIZE = 8 groups)
        let num_groups = MAX_GROUPS.min(8); // DPP_CI_Params_t arrays are [8]
        for g in 0..num_groups {
            // Use the first channel in the group for overrides
            let ch = (g * CHANNELS_PER_GROUP) as u8;
            let ch_config = config.channel_overrides.get(&ch);

            params.thr[g] = ch_config
                .and_then(|c| c.trigger_threshold)
                .or(defaults.trigger_threshold)
                .or(x743.dpp_ci_threshold)
                .unwrap_or(100) as i32;

            params.gate[g] = x743.dpp_ci_gate.unwrap_or(50) as i32;
            params.pgate[g] = x743.dpp_ci_pgate.unwrap_or(5) as i32;
            params.csens[g] = x743.dpp_ci_csens.unwrap_or(0) as i32;
            params.nsbl[g] = x743.dpp_ci_nsbl.unwrap_or(2) as i32;
            params.tvaw[g] = x743.dpp_ci_tvaw.unwrap_or(50) as i32;

            // Self-trigger: from channel config or global trigger_source
            let self_trig = ch_config
                .and_then(|c| c.self_trigger.as_deref())
                .or(defaults.self_trigger.as_deref())
                .map(|s| s.eq_ignore_ascii_case("true"))
                .unwrap_or(x743.trigger_source == "self");
            params.selft[g] = if self_trig { 1 } else { 0 };

            // trgc is deprecated but must be set to 1 (per-group array)
            params.trgc[g] = 1;
        }
        // Scalar fields
        params.trgho = x743.dpp_ci_trgho.unwrap_or(100) as i32;

        // DPP_CI_Params_t arrays are indexed by group (MAX_DPP_CI_CHANNEL_SIZE=8).
        // channelMask for SetDPPParameters should be the group enable mask.
        let channel_mask = x743.group_enable_mask;
        info!(
            "SetDPPParameters: channel_mask=0x{:04X}, thr[0]={}, gate[0]={}, csens[0]={}, nsbl[0]={}, trgho={}",
            channel_mask, params.thr[0], params.gate[0], params.csens[0], params.nsbl[0], params.trgho
        );
        self.set_dpp_parameters(channel_mask, &mut params)?;

        // 4. Set pulse polarity per channel
        let polarity = match x743
            .pulse_polarity
            .as_deref()
            .unwrap_or("negative")
            .to_lowercase()
            .as_str()
        {
            "positive" | "pos" => PulsePolarity::Positive,
            _ => PulsePolarity::Negative,
        };
        for ch in 0..config.num_channels as u32 {
            self.set_channel_pulse_polarity(ch, polarity)?;
        }

        // 5. DC offset per channel
        for ch in 0..config.num_channels as u32 {
            let ch_config = config.channel_overrides.get(&(ch as u8));
            let dc_offset_pct = ch_config
                .and_then(|c| c.dc_offset)
                .or(defaults.dc_offset)
                .unwrap_or(50.0);
            let dc_offset_dac = ((dc_offset_pct / 100.0) * 65535.0) as u32;
            self.set_channel_dc_offset(ch, dc_offset_dac)?;
        }

        // 6. Group enable mask
        self.set_group_enable_mask(x743.group_enable_mask)?;

        // 7. RecordLength — skip for List mode (no waveforms)

        // 8. DPP pre-trigger (all channels)
        if let Some(pre_trigger) = x743.dpp_ci_pre_trigger {
            self.set_dpp_pre_trigger_size(-1, pre_trigger)?;
        }

        // 9. DPP acquisition mode: List + EnergyAndTime
        self.set_dpp_acquisition_mode(DppAcqMode::List, DppSaveParam::EnergyAndTime)?;

        // 10-11. Event aggregation (auto)
        self.set_dpp_event_aggregation(0, 0)?;

        // 12. SAM sampling frequency
        let freq = parse_sam_frequency(&x743.sampling_frequency)?;
        self.set_sam_sampling_frequency(freq)?;

        // 13. SAM correction level
        let correction = parse_correction_level(&x743.correction_level)?;
        self.set_sam_correction_level(correction)?;

        // 14. SAM post-trigger size per group
        for g in 0..MAX_GROUPS as u32 {
            if x743.group_enable_mask & (1 << g) != 0 {
                self.set_sam_post_trigger_size(g, x743.post_trigger_size)?;
            }
        }

        // 15. Pair trigger logic
        let pair_logic = match x743
            .pair_trigger_logic
            .as_deref()
            .unwrap_or("or")
            .to_lowercase()
            .as_str()
        {
            "and" => TriggerLogic::And,
            _ => TriggerLogic::Or,
        };
        let pair_window = x743.pair_coincidence_window.unwrap_or(15);
        for g in 0..MAX_GROUPS {
            let ch_a = (g * CHANNELS_PER_GROUP) as u32;
            let ch_b = ch_a + 1;
            if x743.group_enable_mask & (1 << g) != 0 {
                self.set_channel_pair_trigger_logic(ch_a, ch_b, pair_logic, pair_window)?;
            }
        }

        // 16. Board trigger logic
        let board_logic = match x743
            .board_trigger_logic
            .as_deref()
            .unwrap_or("or")
            .to_lowercase()
            .as_str()
        {
            "and" => TriggerLogic::And,
            "majority" | "maj" => TriggerLogic::Majority,
            _ => TriggerLogic::Or,
        };
        let majority = x743.board_majority_level.unwrap_or(0);
        self.set_trigger_logic(board_logic, majority)?;

        // 17. Trigger source
        match x743.trigger_source.as_str() {
            "software" | "sw" => {
                self.set_sw_trigger_mode(TriggerMode::AcqOnly)?;
                self.set_ext_trigger_input_mode(TriggerMode::Disabled)?;
            }
            "external" | "ext" => {
                self.set_sw_trigger_mode(TriggerMode::Disabled)?;
                self.set_ext_trigger_input_mode(TriggerMode::AcqOnly)?;
            }
            "self" => {
                self.set_sw_trigger_mode(TriggerMode::Disabled)?;
                self.set_ext_trigger_input_mode(TriggerMode::Disabled)?;
            }
            _ => {
                self.set_sw_trigger_mode(TriggerMode::Disabled)?;
                self.set_ext_trigger_input_mode(TriggerMode::AcqOnly)?;
            }
        }

        // 18. I/O level
        let io = parse_io_level(&x743.io_level)?;
        self.set_io_level(io)?;

        // 19. Acquisition mode (always SW controlled)
        self.set_acquisition_mode(AcqMode::SWControlled)?;

        // 20. Pulse generator
        if x743.pulse_gen_enabled {
            let source = parse_pulse_source(&x743.pulse_source)?;
            for ch in 0..MAX_CHANNELS as u32 {
                self.enable_sam_pulse_gen(ch, x743.pulse_pattern, source)?;
            }
        }

        info!("V1743 DPP-CI configuration applied successfully");
        Ok(())
    }
}

/// Parse sampling frequency string to enum
fn parse_sam_frequency(s: &str) -> Result<SamFrequency, DigitizerError> {
    match s.to_lowercase().as_str() {
        "3.2ghz" | "3200mhz" => Ok(SamFrequency::Ghz3_2),
        "1.6ghz" | "1600mhz" => Ok(SamFrequency::Ghz1_6),
        "800mhz" | "0.8ghz" => Ok(SamFrequency::Mhz800),
        "400mhz" | "0.4ghz" => Ok(SamFrequency::Mhz400),
        _ => Err(DigitizerError::new(
            -1,
            &format!("Unknown sampling frequency: {}", s),
        )),
    }
}

/// Parse correction level string to enum
fn parse_correction_level(s: &str) -> Result<SamCorrectionLevel, DigitizerError> {
    match s.to_lowercase().as_str() {
        "all" | "full" => Ok(SamCorrectionLevel::All),
        "pedestal_only" | "pedestal" => Ok(SamCorrectionLevel::PedestalOnly),
        "inl" => Ok(SamCorrectionLevel::INL),
        "disabled" | "none" => Ok(SamCorrectionLevel::Disabled),
        _ => Err(DigitizerError::new(
            -1,
            &format!("Unknown correction level: {}", s),
        )),
    }
}

/// Parse I/O level string to enum
fn parse_io_level(s: &str) -> Result<IOLevel, DigitizerError> {
    match s.to_lowercase().as_str() {
        "nim" => Ok(IOLevel::NIM),
        "ttl" => Ok(IOLevel::TTL),
        _ => Err(DigitizerError::new(
            -1,
            &format!("Unknown IO level: {}", s),
        )),
    }
}

/// Parse pulse source string to enum
fn parse_pulse_source(s: &str) -> Result<SamPulseSource, DigitizerError> {
    match s.to_lowercase().as_str() {
        "software" | "sw" => Ok(SamPulseSource::Software),
        "continuous" | "cont" => Ok(SamPulseSource::Continuous),
        _ => Err(DigitizerError::new(
            -1,
            &format!("Unknown pulse source: {}", s),
        )),
    }
}

impl Drop for X743Handle {
    fn drop(&mut self) {
        info!("Closing V1743 (handle={})", self.handle);
        let ret = unsafe { ffi::CAEN_DGTZ_CloseDigitizer(self.handle) };
        if !DigitizerError::is_success(ret) {
            warn!("CAEN_DGTZ_CloseDigitizer failed: {:?} (handle={})", ret, self.handle);
        }
    }
}

/// Readout buffer allocated by the CAENDigitizer library
pub struct ReadoutBuffer {
    buffer: *mut std::os::raw::c_char,
    allocated_size: u32,
}

impl ReadoutBuffer {
    pub fn allocated_size(&self) -> u32 {
        self.allocated_size
    }
}

impl Drop for ReadoutBuffer {
    fn drop(&mut self) {
        if !self.buffer.is_null() {
            let ret = unsafe { ffi::CAEN_DGTZ_FreeReadoutBuffer(&mut self.buffer) };
            if !DigitizerError::is_success(ret) {
                warn!("CAEN_DGTZ_FreeReadoutBuffer failed: {:?}", ret);
            }
        }
    }
}

/// Event buffer for decoded X743 events
pub struct EventBuffer {
    handle: i32,
    event: *mut ffi::CAEN_DGTZ_X743_EVENT_t,
}

impl EventBuffer {
    /// Access the decoded event data (read-only)
    ///
    /// # Safety
    /// Only valid after a successful decode_event() call.
    pub fn event(&self) -> &ffi::CAEN_DGTZ_X743_EVENT_t {
        unsafe { &*self.event }
    }
}

impl Drop for EventBuffer {
    fn drop(&mut self) {
        if !self.event.is_null() {
            let ret = unsafe {
                ffi::CAEN_DGTZ_FreeEvent(
                    self.handle,
                    &mut self.event as *mut *mut ffi::CAEN_DGTZ_X743_EVENT_t
                        as *mut *mut std::ffi::c_void,
                )
            };
            if !DigitizerError::is_success(ret) {
                warn!("CAEN_DGTZ_FreeEvent failed: {:?}", ret);
            }
        }
    }
}

// --- Rust-friendly enum wrappers ---
// These map to the bindgen-generated ffi enum types.

/// Connection type for OpenDigitizer
#[derive(Debug, Clone, Copy)]
pub enum ConnectionType {
    USB,
    OpticalLink,
}

impl ConnectionType {
    fn to_ffi(self) -> ffi::CAEN_DGTZ_ConnectionType {
        match self {
            Self::USB => ffi::CAEN_DGTZ_ConnectionType::CAEN_DGTZ_USB,
            Self::OpticalLink => ffi::CAEN_DGTZ_ConnectionType::CAEN_DGTZ_OpticalLink,
        }
    }
}

/// SAM sampling frequency
#[derive(Debug, Clone, Copy)]
pub enum SamFrequency {
    /// 3.2 GHz (highest)
    Ghz3_2,
    /// 1.6 GHz
    Ghz1_6,
    /// 800 MHz
    Mhz800,
    /// 400 MHz (lowest)
    Mhz400,
}

impl SamFrequency {
    fn to_ffi(self) -> ffi::CAEN_DGTZ_SAMFrequency_t {
        match self {
            Self::Ghz3_2 => ffi::CAEN_DGTZ_SAMFrequency_t::CAEN_DGTZ_SAM_3_2GHz,
            Self::Ghz1_6 => ffi::CAEN_DGTZ_SAMFrequency_t::CAEN_DGTZ_SAM_1_6GHz,
            Self::Mhz800 => ffi::CAEN_DGTZ_SAMFrequency_t::CAEN_DGTZ_SAM_800MHz,
            Self::Mhz400 => ffi::CAEN_DGTZ_SAMFrequency_t::CAEN_DGTZ_SAM_400MHz,
        }
    }
}

/// SAM correction level
#[derive(Debug, Clone, Copy)]
pub enum SamCorrectionLevel {
    Disabled,
    PedestalOnly,
    INL,
    All,
}

impl SamCorrectionLevel {
    fn to_ffi(self) -> ffi::CAEN_DGTZ_SAM_CORRECTION_LEVEL_t {
        match self {
            Self::Disabled => {
                ffi::CAEN_DGTZ_SAM_CORRECTION_LEVEL_t::CAEN_DGTZ_SAM_CORRECTION_DISABLED
            }
            Self::PedestalOnly => {
                ffi::CAEN_DGTZ_SAM_CORRECTION_LEVEL_t::CAEN_DGTZ_SAM_CORRECTION_PEDESTAL_ONLY
            }
            Self::INL => {
                ffi::CAEN_DGTZ_SAM_CORRECTION_LEVEL_t::CAEN_DGTZ_SAM_CORRECTION_INL
            }
            Self::All => {
                ffi::CAEN_DGTZ_SAM_CORRECTION_LEVEL_t::CAEN_DGTZ_SAM_CORRECTION_ALL
            }
        }
    }
}

/// Trigger mode
#[derive(Debug, Clone, Copy)]
pub enum TriggerMode {
    Disabled,
    AcqOnly,
    AcqAndExtOut,
    ExtOutOnly,
}

impl TriggerMode {
    fn to_ffi(self) -> ffi::CAEN_DGTZ_TriggerMode_t {
        match self {
            Self::Disabled => ffi::CAEN_DGTZ_TriggerMode_t::CAEN_DGTZ_TRGMODE_DISABLED,
            Self::AcqOnly => ffi::CAEN_DGTZ_TriggerMode_t::CAEN_DGTZ_TRGMODE_ACQ_ONLY,
            Self::AcqAndExtOut => {
                ffi::CAEN_DGTZ_TriggerMode_t::CAEN_DGTZ_TRGMODE_ACQ_AND_EXTOUT
            }
            Self::ExtOutOnly => ffi::CAEN_DGTZ_TriggerMode_t::CAEN_DGTZ_TRGMODE_EXTOUT_ONLY,
        }
    }
}

/// Trigger polarity
#[derive(Debug, Clone, Copy)]
pub enum TriggerPolarity {
    RisingEdge,
    FallingEdge,
}

impl TriggerPolarity {
    fn to_ffi(self) -> ffi::CAEN_DGTZ_TriggerPolarity_t {
        match self {
            Self::RisingEdge => {
                ffi::CAEN_DGTZ_TriggerPolarity_t::CAEN_DGTZ_TriggerOnRisingEdge
            }
            Self::FallingEdge => {
                ffi::CAEN_DGTZ_TriggerPolarity_t::CAEN_DGTZ_TriggerOnFallingEdge
            }
        }
    }
}

/// I/O level
#[derive(Debug, Clone, Copy)]
pub enum IOLevel {
    NIM,
    TTL,
}

impl IOLevel {
    fn to_ffi(self) -> ffi::CAEN_DGTZ_IOLevel_t {
        match self {
            Self::NIM => ffi::CAEN_DGTZ_IOLevel_t::CAEN_DGTZ_IOLevel_NIM,
            Self::TTL => ffi::CAEN_DGTZ_IOLevel_t::CAEN_DGTZ_IOLevel_TTL,
        }
    }
}

/// Acquisition mode
#[derive(Debug, Clone, Copy)]
pub enum AcqMode {
    SWControlled,
    SINControlled,
    FirstTrigControlled,
}

impl AcqMode {
    fn to_ffi(self) -> ffi::CAEN_DGTZ_AcqMode_t {
        match self {
            Self::SWControlled => ffi::CAEN_DGTZ_AcqMode_t::CAEN_DGTZ_SW_CONTROLLED,
            Self::SINControlled => ffi::CAEN_DGTZ_AcqMode_t::CAEN_DGTZ_S_IN_CONTROLLED,
            Self::FirstTrigControlled => {
                ffi::CAEN_DGTZ_AcqMode_t::CAEN_DGTZ_FIRST_TRG_CONTROLLED
            }
        }
    }
}

/// SAM pulse generator source
#[derive(Debug, Clone, Copy)]
pub enum SamPulseSource {
    Software,
    Continuous,
}

impl SamPulseSource {
    fn to_ffi(self) -> ffi::CAEN_DGTZ_SAMPulseSourceType_t {
        match self {
            Self::Software => {
                ffi::CAEN_DGTZ_SAMPulseSourceType_t::CAEN_DGTZ_SAMPulseSoftware
            }
            Self::Continuous => ffi::CAEN_DGTZ_SAMPulseSourceType_t::CAEN_DGTZ_SAMPulseCont,
        }
    }
}

// --- DPP-CI specific enums ---

/// SAM acquisition mode (Standard waveform vs DPP-CI charge integration)
#[derive(Debug, Clone, Copy)]
pub enum SamAcquisitionMode {
    Standard,
    DppCI,
}

impl SamAcquisitionMode {
    fn to_ffi(self) -> ffi::CAEN_DGTZ_AcquisitionMode_t {
        match self {
            Self::Standard => ffi::CAEN_DGTZ_AcquisitionMode_t::CAEN_DGTZ_AcquisitionMode_STANDARD,
            Self::DppCI => ffi::CAEN_DGTZ_AcquisitionMode_t::CAEN_DGTZ_AcquisitionMode_DPP_CI,
        }
    }
}

/// Pulse polarity for DPP firmware
#[derive(Debug, Clone, Copy)]
pub enum PulsePolarity {
    Positive,
    Negative,
}

impl PulsePolarity {
    fn to_ffi(self) -> ffi::CAEN_DGTZ_PulsePolarity_t {
        match self {
            Self::Positive => ffi::CAEN_DGTZ_PulsePolarity_t_CAEN_DGTZ_PulsePolarityPositive,
            Self::Negative => ffi::CAEN_DGTZ_PulsePolarity_t_CAEN_DGTZ_PulsePolarityNegative,
        }
    }
}

/// DPP acquisition mode (what data to acquire)
#[derive(Debug, Clone, Copy)]
pub enum DppAcqMode {
    Oscilloscope,
    List,
    Mixed,
}

impl DppAcqMode {
    fn to_ffi(self) -> ffi::CAEN_DGTZ_DPP_AcqMode_t {
        match self {
            Self::Oscilloscope => ffi::CAEN_DGTZ_DPP_AcqMode_t_CAEN_DGTZ_DPP_ACQ_MODE_Oscilloscope,
            Self::List => ffi::CAEN_DGTZ_DPP_AcqMode_t_CAEN_DGTZ_DPP_ACQ_MODE_List,
            Self::Mixed => ffi::CAEN_DGTZ_DPP_AcqMode_t_CAEN_DGTZ_DPP_ACQ_MODE_Mixed,
        }
    }
}

/// DPP save parameter (what to save per event)
#[derive(Debug, Clone, Copy)]
pub enum DppSaveParam {
    EnergyOnly,
    TimeOnly,
    EnergyAndTime,
    None,
}

impl DppSaveParam {
    fn to_ffi(self) -> ffi::CAEN_DGTZ_DPP_SaveParam_t {
        match self {
            Self::EnergyOnly => ffi::CAEN_DGTZ_DPP_SaveParam_t_CAEN_DGTZ_DPP_SAVE_PARAM_EnergyOnly,
            Self::TimeOnly => ffi::CAEN_DGTZ_DPP_SaveParam_t_CAEN_DGTZ_DPP_SAVE_PARAM_TimeOnly,
            Self::EnergyAndTime => ffi::CAEN_DGTZ_DPP_SaveParam_t_CAEN_DGTZ_DPP_SAVE_PARAM_EnergyAndTime,
            Self::None => ffi::CAEN_DGTZ_DPP_SaveParam_t_CAEN_DGTZ_DPP_SAVE_PARAM_None,
        }
    }
}

/// Trigger logic for channel pairs and board level
#[derive(Debug, Clone, Copy)]
pub enum TriggerLogic {
    Or,
    And,
    Majority,
}

impl TriggerLogic {
    fn to_ffi(self) -> ffi::CAEN_DGTZ_TrigerLogic_t {
        match self {
            Self::Or => ffi::CAEN_DGTZ_TrigerLogic_t::CAEN_DGTZ_LOGIC_OR,
            Self::And => ffi::CAEN_DGTZ_TrigerLogic_t::CAEN_DGTZ_LOGIC_AND,
            Self::Majority => ffi::CAEN_DGTZ_TrigerLogic_t::CAEN_DGTZ_LOGIC_MAJORITY,
        }
    }
}

// --- DPP Event Buffer (RAII) ---

/// DPP event buffer allocated by MallocDPPEvents.
/// Owns the per-channel event matrix and frees it on drop.
pub struct DppEventBuffer {
    handle: i32,
    events: *mut std::ffi::c_void,
    _allocated_size: u32,
}

impl DppEventBuffer {
    /// Access decoded DPP-CI events for a specific channel.
    ///
    /// # Safety
    /// Only valid after a successful `get_dpp_events()` call.
    /// `count` must not exceed the value returned by `get_dpp_events()` for this channel.
    pub unsafe fn get_channel_events(
        &self,
        channel: usize,
        count: u32,
    ) -> &[ffi::CAEN_DGTZ_DPP_CI_Event_t] {
        let events_ptr = self.events as *mut *mut ffi::CAEN_DGTZ_DPP_CI_Event_t;
        let ch_ptr = *events_ptr.add(channel);
        std::slice::from_raw_parts(ch_ptr, count as usize)
    }

    /// Get the raw events pointer (for FFI calls)
    pub fn as_mut_ptr(&mut self) -> *mut std::ffi::c_void {
        self.events
    }
}

impl Drop for DppEventBuffer {
    fn drop(&mut self) {
        if !self.events.is_null() {
            let ret = unsafe {
                ffi::CAEN_DGTZ_FreeDPPEvents(
                    self.handle,
                    &mut self.events as *mut *mut std::ffi::c_void,
                )
            };
            if !DigitizerError::is_success(ret) {
                warn!("CAEN_DGTZ_FreeDPPEvents failed: {:?}", ret);
            }
        }
    }
}
