import unittest
from metrics import summarize, throughput_verdict, verdict


def result(label, cpu):
    return dict(label=label, aggregate_cpu_percent=cpu, query_p95_ms=10, fixture_version=2,
                requests=100, history=2000, payload_count=10, payload_bytes=1024,
                response_digest='same', inventory_digest='same', errors=[],
                schedule_lag_max_seconds=.01)


def throughput_rows():
    return [dict(label=label, round=turn, sha256=label, completed_tasks=160,
                 verified_snapshots=160, task_round_seconds=[seconds / 5] * 5)
            for turn, duration in enumerate([1.06, .9, .9])
            for label, seconds in [('before', 1), ('after', duration)]]


class MetricsTests(unittest.TestCase):
    def test_moving_cpu_between_processes_does_not_count_as_reduction(self):
        before = {role: dict(start='1', ticks=0, rss_bytes=0)
                  for role in ('server', 'host', 'runtime')}
        after = {role: dict(start='1', ticks=ticks, rss_bytes=0)
                 for role, ticks in [('server', 0), ('host', 0), ('runtime', 100)]}
        total = summarize(before, after, 1)['aggregate_cpu_percent']
        after['server']['ticks'], after['runtime']['ticks'] = 100, 0
        self.assertEqual(summarize(before, after, 1)['aggregate_cpu_percent'], total)

    def test_failures_and_different_workloads_cannot_pass(self):
        for field, value in [('errors', ['timeout']), ('requests', 99), ('fixture_version', 1),
                             ('response_digest', 'different'), ('inventory_digest', 'different'),
                             ('schedule_lag_max_seconds', 2)]:
            changed = result('after', 1)
            changed[field] = value
            with self.assertRaises(AssertionError):
                verdict([result('before', 100), changed])

    def test_latency_and_cpu_are_both_gates(self):
        self.assertTrue(verdict([result('before', 100), result('after', 40)])['passed'])
        self.assertFalse(verdict([result('before', 100), result('after', 60)])['passed'])
        changed = result('after', 40)
        changed['query_p95_ms'] = 12
        self.assertFalse(verdict([result('before', 100), changed])['passed'])

    def test_restart_invalidates_sample(self):
        with self.assertRaises(AssertionError):
            summarize({'server': dict(start='1', ticks=10)},
                      {'server': dict(start='2', ticks=100)}, 1)

    def test_throughput_includes_the_initial_failed_sample_in_all_elapsed_time(self):
        rows = throughput_rows()
        self.assertFalse(throughput_verdict(rows[:2], 1)['passed'])
        combined = throughput_verdict(rows, 3)
        self.assertAlmostEqual(combined['throughput_ratio'], 3 / 2.86)
        self.assertEqual(combined['verified_tasks_per_binary'], 480)
        self.assertTrue(combined['passed'])

    def test_throughput_rejects_omitted_rounds_changed_binaries_and_lost_tasks(self):
        with self.assertRaises(AssertionError):
            throughput_verdict(throughput_rows()[2:], 3)
        for key, value in [('round', 1), ('sha256', 'changed'), ('completed_tasks', 159),
                           ('verified_snapshots', 159), ('task_round_seconds', [1])]:
            rows = throughput_rows()
            rows[0][key] = value
            with self.assertRaises(AssertionError):
                throughput_verdict(rows, 3)


if __name__ == '__main__':
    unittest.main()
