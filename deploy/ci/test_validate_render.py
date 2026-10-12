#!/usr/bin/env python3
"""Unit tests for validate_render.py — synthetic manifests, no helm needed.

    python3 -m unittest deploy/ci/test_validate_render.py -v
"""
import copy
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import validate_render as vr  # noqa: E402

GOOD_DIGEST = "sha256:" + "ab" * 32


def secret_env(name: str, secret: str = "loom-secrets", key: str | None = None) -> dict:
    return {"name": name, "valueFrom": {"secretKeyRef": {"name": secret, "key": key or name}}}


def make_docs(
    *,
    env: list[dict] | None = None,
    extra_env: list[dict] | None = None,
    image: str = "ghcr.io/pukekos/loom:0.1.0",
    mounts: list[dict] | None = None,
    volumes: list[dict] | None = None,
) -> list[dict]:
    """Factory for a minimal valid render; every test derives its own copy."""
    base_env = env if env is not None else [
        {"name": "LOOM_BIND_ADDR", "value": "0.0.0.0:8080"},
        secret_env("DATABASE_URL"),
        secret_env("LOOM_ENCRYPTION_KEY"),
        secret_env("LOOM_ROOT_ADMIN_TOKEN"),
    ]
    container = {
        "name": "loom-server",
        "image": image,
        "env": copy.deepcopy(base_env) + copy.deepcopy(extra_env or []),
        "livenessProbe": {"httpGet": {"path": "/healthz"}},
        "readinessProbe": {"httpGet": {"path": "/readyz"}},
        "volumeMounts": [{"name": "tmp", "mountPath": "/tmp"}] + copy.deepcopy(mounts or []),
    }
    deployment = {
        "kind": "Deployment",
        "metadata": {"name": "loom"},
        "spec": {"template": {"spec": {
            "containers": [container],
            "volumes": [{"name": "tmp", "emptyDir": {}}] + copy.deepcopy(volumes or []),
        }}},
    }
    return [
        {"kind": "ServiceAccount", "metadata": {"name": "loom"}},
        {"kind": "Service", "metadata": {"name": "loom"}},
        deployment,
    ]


def ca_mount(read_only: bool = True) -> dict:
    return {"name": "db-ca", "mountPath": "/etc/loom/db-ca", "readOnly": read_only}


def ca_volume(items: list[dict]) -> dict:
    return {"name": "db-ca", "secret": {"secretName": "db-ca", "items": items}}


def ca_env() -> dict:
    return {"name": "PGSSLROOTCERT", "value": "/etc/loom/db-ca/ca.crt"}


class EnvChecks(unittest.TestCase):
    def test_good_manifest_passes(self) -> None:
        self.assertEqual(vr.validate_docs(make_docs()), [])

    def test_duplicate_database_url_fails(self) -> None:
        docs = make_docs(extra_env=[secret_env("DATABASE_URL", secret="other")])
        errs = vr.validate_docs(docs)
        self.assertTrue(any("DATABASE_URL must appear exactly once" in e for e in errs), errs)
        self.assertTrue(any("appears 2 times" in e for e in errs), errs)

    def test_missing_database_url_fails(self) -> None:
        docs = make_docs(env=[secret_env("LOOM_ENCRYPTION_KEY"), secret_env("LOOM_ROOT_ADMIN_TOKEN")])
        errs = vr.validate_docs(docs)
        self.assertTrue(any("exactly once, found 0" in e for e in errs), errs)

    def test_literal_secret_fails(self) -> None:
        docs = make_docs(env=[
            {"name": "DATABASE_URL", "value": "postgres://x"},
            secret_env("LOOM_ENCRYPTION_KEY"),
            secret_env("LOOM_ROOT_ADMIN_TOKEN"),
        ])
        errs = vr.validate_docs(docs)
        self.assertTrue(any("literal value" in e for e in errs), errs)

    def test_client_key_env_fails(self) -> None:
        docs = make_docs(extra_env=[{"name": "PGSSLKEY", "value": "/x/tls.key"}])
        errs = vr.validate_docs(docs)
        self.assertTrue(any("PGSSLKEY must never be set" in e for e in errs), errs)

    def test_harmless_extra_env_passes(self) -> None:
        docs = make_docs(extra_env=[{"name": "LOOM_EXAMPLE_FLAG", "value": "1"}])
        self.assertEqual(vr.validate_docs(docs), [])


