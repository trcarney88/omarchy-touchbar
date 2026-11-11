import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "src/provider_credentials.py"


class Credentials(unittest.TestCase):
    def test_only_booleans_and_fail_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "bin").mkdir()
            (root / "bin/omarchy-agent-usage-deepseek").write_text("def api_key(): return 'SECRET-NOT-FOR-UI'\n")
            (root / "bin/omarchy-agent-usage-minimax").write_text("def credentials(): return ('', 'https://example.invalid')\n")
            # Accidental upstream logging must not escape the helper either.
            (root / "bin/omarchy-agent-usage-synthetic").write_text("print('SECRET-LOG')\ndef api_key(): raise ValueError('SECRET-ERROR')\n")
            p = subprocess.run(["python3", str(SCRIPT)], env={**os.environ, "OMARCHY_PATH": tmp}, text=True, capture_output=True, check=True)
            self.assertNotIn("SECRET", p.stdout + p.stderr)
            state = json.loads(p.stdout)
            self.assertEqual(len(state), 12)
            self.assertTrue(state["deepseek"])
            self.assertFalse(state["minimax"])
            self.assertFalse(state["synthetic"])
            self.assertFalse(state["codex"])
            self.assertTrue(all(type(v) is bool for v in state.values()))


if __name__ == "__main__":
    unittest.main()
