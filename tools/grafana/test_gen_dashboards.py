"""Tests for gen_dashboards.py. Run: python3 -m unittest discover tools/grafana"""

import contextlib
import io
import json
import re
import tempfile
import unittest
from pathlib import Path

import gen_dashboards as g

REPO = Path(__file__).resolve().parents[2]
DASH_DIR = REPO / "docker/grafana/provisioning/dashboards/json"

MEASUREMENTS = r"(?:system_rate|channel_rate|backlog|disk|run_event)"
SCAN = re.compile(rf"\b(?:FROM|JOIN)\s+{MEASUREMENTS}\b")
# A scan directly followed by a time bound on the dashboard's time range
# (InfluxDB 3 Core refuses queries over > 432 parquet files).
BOUNDED_SCAN = re.compile(
    rf"\bFROM\s+{MEASUREMENTS}\s+WHERE\s+time\s*"
    r"(?:>=\s*\$__timeFrom|>\s*\$__timeTo - interval '[^']+')\s+AND\s+time\s*<=\s*\$__timeTo"
)


def boards_fixture():
    return [
        g.Board(module=0, tag="0", name="PHA1-Si0", num_channels=16),
        g.Board(module=1, tag="1", name="PSD2-LS", num_channels=32, disabled=frozenset({30, 31})),
    ]


def all_sql(dash):
    out = [p_t["rawSql"] for p in dash["panels"] for p_t in p.get("targets", [])]
    out += [a["target"]["rawSql"] for a in dash["annotations"]["list"] if "target" in a]
    return out


class AssertBounded:
    def assertBounded(self, sql):
        scans = len(SCAN.findall(sql))
        self.assertGreater(scans, 0, sql)
        self.assertEqual(scans, len(BOUNDED_SCAN.findall(sql)), f"unbounded scan in:\n{sql}")


class ParseMapTest(unittest.TestCase):
    def test_comments_blank_lines_tabs_and_extra_columns(self):
        text = (
            "# module channel name group\n"
            "\n"
            "0  0  dE_Ri00  dE_inner\n"
            "0\t1\tdE_Ri01\tdE_inner   # trailing comment\n"
            "   # indented comment\n"
            "1 5 LS06 LS 12.5 0.98 extra/analysis.columns\n"
            "1 6 LS07 LS#comment glued to the group\n"
        )
        entries = g.parse_map(text, boards_fixture(), "m.txt")
        self.assertEqual(
            [(e.module, e.channel, e.name, e.group, e.line) for e in entries],
            [(0, 0, "dE_Ri00", "dE_inner", 3), (0, 1, "dE_Ri01", "dE_inner", 4),
             (1, 5, "LS06", "LS", 6), (1, 6, "LS07", "LS", 7)],
        )

    def test_empty_map(self):
        self.assertEqual(g.parse_map("# nothing yet\n\n", boards_fixture()), [])

    def assertMapErrors(self, text, *expected):
        with self.assertRaises(g.GenError) as cm:
            g.parse_map(text, boards_fixture(), "m.txt")
        msg = str(cm.exception)
        for part in expected:
            self.assertIn(part, msg)
        return msg

    def test_all_errors_reported_with_line_numbers(self):
        text = (
            "0 0 a g\n"
            "x 1 b g\n"         # 2: module not an integer
            "0 1.5 c g\n"       # 3: channel not an integer
            "-1 2 d g\n"        # 4: negative
            "0 0 e g\n"         # 5: duplicate (module, channel)
            "0 3 a g\n"         # 6: duplicate name
            "7 0 f g\n"         # 7: unknown module
            "0 16 h g\n"        # 8: channel out of range (16 ch)
            "0 4 only_three\n"  # 9: too few columns
        )
        msg = self.assertMapErrors(
            text,
            "m.txt:2: module 'x'", "m.txt:3: channel '1.5'", "m.txt:4: module '-1'",
            "m.txt:5: module 0 channel 0 already listed on line 1",
            "m.txt:6: name 'a' already used on line 1",
            "m.txt:7: module 7 is not in the DAQ config",
            "m.txt:8: channel 16 out of range", "m.txt:9: expected 'module channel name group'",
        )
        self.assertEqual(len(msg.splitlines()), 8)

    def test_charset(self):
        ok = g.parse_map("0 0 Si.ring+1/a-b_c dE-1/2\n", boards_fixture())
        self.assertEqual(ok[0].name, "Si.ring+1/a-b_c")
        for bad in ("dE'1", 'dE"1', "dE;1", "dE,1", "dE\\1", "dEµ", "dE(1)"):
            with self.subTest(bad=bad):
                self.assertMapErrors(f"0 0 {bad} g\n", "m.txt:1: name")
                self.assertMapErrors(f"0 0 n {bad}\n", "m.txt:1: group")

    def test_user_name_must_not_take_a_default_name(self):
        self.assertMapErrors("0 0 M0/Ch3 g\n", "m.txt:1: name 'M0/Ch3' is the default name of module 0 channel 3")
        # Fine when that channel has its own name, or for its own channel.
        g.parse_map("0 0 M0/Ch3 g\n0 3 other g\n", boards_fixture())
        g.parse_map("0 3 M0/Ch3 g\n", boards_fixture())


