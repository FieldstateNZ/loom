#!/usr/bin/env python3
"""Unit tests for validate_render.py — synthetic manifests, no helm needed.

    python3 -m unittest deploy/ci/test_validate_render.py -v
"""
import copy
import os
import subprocess
import sys
import tempfile
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


SENTINEL = "SENTINEL-s3cr3t-7f3a9c"


def run_cli(*paths: str) -> subprocess.CompletedProcess:
    script = os.path.join(os.path.dirname(os.path.abspath(__file__)), "validate_render.py")
    return subprocess.run([sys.executable, script, *paths], capture_output=True, text=True, check=False)


class DiagnosticsAreValueFree(unittest.TestCase):
    """Rendered Secret-derived content must never reach CI logs (stdout or stderr)."""

    def _run_text(self, text: str) -> subprocess.CompletedProcess:
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "render.yaml")
            with open(path, "w", encoding="utf-8") as fh:
                fh.write(text)
            return run_cli(path)

    def _assert_clean(self, proc: subprocess.CompletedProcess) -> None:
        self.assertNotIn(SENTINEL, proc.stdout)
        self.assertNotIn(SENTINEL, proc.stderr)
        self.assertNotIn("Traceback", proc.stderr)

    def test_malformed_yaml_fails_without_echoing_source(self) -> None:
        # Each document makes PyYAML quote document content in str(exception):
        # an undefined alias and an unknown tag name echo the sentinel, a bad
        # token echoes a character, and all carry file/line context.
        malformed = {
            "undefined alias": f"kind: Secret\ndata:\n  token: *{SENTINEL}\n",
            "unknown tag": f"kind: Secret\ndata:\n  token: !{SENTINEL} value\n",
            "bad token": f"kind: Secret\ndata:\n  token: {SENTINEL}\n\t- {SENTINEL}: : :\n",
            "unclosed flow": f"kind: Secret\ndata: [{SENTINEL}, {{a: {SENTINEL}\n",
        }
        for label, text in malformed.items():
            with self.subTest(label):
                proc = self._run_text(text)
                self.assertEqual(proc.returncode, 1)
                self.assertIn("invalid YAML", proc.stdout)
                self._assert_clean(proc)

    def test_malformed_yaml_message_has_no_parser_detail(self) -> None:
        for text in (f"a: *{SENTINEL}\n", f"a: !{SENTINEL} v\n", f"a: b\n\tkey: {SENTINEL}\n"):
            errs = vr.validate(self._write(text))
            self.assertEqual(len(errs), 1)
            self.assertNotIn(SENTINEL, errs[0])
            self.assertIn("invalid YAML", errs[0])

    def _write(self, text: str) -> str:
        fd, path = tempfile.mkstemp(suffix=".yaml")
        self.addCleanup(os.unlink, path)
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.write(text)
        return path

    def test_every_check_failure_omits_document_values(self) -> None:
        docs = make_docs(
            env=[
                {"name": "DATABASE_URL", "value": SENTINEL},
                {"name": "DATABASE_URL", "value": SENTINEL},
                {"name": f"X_{SENTINEL}", "value": SENTINEL},
                {"name": f"X_{SENTINEL}", "value": SENTINEL},
                {"name": "PGSSLKEY", "value": SENTINEL},
                {"name": "PGSSLROOTCERT", "value": f"/{SENTINEL}/ca.crt"},
            ],
            image=f"{SENTINEL}/loom@sha256:{SENTINEL}",
            mounts=[{"name": f"m-{SENTINEL}", "mountPath": f"/{SENTINEL}"}],
            volumes=[{"name": f"m-{SENTINEL}", "secret": {"items": [
                {"key": f"{SENTINEL}.key", "path": SENTINEL},
                {"key": f"tls-{SENTINEL}.key", "path": SENTINEL},
            ]}}],
        )
        server = vr.find_server(docs)
        app = vr.find_app_container(server)
        app["livenessProbe"]["httpGet"]["path"] = SENTINEL
        app["readinessProbe"]["httpGet"]["path"] = SENTINEL
        errs = vr.validate_docs(docs)
        self.assertGreater(len(errs), 5, errs)
        for e in errs:
            self.assertNotIn(SENTINEL, e)

    def test_ca_checks_omit_sentinels(self) -> None:
        for items, path in (
            ([{"key": f"{SENTINEL}.key", "path": SENTINEL}], f"/etc/loom/db-ca/{SENTINEL}"),
            ([{"key": SENTINEL, "path": SENTINEL}] * 2, "/etc/loom/db-ca/ca.crt"),
        ):
            docs = make_docs(
                extra_env=[{"name": "PGSSLROOTCERT", "value": path}],
                mounts=[ca_mount(read_only=False)],
                volumes=[ca_volume(items)],
            )
            errs = vr.validate_docs(docs)
            self.assertTrue(errs)
            self.assertFalse([e for e in errs if SENTINEL in e], errs)
        errs = vr.validate_docs(make_docs(extra_env=[{"name": "PGSSLROOTCERT", "value": f"relative-{SENTINEL}"}]))
        self.assertEqual(errs, ["PGSSLROOTCERT must be a literal absolute file path"])

    def test_unexpected_structure_fails_closed_without_traceback(self) -> None:
        proc = self._run_text(f"- {SENTINEL}\n- kind: Deployment\n  metadata: {{name: {SENTINEL}}}\n")
        self.assertEqual(proc.returncode, 1)
        self._assert_clean(proc)
        docs = make_docs()
        vr.find_server(docs)["spec"]["template"]["spec"]["containers"] = [{"name": SENTINEL, "env": [{"value": SENTINEL}]}]
        with tempfile.TemporaryDirectory() as tmp:
            import yaml
            path = os.path.join(tmp, "render.yaml")
            with open(path, "w", encoding="utf-8") as fh:
                yaml.safe_dump_all(docs, fh)
            proc = run_cli(path)
        self.assertEqual(proc.returncode, 1)
        self._assert_clean(proc)

    def test_explicit_constructor_tags_fail_closed_without_echo(self) -> None:
        # Constructors raise non-YAMLError exceptions (ValueError, AttributeError,
        # ...) whose text quotes the scalar, in original and lowercased form.
        for tag in ("int", "float", "timestamp", "bool", "set", "omap", "pairs"):
            for text in (
                f"a: !!{tag} {SENTINEL}\n",
                f"kind: Secret\ndata:\n  token: !!{tag} {SENTINEL}\n",
            ):
                with self.subTest(tag=tag, text=text):
                    proc = self._run_text(text)
                    self.assertEqual(proc.returncode, 1)
                    self.assertIn("RENDER VALIDATION FAILED", proc.stdout)
                    self._assert_clean(proc)
                    for stream in (proc.stdout, proc.stderr):
                        self.assertNotIn(SENTINEL.lower(), stream.lower())
                    errs = vr.validate(self._write(text))
                    self.assertEqual(len(errs), 1)
                    self.assertNotIn(SENTINEL.lower(), errs[0].lower())

    def test_unreadable_input_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "bad.yaml")
            with open(path, "wb") as fh:
                fh.write(b"token: \xff\xfe" + SENTINEL.encode())
            proc = run_cli(path)
        self.assertEqual(proc.returncode, 1)
        self._assert_clean(proc)

    def test_refusal_semantics_unchanged_for_good_and_bad_files(self) -> None:
        import yaml
        with tempfile.TemporaryDirectory() as tmp:
            good = os.path.join(tmp, "good.yaml")
            bad = os.path.join(tmp, "bad.yaml")
            with open(good, "w", encoding="utf-8") as fh:
                yaml.safe_dump_all(make_docs(), fh)
            with open(bad, "w", encoding="utf-8") as fh:
                yaml.safe_dump_all(make_docs(env=[{"name": "DATABASE_URL", "value": SENTINEL}]), fh)
            self.assertEqual(run_cli(good).returncode, 0)
            proc = run_cli(good, bad)
        self.assertEqual(proc.returncode, 1)
        self.assertIn("RENDER VALIDATION FAILED", proc.stdout)
        self._assert_clean(proc)


if __name__ == "__main__":
    unittest.main()
