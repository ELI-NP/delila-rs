"""Unit tests for sy5527_monitor.py (pure parts + poll logic with a fake crate).

Run: python3 -m unittest discover tools/hv_monitor -p 'test_*.py'
The ctypes layer (HVConnection) was checked against the real SY5527 on pf-qaq
(2026-10-22: slots 1 and 3, A1833, 24 channels NWA01..NWB12).
"""

import pathlib
import re
import unittest

import sy5527_monitor as m


class FakeConn:
    """Duck-typed HVConnection: per-slot (names, values) or a failure."""

    def __init__(self, slots):
        self.slots = slots

    def read_slot(self, slot, nch, warn):
        names, values, failed = self.slots[slot]
        for f in failed:
            warn(f"param-{slot}-{f}", f"slot {slot}: {f} failed")
        return names, values


class LineTests(unittest.TestCase):
    def test_full_line(self):
        lines = m.build_lines(1, ["NWA01"], {
            "vmon": [1726.0], "vset": [1725.0], "imon": [369.79998779296875],
            "status": [1], "pw": [1]})
        self.assertEqual(lines, [
            "sy5527_hv,slot=1,channel=0,name=NWA01 vmon=1726.0,vset=1725.0,imon=369.8,status=1i,pw=1i"])

    def test_failed_field_is_left_out_not_zero(self):
        lines = m.build_lines(3, ["NWB01", "NWB02"], {"vmon": [1300.5, 1325.8], "pw": [1, 0]})
        self.assertEqual(lines, [
            "sy5527_hv,slot=3,channel=0,name=NWB01 vmon=1300.5,pw=1i",
            "sy5527_hv,slot=3,channel=1,name=NWB02 vmon=1325.8,pw=0i"])
        self.assertNotIn("vset", "".join(lines))

    def test_no_fields_no_lines(self):
        self.assertEqual(m.build_lines(1, ["A"], {}), [])

    def test_name_escaping_and_unnamed(self):
        lines = m.build_lines(0, ["a b,c=d", ""], {"vmon": [1.0, 2.0]})
        self.assertTrue(lines[0].startswith("sy5527_hv,slot=0,channel=0,name=a\\ b\\,c\\=d "))
        self.assertIn("name=unnamed ", lines[1])

    def test_names_missing(self):
        self.assertIn("name=unnamed ", m.build_lines(0, None, {"vmon": [1.0]})[0])


class SlotTests(unittest.TestCase):
    def test_all_populated_when_not_configured(self):
        self.assertEqual(m.select_slots([(1, 12), (3, 12)], None), [(1, 12), (3, 12)])

    def test_configured_subset_in_configured_order(self):
        warned = []
        got = m.select_slots([(1, 12), (3, 12), (5, 24)], [5, 1, 4],
                             warn=lambda k, msg: warned.append(k))
        self.assertEqual(got, [(5, 24), (1, 12)])
        self.assertEqual(warned, ["empty-slot-4"])


class PollTests(unittest.TestCase):
    def test_partial_failure_keeps_other_fields(self):
        warned = []
        conn = FakeConn({1: (["NWA01"], {"vmon": [1.0]}, ["vset"])})
        lines = m.poll_once(conn, [(1, 1)], lambda k, msg: warned.append(k))
        self.assertEqual(lines, ["sy5527_hv,slot=1,channel=0,name=NWA01 vmon=1.0"])
        self.assertEqual(warned, ["param-1-vset"])

    def test_nothing_readable_means_reconnect(self):
        conn = FakeConn({1: (None, {}, ["vmon", "vset", "imon", "status", "pw"])})
        with self.assertRaises(m.ConnectionLost):
            m.poll_once(conn, [(1, 12)], lambda k, msg: None)


class WarnLimiterTests(unittest.TestCase):
    def test_repeats_are_suppressed_within_interval(self):
        t = [0.0]
        w = m.WarnLimiter(interval_s=300.0, clock=lambda: t[0])
        with self.assertLogs("sy5527_monitor", "WARNING") as logs:
            self.assertTrue(w("k", "boom"))
            self.assertFalse(w("k", "boom"))
            t[0] = 301.0
            self.assertTrue(w("k", "boom"))
        self.assertIn("repeated 1 times", logs.output[-1])


class SafetyTests(unittest.TestCase):
    SOURCE = (pathlib.Path(__file__).parent / "sy5527_monitor.py").read_text()

    def test_reads_v0set_not_vset(self):
        params = [p for _, p, _ in m.CHANNEL_PARAMS]
        self.assertIn(b"V0Set", params)
        self.assertNotIn(b"VSet", params)

    def test_never_calls_a_set_function(self):
        # Read-only by construction: no CAENHV_Set* (SetChParam, SetBdParam,
        # SetSysProp, SetChName, ...) anywhere in the code.
        self.assertEqual(re.findall(r"CAENHV_Set\w*", self.SOURCE), [])

    def test_system_types_match_header(self):
        # CAENHVWrapper.h v7.5: SY1527=0, SY2527=1, SY4527=2, SY5527=3, R6060=16
        self.assertEqual(m.SYSTEM_TYPES["SY5527"], 3)
        self.assertEqual(m.SYSTEM_TYPES["SY4527"], 2)
        self.assertEqual(m.LINKTYPE_TCPIP, 0)


if __name__ == "__main__":
    unittest.main()