class ChannelsTest(unittest.TestCase):
    def test_defaults_for_unlisted_channels(self):
        boards = boards_fixture()
        entries = g.parse_map("1 0 LS01 LS\n0 2 dE_Ri02 dE_inner\n", boards)
        chans = g.build_channels(boards, entries)
        self.assertEqual(len(chans), 48)
        by_key = {(c.board.module, c.channel): c for c in chans}
        self.assertEqual((by_key[0, 2].name, by_key[0, 2].group), ("dE_Ri02", "dE_inner"))
        self.assertEqual((by_key[0, 3].name, by_key[0, 3].group), ("M0/Ch3", "M0"))
        self.assertEqual((by_key[1, 31].name, by_key[1, 31].group), ("M1/Ch31", "M1"))
        self.assertFalse(by_key[1, 31].enabled)

    def test_groups_in_map_order_then_defaults_without_unused_channels(self):
        boards = boards_fixture()
        # ch 30 of module 1 is disabled but named: kept. ch 31 disabled + unlisted: unused.
        entries = g.parse_map("1 0 LS01 LS\n0 2 dE_Ri02 dE_inner\n1 30 LS31 LS\n", boards)
        groups = g.group_channels(g.build_channels(boards, entries), entries)
        self.assertEqual(list(groups), ["LS", "dE_inner", "M0", "M1"])
        self.assertEqual([c.name for c in groups["LS"]], ["LS01", "LS31"])
        self.assertEqual(len(groups["M0"]), 15)
        self.assertEqual(len(groups["M1"]), 29)  # 32 - LS01 - LS31 - unused ch31
        self.assertNotIn("M1/Ch31", [c.name for c in groups["M1"]])

    def test_slug(self):
        self.assertEqual(g.slug("X730_ThGEM_Test"), "x730-thgem-test")
        self.assertEqual(g.slug("N_Wall"), "n-wall")
        self.assertLessEqual(len("rates-" + g.slug("A" * 80)), 40)  # Grafana uid limit
        with self.assertRaises(g.GenError):
            g.slug("___")


class SqlTest(AssertBounded, unittest.TestCase):
    def setUp(self):
        self.boards = boards_fixture()
        self.entries = g.parse_map("0 0 dE_Ri00 dE_inner\n0 3 dE_Ri03 dE_inner\n1 5 LS06 LS\n", self.boards)
        self.chans = g.build_channels(self.boards, self.entries)
        self.groups = g.group_channels(self.chans, self.entries)

    def test_group_sql_maps_names_and_filters_its_channels(self):
        sql = g.group_rate_sql(self.groups["dE_inner"])
        self.assertIn("WHEN module = '0' AND channel = '0' THEN 'dE_Ri00'", sql)
        self.assertIn("WHEN module = '0' AND channel = '3' THEN 'dE_Ri03'", sql)
        self.assertIn("(module = '0' AND channel IN ('0', '3'))", sql)
        self.assertNotIn("LS06", sql)
        self.assertIn("AS name", sql)
        self.assertBounded(sql)
        default = g.group_rate_sql(self.groups["M0"])
        self.assertIn("WHEN module = '0' AND channel = '1' THEN 'M0/Ch1'", default)
        self.assertNotIn("channel = '3'", default)

    def test_module_id_maps_to_the_source_id_tag(self):
        # The Operator tags InfluxDB rows with the source id; the map uses module_id.
        b = g.Board(module=5, tag="2", name="B", num_channels=2)
        e = g.parse_map("5 1 X g\n", [b])
        sql = g.group_rate_sql(g.build_channels([b], e)[1:])
        self.assertIn("WHEN module = '2' AND channel = '1' THEN 'X'", sql)
        self.assertIn("WHEN '2' THEN 'M5 B'", g.module_rate_sql([b]))

    def test_grid_sql(self):
        sql = g.grid_sql(self.boards, self.chans)
        self.assertBounded(sql)
        self.assertIn("time > $__timeTo - interval '10 seconds' AND time <= $__timeTo", sql)
        for i in range(32):
            self.assertIn(f'AS "{i}"', sql)
        self.assertNotIn('AS "32"', sql)
        self.assertIn("('0', '15', true)", sql)
        self.assertNotIn("('0', '16',", sql)  # module 0 has 16 channels
        self.assertIn("('1', '31', false)", sql)  # disabled
        self.assertIn("(0, '0', 'M0 PHA1-Si0'), (1, '1', 'M1 PSD2-LS')", sql)

    def test_silent_sql_lists_enabled_channels_only(self):
        sql = g.silent_sql(self.chans)
        self.assertBounded(sql)
        self.assertIn("interval '1 minute'", sql)
        self.assertIn("('1', '5', 1, 5, 'LS06', 'LS')", sql)
        self.assertNotIn("'M1/Ch31'", sql)
        self.assertIn("COALESCE(r.rate, 0) = 0", sql)

    def test_literals_are_escaped(self):
        self.assertEqual(g.lit("it's"), "'it''s'")
        b = g.Board(module=0, tag="0", name="Bob's board", num_channels=1)
        self.assertIn("'M0 Bob''s board'", g.module_rate_sql([b]))
        self.assertIn("exp = 'O''Brien'", g.run_event_sql("start", "O'Brien"))

    def test_run_event_sql(self):
        sql = g.run_event_sql("stop", "N_Wall")
        self.assertBounded(sql)
        self.assertIn("exp = 'N_Wall' AND event = 'stop'", sql)
        self.assertIn("AS text", sql)
        self.assertNotIn("exp =", g.run_event_sql("start", None))


