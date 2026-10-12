#!/usr/bin/env python3
"""Semantic render tests for the Loom Helm chart.

Renders the chart with `helm template` (binary from $HELM, default "helm") and
asserts the *meaning* of the output: where DATABASE_URL comes from, that it is
rendered exactly once, how the CA-only mount looks, how image.digest resolves,
and that invalid inputs fail the render with a clear message. Every successful
render is also passed through validate_render.validate_docs().

    HELM=/path/to/helm python3 -m unittest deploy/ci/test_chart.py -v
"""
import os
import subprocess
import sys
import unittest

import yaml

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import validate_render as vr  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
CHART = os.path.normpath(os.path.join(HERE, "..", "chart", "loom"))
HELM = os.environ.get("HELM", "helm")
STAGING = os.path.join(CHART, "values-staging.yaml")
PRODUCTION = os.path.join(CHART, "values-production.yaml")
GOOD_DIGEST = "sha256:" + "ab" * 32
CA_FILE = "/etc/loom/db-ca/ca.crt"


def helm_template(*args: str) -> subprocess.CompletedProcess:
    cmd = [HELM, "template", "loom", CHART, "--namespace", "loom", *args]
    return subprocess.run(cmd, capture_output=True, text=True, check=False)


def render(*args: str) -> list[dict]:
    """Render, assert success, run the generic checker, return parsed docs."""
    proc = helm_template(*args)
    if proc.returncode != 0:
        raise AssertionError(f"helm template failed:\n{proc.stderr}")
    docs = [d for d in yaml.safe_load_all(proc.stdout) if d]
    errs = vr.validate_docs(docs)
    if errs:
        raise AssertionError(f"validate_render rejected a successful render: {errs}")
    return docs


def render_fails(*args: str) -> str:
    proc = helm_template(*args)
    if proc.returncode == 0:
        raise AssertionError(f"expected helm template to FAIL for {args!r}, but it rendered")
    return proc.stderr


def app_of(docs: list[dict]) -> dict:
    return vr.find_app_container(vr.find_server(docs))


def env_of(docs: list[dict]) -> dict[str, dict]:
    env = app_of(docs).get("env", [])
    return {e["name"]: e for e in env}


def env_count(docs: list[dict], name: str) -> int:
    return sum(1 for e in app_of(docs).get("env", []) if e["name"] == name)


def secret_ref(env: dict[str, dict], name: str) -> tuple[str, str]:
    ref = env[name]["valueFrom"]["secretKeyRef"]
    return ref["name"], ref["key"]


def volumes_of(docs: list[dict]) -> dict[str, dict]:
    return {v["name"]: v for v in vr.find_server(docs)["spec"]["template"]["spec"]["volumes"]}


def mounts_of(docs: list[dict]) -> dict[str, dict]:
    return {m["name"]: m for m in app_of(docs)["volumeMounts"]}


