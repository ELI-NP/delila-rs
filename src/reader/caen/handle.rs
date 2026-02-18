//! Safe wrapper for CAEN device handle
//!
//! Provides RAII-based handle management (like C++ std::unique_ptr with custom deleter).

use super::error::CaenError;
use super::ffi;
use super::validation::{self, ApplyConfigResult, ParamApplyResult, ParamApplyStatus};
use std::collections::HashMap;
use std::ffi::CString;

// C wrapper for variadic CAEN_FELib_ReadData function
// Rust cannot directly call C variadic functions on all platforms (especially macOS ARM64).
// We use a C wrapper function compiled via cc crate.
extern "C" {
    /// C wrapper for CAEN_FELib_ReadData with RAW format
    /// Defined in wrapper.c, compiled by build.rs
    fn caen_read_data_raw(
        handle: u64,
        timeout: std::os::raw::c_int,
        data: *mut u8,
        size: *mut usize,
        n_events: *mut u32,
    ) -> std::os::raw::c_int;

    /// C wrapper for CAEN_FELib_ReadData with OpenDPP format (no waveform)
    /// Defined in wrapper.c, compiled by build.rs
    fn caen_read_data_opendpp(
        handle: u64,
        timeout: std::os::raw::c_int,
        channel: *mut u8,
        timestamp: *mut u64,
        fine_timestamp: *mut u16,
        energy: *mut u16,
        flags_b: *mut u16,
        flags_a: *mut u16,
        psd: *mut u16,
        user_info: *mut u64,
        user_info_size: *mut usize,
        event_size: *mut usize,
    ) -> std::os::raw::c_int;

    /// C wrapper for CAEN_FELib_ReadData with OpenDPP format (with waveform)
    /// Defined in wrapper.c, compiled by build.rs
    fn caen_read_data_opendpp_waveform(
        handle: u64,
        timeout: std::os::raw::c_int,
        channel: *mut u8,
        timestamp: *mut u64,
        fine_timestamp: *mut u16,
        energy: *mut u16,
        flags_b: *mut u16,
        flags_a: *mut u16,
        psd: *mut u16,
        user_info: *mut u64,
        user_info_size: *mut usize,
        waveform: *mut u16,
        waveform_size: *mut usize,
        event_size: *mut usize,
    ) -> std::os::raw::c_int;
}

/// Safe wrapper for CAEN device handle
///
/// Automatically closes the device when dropped (RAII pattern).
/// Equivalent to C++ unique_ptr<void, CaenDeleter>.
pub struct CaenHandle {
    handle: u64,
}

/// Handle for data endpoint (for ReadData operations)
///
/// This is a sub-handle obtained from the main device handle.
/// It does NOT implement Drop - it's just a reference to an internal resource.
pub struct EndpointHandle {
    handle: u64,
}

/// Raw data read result
#[derive(Debug)]
pub struct RawData {
    /// Raw binary data from digitizer
    pub data: Vec<u8>,
    /// Actual size of valid data in bytes
    pub size: usize,
    /// Number of events in this data block
    pub n_events: u32,
}

/// OpenDPP decoded event data (single event)
#[derive(Debug, Clone)]
pub struct OpenDppEvent {
    /// Channel number
    pub channel: u8,
    /// Timestamp (in clock ticks, 1 LSB = 8 ns for VX2730)
    pub timestamp: u64,
    /// Fine timestamp (sub-clock resolution)
    pub fine_timestamp: u16,
    /// Energy value
    pub energy: u16,
    /// Flags B (12 bits)
    pub flags_b: u16,
    /// Flags A (8 bits)
    pub flags_a: u16,
    /// PSD value
    pub psd: u16,
    /// User info words
    pub user_info: Vec<u64>,
    /// Waveform samples (optional, only if configured with include_waveform=true)
    pub waveform: Option<Vec<u16>>,
    /// Total event size in bytes
    pub event_size: usize,
}

/// Device information retrieved from digitizer
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceInfo {
    /// Model name (e.g., "VX2730")
    pub model: String,
    /// Serial number
    pub serial_number: String,
    /// Firmware type (e.g., "DPP_PSD")
    pub firmware_type: String,
    /// Number of channels
    pub num_channels: u32,
    /// ADC resolution in bits
    pub adc_bits: u32,
    /// Sampling rate in samples/sec
    pub sampling_rate_sps: u64,
}

/// Parameter metadata from DevTree
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParamInfo {
    /// Parameter name
    pub name: String,
    /// Data type (e.g., "NUMBER", "STRING", "BOOL")
    pub datatype: String,
    /// Access mode (e.g., "READ_WRITE", "READ_ONLY")
    pub access_mode: String,
    /// Whether parameter can be changed during acquisition
    pub setinrun: bool,
    /// Minimum value (for numeric types)
    pub min_value: Option<String>,
    /// Maximum value (for numeric types)
    pub max_value: Option<String>,
    /// Allowed values (for enum types)
    pub allowed_values: Vec<String>,
    /// Unit of measurement
    pub unit: Option<String>,
    /// Step increment (e.g., "8", "2", "0.1")
    pub increment: Option<String>,
    /// Default value from DevTree
    pub default_value: Option<String>,
    /// Unit exponent (e.g., -9 for nanoseconds, 0 for base unit)
    pub expuom: Option<i32>,
}

impl CaenHandle {
    /// Open a connection to a CAEN device
    ///
    /// # Arguments
    /// * `url` - Device URL (e.g., "dig2://172.18.4.56")
    ///
    /// # Example
    /// ```no_run
    /// use delila_rs::reader::caen::CaenHandle;
    /// let handle = CaenHandle::open("dig2://172.18.4.56").unwrap();
    /// ```
    pub fn open(url: &str) -> Result<Self, CaenError> {
        let c_url = CString::new(url).map_err(|_| CaenError {
            code: -2,
            name: "InvalidParam".to_string(),
            description: "URL contains null byte".to_string(),
        })?;

        let mut handle: u64 = 0;
        let ret = unsafe { ffi::CAEN_FELib_Open(c_url.as_ptr(), &mut handle) };

        CaenError::check(ret)?;
        Ok(Self { handle })
    }

    /// Get the raw handle value (for advanced use)
    pub fn raw(&self) -> u64 {
        self.handle
    }

    /// Check if the handle is connected (non-zero handle value)
    ///
    /// Note: This only checks if we have a valid handle. It does not
    /// verify the connection is still alive. Use get_device_info()
    /// for active connection verification.
    pub fn is_connected(&self) -> bool {
        self.handle != 0
    }

    /// Get device information
    ///
    /// Retrieves model name, serial number, firmware type, and hardware specs.
    ///
    /// # Example
    /// ```no_run
    /// use delila_rs::reader::caen::CaenHandle;
    /// let handle = CaenHandle::open("dig2://172.18.4.56").unwrap();
    /// let info = handle.get_device_info().unwrap();
    /// println!("Model: {}, FW: {}", info.model, info.firmware_type);
    /// ```
    pub fn get_device_info(&self) -> Result<DeviceInfo, CaenError> {
        let model = self.get_value("/par/ModelName")?;
        let serial_number = self.get_value("/par/SerialNum")?;
        let firmware_type = self.get_value("/par/FwType")?;
        let num_channels: u32 = self.get_value("/par/NumCh")?.parse().unwrap_or(0);
        let adc_bits: u32 = self.get_value("/par/ADC_Nbit")?.parse().unwrap_or(0);
        let sampling_rate_sps: u64 = self.get_value("/par/ADC_SamplRate")?.parse().unwrap_or(0);

        Ok(DeviceInfo {
            model,
            serial_number,
            firmware_type,
            num_channels,
            adc_bits,
            sampling_rate_sps,
        })
    }

