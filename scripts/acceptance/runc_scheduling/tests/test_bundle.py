"""Native acceptance uses the release verifier before creating its runtime."""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
spec = importlib.util.spec_from_file_location('native_acceptance_main', ROOT / 'main.py')
main = importlib.util.module_from_spec(spec)
spec.loader.exec_module(main)
sys.path.insert(0, str(ROOT.parents[1] / 'platform'))


class BundleTests(unittest.TestCase):
    def test_unverified_bundle_cannot_start_or_create_runtime(self):
        with tempfile.TemporaryDirectory() as directory:
            bundle = Path(directory)
            with patch.object(main, 'Runtime') as runtime:
                from rolling.manifest import _installer
                with self.assertRaises(_installer.InstallError):
                    main.bundle_binaries(bundle)
                runtime.assert_not_called()
            self.assertEqual(list(bundle.iterdir()), [])

    def test_canonical_verifier_selects_only_packaged_binaries(self):
        with patch('rolling.manifest.verify') as verify:
            bundle = Path('/private/candidate')
            paths = main.bundle_binaries(bundle)
            verify.assert_called_once_with(bundle)
            self.assertEqual(paths, {name: str(bundle / 'bin' / name)
                for name in ('opencoder-server', 'opencoder-agent')})


if __name__ == '__main__':
    unittest.main()
