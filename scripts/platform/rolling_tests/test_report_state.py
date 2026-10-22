"""Report-state writers require a verified rollback reader."""
from copy import deepcopy
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.manifest import compatible, report_state, verify, writer_preflight, _installer


def release(name, reader=0, writer=0):
    return dict(release_id=name, protocol_version=10, brain_schema_version=7,
                storage_engine='turso', execution_io=dict(reader=1, writer=1),
                report_state=dict(reader=reader, writer=writer),
                compatibility=dict(protocol=dict(min=1, max=1), data_format=dict(min=12 if writer else 11, max=12 if writer else 11)))


class ReportStateTests(unittest.TestCase):
    def test_writer_cannot_claim_the_legacy_format(self):
        forged = release('writer', 1, 1)
        forged['compatibility']['data_format'] = dict(min=11, max=11)
        with self.assertRaisesRegex(ValueError, 'requires handoff format 12'):
            compatible(forged, [])
        forged = release('reader', 1)
        forged['compatibility']['data_format'] = dict(min=12, max=12)
        with self.assertRaisesRegex(ValueError, 'format 12 requires'):
            compatible(forged, [])

    def test_writer_requires_reader_and_rollback_preserves_retained_runtime(self):
        legacy, bridge, writer = release('old'), release('reader', 1), release('writer', 1, 1)
        legacy.pop('report_state')
        self.assertEqual(report_state(legacy), dict(reader=0, writer=0))
        writer_preflight(bridge, legacy)
        writer_preflight(writer, bridge)
        with self.assertRaisesRegex(ValueError, 'report state reader bridge'):
            writer_preflight(writer, legacy)
        compatible(writer, [legacy, bridge])
        compatible(bridge, [writer])
        with self.assertRaisesRegex(ValueError, 'incompatible rollback'):
            compatible(legacy, [writer])

    def test_invalid_or_forged_metadata_is_rejected(self):
        for value in (None, {}, dict(reader=0, writer=1), dict(reader=True, writer=0), dict(reader=2, writer=0)):
            with self.subTest(value=value), self.assertRaises(ValueError):
                report_state(dict(report_state=value))
        candidate = release('writer', 1, 1)
        info = dict(candidate, release_compatibility=candidate['compatibility'])
        with patch.object(_installer, 'verify_bundle', return_value=candidate), \
                patch.object(_installer, 'bundle_names', return_value=_installer.NAMES), \
                patch.object(_installer, 'build_info', return_value=info):
            self.assertEqual(verify(Path('bundle')), candidate)
            forged = deepcopy(candidate)
            forged['report_state']['writer'] = 0
            with patch.object(_installer, 'verify_bundle', return_value=forged):
                with self.assertRaisesRegex(ValueError, 'compiled binary'):
                    verify(Path('bundle'))

    def test_historical_format_eleven_writer_is_readable_but_not_republished(self):
        historical = release('historical', 1, 1)
        historical['compatibility']['data_format'] = dict(min=11, max=11)
        writer, bridge = release('writer', 1, 1), release('bridge', 1)
        compatible(writer, [historical, bridge])
        compatible(bridge, [historical, writer])
        writer_preflight(writer, historical)
        with self.assertRaisesRegex(ValueError, 'requires handoff format 12'):
            compatible(historical, [writer])
        with self.assertRaisesRegex(ValueError, 'incompatible rollback'):
            compatible(release('legacy'), [historical])
        malformed = deepcopy(historical)
        malformed['report_state']['reader'] = 0
        with self.assertRaisesRegex(ValueError, 'unsupported report state'):
            compatible(writer, [malformed])
