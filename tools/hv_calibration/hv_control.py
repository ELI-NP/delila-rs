"""CAENHVWrapper ctypes wrapper for SY5527 HV power supply.

SAFETY WARNING: This module controls HIGH VOLTAGE equipment.
Always use --dry-run for testing. Never operate during wiring work.
"""

import ctypes
import ctypes.util
import logging
import time
from dataclasses import dataclass

logger = logging.getLogger(__name__)

# Constants from CAENHVWrapper.h
CAENHV_OK = 0
SY5527 = 3
LINKTYPE_TCPIP = 0
MAX_CH_NAME = 12


CAENHV_ERROR_MESSAGES = {
    0: "OK",
    1: "Generic error",
    2: "Init error",
    3: "Deinit error",
    4: "Communication error",
    5: "Write error",
    6: "Read error",
    7: "No route to device",
    8: "Invalid parameter",
    9: "No data",
    10: "Device already open",
    11: "Too many devices",
    12: "Invalid handle",
    13: "Login failed",
    14: "Login timeout",
    15: "Logout failed",
    16: "Link not supported",
    17: "Cmd not supported",
}


class CAENHVError(Exception):
    """Exception raised by CAENHVWrapper API calls."""

    def __init__(self, code: int, func_name: str = ""):
        self.code = code
        msg = CAENHV_ERROR_MESSAGES.get(code, f"Unknown error {code}")
        if func_name:
            msg = f"{func_name}: {msg}"
        super().__init__(msg)


@dataclass
class HVChannel:
    """Read-only snapshot of one HV channel's state."""
    slot: int
    channel: int
    name: str
    v_set: float
    v_mon: float
    i_mon: float
    status: int
    pw: int  # 1=ON, 0=OFF
    sv_max: float = 0.0  # Software voltage limit (set via GECO2020)


@dataclass
class SlotInfo:
    """One slot in the crate map."""
    slot: int
    model: str
    num_channels: int
    serial_number: int


