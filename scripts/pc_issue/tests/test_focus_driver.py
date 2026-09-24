from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, '/opt/device-cases/harness')
from controller.driver import Driver
from focus_driver import CandidateDriver


class FocusDriver(unittest.TestCase):
    def test_exact_foreground_confirms_ambiguous_action(self):
        observed = {'windows': [
            {'id': 44, 'process': 'JianyingPro.exe', 'is_foreground': True},
            {'id': 45, 'process': 'Other.exe', 'is_foreground': False},
        ]}
        with patch.object(Driver, 'call', side_effect=[
            RuntimeError('foreground HWND differs from selected UI window'), observed,
        ]) as call:
            result = CandidateDriver({}, {}, Path('/tmp')).call('raise_window', title='剪映专业版', window_id=44)
        self.assertEqual(result['verified_by'], 'fresh_exact_foreground_observation')
        self.assertEqual(call.call_count, 2)

    def test_other_foreground_never_passes(self):
        observed = {'windows': [{'id': 45, 'process': 'JianyingPro.exe', 'is_foreground': True}]}
        responses = [item for _ in range(3) for item in (
            RuntimeError('foreground HWND differs from selected UI window'), observed)]
        with patch.object(Driver, 'call', side_effect=responses), patch('focus_driver.time.sleep'):
            with self.assertRaisesRegex(RuntimeError, 'foreground HWND differs'):
                CandidateDriver({}, {}, Path('/tmp')).call('raise_window', title='剪映专业版', window_id=44)


if __name__ == '__main__':
    unittest.main()
