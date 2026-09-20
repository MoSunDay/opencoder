"""Lost network replies must reuse the durable public acceptance."""
import copy
from pathlib import Path
import sys
import tempfile
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.io import HttpFailure
from rolling.probes import candidate_locked, probe_id, spec, submit_probe


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


class Candidate:
    def __init__(self, record, accepted=False, lose_reply=False):
        self.record = record
        self.accepted = accepted
        self.lose_reply = lose_reply
        self.creates = 0
        self.definition = spec()

    def http(self, base, path, method="GET", body=None):
        index = {"id": probe_id(self.record), "kind": "dag", "node_id": "node-test",
                 "created_at": self.record["created_at"], "status": "done"}
        if path == "/inventory":
            return {"runtime_id": self.record["id"], "build": {"git_commit": "commit"},
                    "registration": {"id": "node-test"}, "snapshot": {"ready": True},
                    "indexes": [index] if self.accepted else []}
        if body["operation"] == "inspect":
            if not self.accepted:
                return {"status": 404}
            return {"status": 200, "body": {"execution": index, "definition": self.definition,
                    "request": {"id": index["id"], "kind": "dag", "node_id": "node-test", "input": {}, "target": None}}}
        self.creates += 1
        self.accepted = True
        if self.lose_reply:
            raise TimeoutError("acceptance reply lost")
        return {"status": 200}

    def wait(self, check, seconds):
        for _ in range(3):
            try:
                result = check()
                if result:
                    return result
            except TimeoutError:
                pass
        raise AssertionError("candidate did not become ready")


class CandidateProbeTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.record = {"id": "release-test", "probe_epoch": 2, "runtime_port": 3100,
                       "runtime_data": self.directory.name, "created_at": 123,
                       "manifest": {"commit": "commit"}}

    def test_resumed_activation_recovers_completed_probe_without_admission(self):
        operations = Candidate(self.record, accepted=True)
        self.assertEqual(candidate_locked(None, self.record, operations, 1), "node-test")
        self.assertEqual(operations.creates, 0)

    def test_lost_create_reply_is_recovered_without_duplicate_submission(self):
        operations = Candidate(self.record, lose_reply=True)
        self.assertEqual(candidate_locked(None, self.record, operations, 1), "node-test")
        self.assertEqual(operations.creates, 1)

    def test_same_id_with_different_definition_is_rejected(self):
        operations = Candidate(self.record, accepted=True)
        operations.definition = {"name": "another task", "steps": []}
        with self.assertRaisesRegex(ValueError, "conflicts"):
            candidate_locked(None, self.record, operations, 1)
        self.assertEqual(operations.creates, 0)


if __name__ == "__main__":
    unittest.main()
