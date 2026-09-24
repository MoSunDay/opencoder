from pathlib import Path
import sys,tempfile,unittest,zipfile
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
sys.path.insert(0,'/opt/device-cases/harness')
from candidate_runtime import guarded_payload,HOST_READY


class Runtime(unittest.TestCase):
    def test_guard_precedes_business_and_original_is_unchanged(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);original=root/'original.zip'
            script=HOST_READY+'\n $submitBusiness = $true\n'
            with zipfile.ZipFile(original,'w') as z:
                z.writestr('tools/Invoke-Campaign.ps1',script.encode('utf-8-sig'))
                z.writestr('profile.json',b'original profile')
                z.writestr('tools/runtime/UI-Checkpoint.ps1','$deadline=(Get-Date).AddMinutes(20)')
            before=original.read_bytes()
            with patch('controller.package.payload',return_value=original):
                output=guarded_payload(root,None)
            self.assertEqual(before,original.read_bytes())
            with zipfile.ZipFile(output) as z:
                body=z.read('tools/Invoke-Campaign.ps1').decode('utf-8-sig')
                self.assertLess(body.index('Candidate sandbox readiness failed'),body.index('$submitBusiness'))
                self.assertIn('!$sandbox.capabilities.supported',body)
                self.assertIn('!$sandbox.capabilities.initialized',body)
                self.assertEqual(z.read('profile.json'),b'original profile')
                self.assertIn(b'AddMinutes(75)',z.read('tools/runtime/UI-Checkpoint.ps1'))

    def test_changed_frozen_hook_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);archive=root/'original.zip'
            with zipfile.ZipFile(archive,'w') as z:z.writestr('tools/Invoke-Campaign.ps1','unexpected')
            with patch('controller.package.payload',return_value=archive),self.assertRaisesRegex(ValueError,'hook changed'):
                guarded_payload(root,None)


if __name__=='__main__':unittest.main()
