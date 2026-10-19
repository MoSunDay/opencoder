"""Deterministic cleanup selection; no names or paths are special-cased."""
from collections import defaultdict
from datetime import datetime
import hashlib
import json
import re


class ReferenceNames(frozenset):
    def __new__(cls, names):
        result = super().__new__(cls, names)
        result.literal_names = tuple(n for n in result if not re.fullmatch(r'[\w.-]+', n))
        return result


def digest(value):
    return hashlib.sha256(json.dumps(value, ensure_ascii=False, sort_keys=True,
                                    separators=(',', ':')).encode()).hexdigest()


def stamp(value):
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return value / 1000 if value > 10**11 else value
    if isinstance(value, str):
        try:
            # Python 3.8 accepts milliseconds/microseconds, while Rust stores
            # nanoseconds. Truncation retains subsecond retention precision.
            normalized = re.sub(r'(\.\d{6})\d+(?=[Z+-])', r'\1', value)
            parsed = datetime.fromisoformat(normalized.replace('Z', '+00:00'))
            return parsed.timestamp() if parsed.tzinfo else None
        except ValueError:
            return None
    return None


def mentions(value, names, dag=False):
    if isinstance(value, dict):
        return set().union(*(mentions(v, names, dag) for v in value.values())) if value else set()
    if isinstance(value, list):
        return set().union(*(mentions(v, names, dag) for v in value)) if value else set()
    if not isinstance(value, str):
        return set()
    tokens = set(re.findall(r'[\w.-]+', value))
    found = tokens & names
    if dag:
        found |= {token[4:] for token in tokens if token.startswith('dag-') and token[4:] in names}
    # Definitions may use names containing spaces/punctuation. Protect them
    # conservatively instead of assuming ASCII resource-name validation applies.
    literal_names = names.literal_names if isinstance(names, ReferenceNames) else ReferenceNames(names).literal_names
    found |= {n for n in literal_names if n in value}
    return found


def dag_key(definition):
    spec = dict(definition.get('spec', definition))
    spec.pop('name', None)
    # Do not erase descriptions, dependency order, arguments, timeouts or pins.
    return digest(spec)


def pool_key(pool):
    versions = pool.get('history', [])
    if not versions or any(not v or not v.get('sha256') for v in versions):
        return None
    return digest({'current': pool['current'], 'versions': [
        [v['version'], v['sha256'], v['size_bytes']] for v in versions]})


def select(objects, references, last_used, successful, now, retention_days, key):
    """Referenced items stay. Independent idle items require known timestamps."""
    groups = defaultdict(list)
    for name, value in objects.items():
        identity = key(value)
        if identity:
            groups[identity].append(name)
    redundant = {}
    for group in groups.values():
        if len(group) < 2:
            continue
        ranked = sorted(group, key=lambda n: (
            not bool(references.get(n)), -successful.get(n, 0),
            stamp(objects[n].get('created_at')) or float('inf'), n))
        keeper = ranked[0]
        for name in ranked[1:]:
            if not references.get(name):
                redundant[name] = keeper
    cutoff = now - retention_days * 86400
    selected, retained = [], []
    for name, value in sorted(objects.items()):
        refs = sorted(references.get(name, []))
        times = [stamp(value.get('created_at')), stamp(value.get('updated_at'))]
        used = last_used.get(name, 0)
        known = all(t is not None for t in times) and used is not None
        old = known and max(*times, used) < cutoff
        if refs or not known or (name not in redundant and not old):
            retained.append({'name': name, 'references': refs,
                             'reason': 'referenced' if refs else 'recent-or-unknown'})
            continue
        selected.append({'name': name, 'reason': 'duplicate' if name in redundant else 'idle',
                         'keep': redundant.get(name), 'snapshot': value, 'digest': digest(value),
                         'last_used': used})
    # Do not also remove the keeper as idle while its duplicate is selected.
    keepers = {item['keep'] for item in selected if item['keep']}
    selected = [item for item in selected if item['name'] not in keepers]
    return selected, retained