    /// Get parameter metadata from DevTree
    ///
    /// Parses the device tree to extract parameter attributes like
    /// datatype, access mode, setinrun flag, min/max values, etc.
    ///
    /// # Arguments
    /// * `path` - Parameter path (e.g., "/ch/0/par/DCOffset" or "DCOffset")
    ///
    /// # Note
    /// This method parses the full DevTree JSON which can be expensive.
    /// Consider caching the result if calling frequently.
    pub fn get_param_info(&self, path: &str) -> Result<ParamInfo, CaenError> {
        let tree_json = self.get_device_tree()?;
        let tree: serde_json::Value = serde_json::from_str(&tree_json).map_err(|e| CaenError {
            code: -1,
            name: "JsonParseError".to_string(),
            description: format!("Failed to parse DevTree JSON: {}", e),
        })?;

        // Extract parameter name from path (last component after /par/)
        let param_name = path.rsplit('/').find(|s| !s.is_empty()).unwrap_or(path);

        // Search for parameter in DevTree
        // DevTree structure: { "par": { "ParamName": { ... } }, "ch": { ... } }
        let param_node = Self::find_param_in_tree(&tree, param_name).ok_or_else(|| CaenError {
            code: -1,
            name: "ParamNotFound".to_string(),
            description: format!("Parameter '{}' not found in DevTree", param_name),
        })?;

        Self::extract_param_info(param_name, param_node)
    }

