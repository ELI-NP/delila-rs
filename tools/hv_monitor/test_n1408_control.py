"""Unit tests for n1408_control (no hardware, no pyserial needed).

Run from the repo root:
    python3 -m unittest discover tools/hv_monitor -p 'test_*.py'

Every reply here is a string the real N1408 returned (FW 1.00, SN 1592, pf-qaq, 2026-11-05,
TODO 82 §2). Tests list the replies they need; nothing models how the unit behaves.
"""

import inspect
import json
import os
import re
import tempfile
import threading
import unittest
import urllib.error
import urllib.request

import n1408_control as m

BD = "$BD:00,CMD:"


def mon(par, ch=4):
    return f"{BD}MON,CH:{ch},PAR:{par}"


def bd(par):
    return f"{BD}MON,PAR:{par}"


def set_cmd(ch, par):
    return f"{BD}SET,CH:{ch},PAR:{par}"


def ok(value=None):
    return "#BD:00,CMD:OK" + ("" if value is None else f",VAL:{value}")


# Captured replies to one full read, with channel 1 OFF (15:41, after the power cycles).
REAL = {
    bd("BDNAME"): ok("N1408"),
    bd("BDNCH"): ok("4"),
    bd("BDFREL"): ok("1.00"),
    bd("BDSNUM"): ok("1592"),
    bd("BDILKM"): ok("CLOSED"),
    bd("BDCTR"): ok("REMOTE"),
    bd("BDILK"): ok("NO"),
    bd("BDALARM"): ok("0"),
    mon("STAT"): ok("00000;00000;01024;01024"),
    mon("VMON"): ok("000.00;000.04;000.02;000.02"),
    mon("IMON"): ok("+00.0000;+00.0000;+00.0000;+00.0000"),
    mon("VSET"): ok("120.00;020.00;035.00;060.00"),
    mon("ISET"): ok("02.100;09.100;09.100;09.100"),
    mon("MAXV"): ok("200;150;180;080"),
    mon("RUP"): ok("003;005;005;005"),
    mon("RDW"): ok("003;005;005;005"),
    mon("TRIP"): ok("0010.0;0010.0;0010.0;0010.0"),
    mon("PDWN"): ok("KILL;KILL;KILL;KILL"),
    mon("POL"): ok("+;-;-;-"),
}


class ScriptedPort:
    """Replays replies: a dict value is the reply to that command; a list is used up in order
    (its last entry repeats). A command with no entry gets no reply, like a silent line."""

    def __init__(self, replies=None):
        self.replies = dict(REAL if replies is None else replies)
        self.written = []
        self.closed = False
        self._rx = []

    def reset_input_buffer(self):
        self._rx.clear()

    def write(self, data):
        cmd = bytes(data).decode("ascii")
        self.written.append(cmd)
        reply = self.replies.get(cmd.removesuffix("\r\n"))
        if isinstance(reply, list):
            reply = reply.pop(0) if len(reply) > 1 else reply[0]
        if reply is not None:
            self._rx.append(reply)
        return len(data)

    def readline(self):
        return (self._rx.pop(0) + "\r\n").encode("ascii") if self._rx else b""

    def close(self):
        self.closed = True

    def sets(self):
        return [c.strip() for c in self.written if ",CMD:SET," in c]


def make_config(**over):
    channels = {0: m.ChannelConfig("Si A", True), 1: m.ChannelConfig("Si B", True),
                2: m.ChannelConfig("spare", False), 3: m.ChannelConfig("spare", False)}
    values = dict(
        serial_port="/dev/null", board=0, expected_serial="1592", device_name="si_bias",
        poll_interval_sec=2.0, influx_url="http://x/write", http_host="127.0.0.1", http_port=0,
        passphrase_file="/nonexistent", audit_log="", control_idle_timeout_sec=600.0,
        readback_timeout_sec=5.0, channels=channels,
    )
    values.update(over)
    return m.Config(**values)


class FakeTime:
    def __init__(self):
        self.t = 0.0

    def __call__(self):
        return self.t

    def sleep(self, dt):
        self.t += dt


def execute(port, action="on", channel=1, cfg=None):
    ft = FakeTime()
    return m.execute_command(port, cfg or make_config(), action, channel, ft, ft.sleep)


# --- allowlist ----------------------------------------------------------------------------------


