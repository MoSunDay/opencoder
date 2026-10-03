"""Keep a local Kubernetes tunnel to an existing guest desktop; never replay tasks."""

import argparse
import json
import os
import shlex
import signal
import subprocess
import time
import urllib.request


def pod_for(args):
    result = subprocess.run(
        [args.kubectl, "-n", args.namespace, "get", "pods", "-l", args.selector,
         "--field-selector=status.phase=Running", "-o", "json"],
        check=True, capture_output=True, text=True, timeout=15,
    )
    pods = json.loads(result.stdout)["items"]
    if len(pods) != 1:
        raise RuntimeError("Expected exactly one running desktop pod")
    return pods[0]["metadata"]["name"]


def relay_script(args, stop=False):
    path = f"/tmp/opencoder-computer-{args.local_port}.pid"
    listener = f"TCP-LISTEN:{args.guest_port},bind=127.0.0.1,reuseaddr,fork"
    destination = f"TCP:{args.guest}:{args.guest_port}"
    # This file contains only the PID of the relay created by this supervisor.
    verify = (f"test -r {shlex.quote(path)} && p=$(cat {shlex.quote(path)}) && "
              'test -r /proc/$p/cmdline && '
              'test "$(cat /proc/$p/comm)" = socat && '
              f"tr '\\0' ' ' < /proc/$p/cmdline | grep -Fq {shlex.quote(listener)} && "
              f"tr '\\0' ' ' < /proc/$p/cmdline | grep -Fq {shlex.quote(destination)}")
    if stop:
        return f"if {verify}; then kill $p; fi; rm -f {shlex.quote(path)}"
    command = f"socat {shlex.quote(listener)} {shlex.quote(destination)}"
    return (f"if {verify}; then exit 0; fi; "
            f"nohup {command} >/tmp/opencoder-computer-{args.local_port}.log 2>&1 & "
            f"echo $! > {shlex.quote(path)}; sleep 1; "
            f"p=$(cat {shlex.quote(path)}); kill -0 $p")


def remote(args, pod, script):
    subprocess.run([args.kubectl, "-n", args.namespace, "exec", pod, "--", "sh", "-c", script],
                   check=True, timeout=15)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kubectl", default="kubectl")
    parser.add_argument("--namespace", required=True)
    parser.add_argument("--selector", required=True)
    parser.add_argument("--guest", required=True)
    parser.add_argument("--guest-port", type=int, default=8000)
    parser.add_argument("--local-port", type=int, default=18000)
    args = parser.parse_args()
    stopping = False

    def stop(signum, frame):
        nonlocal stopping
        stopping = True

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    while not stopping:
        pod, process = None, None
        try:
            pod = pod_for(args)
            remote(args, pod, relay_script(args))
            process = subprocess.Popen(
                [args.kubectl, "-n", args.namespace, "port-forward", "--address", "127.0.0.1",
                 pod, f"{args.local_port}:{args.guest_port}"],
                start_new_session=True,
            )
            failures = 0
            while not stopping and process.poll() is None:
                time.sleep(2)
                try:
                    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
                    with opener.open(f"http://127.0.0.1:{args.local_port}/status", timeout=3) as r:
                        if r.status != 200:
                            raise RuntimeError("Desktop server not ready")
                    failures = 0
                except Exception:
                    failures += 1
                    if failures >= 3:
                        raise RuntimeError("Desktop tunnel health failed")
        except Exception as error:
            print(str(error), flush=True)
        finally:
            if process is not None and process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
            if pod is not None:
                try:
                    remote(args, pod, relay_script(args, stop=True))
                except Exception as error:
                    print(f"Relay cleanup: {error}", flush=True)
        if not stopping:
            time.sleep(3)


if __name__ == "__main__":
    main()
