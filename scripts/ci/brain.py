#!/usr/bin/env python3
"""Build as the invoking user; isolate privileged native test processes."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

REPO = Path(__file__).resolve().parents[2]
SUITES = {
    'project': ('opencoder-project', None, []),
    'milestone': ('opencoder-brain', 'milestone', []),
    'scheduler': ('opencoder-worker', 'brain_scheduler_v4', []),
    'restart': ('opencoder-worker', 'brain_server_restart', []),
    'browser': ('opencoder-worker', 'brain_browser', [
        '--exact', 'schema_seven_canvas_parallel_return_and_execution_detail',
        '--ignored', '--nocapture']),
}
NATIVE = {'project', 'restart', 'browser'}


def artifact(line, target):
    try:
        record = json.loads(line)
    except ValueError:
        return None
    if (record.get('reason') == 'compiler-artifact'
            and (target is None or record.get('target', {}).get('name') == target)
            and record.get('profile', {}).get('test')):
        return record.get('executable')
    return None


def report_failure(label, path, code):
    tail = '\n'.join(path.read_text(errors='replace').splitlines()[-60:])
    print(f'{label} failed with exit code {code}; log: {path}', file=sys.stderr)
    if os.environ.get('GITHUB_ACTIONS') == 'true':
        escaped = tail.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
        print(f'::error title=Brain {label}::{escaped}')
    summary = os.environ.get('GITHUB_STEP_SUMMARY')
    if summary:
        with open(summary, 'a') as stream:
            stream.write(f'### Brain {label} (exit {code})\n\n```text\n{tail}\n```\n')


def logged(command, path, env=None, target=None):
    executables = []
    print('Running:', ' '.join(map(str, command)), flush=True)
    with path.open('w') as stream:
        process = subprocess.Popen(command, cwd=REPO, env=env, text=True,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        with process.stdout:
            for line in process.stdout:
                stream.write(line)
                stream.flush()
                executable = artifact(line, target)
                if executable and executable not in executables:
                    executables.append(executable)
                if not line.startswith('{'):
                    print(line, end='', flush=True)
                else:
                    try:
                        rendered = json.loads(line).get('message', {}).get('rendered')
                        if rendered:
                            print(rendered, end='', flush=True)
                    except (ValueError, AttributeError):
                        print(line, end='', flush=True)
        code = process.wait()
    if code:
        report_failure(path.stem, path, code)
        raise subprocess.CalledProcessError(code, command)
    return executables


def native_command(executable, arguments, env):
    # A private PID namespace also kills leftover fixture children on exit.
    command = ['unshare', '--mount', '--pid', '--fork', '--kill-child',
               '--mount-proc', '--propagation', 'private', '--',
               str(executable), *arguments]
    if os.geteuid() != 0:
        keys = ('PATH', 'DAG_TEST_ROOTFS', 'TMPDIR', 'CHROME_PATH',
                'RUST_BACKTRACE', 'RUST_TEST_THREADS', 'DEBUG')
        command = ['sudo', '-n', '--', 'env',
                   *(f'{key}={env[key]}' for key in keys if key in env), *command]
    return command


def preflight(rootfs):
    for name in ('runc', 'mount.nfs', 'unshare', 'cc', 'node'):
        if shutil.which(name) is None:
            raise RuntimeError(f'required native test dependency missing: {name}')
    infos = []
    for name in ('dag-runner', 'agent-step-runner', 'agent-session-runner'):
        path = rootfs / 'usr/bin' / name
        if not path.is_file() or path.is_symlink():
            raise RuntimeError(f'prepare the native test image first: missing {path}')
        # The session runner only accepts its execution environment, not CLI flags.
        if name != 'agent-session-runner':
            infos.append(json.loads(subprocess.check_output([path, '--build-info'], text=True)))
    if any(info != infos[0] for info in infos[1:]):
        raise RuntimeError('native test runner build metadata differs')
    # ContainerFixture additionally compares the image with the test executable.


def passed_tests(log):
    result = re.search(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', log)
    if not result or int(result[1]) == 0 or int(result[2]) != 0 or int(result[3]) != 0:
        raise RuntimeError('test run must pass at least one test without failures or ignored tests')
    return int(result[1])


def run_suite(suite, output):
    package, target, arguments = SUITES[suite]
    selection = ['--test', target] if target else []
    executables = logged(['cargo', 'test', '--locked', '-p', package, *selection,
                          '--no-run', '--message-format=json'],
                         output / f'{suite}-build.log', target=target)
    if not executables or any(not Path(path).is_file() for path in executables):
        raise RuntimeError(f'cargo did not produce the {suite} test executables')
    temp = output / f'{suite}-tmp'
    temp.mkdir(exist_ok=True)
    env = {**os.environ, 'TMPDIR': str(temp), 'RUST_BACKTRACE': '1',
           'DAG_TEST_ROOTFS': str(output / 'rootfs')}
    if suite == 'browser':
        env['DEBUG'] = 'pw:browser'
    if suite in NATIVE:
        preflight(output / 'rootfs')
    try:
        count = 0
        for index, executable in enumerate(executables):
            command = [executable, *arguments]
            if suite in NATIVE:
                command = native_command(executable, arguments, env)
            log = output / f'{suite}-test-{index}.log'
            logged(command, log, env)
            count += passed_tests(log.read_text())
        (output / f'{suite}-result.json').write_text(json.dumps({
            'suite': suite, 'executables': executables, 'passed': count, 'exit_code': 0,
        }, indent=2) + '\n')
    finally:
        if suite in NATIVE and os.geteuid() != 0:
            # Only this suite's temporary evidence can be owned by root.
            subprocess.run(['sudo', '-n', '--', 'chown', '-R',
                            f'{os.getuid()}:{os.getgid()}', str(temp)], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('stage', choices=['prepare', *SUITES])
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    try:
        if args.stage == 'prepare':
            logged(['bash', str(REPO / 'scripts/prepare-dag-rootfs.sh'),
                    str(output / 'rootfs')], output / 'prepare.log')
            preflight(output / 'rootfs')
        else:
            run_suite(args.stage, output)
    except (OSError, RuntimeError, ValueError) as error:
        path = output / f'{args.stage}-error.log'
        path.write_text(str(error) + '\n')
        report_failure(args.stage, path, 1)
        raise


if __name__ == '__main__':
    try:
        main()
    except subprocess.CalledProcessError as error:
        sys.exit(error.returncode if error.returncode > 0 else 1)
    except (OSError, RuntimeError, ValueError) as error:
        print(f'Brain acceptance: {error}', file=sys.stderr)
        sys.exit(1)
