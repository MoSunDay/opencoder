"""Lost network replies must reuse the durable public acceptance."""
import copy
from pathlib import Path
import sys
import tempfile
import unittest
from types import SimpleNamespace
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.io import HttpFailure
from rolling.probes import candidate_locked, probe_id, public, spec, submit_probe


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

    def test_timeout_preserves_the_runtime_rejection(self):
        operations = Candidate(self.record)
        original = operations.http
        def http(base, path, method="GET", body=None):
            if body and body["operation"] == "create":
                return {"status": 503, "body": {"error": "node admission is frozen"}}
            return original(base, path, method, body)
        def wait(check, seconds):
            value = check()
            if value:
                return value
            raise TimeoutError("deployment probe")
        operations.http, operations.wait = http, wait
        with self.assertRaisesRegex(TimeoutError, "create.*503.*node admission is frozen") as raised:
            candidate_locked(None, self.record, operations, 1)
        self.assertEqual(str(raised.exception.__cause__), "deployment probe")

    def test_recovered_probe_still_requires_successful_runtime_execution(self):
        operations = Candidate(self.record, accepted=True)
        original = operations.http
        def http(base, path, *args):
            reply = original(base, path, *args)
            if path == "/inventory":
                reply["indexes"][0]["status"] = "error"
            return reply
        operations.http = http
        with self.assertRaisesRegex(ValueError, "candidate probe failed"):
            candidate_locked(None, self.record, operations, 1)
        self.assertEqual(operations.creates, 0)

    def test_recovered_probe_rejects_changed_input_and_activation_identity(self):
        for field in ("input", "created_at", "node_id"):
            operations = Candidate(self.record, accepted=True)
            original = operations.http
            def http(base, path, *args):
                reply = original(base, path, *args)
                if path == "/rpc":
                    detail = reply["body"]
                    if field == "input":
                        detail["request"][field] = {"changed": True}
                    else:
                        detail["execution"][field] = "different"
                return reply
            operations.http = http
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "conflicts"):
                candidate_locked(None, self.record, operations, 1)
            self.assertEqual(operations.creates, 0)


class PublicProbeTests(unittest.TestCase):
    def setUp(self):
        self.record = {"id": "release-local", "runtime_port": 3100,
                       "manifest": {"commit": "local-commit"}}
        self.inventory = {"runtime_id": "release-local", "build": {"git_commit": "local-commit"},
                          "registration": {"id": "node-local"}}
        self.node_id = "node-local"
        self.accepted = False
        self.posts = []

    def http(self, base, path, method="GET", body=None):
        if path == "/api/admin/release":
            return {"instance_release": "release-local"}
        if path == "/inventory":
            self.assertEqual(base, "http://127.0.0.1:3100")
            return self.inventory
        if path.endswith("/receipt"):
            if self.accepted:
                return {"phase": "accepted"}
            raise HttpFailure(method, path, 404, "no receipt")
        if method == "POST":
            self.posts.append(copy.deepcopy(body))
            self.accepted = True
            return {"id": body["id"]}
        return {"execution": {"status": "done", "node_id": self.node_id}}

    def wait(self, check, seconds):
        result = check()
        self.assertTrue(result)
        return result

    def test_public_probe_uses_published_runtime_node_and_recovers_same_receipt(self):
        settings = SimpleNamespace(public_url="http://public")
        public(settings, self.record, self, 1)
        public(settings, self.record, self, 1)
        self.assertEqual(len(self.posts), 1)
        self.assertEqual(self.posts[0]["node_id"], "node-local")
        self.assertEqual(self.posts[0]["id"], probe_id(self.record, public=True))

    def test_success_on_another_node_cannot_validate_this_release(self):
        self.node_id = "node-unrelated"
        with self.assertRaisesRegex(ValueError, "another node"):
            public(SimpleNamespace(public_url="http://public"), self.record, self, 1)

    def test_wrong_runtime_or_binary_is_rejected_before_submission(self):
        for field in ("runtime_id", "build"):
            original = copy.deepcopy(self.inventory)
            self.inventory[field] = {"git_commit": "old"} if field == "build" else "other"
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "activated release"):
                public(SimpleNamespace(public_url="http://public"), self.record, self, 1)
            self.inventory = original
        self.assertEqual(self.posts, [])


if __name__ == "__main__":
    unittest.main()