class DashboardTest(AssertBounded, unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / "config/digitizers").mkdir(parents=True)
        for i, (nch, disabled) in enumerate([(16, ()), (32, (30, 31))]):
            dig = {"num_channels": nch, "channel_defaults": {"enabled": "True"},
                   "channel_overrides": {str(c): {"enabled": "False"} for c in disabled}}
            (self.root / f"config/digitizers/d{i}.json").write_text(json.dumps(dig))
        self.toml = self.root / "config/daq.toml"
        self.toml.write_text(
            '[operator]\nexperiment_name = "N_Wall"\n\n[network]\n'
            '[[network.sources]]\nid = 0\nname = "PHA1-Si0"\nconfig_file = "config/digitizers/d0.json"\n'
            '[[network.sources]]\nid = 1\nname = "PSD2-LS"\nconfig_file = "config/digitizers/d1.json"\n'
            '#[[network.sources]]\n#id = 2\n'
        )
        self.map = self.root / "map.txt"
        self.map.write_text("# m c name group\n0 0 dE_Ri00 dE_inner\n1 0 LS01 LS\n1 1 LS02 LS\n")
        self.out = self.root / "out"

    def tearDown(self):
        self.tmp.cleanup()

    def run_main(self, *args):
        err, out = io.StringIO(), io.StringIO()
        with contextlib.redirect_stderr(err), contextlib.redirect_stdout(out):
            rc = g.main(["--config", str(self.toml), "--out", str(self.out), *args])
        return rc, out.getvalue(), err.getvalue()

    def test_load_config_resolves_json_from_the_toml_parent_parent(self):
        exp, boards = g.load_config(self.toml)
        self.assertEqual(exp, "N_Wall")
        self.assertEqual([(b.module, b.tag, b.name, b.num_channels) for b in boards],
                         [(0, "0", "PHA1-Si0", 16), (1, "1", "PSD2-LS", 32)])
        self.assertEqual(boards[1].disabled, frozenset({30, 31}))

    def test_load_config_module_id(self):
        self.toml.write_text(self.toml.read_text().replace("id = 1\n", "id = 1\nmodule_id = 7\n"))
        _, boards = g.load_config(self.toml)
        self.assertEqual((boards[1].module, boards[1].tag), (7, "1"))

    def test_load_config_errors(self):
        self.toml.write_text(self.toml.read_text().replace("d1.json", "missing.json"))
        with self.assertRaisesRegex(g.GenError, r"source id=1 \(PSD2-LS\).*'config/digitizers/missing.json' not found"):
            g.load_config(self.toml)
        self.toml.write_text('[operator]\nexperiment_name = "x"\n[[network.sources]]\nid = 0\n')
        with self.assertRaisesRegex(g.GenError, "has no config_file"):
            g.load_config(self.toml)
        self.toml.write_text("[operator]\n")
        with self.assertRaisesRegex(g.GenError, "experiment_name is missing"):
            g.load_config(self.toml)

    def test_generated_dashboard(self):
        rc, out, err = self.run_main("--map", str(self.map))
        self.assertEqual(rc, 0, err)
        path = self.out / "rates_n-wall.json"
        self.assertIn(str(path), out)
        dash = json.loads(path.read_text())
        self.assertEqual(dash["uid"], "rates-n-wall")
        self.assertEqual(dash["title"], "Rates — N_Wall")
        self.assertEqual(dash["time"], {"from": "now-30m", "to": "now"})
        self.assertIn(dash["refresh"], ("5s", "10s"))

        panels = dash["panels"]
        types = [(p["type"], p["title"]) for p in panels]
        self.assertEqual(types[0][0], "table")  # module x channel grid
        self.assertEqual(panels[0]["fieldConfig"]["defaults"]["custom"]["cellOptions"]["type"], "color-background")
        self.assertEqual(panels[0]["fieldConfig"]["defaults"]["thresholds"]["steps"][0]["color"], "red")
        self.assertIn(("timeseries", "Event rate per module (Hz)"), types)
        self.assertTrue(any(t == "table" and title.startswith("Silent channels") for t, title in types))
        group_titles = [title for t, title in types[3:] if t == "timeseries"]
        self.assertEqual(group_titles, ["dE_inner (1 ch)", "LS (2 ch)", "M0 (15 ch)", "M1 (28 ch)"])
        for p in panels:
            if p["type"] == "timeseries":
                self.assertTrue(p["fieldConfig"]["defaults"]["displayName"].startswith("${__field.labels."))

        ids = [p["id"] for p in panels]
        self.assertEqual(len(ids), len(set(ids)))
        cells = set()
        for p in panels:
            gp = p["gridPos"]
            self.assertLessEqual(gp["x"] + gp["w"], 24)
            for x in range(gp["x"], gp["x"] + gp["w"]):
                for y in range(gp["y"], gp["y"] + gp["h"]):
                    self.assertNotIn((x, y), cells, f"panel {p['title']} overlaps")
                    cells.add((x, y))

        for p in panels:
            for t in p.get("targets", []):
                self.assertEqual(t["datasource"], {"type": "influxdb", "uid": g.DEFAULT_DS_UID})
        for sql in all_sql(dash):
            self.assertBounded(sql)

        annos = [a for a in dash["annotations"]["list"] if "target" in a]
        self.assertEqual([a["name"] for a in annos], ["Run start", "Run stop"])
        for a in annos:
            self.assertIn("FROM run_event", a["target"]["rawSql"])
            self.assertIn("exp = 'N_Wall'", a["target"]["rawSql"])
            # Without `query`, Grafana's InfluxDB plugin turns it into an InfluxQL annotation.
            self.assertEqual(a["target"]["query"], a["target"]["rawSql"])

    def test_bad_map_fails_with_line_numbers(self):
        self.map.write_text("0 0 ok g\n0 99 bad g\n")
        rc, _, err = self.run_main("--map", str(self.map))
        self.assertEqual(rc, 1)
        self.assertIn(":2: channel 99 out of range", err)
        self.assertFalse((self.out / "rates_n-wall.json").exists())

    def test_explicit_map_must_exist(self):
        rc, _, err = self.run_main("--map", str(self.root / "nope.txt"))
        self.assertEqual(rc, 1)
        self.assertIn("no such file", err)

    def test_regeneration_renames_in_place(self):
        self.assertEqual(self.run_main("--map", str(self.map))[0], 0)
        self.map.write_text(self.map.read_text().replace("LS01", "LS_left"))
        self.assertEqual(self.run_main("--map", str(self.map))[0], 0)
        self.assertEqual([p.name for p in self.out.iterdir()], ["rates_n-wall.json"])
        text = (self.out / "rates_n-wall.json").read_text()
        self.assertIn("LS_left", text)
        self.assertNotIn("LS01", text)


