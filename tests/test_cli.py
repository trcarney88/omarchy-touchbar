"""Exercise the real CLI in an isolated runtime directory; never inject live data."""
import json
import os
from pathlib import Path
import stat
import struct
import subprocess
import tempfile
import unittest

BINARY = Path(__file__).resolve().parents[1] / "build/touchmeter"


class Cli(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.env = {**os.environ, "XDG_RUNTIME_DIR": str(self.root), "XDG_CONFIG_HOME": str(self.root / "config"), "OMARCHY_PATH": str(self.root / "no-collectors")}
        bindir = self.root / "bin"
        bindir.mkdir()
        wpctl = bindir / "wpctl"
        wpctl.write_text("#!/usr/bin/python3\nfrom pathlib import Path\nPath(__file__).parents[1].joinpath('audio-called').touch()\nraise SystemExit(1)\n")
        wpctl.chmod(0o700)
        self.env["PATH"] = str(bindir) + os.pathsep + self.env.get("PATH", "")
        self.env.pop("DISPLAY", None)
        self.env.pop("WAYLAND_DISPLAY", None)

    def cli(self, *args, ok=True):
        p = subprocess.run([str(BINARY), "test", *args], env=self.env, capture_output=True, text=True, timeout=10)
        self.assertEqual(p.returncode == 0, ok, p.stdout + p.stderr)
        return p

    def state(self, simulator=False):
        return json.loads(self.cli("status", *(["--simulator"] if simulator else [])).stdout)

    def test_targets_are_independent_and_flag_can_move(self):
        self.cli("scenario", "signed-out")
        physical = (self.root / "touchmeter-test.json").read_bytes()
        self.assertEqual(self.state(True), {"active": False})
        self.cli("--simulator", "scenario", "fire")
        self.assertIn("now", self.state(True))
        self.cli("set", "--simulator", "claude", "--enabled", "--short", "95", "--weekly", "42", "--burn-minutes", "20")
        data = self.state(True)
        self.assertEqual([x["percent"] for x in data["providers"]["claude"]["limits"]], [0.95, 0.42])
        self.assertEqual(data["providers"]["claude"]["burnMinutes"], 20)
        self.assertEqual(stat.S_IMODE((self.root / "touchmeter-simulator-test.json").stat().st_mode), 0o600)
        fixture = self.root / "simulator.json"
        data.pop("now")  # Loading must preserve an explicit fixture's moving clock.
        fixture.write_text(json.dumps(data))
        self.cli("load", str(fixture), "--simulator")
        self.assertEqual(self.state(True), data)
        self.cli("set", "claude", "--short", "101", "--simulator", ok=False)
        self.cli("clear", "--simulator", "--simulator", ok=False)
        self.assertEqual(self.state(True), data)
        self.assertEqual((self.root / "touchmeter-test.json").read_bytes(), physical)
        simulator = (self.root / "touchmeter-simulator-test.json").read_bytes()
        self.cli("set", "claude", "--enabled", "--short", "12")
        self.cli("clear")
        self.assertEqual((self.root / "touchmeter-simulator-test.json").read_bytes(), simulator)
        self.cli("scenario", "menu")
        physical = (self.root / "touchmeter-test.json").read_bytes()
        self.cli("clear", "--simulator")
        self.assertEqual(self.state(True), {"active": False})
        self.assertEqual((self.root / "touchmeter-test.json").read_bytes(), physical)
        self.assertFalse(self.root.joinpath("audio-called").exists())

    def test_scenarios_set_and_clear(self):
        self.assertEqual(self.state(), {"active": False})
        self.cli("scenario", "all-providers")
        data = self.state()
        self.assertEqual(data["mode"], "providers")
        self.assertEqual(len(data["providers"]), 12)
        self.assertTrue(all(p["enabled"] for p in data["providers"].values()))
        self.assertEqual(stat.S_IMODE((self.root / "touchmeter-test.json").stat().st_mode), 0o600)
        self.cli("set", "claude", "--enabled", "--short", "95", "--weekly", "42", "--stale")
        data = self.state()
        self.assertEqual(data["selected"], "claude")
        self.assertEqual(data["mode"], "usage")
        p = data["providers"]["claude"]
        self.assertTrue(p["limitsStale"])
        self.assertEqual([l["percent"] for l in p["limits"]], [0.95, 0.42])
        self.cli("set", "claude", "--disabled", "--fresh", "--mode", "functions")
        data = self.state()
        self.assertFalse(data["providers"]["claude"]["enabled"])
        self.assertFalse(data["providers"]["claude"]["limitsStale"])
        self.assertEqual(data["mode"], "functions")
        self.cli("clear")
        self.cli("clear")
        self.assertEqual(self.state(), {"active": False})

    def test_all_presets(self):
        for name in ["high-usage", "stale", "missing", "signed-out", "fire", "menu", "volume", "volume-muted", "volume-unavailable"]:
            self.cli("scenario", name)
            p = self.state()["providers"]["claude"]
            if name == "high-usage": self.assertEqual(p["limits"][0]["percent"], 0.96)
            if name == "stale": self.assertTrue(p["limitsStale"])
            if name == "missing": self.assertEqual(p["limits"], [])
            if name == "signed-out": self.assertFalse(p["enabled"])
            if name == "fire": self.assertEqual(p["burnMinutes"], 20)
            if name == "menu": self.assertEqual(self.state()["mode"], "menu")
            if name.startswith("volume"):
                self.assertEqual(self.state()["mode"], "volume")
                self.assertEqual(self.state()["volume"]["muted"], name == "volume-muted")
                self.assertEqual(self.state()["volume"]["available"], name != "volume-unavailable")

    def test_burn_override(self):
        self.cli("set", "claude", "--enabled", "--burn-minutes", "25")
        self.assertEqual(self.state()["providers"]["claude"]["burnMinutes"], 25)
        self.cli("set", "claude", "--burn-minutes", "nan", ok=False)
        self.cli("set", "claude", "--burn-minutes", "-1", ok=False)
        self.cli("set", "claude", "--no-burn")
        self.assertNotIn("burnMinutes", self.state()["providers"]["claude"])

    def test_invalid_input_leaves_previous_state_untouched(self):
        self.cli("scenario", "all-providers")
        before = self.state()
        for args in [("scenario", "nope"), ("set", "unknown", "--enabled"), ("set", "claude", "--short", "101"),
                     ("set", "claude", "--weekly", "nan"), ("set", "claude", "--short"),
                     ("set", "claude", "--mode", "invalid"), ("clear", "extra")]:
            self.cli(*args, ok=False)
            self.assertEqual(self.state(), before)
        fixture = self.root / "invalid.json"
        for data in ["not-json", "[]", "{}", "x" * 65537]:
            fixture.write_text(data)
            self.cli("load", str(fixture), ok=False)
            self.assertEqual(self.state(), before)
        for field, value in [("version", 2), ("mode", "invalid"), ("selected", "unknown"), ("now", "not-time"), ("credentials", "not-allowed")]:
            fixture.write_text(json.dumps({**before, field: value}))
            self.cli("load", str(fixture), ok=False)
            self.assertEqual(self.state(), before)

    def test_load_roundtrip_and_reset_due(self):
        self.cli("scenario", "all-providers")
        data = self.state()
        data["now"] = "2026-11-10T12:00:00Z"
        for p in data["providers"].values():
            p["limitsFetchedAt"] = 1794312000000
            for limit in p["limits"]: limit["resetsAt"] = "2026-11-10T11:59:00Z"
        fixture = self.root / "fixture.json"
        fixture.write_text(json.dumps(data))
        self.cli("load", str(fixture))
        self.assertEqual(self.state(), data)
        self.cli("set", "claude", "--short", "100")
        self.assertEqual(self.state()["providers"]["claude"]["limits"][0]["resetsAt"], "2026-11-10T14:00:00Z")

    def test_fixture_preview_is_isolated(self):
        self.cli("scenario", "signed-out")
        before = self.state()
        fixture = BINARY.parents[1] / "docs/fixtures/claude-high.json"
        output = self.root / "preview.png"
        p = subprocess.run([str(BINARY), "preview", str(output), "--fixture", str(fixture)], env=self.env, capture_output=True, text=True, timeout=30)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        png = output.read_bytes()
        self.assertEqual(png[:8], b"\x89PNG\r\n\x1a\n")
        self.assertEqual(struct.unpack(">II", png[16:24]), (2008, 60))
        self.assertEqual(self.state(), before)
        p = subprocess.run([str(BINARY), "preview", str(output), "--fixture", str(fixture), "--demo"], env=self.env, capture_output=True, text=True, timeout=10)
        self.assertNotEqual(p.returncode, 0)
        fixture = BINARY.parents[1] / "docs/fixtures/volume.json"
        p = subprocess.run([str(BINARY), "preview", str(output), "--fixture", str(fixture)], env=self.env, capture_output=True, text=True, timeout=30)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertFalse(self.root.joinpath("audio-called").exists())
        self.assertEqual(self.state(), before)

    def test_volume_fixture_validation(self):
        self.cli("scenario", "volume")
        before = self.state()
        fixture = self.root / "invalid-volume.json"
        for volume in [{"percent": 101}, {"percent": -1}, {"percent": "50"}, {"muted": "false"}, {"available": 1}, {"unknown": True}]:
            fixture.write_text(json.dumps({**before, "volume": volume}))
            self.cli("load", str(fixture), ok=False)
            self.assertEqual(self.state(), before)

    def test_simulator_commands_and_safe_process_lifecycle(self):
        self.cli("scenario", "signed-out")
        before = self.state()
        env = {**self.env, "QT_QPA_PLATFORM": "offscreen", "QT_QPA_PLATFORMTHEME": ""}
        p = subprocess.run([str(BINARY), "simulate", "--help"], env=env, capture_output=True, text=True, timeout=10)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("always simulated", p.stdout)
        p = subprocess.run([str(BINARY), "simulate", "--list-monitors"], env=env, capture_output=True, text=True, timeout=10)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("(primary)", p.stdout)
        for args in [("--monitor", "not-a-monitor"), ("--monitor",), ("--unknown",), ("--scenario", "invalid")]:
            p = subprocess.run([str(BINARY), "simulate", *args], env=env, capture_output=True, text=True, timeout=10)
            self.assertNotEqual(p.returncode, 0)
        p = subprocess.run(["timeout", "--preserve-status", "--kill-after=2s", "--signal=TERM", "2s", str(BINARY), "simulate", "--scenario", "fire", "--monitor", "0"],
                           env=env, capture_output=True, text=True, timeout=8)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertNotIn("Touch Bar leased", p.stderr)
        self.assertFalse(self.root.joinpath("audio-called").exists())
        self.assertEqual(self.state(), before)

    def test_symlink_cannot_overwrite_another_file(self):
        target = self.root / "leave-alone.json"
        target.write_text("do not overwrite")
        (self.root / "touchmeter-test.json").symlink_to(target)
        self.cli("scenario", "all-providers", ok=False)
        self.assertEqual(target.read_text(), "do not overwrite")


if __name__ == "__main__":
    unittest.main()
