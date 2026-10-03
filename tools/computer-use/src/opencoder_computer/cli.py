"""Machine-readable CLI; credentials are configured using files."""

import argparse
import asyncio
import json
import sys
import uuid
from pathlib import Path

from . import __version__
from .config import load_settings
from .results import EXIT_CODES, diagnostics, redact, terminal_result
from .runner import doctor, execute
from .state import read_status, request_cancel


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(prog="opencoder-computer")
    root.add_argument("--version", action="version", version=__version__)
    root.add_argument("--config", type=Path, default=Path.home() / ".opencoder/computer.json")
    commands = root.add_subparsers(dest="command", required=True)
    check = commands.add_parser(
        "doctor", help="Check the remote desktop and local Cua dependencies"
    )
    check.add_argument("--target", required=True)
    check.add_argument("--timeout", type=float, default=30)
    check.add_argument("--check-model", action="store_true", help="Predict without desktop actions")
    run = commands.add_parser("run", help="Let Cua complete a natural-language desktop task")
    run.add_argument("--target", required=True)
    run.add_argument(
        "--task-file", type=Path, required=True, help="UTF-8 task file; '-' reads stdin"
    )
    run.add_argument("--output-dir", type=Path)
    run.add_argument("--timeout", type=float, default=600)
    run.add_argument("--max-actions", type=int, default=50)
    for name in ("status", "cancel"):
        command = commands.add_parser(name)
        command.add_argument("--run-dir", type=Path, required=True)
    return root


def main(argv=None) -> int:
    args = parser().parse_args(argv)
    secrets = ()
    try:
        if args.command in {"status", "cancel"}:
            result = (
                read_status(args.run_dir)
                if args.command == "status"
                else request_cancel(args.run_dir)
            )
        else:
            settings = load_settings(args.config.expanduser(), args.target)
            secrets = settings.secrets
            # Upstream SDKs occasionally print diagnostics; reserve stdout for our final JSON.
            with diagnostics(sys.stderr, secrets):
                if args.command == "doctor":
                    result = asyncio.run(
                        doctor(settings, args.timeout, check_model=args.check_model)
                    )
                else:
                    task = (
                        sys.stdin.read()
                        if str(args.task_file) == "-"
                        else (args.task_file.read_text(encoding="utf-8"))
                    )
                    directory = args.output_dir or (
                        Path.cwd() / ".opencoder/computer/runs" / uuid.uuid4().hex
                    )
                    result = asyncio.run(
                        execute(
                            settings,
                            task,
                            directory,
                            Path.home() / ".opencoder/computer/locks",
                            args.timeout,
                            args.max_actions,
                        )
                    )
        print(json.dumps(terminal_result(result), ensure_ascii=False))
        return EXIT_CODES.get(result["status"], 0 if result["status"] == "ready" else 1)
    except Exception as error:
        print(
            json.dumps(
                {"status": "failed", "error": redact(str(error), secrets)}, ensure_ascii=False
            )
        )
        return 1