class TestAllowlist(unittest.TestCase):
    def test_only_set_commands_are_per_channel_on_and_off(self):
        sets = {c for c in m.allowed_commands(0) if ",CMD:SET," in c}
        self.assertEqual(
            sets, {set_cmd(c, p) for c in range(4) for p in ("ON", "OFF")}
        )

    def test_everything_else_is_a_monitor_command(self):
        for c in m.allowed_commands(0):
            self.assertTrue(",CMD:SET," in c or ",CMD:MON," in c, c)

    def assert_rejected(self, cmd):
        port = ScriptedPort()
        with self.assertRaises(m.ForbiddenCommandError, msg=cmd):
            m.send_command(port, 0, cmd)
        self.assertEqual(port.written, [], f"bytes written for {cmd!r}")

    def test_voltage_and_protection_settings_cannot_be_sent(self):
        for par in ("VSET,VAL:10", "ISET,VAL:1", "MAXV,VAL:100", "RUP,VAL:5", "RDW,VAL:5",
                    "TRIP,VAL:1", "PDWN,VAL:RAMP", "ZCADJ,VAL:EN", "ZCDTC"):
            self.assert_rejected(f"{BD}SET,CH:1,PAR:{par}")

    def test_all_channels_on_cannot_be_sent(self):
        self.assert_rejected(set_cmd(4, "ON"))
        self.assert_rejected(set_cmd(4, "OFF"))

    def test_board_settings_cannot_be_sent(self):
        self.assert_rejected(f"{BD}SET,PAR:BDILKM,VAL:OPEN")
        self.assert_rejected(f"{BD}SET,PAR:BDCLR")

    def test_malformed_strings_are_rejected(self):
        for cmd in (set_cmd(1, "ON") + "\r\n", set_cmd(1, "ON") + " ", set_cmd(1, "on"),
                    set_cmd(1, "ON") + "\r\n" + set_cmd(0, "ON"), "$BD:01,CMD:SET,CH:1,PAR:ON"):
            self.assert_rejected(cmd)

    def test_send_command_is_the_only_write_site(self):
        src = inspect.getsource(m)
        self.assertEqual(len(re.findall(r"port\.write\(", src)), 1)
        self.assertIn("port.write(", inspect.getsource(m.send_command))


# --- replies ------------------------------------------------------------------------------------


class TestReplies(unittest.TestCase):
    def test_real_values(self):
        port = ScriptedPort()
        self.assertEqual(m.query(port, 0, bd("BDNAME")), "N1408")
        self.assertEqual(m.query(port, 0, mon("POL")), "+;-;-;-")

    def test_plain_ok_has_no_value(self):
        port = ScriptedPort({set_cmd(1, "ON"): "#BD:00,CMD:OK"})
        self.assertIsNone(m.query(port, 0, set_cmd(1, "ON")))

    def test_real_error_replies(self):
        for reply, code in (("#BD:00,LOC:ERR", "LOC:ERR"), ("#BD:00,PAR:ERR", "PAR:ERR"),
                            ("#BD:00,CH:ERR", "CH:ERR"), ("#BD:00,CMD:ERR", "CMD:ERR")):
            port = ScriptedPort({bd("BDCTR"): reply})
            with self.assertRaises(m.ReplyError) as cm:
                m.query(port, 0, bd("BDCTR"))
            self.assertEqual(cm.exception.code, code)

    def test_reply_from_another_board_is_rejected(self):
        port = ScriptedPort({bd("BDCTR"): "#BD:07,CMD:OK,VAL:REMOTE"})
        with self.assertRaises(m.ReplyError):
            m.query(port, 0, bd("BDCTR"))

    def test_no_reply_is_a_silent_device(self):
        with self.assertRaises(m.DeviceSilentError):
            m.query(ScriptedPort({}), 0, bd("BDCTR"))

    def test_partial_line_is_a_silent_device(self):
        class Partial(ScriptedPort):
            def readline(self):
                return b"#BD:00,CMD:O"

        with self.assertRaises(m.DeviceSilentError):
            m.query(Partial(), 0, bd("BDCTR"))


