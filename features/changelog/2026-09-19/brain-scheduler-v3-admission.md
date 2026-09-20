Commit: f11f2352ca83e95256a4d05c5afa9af9a839f638

# Brain v3 admission and terminal delivery

## Context

The event-driven brain scheduler is the only writable brain-run protocol. Legacy v2 plans and runs remain available for historical reads, while new callers need an explicit schema marker so the control plane cannot silently construct the old graph runtime.

## Change Summary

- `POST /api/brain/runs`, generic brain command forwarding, input writes, scheduled brain runs, and CLI creation now require `schema_version: 3`; rejected legacy writes return the migration contract without creating a run.
- Repeated v3 create requests with the same run intent replay the accepted execution index; a changed intent remains a conflict.
- Child terminal outbox frames always carry a positive source sequence, including executions whose terminal record has no prior event rows, so a successful child cannot leave its scheduler operation in `running` forever.
- Late cancellation settles the operation index after a failed root without changing the root's terminal phase or error.

## Impact Surface

- V3 snapshots, rounds, events, and commands continue to be served by the owning root node.
- Sub-execution details remain addressable through their execution ID; scheduler events contain indexes, references, and summaries only.
- Existing v2 write fixtures were converted to migration/read-only assertions; v3 round-trip coverage exercises the real Control-to-Worker dispatch and terminal barrier.

## Related Docs

- [Brain logic](../../../agents/brain/index.md)
- [Control logic](../../../agents/control/index.md)
- [Worker logic](../../../agents/worker/index.md)
- [Node ownership changelog](./brain-scheduler-node-ownership.md)