class DatabaseUrlSource(unittest.TestCase):
    def test_default_values_use_legacy_app_secret(self) -> None:
        docs = render()
        env = env_of(docs)
        self.assertEqual(env_count(docs, "DATABASE_URL"), 1)
        self.assertEqual(secret_ref(env, "DATABASE_URL"), ("loom-secrets", "DATABASE_URL"))
        self.assertNotIn("PGSSLROOTCERT", env)
        self.assertNotIn("db-ca", volumes_of(docs))
        self.assertNotIn("db-ca", mounts_of(docs))
        self.assertEqual(app_of(docs)["image"], "ghcr.io/pukekos/loom:0.1.0")

    def test_staging_bundled_db_wins(self) -> None:
        docs = render("-f", STAGING, "--set", "postgresql.auth.password=ci-fixed-password")
        env = env_of(docs)
        self.assertEqual(env_count(docs, "DATABASE_URL"), 1)
        self.assertEqual(secret_ref(env, "DATABASE_URL"), ("loom-postgresql", "DATABASE_URL"))
        # The bundled DB Secret is rendered alongside, and carries the URL.
        secrets = [d for d in docs if d["kind"] == "Secret" and d["metadata"]["name"] == "loom-postgresql"]
        self.assertEqual(len(secrets), 1)
        self.assertIn("DATABASE_URL", secrets[0]["stringData"])

    def test_production_profile_is_unchanged(self) -> None:
        docs = render("-f", PRODUCTION, "--set", "image.tag=v0.0.0-ci")
        env = env_of(docs)
        self.assertEqual(secret_ref(env, "DATABASE_URL"), ("loom-secrets", "DATABASE_URL"))
        self.assertEqual(secret_ref(env, "LOOM_ENCRYPTION_KEY"), ("loom-secrets", "LOOM_ENCRYPTION_KEY"))
        self.assertNotIn("PGSSLROOTCERT", env)
        self.assertEqual(set(mounts_of(docs)), {"tmp"})
        self.assertEqual(set(volumes_of(docs)), {"tmp"})
        self.assertEqual(app_of(docs)["image"], "ghcr.io/pukekos/loom:v0.0.0-ci")

    def test_external_database_secret(self) -> None:
        docs = render("--set", "database.existingSecret=cnpg-app", "--set", "database.urlKey=uri")
        env = env_of(docs)
        self.assertEqual(env_count(docs, "DATABASE_URL"), 1)
        self.assertEqual(secret_ref(env, "DATABASE_URL"), ("cnpg-app", "uri"))
        # The app secrets still come from the legacy app Secret.
        self.assertEqual(secret_ref(env, "LOOM_ENCRYPTION_KEY"), ("loom-secrets", "LOOM_ENCRYPTION_KEY"))
        self.assertEqual(secret_ref(env, "LOOM_ROOT_ADMIN_TOKEN"), ("loom-secrets", "LOOM_ROOT_ADMIN_TOKEN"))

    def test_external_database_secret_default_key(self) -> None:
        docs = render("--set", "database.existingSecret=db-url")
        self.assertEqual(secret_ref(env_of(docs), "DATABASE_URL"), ("db-url", "DATABASE_URL"))

    def test_external_secret_with_bundled_db_conflicts(self) -> None:
        err = render_fails("-f", STAGING, "--set", "database.existingSecret=cnpg-app")
        self.assertIn("database.existingSecret", err)
        self.assertIn("postgresql.enabled", err)

    def test_external_secret_with_empty_key_fails(self) -> None:
        err = render_fails("--set", "database.existingSecret=cnpg-app", "--set", "database.urlKey=")
        self.assertIn("database.urlKey", err)


class ExtraEnvGuard(unittest.TestCase):
    def test_extra_env_cannot_override_database_url(self) -> None:
        err = render_fails("--set", "extraEnv[0].name=DATABASE_URL", "--set", "extraEnv[0].value=postgres://x")
        self.assertIn("DATABASE_URL", err)
        self.assertIn("extraEnv", err)

    def test_extra_env_cannot_override_secrets_or_ca(self) -> None:
        for name in ("LOOM_ENCRYPTION_KEY", "LOOM_ROOT_ADMIN_TOKEN", "PGSSLROOTCERT", "PGSSLKEY", "RUST_LOG"):
            err = render_fails("--set", f"extraEnv[0].name={name}", "--set", "extraEnv[0].value=x")
            self.assertIn(name, err)

    def test_extra_env_rust_log_allowed_when_chart_does_not_render_it(self) -> None:
        docs = render("--set", "config.rustLog=", "--set", "extraEnv[0].name=RUST_LOG", "--set", "extraEnv[0].value=debug")
        self.assertEqual(env_count(docs, "RUST_LOG"), 1)
        self.assertEqual(env_of(docs)["RUST_LOG"]["value"], "debug")

    def test_extra_env_duplicate_names_fail(self) -> None:
        err = render_fails(
            "--set", "extraEnv[0].name=LOOM_EXAMPLE_FLAG", "--set", "extraEnv[0].value=1",
            "--set", "extraEnv[1].name=LOOM_EXAMPLE_FLAG", "--set", "extraEnv[1].value=2",
        )
        self.assertIn("more than once", err)

    def test_harmless_extra_env_renders_once(self) -> None:
        docs = render("--set", "extraEnv[0].name=LOOM_EXAMPLE_FLAG", "--set", "extraEnv[0].value=1")
        self.assertEqual(env_count(docs, "LOOM_EXAMPLE_FLAG"), 1)
        self.assertEqual(env_count(docs, "DATABASE_URL"), 1)


