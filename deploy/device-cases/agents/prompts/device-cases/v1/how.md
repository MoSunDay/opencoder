Use the host-authenticated identity and private transport PATH appended by the executor. Never print or read credential contents into conversation, logs, or artifacts. Do not access host admin credentials.

For allocate: execute the host-provided client.py reserve command and return its JSON object unchanged. It contains exactly the requested assignments. Do not create your own assignments or alter items.

For execute: the dynamic item is one assigned device and its ordered case_ids. Run every case in that order, one at a time, with:
/opt/device-cases/harness/dependencies/runtime/bin/python3 -B /opt/device-cases/harness/native-harness.py --home /data00/device_mananger/runtime/native-harness --device-context PRIVATE_PATH run --case-source ORIGINAL_CASE_SOURCE --case-id ORIGINAL_CASE_ID --concurrency 1 --full-evidence --inspect-ui
Use the frozen case_source from Device input, and each original case_id from the assignment. Keep the original prompt and assertions. Do not invent a simpler case. Never use the legacy fleet allocator or select a different device. If a command is still running, collect its completion; do not resubmit it. The Harness uses a deterministic native_run_id and resumes the same work on retry.

Finish evidence collection and verify application, task, capture, proxy, and firewall restoration before the next case. A true product assertion failure must stay a failure in the result; do not repeat completed business to conceal it. If recovery is incomplete, stop the sequence and report the actual blocker. For recovery_only transport, only resume the already registered work; do not start new business.

Return JSON with each case_id/native_run_id, execution and business result separately, evidence paths, and recovery result. Never claim a product passed solely because the command completed or the device recovered. Do not return successful completion without executing the full ordered batch. The host verifies authoritative work coverage before accepting completion.

Every run, status, and resume command must include the same explicit --home /data00/device_mananger/runtime/native-harness; never use the default legacy home.
