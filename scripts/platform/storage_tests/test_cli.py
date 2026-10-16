import json
from pathlib import Path
import subprocess
import tempfile
import unittest


class StorageCliTests(unittest.TestCase):
    def fixture(self, root):
        source = root / "source"
        source.mkdir()
        (source / "input.txt").write_text("source stays unchanged")
        config = root / "declaration.json"
        config.write_text(json.dumps({"deployment": {
            "state_dir": str(root / "platform"), "token_file": str(root / "token"),
            "server_user": "root", "systemd_dir": str(root / "units")},
            "dag": {"workspace_dir": str(source)},
            "provider": {"api_key": "private-fixture-credential"}}))
        return config

    def run_cli(self, config, *args):
        executable = Path(__file__).resolve().parents[1] / "deploy.sh"
        return subprocess.run([str(executable), "--config", str(config), *args],
                              capture_output=True, text=True)

    def test_preview_and_preparation_use_the_real_cli_without_starting_services(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            config = self.fixture(root)
            preview = self.run_cli(config, "--storage-plan")
            self.assertEqual(preview.returncode, 0, preview.stderr)
            result = json.loads(preview.stdout)
            self.assertEqual(result["paths"]["server"], str(root / "platform/server"))
            self.assertFalse((root / "platform").exists())
            prepared = self.run_cli(config, "--prepare-storage")
            self.assertEqual(prepared.returncode, 0, prepared.stderr)
            receipt = json.loads(prepared.stdout)
            self.assertFalse(receipt["services_started"])
            self.assertFalse(receipt["data_migrated"])
            self.assertNotIn("private-fixture-credential", prepared.stdout)
            self.assertFalse((root / "units").exists(), "units are staged under the root for review")
            self.assertEqual((root / "source/input.txt").read_text(), "source stays unchanged")
            again = self.run_cli(Path(receipt["configuration"]["deployment"]), "--prepare-storage")
            self.assertEqual(again.returncode, 0, again.stderr)
            self.assertEqual(json.loads(again.stdout), receipt)

    def test_storage_options_reject_deployment_actions_before_side_effects(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            config = self.fixture(root)
            for args in (("--storage-plan", "--signal"),
                         ("--prepare-storage", "--maintenance"),
                         ("--prepare-storage", "--bundle", "/bundle"),
                         ("--storage-plan", "--status")):
                with self.subTest(args=args):
                    result = self.run_cli(config, *args)
                    self.assertEqual(result.returncode, 2)
                    self.assertFalse((root / "platform").exists())
