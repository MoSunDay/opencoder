# Isolated read-only resource exports

`cargo build --locked --release -p opencoder-agents --example isolated_nfs`
builds a loopback-only NFSv3 exporter without a control-plane database.
Run `isolated_nfs /absolute/immutable-agent-bundle 20491` and mount with
`ro,vers=3,port=20491,mountport=20491,nolock,proto=tcp,mountproto=tcp`.

A dedicated execution node may set its workdir's `agent.agents_dir` to this
mount. Publish a content-addressed bundle first: copy the registered Agent
card, every referenced resource metadata file, and each referenced current
version; reject symlinks and verify hashes and unchanged source metadata.
Scope this to a dedicated workload whose complete resource closure is known.
Delegating Agents need the complete closure of their permitted callees.

Pin the exporter binary, bundle manifest and mount in the deployment receipt.
Future resource changes require a new verified bundle and mount; never replace
files inside an existing bundle. Existing execution-local snapshots remain
authoritative. The node still verifies a real read-only NFS mount and freezes
resources separately for every execution.
