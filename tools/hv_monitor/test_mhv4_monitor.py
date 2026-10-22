"""Unit tests for mhv4_monitor (no hardware, no pyserial needed).

Run from the repo root:
    python3 -m unittest discover tools/hv_monitor -p 'test_*.py'
"""

import inspect
import logging
import os
import unittest

import requests

import mhv4_monitor as m

HERE = os.path.dirname(os.path.abspath(__file__))

# Exact byte strings captured from the real MHV-4 (FW V4.2) on 2026-10-22, TODO 75 §2.1.
REAL = {
    "RU 0": b"RU 0\n\rU0: +0.0\n\rMHV-4>",
    "RUL 0": b"RUL 0\n\rUlim 0: +27.50\n\rMHV-4>",
    "RUL 3": b"RUL 3\n\rUlim 3: +110.2\n\rMHV-4>",
    "RI 0": b"RI 0\n\rI 0: -0.004 uA\n\rMHV-4>",
    "RIL 0": b"RIL 0\n\rIlim 0: 20.000 uA\n\rMHV-4>",
    "RP 0": b"RP 0\n\rPolarity 0: positive\n\rMHV-4>",
    "RRA": b"RRA\n\rMHV ramp speed: 5 V/s\n\rMHV-4>",
}
REAL_RUP_ERROR = b"RUP 0\n\rERROR!\n\rMHV-4>"  # RUP is never sent; the bytes show the ERROR! form

SET_COMMANDS = ["ON", "OFF", "SU", "SUL", "SIL", "SP", "AS", "STC", "STO", "STS", "SRA"]


def device_replies(vmon=(0.0, 0.0, 0.0, 0.0), polarity=("positive",) * 4, ramp=5):
    """Replies for all 21 commands in the format of the real replies above.

    Channels 1-3 are synthesized in the same format as the captured channel-0 replies.
    """
    replies = {}
    for c in range(4):
        replies[f"RU {c}"] = f"RU {c}\n\rU{c}: {vmon[c]:+.1f}\n\rMHV-4>".encode()
        replies[f"RUL {c}"] = f"RUL {c}\n\rUlim {c}: +110.2\n\rMHV-4>".encode()
        replies[f"RI {c}"] = f"RI {c}\n\rI {c}: -0.004 uA\n\rMHV-4>".encode()
        replies[f"RIL {c}"] = f"RIL {c}\n\rIlim {c}: 20.000 uA\n\rMHV-4>".encode()
        replies[f"RP {c}"] = f"RP {c}\n\rPolarity {c}: {polarity[c]}\n\rMHV-4>".encode()
    replies["RRA"] = f"RRA\n\rMHV ramp speed: {ramp} V/s\n\rMHV-4>".encode()
    return replies


class FakeClock:
    def __init__(self):
        self.t = 0.0

    def __call__(self):
        return self.t

    def advance(self, dt):
        self.t += dt


class FakeSerial:
    """Replays canned replies: write(b'<cmd>\\r') queues replies[cmd] for reading.

    read() returns at most `chunk` bytes per call; an empty read advances the fake clock by the
    pyserial per-read timeout, as a real port would block for it.
    """

    def __init__(self, replies, chunk=None, clock=None, fail_on_read=None):
        self.replies = dict(replies)
        self.chunk = chunk
        self.clock = clock
        self.fail_on_read = fail_on_read  # exception to raise from read()
        self.written = []
        self.resets = 0
        self.closed = False
        self._rx = bytearray()

    @property
    def in_waiting(self):
        n = len(self._rx)
        return min(n, self.chunk) if self.chunk else n

    def reset_input_buffer(self):
        self.resets += 1
        self._rx.clear()

    def write(self, data):
        self.written.append(bytes(data))
        reply = self.replies.get(bytes(data).decode("ascii").removesuffix("\r"))
        if reply is not None:
            self._rx += reply
        return len(data)

    def read(self, n=1):
        if self.fail_on_read is not None:
            raise self.fail_on_read
        if not self._rx:
            if self.clock is not None:
                self.clock.advance(m.READ_SLICE_S)
            return b""
        k = min(n, len(self._rx), self.chunk or n)
        out = bytes(self._rx[:k])
        del self._rx[:k]
        return out

    def close(self):
        self.closed = True


