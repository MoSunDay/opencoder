"""The registration preflight rejects an administrator credential."""

import tempfile
import json
import unittest
from pathlib import Path
from unittest.mock import patch

import register


class ScopePreflightTest(unittest.TestCase):
    def test_only_metrics_get_is_allowed(self):
        with tempfile.TemporaryDirectory() as directory:
            token_file = Path(directory) / "metrics.token"
            token_file.write_text("scoped-token\n")
            token = register.read_token(token_file)
            paths = []

            class Reply:
                status = 200

                def __enter__(self):
                    return self

                def __exit__(self, *_):
                    return False

            def scoped(request, timeout):
                self.assertEqual(timeout, 10)
                self.assertEqual(request.get_header("Authorization"), "Bearer scoped-token")
                path = request.full_url.removeprefix("http://server")
                paths.append(path)
                if path == "/api/metrics/scheduler":
                    from urllib.error import HTTPError
                    raise HTTPError(request.full_url, 401, "unauthorized", {}, None)
                return Reply()

            with patch.object(register, "urlopen", side_effect=scoped):
                register.verify_scoped_token(token, "http://server")
            self.assertEqual(paths, ["/metrics", "/api/metrics/scheduler"])

            with patch.object(register, "urlopen", return_value=Reply()):
                with self.assertRaisesRegex(ValueError, "scope check failed"):
                    register.verify_scoped_token(token, "http://server")

    def test_journal_keeps_first_anchor_across_retry(self):
        with tempfile.TemporaryDirectory() as directory:
            filename = Path(directory) / "rollback.json"
            first = {"prometheus_config": "original", "had_token_volume": False,
                     "had_token_mount": False, "had_dashboard": False}
            register.write_journal(filename, first)
            register.write_journal(filename, {"prometheus_config": "viking-prometheus-config-opencoder-abc"})
            self.assertEqual(json.loads(filename.read_text()), first)
            self.assertEqual(filename.stat().st_mode & 0o777, 0o600)
            filename.write_text("{")
            with patch.object(register, "resource") as read:
                with self.assertRaises(json.JSONDecodeError):
                    register.restore(filename)
                read.assert_not_called()

    def test_restore_changes_only_registered_deployment_fields(self):
        prometheus = {"metadata": {"resourceVersion": "1"}, "spec": {"template": {"spec": {
            "volumes": [
                {"name": "config", "configMap": {"name": "viking-prometheus-config-opencoder-abc"}},
                {"name": register.TOKEN_SECRET, "secret": {"secretName": register.TOKEN_SECRET}},
            ],
            "containers": [{"name": "prometheus", "volumeMounts": [
                {"name": register.TOKEN_SECRET, "mountPath": "/etc/prometheus/opencoder"}
            ]}],
        }}}}
        grafana = {"metadata": {"resourceVersion": "2"}, "spec": {"template": {"spec": {
            "volumes": [{"name": "dashboards", "projected": {"sources": [
                {"configMap": {"name": "existing"}},
                {"configMap": {"name": register.DASHBOARD_CONFIG}},
            ]}}],
        }}}}
        resources = {register.PROM_DEPLOYMENT: prometheus, register.GRAFANA_DEPLOYMENT: grafana}
        with tempfile.TemporaryDirectory() as directory:
            filename = Path(directory) / "rollback.json"
            register.write_journal(filename, {"prometheus_config": "original", "had_token_volume": False,
                                              "had_token_mount": False, "had_dashboard": False})
            with patch.object(register, "resource", side_effect=lambda _, name: resources.get(name, {})), \
                 patch.object(register, "patch_deployment", return_value=False) as apply:
                register.restore(filename)
            prom_ops = apply.call_args_list[0].args[2]
            grafana_ops = apply.call_args_list[1].args[2]
            self.assertEqual([op["op"] for op in prom_ops], ["replace", "remove", "remove"])
            self.assertEqual(prom_ops[0]["value"], "original")
            self.assertEqual(grafana_ops, [{"op": "remove", "path":
                "/spec/template/spec/volumes/0/projected/sources/1"}])


if __name__ == "__main__":
    unittest.main()
