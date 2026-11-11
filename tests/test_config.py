"""Persistent settings CLI, always using an isolated config/runtime directory."""
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest

BINARY = Path(__file__).resolve().parents[1] / "build/touchmeter"
KEY = "fire.trigger-minutes"


class Configuration(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.env = {**os.environ, "XDG_CONFIG_HOME": str(self.root / "config"), "XDG_RUNTIME_DIR": str(self.root)}
        self.env.pop("DISPLAY", None)
        self.env.pop("WAYLAND_DISPLAY", None)
        self.path = self.root / "config/touchmeter/config.json"

    def cli(self, *args, ok=True):
        p = subprocess.run([str(BINARY), "config", *args], env=self.env, text=True, capture_output=True, timeout=10)
        self.assertEqual(p.returncode == 0, ok, p.stdout + p.stderr)
        return p

    def test_defaults_do_not_write_files(self):
        self.assertEqual(self.cli("get", KEY).stdout.strip(), "30")
        self.assertEqual(json.loads(self.cli("show").stdout), {KEY: 30})
        self.assertEqual(self.cli("path").stdout.strip(), str(self.path))
        self.cli("--help")
        self.assertFalse(self.path.parent.exists())

    def test_persistence_backup_and_reset(self):
        self.cli("set", KEY, "60")
        self.assertEqual(self.cli("get", KEY).stdout.strip(), "60")
        original = self.path.read_bytes()
        self.assertEqual(json.loads(original), {"version": 1, "fire": {"triggerMinutes": 60}})
        self.assertEqual(stat.S_IMODE(self.path.stat().st_mode), 0o600)
        self.cli("set", KEY, "15")
        backups = list(self.path.parent.glob("config.json.bak.*"))
        self.assertEqual(len(backups), 1)
        self.assertEqual(backups[0].read_bytes(), original)
        self.assertEqual(self.cli("get", KEY).stdout.strip(), "15")
        self.cli("reset", KEY)
        self.assertEqual(self.cli("get", KEY).stdout.strip(), "30")
        self.cli("set", KEY, "1440")
        self.cli("reset")
        self.assertEqual(self.cli("get", KEY).stdout.strip(), "30")

    def test_invalid_input_is_not_written(self):
        self.cli("set", KEY, "60")
        original = self.path.read_bytes()
        for value in ["0", "-1", "1441", "nan", "inf", "30.5", "word"]:
            self.cli("set", KEY, value, ok=False)
        for args in [("set", "unknown", "1"), ("reset", "unknown"), ("get", "unknown"), ("set", KEY), ("show", "extra")]:
            self.cli(*args, ok=False)
        self.assertEqual(self.path.read_bytes(), original)
        self.assertFalse(list(self.path.parent.glob("config.json.bak.*")))

    def test_corrupt_or_future_config_is_never_overwritten(self):
        self.path.parent.mkdir(parents=True)
        for contents in ["not-json", "[]", "x" * 65537, '{"version":2}',
                         '{"version":1,"fire":{"triggerMinutes":"30"}}',
                         '{"version":1,"unknown":true}']:
            self.path.write_text(contents)
            self.cli("show", ok=False)
            self.cli("set", KEY, "60", ok=False)
            self.cli("reset", ok=False)
            self.assertEqual(self.path.read_text(), contents)

    def test_symlink_cannot_overwrite_another_file(self):
        self.path.parent.mkdir(parents=True)
        target = self.root / "untouched.json"
        target.write_text('{"version":1}')
        self.path.symlink_to(target)
        self.cli("set", KEY, "60", ok=False)
        self.assertEqual(target.read_text(), '{"version":1}')

    def test_test_data_and_preferences_are_independent(self):
        self.cli("set", KEY, "60")
        subprocess.run([str(BINARY), "test", "scenario", "fire"], env=self.env, check=True, capture_output=True)
        subprocess.run([str(BINARY), "test", "clear"], env=self.env, check=True, capture_output=True)
        self.assertEqual(self.cli("get", KEY).stdout.strip(), "60")


if __name__ == "__main__":
    unittest.main()
