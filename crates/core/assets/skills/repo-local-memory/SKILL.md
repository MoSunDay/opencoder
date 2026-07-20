---
name: repo-local-memory
description: Maintain repository local memory docs so `agents.md`, `agents/*`, `features/*`, and changelog entries stay accurate after code changes. Use when initializing local memory, repairing touched memory, or updating memory after feature, bug-fix, or refactor work.
---

# Repository Local Memory Maintainer
## Purpose

Maintain repository local memory so it reflects the current stable semantic model of the codebase and stays useful for later development, debugging, and refactoring.
Local memory is for:
- quickly recovering repository context
- understanding module responsibilities, boundaries, dependencies, and main flows
- understanding user-visible capabilities, rules, and states
- analyzing change impact across iterations

Local memory is not:
- git history
- a PR summary
- a commit log
- a task diary

## Repository-First Policy
If the repository already defines explicit local-memory rules in `AGENTS.md`, `agents.md`, or nearby instructions, follow those repository rules first for document structure, naming, scope, language, indexing, and changelog policy.
Use this skill as the default fallback and consistency layer when:
- the repo has no clear local-memory contract
- the repo contract is incomplete
- you need help deciding the minimum accurate update set

Treat existing memory docs as auditable inputs, not as guaranteed truth.
If repository docs are stale, inaccurate, weak, or internally inconsistent, repair them against the code while preserving the repository's explicit governance rules.
Do not use this skill to override stricter repository-specific rules, but do use it to correct low-quality repository memory content.

## Language Policy
Unless the user explicitly requests otherwise, or the repository already has a strong and consistent convention, write newly created or updated local memory docs in Simplified Chinese.

Keep file paths, code symbols, class names, function names, config keys, protocol fields, API names, and commit sha unchanged.

## When To Use

Use this skill when:
- initializing local memory for an existing repository
- updating memory after feature work, bug fixes, refactors, or other meaningful code changes
- repairing memory for areas touched by the current task when that memory is missing, stale, misleading, or low quality

Default mode is repair-on-touch:
- if the current task touches an area and its memory is weak, repair it now
- if the task does not touch an area, do not rewrite it unless the task is explicitly about memory governance

Default inspection scope is local:
- inspect only the touched modules, touched features, directly related memory docs, and required parent indexes
- escalate to repository-wide inspection only for initialization, explicit repository-wide governance requests, or clear map drift such as module or feature split, merge, add, or remove
- do not scan unrelated modules merely to confirm a local update

## Output Scope
Stable layer:
- `agents.md`
- `agents/{module}/index.md`
- `agents/{module}/{submodule}/index.md` when needed
- `features/index.md`
- `features/{feature}/index.md`
Change layer:
- `features/changelog/YYYY-MM-DD/{topic}.md`

## Core Model

### 1. Stable Docs Describe Current State

Stable docs describe what the system is now, not how the task was executed.

### 2. Logic And Business Stay Separate

- `agents/*` describes logic structure: responsibilities, boundaries, abstractions, dependencies, and main flows
- `features/*` describes business behavior: user-visible capability, rules, states, and external behavior

They are related but not required to mirror each other one-to-one.

### 3. Changelog Preserves Timeline

`features/changelog/*` records meaningful time-anchored change topics. Do not retroactively merge a new change into an older entry just because the topic is similar.

### 4. Minimal Necessary Update

Update only the smallest set of docs required to keep memory accurate and useful.

### 5. Reliability Beats Coverage

Prefer fewer, denser, trusted docs over broad but low-value coverage.

## Documentation Baseline
Every local memory document must start with `Commit: <documentation baseline git commit sha>`.
This sha marks the code baseline used to verify that document.
Rules:
- preserve the repository's existing baseline convention when one already exists
- do not rewrite untouched docs only to refresh the sha
- do not omit the baseline line on new or updated memory docs

## Evidence Priority
When judging or repairing local memory, prefer evidence in this order:
1. code, config, schemas, APIs, and declared interfaces
2. tests, fixtures, and executable examples
3. existing local memory docs only as supporting context

Never preserve a claim only because it already exists in memory docs.

## Document Responsibilities
### `agents.md`

Repository-level logic map. Keep only:

- repository overview
- high-level module index
- relative links to relevant `agents/*`
- relative link to `features/index.md`

Do not put implementation detail, changelog enumeration, or patch history here.

Update only when the logic map changes.

### `agents/{module}/index.md`

Module-level logic doc. Describe:

- responsibility
- boundary and non-goals
- key abstractions
- main flow
- dependencies and interfaces
- related modules
- representative code anchors when useful

Do not put business value, task diary, or changelog-style history here.

If the document approaches 400 lines, split by semantic boundary. The parent stays as overview and index; child docs hold details.