class CommittedFilesTest(AssertBounded, unittest.TestCase):
    def test_overview_follows_the_time_picker(self):
        dash = json.loads((DASH_DIR / "delila_overview.json").read_text())
        self.assertEqual(dash["uid"], "delila-overview")
        self.assertFalse(any("Beam" in p["title"] for p in dash["panels"]))
        sqls = all_sql(dash)
        self.assertTrue(any("FROM run_event" in s for s in sqls))
        for sql in sqls:
            self.assertNotIn("now()", sql)
            self.assertBounded(sql)

    def test_fission_beam_panels_kept(self):
        dash = json.loads((DASH_DIR / "fission2026_beam.json").read_text())
        self.assertEqual(dash["uid"], "fission2026-beam")
        self.assertEqual([p["title"] for p in dash["panels"]], ["Beam Current", "Beam Current History"])

    def test_template_maps_parse(self):
        for exp in ("N_Wall", "X730_ThGEM_Test"):
            text = (REPO / f"config/channel_map_{exp}.txt").read_text()
            self.assertEqual(g.parse_map(text, boards_fixture()), [], exp)

    def test_generated_dir_is_gitignored(self):
        text = (DASH_DIR / "generated/.gitignore").read_text()
        self.assertIn("*", text.splitlines())


if __name__ == "__main__":
    unittest.main()