    /// Find a parameter node in the DevTree by name (recursive search)
    fn find_param_in_tree<'a>(
        node: &'a serde_json::Value,
        param_name: &str,
    ) -> Option<&'a serde_json::Value> {
        if let Some(obj) = node.as_object() {
            // Check if this object has the parameter directly
            if let Some(param) = obj.get(param_name) {
                // Verify it's a parameter (has datatype or value)
                if param.get("datatype").is_some() || param.get("value").is_some() {
                    return Some(param);
                }
            }

            // Check in "par" subfolder
            if let Some(par_folder) = obj.get("par") {
                if let Some(param) = Self::find_param_in_tree(par_folder, param_name) {
                    return Some(param);
                }
            }

            // Recursively search in child objects
            for (_key, value) in obj {
                if let Some(param) = Self::find_param_in_tree(value, param_name) {
                    return Some(param);
                }
            }
        }
        None
    }

    /// Extract ParamInfo from a DevTree parameter node
    fn extract_param_info(name: &str, node: &serde_json::Value) -> Result<ParamInfo, CaenError> {
        let get_attr_value = |attr: &str| -> Option<String> {
            node.get(attr)
                .and_then(|v| v.get("value"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        };

        let datatype = get_attr_value("datatype").unwrap_or_else(|| "UNKNOWN".to_string());
        let access_mode = get_attr_value("accessmode").unwrap_or_else(|| "READ_WRITE".to_string());
        let setinrun = get_attr_value("setinrun")
            .map(|s| s.to_lowercase() == "true")
            .unwrap_or(false);
        let min_value = get_attr_value("minvalue");
        let max_value = get_attr_value("maxvalue");
        let unit = get_attr_value("uom").filter(|s| !s.is_empty());
        let increment = get_attr_value("increment");
        let default_value = get_attr_value("defaultvalue");
        let expuom = get_attr_value("expuom").and_then(|s| s.parse::<i32>().ok());

        // Extract allowed values for enum types
        let mut allowed_values = Vec::new();
        if let Some(av) = node.get("allowedvalues") {
            if let Some(obj) = av.as_object() {
                for (key, val) in obj {
                    // Skip non-numeric keys (like "handle", "value")
                    if key.parse::<u32>().is_ok() {
                        if let Some(v) = val.get("value").and_then(|v| v.as_str()) {
                            allowed_values.push(v.to_string());
                        }
                    }
                }
            }
        }

        Ok(ParamInfo {
            name: name.to_string(),
            datatype,
            access_mode,
            setinrun,
            min_value,
            max_value,
            allowed_values,
            unit,
            increment,
            default_value,
            expuom,
        })
    }

    /// Get device tree as JSON string
    pub fn get_device_tree(&self) -> Result<String, CaenError> {
        // First call to get required buffer size
        let size = unsafe { ffi::CAEN_FELib_GetDeviceTree(self.handle, std::ptr::null_mut(), 0) };

        if size <= 0 {
            return Err(CaenError {
                code: size,
                name: "GetDeviceTreeError".to_string(),
                description: "Failed to get device tree size".to_string(),
            });
        }

        // Allocate buffer with extra space and get the actual data
        // size is returned as number of characters needed (including null terminator)
        let buffer_size = (size as usize) + 1024; // Extra padding for safety
        let mut buffer = vec![0i8; buffer_size];
        let ret =
            unsafe { ffi::CAEN_FELib_GetDeviceTree(self.handle, buffer.as_mut_ptr(), buffer_size) };

        if ret < 0 {
            return Err(CaenError::from_code(ret).unwrap_or(CaenError {
                code: ret,
                name: "Unknown".to_string(),
                description: "Failed to get device tree".to_string(),
            }));
        }

        // Find the actual string length (look for null terminator)
        let actual_len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());

        // Convert to Rust string using the actual length
        let bytes: Vec<u8> = buffer[..actual_len].iter().map(|&c| c as u8).collect();
        String::from_utf8(bytes).map_err(|_| CaenError {
            code: -1,
            name: "Utf8Error".to_string(),
            description: "Device tree contains invalid UTF-8".to_string(),
        })
    }

    /// Get a parameter value
    ///
    /// # Arguments
    /// * `path` - Parameter path (e.g., "/par/ModelName")
    pub fn get_value(&self, path: &str) -> Result<String, CaenError> {
        let c_path = CString::new(path).map_err(|_| CaenError {
            code: -2,
            name: "InvalidParam".to_string(),
            description: "Path contains null byte".to_string(),
        })?;

        let mut buffer = [0i8; 256];
        let ret =
            unsafe { ffi::CAEN_FELib_GetValue(self.handle, c_path.as_ptr(), buffer.as_mut_ptr()) };

        CaenError::check(ret)?;

        let c_str = unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) };
        Ok(c_str.to_string_lossy().into_owned())
    }

    /// Set a parameter value
    ///
    /// # Arguments
    /// * `path` - Parameter path (e.g., "/ch/0/par/ChEnable")
    /// * `value` - Value to set (e.g., "True")
    pub fn set_value(&self, path: &str, value: &str) -> Result<(), CaenError> {
        let c_path = CString::new(path).map_err(|_| CaenError {
            code: -2,
            name: "InvalidParam".to_string(),
            description: "Path contains null byte".to_string(),
        })?;

        let c_value = CString::new(value).map_err(|_| CaenError {
            code: -2,
            name: "InvalidParam".to_string(),
            description: "Value contains null byte".to_string(),
        })?;

        let ret =
            unsafe { ffi::CAEN_FELib_SetValue(self.handle, c_path.as_ptr(), c_value.as_ptr()) };

        CaenError::check(ret)
    }

    /// Send a command to the device
    ///
    /// # Arguments
    /// * `path` - Command path (e.g., "/cmd/Reset")
    pub fn send_command(&self, path: &str) -> Result<(), CaenError> {
        let c_path = CString::new(path).map_err(|_| CaenError {
            code: -2,
            name: "InvalidParam".to_string(),
            description: "Path contains null byte".to_string(),
        })?;

        let ret = unsafe { ffi::CAEN_FELib_SendCommand(self.handle, c_path.as_ptr()) };

        CaenError::check(ret)
    }

    /// Read a user register value
    ///
    /// # Arguments
    /// * `address` - Register address (e.g., 0xEF24)
    ///
    /// # Note
    /// This is a low-level operation. Use with caution.
    pub fn get_user_register(&self, address: u32) -> Result<u32, CaenError> {
        let mut value: u32 = 0;
        let ret = unsafe { ffi::CAEN_FELib_GetUserRegister(self.handle, address, &mut value) };
        CaenError::check(ret)?;
        Ok(value)
    }

    /// Write a user register value
    ///
    /// # Arguments
    /// * `address` - Register address (e.g., 0xEF24 for software reset)
    /// * `value` - Value to write
    ///
    /// # Note
    /// This is a low-level operation. Use with caution.
    /// Desktop digitizers only: writing any value to 0xEF24 triggers a software reset.
    pub fn set_user_register(&self, address: u32, value: u32) -> Result<(), CaenError> {
        let ret = unsafe { ffi::CAEN_FELib_SetUserRegister(self.handle, address, value) };
        CaenError::check(ret)
    }

    /// Get a sub-handle for a given path
    ///
    /// # Arguments
    /// * `path` - Path to the resource (e.g., "/endpoint/RAW")
    pub fn get_handle(&self, path: &str) -> Result<u64, CaenError> {
        let c_path = CString::new(path).map_err(|_| CaenError {
            code: -2,
            name: "InvalidParam".to_string(),
            description: "Path contains null byte".to_string(),
        })?;

        let mut sub_handle: u64 = 0;
        let ret =
            unsafe { ffi::CAEN_FELib_GetHandle(self.handle, c_path.as_ptr(), &mut sub_handle) };

        CaenError::check(ret)?;
        Ok(sub_handle)
    }

    /// Get parent handle of a given handle
    ///
    /// # Arguments
    /// * `handle` - The handle to get parent of
    pub fn get_parent_handle(&self, handle: u64) -> Result<u64, CaenError> {
        let mut parent_handle: u64 = 0;
        let ret = unsafe {
            ffi::CAEN_FELib_GetParentHandle(handle, std::ptr::null(), &mut parent_handle)
        };

        CaenError::check(ret)?;
        Ok(parent_handle)
    }

    /// Set value using a sub-handle
    ///
    /// # Arguments
    /// * `handle` - Sub-handle to use
    /// * `path` - Parameter path
    /// * `value` - Value to set
    pub fn set_value_with_handle(
        &self,
        handle: u64,
        path: &str,
        value: &str,
    ) -> Result<(), CaenError> {
        let c_path = CString::new(path).map_err(|_| CaenError {
            code: -2,
            name: "InvalidParam".to_string(),
            description: "Path contains null byte".to_string(),
        })?;

        let c_value = CString::new(value).map_err(|_| CaenError {
            code: -2,
            name: "InvalidParam".to_string(),
            description: "Value contains null byte".to_string(),
        })?;

        let ret = unsafe { ffi::CAEN_FELib_SetValue(handle, c_path.as_ptr(), c_value.as_ptr()) };

        CaenError::check(ret)
    }

    /// Configure endpoint for RAW data reading
    ///
    /// This sets up the RAW endpoint and returns an EndpointHandle for data reading.
    /// Follows the C++ pattern from Digitizer1/Digitizer2::EndpointConfigure()
    ///
    /// # Arguments
    /// * `include_n_events` - If true, include N_EVENTS in the read format (DIG2).
    ///   If false, use DATA + SIZE only (DIG1).
    pub fn configure_endpoint(&self, include_n_events: bool) -> Result<EndpointHandle, CaenError> {
        // Get endpoint handle
        let ep_handle = self.get_handle("/endpoint/RAW")?;

        // Get parent (endpoint folder) handle
        let ep_folder_handle = self.get_parent_handle(ep_handle)?;

        // Set active endpoint to RAW
        self.set_value_with_handle(ep_folder_handle, "/par/activeendpoint", "RAW")?;

        // Get fresh handle for read operations
        let read_data_handle = self.get_handle("/endpoint/RAW")?;

        // Set data format based on digitizer generation
        let format_json = if include_n_events {
            // DIG2 (VX2730 etc.): DATA, SIZE, N_EVENTS
            r#"[
            {"name": "DATA", "type": "U8", "dim": 1},
            {"name": "SIZE", "type": "SIZE_T", "dim": 0},
            {"name": "N_EVENTS", "type": "U32", "dim": 0}
        ]"#
        } else {
            // DIG1 (DT5730 etc.): DATA, SIZE only
            r#"[
            {"name": "DATA", "type": "U8", "dim": 1},
            {"name": "SIZE", "type": "SIZE_T", "dim": 0}
        ]"#
        };

        let c_format = CString::new(format_json).map_err(|_| CaenError {
            code: -2,
            name: "InvalidParam".to_string(),
            description: "Format JSON contains null byte".to_string(),
        })?;

        let ret = unsafe { ffi::CAEN_FELib_SetReadDataFormat(read_data_handle, c_format.as_ptr()) };
        CaenError::check(ret)?;

        Ok(EndpointHandle {
            handle: read_data_handle,
        })
    }

    /// Configure endpoint for OpenDPP decoded data reading
    ///
    /// This sets up the OpenDPP endpoint and returns an EndpointHandle for data reading.
    /// The OpenDPP endpoint provides decoded event data (channel, timestamp, energy, etc.)
    /// instead of raw binary data.
    ///
    /// # Arguments
    /// * `include_waveform` - If true, include waveform data in the output
    pub fn configure_opendpp_endpoint(
        &self,
        include_waveform: bool,
    ) -> Result<EndpointHandle, CaenError> {
        // Get endpoint handle
        let ep_handle = self.get_handle("/endpoint/opendpp")?;

        // Get parent (endpoint folder) handle
        let ep_folder_handle = self.get_parent_handle(ep_handle)?;

        // Set active endpoint to OpenDPP
        self.set_value_with_handle(ep_folder_handle, "/par/activeendpoint", "OpenDPP")?;

        // Get fresh handle for read operations
        let read_data_handle = self.get_handle("/endpoint/opendpp")?;

        // Set data format for OpenDPP
        let format_json = if include_waveform {
            r#"[
            {"name": "CHANNEL", "type": "U8"},
            {"name": "TIMESTAMP", "type": "U64"},
            {"name": "FINE_TIMESTAMP", "type": "U16"},
            {"name": "ENERGY", "type": "U16"},
            {"name": "FLAGS_B", "type": "U16"},
            {"name": "FLAGS_A", "type": "U16"},
            {"name": "PSD", "type": "U16"},
            {"name": "USER_INFO", "type": "U64", "dim": 1},
            {"name": "USER_INFO_SIZE", "type": "SIZE_T"},
            {"name": "WAVEFORM", "type": "U16", "dim": 1},
            {"name": "WAVEFORM_SIZE", "type": "SIZE_T"},
            {"name": "EVENT_SIZE", "type": "SIZE_T"}
        ]"#
        } else {
            r#"[
            {"name": "CHANNEL", "type": "U8"},
            {"name": "TIMESTAMP", "type": "U64"},
            {"name": "FINE_TIMESTAMP", "type": "U16"},
            {"name": "ENERGY", "type": "U16"},
            {"name": "FLAGS_B", "type": "U16"},
            {"name": "FLAGS_A", "type": "U16"},
            {"name": "PSD", "type": "U16"},
            {"name": "USER_INFO", "type": "U64", "dim": 1},
            {"name": "USER_INFO_SIZE", "type": "SIZE_T"},
            {"name": "EVENT_SIZE", "type": "SIZE_T"}
        ]"#
        };

        let c_format = CString::new(format_json).map_err(|_| CaenError {
            code: -2,
            name: "InvalidParam".to_string(),
            description: "Format JSON contains null byte".to_string(),
        })?;

        let ret = unsafe { ffi::CAEN_FELib_SetReadDataFormat(read_data_handle, c_format.as_ptr()) };
        CaenError::check(ret)?;

        Ok(EndpointHandle {
            handle: read_data_handle,
        })
    }

    /// Validate that config num_channels does not exceed hardware channel count.
    fn validate_num_channels(&self, config_num_ch: u8) -> Result<(), CaenError> {
        let hw_num_ch: u32 = self
            .get_value("/par/NumCh")
            .unwrap_or_default()
            .parse()
            .unwrap_or(0);
        let config_num_ch = config_num_ch as u32;

        if hw_num_ch > 0 && config_num_ch > hw_num_ch {
            return Err(CaenError {
                code: -1,
                name: "NumChannelsMismatch".to_string(),
                description: format!(
                    "Config num_channels={} exceeds hardware NumCh={}. \
                     Fix the JSON config or run Detect to auto-correct.",
                    config_num_ch, hw_num_ch
                ),
            });
        }
        Ok(())
    }

    /// Apply digitizer configuration.
    ///
    /// Applies all parameters from DigitizerConfig to the device.
    /// Parameters are applied in order: board-level first, then channel defaults,
    /// then channel-specific overrides.
    pub fn apply_config(
        &self,
        config: &crate::config::digitizer::DigitizerConfig,
    ) -> Result<usize, CaenError> {
        use tracing::{debug, info, warn};

        // Validate num_channels against hardware
        self.validate_num_channels(config.num_channels)?;

        let params = config.to_caen_parameters();
        info!("Applying {} parameters to digitizer", params.len());

        let mut applied = 0;
        let mut errors = Vec::new();

        for param in &params {
            match self.set_value(&param.path, &param.value) {
                Ok(()) => {
                    debug!(path = %param.path, value = %param.value, "Parameter set");
                    applied += 1;
                }
                Err(e) => {
                    warn!(
                        path = %param.path,
                        value = %param.value,
                        error = %e,
                        "Failed to set parameter"
                    );
                    errors.push((param.path.clone(), e));
                }
            }
        }

        info!(applied, errors = errors.len(), "Configuration applied");

        // Return error if any critical parameters failed
        if !errors.is_empty() {
            warn!(
                "Some parameters failed to apply: {:?}",
                errors.iter().map(|(p, _)| p).collect::<Vec<_>>()
            );
        }

        // Defense in depth: detect if ALL channel parameters failed
        let ch_params: Vec<_> = params.iter().filter(|p| p.path.contains("/ch/")).collect();
        let ch_errors = errors.iter().filter(|(p, _)| p.contains("/ch/")).count();
        if !ch_params.is_empty() && ch_errors == ch_params.len() {
            return Err(CaenError {
                code: -1,
                name: "AllChannelParamsFailed".to_string(),
                description: format!(
                    "All {} channel parameters failed. \
                     Likely num_channels mismatch. Run Detect to update.",
                    ch_errors
                ),
            });
        }

        Ok(applied)
    }

    /// Apply only SetInRun parameters (safe to call while Running)
    ///
    /// Filters parameters to only those the hardware supports changing
    /// during data acquisition. Non-SetInRun parameters are silently skipped.
    pub fn apply_config_running(
        &self,
        config: &crate::config::digitizer::DigitizerConfig,
    ) -> Result<usize, CaenError> {
        use tracing::{debug, info, warn};

        let params = config.to_caen_parameters_set_in_run();
        info!(
            "Applying {} SetInRun parameters to digitizer (running)",
            params.len()
        );

        let mut applied = 0;
        let mut errors = Vec::new();

        for param in &params {
            match self.set_value(&param.path, &param.value) {
                Ok(()) => {
                    debug!(path = %param.path, value = %param.value, "SetInRun parameter set");
                    applied += 1;
                }
                Err(e) => {
                    warn!(
                        path = %param.path,
                        value = %param.value,
                        error = %e,
                        "Failed to set SetInRun parameter"
                    );
                    errors.push((param.path.clone(), e));
                }
            }
        }

        info!(
            applied,
            errors = errors.len(),
            "SetInRun configuration applied"
        );

        if !errors.is_empty() {
            warn!(
                "Some SetInRun parameters failed: {:?}",
                errors.iter().map(|(p, _)| p).collect::<Vec<_>>()
            );
        }

        Ok(applied)
    }

    /// Build a parameter cache from the DevTree.
    ///
    /// Fetches the device tree once and parses all parameter metadata
    /// (min, max, increment, allowed_values, etc.) into a HashMap keyed by
    /// parameter name. Both board-level (`/par/`) and channel-level
    /// (`/ch/0/par/`) parameters are collected.
    ///
    /// # Returns
    /// * `Ok(cache)` - HashMap mapping parameter name → ParamInfo
    /// * `Err(...)` - If DevTree fetch or JSON parse fails
    pub fn build_param_cache(&self) -> Result<HashMap<String, ParamInfo>, CaenError> {
        use tracing::{debug, info, warn};

        let tree_json = self.get_device_tree()?;
        let tree: serde_json::Value = serde_json::from_str(&tree_json).map_err(|e| CaenError {
            code: -1,
            name: "JsonParseError".to_string(),
            description: format!("Failed to parse DevTree JSON: {}", e),
        })?;

        let mut cache = HashMap::new();

        // Collect board-level parameters from /par/
        if let Some(par) = tree.get("par").and_then(|v| v.as_object()) {
            for (name, node) in par {
                if name == "handle" {
                    continue;
                }
                if let Some(obj) = node.as_object() {
                    // Skip non-parameter nodes (those without datatype)
                    if obj.contains_key("datatype") {
                        match Self::extract_param_info(name, node) {
                            Ok(info) => {
                                debug!(param = %name, "Cached board param");
                                cache.insert(name.clone(), info);
                            }
                            Err(e) => {
                                warn!(param = %name, error = %e, "Failed to parse board param");
                            }
                        }
                    }
                }
            }
        }

        // Collect channel-level parameters from /ch/0/par/
        // (metadata is identical across channels — just sample ch0)
        if let Some(ch0_par) = tree
            .get("ch")
            .and_then(|ch| ch.get("0"))
            .and_then(|ch0| ch0.get("par"))
            .and_then(|v| v.as_object())
        {
            for (name, node) in ch0_par {
                if name == "handle" {
                    continue;
                }
                if let Some(obj) = node.as_object() {
                    if obj.contains_key("datatype") && !cache.contains_key(name) {
                        match Self::extract_param_info(name, node) {
                            Ok(info) => {
                                debug!(param = %name, "Cached channel param");
                                cache.insert(name.clone(), info);
                            }
                            Err(e) => {
                                warn!(param = %name, error = %e, "Failed to parse channel param");
                            }
                        }
                    }
                }
            }
        }

        info!(total = cache.len(), "Parameter cache built from DevTree");
        Ok(cache)
    }

    /// Apply digitizer configuration with validation.
    ///
    /// Each parameter is validated against DevTree metadata before applying:
    /// - Numeric values are snapped to the nearest valid step
    /// - Values are clamped to [min, max]
    /// - Unknown parameters (not in DevTree) are applied without validation
    ///
    /// This replaces `apply_config()` when a param cache is available.
    pub fn apply_config_validated(
        &self,
        config: &crate::config::digitizer::DigitizerConfig,
        param_cache: &HashMap<String, ParamInfo>,
    ) -> Result<ApplyConfigResult, CaenError> {
        // Validate num_channels against hardware
        self.validate_num_channels(config.num_channels)?;

        let params = config.to_caen_parameters();
        self.apply_params_validated(&params, param_cache)
    }

    /// Apply only SetInRun parameters with validation (safe while Running).
    pub fn apply_config_running_validated(
        &self,
        config: &crate::config::digitizer::DigitizerConfig,
        param_cache: &HashMap<String, ParamInfo>,
    ) -> Result<ApplyConfigResult, CaenError> {
        let params = config.to_caen_parameters_set_in_run();
        self.apply_params_validated(&params, param_cache)
    }

    /// Internal: validate and apply a list of parameters.
    fn apply_params_validated(
        &self,
        params: &[crate::config::digitizer::CaenParameter],
        param_cache: &HashMap<String, ParamInfo>,
    ) -> Result<ApplyConfigResult, CaenError> {
        use tracing::{debug, info, warn};

        info!("Applying {} parameters with validation", params.len());

        let mut result = ApplyConfigResult {
            total: params.len(),
            ..Default::default()
        };

        for param in params {
            // Extract parameter name from path (last segment after '/')
            let param_name = param.path.rsplit('/').next().unwrap_or("");

            match param_cache.get(param_name) {
                Some(info) => {
                    // Validate against DevTree metadata
                    let validated = validation::validate_param(&param.value, info);

                    if validated.adjusted {
                        info!(
                            path = %param.path,
                            original = %param.value,
                            adjusted = %validated.value,
                            message = ?validated.message,
                            "Parameter value adjusted"
                        );
                    }

                    match self.set_value(&param.path, &validated.value) {
                        Ok(()) => {
                            let status = if validated.adjusted {
                                result.adjusted += 1;
                                ParamApplyStatus::Adjusted
                            } else {
                                result.ok += 1;
                                ParamApplyStatus::Ok
                            };
                            debug!(path = %param.path, value = %validated.value, "Parameter set");
                            result.details.push(ParamApplyResult {
                                path: param.path.clone(),
                                original_value: param.value.clone(),
                                applied_value: validated.value,
                                status,
                                message: validated.message,
                            });
                        }
                        Err(e) => {
                            warn!(
                                path = %param.path,
                                value = %validated.value,
                                error = %e,
                                "Failed to set parameter"
                            );
                            result.failed += 1;
                            result.details.push(ParamApplyResult {
                                path: param.path.clone(),
                                original_value: param.value.clone(),
                                applied_value: validated.value,
                                status: ParamApplyStatus::Failed,
                                message: Some(format!("{}", e)),
                            });
                        }
                    }
                }
                None => {
                    // Parameter not in cache — apply without validation
                    // (e.g., dt_ext_clock on VME, or range-expanded paths)
                    match self.set_value(&param.path, &param.value) {
                        Ok(()) => {
                            debug!(path = %param.path, value = %param.value, "Parameter set (no cache)");
                            result.ok += 1;
                            result.details.push(ParamApplyResult {
                                path: param.path.clone(),
                                original_value: param.value.clone(),
                                applied_value: param.value.clone(),
                                status: ParamApplyStatus::Ok,
                                message: None,
                            });
                        }
                        Err(e) => {
                            warn!(
                                path = %param.path,
                                value = %param.value,
                                error = %e,
                                "Parameter not in DevTree and set_value failed"
                            );
                            result.skipped += 1;
                            result.details.push(ParamApplyResult {
                                path: param.path.clone(),
                                original_value: param.value.clone(),
                                applied_value: param.value.clone(),
                                status: ParamApplyStatus::Skipped,
                                message: Some(format!("Not in DevTree, set_value failed: {}", e)),
                            });
                        }
                    }
                }
            }
        }

        info!(
            total = result.total,
            ok = result.ok,
            adjusted = result.adjusted,
            failed = result.failed,
            skipped = result.skipped,
            "Validated configuration applied"
        );

        if result.adjusted > 0 {
            let adjusted_params: Vec<_> = result
                .details
                .iter()
                .filter(|d| d.status == ParamApplyStatus::Adjusted)
                .map(|d| format!("{}: {} → {}", d.path, d.original_value, d.applied_value))
                .collect();
            info!("Adjusted parameters: {:?}", adjusted_params);
        }

        // Defense in depth: detect if ALL channel parameters failed
        let ch_total = result
            .details
            .iter()
            .filter(|d| d.path.contains("/ch/"))
            .count();
        let ch_failed = result
            .details
            .iter()
            .filter(|d| {
                d.path.contains("/ch/")
                    && matches!(
                        d.status,
                        ParamApplyStatus::Failed | ParamApplyStatus::Skipped
                    )
            })
            .count();
        if ch_total > 0 && ch_failed == ch_total {
            return Err(CaenError {
                code: -1,
                name: "AllChannelParamsFailed".to_string(),
                description: format!(
                    "All {} channel parameters failed. \
                     Likely num_channels mismatch. Run Detect to update.",
                    ch_failed
                ),
            });
        }

        Ok(result)
    }
}

