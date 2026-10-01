# Isolated maintenance acceptance

Run from the repository root as root on a Linux host with mount namespaces,
NFS client support, Nginx and runc. No build, commit or production deployment is
performed. Each run creates a fresh `mr-*` directory under `--data-parent`
(default `/root/.cache/opencoder-e2e`) and prints the complete result JSON.

```sh
python3 scripts/acceptance/maintenance_release/main.py \
  --platform-bundle /absolute/path/to/candidate-bundle \
  --rootfs /absolute/path/to/rootfs \
  --old-bundle /absolute/path/to/actual-data-format1-bundle
```

The local fixture example uses these arguments with the same entry point:

```sh
python3 scripts/acceptance/maintenance_release/main.py \
  --platform-bundle /tmp/opencoder-launch-bundle-da29072 \
  --rootfs /tmp/opencoder-launch-rootfs-23bc55b \
  --old-bundle /var/lib/opencoder-platform/releases/rel-33bccb1e125707a468dbb764b64e1d2dac4fc521/bundle
```

That fixture tests current repository controller modules using existing clean
da29072 binaries; it does not verify the current whole formal candidate.
The latest retained data-format1 92b9f4ca
Server actually initializes schema 32, so it cannot prove a schema transition;
the retained 33bccb1e Server supplies the real older schema. Alternatively use `--bin-dir` with an
immutable directory containing all six binaries. Such inputs get a private
package with their actual metadata and hashes preserved and always report
`release_bundle: false`. Both modes use unchanged strict installer validation;
dirty builds or unknown SPA hashes remain rejected. No verifier or launcher
installation is patched. Use stripped binaries: the real native probe limits
its executable to 32 MiB. The controller freezes the supplied rootfs, installs
candidate runner copies into the frozen image, and checks their complete
metadata by actually executing both helpers in chroot before stopping services.

The actual old Server is copied from the retained bundle.
Only bundle binaries and bundle metadata are read there. Tokens, configuration,
databases, mounts and logs are created exclusively for this fixture. Generated
service commands and on-failure restarts run as owned child processes; generated `.mount` units become
actual read-only NFS mounts in the private namespace. Host calls to `systemctl`
use a private socket through a fixture-local executable. No host systemd units
are installed and no production units or Nginx configuration are changed.
This accepts generated unit commands and maintenance behavior, not host systemd
dependency scheduling or restart policy.

The first independent controller exits at durable `installing` with code 86,
before `schema_started`. Recovery uses the real maintenance rollback, verifies
old configuration bytes and backup hashes, then proves the actual old Server
still reads and writes project goals. The second controller runs real migration
and private native verification, fails at `verifying`, proves both public write
gates return 503, and checks rollback refuses without effects. A new controller
retries the identical candidate with identical verified backup/config snapshots;
real public native execution, project writes and Host management writes prove gates reopen.

`source-inventory.json` freezes the workspace path, inode, device, uid, gid,
mode and file hashes before service startup. Checks retain that exact inventory
through crashes, rollback, failure, refusal, retry, success and cleanup. The
source workspace is neither copied nor relinked nor made writable. Authentication
tables are read through SQLite `mode=ro`, and only counts and hashes appear in
the result; the harness never updates existing authentication data.

The registered old node and independent NFS exporter intentionally use candidate
binaries so the initial Host has a real native activation probe. This does not
prove full old Runtime or old exporter behavior. `schema_before` comes from the
actual old Server database, which may be 28. The separate Store tests
`catalog_maintenance::v31_migration_restarts_and_restores_old_project_writes_without_auth_changes`
and `project_tags::v31_migration_removes_legacy_containers_but_preserves_todo_payloads`
cover 31 -> 32; this harness neither fabricates schema 31 nor runs those tests.

Cleanup signals only owned processes and unmounts only owned paths. Data,
backups, config snapshots, generated units, source inventory and scenario logs
remain available. `passed` requires both scenarios and cleanup to pass. On
failure, inspect `result.json`, `failure.txt` and `controller-*.log` in the
printed evidence directory. If rollback completes but changes the backup's file
set, the harness records that failure and still executes the independent forward
retry scenario; the overall result remains failed. It never deletes unexpected
SQLite sidecars or rewrites backup manifests to make preservation checks pass.

```sh
python3 -m unittest discover -s scripts/acceptance/maintenance_release/tests -v
```

Local checks cover exclusive CLI inputs, source identity/content changes,
read-only auth fingerprints, current actual/desired configuration overlays,
debug provenance, generated mount ownership, private socket routing and current
`systemctl --no-block stop` / `--kill-who=main --signal=SIGTERM` commands.
