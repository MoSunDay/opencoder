"""Start a host Codex process with a fresh, process-owned proxy bridge."""
import os
import importlib
from pathlib import Path
import sys


def launch(arguments):
    if len(arguments) < 2:
        raise ValueError('usage: codex_proxy.py PROXY_RUNTIME CODEX [ARGS...]')
    runtime = Path(arguments[0]).resolve(strict=True)
    providers = sorted(path for path in runtime.glob('*_socks5.py') if path.is_file())
    if not (runtime / 'codex_proxy_exec.py').is_file() or len(providers) != 1:
        raise ValueError('proxy runtime requires one SOCKS5 provider and an executor')
    sys.path.insert(0, str(runtime))
    provider = importlib.import_module(providers[0].stem)
    from codex_proxy_exec import run

    proxy = provider.running_proxy_url()
    if proxy is None:
        raise ValueError('local authenticated SOCKS5 proxy is unavailable')
    # Resolve existing proxy credentials at execution time; never persist them
    # or the temporary bridge port in the Server profile.
    os.environ['CODEX_SOCKS5_PROXY'] = proxy
    run(arguments[1:])


if __name__ == '__main__':
    try:
        launch(sys.argv[1:])
    except ValueError as error:
        print(str(error), file=sys.stderr)
        sys.exit(2)
