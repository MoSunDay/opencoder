"""Reject corrupt evidence instead of turning status-only success green."""
import importlib.util
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from assertions import barriers, output

spec = importlib.util.spec_from_file_location('native_fixture', Path(__file__).resolve().parents[1] / 'fixtures/native.py')
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)


class EvidenceTests(unittest.TestCase):
    def test_real_arithmetic_fails_for_signed_inputs_then_passes_after_repair(self):
        bad = native.calculate('edge', 'abs(a) + abs(b)')
        fixed = native.calculate('edge', 'a + b')
        self.assertFalse(bad['passed'])
        self.assertEqual(bad['failures'][0], {'a': -2, 'b': 3, 'expected': 1, 'actual': 5})
        self.assertTrue(fixed['passed'])
        self.assertNotEqual(bad['revision'], fixed['revision'])
        self.assertEqual(bad['summary'], fixed['summary'])

    def test_ordinary_tests_do_not_accidentally_detect_the_boundary_fixture_early(self):
        self.assertTrue(native.calculate('fast', 'abs(a) + abs(b)')['passed'])
        self.assertTrue(native.calculate('hold', 'abs(a) + abs(b)')['passed'])

    def test_model_expression_cannot_access_files_or_import_modules(self):
        for text in ['__import__("os")', 'open("secret")', 'a.__class__', '[a for a in b]', 'abs(a, b)']:
            with self.subTest(text=text), self.assertRaises((ValueError, SyntaxError)):
                native.expression(text)

    def test_revision_is_checked_against_actual_source(self):
        result = native.calculate('edge', 'a + b')
        detail = {'execution': {'status': 'done'}, 'result': {'scheduler_output': {'check': result}}}
        self.assertTrue(output(detail)['passed'])
        detail['result']['scheduler_output']['check']['revision'] = 'stale'
        with self.assertRaisesRegex(AssertionError, 'revision'):
            output(detail)

    def test_execution_error_is_not_a_successful_output(self):
        with self.assertRaisesRegex(AssertionError, 'successfully'):
            output({'execution': {'status': 'error'}})

    @staticmethod
    def trace():
        return {'plan': {'layers': [{'layer_id': 'a'}, {'layer_id': 'b'}],
            'nodes': [{'node_id': 'first', 'layer_id': 'a'}, {'node_id': 'second', 'layer_id': 'b'}]},
            'operations': [{'node_id': 'first', 'activation': 1, 'execution_id': 'e1'},
                           {'node_id': 'second', 'activation': 2, 'execution_id': 'e2'}],
            'events': [
                {'seq': 1, 'event_type': 'layer_started', 'layer': 1, 'activation': 1, 'round': 1},
                {'seq': 2, 'event_type': 'operation_terminal', 'execution_id': 'e1', 'activation': 1},
                {'seq': 3, 'event_type': 'layer_barrier_reached', 'activation': 1},
                {'seq': 4, 'event_type': 'layer_started', 'layer': 2, 'activation': 2, 'round': 1}]}

    def test_missing_terminal_cannot_be_hidden_by_a_barrier_event(self):
        view = self.trace()
        self.assertEqual(len(barriers(view)), 2)
        del view['events'][1]
        with self.assertRaisesRegex(AssertionError, 'terminal receipt'):
            barriers(view)

    def test_barrier_after_dispatch_is_rejected(self):
        view = self.trace()
        view['events'][2]['seq'] = 5
        view['events'].sort(key=lambda e: e['seq'])
        with self.assertRaisesRegex(AssertionError, 'unfinished barrier'):
            barriers(view)

    def test_duplicate_ids_and_missing_parallel_nodes_are_rejected(self):
        for mutate in [lambda v: v['operations'][1].update(execution_id='e1'),
                       lambda v: v['plan']['nodes'].append({'node_id': 'missing', 'layer_id': 'a'})]:
            view = self.trace()
            mutate(view)
            with self.assertRaises(AssertionError):
                barriers(view)

    def test_completed_label_cannot_hide_an_unfinished_final_layer(self):
        view = self.trace()
        view['run'] = {'phase': 'completed', 'valid_layers': 2}
        view['events'].append({'seq': 5, 'event_type': 'run_completed'})
        with self.assertRaisesRegex(AssertionError, 'final barrier'):
            barriers(view)


if __name__ == '__main__':
    unittest.main()
