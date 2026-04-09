//! Safe RAII wrapper for CAENDigitizer handle
//!
//! X743Handle owns the digitizer connection and ensures proper cleanup on drop.
//! All CAENDigitizer API calls go through this handle.

use super::error::DigitizerError;
use super::ffi;
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