def no_warn(*_args, **_kwargs):
    return True


class RecordingWarn:
    def __init__(self):
        self.calls = []

    def __call__(self, key, msg, *args, **kwargs):
        self.calls.append((key, msg))
        return True


# --- allowlist ----------------------------------------------------------------------------------


class TestAllowlist(unittest.TestCase):
    def test_allowlist_is_exactly_the_21_read_commands(self):
        expected = {"RRA"}
        for c in "0123":
            expected |= {f"RU {c}", f"RUL {c}", f"RI {c}", f"RIL {c}", f"RP {c}"}
        self.assertEqual(m.ALLOWED_COMMANDS, frozenset(expected))
        self.assertEqual(len(m.ALLOWED_COMMANDS), 21)

    def assert_rejected(self, cmd):
        port = FakeSerial(REAL)
        with self.assertRaises(m.ForbiddenCommandError, msg=repr(cmd)):
            m.send_command(port, cmd)
        self.assertEqual(port.written, [], f"bytes written for {cmd!r}")
        self.assertEqual(port.resets, 0, f"port touched for {cmd!r}")

    def test_every_set_command_is_rejected_before_any_byte(self):
        for name in SET_COMMANDS:
            for cmd in (name, f"{name} 0", f"{name} 3", f"{name} 4", f"{name} a", f"{name} 0 100"):
                self.assert_rejected(cmd)
        for cmd in ("SU 0 1000", "SUL 1 8000", "SIL 2 200", "SP 0 n", "SP 0 p", "SP a n",
                    "AS 0 1", "STC 0 1", "STO 0 0", "STS 0 1", "SRA 1", "ON a", "OFF a"):
            self.assert_rejected(cmd)

    def test_unlisted_read_commands_are_rejected(self):
        for cmd in ("RUP 0", "RUP 3", "RT 0", "RTC 0", "RU 4", "RU a", "RI 4", "RP a",
                    "RIL 9", "RUL a", "RRA 0"):
            self.assert_rejected(cmd)

    def test_malformed_commands_are_rejected(self):
        for cmd in ("", " ", "RU", "RU0", "RU  0", " RU 0", "RU 0 ", "ru 0", "Ru 0",
                    "RU 0\r", "RU 0\n", "RU 0\rON 0", "RU 0\r\nOFF 0", "RU 0;ON 0",
                    "RU -1", "RU 00", "RU 1.0", "RU 0 0", "RRA\r", "RRA ", "\rON 0"):
            self.assert_rejected(cmd)

    def test_non_string_commands_are_rejected(self):
        for cmd in (b"RU 0", None, 0, ["RU 0"], ("RU", 0), {"RU 0"}):
            self.assert_rejected(cmd)

    def test_allowed_command_is_written_once_with_cr(self):
        port = FakeSerial(REAL)
        m.send_command(port, "RU 0")
        self.assertEqual(port.written, [b"RU 0\r"])
        self.assertEqual(port.resets, 1)

    def test_send_command_is_the_only_write_site(self):
        with open(os.path.join(HERE, "mhv4_monitor.py")) as f:
            source = f.read()
        self.assertEqual(source.count(".write("), 1)
        self.assertNotIn(".writelines(", source)
        self.assertIn(".write(", inspect.getsource(m.send_command))


# --- reply parsing ------------------------------------------------------------------------------


