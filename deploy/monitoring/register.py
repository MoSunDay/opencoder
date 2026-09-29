#!/usr/bin/env python3
"""Register the OpenCoder scrape and dashboard in the existing k3s stack."""

import argparse
import base64
import hashlib
import json
import os
import re
import subprocess
import tempfile
import time
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen


NAMESPACE = "kube-system"
PROM_DEPLOYMENT = "viking-prometheus"
GRAFANA_DEPLOYMENT = "viking-grafana"
TOKEN_SECRET = "opencoder-prometheus-scrape-token"
DASHBOARD_CONFIG = "opencoder-scheduler-dashboard"
ROOT = Path(__file__).resolve().parent


def kubectl(*args, payload=None):
    command = ["kubectl", "-n", NAMESPACE, *args]
    result = subprocess.run(command, input=payload, text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(f"{' '.join(command)}: {result.stderr.strip()}")
    return result.stdout


def resource(kind, name):
    return json.loads(kubectl("get", kind, name, "-o", "json"))


def config_volume(deployment, name):
    volumes = deployment["spec"]["template"]["spec"]["volumes"]
    return next((index, volume) for index, volume in enumerate(volumes) if volume["name"] == name)


def merged_prometheus_config(current, fragment):
    if re.search(r"(?m)^- job_name:\s*opencoder-scheduler\s*$", current):
        if fragment.rstrip() not in current:
            raise ValueError("Existing opencoder-scheduler scrape differs; inspect it before replacing")
        return current
    merged = current.rstrip() + "\n" + fragment
    return merged


def validate_prometheus_config(config):
    # promtool checks file existence. The new Secret is mounted only after the
    # rollout, so use an existing credential path for syntax validation.
    check_config = config.replace(
        "/etc/prometheus/opencoder/token", "/etc/prometheus/secrets/codemaster-vllm-metrics-appkey"
    )
    command = ["kubectl", "-n", NAMESPACE, "exec", "-i", f"deployment/{PROM_DEPLOYMENT}",
               "--", "promtool", "check", "config", "/dev/stdin"]
    result = subprocess.run(command, input=check_config, text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(f"promtool rejected OpenCoder scrape: {result.stdout[-800:]} {result.stderr[-800:]}")


def patch_deployment(name, deployment, operations):
    if not operations:
        return False
    guard = {"op": "test", "path": "/metadata/resourceVersion", "value": deployment["metadata"]["resourceVersion"]}
    kubectl("patch", "deployment", name, "--type=json", "-p", json.dumps([guard, *operations]))
    return True


def read_token(token_file):
    token = Path(token_file).read_bytes().strip()
    if not token or any(byte <= 32 or byte >= 127 for byte in token):
        raise ValueError("Invalid OpenCoder token file")
    return token


def register_secret(token):
    encoded = base64.b64encode(token).decode()
    existing = kubectl("get", "secret", TOKEN_SECRET, "--ignore-not-found", "-o", "json")
    current = json.loads(existing) if existing.strip() else None
    if current and current["data"].get("token") == encoded:
        return False
    manifest = {"apiVersion": "v1", "kind": "Secret",
                "metadata": {"name": TOKEN_SECRET, "namespace": NAMESPACE},
                "type": "Opaque", "data": {"token": encoded}}
    if current:
        manifest["metadata"]["resourceVersion"] = current["metadata"]["resourceVersion"]
    kubectl("replace" if current else "create", "-f", "-", payload=json.dumps(manifest))
    return True


def verify_scoped_token(token, server_url):
    headers = {"Authorization": "Bearer " + token.decode("ascii")}
    for path, expected in (("/metrics", 200), ("/api/metrics/scheduler", 401)):
        request = Request(server_url.rstrip("/") + path, headers=headers)
        try:
            with urlopen(request, timeout=10) as response:
                status = response.status
        except HTTPError as error:
            status = error.code
        if status != expected:
            raise ValueError(f"metrics credential scope check failed for {path}: HTTP {status}")


def register_prometheus(secret_changed=False):
    deployment = resource("deployment", PROM_DEPLOYMENT)
    volume_index, volume = config_volume(deployment, "config")
    source = resource("configmap", volume["configMap"]["name"])
    data = dict(source["data"])
    data["prometheus.yml"] = merged_prometheus_config(
        data["prometheus.yml"], (ROOT / "prometheus-scrape.yml").read_text()
    )
    validate_prometheus_config(data["prometheus.yml"])
    digest = hashlib.sha256(json.dumps(data, sort_keys=True).encode()).hexdigest()[:16]
    name = f"viking-prometheus-config-opencoder-{digest}"
    kubectl("apply", "-f", "-", payload=json.dumps({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": name, "namespace": NAMESPACE,
                     "labels": {"app.kubernetes.io/managed-by": "opencoder-monitoring"}},
        "immutable": True, "data": data,
    }))
    operations = []
    if volume["configMap"]["name"] != name:
        operations.append({"op": "replace", "path": f"/spec/template/spec/volumes/{volume_index}/configMap/name", "value": name})
    pod = deployment["spec"]["template"]["spec"]
    if not any(item["name"] == TOKEN_SECRET for item in pod["volumes"]):
        operations.append({"op": "add", "path": "/spec/template/spec/volumes/-", "value": {
            "name": TOKEN_SECRET, "secret": {"secretName": TOKEN_SECRET, "defaultMode": 256},
        }})
    containers = pod["containers"]
    index = next(i for i, item in enumerate(containers) if item["name"] == "prometheus")
    mounts = containers[index].get("volumeMounts", [])
    if not any(item["name"] == TOKEN_SECRET for item in mounts):
        operations.append({"op": "add", "path": f"/spec/template/spec/containers/{index}/volumeMounts/-", "value": {
            "name": TOKEN_SECRET, "mountPath": "/etc/prometheus/opencoder", "readOnly": True,
        }})
    if secret_changed and not operations:
        annotations = dict(deployment["spec"]["template"]["metadata"].get("annotations", {}))
        annotations["opencoder.monitoring/token-refresh"] = str(time.time_ns())
        operations.append({"op": "add", "path": "/spec/template/metadata/annotations", "value": annotations})
    changed = patch_deployment(PROM_DEPLOYMENT, deployment, operations)
    return name, changed


def register_grafana():
    dashboard = json.loads((ROOT / "opencoder-scheduler-dashboard.json").read_text())
    if dashboard["uid"] != "opencoder-scheduler":
        raise ValueError("Unexpected dashboard UID")
    kubectl("apply", "-f", "-", payload=json.dumps({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": DASHBOARD_CONFIG, "namespace": NAMESPACE,
                     "labels": {"app.kubernetes.io/managed-by": "opencoder-monitoring"}},
        "data": {"opencoder-scheduler.json": json.dumps(dashboard, ensure_ascii=False)},
    }))
    deployment = resource("deployment", GRAFANA_DEPLOYMENT)
    index, volume = config_volume(deployment, "dashboards")
    sources = volume["projected"]["sources"]
    if any(item.get("configMap", {}).get("name") == DASHBOARD_CONFIG for item in sources):
        return False
    return patch_deployment(GRAFANA_DEPLOYMENT, deployment, [{
        "op": "add", "path": f"/spec/template/spec/volumes/{index}/projected/sources/-",
        "value": {"configMap": {"name": DASHBOARD_CONFIG}},
    }])