class TestStatus(unittest.TestCase):
    def test_bits_from_the_manual(self):
        self.assertEqual(m.status_flags(0), [])
        self.assertEqual(m.status_flags(1), ["ON"])
        self.assertEqual(m.status_flags(3), ["ON", "RUP"])
        self.assertEqual(m.status_flags(5), ["ON", "RDW"])
        self.assertEqual(m.status_flags(1029), ["ON", "RDW", "DIS"])
        self.assertEqual(m.status_flags(2048), ["KILL"])

    def test_states_seen_on_the_real_unit(self):
        state = m.channel_state
        self.assertEqual(state(None, 0.0, 20.0), "UNKNOWN")
        self.assertEqual(state(0, 0.04, 20.0), "OFF")
        self.assertEqual(state(1024, 0.04, 20.0), "DISABLED")
        self.assertEqual(state(3, 4.8, 20.0), "RAMP_UP")
        self.assertEqual(state(1, 19.98, 20.0), "ON")
        self.assertEqual(state(5, 16.28, 20.0), "RAMP_DOWN")
        self.assertEqual(state(1029, 16.06, 20.0), "RAMP_DOWN")
        self.assertEqual(state(2048, 14.4, 20.0), "KILL")

    def test_the_tail_of_an_off_is_not_shown_as_on(self):
        # After SET OFF the ON bit stays set without RDW until VMON is about 0.15 V (TODO 82 §2).
        self.assertEqual(m.channel_state(1, 1.94, 20.0), "SETTLING")
        self.assertEqual(m.channel_state(1, 0.14, 20.0), "SETTLING")
        # Close to VSET it is ON; an unknown reading never turns into a claim of ON.
        self.assertEqual(m.channel_state(1, 18.0, 20.0), "ON")
        self.assertEqual(m.channel_state(1, None, 20.0), "SETTLING")
        self.assertEqual(m.channel_state(1, 19.98, None), "SETTLING")

    def test_a_real_under_voltage_stays_on_with_its_flag(self):
        self.assertEqual(m.channel_state(1 + 32, 10.0, 20.0), "ON")

    def test_faults(self):
        self.assertEqual(m.channel_state(128, 0.0, 20.0), "TRIP")
        self.assertEqual(m.channel_state(4096, 0.0, 20.0), "INTERLOCK")


class TestPollCycle(unittest.TestCase):
    def test_full_read(self):
        reading = m.poll_cycle(ScriptedPort(), 0)
        self.assertEqual(reading.board["control"], "REMOTE")
        self.assertEqual(reading.board["interlock"], "NO")
        ch1 = reading.channels[1]
        self.assertEqual(ch1["status"], 0)
        self.assertEqual(ch1["vset"], 20.0)
        self.assertEqual(ch1["imon"], 0.0)
        self.assertEqual(ch1["maxv"], 150.0)
        self.assertEqual(ch1["pdwn"], "KILL")
        self.assertEqual(ch1["polarity"], "-")
        self.assertEqual(reading.channels[0]["polarity"], "+")
        self.assertEqual(reading.channels[2]["status"], 1024)

    def test_sends_only_monitor_commands(self):
        port = ScriptedPort()
        m.poll_cycle(port, 0)
        self.assertEqual(port.sets(), [])

    def test_a_bad_reply_is_unknown_not_zero(self):
        replies = dict(REAL)
        replies[mon("VMON")] = "#BD:00,PAR:ERR"
        replies[mon("STAT")] = ok("00000;00000;nonsense;00000")
        reading = m.poll_cycle(ScriptedPort(replies), 0)
        self.assertIsNone(reading.channels[1]["vmon"])
        self.assertTrue(all(c["status"] is None for c in reading.channels))
        self.assertEqual(reading.channels[1]["vset"], 20.0)

    def test_short_list_is_unknown(self):
        replies = dict(REAL)
        replies[mon("VSET")] = ok("120.00;020.00")
        reading = m.poll_cycle(ScriptedPort(replies), 0)
        self.assertIsNone(reading.channels[0]["vset"])

    def test_silence_aborts_the_cycle(self):
        with self.assertRaises(m.DeviceSilentError):
            m.poll_cycle(ScriptedPort({}), 0)


class TestIdentify(unittest.TestCase):
    def test_right_unit(self):
        info = m.identify(ScriptedPort(), make_config())
        self.assertEqual(info["serial"], "1592")
        self.assertEqual(info["firmware"], "1.00")

    def test_wrong_serial_is_refused(self):
        with self.assertRaises(m.IdentityError):
            m.identify(ScriptedPort(), make_config(expected_serial="9999"))

    def test_wrong_model_is_refused(self):
        replies = dict(REAL)
        replies[bd("BDNAME")] = ok("N1419")
        with self.assertRaises(m.IdentityError):
            m.identify(ScriptedPort(replies), make_config())