class TestRealReplies(unittest.TestCase):
    def q(self, cmd, replies=REAL, **kw):
        clock = FakeClock()
        port = FakeSerial(replies, clock=clock, **kw)
        return m.query(port, cmd, 1.0, clock)

    def test_each_real_reply(self):
        cases = {
            "RU 0": 0.0,
            "RUL 0": 27.5,
            "RUL 3": 110.2,
            "RI 0": -0.004,
            "RIL 0": 20.0,
            "RRA": 5.0,
        }
        for cmd, expected in cases.items():
            value = self.q(cmd)
            self.assertIsInstance(value, float, cmd)
            self.assertAlmostEqual(value, expected, places=9, msg=cmd)
        self.assertEqual(self.q("RP 0"), "positive")

    def test_byte_at_a_time_delivery(self):
        for cmd in REAL:
            self.assertEqual(self.q(cmd, chunk=1), self.q(cmd), cmd)

    def test_reply_lines_and_real_error_reply(self):
        self.assertEqual(
            m.split_reply_lines(REAL["RI 0"]), ["RI 0", "I 0: -0.004 uA", "MHV-4>"]
        )
        port = FakeSerial({})
        port._rx += REAL_RUP_ERROR
        body = m.read_reply(port, "RUP 0", 1.0, FakeClock())
        self.assertEqual(body, ["ERROR!"])
        with self.assertRaises(m.ReplyError) as cm:
            m.parse_value("RU 0", body)
        self.assertEqual(cm.exception.kind, "device-error")

    def test_error_reply_to_an_allowed_command(self):
        with self.assertRaises(m.ReplyError) as cm:
            self.q("RU 1", {"RU 1": b"RU 1\n\rERROR!\n\rMHV-4>"})
        self.assertEqual(cm.exception.kind, "device-error")

    def test_negative_values_and_polarity(self):
        self.assertEqual(self.q("RU 2", {"RU 2": b"RU 2\n\rU2: -350.5\n\rMHV-4>"}), -350.5)
        self.assertEqual(
            self.q("RP 2", {"RP 2": b"RP 2\n\rPolarity 2: negative\n\rMHV-4>"}), "negative"
        )

    def test_digit_count_is_not_assumed(self):
        for text, expected in (("+7", 7.0), ("+800.00", 800.0), ("+0.0125", 0.0125),
                               ("12.", 12.0), (".5", 0.5)):
            reply = f"RUL 1\n\rUlim 1: {text}\n\rMHV-4>".encode()
            self.assertEqual(self.q("RUL 1", {"RUL 1": reply}), expected, text)

    def test_timeout_without_reply(self):
        clock = FakeClock()
        port = FakeSerial({}, clock=clock)
        with self.assertRaises(m.ReplyError) as cm:
            m.query(port, "RU 0", 1.0, clock)
        self.assertEqual(cm.exception.kind, "timeout")
        self.assertGreaterEqual(clock.t, 1.0)
        self.assertLess(clock.t, 1.0 + 2 * m.READ_SLICE_S)

    def test_partial_reply_times_out(self):
        for partial in (b"RU 0\n\rU0: +1", b"RU 0\n\r", b"RU 0", b"RU 0\n\rU0: +12.5\n\rMHV-4"):
            with self.assertRaises(m.ReplyError) as cm:
                self.q("RU 0", {"RU 0": partial})
            self.assertEqual(cm.exception.kind, "timeout", partial)

    def test_late_tail_of_an_earlier_reply_is_skipped(self):
        replies = {"RU 1": b"0 uA\n\rMHV-4>" + b"RU 1\n\rU1: +50.0\n\rMHV-4>"}
        self.assertEqual(self.q("RU 1", replies), 50.0)
        self.assertEqual(self.q("RU 1", replies, chunk=3), 50.0)

    def test_reply_for_another_command_is_never_used(self):
        with self.assertRaises(m.ReplyError) as cm:
            self.q("RU 1", {"RU 1": REAL["RU 0"]})
        self.assertEqual(cm.exception.kind, "timeout")

    def test_malformed_value_lines(self):
        bad = {
            "RU 1": [b"U0: +5.0",          # label is for another channel
                     b"I 1: +5.0",         # label is for another quantity
                     b"U1: +5.0 V",        # unit never seen on RU
                     b"U1: +abc",
                     b"U1:",
                     b"U1 +5.0",           # no ':'
                     b"U1: +5.0\n\rU1: +6.0",  # two value lines
                     b"U1: nan",
                     b"U1: +1e3"],
            "RI 0": [b"I 0: -4 nA",         # wrong unit is rejected, not converted
                     b"I 0: -0.004",
                     b"I 0: -0.004 uA extra"],
            "RRA": [b"MHV ramp speed: 5"],
            "RP 0": [b"Polarity 0: bipolar", b"Polarity 0: +1"],
        }
        for cmd, lines in bad.items():
            for line in lines:
                reply = cmd.encode() + b"\n\r" + line + b"\n\rMHV-4>"
                with self.assertRaises(m.ReplyError, msg=line) as cm:
                    self.q(cmd, {cmd: reply})
                self.assertEqual(cm.exception.kind, "malformed", line)

    def test_empty_body_is_malformed(self):
        with self.assertRaises(m.ReplyError) as cm:
            self.q("RU 0", {"RU 0": b"RU 0\n\rMHV-4>"})
        self.assertEqual(cm.exception.kind, "malformed")