impl EndpointHandle {
    /// Get the raw handle value
    pub fn raw(&self) -> u64 {
        self.handle
    }

    /// Check if data is available
    ///
    /// # Arguments
    /// * `timeout_ms` - Timeout in milliseconds
    ///
    /// # Returns
    /// * `Ok(true)` - Data is available
    /// * `Ok(false)` - Timeout (no data available)
    /// * `Err(...)` - Error occurred
    pub fn has_data(&self, timeout_ms: i32) -> Result<bool, CaenError> {
        let ret = unsafe { ffi::CAEN_FELib_HasData(self.handle, timeout_ms) };

        if ret == 0 {
            // CAEN_FELib_Success
            Ok(true)
        } else if ret == -11 {
            // CAEN_FELib_Timeout
            Ok(false)
        } else {
            Err(CaenError::from_code(ret).unwrap_or(CaenError {
                code: ret,
                name: "Unknown".to_string(),
                description: "Unknown error in HasData".to_string(),
            }))
        }
    }

    /// Read raw data from the endpoint using a pre-allocated reusable buffer.
    ///
    /// The buffer must be large enough for the maximum expected data.
    /// CAEN FELib does NOT check buffer bounds — undersized buffers cause SIGBUS.
    ///
    /// # Arguments
    /// * `timeout_ms` - Timeout in milliseconds (-1 for infinite)
    /// * `buffer` - Pre-allocated reusable buffer (capacity = max data size)
    ///
    /// # Returns
    /// * `Ok(Some(RawData))` - Data was read successfully (owns a copy of actual data)
    /// * `Ok(None)` - Timeout (no data available)
    /// * `Err(...)` - Error occurred
    pub fn read_data(
        &self,
        timeout_ms: i32,
        buffer: &mut Vec<u8>,
    ) -> Result<Option<RawData>, CaenError> {
        let buffer_capacity = buffer.capacity();
        // Ensure buffer is usable at full capacity
        buffer.resize(buffer_capacity, 0);

        let mut size: usize = 0;
        let mut n_events: u32 = 0;

        // Call ReadData via C wrapper (handles variadic calling convention)
        let ret = unsafe {
            caen_read_data_raw(
                self.handle,
                timeout_ms,
                buffer.as_mut_ptr(),
                &mut size,
                &mut n_events,
            )
        };

        if ret == 0 {
            // Success - copy actual data to a right-sized Vec
            let data = buffer[..size].to_vec();
            Ok(Some(RawData {
                data,
                size,
                n_events,
            }))
        } else if ret == -11 {
            // Timeout
            Ok(None)
        } else if ret == -12 {
            // Stop signal - propagate as Err so read_loop can detect it
            Err(CaenError::from_code(ret).unwrap_or(CaenError {
                code: ret,
                name: "Stop".to_string(),
                description: "Acquisition stopped".to_string(),
            }))
        } else {
            Err(CaenError::from_code(ret).unwrap_or(CaenError {
                code: ret,
                name: "Unknown".to_string(),
                description: "Unknown error in ReadData".to_string(),
            }))
        }
    }