# --- ON / OFF -----------------------------------------------------------------------------------


def with_replies(**kw):
    """REAL + the given STAT sequence for channel 1 and board values."""
    replies = dict(REAL)
    if "stat" in kw:
        replies[mon("STAT", 1)] = [ok(f"{s:05d}") for s in kw["stat"]]
    if "control" in kw:
        replies[bd("BDCTR")] = ok(kw["control"])
    if "interlock" in kw:
        replies[bd("BDILK")] = ok(kw["interlock"])
    replies[set_cmd(1, "ON")] = kw.get("set_reply", "#BD:00,CMD:OK")
    replies[set_cmd(1, "OFF")] = kw.get("set_reply", "#BD:00,CMD:OK")
    replies[mon("VSET", 1)] = ok("020.00")
    replies[mon("VMON", 1)] = ok("000.04")
    return replies


class TestExecute(unittest.TestCase):
    def test_on_as_on_the_real_unit(self):
        port = ScriptedPort(with_replies(stat=[0, 1]))
        result = execute(port, "on")
        self.assertTrue(result["ok"], result)
        self.assertEqual(result["result"], "done")
        self.assertEqual(port.sets(), [set_cmd(1, "ON")])
        self.assertEqual((result["status_before"], result["status_after"]), (0, 1))

    def test_off_as_on_the_real_unit(self):
        port = ScriptedPort(with_replies(stat=[1, 5]))
        result = execute(port, "off")
        self.assertTrue(result["ok"], result)
        self.assertEqual(port.sets(), [set_cmd(1, "OFF")])

    def test_off_that_lands_directly_on_zero_is_done(self):
        result = execute(ScriptedPort(with_replies(stat=[1, 0])), "off")
        self.assertTrue(result["ok"], result)

    def test_local_panel_refuses_before_any_set(self):
        for action in ("on", "off"):
            port = ScriptedPort(with_replies(stat=[1], control="LOCAL"))
            result = execute(port, action)
            self.assertFalse(result["ok"])
            self.assertEqual(result["result"], "refused")
            self.assertIn("LOCAL", result["message"])
            self.assertEqual(port.sets(), [])

    def test_disabled_switch_refuses_on_because_the_unit_would_say_ok(self):
        # The real unit answers CMD:OK to SET ON with the switch at OFF and does nothing.
        port = ScriptedPort(with_replies(stat=[1024]))
        result = execute(port, "on")
        self.assertEqual(result["result"], "refused")
        self.assertIn("HV_EN", result["message"])
        self.assertEqual(port.sets(), [])

    def test_kill_interlock_and_calibration_error_refuse_on(self):
        for stat in (2048, 4096, 8192):
            port = ScriptedPort(with_replies(stat=[stat]))
            self.assertEqual(execute(port, "on")["result"], "refused", stat)
            self.assertEqual(port.sets(), [])

    def test_board_interlock_refuses_on_but_not_off(self):
        port = ScriptedPort(with_replies(stat=[0], interlock="YES"))
        self.assertEqual(execute(port, "on")["result"], "refused")
        self.assertEqual(port.sets(), [])
        port = ScriptedPort(with_replies(stat=[1, 5], interlock="YES"))
        self.assertTrue(execute(port, "off")["ok"])

    def test_off_works_with_the_switch_at_off(self):
        port = ScriptedPort(with_replies(stat=[1025, 1029]))
        self.assertTrue(execute(port, "off")["ok"])

    def test_already_on_sends_nothing(self):
        port = ScriptedPort(with_replies(stat=[1]))
        result = execute(port, "on")
        self.assertTrue(result["ok"])
        self.assertEqual(result["result"], "already")
        self.assertEqual(port.sets(), [])

    def test_already_off_sends_nothing(self):
        port = ScriptedPort(with_replies(stat=[0]))
        result = execute(port, "off")
        self.assertEqual(result["result"], "already")
        self.assertEqual(port.sets(), [])

    def test_on_during_ramp_down_is_sent(self):
        # ON during RDW really reverses the ramp on the unit (15:20:50); it is not "already".
        port = ScriptedPort(with_replies(stat=[5, 3]))
        result = execute(port, "on")
        self.assertTrue(result["ok"], result)
        self.assertEqual(port.sets(), [set_cmd(1, "ON")])

    def test_ok_reply_without_effect_is_a_failure(self):
        port = ScriptedPort(with_replies(stat=[0, 0]))
        result = execute(port, "on")
        self.assertFalse(result["ok"])
        self.assertEqual(result["result"], "failed")
        self.assertIn("CMD:OK", result["message"])

    def test_error_reply_to_set_is_a_failure_with_its_code(self):
        port = ScriptedPort(with_replies(stat=[0], set_reply="#BD:00,LOC:ERR"))
        result = execute(port, "on")
        self.assertEqual(result["result"], "failed")
        self.assertIn("LOC:ERR", result["message"])

    def test_channel_not_allowed_touches_nothing(self):
        port = ScriptedPort()
        result = execute(port, "on", channel=2)
        self.assertEqual(result["result"], "refused")
        self.assertEqual(port.written, [])

    def test_unknown_channel_and_action(self):
        port = ScriptedPort()
        self.assertEqual(execute(port, "on", channel=4)["result"], "refused")
        self.assertEqual(execute(port, "toggle")["result"], "refused")
        self.assertEqual(port.written, [])

    def test_only_the_requested_set_is_ever_sent(self):
        port = ScriptedPort(with_replies(stat=[0, 1]))
        execute(port, "on")
        self.assertEqual(len(port.sets()), 1)


