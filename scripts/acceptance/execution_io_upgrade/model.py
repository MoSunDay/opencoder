"""Deterministic decisions and held children for actual mixed-version tests."""
import hashlib
import json
import threading
import urllib.parse
import urllib.request

from fixture import Model as BaseModel


class Model(BaseModel):
    def __init__(self, root):
        self.legacy_release = threading.Event()
        self.consumer_release = threading.Event()
        self.decision_entered = threading.Event()
        self.decision_release = threading.Event()
        self.contexts = []
        self.downloads = []
        super().__init__(root)

    def config(self):
        config = super().config()
        # This deterministic fixture accepts the full two-layer evidence.
        config['context_limit'] = 1_000_000
        return config

    def answer(self, prompt):
        try:
            context = json.loads(prompt)
        except (ValueError, TypeError):
            context = None
        if isinstance(context, dict) and 'plan' in context and 'run' in context:
            self.contexts.append(context)
            layer = context['run']['layer']
            plan = context['plan']
            assessments = {plan['layers'][layer - 1]['layer_id']: {
                'met': True, 'reason': 'the complete child result passed'}} if layer else {}
            if layer == len(plan['layers']):
                return {'decision': 'complete', 'reason': 'both layers verified',
                        'summary': 'cross-version evidence verified', 'assessments': assessments}
            node = plan['nodes'][layer]
            inputs = {}
            if plan['title'] == 'modern artifacts' and layer == 1:
                if context['run']['run_id'] == 'brain-upgrade-dispatch':
                    self.decision_entered.set()
                    assert self.decision_release.wait(600), 'decision was not released after rollback'
                source = context['operations'][0]['execution_id']
                files = context['execution_artifacts'][source]
                assert any(f['file'] == 'report.txt' for f in files), files
                inputs = {'files': {'kind': 'execution', 'execution_id': source,
                                    'source': 'artifacts', 'path': ''}}
            return {'decision': 'dispatch_layer', 'layer': layer + 1,
                    'reason': 'dispatch the next fixed node', 'assessments': assessments,
                    'assignments': [{'node_id': node['node_id'],
                                     'capability_id': node['capability_id'],
                                     'inputs': inputs, 'reason': 'verify the contract'}]}
        if 'LEGACY_FIRST_GATE' in prompt:
            assert self.legacy_release.wait(600), 'legacy child was not released'
            return json.dumps({'passed': True, 'legacy': True})
        if 'IO_CONSUMER_GATE' in prompt:
            assert self.consumer_release.wait(600), 'artifact consumer was not released'
            marker = 'Layered inputs:\n'
            inputs, _ = json.JSONDecoder().raw_decode(prompt.split(marker, 1)[1])
            reference = next(f for f in inputs['files'] if f['file'] == 'report.txt')
            query = urllib.parse.urlencode({'step': reference['step'], 'file': reference['file']})
            request = urllib.request.Request(self.environment.settings.public_url +
                '/api/executions/' + reference['execution']['id'] + '/artifact?' + query,
                headers={'Authorization': 'Bearer ' + self.environment.token})
            with self.environment.opener.open(request, timeout=30) as response:
                content = response.read()
            assert len(content) == reference['bytes']
            assert hashlib.sha256(content).hexdigest() == reference['sha256']
            assert content == b'cross-version artifact\n'
            self.downloads.append(reference)
            return json.dumps({'passed': True, 'artifact_verified': True})
        return json.dumps({'passed': True})


def plan(modern=False):
    return {'schema_version': 7, 'title': 'modern artifacts' if modern else 'legacy transition',
            'objective': 'preserve frozen execution contracts across versions',
            'layers': [{'layer_id': name, 'title': name, 'task': name,
                        'objective': name, 'success_criteria': 'passed is true'}
                       for name in ['produce', 'verify']],
            'nodes': [{'node_id': 'produce', 'layer_id': 'produce', 'title': 'Produce',
                       'objective': 'write the declared artifact' if modern else 'LEGACY_FIRST_GATE',
                       'capability_id': 'dag-upgrade-io' if modern else 'builtin-agent-act'},
                      {'node_id': 'verify', 'layer_id': 'verify', 'title': 'Verify',
                       'objective': 'IO_CONSUMER_GATE' if modern else 'verify legacy result',
                       'capability_id': 'builtin-agent-act'}]}
