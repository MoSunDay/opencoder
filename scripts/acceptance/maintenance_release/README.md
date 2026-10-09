# Maintenance release acceptance

Run as root on a Linux host with systemd, NFS client support, Nginx and runc:

```sh
python3 -B scripts/acceptance/maintenance_release/main.py \
  --platform-bundle /absolute/candidate/bundle \
  --rootfs /absolute/candidate/rootfs \
  --old-bundle /absolute/retained/old/bundle \
  --corrective-bundle /absolute/compatible/different-commit/bundle
```

The controller source and candidate bundle must belong to the same clean commit.
All old Server, Host, Runtime and resource service binaries come from the retained
old bundle. The old activation probe uses its WASM protocol exclusively in this
acceptance fixture. The product has no legacy execution fallback.

Each run creates an `mr-*` evidence directory with independent configuration,
token, databases, ports, Nginx and source workspace. Actual host systemd executes
uniquely named test service units and their restart policies. Host management
calls use a private control socket that translates only this run's unit names.
Read-only NFS mounts have unique paths under the evidence directory. Production
units, configuration, mounts and authentication data remain outside this fixture.

The first controller process exits at durable `installing`, after stopping old
writers and sealing the backup. Recovery restores the old configuration and
services and proves the old project API can still read and write. A second fault
after real catalog migration keeps public platform and Host writes closed.
Recovery again restores old project tables, indexes and schema version while
preserving authentication rows. A new attempt then fails at private verification
and retries the identical candidate and immutable backup through reopening. Public
native execution and project/Host writes verify reopening. Old-backup recovery
is rejected once write reopening has been persisted.

The retry deliberately fails after actual public reopening to verify forward recovery.
After a new project write, a distinct immutable corrective release is staged;
SIGUSR2 sent to the actual Server starts an independent systemd controller job.
The new Runtime executes the native probe, preserves the new write and original
sealed backup, and keeps authentication and source workspace fingerprints stable.
Both bundles must come from different clean compiled commits. The fixture records
and rechecks both binary inventories; changing only a release ID is rejected.

`source-inventory.json` records the source workspace's inode, ownership,
permissions and content. Every crash, migration, recovery, retry and cleanup must
preserve that inventory. Authentication queries use read-only SQLite connections;
receipts contain counts and fingerprints, never tokens. Completed backups are
verified without creating SQLite auxiliary files.

The fixture reports the actual old schema, including old catalogs whose watermark
already says 32. Store `catalog_maintenance` tests independently cover v31 and
legacy v32 migration, repeated startup and old project writes after restoration.

Cleanup stops and removes only run-owned units and mounts. Data, backups, source
inventory and logs remain. Inspect `result.json`, `failure.txt` and controller logs
in the printed evidence directory. `passed` requires all scenarios and cleanup.

For exploratory runs, `--bin-dir` can replace `--platform-bundle`; those receipts
always report `release_bundle: false` and do not prove the final source revision.

```sh
python3 -B -m unittest discover -s scripts/acceptance/maintenance_release/tests -v
```
