import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import brain


class BrainRunnerTests(unittest.TestCase):
    def test_browser_process_failure_survives_a_long_rust_backtrace(self):
        fatal = 'pw:browser [pid=21][err] FATAL: renderer launch failed'
        original = 'Brain browser failure: page.goto: Page crashed'
        log = '\n'.join([fatal, original, *[f'frame {i}' for i in range(100)], 'FAILED'])
        excerpt = brain.failure_excerpt(log)
        self.assertTrue(excerpt.startswith(fatal))
        self.assertIn(original, excerpt)
        self.assertTrue(excerpt.endswith('FAILED'))
        self.assertNotIn('frame 0\n', excerpt)

    def test_failed_command_keeps_output_and_exit_status(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / 'failure.log'
            command = [sys.executable, '-c', 'print("fixture failed", flush=True); exit(17)']
            with patch.dict(brain.os.environ, {}, clear=True):
                with self.assertRaises(subprocess.CalledProcessError) as raised:
                    brain.logged(command, log)
            self.assertEqual(raised.exception.returncode, 17)
            self.assertIn('fixture failed', log.read_text())

    def test_native_child_gets_required_paths_without_host_credentials(self):
        env = {'PATH': '/runner/node:/usr/bin', 'DAG_TEST_ROOTFS': '/fixture/rootfs',
               'TMPDIR': '/fixture/tmp', 'CHROME_PATH': '/runner/chromium',
               'GITHUB_TOKEN': 'fixture-secret', 'RUST_BACKTRACE': '1', 'DEBUG': 'pw:browser'}
        with patch.object(brain.os, 'geteuid', return_value=1001):
            command = brain.native_command('/fixture/test', ['--ignored'], env)
        self.assertIn('CHROME_PATH=/runner/chromium', command)
        self.assertIn('TMPDIR=/fixture/tmp', command)
        self.assertNotIn('GITHUB_TOKEN=fixture-secret', command)
        self.assertIn('DEBUG=pw:browser', command)
        self.assertIn('--kill-child', command)
        self.assertIn('private', command)
        self.assertEqual(command[-2:], ['/fixture/test', '--ignored'])

    def test_only_requested_test_artifact_is_selected(self):
        record = {'reason': 'compiler-artifact', 'target': {'name': 'brain_browser'},
                  'profile': {'test': True}, 'executable': '/fixture/brain-test'}
        self.assertEqual(brain.artifact(json.dumps(record), 'brain_browser'), '/fixture/brain-test')
        self.assertIsNone(brain.artifact(json.dumps(record), 'brain_server_restart'))
        record['profile']['test'] = False
        self.assertIsNone(brain.artifact(json.dumps(record), 'brain_browser'))
        self.assertIsNone(brain.artifact('compiler diagnostic', 'brain_browser'))

    def test_missing_image_is_a_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.object(brain.shutil, 'which', return_value='/usr/bin/tool'):
                with self.assertRaisesRegex(RuntimeError, 'prepare the native test image'):
                    brain.preflight(Path(directory))

    def test_zero_or_ignored_tests_cannot_pass_acceptance(self):
        self.assertEqual(brain.passed_tests('test result: ok. 2 passed; 0 failed; 0 ignored;'), 2)
        for text in ('no tests ran', 'test result: ok. 0 passed; 0 failed; 0 ignored;',
                     'test result: ok. 1 passed; 0 failed; 1 ignored;'):
            with self.assertRaises(RuntimeError):
                brain.passed_tests(text)


if __name__ == '__main__':
    unittest.main()
