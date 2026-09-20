"""Lost network replies must reuse the durable public acceptance."""
import copy
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
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
    def test_candidate_recovers_durable_acceptance_and_checks_live_terminal_state(self):
        with tempfile.TemporaryDirectory() as directory:
            record = {"id": "release-one", "runtime_data": directory, "runtime_port": 1234,
                      "manifest": {"commit": "abc"}, "created_at": 123, "probe_epoch": 2}
            identifier = probe_id(record)
            assignment = {"index": {"id": identifier, "kind": "dag", "node_id": "node-one",
                                    "created_at": 123, "status": "done"},
                          "request": {"id": identifier, "kind": "dag", "node_id": "node-one",
                                      "target": None, "input": {}}, "definition": spec()}
            path = Path(directory) / "dag" / identifier / "execution.json"
            path.parent.mkdir(parents=True)
            path.write_text(json.dumps({"assignment": assignment}))
            calls = []
            def http(base, route, *args):
                calls.append(route)
                self.assertEqual(route, "/inventory", "accepted probes must not be recreated")
                return {"runtime_id": "release-one", "build": {"git_commit": "abc"},
                        "registration": {"id": "node-one"}, "indexes": [assignment["index"]],
                        "snapshot": {"ready": True}}
            operations = SimpleNamespace(http=http, wait=lambda check, seconds: check())
            self.assertEqual(candidate_locked(None, record, operations, 1), "node-one")
            self.assertEqual(calls, ["/inventory", "/inventory"])
            assignment["index"]["status"] = "error"
            with self.assertRaisesRegex(ValueError, "candidate probe failed"):
                candidate_locked(None, record, operations, 1)
            for key in ("request", "definition", "index"):
                conflicting = copy.deepcopy(assignment)
                if key == "request":
                    conflicting[key]["input"] = {"changed": True}
                elif key == "definition":
                    conflicting[key] = {}
                else:
                    conflicting[key]["created_at"] += 1
                path.write_text(json.dumps({"assignment": conflicting}))
                with self.subTest(key=key), self.assertRaisesRegex(ValueError, "differs from activation"):
                    candidate_locked(None, record, operations, 1)

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
