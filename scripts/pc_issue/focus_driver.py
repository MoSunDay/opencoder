"""Retry only a window focus whose exact foreground identity can be verified."""
import time

from controller.driver import Driver


FOCUS_ERRORS = frozenset({
    'foreground HWND differs from selected UI window',
    'selected UI window is not foreground after action',
})


def exact_foreground(windows, window_id):
    return any(str(window.get('id')) == str(window_id)
               and window.get('process', '').lower() == 'jianyingpro.exe'
               and window.get('is_foreground') is True for window in windows)


class CandidateDriver(Driver):
    def call(self, op, **args):
        if op != 'raise_window' or 'window_id' not in args:
            return super().call(op, **args)
        for attempt in range(3):
            try:
                return super().call(op, **args)
            except RuntimeError as error:
                if str(error) not in FOCUS_ERRORS:
                    raise
                observed = super().call('list_windows')['windows']
                if exact_foreground(observed, args['window_id']):
                    return {'ok': True, 'window_id': args['window_id'],
                            'verified_by': 'fresh_exact_foreground_observation'}
                if attempt == 2:
                    raise
                time.sleep(0.3)
