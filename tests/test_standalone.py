"""Standalone build smoke tests; no Omarchy, panel, or audio tools needed."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest

BINARY = Path(os.environ.get("TOUCHMETER_BINARY", Path(__file__).resolve().parents[1] / "build/simulator/touchmeter-simulator"))


class Standalone(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.env = {**os.environ, "XDG_RUNTIME_DIR": str(self.root), "XDG_CONFIG_HOME": str(self.root / "config"),
                    "QT_QPA_PLATFORM": "offscreen", "QT_QPA_PLATFORMTHEME": "", "OMARCHY_PATH": str(self.root / "missing")}
        bindir = self.root / "bin"
        bindir.mkdir()
        for program in ["omarchy", "hyprctl", "wpctl", "brightnessctl"]:
            script = bindir / program
            script.write_text(f'#!/bin/sh\ntouch "{self.root / "external-called"}"\nexit 1\n')
            script.chmod(0o700)
        self.env["PATH"] = str(bindir) + os.pathsep + self.env.get("PATH", "")

    def run_cli(self, *args, ok=True):
        result = subprocess.run([str(BINARY), *args], env=self.env, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode == 0, ok, result.stdout + result.stderr)
        return result

    def test_help_and_hardware_rejection(self):
        self.assertIn("Movable/resizable", self.run_cli("--help").stdout)
        self.run_cli("open", ok=False)
        self.assertFalse((self.root / "touchmeter.lock").exists())

    def test_cli_defaults_to_simulator_target(self):
        physical = self.root / "touchmeter-test.json"
        physical.write_text("untouched physical state")
        self.run_cli("test", "set", "claude", "--enabled", "--short", "95", "--burn-minutes", "20")
        data = json.loads(self.run_cli("test", "status").stdout)
        self.assertEqual(data["providers"]["claude"]["burnMinutes"], 20)
        self.assertEqual(data, json.loads(self.run_cli("test", "status", "--simulator").stdout))
        self.run_cli("test", "clear")
        self.assertEqual(json.loads(self.run_cli("test", "status").stdout), {"active": False})
        self.assertEqual(physical.read_text(), "untouched physical state")

    def test_private_runtime_fallback_without_xdg(self):
        self.env.pop("XDG_RUNTIME_DIR")
        self.env["TMPDIR"] = str(self.root)
        self.run_cli("test", "scenario", "fire")
        data = json.loads(self.run_cli("test", "status").stdout)
        self.assertEqual(data["providers"]["claude"]["burnMinutes"], 20)
        self.run_cli("test", "clear")

    def test_render_and_live_override_lifecycle(self):
        output = self.root / "preview.png"
        self.run_cli("preview", str(output), "--scenario", "fire")
        self.assertEqual(output.read_bytes()[:8], b"\x89PNG\r\n\x1a\n")
        process = subprocess.Popen([str(BINARY), "--scenario", "volume-muted"], env=self.env,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            time.sleep(1)  # Allow Qt/GPU initialization and initial window exposure.
            self.run_cli("test", "scenario", "fire")
            time.sleep(0.75)  # Cross the runtime watcher's 500 ms polling interval.
            self.run_cli("test", "clear")
            time.sleep(0.75)
            self.assertIsNone(process.poll())
            process.terminate()
            stdout, stderr = process.communicate(timeout=10)
            self.assertEqual(process.returncode, 0, stdout + stderr)
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate()
        self.assertFalse((self.root / "external-called").exists())


if __name__ == "__main__":
    unittest.main()
