"""Deterministic decisions; the Server, Runtime and all child executions are real."""
import json

MARKER = 'history-actual-input'
RESULT = 'history-child-result'


def plan():
    nodes = [('agent', 'builtin-agent-act', 1), ('dag', 'dag-history-artifact', 1),
             ('verify', 'builtin-agent-act', 2)]
    return {'schema_version': 7, 'title': 'Run history acceptance',
        'objective': 'Collect results, return once, and verify the second round.',
        'inputs': {'request': 'history-plan-default'}, 'max_rounds': 3,
        'layers': [{'layer_id': f'layer-{i}', 'title': title, 'task': title,
            'objective': title, 'success_criteria': 'Complete with evidence.'}
            for i, title in [(1, 'Collect evidence'), (2, 'Verify evidence')]],
        'nodes': [{'node_id': name, 'title': name, 'layer_id': f'layer-{layer}',
            'objective': 'Return a short result without tools.', 'capability_id': capability}
            for name, capability, layer in nodes],
        'transitions': [{'from': 'layer-1', 'to': 'layer-2', 'condition': 'Evidence ready'},
            {'from': 'layer-2', 'to': 'layer-1', 'condition': 'Repeat once'}], 'edges': []}


def answer(prompt):
    try:
        context = json.loads(prompt)
    except (ValueError, TypeError):
        return RESULT
    if not isinstance(context, dict) or 'plan' not in context or 'run' not in context:
        return RESULT
    layer, round_number = context['run']['layer'], context['run']['round']
    back = layer == 2 and round_number == 1
    assessments = {} if not layer else {f'layer-{layer}': {
        'met': not back, 'reason': 'Repeat to verify history.' if back else 'Result verified.'}}
    common = {'reason': 'History acceptance decision.', 'evidence_execution_ids': [],
              'assessments': assessments}
    if layer == 2 and not back:
        return {**common, 'decision': 'complete', 'summary': 'History acceptance complete.'}
    target = 1 if back else layer + 1
    assignments = [{'node_id': node['node_id'], 'capability_id': node['capability_id'],
        'inputs': {}, 'reason': 'Collect verifiable evidence.'} for node in context['plan']['nodes']
        if node['layer_id'] == f'layer-{target}']
    return {**common, 'decision': 'dispatch_layer', 'layer': target, 'assignments': assignments,
            **({'reflection': 'Verify a second round while preserving prior evidence.'} if back else {})}