# --- line protocol ------------------------------------------------------------------------------


def full_reading():
    reading = m.CycleReading(channels=[m.ChannelReading(c) for c in range(4)], ramp_vps=5.0)
    for ch in reading.channels:
        ch.vmon, ch.vlimit, ch.imon, ch.ilimit, ch.polarity = 0.0, 27.5, -0.004, 20.0, "positive"
    return reading


class TestLineProtocol(unittest.TestCase):
    def test_full_cycle_lines(self):
        reading = full_reading()
        reading.channels[3].vmon = 110.2
        lines = m.format_lines("si_bias", reading, 1.0)
        self.assertEqual(len(lines), 5)
        self.assertEqual(
            lines[0],
            'mhv4,device=si_bias,channel=0 vmon=0.0,vlimit=27.5,imon=-0.004,ilimit=20.0,'
            'polarity="positive",hv_present=false',
        )
        self.assertEqual(
            lines[3],
            'mhv4,device=si_bias,channel=3 vmon=110.2,vlimit=27.5,imon=-0.004,ilimit=20.0,'
            'polarity="positive",hv_present=true',
        )
        self.assertEqual(lines[4], "mhv4_ramp,device=si_bias speed_vps=5.0")

    def test_failed_reads_are_omitted_not_guessed(self):
        reading = full_reading()
        reading.channels[0].vmon = None
        reading.channels[1].polarity = None
        reading.channels[2] = m.ChannelReading(2)  # every read failed
        reading.ramp_vps = None
        lines = m.format_lines("si_bias", reading, 1.0)
        self.assertEqual(
            lines,
            [
                'mhv4,device=si_bias,channel=0 vlimit=27.5,imon=-0.004,ilimit=20.0,'
                'polarity="positive"',
                "mhv4,device=si_bias,channel=1 vmon=0.0,vlimit=27.5,imon=-0.004,ilimit=20.0,"
                "hv_present=false",
                'mhv4,device=si_bias,channel=3 vmon=0.0,vlimit=27.5,imon=-0.004,ilimit=20.0,'
                'polarity="positive",hv_present=false',
            ],
        )

    def test_nothing_read_gives_no_lines(self):
        reading = m.CycleReading(channels=[m.ChannelReading(c) for c in range(4)])
        self.assertEqual(m.format_lines("si_bias", reading, 1.0), [])

    def test_tag_escaping(self):
        self.assertEqual(m.escape_tag("si bias,a=b"), r"si\ bias\,a\=b")
        reading = m.CycleReading(ramp_vps=5.0)
        self.assertEqual(
            m.format_lines("Si bias,x=1", reading, 1.0),
            [r"mhv4_ramp,device=Si\ bias\,x\=1 speed_vps=5.0"],
        )

    def test_string_field_escaping(self):
        self.assertEqual(m.escape_field_string('a"b\\c'), 'a\\"b\\\\c')
        reading = m.CycleReading(channels=[m.ChannelReading(0, polarity='x"y')])
        self.assertEqual(
            m.format_lines("d", reading, 1.0), ['mhv4,device=d,channel=0 polarity="x\\"y"']
        )

    def test_float_literals(self):
        self.assertEqual(m.fmt_float(5), "5.0")  # never an integer literal
        self.assertEqual(m.fmt_float(-0.004), "-0.004")
        self.assertEqual(m.fmt_float(110.2), "110.2")
        self.assertEqual(m.fmt_float(1e-05), "0.00001")
        self.assertNotIn("e", m.fmt_float(1e20))
        for bad in (float("nan"), float("inf"), float("-inf")):
            with self.assertRaises(ValueError):
                m.fmt_float(bad)

    def test_hv_present_threshold(self):
        cases = [
            (0.0, 1.0, False), (0.9, 1.0, False), (1.0, 1.0, False), (1.0125, 1.0, True),
            (-1.0125, 1.0, True), (-800.0, 1.0, True), (-0.5, 1.0, False),
            (4.9, 5.0, False), (5.1, 5.0, True), (0.0125, 0.0, True), (0.0, 0.0, False),
        ]
        for vmon, threshold, expected in cases:
            self.assertIs(m.hv_present(vmon, threshold), expected, (vmon, threshold))
            reading = m.CycleReading(channels=[m.ChannelReading(0, vmon=vmon)])
            line = m.format_lines("d", reading, threshold)[0]
            self.assertTrue(line.endswith(f"hv_present={'true' if expected else 'false'}"), line)