class TestStatusChanges(unittest.TestCase):
    def test_no_events_without_a_previous_reading(self):
        self.assertEqual(m.status_changes(None, [0, 0, 0, 0]), [])

    def test_trip_after_being_on(self):
        events = m.status_changes([1, 0, 0, 0], [128, 0, 0, 0])
        self.assertEqual(len(events), 1)
        self.assertEqual(events[0]["channel"], 0)
        self.assertEqual(events[0]["gone"], ["ON"])
        self.assertEqual(events[0]["new"], ["TRIP"])

    def test_ramp_bits_are_not_events(self):
        self.assertEqual(m.status_changes([1, 0, 0, 0], [3, 0, 0, 0]), [])
        self.assertEqual(m.status_changes([3, 0, 0, 0], [1, 0, 0, 0]), [])

    def test_switch_moved(self):
        events = m.status_changes([0, 0, 0, 0], [0, 1024, 0, 0])
        self.assertEqual(events[0]["channel"], 1)
        self.assertEqual(events[0]["new"], ["DIS"])

    def test_unknown_reading_is_not_a_change(self):
        self.assertEqual(m.status_changes([1, 0, 0, 0], [None, 0, 0, 0]), [])


# --- operator session ---------------------------------------------------------------------------


class TestSession(unittest.TestCase):
    def setUp(self):
        self.clock = FakeTime()
        self.session = m.ControlSession(600.0, self.clock)

    def test_acquire_gives_a_token_for_the_operator(self):
        token, preempted = self.session.acquire("aogaki")
        self.assertIsNone(preempted)
        self.assertEqual(self.session.authorize(token), "aogaki")
        self.assertEqual(self.session.holder(), "aogaki")

    def test_acquire_preempts_and_names_the_previous_holder(self):
        first, _ = self.session.acquire("aogaki")
        second, preempted = self.session.acquire("student")
        self.assertEqual(preempted, "aogaki")
        self.assertIsNone(self.session.authorize(first))
        self.assertEqual(self.session.authorize(second), "student")

    def test_tokens_differ(self):
        self.assertNotEqual(self.session.acquire("a")[0], self.session.acquire("a")[0])

    def test_release(self):
        token, _ = self.session.acquire("aogaki")
        self.assertTrue(self.session.release(token))
        self.assertIsNone(self.session.authorize(token))
        self.assertIsNone(self.session.holder())

    def test_release_with_a_stale_token_does_nothing(self):
        token, _ = self.session.acquire("aogaki")
        self.assertFalse(self.session.release("stale"))
        self.assertEqual(self.session.authorize(token), "aogaki")

    def test_idle_timeout(self):
        token, _ = self.session.acquire("aogaki")
        self.clock.t += 599
        self.assertEqual(self.session.authorize(token), "aogaki")  # use renews it
        self.clock.t += 599
        self.assertEqual(self.session.authorize(token), "aogaki")
        self.clock.t += 601
        self.assertIsNone(self.session.authorize(token))
        self.assertIsNone(self.session.holder())


