"""External effects are injectable so interrupted switches can be rehearsed."""
import json
import subprocess
import time
import urllib.error
import urllib.request


class Operations:
    def __init__(self, token_file):
        self.token = token_file.read_text().strip()
        if not self.token or any(c in self.token for c in "\r\n"):
            raise ValueError("credential file must contain one nonempty token")
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def http(self, base, path, method="GET", body=None, timeout=30):
        request = urllib.request.Request(base.rstrip("/") + path,
            data=None if body is None else json.dumps(body).encode(), method=method,
            headers={"Authorization": f"Bearer {self.token}", "Content-Type": "application/json"})
        try:
            with self.opener.open(request, timeout=timeout) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            detail = error.read(4096).decode(errors="replace")
            raise RuntimeError(f"{method} {path}: HTTP {error.code}: {detail}") from None

    def run(self, *args):
        subprocess.run(args, check=True, stdin=subprocess.DEVNULL)

    def inactive(self, unit):
        result = subprocess.run(["systemctl", "show", "--property=ActiveState", "--value", unit],
            check=True, capture_output=True, text=True)
        return result.stdout.strip() == "inactive"

    def wait(self, check, seconds=90):
        deadline = time.monotonic() + seconds
        last_error = None
        while True:
            try:
                result = check()
                if result:
                    return result
            except (OSError, RuntimeError, urllib.error.URLError) as error:
                last_error = error
            if time.monotonic() >= deadline:
                raise TimeoutError(f"readiness check timed out: {last_error}")
            time.sleep(0.1)