# --- poll cycle ---------------------------------------------------------------------------------


class TestPollCycle(unittest.TestCase):
    def test_full_cycle_sends_only_the_21_read_commands(self):
        clock = FakeClock()
        port = FakeSerial(device_replies(), clock=clock)
        reading = m.poll_cycle(port, 1.0, no_warn, clock)
        sent = [w.decode("ascii") for w in port.written]
        self.assertEqual(len(sent), 21)
        self.assertTrue(all(s.endswith("\r") for s in sent))
        self.assertEqual({s[:-1] for s in sent}, set(m.ALLOWED_COMMANDS))
        self.assertEqual(sent[:5], ["RU 0\r", "RUL 0\r", "RI 0\r", "RIL 0\r", "RP 0\r"])
        self.assertEqual(sent[-1], "RRA\r")
        for ch in reading.channels:
            self.assertEqual(
                (ch.vmon, ch.vlimit, ch.imon, ch.ilimit, ch.polarity),
                (0.0, 110.2, -0.004, 20.0, "positive"),
            )
        self.assertEqual(reading.ramp_vps, 5.0)

    def test_one_bad_reply_skips_only_that_field(self):
        replies = device_replies()
        replies["RI 2"] = b"RI 2\n\rERROR!\n\rMHV-4>"
        clock = FakeClock()
        warn = RecordingWarn()
        reading = m.poll_cycle(FakeSerial(replies, clock=clock), 1.0, warn, clock)
        self.assertIsNone(reading.channels[2].imon)
        self.assertEqual(reading.channels[2].vmon, 0.0)
        self.assertEqual(reading.channels[3].imon, -0.004)
        self.assertEqual([k for k, _ in warn.calls], ["reply:RI 2:device-error"])

    def test_no_state_is_carried_between_cycles(self):
        clock = FakeClock()
        port = FakeSerial(device_replies(vmon=(100.0, 0.0, -350.0, 0.0)), clock=clock)
        first = m.format_lines("si_bias", m.poll_cycle(port, 1.0, no_warn, clock), 1.0)
        self.assertIn("vmon=100.0", first[0])
        self.assertIn("hv_present=true", first[0])
        self.assertIn("hv_present=true", first[2])

        # Front panel: ch0 switched off (0 V). ch2's RU and RP reads fail this cycle.
        port.replies = device_replies(vmon=(0.0, 0.0, -350.0, 0.0))
        port.replies["RU 2"] = b"RU 2\n\rERROR!\n\rMHV-4>"
        del port.replies["RP 2"]  # timeout
        second = m.format_lines("si_bias", m.poll_cycle(port, 1.0, no_warn, clock), 1.0)
        self.assertIn("vmon=0.0", second[0])
        self.assertIn("hv_present=false", second[0])
        self.assertNotIn("vmon=", second[2])
        self.assertNotIn("hv_present", second[2])  # not remembered from the first cycle
        self.assertNotIn("polarity", second[2])

    def test_silent_device_aborts_the_cycle(self):
        clock = FakeClock()
        port = FakeSerial({}, clock=clock)
        with self.assertRaises(m.DeviceSilentError):
            m.poll_cycle(port, 1.0, no_warn, clock)
        self.assertEqual(len(port.written), m.MAX_CONSECUTIVE_TIMEOUTS)

    def test_isolated_timeouts_do_not_abort(self):
        replies = device_replies()
        for cmd in ("RU 0", "RUL 0", "RU 1", "RUL 1"):  # pairs of timeouts, never 3 in a row
            del replies[cmd]
        clock = FakeClock()
        reading = m.poll_cycle(FakeSerial(replies, clock=clock), 1.0, no_warn, clock)
        self.assertIsNone(reading.channels[0].vmon)
        self.assertEqual(reading.channels[0].imon, -0.004)
        self.assertEqual(reading.ramp_vps, 5.0)


