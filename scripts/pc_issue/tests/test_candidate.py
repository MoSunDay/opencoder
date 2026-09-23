from pathlib import Path
import json
import sys
import tempfile
import unittest
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from candidate import load_manifest, validate_archive
from evidence import evidence


class Candidate(unittest.TestCase):
    def test_archive_escape_rejected_before_remote_install(self):
        with tempfile.TemporaryDirectory() as tmp:
            for name in ('../existing.exe', r'C:\existing.exe', 'App/file:stream', r'..\existing.exe'):
                path=Path(tmp)/'candidate.zip'
                with zipfile.ZipFile(path,'w') as archive:
                    archive.writestr(name,b'candidate')
                with self.assertRaises(ValueError):
                    validate_archive(path)

    def test_manifest_freezes_download_and_application_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            archive=Path(tmp)/'candidate.zip'
            with zipfile.ZipFile(archive,'w') as z:
                z.writestr('app.exe',b'candidate')
            ref=evidence(archive)
            value={'archive_path':str(archive),'archive_sha256':ref['sha256'],'archive_bytes':ref['bytes'],
                   'download_sha256':ref['sha256'],'build_execution_id':'team-build',
                   'app_relative_path':'app.exe','package_relative_path':'Resources',
                   'app_sha256':'a'*64,'commit':'b'*40}
            path=Path(tmp)/'manifest.json';path.write_text(json.dumps(value))
            reference={'manifest_path':str(path),'sha256':evidence(path)['sha256']}
            self.assertEqual(load_manifest(reference),value)
            path.write_text('{}')
            with self.assertRaises(ValueError):
                load_manifest(reference)


if __name__=='__main__':
    unittest.main()
