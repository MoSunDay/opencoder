# Any Home task-plan protocol

Use this protocol only when the current repository contains `scripts/any_home_planning.py`.

## Context query

Resolve one or more Any Home entity UUIDs that anchor the task, then run:

```bash
python3 scripts/any_home_planning.py --env debug context \
  --canonical-key '<task stable key>' \
  --anchor '<entity uuid>' \
  --upstream-depth 1 --downstream-depth 1
```

Keep the returned `query_record_id` and `graph_fingerprint`. Planning context is bounded to business entities; previous planning records are excluded.

## PlanRun input

Write a temporary JSON input outside the repository. Required shape:

```json
{
  "canonical_key": "repository:task-key",
  "title": "计划标题",
  "goal": "可验证目标",
  "source_fingerprint": "sha256 or canonical multi-source fingerprint",
  "sources": [{
    "source": "git",
    "identity": "/absolute/repository/path",
    "revision": "commit-or-diff-fingerprint",
    "evidence_level": "strong",
    "observed_at": "2026-08-17T08:00:00Z"
  }],
  "items": [{
    "key": "stable-item-key",
    "title": "计划项",
    "stage": "开发闭环",
    "priority": "P1",
    "status": "pending",
    "action": "要完成的动作",
    "completion_definition": "完成判定",
    "validation": "验收方法",
    "dependencies": [],
    "evidence": ["evidence-key"],
    "support_requests": [],
    "risk": "风险说明",
    "rollback": null
  }],
  "evidence": [{
    "key": "evidence-key",
    "title": "证据名称",
    "source": {
      "source": "git",
      "identity": "file-or-command",
      "revision": "fingerprint",
      "evidence_level": "strong",
      "observed_at": "2026-08-17T08:00:00Z"
    },
    "summary": "证据结论",
    "valid_until": null
  }],
  "support_requests": [{
    "key": "stable-gap-key",
    "gap_type": "data_gap",
    "title": "缺口标题",
    "request": "需要的数据、能力或判断",
    "assignee": null
  }],
  "target_entity_ids": ["entity-uuid"],
  "query_record_ids": ["query-record-uuid"],
  "supersedes": null
}
```

Constraints:

- `priority` is `P0` through `P3`.
- `gap_type` is `data_gap`, `capability_gap`, or `judgement_gap`.
- `evidence_level` is `strong`, `medium`, or `weak`.
- Dependencies must reference another item key and must be acyclic.
- Release-stage items require a non-empty rollback description.
- New SupportRequests always start `pending`; omit `status` in input.

Persist and read back:

```bash
python3 scripts/any_home_planning.py --env debug create --input "$PLAN_INPUT"
python3 scripts/any_home_planning.py --env debug get '<plan-run-uuid>' \
  --source-fingerprint '<current-source-fingerprint>'
```

The plan is acceptable for signing only when `fresh=true`, `graph_fresh=true`, and `source_fresh=true`. Missing current source fingerprint is unknown, not fresh.
