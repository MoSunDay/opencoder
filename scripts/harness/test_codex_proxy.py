"""Verify the launcher boundary without accessing credentials or the network."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name('codex_proxy.py')


class ProxyLauncherTests(unittest.TestCase):
    def run_launcher(self, proxy, arguments):
        with tempfile.TemporaryDirectory() as directory:
            runtime = Path(directory)
            (runtime / 'fixture_socks5.py').write_text(
                'def running_proxy_url():\n    return ' + repr(proxy) + '\n')
            (runtime / 'codex_proxy_exec.py').write_text(
                'import json, os, sys\n'
                'def run(argv):\n'
                '    print(json.dumps({"argv": argv, "stdin": sys.stdin.read(),'
                ' "proxy": os.environ["CODEX_SOCKS5_PROXY"]}))\n')
            return subprocess.run([sys.executable, str(SCRIPT), directory, *arguments],
                                  input='prompt\nsecond line', text=True, capture_output=True)

    def test_preserves_stdin_and_literal_arguments(self):
        arguments = ['/opt/codex', '--auth-slot', '10', 'exec', '--json', '-']
        result = self.run_launcher('socks5h://fixture@127.0.0.1:1080', arguments)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout), {
            'argv': arguments, 'stdin': 'prompt\nsecond line',
            'proxy': 'socks5h://fixture@127.0.0.1:1080'})

    def test_missing_proxy_fails_before_codex_starts(self):
        result = self.run_launcher(None, ['/opt/codex'])
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stdout, '')
        self.assertIn('proxy is unavailable', result.stderr)

    def test_ambiguous_or_missing_provider_fails_before_executor_loads(self):
        for count in (0, 2):
            with self.subTest(count=count), tempfile.TemporaryDirectory() as directory:
                runtime = Path(directory)
                (runtime / 'codex_proxy_exec.py').write_text('raise RuntimeError("loaded")')
                for number in range(count):
                    (runtime / f'fixture{number}_socks5.py').write_text('')
                result = subprocess.run([sys.executable, str(SCRIPT), directory, '/opt/codex'],
                                        text=True, capture_output=True)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(result.stdout, '')
                self.assertIn('requires one SOCKS5 provider', result.stderr)


if __name__ == '__main__':
    unittest.main()
