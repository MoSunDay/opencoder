"""Delivery evidence belongs to the completed job, including no-ticket decisions."""
import sys
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from validation.delivery import delivered


class DeliveryEvidence(unittest.TestCase):
    def test_not_required_does_not_bypass_job_identity_or_completion(self):
        job = {'jobId': 'job-local', 'analysis': {'status': 'done'}, 'viking': {'status': 'not_required'}}
        self.assertTrue(delivered(job, 'job-local', []))
        self.assertFalse(delivered(job, 'job-other', []))
        self.assertFalse(delivered({**job, 'analysis': {'status': 'running'}}, 'job-local', []))

    def test_matching_receipt_is_required_for_both_supported_delivery_types(self):
        job = {'jobId': 'job-local', 'analysis': {'status': 'done'}, 'viking': None}
        self.assertFalse(delivered(job, 'job-local', []))
        for kind in ['local-delivery', 'local-viking']:
            receipt = {'type': kind, 'jobId': 'job-local'}
            self.assertTrue(delivered(job, 'job-local', [receipt]))
            self.assertFalse(delivered(job, 'job-local', [{**receipt, 'jobId': 'job-other'}]))
        self.assertFalse(delivered(job, 'job-local', [{'type': 'local-create', 'jobId': 'job-local'}]))


if __name__ == '__main__':
    unittest.main()