    /// Read a single decoded event from the OpenDPP endpoint.
    ///
    /// This reads one event at a time with decoded fields.
    /// Use with configure_opendpp_endpoint().
    ///
    /// # Arguments
    /// * `timeout_ms` - Timeout in milliseconds (-1 for infinite)
    /// * `user_info_buffer` - Pre-allocated buffer for user info words
    ///
    /// # Returns
    /// * `Ok(Some(OpenDppEvent))` - Event was read successfully
    /// * `Ok(None)` - Timeout (no data available)
    /// * `Err(...)` - Error occurred
    pub fn read_opendpp_event(
        &self,
        timeout_ms: i32,
        user_info_buffer: &mut [u64],
    ) -> Result<Option<OpenDppEvent>, CaenError> {
        let mut channel: u8 = 0;
        let mut timestamp: u64 = 0;
        let mut fine_timestamp: u16 = 0;
        let mut energy: u16 = 0;
        let mut flags_b: u16 = 0;
        let mut flags_a: u16 = 0;
        let mut psd: u16 = 0;
        let mut user_info_size: usize = 0;
        let mut event_size: usize = 0;

        let ret = unsafe {
            caen_read_data_opendpp(
                self.handle,
                timeout_ms,
                &mut channel,
                &mut timestamp,
                &mut fine_timestamp,
                &mut energy,
                &mut flags_b,
                &mut flags_a,
                &mut psd,
                user_info_buffer.as_mut_ptr(),
                &mut user_info_size,
                &mut event_size,
            )
        };

        if ret == 0 {
            // Success
            let user_info = user_info_buffer[..user_info_size].to_vec();
            Ok(Some(OpenDppEvent {
                channel,
                timestamp,
                fine_timestamp,
                energy,
                flags_b,
                flags_a,
                psd,
                user_info,
                waveform: None,
                event_size,
            }))
        } else if ret == -11 {
            // Timeout
            Ok(None)
        } else if ret == -12 {
            // Stop signal
            Err(CaenError::from_code(ret).unwrap_or(CaenError {
                code: ret,
                name: "Stop".to_string(),
                description: "Acquisition stopped".to_string(),
            }))
        } else {
            Err(CaenError::from_code(ret).unwrap_or(CaenError {
                code: ret,
                name: "Unknown".to_string(),
                description: "Unknown error in ReadData (OpenDPP)".to_string(),
            }))
        }
    }