class TestPassphraseFile(unittest.TestCase):
    def test_missing_file_means_control_is_off(self):
        self.assertIsNone(m.read_passphrase("/nonexistent/file"))

    def test_empty_file_means_control_is_off(self):
        with tempfile.NamedTemporaryFile("w") as f:
            f.write("  \n")
            f.flush()
            self.assertIsNone(m.read_passphrase(f.name))

    def test_trailing_newline_is_not_part_of_it(self):
        with tempfile.NamedTemporaryFile("w") as f:
            f.write("not a secret\n")
            f.flush()
            self.assertEqual(m.read_passphrase(f.name), "not a secret")


# --- influx -------------------------------------------------------------------------------------


class TestInflux(unittest.TestCase):
    def test_lines(self):
        reading = m.poll_cycle(ScriptedPort(), 0)
        lines = m.format_lines("si_bias", reading)
        self.assertEqual(len(lines), 4)
        self.assertTrue(lines[1].startswith("n1408,device=si_bias,channel=1 "))
        self.assertIn("vmon=0.04", lines[1])
        self.assertIn("vset=20.0", lines[1])
        self.assertIn("status=0i", lines[1])
        self.assertIn("output_on=false", lines[1])

    def test_failed_reads_are_omitted(self):
        replies = dict(REAL)
        replies[mon("VMON")] = "#BD:00,PAR:ERR"
        replies[mon("STAT")] = "#BD:00,PAR:ERR"
        lines = m.format_lines("si_bias", m.poll_cycle(ScriptedPort(replies), 0))
        self.assertNotIn("vmon=", lines[0])
        self.assertNotIn("status=", lines[0])
        self.assertNotIn("output_on=", lines[0])
        self.assertIn("vset=", lines[0])


# --- config -------------------------------------------------------------------------------------


class TestConfig(unittest.TestCase):
    def raw(self, **over):
        raw = {"serial_port": "/dev/x", "expected_serial": 1592, "device_name": "si_bias",
               "influx_url": "http://x", "passphrase_file": "/p", "audit_log": "/a",
               "channels": {0: {"name": "ch0"}, 1: {"name": "ch1", "control": True},
                            2: {"name": "ch2"}, 3: {"name": "ch3"}}}
        raw.update(over)
        return raw

    def test_defaults(self):
        cfg = m.config_from_dict(self.raw())
        self.assertEqual(cfg.expected_serial, "1592")
        self.assertFalse(cfg.channels[0].control)
        self.assertTrue(cfg.channels[1].control)

    def test_the_repo_config_allows_no_channel_and_holds_no_passphrase(self):
        here = os.path.dirname(os.path.abspath(__file__))
        cfg = m.load_config(os.path.join(here, "n1408_control_config.yaml"))
        self.assertEqual(cfg.expected_serial, "1592")
        self.assertTrue(cfg.passphrase_file.startswith("/"))
        self.assertFalse(any(c.control for c in cfg.channels.values()))
        with open(os.path.join(here, "n1408_control_config.yaml")) as f:
            self.assertNotRegex(f.read(), r"(?m)^passphrase:")

    def test_missing_key_is_an_error(self):
        raw = self.raw()
        del raw["expected_serial"]
        with self.assertRaises(ValueError):
            m.config_from_dict(raw)

    def test_all_four_channels_are_required(self):
        with self.assertRaises(ValueError):
            m.config_from_dict(self.raw(channels={0: {"name": "only"}}))


# --- the running service: worker + HTTP ----------------------------------------------------------


class Reply:
    ok = True
    status_code = 204
    text = ""


