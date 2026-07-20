# Local Memory Examples

This file is a lightweight appendix, not the main rule source.  
Follow `SKILL.md` first.

Use this file mainly when:
- initializing local memory for an existing repository;
- the structure is highly ambiguous;
- the model repeatedly makes mistakes such as over-writing changelog, over-updating indexes, or writing task diaries into stable docs.

---

## Positive Example 1: Internal refactor with no business change
### Change
A parser implementation is replaced, but responsibility, boundary, and external behavior stay the same.

### Correct Action
- Usually no changelog if the change is small and not retrieval-worthy.
- Update `agents/{module}/index.md` only if the old logic description becomes inaccurate.
- Do not update `features/*`.
- Do not update top-level indexes.

### Why
This may be a logic correction, but not necessarily a meaningful timeline topic.

---

## Positive Example 2: Meaningful refactor worth timeline record
### Change
A scheduling subsystem is restructured into a clearer coordinator-based architecture. External behavior is mostly unchanged, but the change is large, coherent, and likely useful to remember later.

### Correct Action
- Update relevant `agents/*` if responsibilities, boundaries, or main flows changed.
- Write a changelog entry if the refactor is substantial and worth future retrieval.
- Do not update `features/*` unless business behavior changed.
- Do not update top-level indexes unless the module or feature map changed.

### Why
A refactor can deserve changelog when it is meaningful and structured, not because every refactor must be logged.

---

## Positive Example 3: Important user-visible bug fix
### Change
Timeout failures used to be swallowed; now they surface as explicit actionable failures to callers.

### Correct Action
- Update the relevant `features/{feature}/index.md`.
- Update `agents/{module}/index.md` if failure-handling flow changed materially.
- Add changelog if this bug fix is important enough to keep in timeline.
- Do not update top-level indexes unless the map changed.

### Why
This changes stable external behavior and may also be worth remembering historically.

---

## Positive Example 4: New logic module, no new feature
### Change
A shared coordinator module is extracted from a larger module. No new user-visible capability is introduced.

### Correct Action
- Create `agents/{new-module}/index.md`.
- Update related `agents/*`.
- Update `agents.md` because the logic map changed.
- Usually do not update `features/*`.
- Changelog is optional and depends on whether the extraction is meaningful enough to record.

### Why
Logic map changed; feature map may not have changed.

---

## Positive Example 5: New feature across multiple modules
### Change
A batch export capability is introduced across API, orchestration, and storage areas.

### Correct Action
- Create or update `features/{feature}/index.md`.
- Link to multiple relevant `agents/*`.
- Update `features/index.md`.
- Update relevant `agents/*` if logic responsibilities or flows changed.
- Add changelog if the feature is a meaningful timeline event.

### Why
A feature can span multiple modules and may deserve both stable-doc and changelog updates.

---

## Negative Example 1: Every change creates changelog
### Bad
Every rename, cleanup, tiny fix, or isolated refactor creates a changelog entry.

### Why Bad
This turns changelog into low-signal noise.

### Correct
Only write changelog when the change is meaningful, coherent, or retrieval-worthy.

---

## Negative Example 2: New changelog forces index churn
### Bad
After adding one changelog entry, the agent updates:
- `features/index.md`
- `agents.md`
- unrelated indexes

### Why Bad
Indexes become an activity feed instead of a stable map.

### Correct
Update indexes only when the logic map or feature map actually changes.

---

## Negative Example 3: Stable docs contain patch diary
### Bad
`agents/{module}/index.md` says:
- “This time we changed ...”
- “Then we adjusted ...”
- “Later we added ...”

### Why Bad
Stable docs should describe the present model, not narrate a patch.

### Correct
Rewrite the section as current responsibility, boundary, and main flow.

---

## Negative Example 4: Similar old changelog gets overwritten
### Bad
The agent finds an older changelog on a similar topic and edits that old file instead of creating a new time-anchored entry.

### Why Bad
This breaks timeline traceability and blurs when each change actually happened.

### Correct
Preserve timeline. Similar topics across time can coexist as separate entries.

---

## Negative Example 5: Changelog causes unnecessary stable-doc updates
### Bad
Because a changelog was created, the agent also updates `agents/*` or `features/*` with repetitive patch narrative even when current stable descriptions remain valid.

### Why Bad
This creates duplication and churn.

### Correct
Update stable docs only if the stable semantic model changed or the existing docs became inaccurate.