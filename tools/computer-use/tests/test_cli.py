import asyncio
import json
import os
import signal
import subprocess
import sys
from pathlib import Path

import pytest

from opencoder_computer.cli import main, parser
from opencoder_computer.state import RunFiles


@pytest.mark.parametrize("command", ["doctor", "run", "status", "cancel"])
def test_cli_parses_all_commands(command):
    flags = ["--target", "desktop"] if command in {"doctor", "run"} else ["--run-dir", "run"]
    if command == "run":
        flags += ["--task-file", "task.txt"]
    assert parser().parse_args([command, *flags]).command == command


def test_cli_config_failure_is_single_json(capsys, tmp_path):
    code = main(["--config", str(tmp_path / "missing"), "doctor", "--target", "desktop"])
    assert code == 1
    assert json.loads(capsys.readouterr().out)["status"] == "failed"


def test_status_and_cancel_work_without_config_or_model(tmp_path):
    files = RunFiles(tmp_path / "run", "desktop")
    files.finish("completed", "done")
    for command in ("status", "cancel"):
        process = subprocess.run(
            [
                sys.executable,
                "-m",
                "opencoder_computer",
                "--config",
                str(tmp_path / "absent"),
                command,
                "--run-dir",
                str(files.directory),
            ],
            capture_output=True,
            text=True,
            check=True,
        )
        assert json.loads(process.stdout)["status"] == "completed"
        assert process.stderr == ""


async def start_cli(config: Path, *arguments):
    return await asyncio.create_subprocess_exec(
        sys.executable,
        "-m",
        "opencoder_computer",
        "--config",
        str(config),
        *arguments,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )


async def test_cli_real_sdk_run_and_doctor_dispatch(service, cli_config, tmp_path):
    for arguments, status in [
        (["doctor", "--target", "desktop"], "ready"),
        (
            [
                "run",
                "--target",
                "desktop",
                "--task-file",
                str(tmp_path / "task.txt"),
                "--output-dir",
                str(tmp_path / "run"),
            ],
            "completed",
        ),
    ]:
        process = await start_cli(cli_config, *arguments)
        stdout, stderr = await asyncio.wait_for(process.communicate(), 60)
        assert process.returncode == 0, stdout.decode()
        assert len(stdout.splitlines()) == 1
        assert json.loads(stdout)["status"] == status
        assert b"model-secret" not in stdout + stderr
        assert b"desktop-secret" not in stdout + stderr


async def test_cli_model_error_has_nonzero_exit_and_private_diagnostics(
    service, cli_config, tmp_path
):
    service.failure = True
    process = await start_cli(
        cli_config,
        "run",
        "--target",
        "desktop",
        "--task-file",
        str(tmp_path / "task.txt"),
        "--output-dir",
        str(tmp_path / "run"),
    )
    stdout, stderr = await asyncio.wait_for(process.communicate(), 60)
    assert process.returncode == 1
    assert json.loads(stdout)["status"] == "failed"
    assert b"model-secret" not in stdout + stderr
    assert b"desktop-secret" not in stdout + stderr


async def test_cancel_cli_stops_running_process(service, cli_config, tmp_path):
    service.stall = True
    process = await start_cli(
        cli_config,
        "run",
        "--target",
        "desktop",
        "--task-file",
        str(tmp_path / "task.txt"),
        "--output-dir",
        str(tmp_path / "run"),
    )
    try:
        for _ in range(500):
            if service.requests:
                break
            await asyncio.sleep(0.05)
        assert service.requests
        check = await start_cli(cli_config, "status", "--run-dir", str(tmp_path / "run"))
        stdout, _ = await check.communicate()
        assert json.loads(stdout)["status"] == "running"
        cancel = await start_cli(cli_config, "cancel", "--run-dir", str(tmp_path / "run"))
        stdout, _ = await cancel.communicate()
        assert cancel.returncode == 0
        assert json.loads(stdout)["status"] == "cancel_requested"
        stdout, _ = await asyncio.wait_for(process.communicate(), 10)
        assert process.returncode == 130
        assert json.loads(stdout)["status"] == "cancelled"
    finally:
        if process.returncode is None:
            process.kill()
            await process.wait()


@pytest.mark.skipif(os.name == "nt", reason="Unix signal delivery; Windows uses cancel CLI")
async def test_sigterm_persists_cancelled_result(service, cli_config, tmp_path):
    service.stall = True
    process = await start_cli(
        cli_config,
        "run",
        "--target",
        "desktop",
        "--task-file",
        str(tmp_path / "task.txt"),
        "--output-dir",
        str(tmp_path / "run"),
    )
    try:
        for _ in range(500):
            if service.requests:
                break
            await asyncio.sleep(0.05)
        assert service.requests
        process.send_signal(signal.SIGTERM)
        stdout, _ = await asyncio.wait_for(process.communicate(), 10)
        assert process.returncode == 130
        assert json.loads(stdout)["status"] == "cancelled"
        assert json.loads((tmp_path / "run/result.json").read_text())["status"] == "cancelled"
    finally:
        if process.returncode is None:
            process.kill()
            await process.wait()
