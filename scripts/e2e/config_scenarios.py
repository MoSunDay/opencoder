"""Key-free config/env CLI e2e scenarios (E20).

These scenarios NEVER call the LLM: they exercise only the config/env CLI
surface (`config show`, `--model` override validation, named-env activation),
so the module runs in any shell without credentials — it is included in BOTH
`--only cli` and `--only web` modes of scripts/e2e_glm.py.

  E20a  env override effective: OPENCODER_MODEL + OPENAI_BASE_URL reach the
        merged config AND the matching `providers` registry entry (the
        registry base_url override is the core behavior this pins).
  E20b  api_key masked: `config show` never prints a full key (providers
        registry AND legacy top-level provider); masked form is first 4
        chars + `***`.
  E20c  envs activation banner: isolated HOME + env dir + plain-text active
        marker -> `config show` stderr carries `active env: <name>` while
        stdout stays pure JSON (banner layout per core config/envs.rs).
  E20d  malformed `--model` rejected: nonzero rc, failing on the model —
        before any API-key requirement.

Run standalone:  python3 scripts/e2e/config_scenarios.py [binary]
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from typing import Any

try:
    from . import lib
    from .lib import Counter
except ImportError:  # standalone: python3 scripts/e2e/config_scenarios.py
    sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    from e2e import lib
    from e2e.lib import Counter

# Provider id present in the seeded config's `providers` registry (must match
# the id lib.make_config's `model` prefix selects).
PROVIDER_ID = "zhipuai-coding-plan"
# Real-looking but never-valid credential: no scenario contacts a network.
FAKE_KEY = "sk-e2e-1234567890abcdef"


def _run(
    bin_path: str,
    args: list[str],
    *,
    env_extra: dict[str, str] | None = None,
    home: str | None = None,
    timeout: int = 120,
) -> tuple[int, str, str]:
    """Run the binary with stdout/stderr captured SEPARATELY (lib.run merges)
    and an optional isolated environment: `env_extra` is merged over the
    current environ; `home` redirects HOME and XDG_CONFIG_HOME so global
    config / envs discovery sees only the given dir."""
    env = dict(os.environ)
    if home is not None:
        env["HOME"] = home
        env["XDG_CONFIG_HOME"] = os.path.join(home, ".config")
    if env_extra:
        env.update(env_extra)
    try:
        p = subprocess.run(
            [bin_path] + args, capture_output=True, text=True, timeout=timeout, env=env
        )
        return p.returncode, p.stdout or "", p.stderr or ""
    except subprocess.TimeoutExpired:
        return 124, "", f"TIMEOUT after {timeout}s"


def _json_or_none(text: str) -> Any:
    try:
        return json.loads(text)
    except Exception:
        return None


def _seed_registry_cfg(api_key: str = FAKE_KEY) -> dict[str, Any]:
    """Base config from lib.make_config plus a `providers` registry entry for
    the active provider id (mirrors the structure core expects: each entry is
    `{base_url, api_key?, model?}`)."""
    cfg = lib.make_config(api_key=api_key)
    cfg["providers"] = {
        PROVIDER_ID: {
            "base_url": "https://open.bigmodel.cn/api/coding/paas/v4",
            "api_key": api_key,
        },
    }
    return cfg


def _e20a_env_override(c: Counter, bin_path: str) -> None:
    print("== E20a: env override effective (OPENCODER_MODEL + OPENAI_BASE_URL) ==")
    model_env = f"{PROVIDER_ID}/env-override-model"
    base_env = "https://env-override.example/v1"
    wd = lib.seed_workdir(_seed_registry_cfg())
    rc, out, err = _run(
        bin_path,
        ["--workdir", wd, "config", "show"],
        env_extra={"OPENCODER_MODEL": model_env, "OPENAI_BASE_URL": base_env},
        timeout=30,
    )
    parsed = _json_or_none(out)
    c.check("config show rc==0", rc == 0, f"rc={rc} err_tail={err[-200:]}")
    c.check("stdout parses as JSON", isinstance(parsed, dict), f"out_tail={out[-200:]}")
    if isinstance(parsed, dict):
        c.check("model == OPENCODER_MODEL value", parsed.get("model") == model_env,
                f"model={parsed.get('model')}")
        prov = parsed.get("provider") or {}
        c.check("provider.base_url == OPENAI_BASE_URL value",
                prov.get("base_url") == base_env, f"base_url={prov.get('base_url')}")
        reg = (parsed.get("providers") or {}).get(PROVIDER_ID) or {}
        c.check(f"providers[{PROVIDER_ID}].base_url == OPENAI_BASE_URL value",
                reg.get("base_url") == base_env,
                f"registry base_url={reg.get('base_url')}")


def _e20b_api_key_masked(c: Counter, bin_path: str) -> None:
    print("== E20b: api_key masked in config show output ==")
    masked = FAKE_KEY[:4] + "***"  # first 4 chars + ***
    wd = lib.seed_workdir(_seed_registry_cfg(api_key=FAKE_KEY))
    rc, out, err = _run(bin_path, ["--workdir", wd, "config", "show"], timeout=30)
    c.check("config show rc==0", rc == 0, f"rc={rc} err_tail={err[-200:]}")
    c.check(f"masked api_key form present ({masked})", masked in out)
    c.check("full api_key never leaks to stdout", FAKE_KEY not in out)


def _e20c_envs_banner(c: Counter, bin_path: str) -> None:
    print("== E20c: envs activation banner (active env: <name>) ==")
    name = "e2eenv"
    home = tempfile.mkdtemp(prefix="opencoder_e2e_home_")
    try:
        # No CLI subcommand manages envs (crates/cli has none), so build the
        # layout directly under the isolated HOME, per core config/envs.rs:
        #   ~/.opencoder/envs/<name>/config.json   (env layer snapshot)
        #   ~/.opencoder/envs/active               (plain-text name marker)
        env_dir = os.path.join(home, ".opencoder", "envs", name)
        os.makedirs(env_dir)
        with open(os.path.join(env_dir, "config.json"), "w") as f:
            json.dump({"fps": 24}, f)  # key the project file never sets
        with open(os.path.join(home, ".opencoder", "envs", "active"), "w") as f:
            f.write(f"{name}\n")

        wd = lib.seed_workdir(lib.make_config(api_key=FAKE_KEY))
        rc, out, err = _run(bin_path, ["--workdir", wd, "config", "show"], home=home,
                            timeout=30)
        parsed = _json_or_none(out)
        c.check("config show rc==0 (isolated HOME)", rc == 0,
                f"rc={rc} err_tail={err[-200:]}")
        c.check("stderr carries active env banner", f"active env: {name}" in err,
                f"err_tail={err[-200:]}")
        c.check("stdout stays pure JSON (banner is stderr-only)",
                isinstance(parsed, dict), f"out_tail={out[-200:]}")
        if isinstance(parsed, dict):
            c.check("env layer merged (fps=24 from env config.json)",
                    parsed.get("fps") == 24, f"fps={parsed.get('fps')}")
    finally:
        shutil.rmtree(home, ignore_errors=True)


def _e20d_malformed_model(c: Counter, bin_path: str) -> None:
    print("== E20d: malformed --model rejected before any API-key requirement ==")
    cfg = lib.make_config(api_key=FAKE_KEY)
    # Point the provider at an unroutable local port so that IF validation
    # lets the run proceed, the LLM call fails instantly (connection refused)
    # instead of touching a real network. Either path yields nonzero rc —
    # without depending on credentials.
    cfg["provider"]["base_url"] = "http://127.0.0.1:9/v1"
    wd = lib.seed_workdir(cfg)
    for label, model_val in (("--model x", "x"), ('--model ""', "")):
        rc, out, err = _run(
            bin_path, ["--workdir", wd, "--model", model_val, "run", "hi"], timeout=60
        )
        c.check(f"{label} rejected with nonzero rc", rc != 0, f"rc={rc}")
        c.soft(f"{label} stderr names the malformed model",
               "malformed" in err.lower() or "provider/model" in err,
               f"err_tail={err[-200:]}")
        c.soft(f"{label} fails before any API-key requirement",
               "api key" not in err.lower() and "OPENAI_API_KEY" not in err,
               f"err_tail={err[-200:]}")


def run_all(bin_path: str) -> Counter:
    """Run every key-free config/env scenario. NOTE: no api_key parameter —
    nothing here may call the LLM."""
    c = Counter()
    _e20a_env_override(c, bin_path)
    _e20b_api_key_masked(c, bin_path)
    _e20c_envs_banner(c, bin_path)
    _e20d_malformed_model(c, bin_path)
    c.summary("Config scenarios")
    return c


def _main() -> int:
    import argparse

    ap = argparse.ArgumentParser(
        description="opencoder key-free config/env e2e (E20; never calls the LLM)"
    )
    ap.add_argument("binary", nargs="?", default=None, help="path to the opencoder binary")
    args = ap.parse_args()

    bin_path = lib.resolve_bin(args.binary)
    if not os.path.isfile(bin_path):
        print(f"FAIL: binary not found: {bin_path}", file=sys.stderr)
        return 2
    total = run_all(bin_path)
    print("\n" + "=" * 60)
    print(f"config e2e result: {total.passed} passed, {total.failed} failed, "
          f"{total.skipped} skipped")
    print("=" * 60)
    return 1 if total.failed else 0


if __name__ == "__main__":
    sys.exit(_main())
