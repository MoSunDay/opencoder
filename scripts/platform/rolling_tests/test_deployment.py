import copy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.config import Settings
from rolling.deployment import deploy, record_for
from rolling.state import Journal, write
from rolling.manifest import compatible
from rolling.units import nginx
from rolling.units import service


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

    def test_retained_data_format_is_checked_before_any_switch(self):
        incompatible = manifest("r3")
        incompatible["compatibility"]["data_format"] = {"min": 2, "max": 2}
        with self.assertRaises(ValueError):
            compatible(incompatible, [manifest("r1")])
        configuration = nginx(self.settings, 19000, 19002)
        self.assertNotIn("worker_shutdown_timeout", configuration)
        self.assertIn("proxy_next_upstream off", configuration)
        self.assertIn("listen 127.0.0.1:18081", configuration)

    def test_systemd_accepts_generated_working_directory_and_command(self):
        import subprocess
        directory = Path(self.directory.name) / 'work directory'
        directory.mkdir()
        unit = Path(self.directory.name) / 'opencoder-unit-check.service'
        unit.write_text(service(['/bin/true'], 'unit verification', workdir=directory))
        subprocess.run(['systemd-analyze','verify',str(unit)],check=True,capture_output=True)
        self.assertIn('WorkingDirectory=' + str(directory) + '\n',unit.read_text())


if __name__ == "__main__":
    unittest.main()
