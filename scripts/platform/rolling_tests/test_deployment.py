import copy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.config import Settings
from rolling.deployment import deploy, record_for, rollback, fresh_frontends
from rolling.state import Journal, write
from rolling.manifest import compatible
from rolling.units import nginx
from rolling.units import service
from rolling.units import freeze_rootfs
from rolling.probes import probe_id


def manifest(identifier):
    return {"release_id": identifier, "protocol_version": 9, "files": {}, "compatibility": {
        "protocol": {"min": 1, "max": 1}, "data_format": {"min": 1, "max": 1}}}


class PowerLoss(BaseException):
    pass


class Operations:
    def __init__(self):
        self.calls = []
        self.active = "r1"
        self.crash = False

    def http(self, base, path, method="GET", body=None):
        self.calls.append((method, path, copy.deepcopy(body)))
        if path.startswith("/runtimes/") and path.endswith("/activate"):
            self.active = path.split("/")[2]
            if self.crash:
                self.crash = False
                raise PowerLoss()
        return {"ok": True}

    def run(self, *args):
        self.calls.append(tuple(args))
        if args[:2] in (("systemctl", "stop"), ("systemctl", "restart")):
            if any("runtime" in arg or "server" in arg or arg == "opencoder-agent.service" for arg in args[2:]):
                raise AssertionError("deployment interrupted a business process")

    def wait(self, check, seconds=90):
        return check()


class DeploymentTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        root = Path(self.directory.name)
        self.settings = Settings(root, root / "server-config", root / "server", root / "agent", root / "token", systemd_dir=root / "units", bin_dir=root / "bin")
        self.journal = Journal(root)
        self.old = record_for(self.settings, manifest("r1"), 0)
        self.journal.data.update(current="r1", phase="complete", releases={"r1": self.old})
        self.journal.save()
        self.operations = Operations()
        for target, value in (("manifest.verify", manifest("r2")), ("manifest.resources", None),
                              ("units.prepare", None), ("units.switch_ingress", None),
                              ("probes.candidate", "node-one"), ("probes.ready", True), ("probes.public", True)):
            mocked = patch("rolling.deployment." + target, return_value=value)
            setattr(self, target.replace(".", "_"), mocked.start())
            self.addCleanup(mocked.stop)

    def test_continuous_release_does_not_stop_active_server_or_runtime(self):
        result = deploy(self.settings, Path("bundle"), self.operations)
        self.assertEqual(result["current"], "r2")
        self.assertEqual(result["phase"], "complete")
        self.assertEqual(self.operations.active, "r2")
        self.assertFalse(any("/api/admin/drain" in call for call in self.operations.calls))
        retire = next(i for i, call in enumerate(self.operations.calls) if "/api/admin/release/retire" in call)
        activate = next(i for i, call in enumerate(self.operations.calls) if "/runtimes/r2/activate" in call)
        self.assertGreater(retire, activate)

    def test_killed_deployer_resumes_switch_with_the_same_runtime_and_probe_ids(self):
        self.operations.crash = True
        with self.assertRaises(PowerLoss):
            deploy(self.settings, Path("bundle"), self.operations)
        interrupted = Journal(self.settings.state_dir)
        self.assertEqual(interrupted.data["phase"], "switching")
        self.assertEqual(interrupted.data["current"], "r2")
        result = deploy(self.settings, Path("bundle"), self.operations)
        self.assertEqual(result["phase"], "complete")
        self.assertEqual(len(result["releases"]), 2)
        self.assertEqual(self.probes_candidate.call_count, 1)
        self.assertEqual(self.units_prepare.call_count, 1)
        self.assertEqual(interrupted.record("r2")["probe_epoch"], result["releases"]["r2"]["probe_epoch"])

    def test_failed_prewarm_preserves_current_admission_and_ownership(self):
        self.probes_candidate.side_effect = RuntimeError("probe failed")
        with self.assertRaisesRegex(RuntimeError, "probe failed"):
            deploy(self.settings, Path("bundle"), self.operations)
        result = Journal(self.settings.state_dir).data
        self.assertEqual(result["current"], "r1")
        self.assertEqual(result["phase"], "failed")
        self.assertEqual(self.operations.active, "r1")
        self.units_switch_ingress.assert_not_called()

    def test_post_switch_failure_rolls_back_new_traffic_and_keeps_all_runtimes(self):
        self.probes_public.side_effect = [RuntimeError("public probe failed"), True]
        with self.assertRaisesRegex(RuntimeError, "public probe failed"):
            deploy(self.settings, Path("bundle"), self.operations)
        result = Journal(self.settings.state_dir).data
        self.assertEqual(result["current"], "r1")
        self.assertEqual(result["phase"], "rolled_back")
        self.assertEqual(set(result["releases"]), {"r1", "r2"})
        self.assertEqual(self.operations.active, "r1")
        self.assertNotEqual(probe_id(self.old, public=True), probe_id(result["releases"]["r1"], public=True))

    def test_retry_after_rollback_executes_new_probes_and_keeps_resume_identity(self):
        self.probes_public.side_effect = [RuntimeError("probe failed"), True]
        with self.assertRaisesRegex(RuntimeError, "probe failed"):
            deploy(self.settings, Path("bundle"), self.operations)
        failed = Journal(self.settings.state_dir).record("r2")
        self.probes_public.side_effect = None
        result = deploy(self.settings, Path("bundle"), self.operations)
        retried = result["releases"]["r2"]
        self.assertNotEqual(probe_id(failed), probe_id(retried))
        self.assertNotEqual(probe_id(failed, public=True), probe_id(retried, public=True))
        resumed = deploy(self.settings, Path("bundle"), self.operations)
        self.assertEqual(probe_id(retried), probe_id(resumed["releases"]["r2"]))
        self.assertEqual(len(result["releases"]), 2)

    def test_retained_data_format_is_checked_before_any_switch(self):
        incompatible = manifest("r3")
        incompatible["compatibility"]["data_format"] = {"min": 2, "max": 2}
        with self.assertRaises(ValueError):
            compatible(incompatible, [manifest("r1")])
        configuration = nginx(self.settings, 19000, 19002)
        self.assertNotIn("worker_shutdown_timeout", configuration)
        self.assertIn("proxy_next_upstream off", configuration)
        self.assertIn("listen 127.0.0.1:18081", configuration)

    def test_failed_rollback_standby_preserves_traffic_and_reuses_probe_on_retry(self):
        deploy(self.settings, Path("bundle"), self.operations)
        self.units_switch_ingress.reset_mock()
        self.probes_candidate.side_effect = TimeoutError("legacy runtime busy")
        with self.assertRaisesRegex(TimeoutError, "legacy runtime busy"):
            rollback(self.settings, self.operations)
        failed = Journal(self.settings.state_dir).data
        self.assertEqual(failed["phase"], "failed")
        self.assertEqual(failed["current"], "r2")
        self.assertIsNone(failed["candidate"])
        self.assertFalse(failed["rollback_switch_started"])
        self.assertEqual(failed["failure"], "legacy runtime busy")
        self.assertEqual(self.operations.active, "r2")
        self.units_switch_ingress.assert_not_called()
        self.probes_candidate.side_effect = None
        result = rollback(self.settings, self.operations)
        self.assertEqual(result["phase"], "rolled_back")
        self.assertEqual(result["current"], "r1")
        for key in ("server_unit", "host_unit", "probe_epoch"):
            self.assertEqual(failed["releases"]["r1"][key], result["releases"]["r1"][key])

    def test_failed_rollback_standby_allows_a_validated_forward_release(self):
        deploy(self.settings, Path("bundle"), self.operations)
        self.probes_candidate.side_effect = TimeoutError("legacy runtime busy")
        with self.assertRaises(TimeoutError):
            rollback(self.settings, self.operations)
        self.probes_candidate.side_effect = None
        self.manifest_verify.return_value = manifest("r3")
        result = deploy(self.settings, Path("next-bundle"), self.operations)
        self.assertEqual(result["current"], "r3")
        self.assertEqual(result["previous"], "r2")
        self.assertEqual(result["phase"], "complete")
        self.assertIsNone(result["rollback_from"])
        self.assertIsNone(result["rollback_switch_started"])

    def test_same_release_retry_after_failed_rollback_retains_original_target(self):
        deploy(self.settings, Path("bundle"), self.operations)
        self.probes_candidate.side_effect = TimeoutError("legacy runtime busy")
        with self.assertRaises(TimeoutError):
            rollback(self.settings, self.operations)
        self.probes_candidate.side_effect = None
        result = deploy(self.settings, Path("bundle"), self.operations)
        self.assertEqual(result["current"], "r2")
        self.assertEqual(result["previous"], "r1")
        self.assertEqual(result["phase"], "complete")
        self.assertEqual(rollback(self.settings, self.operations)["current"], "r1")

    def test_lost_activation_then_failed_rollback_retains_target_on_same_release_retry(self):
        original = self.operations.http
        def lose_activation_reply(base, path, *args, **kwargs):
            result = original(base, path, *args, **kwargs)
            if path == "/runtimes/r2/activate":
                self.operations.http = original
                raise TimeoutError("activation reply lost")
            return result
        self.operations.http = lose_activation_reply
        self.probes_candidate.side_effect = ["node-one", TimeoutError("rollback standby busy")]
        with self.assertRaisesRegex(TimeoutError, "rollback standby busy"):
            deploy(self.settings, Path("bundle"), self.operations)
        failed = Journal(self.settings.state_dir).data
        self.assertEqual(failed["current"], "r2")
        self.assertEqual(failed["previous"], "r1")
        self.assertEqual(failed["phase"], "failed")
        self.assertFalse(failed["rollback_switch_started"])
        self.units_switch_ingress.assert_not_called()
        self.probes_candidate.side_effect = None
        self.probes_candidate.return_value = "node-one"
        result = deploy(self.settings, Path("bundle"), self.operations)
        self.assertEqual(result["current"], "r2")
        self.assertEqual(result["previous"], "r1")
        self.assertEqual(result["phase"], "complete")
        self.assertEqual(result["releases"]["r2"]["runtime_unit"], failed["releases"]["r2"]["runtime_unit"])
        self.assertEqual(rollback(self.settings, self.operations)["current"], "r1")

    def test_interrupted_rollback_switch_cannot_be_treated_as_failed_standby(self):
        deploy(self.settings, Path("bundle"), self.operations)
        self.operations.crash = True
        with self.assertRaises(PowerLoss):
            rollback(self.settings, self.operations)
        interrupted = Journal(self.settings.state_dir).data
        self.assertEqual(interrupted["phase"], "rolling_back")
        self.assertTrue(interrupted["rollback_switch_started"])
        self.assertEqual(self.operations.active, "r1")
        self.probes_candidate.side_effect = TimeoutError("retry not ready")
        with self.assertRaises(TimeoutError):
            rollback(self.settings, self.operations)
        self.assertEqual(Journal(self.settings.state_dir).data["phase"], "rolling_back")
        self.probes_candidate.side_effect = None
        self.assertEqual(rollback(self.settings, self.operations)["phase"], "rolled_back")

    def test_legacy_rollback_without_switch_checkpoint_stays_recoverable(self):
        deploy(self.settings, Path("bundle"), self.operations)
        journal = Journal(self.settings.state_dir)
        journal.data.pop("rollback_switch_started", None)
        journal.phase("rolling_back")
        self.probes_candidate.side_effect = TimeoutError("legacy checkpoint unknown")
        with self.assertRaises(TimeoutError):
            rollback(self.settings, self.operations)
        self.assertEqual(Journal(self.settings.state_dir).data["phase"], "rolling_back")

    def test_republish_keeps_old_response_instances_and_retires_each_new_activation(self):
        endpoints = []
        original = self.operations.http
        def http(base, path, *args, **kwargs):
            if path == '/api/admin/release/retire':
                endpoints.append(base)
            return original(base, path, *args, **kwargs)
        self.operations.http = http
        first = deploy(self.settings, Path('bundle'), self.operations)['releases']['r2']
        rollback(self.settings, self.operations)
        second = deploy(self.settings, Path('bundle'), self.operations)['releases']['r2']
        self.assertNotEqual(first['server_unit'], second['server_unit'])
        self.assertNotEqual(first['host_unit'], second['host_unit'])
        self.assertEqual(first['runtime_unit'], second['runtime_unit'])
        self.assertEqual(first['runtime_data'], second['runtime_data'])
        self.assertIn({'unit':first['server_unit'],'port':first['server_port'],
            'phase':'retiring','failure':None},second['previous_servers'])
        resumed = deploy(self.settings, Path('bundle'), self.operations)['releases']['r2']
        self.assertEqual(resumed['server_unit'], second['server_unit'])
        rollback(self.settings, self.operations)
        self.assertIn(f"http://127.0.0.1:{first['server_port']}", endpoints)
        self.assertIn(f"http://127.0.0.1:{second['server_port']}", endpoints)

    def test_reactivation_port_exhaustion_keeps_the_original_record(self):
        record = copy.deepcopy(self.old)
        occupied = {**record, 'host_port':65534}
        with self.assertRaisesRegex(ValueError, 'no ports'):
            fresh_frontends(record, [record, occupied], 'activate')
        self.assertEqual(record, self.old)

    def test_interrupted_reactivation_reuses_its_new_instance_identity(self):
        deploy(self.settings, Path('bundle'), self.operations)
        rollback(self.settings, self.operations)
        self.operations.crash = True
        with self.assertRaises(PowerLoss):
            deploy(self.settings, Path('bundle'), self.operations)
        pending = Journal(self.settings.state_dir).record('r2')
        result = deploy(self.settings, Path('bundle'), self.operations)
        self.assertEqual(result['phase'], 'complete')
        resumed = result['releases']['r2']
        for key in ('server_unit','host_unit','runtime_unit','probe_epoch'):
            self.assertEqual(resumed[key], pending[key])
        self.assertEqual(resumed['previous_servers'], [
            {**server,'phase':'retiring','failure':None} for server in pending['previous_servers']])

    def test_systemd_accepts_generated_working_directory_and_command(self):
        import subprocess
        directory = Path(self.directory.name) / 'work directory'
        directory.mkdir()
        unit = Path(self.directory.name) / 'opencoder-unit-check.service'
        unit.write_text(service(['/bin/true'], 'unit verification', workdir=directory))
        subprocess.run(['systemd-analyze','verify',str(unit)],check=True,capture_output=True)
        self.assertIn('WorkingDirectory=' + str(directory) + '\n',unit.read_text())

    def test_oci_images_are_pinned_per_runtime_and_ignore_live_mounts(self):
        root = Path(self.directory.name)
        source = root / 'old/dag/rootfs'
        (source / 'usr/bin').mkdir(parents=True)
        (source / 'dev').mkdir()
        (source / 'usr/bin/wasmtime').write_bytes(b'first-version')
        (source / 'dev/ptmx').write_bytes(b'live-device')
        (source / 'bin').symlink_to('usr/bin')
        record = {'runtime_data':str(root / 'new'),'resource_source':str(root / 'old')}
        freeze_rootfs(record)
        (source / 'usr/bin/wasmtime').write_bytes(b'second-version')
        freeze_rootfs(record)
        target = root / 'new/dag/rootfs'
        self.assertEqual((target / 'usr/bin/wasmtime').read_bytes(),b'first-version')
        self.assertFalse((target / 'dev/ptmx').exists())
        self.assertEqual((target / 'bin').readlink(),Path('usr/bin'))
        self.assertEqual((source / 'dev/ptmx').read_bytes(),b'live-device')
        outside = root / 'outside'
        outside.mkdir()
        (source / 'workspace').symlink_to(outside)
        with self.assertRaisesRegex(ValueError,'workspace'):
            freeze_rootfs({'runtime_data':str(root / 'third'),'resource_source':str(root / 'old')})
        self.assertFalse((outside / 'context').exists())


if __name__ == "__main__":
    unittest.main()
