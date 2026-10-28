#!/usr/bin/env python3
"""Inspect an exact locally built Lambda image; never push or contact AWS."""
import argparse
import json
import pathlib
import secrets
import subprocess
import tempfile
import time

HELPER = "node:22-alpine@sha256:fe4f6d74e20370acb04ad889077d14d0f64a9c8ea6a42bc60cddf1e844699914"


def command(*args, timeout=45, check=True):
    result = subprocess.run(["docker", *args], capture_output=True, text=True,
                            timeout=timeout)
    if check and result.returncode:
        # No environment or container logs are printed on failure.
        raise RuntimeError("Owned image qualification command failed: " + args[0])
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image")
    parser.add_argument("--expected-arch", choices=["arm64", "amd64"], required=True)
    options = parser.parse_args()
    image = json.loads(command("image", "inspect", options.image).stdout)[0]
    config = image["Config"]
    assert image["Os"] == "linux" and image["Architecture"] == options.expected_arch
    assert config["Entrypoint"] == ["/var/runtime/bootstrap"]
    assert not config.get("ExposedPorts") and not config.get("Healthcheck")
    hardening = ["--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
                 "--tmpfs", "/tmp:rw,noexec,nosuid,size=16m", "--memory", "256m"]
    assert command("run", "--rm", "--network", "none", *hardening,
                   "--entrypoint", "/usr/bin/id", options.image, "-u").stdout.strip() == "10001"
    command("run", "--rm", "--network", "none", *hardening, "--entrypoint", "/bin/sh",
            options.image, "-c", "test -s /etc/ssl/certs/ca-certificates.crt && test -x /var/runtime/bootstrap && ! touch /root-must-stay-read-only")
    invalid = command("run", "--rm", "--network", "none", *hardening, options.image, check=False)
    assert invalid.returncode == 1
    assert "Private Lambda configuration is invalid" in invalid.stderr

    suffix = secrets.token_hex(8)
    network, peer, runtime = ("loom-lambda-check-" + suffix + tail for tail in ["-net", "-peer", "-runtime"])
    owned_containers = []
    network_created = False
    values = [secrets.token_hex(32), secrets.token_hex(32)]
    try:
        command("network", "create", "--internal", network)
        network_created = True
        source = pathlib.Path(__file__).with_name("lambda_runtime_peer.mjs").resolve()
        owned_containers.append(peer)
        command("run", "-d", "--name", peer, "--network", network, "--network-alias", "runtime-api",
                *hardening, "--user", "10001:10001", "--mount", f"type=bind,src={source},dst=/fixture/peer.mjs,readonly",
                HELPER, "node", "/fixture/peer.mjs", timeout=120)
        with tempfile.TemporaryDirectory(prefix="loom-lambda-env-") as directory:
            env = pathlib.Path(directory) / "runtime.env"
            env.touch(mode=0o600)
            env.write_text("\n".join([
                "AWS_LAMBDA_RUNTIME_API=runtime-api:8080", "AWS_REGION=ap-southeast-2",
                "AWS_LAMBDA_FUNCTION_NAME=synthetic", "AWS_LAMBDA_FUNCTION_MEMORY_SIZE=256",
                "AWS_LAMBDA_FUNCTION_VERSION=$LATEST", "LOOM_REQUEST_DRIVEN=true",
                "DATABASE_URL=postgres://synthetic:synthetic@runtime-api:5432/synthetic",
                "LOOM_ROOT_ADMIN_TOKEN=" + values[0], "LOOM_ENCRYPTION_KEY=" + values[1],
                "LOOM_LAMBDA_ACCOUNT_ID=123456789012",
                'LOOM_LAMBDA_CALLER_ROLE_ARNS=["arn:aws:iam::123456789012:role/tandem-test"]',
            ]) + "\n")
            deadline = time.monotonic() + 20
            while "ready" not in command("logs", peer).stdout:
                assert time.monotonic() < deadline, "Runtime peer did not start"
                time.sleep(0.1)
            owned_containers.append(runtime)
            command("run", "-d", "--name", runtime, "--network", network, *hardening,
                    "--env-file", str(env), options.image)
            evidence = None
            deadline = time.monotonic() + 25
            while time.monotonic() < deadline:
                for line in command("logs", peer).stdout.splitlines():
                    if line.startswith("{"):
                        evidence = json.loads(line)
                if evidence is not None:
                    break
                status = json.loads(command("inspect", runtime).stdout)[0]["State"]
                assert status["Running"], "Lambda runtime exited before returning events"
                time.sleep(0.1)
            assert evidence and evidence["passed"] and evidence["responses"] == 2
            assert evidence["database_connections"] == 0
            for name in owned_containers:
                logs = command("logs", name)
                assert all(value not in logs.stdout + logs.stderr for value in values)
            assert json.loads(command("inspect", runtime).stdout)[0]["State"]["Running"]
            print(json.dumps({"passed": True, "image_id": image["Id"], "architecture": image["Architecture"],
                              "os": image["Os"], "uid": 10001, "read_only": True,
                              "internal_network": True, "ca_bundle": True,
                              "runtime_api_responses": 2, "statuses": evidence["statuses"],
                              "database_connections": 0, "live_ai": False, "aws_calls": False}))
    finally:
        for name in reversed(owned_containers):
            command("rm", "-f", "--volumes", name, check=False)
        if network_created:
            command("network", "rm", network, check=False)


if __name__ == "__main__":
    main()