class CaMountChecks(unittest.TestCase):
    def test_ca_only_mount_passes(self) -> None:
        docs = make_docs(
            extra_env=[ca_env()],
            mounts=[ca_mount()],
            volumes=[ca_volume([{"key": "ca.crt", "path": "ca.crt"}])],
        )
        self.assertEqual(vr.validate_docs(docs), [])

    def test_non_readonly_mount_fails(self) -> None:
        docs = make_docs(
            extra_env=[ca_env()],
            mounts=[ca_mount(read_only=False)],
            volumes=[ca_volume([{"key": "ca.crt", "path": "ca.crt"}])],
        )
        errs = vr.validate_docs(docs)
        self.assertTrue(any("readOnly: true" in e for e in errs), errs)

    def test_two_items_fails(self) -> None:
        docs = make_docs(
            extra_env=[ca_env()],
            mounts=[ca_mount()],
            volumes=[ca_volume([{"key": "ca.crt", "path": "ca.crt"}, {"key": "tls.crt", "path": "tls.crt"}])],
        )
        errs = vr.validate_docs(docs)
        self.assertTrue(any("exactly one item" in e for e in errs), errs)

    def test_private_key_item_fails(self) -> None:
        docs = make_docs(
            extra_env=[ca_env()],
            mounts=[ca_mount()],
            volumes=[ca_volume([{"key": "tls.key", "path": "ca.crt"}])],
        )
        errs = vr.validate_docs(docs)
        self.assertTrue(any("looks like a private key" in e for e in errs), errs)
        self.assertTrue(any("private key material" in e for e in errs), errs)

    def test_whole_secret_projection_fails(self) -> None:
        docs = make_docs(
            extra_env=[ca_env()],
            mounts=[ca_mount()],
            volumes=[{"name": "db-ca", "secret": {"secretName": "db-ca"}}],
        )
        errs = vr.validate_docs(docs)
        self.assertTrue(any("exactly one item" in e for e in errs), errs)

    def test_path_mismatch_fails(self) -> None:
        docs = make_docs(
            extra_env=[ca_env()],
            mounts=[ca_mount()],
            volumes=[ca_volume([{"key": "ca.crt", "path": "root.pem"}])],
        )
        errs = vr.validate_docs(docs)
        self.assertTrue(any("does not match PGSSLROOTCERT" in e for e in errs), errs)

    def test_env_without_mount_fails(self) -> None:
        docs = make_docs(extra_env=[ca_env()])
        errs = vr.validate_docs(docs)
        self.assertTrue(any("not inside any volumeMount" in e for e in errs), errs)

    def test_private_key_regex(self) -> None:
        for bad in ("tls.key", "ca.key", "key", "private.pem", "server-key.pem", "PRIVATE_KEY", "ca_key"):
            self.assertTrue(vr.looks_like_private_key(bad), bad)
        for good in ("ca.crt", "root.pem", "ca-bundle.pem", "tls.crt", "monkey.crt", "keychain.crt"):
            self.assertFalse(vr.looks_like_private_key(good), good)


class ImageChecks(unittest.TestCase):
    def test_tagged_image_passes(self) -> None:
        self.assertEqual(vr.validate_docs(make_docs(image="ghcr.io/pukekos/loom:v1.2.3")), [])

    def test_digest_image_passes(self) -> None:
        self.assertEqual(vr.validate_docs(make_docs(image=f"ghcr.io/pukekos/loom@{GOOD_DIGEST}")), [])

    def test_bad_digest_fails(self) -> None:
        for bad in ("sha256:" + "ab" * 31, "sha256:" + "AB" * 32, "sha512:" + "ab" * 32, "ab" * 32):
            errs = vr.validate_docs(make_docs(image=f"ghcr.io/pukekos/loom@{bad}"))
            self.assertTrue(any("not sha256:<64 hex>" in e for e in errs), (bad, errs))

    def test_untagged_image_fails(self) -> None:
        errs = vr.validate_docs(make_docs(image="ghcr.io/pukekos/loom"))
        self.assertTrue(any("must carry a tag" in e for e in errs), errs)
        errs = vr.validate_docs(make_docs(image="localhost:5000/loom"))
        self.assertTrue(any("must carry a tag" in e for e in errs), errs)


class StructureChecks(unittest.TestCase):
    def test_missing_kind_reported(self) -> None:
        docs = [d for d in make_docs() if d["kind"] != "Service"]
        self.assertIn("missing required kind Service", vr.validate_docs(docs))

    def test_wrong_probe_reported(self) -> None:
        docs = make_docs()
        vr.find_app_container(vr.find_server(docs))["readinessProbe"]["httpGet"]["path"] = "/healthz"
        errs = vr.validate_docs(docs)
        self.assertTrue(any("readinessProbe path" in e for e in errs), errs)


if __name__ == "__main__":
    unittest.main()
