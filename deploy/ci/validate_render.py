#!/usr/bin/env python3
"""Validate `helm template` output for the Loom chart.

Run in CI after rendering one or more manifests:

    python3 deploy/ci/validate_render.py rendered/staging.yaml ...

Assertions per rendered file:
  * every document parses as YAML;
  * required kinds are present (Deployment, Service, ServiceAccount);
  * the loom-server container env list has no duplicate names and exactly one
    DATABASE_URL (Kubernetes keeps the last duplicate, so a stray second entry
    would silently re-point the server);
  * DATABASE_URL / LOOM_ENCRYPTION_KEY / LOOM_ROOT_ADMIN_TOKEN are wired via
    `secretKeyRef` (never as a literal `value`), so no secret is ever baked into
    a manifest;
  * if PGSSLROOTCERT is set it points at a file inside a read-only mount of a
    `secret` volume that projects exactly ONE item (the CA certificate) whose
    key does not look like private-key material; PGSSLCERT/PGSSLKEY are absent;
  * no volume in the pod projects a Secret key that looks like a private key;
  * the container image is either `repo:tag` or `repo@sha256:<64 hex>`;
  * probes point at /healthz (liveness) and /readyz (readiness).

Exit non-zero on any failure so the CI job goes red. The check functions take
parsed documents so they can be unit-tested (see test_validate_render.py).
"""
import posixpath
import re
import sys

import yaml

SECRET_ENV = {"DATABASE_URL", "LOOM_ENCRYPTION_KEY", "LOOM_ROOT_ADMIN_TOKEN", "LOOM_KEY_PEPPER"}
REQUIRED_SECRET_ENV = ("DATABASE_URL", "LOOM_ENCRYPTION_KEY", "LOOM_ROOT_ADMIN_TOKEN")
REQUIRED_KINDS = {"Deployment", "Service", "ServiceAccount"}
# Client-side key material must never be configured through the chart.
FORBIDDEN_ENV = {"PGSSLCERT", "PGSSLKEY"}

PRIVATE_KEY_RE = re.compile(r"(^|[._-])(key|private)([._-]|$)", re.IGNORECASE)
DIGEST_IMAGE_RE = re.compile(r"^[^@]+@sha256:[0-9a-f]{64}$")


def looks_like_private_key(name: str) -> bool:
    """True if a Secret key name looks like key material (tls.key, ca.key, server-key.pem...)."""
    return bool(PRIVATE_KEY_RE.search(name or ""))


def load_docs(path: str) -> list[dict]:
    with open(path, encoding="utf-8") as fh:
        return [d for d in yaml.safe_load_all(fh) if d]


def find_server(docs: list[dict]) -> dict | None:
    deploys = [d for d in docs if d.get("kind") == "Deployment"]
    return next((d for d in deploys if "loom" in d["metadata"]["name"]), None)


def find_app_container(server: dict) -> dict:
    containers = server["spec"]["template"]["spec"]["containers"]
    return next((c for c in containers if c["name"] == "loom-server"), containers[0])


def check_env(app: dict) -> list[str]:
    """Env list shape: unique names, exactly one DATABASE_URL, secrets via secretKeyRef."""
    errs: list[str] = []
    env_list = app.get("env", []) or []
    names = [e.get("name") for e in env_list]
    for name in sorted({n for n in names if names.count(n) > 1}):
        errs.append(f"env {name} appears {names.count(name)} times (must be unique)")
    if names.count("DATABASE_URL") != 1:
        errs.append(f"DATABASE_URL must appear exactly once, found {names.count('DATABASE_URL')}")

    env = {e["name"]: e for e in env_list}
    for name in REQUIRED_SECRET_ENV:
        e = env.get(name)
        if e is None:
            errs.append(f"Deployment missing env {name}")
            continue
        if "value" in e:
            errs.append(f"SECRET {name} rendered as a literal value (must be secretKeyRef)")
        if "secretKeyRef" not in (e.get("valueFrom") or {}):
            errs.append(f"{name} must be sourced via secretKeyRef")

    # Any secret-shaped env that slipped through as a literal is a hard fail.
    for name, e in env.items():
        if name in SECRET_ENV and "value" in e:
            errs.append(f"SECRET {name} has an inline value")
        if name in FORBIDDEN_ENV:
            errs.append(f"env {name} must never be set: client key material is not supported")
    return errs