class ImageDigest(unittest.TestCase):
    def test_digest_pins_image_and_ignores_tag(self) -> None:
        docs = render("--set", f"image.digest={GOOD_DIGEST}", "--set", "image.tag=v9.9.9")
        self.assertEqual(app_of(docs)["image"], f"ghcr.io/pukekos/loom@{GOOD_DIGEST}")

    def test_tag_still_wins_over_app_version(self) -> None:
        docs = render("--set", "image.tag=v1.2.3")
        self.assertEqual(app_of(docs)["image"], "ghcr.io/pukekos/loom:v1.2.3")

    def test_invalid_digests_fail(self) -> None:
        for bad in ("ab" * 32, "sha256:" + "ab" * 31, "sha256:" + "AB" * 32, "sha512:" + "ab" * 32):
            err = render_fails("--set", f"image.digest={bad}")
            self.assertIn("image.digest", err, bad)

    def test_repository_with_tag_or_digest_fails(self) -> None:
        self.assertIn("image.repository", render_fails("--set", "image.repository=ghcr.io/pukekos/loom:latest"))
        self.assertIn("image.repository", render_fails("--set", f"image.repository=ghcr.io/pukekos/loom@{GOOD_DIGEST}"))

    def test_registry_port_in_repository_is_allowed(self) -> None:
        docs = render("--set", "image.repository=registry.local:5000/loom", "--set", "image.tag=v1")
        self.assertEqual(app_of(docs)["image"], "registry.local:5000/loom:v1")


class DatabaseCa(unittest.TestCase):
    def test_ca_only_mount(self) -> None:
        docs = render("--set", "database.tls.caSecret=cnpg-ca")
        env = env_of(docs)
        self.assertEqual(env["PGSSLROOTCERT"], {"name": "PGSSLROOTCERT", "value": CA_FILE})
        self.assertEqual(env_count(docs, "PGSSLROOTCERT"), 1)
        self.assertNotIn("PGSSLKEY", env)
        self.assertNotIn("PGSSLCERT", env)
        mount = mounts_of(docs)["db-ca"]
        self.assertEqual(mount["mountPath"], "/etc/loom/db-ca")
        self.assertIs(mount["readOnly"], True)
        vol = volumes_of(docs)["db-ca"]
        self.assertEqual(vol["secret"]["secretName"], "cnpg-ca")
        self.assertEqual(vol["secret"]["items"], [{"key": "ca.crt", "path": "ca.crt"}])
        self.assertEqual(vol["secret"]["defaultMode"], 0o444)

    def test_custom_ca_key(self) -> None:
        docs = render("--set", "database.tls.caSecret=cnpg-ca", "--set", "database.tls.caKey=root.pem")
        vol = volumes_of(docs)["db-ca"]
        self.assertEqual(vol["secret"]["items"], [{"key": "root.pem", "path": "ca.crt"}])
        self.assertEqual(env_of(docs)["PGSSLROOTCERT"]["value"], CA_FILE)

    def test_private_key_names_fail(self) -> None:
        for bad in ("tls.key", "ca.key", "server-key.pem", "private.pem"):
            err = render_fails("--set", "database.tls.caSecret=cnpg-ca", "--set", f"database.tls.caKey={bad}")
            self.assertIn("private key", err, bad)

    def test_empty_ca_key_fails(self) -> None:
        err = render_fails("--set", "database.tls.caSecret=cnpg-ca", "--set", "database.tls.caKey=")
        self.assertIn("database.tls.caKey", err)

    def test_ca_combined_with_external_secret(self) -> None:
        docs = render(
            "--set", "database.existingSecret=cnpg-app", "--set", "database.urlKey=uri",
            "--set", "database.tls.caSecret=cnpg-ca",
        )
        env = env_of(docs)
        self.assertEqual(secret_ref(env, "DATABASE_URL"), ("cnpg-app", "uri"))
        self.assertEqual(env["PGSSLROOTCERT"]["value"], CA_FILE)


if __name__ == "__main__":
    unittest.main()