# --- warning rate limit -------------------------------------------------------------------------


class TestWarnLimiter(unittest.TestCase):
    def test_repeats_are_suppressed_and_counted(self):
        clock = FakeClock()
        logger = logging.getLogger("test_mhv4.limiter")
        warn = m.WarnLimiter(logger, interval_s=300.0, clock=clock)
        with self.assertLogs(logger, level="WARNING") as cm:
            self.assertTrue(warn("k", "RU 0 bad"))
            for _ in range(10):
                clock.advance(2.0)
                self.assertFalse(warn("k", "RU 0 bad"))
            self.assertTrue(warn("other", "RI 0 bad"))
            clock.advance(300.0)
            self.assertTrue(warn("k", "RU 0 bad"))
            self.assertTrue(warn("k2", "x", level=logging.ERROR))
        self.assertEqual(len(cm.output), 4)
        self.assertIn("repeated 10x", cm.output[2])
        self.assertTrue(cm.output[3].startswith("ERROR"))


# --- main loop ----------------------------------------------------------------------------------


class FakeResponse:
    def __init__(self, status=204, text=""):
        self.status_code = status
        self.ok = 200 <= status < 300
        self.text = text


class TestRun(unittest.TestCase):
    def setUp(self):
        self.cfg = m.Config(
            serial_port="/dev/fake",
            device_name="si_bias",
            influx_url="http://influx.invalid/api/v3/write_lp?db=delila",
            poll_interval_sec=0.001,
        )
        logging.getLogger("mhv4_monitor").setLevel(logging.CRITICAL)

    def tearDown(self):
        logging.getLogger("mhv4_monitor").setLevel(logging.NOTSET)

    def test_reconnects_after_open_failure_and_writes(self):
        ports = []
        attempts = []

        def opener(path):
            attempts.append(path)
            if len(attempts) == 1:
                raise FileNotFoundError(2, "No such file or directory", path)
            ports.append(FakeSerial(device_replies()))
            return ports[-1]

        stop = m.StopFlag()
        posts = []

        def post(url, data, headers, timeout):
            posts.append((url, data))
            stop.set("test")
            return FakeResponse()

        m.run(self.cfg, stop, open_port=opener, post=post, retry_delay_s=0.001)
        self.assertEqual(attempts, ["/dev/fake", "/dev/fake"])
        self.assertEqual(len(posts), 1)
        url, data = posts[0]
        self.assertEqual(url, self.cfg.influx_url)
        lines = data.decode().split("\n")
        self.assertEqual(len(lines), 5)
        self.assertTrue(lines[0].startswith("mhv4,device=si_bias,channel=0 vmon=0.0,"))
        self.assertEqual(lines[4], "mhv4_ramp,device=si_bias speed_vps=5.0")
        self.assertTrue(ports[0].closed)

    def test_serial_error_mid_cycle_closes_and_reopens(self):
        ports = []

        def opener(_path):
            fail = OSError(5, "Input/output error") if not ports else None
            ports.append(FakeSerial(device_replies(), fail_on_read=fail))
            return ports[-1]

        stop = m.StopFlag()

        def post(url, data, headers, timeout):
            stop.set("test")
            return FakeResponse()

        m.run(self.cfg, stop, open_port=opener, post=post, retry_delay_s=0.001)
        self.assertEqual(len(ports), 2)
        self.assertTrue(ports[0].closed)
        self.assertTrue(ports[1].closed)

    def test_influx_errors_do_not_stop_the_loop(self):
        stop = m.StopFlag()
        calls = []

        def post(url, data, headers, timeout):
            calls.append(data)
            if len(calls) == 1:
                raise requests.ConnectionError("connection refused")
            if len(calls) == 2:
                return FakeResponse(500, "boom")
            stop.set("test")
            return FakeResponse()

        port = FakeSerial(device_replies())
        m.run(self.cfg, stop, open_port=lambda _p: port, post=post, retry_delay_s=0.001)
        self.assertEqual(len(calls), 3)
        self.assertTrue(port.closed)

    def test_stop_flag_wait(self):
        stop = m.StopFlag()
        self.assertFalse(stop.wait(0.01))
        stop.set("SIGTERM")
        self.assertTrue(stop.wait(10.0))
        self.assertEqual(stop.reason, "SIGTERM")