class HVController:
    """Control interface for CAEN SY5527 HV power supply via CAENHVWrapper.

    Usage:
        with HVController("172.18.5.215", "admin", "eli-np") as hv:
            crate_map = hv.get_crate_map()
            params = hv.get_channel_params(0, list(range(24)))
    """

    def __init__(self, host: str, username: str, password: str,
                 lib_path: str = "/usr/lib64/libcaenhvwrapper.so"):
        self._host = host.encode()
        self._username = username.encode()
        self._password = password.encode()
        self._handle = ctypes.c_int(-1)
        self._connected = False

        try:
            self._lib = ctypes.CDLL(lib_path)
        except OSError:
            raise RuntimeError(
                f"Cannot load {lib_path}. "
                "Ensure CAENHVWrapper is installed on this machine."
            )
        self._setup_prototypes()

    def _setup_prototypes(self):
        """Define C function signatures for type safety."""
        # CAENHV_InitSystem
        self._lib.CAENHV_InitSystem.argtypes = [
            ctypes.c_int,                       # system type
            ctypes.c_int,                       # link type
            ctypes.c_void_p,                    # arg (host)
            ctypes.c_char_p,                    # username
            ctypes.c_char_p,                    # password
            ctypes.POINTER(ctypes.c_int),       # handle out
        ]
        self._lib.CAENHV_InitSystem.restype = ctypes.c_int

        # CAENHV_DeinitSystem
        self._lib.CAENHV_DeinitSystem.argtypes = [ctypes.c_int]
        self._lib.CAENHV_DeinitSystem.restype = ctypes.c_int

        # CAENHV_GetCrateMap — don't set argtypes, use dynamic calling
        # The char** parameters are library-allocated pointer arrays
        self._lib.CAENHV_GetCrateMap.restype = ctypes.c_int

        # CAENHV_GetChParam
        self._lib.CAENHV_GetChParam.argtypes = [
            ctypes.c_int,                           # handle
            ctypes.c_ushort,                        # slot
            ctypes.c_char_p,                        # param name
            ctypes.c_ushort,                        # num channels
            ctypes.POINTER(ctypes.c_ushort),        # channel list
            ctypes.c_void_p,                        # values out
        ]
        self._lib.CAENHV_GetChParam.restype = ctypes.c_int

        # CAENHV_SetChParam
        self._lib.CAENHV_SetChParam.argtypes = [
            ctypes.c_int,                           # handle
            ctypes.c_ushort,                        # slot
            ctypes.c_char_p,                        # param name
            ctypes.c_ushort,                        # num channels
            ctypes.POINTER(ctypes.c_ushort),        # channel list
            ctypes.c_void_p,                        # values
        ]
        self._lib.CAENHV_SetChParam.restype = ctypes.c_int

        # CAENHV_GetChName
        self._lib.CAENHV_GetChName.argtypes = [
            ctypes.c_int,                           # handle
            ctypes.c_ushort,                        # slot
            ctypes.c_ushort,                        # num channels
            ctypes.POINTER(ctypes.c_ushort),        # channel list
            ctypes.c_void_p,                        # names out
        ]
        self._lib.CAENHV_GetChName.restype = ctypes.c_int

        # CAENHV_Free
        self._lib.CAENHV_Free.argtypes = [ctypes.c_void_p]
        self._lib.CAENHV_Free.restype = ctypes.c_int

    def _check(self, result: int, func_name: str = ""):
        """Check return value and raise on error."""
        if result != CAENHV_OK:
            raise CAENHVError(result, func_name)

    # --- Context manager ---

    def __enter__(self) -> "HVController":
        self.connect()
        return self

    def __exit__(self, *args):
        self.disconnect()

    def connect(self):
        """Connect to the SY5527."""
        if self._connected:
            return
        result = self._lib.CAENHV_InitSystem(
            SY5527,
            LINKTYPE_TCPIP,
            self._host,
            self._username,
            self._password,
            ctypes.byref(self._handle),
        )
        self._check(result, "CAENHV_InitSystem")
        self._connected = True
        logger.info("Connected to SY5527 (handle=%d)", self._handle.value)

    def disconnect(self):
        """Disconnect from the SY5527."""
        if not self._connected:
            return
        try:
            self._lib.CAENHV_DeinitSystem(self._handle)
        except Exception as e:
            logger.warning("DeinitSystem error: %s", e)
        self._connected = False
        logger.info("Disconnected from SY5527")

    # --- Read operations ---

    def get_crate_map(self) -> list[SlotInfo]:
        """Get the crate slot/board configuration.

        Returns list of populated slots with model, channel count, serial.
        Note: GetCrateMap allocates memory that must be freed with CAENHV_Free.
        """
        nr_slots = ctypes.c_ushort()
        p_nrch = ctypes.POINTER(ctypes.c_ushort)()
        p_model = ctypes.POINTER(ctypes.c_char_p)()
        p_desc = ctypes.POINTER(ctypes.c_char_p)()
        p_serial = ctypes.POINTER(ctypes.c_ushort)()
        p_fwmin = ctypes.POINTER(ctypes.c_ubyte)()
        p_fwmax = ctypes.POINTER(ctypes.c_ubyte)()

        result = self._lib.CAENHV_GetCrateMap(
            self._handle,
            ctypes.byref(nr_slots),
            ctypes.byref(p_nrch),
            ctypes.byref(p_model),
            ctypes.byref(p_desc),
            ctypes.byref(p_serial),
            ctypes.byref(p_fwmin),
            ctypes.byref(p_fwmax),
        )
        self._check(result, "CAENHV_GetCrateMap")

        slots = []
        n = nr_slots.value
        for i in range(n):
            n_ch = p_nrch[i]
            if n_ch == 0:
                continue  # empty slot
            model = p_model[i].decode() if p_model[i] else "Unknown"
            serial = p_serial[i]
            slots.append(SlotInfo(
                slot=i,
                model=model,
                num_channels=n_ch,
                serial_number=serial,
            ))

        # Free CAEN-allocated memory
        for ptr in [p_nrch, p_model, p_desc, p_serial, p_fwmin, p_fwmax]:
            if ptr:
                self._lib.CAENHV_Free(ptr)

        return slots

    def get_channel_names(self, slot: int, channels: list[int]) -> list[str]:
        """Get channel names set via GECO2020.

        Returns list of names (MAX_CH_NAME=12 chars each).
        """
        n = len(channels)
        ch_arr = (ctypes.c_ushort * n)(*channels)
        # Names: array of char[MAX_CH_NAME] = char[12]
        NameType = ctypes.c_char * MAX_CH_NAME
        names_arr = (NameType * n)()

        result = self._lib.CAENHV_GetChName(
            self._handle,
            ctypes.c_ushort(slot),
            ctypes.c_ushort(n),
            ch_arr,
            ctypes.cast(names_arr, ctypes.c_void_p),
        )
        self._check(result, "CAENHV_GetChName")

        return [names_arr[i].value.decode().strip('\x00') for i in range(n)]

    def _get_float_param(self, slot: int, param: str,
                         channels: list[int]) -> list[float]:
        """Read a float parameter (VSet, VMon, IMon) for multiple channels."""
        n = len(channels)
        ch_arr = (ctypes.c_ushort * n)(*channels)
        val_arr = (ctypes.c_float * n)()

        result = self._lib.CAENHV_GetChParam(
            self._handle,
            ctypes.c_ushort(slot),
            param.encode(),
            ctypes.c_ushort(n),
            ch_arr,
            ctypes.cast(val_arr, ctypes.c_void_p),
        )
        self._check(result, f"CAENHV_GetChParam({param})")
        return [val_arr[i] for i in range(n)]

    def _get_uint_param(self, slot: int, param: str,
                        channels: list[int]) -> list[int]:
        """Read an unsigned int parameter (Status, Pw) for multiple channels."""
        n = len(channels)
        ch_arr = (ctypes.c_ushort * n)(*channels)
        val_arr = (ctypes.c_uint * n)()

        result = self._lib.CAENHV_GetChParam(
            self._handle,
            ctypes.c_ushort(slot),
            param.encode(),
            ctypes.c_ushort(n),
            ch_arr,
            ctypes.cast(val_arr, ctypes.c_void_p),
        )
        self._check(result, f"CAENHV_GetChParam({param})")
        return [val_arr[i] for i in range(n)]

    def _get_float_param_fallback(self, slot: int, primary: str,
                                   fallback: str,
                                   channels: list[int]) -> list[float]:
        """Try primary param name, fall back to alternative."""
        try:
            return self._get_float_param(slot, primary, channels)
        except CAENHVError:
            try:
                return self._get_float_param(slot, fallback, channels)
            except CAENHVError:
                return [0.0] * len(channels)

    def get_channel_params(self, slot: int,
                           channels: list[int]) -> list[HVChannel]:
        """Read VSet, VMon, IMon, Status, Pw for multiple channels.

        Handles board variants: some use V0Set/I0Set instead of VSet/ISet.
        """
        try:
            names = self.get_channel_names(slot, channels)
        except CAENHVError:
            names = [f"ch{c}" for c in channels]
        v_set = self._get_float_param_fallback(slot, "VSet", "V0Set", channels)
        try:
            v_mon = self._get_float_param(slot, "VMon", channels)
        except CAENHVError:
            v_mon = [0.0] * len(channels)
        try:
            i_mon = self._get_float_param(slot, "IMon", channels)
        except CAENHVError:
            i_mon = [0.0] * len(channels)
        try:
            status = self._get_uint_param(slot, "Status", channels)
        except CAENHVError:
            status = [0] * len(channels)
        try:
            pw = self._get_uint_param(slot, "Pw", channels)
        except CAENHVError:
            pw = [0] * len(channels)
        try:
            sv_max = self._get_float_param(slot, "SVMax", channels)
        except CAENHVError:
            sv_max = [0.0] * len(channels)

        return [
            HVChannel(
                slot=slot,
                channel=channels[i],
                name=names[i],
                v_set=v_set[i],
                v_mon=v_mon[i],
                i_mon=i_mon[i],
                status=status[i],
                pw=pw[i],
                sv_max=sv_max[i],
            )
            for i in range(len(channels))
        ]

    # --- Write operations ---

    def set_voltage(self, slot: int, channel: int, voltage: float):
        """Set VSet for one channel."""
        self.set_voltages(slot, [channel], [voltage])

    def set_voltages(self, slot: int, channels: list[int],
                     voltages: list[float]):
        """Set VSet for multiple channels at once.

        Reads SVMax and clamps voltages to stay within limits.
        """
        n = len(channels)
        assert len(voltages) == n

        # Safety: read SVMax and clamp
        try:
            sv_max = self._get_float_param(slot, "SVMax", channels)
            clamped = []
            for i in range(n):
                v = voltages[i]
                if sv_max[i] > 0 and v > sv_max[i]:
                    logger.warning("Clamping ch%d voltage %.1f -> SVMax %.1f",
                                   channels[i], v, sv_max[i])
                    v = sv_max[i]
                clamped.append(max(0.0, v))
            voltages = clamped
        except CAENHVError:
            voltages = [max(0.0, v) for v in voltages]

        ch_arr = (ctypes.c_ushort * n)(*channels)
        val_arr = (ctypes.c_float * n)(*voltages)

        # Try VSet first, fall back to V0Set
        for param_name in [b"VSet", b"V0Set"]:
            result = self._lib.CAENHV_SetChParam(
                self._handle,
                ctypes.c_ushort(slot),
                param_name,
                ctypes.c_ushort(n),
                ch_arr,
                ctypes.cast(val_arr, ctypes.c_void_p),
            )
            if result == CAENHV_OK:
                break
        self._check(result, "CAENHV_SetChParam(VSet/V0Set)")
        logger.info("Set voltage: slot=%d channels=%s voltages=%s",
                     slot, channels, voltages)

    def set_power(self, slot: int, channel: int, on: bool):
        """Turn a channel ON or OFF."""
        n = 1
        ch_arr = (ctypes.c_ushort * n)(channel)
        val_arr = (ctypes.c_uint * n)(1 if on else 0)

        result = self._lib.CAENHV_SetChParam(
            self._handle,
            ctypes.c_ushort(slot),
            b"Pw",
            ctypes.c_ushort(n),
            ch_arr,
            ctypes.cast(val_arr, ctypes.c_void_p),
        )
        self._check(result, "CAENHV_SetChParam(Pw)")
        logger.info("Set Pw: slot=%d ch=%d on=%s", slot, channel, on)

    def wait_ramp(self, slot: int, channels: list[int],
                  tolerance: float = 1.0, timeout: float = 60.0) -> bool:
        """Wait until VMon reaches VSet ± tolerance for all channels.

        Returns True if converged, False if timed out.
        """
        start = time.time()
        while time.time() - start < timeout:
            v_set = self._get_float_param_fallback(slot, "VSet", "V0Set", channels)
            v_mon = self._get_float_param(slot, "VMon", channels)
            all_stable = all(
                abs(v_mon[i] - v_set[i]) <= tolerance
                for i in range(len(channels))
            )
            if all_stable:
                return True
            time.sleep(1.0)
        logger.warning("Ramp timeout: slot=%d channels=%s", slot, channels)
        return False