### `features/index.md`

Repository-level capability map. Keep only:

- major capability groups
- relative links to `features/*`
- a stable link to the changelog root or entry point

Do not turn this file into an activity feed.
Never list dated changelog entries from `features/changelog/YYYY-MM-DD/*` directly in `features/index.md`; link only the changelog root or a stable changelog index entry point.

Update only when the feature map changes.

### `features/{feature}/index.md`

Feature-level business doc. Describe:

- user- or caller-visible capability
- triggers or actors
- behavior and rules
- key states and error surface
- constraints and edge cases
- links to one or more related `agents/*`

Do not put class walkthroughs, directory implementation detail, or patch narrative here.

### `features/changelog/YYYY-MM-DD/{topic}.md`

A changelog entry records a meaningful change topic from that time point.

Recommended sections:

- Context
- Change Summary
- Impact Surface
- Notes / Compatibility
- Related Docs

Do not use changelog as a commit-by-commit diary or a full current-state rewrite.

## Update Rules

Update `agents/*` only when one of these changed:

- module responsibility
- module boundary
- key abstraction
- main flow
- key dependency or interface
- the existing logic doc is materially inaccurate

Update `features/*` only when one of these changed:

- user-visible capability
- business rule
- key state
- error semantics
- external contract
- the existing feature doc is materially inaccurate

Write `features/changelog/*` when the change is retrieval-worthy, for example:

- a coherent feature change
- a meaningful bug fix
- a module-level or architecture-level refactor
- a structural or operational change worth remembering
- a notable cleanup or reorganization with lasting value

Usually skip changelog for:

- tiny isolated edits
- trivial rename, move, or cleanup with no broader value
- test-only changes
- doc-only corrections
- purely local fixes with no meaningful retrieval value

Update top-level indexes only if the map changes:

- module added, removed, split, or merged -> update `agents.md`
- feature added, removed, split, or merged -> update `features/index.md`

Do not update top-level indexes merely because a new changelog entry exists.
Do not add dated changelog bullets to top-level indexes; if a changelog needs an entry point, use the changelog root or a stable changelog index file instead.

## Splitting Rules

Create a separate logic module doc only if at least two are true:

- it has a stable responsibility
- it has a clear boundary
- it owns meaningful interfaces, dependencies, or data
- it is a frequent hotspot for understanding or change
- it has a distinct main flow or abstraction worth naming

Create a separate feature doc only if at least two are true:

- users or callers can perceive it as an independent capability
- it has distinct rules, states, or failure surface
- it has a distinct entry point such as page, API, command, event, or workflow
- it spans multiple logic modules
- future work often targets it directly

Otherwise, keep it inside the parent document.

## Operating Procedure

Before editing local memory:

1. Inspect the touched code, config, tests, and relevant existing memory docs within the current scope.
2. Identify the touched logic modules.
3. Identify the touched business capabilities.
4. Decide whether the change affects stable semantics, timeline, or whether scope must expand.
5. Reuse and repair existing docs before creating new ones.
6. Update only the minimum required files.

When initializing memory for an existing repository:

- build a current-state baseline, not a reconstruction of history
- start with `agents.md` and `features/index.md`
- add only a small number of high-value module and feature docs first
- do not create docs for every directory
- changelog may be empty at initialization

Use appendices only when helpful: [EXAMPLES.md](./EXAMPLES.md), [TEMPLATES.md](./TEMPLATES.md)

## Writing Constraints

All local memory docs must:
- stay within 400 lines unless repository rules require less; split when needed
- use markdown relative links
- describe current facts, not process narrative
- link instead of repeating content
- mark uncertainty explicitly instead of inventing unsupported claims

All local memory docs must not:
- become a diary
- say "this time we changed ..." inside stable docs
- paste large file trees or long symbol inventories
- put business value in `agents/*`
- put implementation detail in `features/*`
- create changelog by reflex
- update indexes by reflex

## Expected Output
When using this skill:
- update only the required memory files
- keep untouched doc classes unchanged instead of adding filler updates
- ensure every new or updated memory doc starts with `Commit:` and includes required relative links
- briefly summarize what changed and why, including when changelog or top-level indexes were intentionally skipped

## Final Gate
Before finishing, verify:
1. Stable docs describe the current model, not the task history.
2. `agents/*` and `features/*` stay clearly separated.
3. Changelog records meaningful timeline topics, not patch exhaust.
4. Only affected docs were updated.
5. Top-level indexes changed only if the logic map or feature map changed.
6. Claims are supported by code, config, interfaces, tests, or trusted existing memory.
Ask one last question:

**Am I correcting the current semantic model, or am I only recording that an action happened?**

- If it is only action-recording, do not put it into stable docs.
- If it is not meaningful enough for future retrieval, do not write changelog.
