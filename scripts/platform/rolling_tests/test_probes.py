"""Lost network replies must reuse the durable public acceptance."""
import copy
from pathlib import Path
import sys
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.io import HttpFailure
from rolling.probes import submit_probe


class LostReply:
    def __init__(self, persist=True, rejection=None):
        self.persist = persist
        self.rejection = rejection
        self.accepted = False
        self.posts = []
        self.queries = 0

    def http(self, base, path, method="GET", body=None):
        if method == "GET":
            self.queries += 1
            if not self.accepted:
                raise HttpFailure(method, path, 404, "no receipt")
            return {"phase": "accepted", "receipt": {"status": 202}}
        self.posts.append(copy.deepcopy(body))
        if self.rejection:
            raise HttpFailure(method, path, self.rejection, "rejected input")
        if len(self.posts) == 1:
            self.accepted = self.persist
            raise HttpFailure(method, path, 504, "node reply lost")
        self.accepted = True
        return {"id": body["id"]}

    def wait(self, check, seconds):
        for _ in range(3):
            if check():
                return
        raise AssertionError("probe did not recover")


class ProbeTests(unittest.TestCase):
    def test_lost_acceptance_reply_is_recovered_by_receipt_without_resubmit(self):
        operations = LostReply()
        request = {"id": "dag-probe-fixed", "kind": "dag", "input": {"definition": "frozen"}}
        submit_probe(operations, "http://localhost", request["id"], request, 1)
        self.assertEqual(operations.posts, [request])
        self.assertEqual(operations.queries, 2)
        # A restarted deployer consults the very same persisted receipt.
        submit_probe(operations, "http://localhost", request["id"], request, 1)
        self.assertEqual(operations.posts, [request])

    def test_unconfirmed_request_retries_identical_id_and_input(self):
        operations = LostReply(persist=False)
        request = {"id": "dag-probe-fixed", "kind": "dag", "input": {"definition": "frozen"}}
        submit_probe(operations, "http://localhost", request["id"], request, 1)
        self.assertEqual(operations.posts, [request, request])

    def test_conflicting_input_is_not_retried_or_reported_ready(self):
        operations = LostReply(rejection=409)
        with self.assertRaisesRegex(ValueError, "409"):
            submit_probe(operations, "http://localhost", "fixed", {"id": "fixed"}, 1)
        self.assertEqual(len(operations.posts), 1)


if __name__ == "__main__":
    unittest.main()
