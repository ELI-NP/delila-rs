/**
 * C wrapper for CAEN_FELib variadic functions
 *
 * Rust cannot directly call C variadic functions correctly on all platforms
 * (especially macOS ARM64). This wrapper provides non-variadic functions
 * that Rust can safely call.
 */

#include <stddef.h>
#include <stdint.h>
#include <CAEN_FELib.h>

/**
 * Wrapper for CAEN_FELib_ReadData with RAW format:
 * - DATA: uint8_t* (pointer to buffer)
 * - SIZE: size_t* (pointer to receive actual size)
 * - N_EVENTS: uint32_t* (pointer to receive event count)
 */
int caen_read_data_raw(
    uint64_t handle,
    int timeout,
    uint8_t* data,
    size_t* size,
    uint32_t* n_events
) {
    return CAEN_FELib_ReadData(handle, timeout, data, size, n_events);
}

/**
 * Wrapper for CAEN_FELib_ReadData with OpenDPP format (no waveform):
 * Format: CHANNEL, TIMESTAMP, FINE_TIMESTAMP, ENERGY, FLAGS_B, FLAGS_A, PSD,
 *         USER_INFO (array), USER_INFO_SIZE, EVENT_SIZE
 */
int caen_read_data_opendpp(
    uint64_t handle,
    int timeout,
    uint8_t* channel,
    uint64_t* timestamp,
    uint16_t* fine_timestamp,
    uint16_t* energy,
    uint16_t* flags_b,
    uint16_t* flags_a,
    uint16_t* psd,
    uint64_t* user_info,
    size_t* user_info_size,
    size_t* event_size
) {
    return CAEN_FELib_ReadData(handle, timeout,
        channel, timestamp, fine_timestamp, energy,
        flags_b, flags_a, psd,
        user_info, user_info_size, event_size);
}

/**
 * Wrapper for CAEN_FELib_ReadData with OpenDPP format (with waveform):
 * Format: CHANNEL, TIMESTAMP, FINE_TIMESTAMP, ENERGY, FLAGS_B, FLAGS_A, PSD,
 *         USER_INFO (array), USER_INFO_SIZE, WAVEFORM (array), WAVEFORM_SIZE, EVENT_SIZE
 */
int caen_read_data_opendpp_waveform(
    uint64_t handle,
    int timeout,
    uint8_t* channel,
    uint64_t* timestamp,
    uint16_t* fine_timestamp,
    uint16_t* energy,
    uint16_t* flags_b,
    uint16_t* flags_a,
    uint16_t* psd,
    uint64_t* user_info,
    size_t* user_info_size,
    uint16_t* waveform,
    size_t* waveform_size,
    size_t* event_size
) {
    return CAEN_FELib_ReadData(handle, timeout,
        channel, timestamp, fine_timestamp, energy,
        flags_b, flags_a, psd,
        user_info, user_info_size,
        waveform, waveform_size, event_size);
}
