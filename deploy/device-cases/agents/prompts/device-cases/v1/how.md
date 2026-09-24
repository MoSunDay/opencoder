Use the host-authenticated identity and private transport PATH appended by the executor. Never print or read credential contents into conversation, logs, or artifacts. Do not access host admin credentials.

For allocate: execute the host-provided client.py reserve command and return its JSON object unchanged. It contains exactly the requested assignments. Do not create your own assignments or alter items.

For execute: the dynamic item is one assigned device and its ordered case_ids. Run every case in that order, one at a time, with:
/opt/device-cases/harness/dependencies/runtime/bin/python3 -B /opt/device-cases/harness/native-harness.py --home /data00/device_mananger/runtime/native-harness --device-context PRIVATE_PATH run --case-source ORIGINAL_CASE_SOURCE --case-id ORIGINAL_CASE_ID --concurrency 1 --full-evidence --inspect-ui
Before starting a native run, inspect the frozen parent input. If it contains candidate, the following adapter is mandatory; use it for the initial invocation and every recovery. Do not invoke the default native-harness.py command for that assignment:
/data00/device_mananger/runtime/native-harness/dag/candidates/cfb71ad6b396ba6341a9c7e06112a53d96411ee91c0d2717043751b19b2d6b52/dependencies/runtime/bin/python3 -B /data00/workspace/artifacts/ticket-1177-fix/candidate-entry/runtime/782a75531400a3814dff5a092cfc945dfc108eb1c3d3e2af5f228e6fa31982be/native.py --home /data00/device_mananger/runtime/native-harness --device-context PRIVATE_PATH --case-source ORIGINAL_CASE_SOURCE --case-id ORIGINAL_CASE_ID
Adapter SHA256: deb86abe27e572befabe4b7041ec89b6186d5b6eeaeb4aeb8c8f73285e258070. Candidate transfer module SHA256: fb38a4c63f0b28dc9d6553654ac204401bfb064368f23c886c3f9593eba7e1a5. Keep the same assigned device, private context and original case. Verify actual candidate Native/Host paths, resource commit and SDK initialization before business acceptance. Never modify a registered profile or substitute the old app. On interruption resume the same work through this adapter; never resubmit completed business.

Use the frozen case_source from Device input, and each original case_id from the assignment. Keep the original prompt and assertions. Do not invent a simpler case. Never use the legacy fleet allocator or select a different device. If a command is still running, collect its completion; do not resubmit it. The Harness uses a deterministic native_run_id and resumes the same work on retry.

Finish evidence collection and verify application, task, capture, proxy, and firewall restoration before the next case. A true product assertion failure must stay a failure in the result; do not repeat completed business to conceal it. If recovery is incomplete, stop the sequence and report the actual blocker. For recovery_only transport, only resume the already registered work; do not start new business.

Return JSON with each case_id/native_run_id, execution and business result separately, evidence paths, and recovery result. Never claim a product passed solely because the command completed or the device recovered. Do not return successful completion without executing the full ordered batch. The host verifies authoritative work coverage before accepting completion.

Every run, status, and resume command must include the same explicit --home /data00/device_mananger/runtime/native-harness; never use the default legacy home.