# --- config -------------------------------------------------------------------------------------


class TestConfig(unittest.TestCase):
    BASE = {
        "serial_port": "/dev/serial/by-id/x",
        "device_name": "si_bias",
        "influx_url": "http://localhost:8086/api/v3/write_lp?db=delila",
    }

    def test_shipped_config(self):
        cfg = m.load_config(os.path.join(HERE, "mhv4_config.yaml"))
        self.assertEqual(
            cfg.serial_port, "/dev/serial/by-id/usb-mesytec_MHV4_V42_0915075-if00-port0"
        )
        self.assertEqual(cfg.device_name, "si_bias")
        self.assertEqual(cfg.poll_interval_sec, 2.0)
        self.assertEqual(cfg.influx_url, "http://localhost:8086/api/v3/write_lp?db=delila")
        self.assertEqual(cfg.hv_present_threshold_v, 1.0)
        self.assertEqual(cfg.reply_timeout_sec, 1.0)

    def test_defaults(self):
        cfg = m.config_from_dict(dict(self.BASE))
        self.assertEqual((cfg.poll_interval_sec, cfg.hv_present_threshold_v), (2.0, 1.0))

    def test_invalid_configs(self):
        bad = [
            None,
            [],
            {k: v for k, v in self.BASE.items() if k != "serial_port"},
            {**self.BASE, "hv_present_threshold": 5.0},  # typo must not fall back to default
            {**self.BASE, "device_name": ""},
            {**self.BASE, "device_name": 'a"b'},
            {**self.BASE, "poll_interval_sec": 0},
            {**self.BASE, "poll_interval_sec": "2"},
            {**self.BASE, "poll_interval_sec": True},
            {**self.BASE, "hv_present_threshold_v": -1.0},
            {**self.BASE, "reply_timeout_sec": float("nan")},
        ]
        for raw in bad:
            with self.assertRaises(ValueError, msg=repr(raw)):
                m.config_from_dict(raw)


if __name__ == "__main__":
    unittest.main()