    /// Read a single decoded event with waveform from the OpenDPP endpoint.
    ///
    /// This reads one event at a time with decoded fields and waveform data.
    /// Use with configure_opendpp_endpoint(true).
    ///
    /// # Arguments
    /// * `timeout_ms` - Timeout in milliseconds (-1 for infinite)
    /// * `user_info_buffer` - Pre-allocated buffer for user info words
    /// * `waveform_buffer` - Pre-allocated buffer for waveform samples
    ///
    /// # Returns
    /// * `Ok(Some(OpenDppEvent))` - Event was read successfully
    /// * `Ok(None)` - Timeout (no data available)
    /// * `Err(...)` - Error occurred
    pub fn read_opendpp_event_with_waveform(
        &self,
        timeout_ms: i32,
        user_info_buffer: &mut [u64],
        waveform_buffer: &mut [u16],
    ) -> Result<Option<OpenDppEvent>, CaenError> {
        let mut channel: u8 = 0;
        let mut timestamp: u64 = 0;
        let mut fine_timestamp: u16 = 0;
        let mut energy: u16 = 0;
        let mut flags_b: u16 = 0;
        let mut flags_a: u16 = 0;
        let mut psd: u16 = 0;
        let mut user_info_size: usize = 0;
        let mut waveform_size: usize = 0;
        let mut event_size: usize = 0;

        let ret = unsafe {
            caen_read_data_opendpp_waveform(
                self.handle,
                timeout_ms,
                &mut channel,
                &mut timestamp,
                &mut fine_timestamp,
                &mut energy,
                &mut flags_b,
                &mut flags_a,
                &mut psd,
                user_info_buffer.as_mut_ptr(),
                &mut user_info_size,
                waveform_buffer.as_mut_ptr(),
                &mut waveform_size,
                &mut event_size,
            )
        };

        if ret == 0 {
            // Success
            let user_info = user_info_buffer[..user_info_size].to_vec();
            let waveform = if waveform_size > 0 {
                Some(waveform_buffer[..waveform_size].to_vec())
            } else {
                None
            };
            Ok(Some(OpenDppEvent {
                channel,
                timestamp,
                fine_timestamp,
                energy,
                flags_b,
                flags_a,
                psd,
                user_info,
                waveform,
                event_size,
            }))
        } else if ret == -11 {
            // Timeout
            Ok(None)
        } else if ret == -12 {
            // Stop signal
            Err(CaenError::from_code(ret).unwrap_or(CaenError {
                code: ret,
                name: "Stop".to_string(),
                description: "Acquisition stopped".to_string(),
            }))
        } else {
            Err(CaenError::from_code(ret).unwrap_or(CaenError {
                code: ret,
                name: "Unknown".to_string(),
                description: "Unknown error in ReadData (OpenDPP with waveform)".to_string(),
            }))
        }
    }
}

