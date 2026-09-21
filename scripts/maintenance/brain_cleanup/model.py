"""Pure selection rules for removing retired Brain data, never credentials."""
import hashlib
import json


def object_value(value):
    return value if isinstance(value, dict) else {}


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def execution_scope(assignments, persisted_roots=()):
    roots = {key for key, row in assignments.items()
             if row.get('request', {}).get('kind') == 'brain'
             and object_value(row.get('request', {}).get('input')).get('schema_version') != 4}
    roots.update(persisted_roots)
    selected = set(roots)
    while True:
        previous = len(selected)
        for key, row in assignments.items():
            data = object_value(row.get('request', {}).get('input'))
            parent = (object_value(data.get('brain_scheduler')).get('run_id')
                      or object_value(object_value(data.get('_brain')).get('parent')).get('run_id')
                      or object_value(data.get('brain_receipt')).get('run_id')
                      or object_value(data.get('brain_origin')).get('run_id'))
            if parent in selected:
                selected.add(key)
        if len(selected) == previous:
            return roots, selected & assignments.keys()


# Explicit allowlist. Authentication, capabilities, shared definitions and v4
# projections are deliberately absent. SQL names never come from a manifest.
ID_COLUMNS = {
    'execution_index': ('id',), 'execution_assignments': ('id',),
    'execution_ownership': ('execution_id', 'brain_run_id'),
    'runtime_owners': ('execution_id',), 'brain_runs': ('id',),
    'brain_run_events': ('run_id',), 'brain_resource_claims': ('execution_id', 'run_id'),
    'brain_scheduler_runs': ('run_id',), 'brain_scheduler_operations': ('run_id', 'execution_id'),
    'brain_scheduler_events': ('run_id', 'execution_id'),
    'sessions': ('id',), 'messages': ('session_id',),
    'session_inputs': ('session_id',), 'session_events': ('session_id',),
    'subagent_tasks': ('parent_session_id', 'child_session_id'),
    'todo_workflows': ('id',), 'todo_items': ('workflow_id',), 'todo_events': ('workflow_id',),
    'dag_runs': ('id',), 'dag_events': ('run_id',), 'team_topic_runs': ('topic_id',),
}


RETIRED_TABLES = {'brain_plans', 'brain_playbooks', 'brain_runs', 'brain_run_events',
                  'brain_resource_claims', 'brain_scheduler_runs', 'brain_scheduler_operations',
                  'brain_scheduler_events'}
ALLOWED_TABLES = set(ID_COLUMNS) | RETIRED_TABLES | {'brain_plan_versions', 'fleet_definitions', 'dispatch_receipts'}


def selected_row(table, row, executions, plans):
    if table in RETIRED_TABLES:
        return True
    if table in ID_COLUMNS:
        return any(row.get(key) in executions for key in ID_COLUMNS[table])
    if table == 'brain_plan_versions':
        return (row['id'], row['version']) in plans
    if table == 'fleet_definitions':
        return row['kind'] == 'brain_plan' and row['id'] in {key for key, _ in plans}
    if table == 'dispatch_receipts':
        return row['scope'] in ('execution', 'brain-run', 'brain-control') and row['id'] in executions
    return False