def rollback_snapshot():
    prometheus = resource("deployment", PROM_DEPLOYMENT)
    grafana = resource("deployment", GRAFANA_DEPLOYMENT)
    _, volume = config_volume(prometheus, "config")
    pod = prometheus["spec"]["template"]["spec"]
    container = next(item for item in pod["containers"] if item["name"] == "prometheus")
    _, dashboards = config_volume(grafana, "dashboards")
    return {
        "prometheus_config": volume["configMap"]["name"],
        "had_token_volume": any(item["name"] == TOKEN_SECRET for item in pod["volumes"]),
        "had_token_mount": any(item["name"] == TOKEN_SECRET for item in container.get("volumeMounts", [])),
        "had_dashboard": any(item.get("configMap", {}).get("name") == DASHBOARD_CONFIG
                             for item in dashboards["projected"]["sources"]),
    }


def write_journal(filename, snapshot):
    file = Path(filename)
    def check_existing():
        previous = load_journal(file)
        current = snapshot["prometheus_config"]
        if current != previous["prometheus_config"] and not current.startswith(
            "viking-prometheus-config-opencoder-"
        ):
            raise ValueError("Prometheus changed outside the registered configuration")
    if file.exists():
        check_existing()
        return
    file.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=".monitoring-rollback-", dir=file.parent)
    try:
        with os.fdopen(fd, "w") as output:
            json.dump(snapshot, output)
            output.flush()
            os.fsync(output.fileno())
        try:
            os.link(temporary, file)
        except FileExistsError:
            check_existing()
        else:
            directory_fd = os.open(file.parent, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(directory_fd)
            finally:
                os.close(directory_fd)
    finally:
        os.unlink(temporary)


def load_journal(filename):
    anchor = json.loads(Path(filename).read_text())
    if not isinstance(anchor, dict) or not isinstance(anchor.get("prometheus_config"), str) \
            or not all(isinstance(anchor.get(key), bool) for key in
                       ("had_token_volume", "had_token_mount", "had_dashboard")):
        raise ValueError("Invalid monitoring rollback journal")
    return anchor


def restore(filename):
    anchor = load_journal(filename)
    resource("configmap", anchor["prometheus_config"])
    prometheus = resource("deployment", PROM_DEPLOYMENT)
    volume_index, volume = config_volume(prometheus, "config")
    current = volume["configMap"]["name"]
    if current != anchor["prometheus_config"]:
        if not current.startswith("viking-prometheus-config-opencoder-"):
            raise ValueError("Prometheus config changed outside this registration")
        operations = [{"op": "replace", "path": f"/spec/template/spec/volumes/{volume_index}/configMap/name",
                       "value": anchor["prometheus_config"]}]
    else:
        operations = []
    pod = prometheus["spec"]["template"]["spec"]
    if not anchor["had_token_volume"]:
        for index, item in enumerate(pod["volumes"]):
            if item["name"] == TOKEN_SECRET:
                operations.append({"op": "remove", "path": f"/spec/template/spec/volumes/{index}"})
    container_index = next(i for i, item in enumerate(pod["containers"])
                           if item["name"] == "prometheus")
    if not anchor["had_token_mount"]:
        mounts = pod["containers"][container_index].get("volumeMounts", [])
        for index, item in enumerate(mounts):
            if item["name"] == TOKEN_SECRET:
                operations.append({"op": "remove", "path": f"/spec/template/spec/containers/{container_index}/volumeMounts/{index}"})
    prom_changed = patch_deployment(PROM_DEPLOYMENT, prometheus, operations)
    grafana = resource("deployment", GRAFANA_DEPLOYMENT)
    volume_index, dashboards = config_volume(grafana, "dashboards")
    operations = []
    if not anchor["had_dashboard"]:
        for index, item in enumerate(dashboards["projected"]["sources"]):
            if item.get("configMap", {}).get("name") == DASHBOARD_CONFIG:
                operations.append({"op": "remove", "path": f"/spec/template/spec/volumes/{volume_index}/projected/sources/{index}"})
    grafana_changed = patch_deployment(GRAFANA_DEPLOYMENT, grafana, operations)
    for deployment, changed in ((PROM_DEPLOYMENT, prom_changed), (GRAFANA_DEPLOYMENT, grafana_changed)):
        if changed:
            kubectl("rollout", "status", f"deployment/{deployment}", "--timeout=180s")
    return {"prometheus_restored": prom_changed, "grafana_restored": grafana_changed}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--token-file", default="/etc/opencoder/metrics.token")
    parser.add_argument("--server-url", default="http://10.199.71.70:18081")
    parser.add_argument("--journal", default="/var/lib/opencoder/monitoring-rollback.json")
    parser.add_argument("--restore", action="store_true")
    args = parser.parse_args()
    if args.restore:
        print(json.dumps(restore(args.journal)))
        return
    token = read_token(args.token_file)
    verify_scoped_token(token, args.server_url)
    write_journal(args.journal, rollback_snapshot())
    secret_changed = register_secret(token)
    config_name, prom_changed = register_prometheus(secret_changed)
    grafana_changed = register_grafana()
    for deployment, changed in [(PROM_DEPLOYMENT, prom_changed), (GRAFANA_DEPLOYMENT, grafana_changed)]:
        if changed:
            kubectl("rollout", "status", f"deployment/{deployment}", "--timeout=180s")
    print(json.dumps({"prometheus_config": config_name, "prometheus_updated": prom_changed,
                      "grafana_updated": grafana_changed}, ensure_ascii=False))


if __name__ == "__main__":
    main()
