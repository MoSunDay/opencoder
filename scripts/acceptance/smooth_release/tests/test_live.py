"""Acceptance fixtures must prove process continuity and real scheduling."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from live import chain
from fixture import release_wasi_gate
from metrics import summarize, verify
from transitions import command
from types import SimpleNamespace


class AcceptanceTests(unittest.TestCase):
    def test_late_admission_keeps_its_release_gate_without_touching_other_runs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sibling = root / 'dag' / 'other'
            sibling.mkdir(parents=True)
            run = root / 'dag' / 'own'
            def admit():
                time.sleep(.02)
                run.mkdir()
                (run / 'execution.json').write_text('{}')
            writer = threading.Thread(target=admit)
            writer.start()
            release_wasi_gate(root, 'own', seconds=2)
            writer.join()
            self.assertTrue((run / 'release').is_file())
            self.assertFalse((sibling / 'release').exists())
            (run / 'hold').mkdir()
            (run / 'hold' / 'context.json').write_text('{}')
            release_wasi_gate(root, 'own')
            self.assertTrue((run / 'release').is_file())

    def test_unconfirmed_admission_does_not_create_an_orphan_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaisesRegex(TimeoutError, 'unconfirmed'):
                release_wasi_gate(root, 'absent', seconds=0)
            self.assertFalse((root / 'dag' / 'absent').exists())

    def test_signal_acceptance_uses_operator_cli_for_publish_and_rollback(self):
        args = SimpleNamespace(config=Path('/config.json'), bundle=Path('/bundle'), signal=True)
        self.assertEqual(command(args)[-3:], ['--signal', '--bundle', '/bundle'])
        self.assertEqual(command(args, rollback=True)[-2:], ['--signal', '--rollback'])
        args.signal = False
        self.assertNotIn('--signal', command(args))

    def test_model_scripts_wait_for_release_and_enforce_the_dependency(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            spec = chain(root)
            self.assertEqual(spec['todos'][1]['depends_on'], ['first'])
            process = subprocess.Popen([sys.executable, str(root / 'first.py')],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                deadline = time.monotonic() + 5
                while not (root / 'model-shell.pid').exists():
                    self.assertLess(time.monotonic(), deadline)
                    time.sleep(.01)
                self.assertEqual(int((root / 'model-shell.pid').read_text()), process.pid)
                self.assertIsNone(process.poll())
                self.assertFalse((root / 'first.done').exists())
                early = subprocess.run([sys.executable, str(root / 'second.py')], capture_output=True)
                self.assertNotEqual(early.returncode, 0)
                self.assertFalse((root / 'second.done').exists())
            finally:
                (root / 'release').touch()
                output, error = process.communicate(timeout=5)
            self.assertEqual(process.returncode, 0, error)
            self.assertEqual(output, b'first dependency completed\n\n')
            second = subprocess.run([sys.executable, str(root / 'second.py')], capture_output=True, check=True)
            self.assertEqual(second.stdout, b'second consumed first dependency\n\n')

    def test_continuity_distinguishes_queue_wait_from_a_scheduling_pause(self):
        traffic = [{'id': 'dag-a', 'at': 0, 'seconds': .1},
            {'id': 'dag-b', 'at': .2, 'seconds': .1}]
        executions = [{'created_at_ms': 0, 'started_at_ms': 5000},
            {'created_at_ms': 200, 'started_at_ms': 5200}]
        metrics = summarize(traffic, executions)
        self.assertAlmostEqual(metrics['max_accept_gap_seconds'], .2)
        self.assertEqual(metrics['max_scheduling_gap_seconds'], .2)
        self.assertEqual(metrics['max_scheduling_delay_seconds'], 5)
        with self.assertRaisesRegex(AssertionError, 'at least two'):
            summarize([], [])

    def test_durable_metrics_reject_duplicate_owners_and_scheduling_gaps(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            traffic = [{'id': 'dag-a', 'at': 0, 'seconds': .1},
                {'id': 'dag-b', 'at': .2, 'seconds': .1}]

            def save(runtime, identifier, started):
                run = root / runtime / 'dag' / identifier
                (run / 'execute').mkdir(parents=True, exist_ok=True)
                (run / 'execution.json').write_text(json.dumps({'assignment': {'index': {'created_at': 0}}}))
                (run / 'execute/meta.json').write_text(json.dumps({'outcome': 'done', 'started_at_ms': started}))

            save('r1', 'dag-a', 100)
            save('r2', 'dag-b', 300)
            runtimes = [root / 'r1', root / 'r2']
            self.assertEqual(verify(runtimes, traffic)['metrics']['max_scheduling_gap_seconds'], .2)
            save('r2', 'dag-b', 2100)
            with self.assertRaisesRegex(AssertionError, 'scheduling_gap'):
                verify(runtimes, traffic)
            save('r2', 'dag-a', 100)
            with self.assertRaisesRegex(AssertionError, 'exactly one Runtime'):
                verify(runtimes, traffic)


if __name__ == '__main__':
    unittest.main()
