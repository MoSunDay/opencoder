"""Observe the isolated service after switching and rollback with real work."""
import json
import time
from . import ontology


def observe(env, proof, definition, seconds, completed, until):
    started = time.monotonic()
    deadline = started + seconds
    samples = []
    receipt = env.root / 'observation.json'
    while time.monotonic() < deadline:
        sample_started = time.monotonic()
        ontology.verify(env, proof)
        identifier = f'dag-observation-{len(samples)}'
        env.api('/api/executions', 'POST', {
            'id': identifier, 'kind': 'dag', 'input': {'definition': definition}})
        until(lambda: completed(env, identifier), 'observation task completed')
        elapsed = time.monotonic() - sample_started
        assert elapsed < 30, f'observation task exceeded 30 seconds: {elapsed}'
        samples.append({'id': identifier, 'elapsed_seconds': elapsed,
                        'at_seconds': time.monotonic() - started})
        receipt.write_text(json.dumps({'passed': False, 'required_seconds': seconds,
                                      'samples': samples}, indent=2))
        print(f'observation: {time.monotonic() - started:.1f}/{seconds}s, '
              f'{len(samples)} tasks completed', flush=True)
        time.sleep(max(0, min(15, deadline - time.monotonic())))
    result = {'passed': True, 'required_seconds': seconds,
              'elapsed_seconds': time.monotonic() - started, 'samples': samples}
    receipt.write_text(json.dumps(result, indent=2))
    return result
