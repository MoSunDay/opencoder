from pathlib import Path
import json
import sys
import tempfile
import unittest
import zipfile
from unittest.mock import Mock, patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
sys.path.insert(0,'/opt/device-cases/harness')
from candidate import deploy_candidate, validate_archive


class Deployment(unittest.TestCase):
    def test_duplicate_windows_names_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'package.zip'
            with zipfile.ZipFile(path,'w') as z:
                z.writestr('App/Bun.exe',b'a'); z.writestr('app/bun.EXE',b'b')
            with self.assertRaisesRegex(ValueError,'duplicate Windows'):
                validate_archive(path)

    def test_guest_corruption_rejected_before_extract_and_profile(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);driver=Mock()
            driver.json.return_value={'bytes':12,'sha256':'wrong'}
            manifest={'archive_path':'local.zip','archive_bytes':12,'archive_sha256':'expected'}
            with patch('candidate.load_manifest',return_value=manifest), patch('transfer_chunked.upload_candidate'):
                with self.assertRaisesRegex(ValueError,'differs before extraction'):
                    deploy_candidate(driver,root,{'sha256':'frozen'}, {})
            self.assertFalse(any('Expand-Archive' in str(c) for c in driver.ps.call_args_list))
            self.assertFalse((root/'candidate.json').exists())
            self.assertEqual(json.loads((root/'candidate-deployment.json').read_text())['stage'],'uploading')

    def test_verified_profile_and_inventory_are_per_work(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);driver=Mock();driver.machine={'name':'win-19'}
            manifest={'archive_path':'local.zip','archive_bytes':12,'archive_sha256':'expected',
                      'app_relative_path':'App/app.exe','package_relative_path':'App/Resources',
                      'app_sha256':'exe','commit':'final'}
            driver.json.side_effect=[{'bytes':12,'sha256':'expected'},
                                    {'app_path':'candidate','app_commit':'final'},
                                    {'path':'inventory','sha256':'all-files'}]
            original={'app_path':'registered'}
            with patch('candidate.load_manifest',return_value=manifest), patch('transfer_chunked.upload_candidate'):
                profile=deploy_candidate(driver,root,{'sha256':'frozen'},original)
            self.assertEqual(original,{'app_path':'registered'})
            self.assertEqual(profile['app_commit'],'final')
            self.assertEqual(json.loads((root/'candidate-deployment.json').read_text())['stage'],'verified')
            self.assertEqual(json.loads((root/'candidate.json').read_text())['registered_profile_unchanged'],original)


if __name__=='__main__':unittest.main()
