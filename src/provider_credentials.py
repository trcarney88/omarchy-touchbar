"""Read-only Omarchy credential probes. Only booleans leave this process.

Import the installed collectors without running their main/network routines so
their own credential precedence (including pi and environment overrides) is kept.
Never infer authentication from historical usage, a directory, or `ready`.
"""
import contextlib
import io
import json
import os
from pathlib import Path
import runpy
import sys

sys.dont_write_bytecode = True
root = Path(os.environ.get("OMARCHY_PATH", "/usr/share/omarchy"))
os.environ.setdefault("OMARCHY_PATH", str(root))


def probe(provider):
    # An upstream collector changing its private API fails closed for that
    # provider, not for the whole picker. No diagnostics may contain credentials.
    try:
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            c = runpy.run_path(str(root / "bin" / ("omarchy-agent-usage-" + provider)))
            if provider == "codex":
                home = c["codex_home"]()
                auth = json.loads((home / "auth.json").read_text()) if (home / "auth.json").is_file() else {}
                tokens = auth.get("tokens") or {}
                return bool(c["ENV"].get("CODEX_ACCESS_TOKEN") or auth.get("OPENAI_API_KEY") or tokens.get("access_token") or tokens.get("refresh_token"))
            if provider == "claude":
                return bool(c["oauth_login"](c["config_dir"]())[0] or os.environ.get("ANTHROPIC_API_KEY"))
            if provider == "grok":
                return bool(c["login"](c["default_home"]()).get("key"))
            if provider == "kimi":
                return c["has_credential"]()
            if provider in ("minimax", "zai"):
                return bool(c["credentials"]()[0])
            if provider == "openrouter":
                return bool(c["api_key"](c["read_config"]()))
            if provider == "opencode":
                return bool(c["credential_candidates"]())
            if provider == "cursor":
                return bool(c["cursor_token"]())
            return bool(c["api_key"]())
    except Exception:
        return False


if __name__ == "__main__":
    providers = ("codex", "claude", "grok", "kimi", "deepseek", "minimax", "zai", "openrouter", "synthetic", "ollama", "opencode", "cursor")
    print(json.dumps({provider: bool(probe(provider)) for provider in providers}))