class TestService(unittest.TestCase):
    """The real Worker thread and HTTP server, on a scripted port and a loopback socket."""

    def start(self, replies, passphrase="not a secret", **cfg_over):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        pass_file = os.path.join(self.dir.name, "pass")
        if passphrase is not None:
            with open(pass_file, "w") as f:
                print(passphrase, file=f)
        self.audit_path = os.path.join(self.dir.name, "audit", "n1408.jsonl")
        cfg = make_config(passphrase_file=pass_file, audit_log=self.audit_path,
                          poll_interval_sec=0.05, readback_timeout_sec=1.0, **cfg_over)
        self.port = ScriptedPort(replies)
        self.posts = []

        def post(url, data=None, headers=None, timeout=None):
            self.posts.append(data)
            return Reply()

        self.app = m.App(cfg, open_port=lambda _cfg: self.port, post=post)
        self.app.start()
        self.addCleanup(self.app.stop)
        self.url = f"http://127.0.0.1:{self.app.http_port}"
        self.wait_for(lambda s: s["connected"])

    def wait_for(self, cond, timeout=5.0):
        event = threading.Event()
        for _ in range(int(timeout / 0.02)):
            status = self.get("/api/status")
            if cond(status):
                return status
            event.wait(0.02)
        self.fail(f"condition not met: {status}")

    def get(self, path):
        with urllib.request.urlopen(self.url + path, timeout=5) as r:
            return json.load(r)

    def post(self, path, body):
        req = urllib.request.Request(self.url + path, data=json.dumps(body).encode(),
                                     headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=10) as r:
                return r.status, json.load(r)
        except urllib.error.HTTPError as e:
            return e.code, json.load(e)

    def acquire(self):
        code, body = self.post("/api/control/acquire",
                               {"operator": "aogaki", "passphrase": "not a secret"})
        self.assertEqual(code, 200, body)
        return body["token"]

    def audit(self):
        with open(self.audit_path) as f:
            return [json.loads(line) for line in f]

    def test_status_is_open_and_complete(self):
        self.start(dict(REAL))
        status = self.get("/api/status")
        self.assertTrue(status["connected"])
        self.assertEqual(status["board"]["serial"], "1592")
        self.assertEqual(status["board"]["control"], "REMOTE")
        ch = status["channels"][1]
        self.assertEqual((ch["name"], ch["state"], ch["control_allowed"]), ("Si B", "OFF", True))
        self.assertEqual(status["channels"][2]["state"], "DISABLED")
        self.assertFalse(status["channels"][2]["control_allowed"])
        self.assertTrue(status["control_enabled"])
        self.assertIsNone(status["holder"])

    def test_page_is_served(self):
        self.start(dict(REAL))
        with urllib.request.urlopen(self.url + "/", timeout=5) as r:
            self.assertIn(b"N1408", r.read())

    def test_on_needs_a_token(self):
        self.start(with_replies(stat=[0, 1]))
        code, body = self.post("/api/channel/1/on", {"token": "wrong"})
        self.assertEqual(code, 401, body)
        self.assertEqual(self.port.sets(), [])

    def test_wrong_passphrase(self):
        self.start(dict(REAL))
        code, body = self.post("/api/control/acquire", {"operator": "x", "passphrase": "guess"})
        self.assertEqual(code, 403, body)
        rows = self.audit()
        self.assertEqual(rows[-1]["action"], "acquire")
        self.assertEqual(rows[-1]["result"], "refused")
        self.assertNotIn("guess", json.dumps(rows))

    def test_operator_name_is_required(self):
        self.start(dict(REAL))
        code, _ = self.post("/api/control/acquire", {"operator": " ", "passphrase": "not a secret"})
        self.assertEqual(code, 400)

    def test_no_passphrase_file_means_read_only(self):
        self.start(dict(REAL), passphrase=None)
        self.assertFalse(self.get("/api/status")["control_enabled"])
        code, _ = self.post("/api/control/acquire", {"operator": "x", "passphrase": ""})
        self.assertEqual(code, 503)
        code, _ = self.post("/api/control/acquire", {"operator": "x", "passphrase": "not a secret"})
        self.assertEqual(code, 503)

    def test_on_end_to_end_with_audit(self):
        self.start(with_replies(stat=[0, 1]))
        code, body = self.post("/api/channel/1/on", {"token": self.acquire()})
        self.assertEqual(code, 200, body)
        self.assertTrue(body["ok"])
        self.assertEqual(self.port.sets(), [set_cmd(1, "ON")])
        row = self.audit()[-1]
        self.assertEqual((row["action"], row["channel"], row["operator"]), ("on", 1, "aogaki"))
        self.assertEqual(row["result"], "done")
        self.assertEqual((row["status_before"], row["status_after"]), (0, 1))
        self.assertTrue(any(b"n1408_event" in p for p in self.posts))

    def test_refused_on_is_409_and_audited(self):
        self.start(with_replies(stat=[1024]))
        code, body = self.post("/api/channel/1/on", {"token": self.acquire()})
        self.assertEqual(code, 409, body)
        self.assertEqual(self.port.sets(), [])
        self.assertEqual(self.audit()[-1]["result"], "refused")

    def test_set_without_effect_is_502(self):
        self.start(with_replies(stat=[0, 0]))
        code, body = self.post("/api/channel/1/on", {"token": self.acquire()})
        self.assertEqual(code, 502, body)
        self.assertEqual(self.audit()[-1]["result"], "failed")

    def test_channel_not_in_control_config(self):
        self.start(dict(REAL))
        code, _ = self.post("/api/channel/2/on", {"token": self.acquire()})
        self.assertEqual(code, 409)
        self.assertEqual(self.port.sets(), [])

    def test_bad_paths(self):
        self.start(dict(REAL))
        token = self.acquire()
        for path in ("/api/channel/9/on", "/api/channel/1/toggle", "/api/channel/x/on", "/nope"):
            code, _ = self.post(path, {"token": token})
            self.assertIn(code, (404, 409), path)
        self.assertEqual(self.port.sets(), [])

    def test_all_off_goes_through_the_allowed_channels_only(self):
        replies = with_replies(stat=[1, 5])
        replies[mon("STAT", 0)] = [ok("00001"), ok("00005")]
        replies[mon("VSET", 0)] = ok("120.00")
        replies[mon("VMON", 0)] = ok("120.00")
        replies[set_cmd(0, "OFF")] = "#BD:00,CMD:OK"
        self.start(replies)
        code, body = self.post("/api/all_off", {"token": self.acquire()})
        self.assertEqual(code, 200, body)
        self.assertTrue(body["ok"])
        self.assertEqual(self.port.sets(), [set_cmd(0, "OFF"), set_cmd(1, "OFF")])
        self.assertEqual([r["channel"] for r in body["results"]], [0, 1])

    def test_all_off_reports_a_partial_failure(self):
        replies = with_replies(stat=[1, 1])  # channel 1 stays ON without RDW
        replies[mon("STAT", 0)] = [ok("00001"), ok("00005")]
        replies[mon("VSET", 0)] = ok("120.00")
        replies[mon("VMON", 0)] = ok("120.00")
        replies[set_cmd(0, "OFF")] = "#BD:00,CMD:OK"
        self.start(replies)
        code, body = self.post("/api/all_off", {"token": self.acquire()})
        self.assertEqual(code, 502, body)
        self.assertFalse(body["ok"])
        self.assertEqual([r["ok"] for r in body["results"]], [True, False])

    def test_values_reach_influx(self):
        self.start(dict(REAL))
        self.wait_for(lambda s: self.posts)
        self.assertTrue(any(b"n1408,device=si_bias,channel=1 " in p for p in self.posts))

    def test_status_change_is_audited_without_an_operator(self):
        replies = dict(REAL)
        replies[mon("STAT")] = [ok("00001;00000;01024;01024"), ok("00128;00000;01024;01024")]
        self.start(replies)
        self.wait_for(lambda s: s["channels"][0]["state"] == "TRIP")
        rows = [r for r in self.audit() if r["action"] == "status_change"]
        self.assertEqual(rows[-1]["channel"], 0)
        self.assertEqual(rows[-1]["new"], ["TRIP"])
        self.assertIsNone(rows[-1]["operator"])

    def test_silent_unit_shows_unknown_and_recovers(self):
        self.start(dict(REAL), retry_delay_sec=0.05)
        good = self.port.replies
        self.port.replies = {}
        status = self.wait_for(lambda s: not s["connected"])
        self.assertTrue(all(c["state"] == "UNKNOWN" for c in status["channels"]))
        self.assertIsNotNone(status["error"])
        code, body = self.post("/api/channel/1/on", {"token": self.acquire()})
        self.assertEqual(code, 503, body)
        self.assertEqual(self.port.sets(), [])
        self.port.replies = good
        self.wait_for(lambda s: s["connected"])

    def test_wrong_unit_is_not_polled_or_controlled(self):
        self.start_wrong_unit()

    def start_wrong_unit(self):
        replies = dict(REAL)
        replies[bd("BDSNUM")] = ok("9999")
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        cfg = make_config(audit_log=os.path.join(self.dir.name, "a.jsonl"), retry_delay_sec=0.05,
                          poll_interval_sec=0.05)
        self.port = ScriptedPort(replies)
        self.app = m.App(cfg, open_port=lambda _p: self.port, post=lambda *a, **k: Reply())
        self.app.start()
        self.addCleanup(self.app.stop)
        self.url = f"http://127.0.0.1:{self.app.http_port}"
        status = self.wait_for(lambda s: s["error"] is not None)
        self.assertFalse(status["connected"])
        self.assertIn("9999", status["error"])
        self.assertEqual(self.port.sets(), [])


if __name__ == "__main__":
    unittest.main()
