from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling import ingress


def stat(pid, ticks, state='S'):
    fields = [state] + ['0'] * 18 + [str(ticks)]
    return f'{pid} (worker (with spaces)) ' + ' '.join(fields)


class IngressTests(unittest.TestCase):
    def test_switch_waits_for_acceptance_to_stop_without_waiting_for_node_channels(self):
        with tempfile.TemporaryDirectory() as directory:
            proc = Path(directory)
            root = proc / '11'
            root.mkdir()
            (root / 'stat').write_text(stat(11, 111))
            (root / 'cmdline').write_bytes(b'nginx: worker process\0')
            (root / 'net').mkdir()
            (root / 'fd').mkdir()
            (root / 'net/tcp').write_text('header\n0: local remote 0A 0 0 0 0 0 999\n')
            listener = root / 'fd/6'
            listener.symlink_to('socket:[999]')
            (root / 'fd/7').symlink_to('socket:[1000]')
            workers = [{'pid': 11, 'start_ticks': 111}]
            self.assertFalse(ingress.switched(workers, proc))
            (root / 'cmdline').write_bytes(b'nginx: worker process is shutting down\0')
            self.assertFalse(ingress.switched(workers, proc))
            listener.unlink()
            self.assertTrue(ingress.switched(workers, proc))
            self.assertFalse(ingress.drained(workers, proc))
            (root / 'cmdline').write_bytes(b'nginx: worker process\0')
            (root / 'stat').write_text(stat(11, 112))
            self.assertTrue(ingress.switched(workers, proc))
            (root / 'stat').unlink()
            self.assertTrue(ingress.switched(workers, proc))

    def test_snapshot_includes_only_live_nginx_workers_and_tracks_pid_reuse(self):
        with tempfile.TemporaryDirectory() as directory:
            proc = Path(directory)
            children = proc / '10/task/10/children'
            children.parent.mkdir(parents=True)
            children.write_text('11 12 13')
            for pid, command, state in [(11, b'nginx: worker process', 'S'),
                                        (12, b'nginx: cache manager process', 'S'),
                                        (13, b'nginx: worker process is shutting down', 'Z')]:
                root = proc / str(pid)
                root.mkdir()
                (root / 'cmdline').write_bytes(command)
                (root / 'stat').write_text(stat(pid, 100 + pid, state))
            workers = ingress.snapshot(10, proc)
            self.assertEqual(workers, [{'pid': 11, 'start_ticks': 111}])
            self.assertFalse(ingress.drained(workers, proc))
            (proc / '11/stat').write_text(stat(11, 112))
            self.assertTrue(ingress.drained(workers, proc))
            (proc / '11/stat').write_text('invalid process state')
            with self.assertRaises((IndexError, ValueError)):
                ingress.drained(workers, proc)

    def test_new_server_receives_the_complete_ingress_frontier(self):
        operations = Mock()
        operations.http.side_effect = [{'retirement_protocol': 2}, {'retiring': True}]
        workers = [{'pid': 11, 'start_ticks': 111}]
        self.assertTrue(ingress.retire(operations, 'http://server', workers, 1234))
        operations.http.assert_called_with('http://server', '/api/admin/release/retire',
                                          'POST', {'ingress_workers': workers, 'successor_port': 1234})
        operations.ingress_drained.assert_not_called()

    def test_legacy_listener_is_preserved_until_its_ingress_workers_exit(self):
        operations = Mock()
        operations.http.return_value = {'instance_release': 'legacy'}
        operations.ingress_drained.return_value = False
        workers = [{'pid': 11, 'start_ticks': 111}]
        self.assertFalse(ingress.retire(operations, 'http://server', workers, 1234))
        operations.http.assert_called_once_with('http://server', '/api/admin/release')
        operations.ingress_drained.return_value = True
        self.assertTrue(ingress.retire(operations, 'http://server', workers, 1234))
        operations.http.assert_called_with('http://server', '/api/admin/release/retire', 'POST', {})
