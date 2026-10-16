import copy
import json
import os
from pathlib import Path
import sqlite3
import stat
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.config import load
from rolling import storage
from rolling.maintenance.preflight import configuration
from rolling.native import runtime_config
from rolling.deployment import record_for
from rolling.units import prepare_host, prepare_server
from rolling.units import service


class StorageTests(unittest.TestCase):
    def fixture(self, root):
        source = root / "source"
        source.mkdir()
        (source / "project.txt").write_bytes(b"immutable source")
        declaration = {"deployment": {"state_dir": str(root / "platform"),
                                      "token_file": str(root / "token"), "server_user": "root",
                                      "systemd_dir": str(root / "units")},
                       "dag": {"workspace_dir": str(source)},
                       "provider": {"api_key": "fixture-private-credential"}}
        path = root / "declaration.json"
        path.write_text(json.dumps(declaration))
        return load(path), declaration, path

    def source_inventory(self, source):
        return [(str(path.relative_to(source)), path.stat().st_ino,
                 path.stat().st_uid, stat.S_IMODE(path.stat().st_mode),
                 path.read_bytes() if path.is_file() else None)
                for path in [source, *sorted(source.rglob("*"))]]

    def test_one_root_drives_configs_mounts_runtime_and_server_units(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings, declaration, _ = self.fixture(root)
            source = Path(declaration["dag"]["workspace_dir"])
            before = self.source_inventory(source)
            original = copy.deepcopy(declaration)
            plan = storage.plan(settings, declaration)
            self.assertFalse(settings.state_dir.exists(), "preview must not initialize the deployment")
            prepared = storage.prepare(settings, declaration)
            self.assertEqual(storage.prepare(settings, declaration), prepared)
            agent = json.loads((settings.agent_workdir / "opencoder.json").read_text())
            server = json.loads((settings.server_workdir / "opencoder.json").read_text())
            self.assertEqual(configuration(agent, server)["rootfs_dir"],
                             settings.state_dir / "images/dag-rootfs")
            self.assertEqual(server["dag"]["workspace_dir"], str(source))
            self.assertEqual(agent["dag"]["workspace_dir"], str(settings.state_dir / "mounts/workspace"))
            self.assertEqual(server["agent"]["agents_dir"], str(settings.state_dir / "exports/agents"))
            self.assertEqual(server["agent"]["share_dir"], str(settings.server_data / ".opencoder/share"))
            self.assertNotIn("share_dir", agent["agent"])
            self.assertTrue(Path(server["agent"]["share_dir"]).is_dir())
            self.assertEqual(server["ontology"]["files_dir"], str(settings.state_dir / "exports/ontology"))
            self.assertEqual(storage.configurations(settings,
                             {"deployment": declaration["deployment"], **server}), (agent, server))
            self.assertNotIn("deployment", agent)
            self.assertNotIn("deployment", server)
            self.assertEqual(json.loads((settings.state_dir / "deployment.json").read_text()),
                             {"deployment": declaration["deployment"]})
            self.assertTrue(all(Path(path).is_relative_to(settings.state_dir) for path in plan["paths"].values()))
            self.assertEqual(plan["paths"]["runners"], str(settings.state_dir / "runners"))
            for mount in plan["mounts"]:
                content = (settings.state_dir / "install/systemd" / mount["unit"]).read_text()
                self.assertIn("Options=ro,", content)
                self.assertIn("Where=" + mount["path"], content)
                self.assertIn("port=" + str(mount["port"]), content)
            settings.systemd_dir.mkdir()
            record = record_for(settings, {"release_id": "fixture"}, 0)
            prepare_server(settings, record)
            prepare_host(settings, record)
            server_unit = (settings.systemd_dir / record["server_unit"]).read_text()
            self.assertIn('"--data-dir" "' + str(settings.state_dir / "server") + '"', server_unit)
            self.assertIn('"--workdir" "' + str(settings.state_dir / "workspaces/server") + '"', server_unit)
            image = Path(record["runtime_data"]) / "dag/rootfs"
            accepted = runtime_config(settings, record, image, home=root / "empty-home")
            frozen = json.loads((accepted / "opencoder.json").read_text())
            self.assertEqual(frozen["dag"]["data_dir"], str(Path(record["runtime_data"]) / "dag/runs"))
            self.assertEqual(frozen["dag"]["rootfs_dir"], str(image))
            self.assertEqual(frozen["provider"]["api_key"], "fixture-private-credential")
            self.assertEqual(stat.S_IMODE((settings.server_workdir / "opencoder.json").stat().st_mode), 0o600)
            self.assertEqual(declaration, original)
            self.assertEqual(self.source_inventory(source), before)

    def test_redundant_deployment_paths_require_explicit_migration(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            _, declaration, path = self.fixture(root)
            for key in ("server_data", "server_workdir", "agent_workdir"):
                candidate = copy.deepcopy(declaration)
                candidate["deployment"][key] = str(root / "old")
                path.write_text(json.dumps(candidate))
                with self.subTest(key=key), self.assertRaisesRegex(ValueError, "migrate existing data"):
                    load(path)
            self.assertFalse((root / "platform").exists())

    def test_conflicting_managed_paths_and_writable_exports_fail_before_preparation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings, declaration, _ = self.fixture(root)
            for section, key, value in [("agent", "agents_dir", str(root / "old-agents")),
                                        ("agent", "share_dir", str(root / "old-share")),
                                        ("dag", "binary_dir", str(root / "old-binaries")),
                                        ("ontology", "files_dir", str(root / "old-text")),
                                        ("dag", "data_dir", str(root / "old-runs")),
                                        ("dag", "nfs", {"read_only": False}),
                                        ("agent", "nfs", {"enabled": False})]:
                candidate = copy.deepcopy(declaration)
                candidate.setdefault(section, {})[key] = value
                with self.subTest(key=key), self.assertRaises(ValueError):
                    storage.prepare(settings, candidate)
                self.assertFalse(settings.state_dir.exists())

    def test_missing_nested_and_aliased_source_workspaces_are_never_modified(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings, declaration, _ = self.fixture(root)
            source = root / "source"
            before = self.source_inventory(source)
            alias = root / "source-alias"
            alias.symlink_to(source, target_is_directory=True)
            for path in (root / "missing", settings.state_dir, source, alias):
                candidate = copy.deepcopy(declaration)
                candidate["dag"]["workspace_dir"] = str(path)
                # Last two cases make the state root a child of the source.
                if path in (source, alias):
                    from dataclasses import replace
                    current = replace(settings, state_dir=path / "state")
                else:
                    current = settings
                with self.subTest(path=path), self.assertRaises(ValueError):
                    storage.prepare(current, candidate)
            self.assertEqual(self.source_inventory(source), before)
            self.assertFalse(settings.state_dir.exists())

    def test_existing_edits_and_managed_directory_symlinks_are_not_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings, declaration, _ = self.fixture(root)
            settings.server_workdir.mkdir(parents=True)
            target = settings.server_workdir / "opencoder.json"
            target.write_text('{"operator_note":"keep"}')
            with self.assertRaisesRegex(ValueError, "different content"):
                storage.prepare(settings, declaration)
            self.assertEqual(target.read_text(), '{"operator_note":"keep"}')
            self.assertFalse(settings.agent_workdir.exists())
            target.unlink()
            exports = settings.state_dir / "exports"
            exports.symlink_to(root / "source", target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "symlink"):
                storage.prepare(settings, declaration)
            self.assertEqual((root / "source/project.txt").read_bytes(), b"immutable source")

    def test_existing_layout_is_read_from_role_configs_without_repeating_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings, declaration, _ = self.fixture(root)
            storage.prepare(settings, declaration)
            for role, workdir in (("server", settings.server_workdir), ("agent", settings.agent_workdir)):
                path = workdir / "opencoder.json"
                value = json.loads(path.read_text())
                value["model"] = role + "/model"
                path.write_text(json.dumps(value))
            minimal = {"deployment": declaration["deployment"]}
            preview = storage.plan(settings, minimal)
            agent, server = storage.configurations(settings, minimal)
            self.assertEqual(preview["source_workspace"], str(root / "source"))
            self.assertEqual(agent["model"], "agent/model")
            self.assertEqual(server["model"], "server/model")
            self.assertNotIn("private-credential", json.dumps(preview))

    def test_ontology_binding_is_checked_read_only_before_preparing_new_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings, declaration, _ = self.fixture(root)
            settings.server_data.mkdir(parents=True)
            database = settings.server_data / "ontology.db"
            with sqlite3.connect(database) as connection:
                connection.execute("CREATE TABLE ontology_schema_version(version INTEGER, files_root TEXT)")
                connection.execute("INSERT INTO ontology_schema_version VALUES(1, ?)", (str(root / "old-text"),))
            before = database.read_bytes()
            with self.assertRaisesRegex(ValueError, "verify and migrate all ready text revisions"):
                storage.prepare(settings, declaration)
            self.assertEqual(database.read_bytes(), before)
            self.assertFalse(settings.agent_workdir.exists())

    def test_invalid_root_and_duplicate_nfs_ports_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings, declaration, path = self.fixture(root)
            for state in ("relative", "/invalid\nroot", "/invalid%root"):
                candidate = copy.deepcopy(declaration)
                candidate["deployment"]["state_dir"] = state
                path.write_text(json.dumps(candidate))
                with self.subTest(state=state), self.assertRaisesRegex(ValueError, "absolute path"):
                    load(path)
            declaration["agent"] = {"nfs": {"port": 2050}}
            with self.assertRaisesRegex(ValueError, "distinct"):
                storage.plan(settings, declaration)

    def test_restrictive_umask_and_existing_root_allow_the_actual_service_user(self):
        from dataclasses import replace
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            root.chmod(0o755)
            settings, declaration, _ = self.fixture(root)
            settings = replace(settings, server_user="nobody")
            declaration["deployment"]["server_user"] = "nobody"
            settings.state_dir.mkdir(mode=0o700)
            previous = os.umask(0o077)
            try:
                storage.prepare(settings, declaration)
                binary = settings.state_dir / "releases/fixture/bundle/bin/true"
                binary.parent.mkdir(parents=True)
                binary.write_bytes(Path("/bin/true").read_bytes())
                binary.chmod(0o755)
                storage.allow_server_traversal(settings, binary.parent)
            finally:
                os.umask(previous)
            for path in (settings.server_data, settings.server_workdir,
                         settings.state_dir / "resources", settings.state_dir / "exports/agents"):
                for permission in ("-r", "-x"):
                    subprocess.run(["runuser", "-u", "nobody", "--", "test", permission, str(path)], check=True)
            subprocess.run(["runuser", "-u", "nobody", "--", "cat",
                            str(settings.server_workdir / "opencoder.json")],
                           stdout=subprocess.DEVNULL, check=True)
            subprocess.run(["runuser", "-u", "nobody", "--", str(binary)], check=True)
            shared = settings.server_data / ".opencoder/share"
            probe = shared / "write-check"
            subprocess.run(["runuser", "-u", "nobody", "--", "touch", str(probe)], check=True)
            subprocess.run(["runuser", "-u", "nobody", "--", "rm", str(probe)], check=True)
            self.assertEqual(stat.S_IMODE((settings.state_dir / "runtimes").stat().st_mode), 0o700)
            self.assertEqual(stat.S_IMODE((settings.state_dir / "deployment.json").stat().st_mode), 0o600)
            outside = root / "source"
            before = self.source_inventory(outside)
            with self.assertRaisesRegex(ValueError, "inside persistent root"):
                storage.allow_server_traversal(settings, outside)
            self.assertEqual(self.source_inventory(outside), before)

    def test_resource_service_has_no_ordering_dependency_on_itself(self):
        unit = service(["/bin/true"], "resources", resource=True)
        after = next(line for line in unit.splitlines() if line.startswith("After="))
        self.assertNotIn("opencoder-resources.service", after)
        self.assertIn("remote-fs.target", after)
        self.assertIn("opencoder-resources.service", service(["/bin/true"], "server"))