def check_ca_mount(server: dict, app: dict) -> list[str]:
    """PGSSLROOTCERT, if present, must be a read-only CA-only Secret projection."""
    errs: list[str] = []
    env = {e["name"]: e for e in app.get("env", []) or []}
    root = env.get("PGSSLROOTCERT")
    if root is None:
        return errs
    path = root.get("value")
    if not isinstance(path, str) or not path.startswith("/"):
        return [f"PGSSLROOTCERT must be a literal absolute file path, got {root!r}"]

    mounts = app.get("volumeMounts", []) or []
    mount = next(
        (m for m in mounts if path.startswith(m["mountPath"].rstrip("/") + "/")),
        None,
    )
    if mount is None:
        return [f"PGSSLROOTCERT {path} is not inside any volumeMount"]
    if mount.get("readOnly") is not True:
        errs.append(f"CA mount {mount['name']} at {mount['mountPath']} must be readOnly: true")

    volumes = {v["name"]: v for v in server["spec"]["template"]["spec"].get("volumes", []) or []}
    vol = volumes.get(mount["name"])
    if vol is None:
        return errs + [f"volumeMount {mount['name']} has no matching volume"]
    secret = vol.get("secret")
    if not isinstance(secret, dict):
        return errs + [f"CA volume {mount['name']} must be a `secret` volume"]
    items = secret.get("items")
    if not isinstance(items, list) or len(items) != 1:
        errs.append(f"CA volume {mount['name']} must project exactly one item, got {items!r}")
        return errs
    item = items[0]
    expected_file = posixpath.relpath(path, mount["mountPath"])
    if item.get("path") != expected_file:
        errs.append(
            f"CA volume item path {item.get('path')!r} does not match PGSSLROOTCERT file {expected_file!r}"
        )
    if looks_like_private_key(str(item.get("key", ""))):
        errs.append(f"CA volume projects Secret key {item.get('key')!r} which looks like a private key")
    return errs


def check_no_private_keys_projected(server: dict) -> list[str]:
    errs: list[str] = []
    for vol in server["spec"]["template"]["spec"].get("volumes", []) or []:
        secret = vol.get("secret")
        if not isinstance(secret, dict):
            continue
        for item in secret.get("items", []) or []:
            if looks_like_private_key(str(item.get("key", ""))):
                errs.append(f"volume {vol['name']} projects Secret key {item.get('key')!r} (private key material)")
    return errs


def check_image(app: dict) -> list[str]:
    image = str(app.get("image", ""))
    if "@" in image:
        if not DIGEST_IMAGE_RE.match(image):
            return [f"image {image!r} has a digest that is not sha256:<64 hex>"]
        return []
    last = image.rsplit("/", 1)[-1]
    if ":" not in last or last.endswith(":"):
        return [f"image {image!r} must carry a tag or a sha256 digest"]
    return []


def check_probes(app: dict) -> list[str]:
    errs: list[str] = []
    live = app.get("livenessProbe", {}).get("httpGet", {}).get("path")
    ready = app.get("readinessProbe", {}).get("httpGet", {}).get("path")
    if live != "/healthz":
        errs.append(f"livenessProbe path is {live!r}, expected /healthz")
    if ready != "/readyz":
        errs.append(f"readinessProbe path is {ready!r}, expected /readyz")
    return errs


def validate_docs(docs: list[dict]) -> list[str]:
    """All checks over already-parsed documents. Returns a list of error strings."""
    errs: list[str] = []
    kinds = {d.get("kind") for d in docs}
    for k in sorted(REQUIRED_KINDS):
        if k not in kinds:
            errs.append(f"missing required kind {k}")

    server = find_server(docs)
    if server is None:
        return errs + ["no loom Deployment found"]
    app = find_app_container(server)

    errs.extend(check_env(app))
    errs.extend(check_ca_mount(server, app))
    errs.extend(check_no_private_keys_projected(server))
    errs.extend(check_image(app))
    errs.extend(check_probes(app))
    return errs


def validate(path: str) -> list[str]:
    try:
        docs = load_docs(path)
    except yaml.YAMLError as e:
        return [f"{path}: invalid YAML: {e}"]
    return [f"{path}: {e}" for e in validate_docs(docs)]


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
