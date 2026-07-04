#!/usr/bin/env python3
"""Validate `helm template` output for the Loom chart.

Run in CI after rendering one or more manifests:

    python3 deploy/ci/validate_render.py rendered/staging.yaml ...

Assertions per rendered file:
  * every document parses as YAML;
  * required kinds are present (Deployment, Service, ServiceAccount);
  * the loom-server Deployment wires DATABASE_URL / LOOM_ENCRYPTION_KEY /
    LOOM_ROOT_ADMIN_TOKEN via `secretKeyRef` (never as a literal `value`), so no
    secret is ever baked into a manifest;
  * probes point at /healthz (liveness) and /readyz (readiness).

Exit non-zero on any failure so the CI job goes red.
"""
import sys
import yaml

SECRET_ENV = {"DATABASE_URL", "LOOM_ENCRYPTION_KEY", "LOOM_ROOT_ADMIN_TOKEN", "LOOM_KEY_PEPPER"}
REQUIRED_KINDS = {"Deployment", "Service", "ServiceAccount"}


def validate(path: str) -> list[str]:
    errs: list[str] = []
    try:
        docs = [d for d in yaml.safe_load_all(open(path)) if d]
    except yaml.YAMLError as e:
        return [f"{path}: invalid YAML: {e}"]

    kinds = {d.get("kind") for d in docs}
    for k in REQUIRED_KINDS:
        if k not in kinds:
            errs.append(f"{path}: missing required kind {k}")

    deploys = [d for d in docs if d.get("kind") == "Deployment"]
    server = next(
        (d for d in deploys if d["metadata"]["name"].endswith("loom") or "loom" in d["metadata"]["name"]),
        None,
    )
    if server is None:
        return errs + [f"{path}: no loom Deployment found"]

    containers = server["spec"]["template"]["spec"]["containers"]
    app = next((c for c in containers if c["name"] == "loom-server"), containers[0])

    env = {e["name"]: e for e in app.get("env", [])}
    for name in ("DATABASE_URL", "LOOM_ENCRYPTION_KEY", "LOOM_ROOT_ADMIN_TOKEN"):
        e = env.get(name)
        if e is None:
            errs.append(f"{path}: Deployment missing env {name}")
            continue
        if "value" in e:
            errs.append(f"{path}: SECRET {name} rendered as a literal value (must be secretKeyRef)")
        if "valueFrom" not in e or "secretKeyRef" not in e.get("valueFrom", {}):
            errs.append(f"{path}: {name} must be sourced via secretKeyRef")

    # Any secret-shaped env that slipped through as a literal is a hard fail.
    for name, e in env.items():
        if name in SECRET_ENV and "value" in e:
            errs.append(f"{path}: SECRET {name} has an inline value")

    live = app.get("livenessProbe", {}).get("httpGet", {}).get("path")
    ready = app.get("readinessProbe", {}).get("httpGet", {}).get("path")
    if live != "/healthz":
        errs.append(f"{path}: livenessProbe path is {live!r}, expected /healthz")
    if ready != "/readyz":
        errs.append(f"{path}: readinessProbe path is {ready!r}, expected /readyz")

    return errs


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print("usage: validate_render.py <rendered.yaml> [...]", file=sys.stderr)
        return 2
    all_errs: list[str] = []
    for path in argv[1:]:
        all_errs.extend(validate(path))
    if all_errs:
        print("RENDER VALIDATION FAILED:")
        for e in all_errs:
            print("  -", e)
        return 1
    print(f"Render validation PASSED for {len(argv) - 1} file(s).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
