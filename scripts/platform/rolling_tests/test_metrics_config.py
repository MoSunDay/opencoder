import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.config import load


class MetricsCredentialConfigTest(unittest.TestCase):
    def test_distinct_credential_is_required_before_release(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            server = root / "server.token"
            metrics = root / "metrics.token"
            config = root / "opencoder.json"
            server.write_text("admin")
            metrics.write_text("admin")
            config.write_text(json.dumps({"deployment": {
                "state_dir": str(root), "server_workdir": str(root),
                "server_data": str(root), "agent_workdir": str(root),
                "token_file": str(server), "metrics_token_file": str(metrics),
            }}))
            with self.assertRaisesRegex(ValueError, "distinct"):
                load(config)
            metrics.write_text("scoped")
            self.assertEqual(load(config).metrics_token_file, metrics)


if __name__ == "__main__":
    unittest.main()