/// RAII: Automatically close the device when the handle is dropped
impl Drop for CaenHandle {
    fn drop(&mut self) {
        unsafe {
            // Ignore errors on close - we're in a destructor
            let _ = ffi::CAEN_FELib_Close(self.handle);
        }
    }
}

// CaenHandle is NOT Send/Sync because CAEN_FELib_Open/Close are not thread-safe
// according to the documentation. If thread safety is needed, wrap in Arc<Mutex<>>.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raw_data_struct() {
        let raw = RawData {
            data: vec![1, 2, 3, 4],
            size: 4,
            n_events: 1,
        };
        assert_eq!(raw.data.len(), 4);
        assert_eq!(raw.size, 4);
        assert_eq!(raw.n_events, 1);
    }

    #[test]
    fn test_raw_data_debug() {
        let raw = RawData {
            data: vec![0xAB, 0xCD],
            size: 2,
            n_events: 0,
        };
        let debug = format!("{:?}", raw);
        assert!(debug.contains("RawData"));
        assert!(debug.contains("size: 2"));
    }

    #[test]
    fn test_cstring_null_byte_in_url() {
        // Test that null bytes in URL are rejected
        let url_with_null = "dig2://192.168.0.1\0/extra";
        let c_string = CString::new(url_with_null);
        assert!(c_string.is_err());
    }

    #[test]
    fn test_cstring_valid_url() {
        let valid_url = "dig2://192.168.0.1";
        let c_string = CString::new(valid_url);
        assert!(c_string.is_ok());
    }

    #[test]
    fn test_cstring_null_byte_in_path() {
        // Test that null bytes in path are rejected
        let path_with_null = "/par/Model\0Name";
        let c_string = CString::new(path_with_null);
        assert!(c_string.is_err());
    }

    #[test]
    fn test_cstring_valid_path() {
        let valid_path = "/par/ModelName";
        let c_string = CString::new(valid_path);
        assert!(c_string.is_ok());
    }

    #[test]
    fn test_endpoint_handle_raw() {
        let ep = EndpointHandle { handle: 12345 };
        assert_eq!(ep.raw(), 12345);
    }

    #[test]
    fn test_format_json_validity() {
        // Test that the format JSON used in configure_endpoint is valid JSON
        let format_json = r#"[
            {"name": "DATA", "type": "U8", "dim": 1},
            {"name": "SIZE", "type": "SIZE_T", "dim": 0},
            {"name": "N_EVENTS", "type": "U32", "dim": 0}
        ]"#;

        let parsed: Result<serde_json::Value, _> = serde_json::from_str(format_json);
        assert!(parsed.is_ok());

        let arr = parsed.unwrap();
        assert!(arr.is_array());
        assert_eq!(arr.as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_buffer_sizes() {
        // Verify buffer sizes used in the code are reasonable
        let value_buffer_size = 256;
        let name_buffer_size = 32;
        let desc_buffer_size = 256;

        // These should be large enough for typical CAEN responses
        assert!(value_buffer_size >= 128);
        assert!(name_buffer_size >= 16);
        assert!(desc_buffer_size >= 64);
    }

    #[test]
    fn test_device_info_struct() {
        let info = DeviceInfo {
            model: "VX2730".to_string(),
            serial_number: "12345".to_string(),
            firmware_type: "DPP_PSD".to_string(),
            num_channels: 32,
            adc_bits: 14,
            sampling_rate_sps: 125_000_000,
        };
        assert_eq!(info.model, "VX2730");
        assert_eq!(info.num_channels, 32);
        assert_eq!(info.adc_bits, 14);
    }

    #[test]
    fn test_device_info_clone() {
        let info = DeviceInfo {
            model: "VX2730".to_string(),
            serial_number: "12345".to_string(),
            firmware_type: "DPP_PSD".to_string(),
            num_channels: 32,
            adc_bits: 14,
            sampling_rate_sps: 125_000_000,
        };
        let cloned = info.clone();
        assert_eq!(info.model, cloned.model);
        assert_eq!(info.serial_number, cloned.serial_number);
    }

    #[test]
    fn test_device_info_debug() {
        let info = DeviceInfo {
            model: "VX2730".to_string(),
            serial_number: "12345".to_string(),
            firmware_type: "DPP_PSD".to_string(),
            num_channels: 32,
            adc_bits: 14,
            sampling_rate_sps: 125_000_000,
        };
        let debug = format!("{:?}", info);
        assert!(debug.contains("VX2730"));
        assert!(debug.contains("DPP_PSD"));
    }

    #[test]
    fn test_param_info_struct() {
        let info = ParamInfo {
            name: "DCOffset".to_string(),
            datatype: "NUMBER".to_string(),
            access_mode: "READ_WRITE".to_string(),
            setinrun: true,
            min_value: Some("0".to_string()),
            max_value: Some("100".to_string()),
            allowed_values: vec![],
            unit: Some("%".to_string()),
            increment: Some("0.1".to_string()),
            default_value: Some("20".to_string()),
            expuom: Some(0),
        };
        assert_eq!(info.name, "DCOffset");
        assert!(info.setinrun);
        assert_eq!(info.min_value, Some("0".to_string()));
        assert_eq!(info.increment, Some("0.1".to_string()));
        assert_eq!(info.expuom, Some(0));
    }

    #[test]
    fn test_param_info_enum_type() {
        let info = ParamInfo {
            name: "Polarity".to_string(),
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
        assert_eq!(info.allowed_values.len(), 2);
        assert!(!info.setinrun);
    }

    #[test]
    fn test_param_info_clone() {
        let info = ParamInfo {
            name: "TriggerThr".to_string(),
            datatype: "NUMBER".to_string(),
            access_mode: "READ_WRITE".to_string(),
            setinrun: true,
            min_value: Some("0".to_string()),
            max_value: Some("16383".to_string()),
            allowed_values: vec![],
            unit: None,
            increment: Some("1".to_string()),
            default_value: Some("100".to_string()),
            expuom: Some(0),
        };
        let cloned = info.clone();
        assert_eq!(info.name, cloned.name);
        assert_eq!(info.setinrun, cloned.setinrun);
        assert_eq!(info.increment, cloned.increment);
    }

    #[test]
    fn test_extract_param_info_from_json() {
        // Simulate DevTree parameter node structure (DC offset with all metadata)
        let json_str = r#"{
            "accessmode": { "value": "READ_WRITE" },
            "datatype": { "value": "NUMBER" },
            "setinrun": { "value": "true" },
            "minvalue": { "value": "0.0" },
            "maxvalue": { "value": "100.0" },
            "increment": { "value": "0.1" },
            "defaultvalue": { "value": "20" },
            "expuom": { "value": "0" },
            "uom": { "value": "%" }
        }"#;

        let node: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let info = CaenHandle::extract_param_info("DCOffset", &node).unwrap();

        assert_eq!(info.name, "DCOffset");
        assert_eq!(info.datatype, "NUMBER");
        assert!(info.setinrun);
        assert_eq!(info.min_value, Some("0.0".to_string()));
        assert_eq!(info.max_value, Some("100.0".to_string()));
        assert_eq!(info.unit, Some("%".to_string()));
        assert_eq!(info.increment, Some("0.1".to_string()));
        assert_eq!(info.default_value, Some("20".to_string()));
        assert_eq!(info.expuom, Some(0));
    }

    #[test]
    fn test_extract_param_info_time_param() {
        // Simulate DevTree node for a time parameter (ch_trg_holdoff from PSD1)
        let json_str = r#"{
            "accessmode": { "value": "READ_WRITE" },
            "datatype": { "value": "NUMBER" },
            "setinrun": { "value": "true" },
            "minvalue": { "value": "0" },
            "maxvalue": { "value": "524280" },
            "increment": { "value": "8" },
            "defaultvalue": { "value": "1024" },
            "expuom": { "value": "-9" },
            "uom": { "value": "s" }
        }"#;

        let node: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let info = CaenHandle::extract_param_info("ch_trg_holdoff", &node).unwrap();

        assert_eq!(info.increment, Some("8".to_string()));
        assert_eq!(info.expuom, Some(-9));
        assert_eq!(info.default_value, Some("1024".to_string()));
    }

    #[test]
    fn test_extract_param_info_enum() {
        // Simulate DevTree parameter node with allowed values
        let json_str = r#"{
            "accessmode": { "value": "READ_WRITE" },
            "datatype": { "value": "STRING" },
            "setinrun": { "value": "false" },
            "allowedvalues": {
                "handle": 123,
                "value": "2",
                "0": { "value": "Positive" },
                "1": { "value": "Negative" }
            }
        }"#;

        let node: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let info = CaenHandle::extract_param_info("Polarity", &node).unwrap();

        assert_eq!(info.datatype, "STRING");
        assert!(!info.setinrun);
        assert_eq!(info.allowed_values.len(), 2);
        assert!(info.allowed_values.contains(&"Positive".to_string()));
        assert!(info.allowed_values.contains(&"Negative".to_string()));
    }

    /// Test build_param_cache logic by parsing a real DevTree JSON file
    #[test]
    fn test_build_param_cache_from_devtree_json() {
        // Load real DevTree JSON from docs/devtree_examples/
        let json_str =
            std::fs::read_to_string("docs/devtree_examples/dt5730b_psd1_sn990.json").unwrap();
        let tree: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let mut cache = HashMap::new();

        // Collect board-level parameters from /par/
        if let Some(par) = tree.get("par").and_then(|v| v.as_object()) {
            for (name, node) in par {
                if name == "handle" {
                    continue;
                }
                if let Some(obj) = node.as_object() {
                    if obj.contains_key("datatype") {
                        if let Ok(info) = CaenHandle::extract_param_info(name, node) {
                            cache.insert(name.clone(), info);
                        }
                    }
                }
            }
        }

        // Collect channel-level parameters from /ch/0/par/
        if let Some(ch0_par) = tree
            .get("ch")
            .and_then(|ch| ch.get("0"))
            .and_then(|ch0| ch0.get("par"))
            .and_then(|v| v.as_object())
        {
            for (name, node) in ch0_par {
                if name == "handle" {
                    continue;
                }
                if let Some(obj) = node.as_object() {
                    if obj.contains_key("datatype") && !cache.contains_key(name) {
                        if let Ok(info) = CaenHandle::extract_param_info(name, node) {
                            cache.insert(name.clone(), info);
                        }
                    }
                }
            }
        }

        // Verify we got a reasonable number of params
        assert!(cache.len() > 20, "Expected >20 params, got {}", cache.len());

        // Verify specific board-level params exist
        assert!(cache.contains_key("startmode"), "Missing startmode");
        assert!(
            cache.contains_key("dt_ext_clock"),
            "Missing dt_ext_clock (Desktop)"
        );

        // Verify specific channel-level params exist with correct metadata
        let pretrg = cache.get("ch_pretrg").expect("Missing ch_pretrg");
        assert_eq!(pretrg.datatype, "NUMBER");
        assert_eq!(pretrg.increment.as_deref(), Some("8"));
        assert_eq!(pretrg.min_value.as_deref(), Some("40"));
        assert_eq!(pretrg.expuom, Some(-9));

        let gate = cache.get("ch_gate").expect("Missing ch_gate");
        assert_eq!(gate.increment.as_deref(), Some("2"));
        assert_eq!(gate.min_value.as_deref(), Some("4"));

        let holdoff = cache.get("ch_trg_holdoff").expect("Missing ch_trg_holdoff");
        assert_eq!(holdoff.increment.as_deref(), Some("8"));

        let dc_offset = cache.get("ch_dcoffset").expect("Missing ch_dcoffset");
        assert_eq!(dc_offset.increment.as_deref(), Some("0.1"));
        assert_eq!(dc_offset.min_value.as_deref(), Some("0.0"));
        assert_eq!(dc_offset.max_value.as_deref(), Some("100.0"));
    }

    /// Test that param_cache correctly validates real PSD1 parameters
    #[test]
    fn test_param_cache_validation_integration() {
        let json_str =
            std::fs::read_to_string("docs/devtree_examples/dt5730b_psd1_sn990.json").unwrap();
        let tree: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let mut cache = HashMap::new();

        // Build cache (same logic as build_param_cache)
        if let Some(ch0_par) = tree
            .get("ch")
            .and_then(|ch| ch.get("0"))
            .and_then(|ch0| ch0.get("par"))
            .and_then(|v| v.as_object())
        {
            for (name, node) in ch0_par {
                if name == "handle" {
                    continue;
                }
                if let Some(obj) = node.as_object() {
                    if obj.contains_key("datatype") {
                        if let Ok(info) = CaenHandle::extract_param_info(name, node) {
                            cache.insert(name.clone(), info);
                        }
                    }
                }
            }
        }

        // Validate ch_pretrg: 101 → 104 (step=8, min=40)
        let pretrg = cache.get("ch_pretrg").unwrap();
        let result = validation::validate_param("101", pretrg);
        assert!(result.adjusted);
        assert_eq!(result.value, "104");

        // Validate ch_gate: 301 → 302 (step=2, min=4)
        let gate = cache.get("ch_gate").unwrap();
        let result = validation::validate_param("301", gate);
        assert!(result.adjusted);
        assert_eq!(result.value, "302");

        // Validate ch_dcoffset: 50.0 → 50.0 (step=0.1, exact)
        let dc = cache.get("ch_dcoffset").unwrap();
        let result = validation::validate_param("50.0", dc);
        assert!(!result.adjusted);
        assert_eq!(result.value, "50.0");

        // Validate ch_dcoffset: 50.35 → 50.4 (step=0.1)
        let result = validation::validate_param("50.35", dc);
        assert!(result.adjusted);
        assert_eq!(result.value, "50.4");
    }
}
