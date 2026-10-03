from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from rolling.maintenance import services


class StopTests(unittest.TestCase):
    def test_inactive_mount_has_no_main_process_property(self):
        class Operations:
            def run(self, *args):
                calls.append(args)

            def output(self, *args):
                return 'ActiveState=inactive\n'

            def wait(self, check, seconds):
                self_outer.assertTrue(check())

        calls, self_outer = [], self
        services.stop_unit('explicit.mount', Operations(), 10)
        self.assertEqual(calls, [('systemctl', '--no-block', 'stop', 'explicit.mount')])

    def test_nonterminating_graceful_stop_forces_only_the_explicit_unit(self):
        class Operations:
            def __init__(self):
                self.calls = []
                self.reaped = False

            def run(self, *args):
                self.calls.append(args)
                if '--signal=SIGKILL' in args:
                    self.reaped = True

            def output(self, *args):
                return ('ActiveState=inactive\nMainPID=0\n' if self.reaped
                        else 'ActiveState=deactivating\nMainPID=123\n')

            def wait(self, check, seconds):
                self_outer.assertFalse(check())
                self_outer.assertFalse(check())
                self_outer.assertTrue(check())

        self_outer = self
        operations = Operations()
        with patch.object(services.time, 'monotonic', side_effect=[0, 1, 6]):
            services.stop_unit('explicit.service', operations, 10)
        self.assertEqual(operations.calls, [
            ('systemctl', '--no-block', 'stop', 'explicit.service'),
            ('systemctl', 'kill', '--kill-who=main', '--signal=SIGTERM', 'explicit.service'),
            ('systemctl', 'kill', '--kill-who=all', '--signal=SIGKILL', 'explicit.service'),
        ])

    def test_force_stop_reply_loss_requires_reaped_main_process(self):
        for pid in ('0', '123'):
            with self.subTest(pid=pid):
                class Operations:
                    def run(self, *args):
                        if '--signal=SIGKILL' in args:
                            raise subprocess.CalledProcessError(1, args)

                    def output(self, *args):
                        return next(states)

                    def wait(self, check, seconds):
                        return check()

                states = iter(['ActiveState=deactivating\nMainPID=123\n',
                               f'ActiveState=failed\nMainPID={pid}\n'])
                with patch.object(services.time, 'monotonic', side_effect=[0, 6]):
                    if pid == '0':
                        services.stop_unit('explicit.service', Operations(), 10)
                    else:
                        with self.assertRaises(subprocess.CalledProcessError):
                            services.stop_unit('explicit.service', Operations(), 10)


if __name__ == '__main__':
    unittest.main()
