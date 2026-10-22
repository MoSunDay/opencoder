"""Validate capability details against an isolated Server and its embedded SPA."""
import argparse
import hashlib
import json
import socket
import subprocess
import time
import urllib.request
from pathlib import Path


def main(browser_script=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", required=True, type=Path)
    parser.add_argument("--root", required=True, type=Path)
    args = parser.parse_args()
    root = args.root.resolve()
    repo = Path(__file__).resolve().parents[3]
    if root.exists() or root.is_relative_to(repo):
        raise ValueError("Use a new evidence directory outside the repository")
    root.mkdir(parents=True)
    (root / "data").mkdir()
    work = root / "work"
    work.mkdir()
    (root / "agents").mkdir()
    (root / "share" / "todo").mkdir(parents=True)
    config = {
        "agent": {"agents_dir": str(root / "agents"), "share_dir": str(root / "share"),
                  "nfs": {"enabled": False}},
        "dag": {"binary_dir": str(root / "binary"), "workspace_dir": str(root / "workspace"),
                "nfs": {"enabled": False}, "workspace_nfs": {"enabled": False}},
        "ontology": {"files_dir": str(root / "files"), "nfs": {"enabled": False}},
    }
    (work / "opencoder.json").write_text(json.dumps(config))
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    base = f"http://127.0.0.1:{port}"
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def request(path):
        with opener.open(base + path, timeout=10) as response:
            return response.read()

    receipt = {"passed": False}
    with (root / "server.log").open("w") as log:
        process = subprocess.Popen([
            str(args.server.resolve()), "--workdir", str(work), "--data-dir", str(root / "data"),
            "--port", str(port), "--token", "ontology-fixture", "--auth-enabled", "false", "--web",
        ], stdout=log, stderr=log)
        try:
            # A new Turso catalog may initialize while release builds and
            # online backups are using the same disk. This is preparation,
            # separate from the browser assertions and live release gates.
            deadline = time.monotonic() + 180
            last_status = None
            while True:
                try:
                    status = json.loads(request("/api/ontology/envs/debug/sync"))
                    last_status = status
                    if status["state"] == "ready":
                        break
                except OSError:
                    pass
                if process.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError(f"Isolated Server did not initialize: {last_status}; see server.log")
                time.sleep(0.2)
            # Compare the actual served assets to this checkout, including changes
            # made after an earlier binary build.
            assets = {}
            for relative in ["index.html", "static/app.js", "static/app.css"]:
                served = request("/" if relative == "index.html" else "/" + relative)
                expected = (repo / "crates/web/spa/dist" / relative).read_bytes()
                if served != expected:
                    raise RuntimeError(f"Embedded SPA differs from current build: {relative}")
                assets[relative] = hashlib.sha256(served).hexdigest()
            browser = subprocess.run(
                ["node", str(browser_script or Path(__file__).with_suffix(".mjs")), base, str(root)],
                capture_output=True, text=True, timeout=180,
            )
            (root / "browser.log").write_text(browser.stdout + browser.stderr)
            browser.check_returncode()
            receipt.update(
                passed=True, assets=assets,
                build=json.loads(subprocess.check_output([str(args.server.resolve()), "--build-info"])),
                browser=json.loads((root / "browser.json").read_text()),
            )
        finally:
            process.terminate()
            try:
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            (root / "receipt.json").write_text(json.dumps(receipt, indent=2))
    print(json.dumps({"passed": True, "evidence": str(root)}))


if __name__ == "__main__":
    main()
