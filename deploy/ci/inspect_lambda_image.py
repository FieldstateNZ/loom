#!/usr/bin/env python3
"""Inspect an exact locally built Lambda image; never push or contact AWS."""
import argparse
import json
import hashlib
import os
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
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--bootstrap", action="store_true")
    modes.add_argument("--bootstrap-reject", action="store_true")
    options = parser.parse_args()
    encrypted = options.bootstrap or options.bootstrap_reject
    reject = options.bootstrap_reject
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
        with tempfile.TemporaryDirectory(prefix="loom-lambda-env-") as directory:
            data = pathlib.Path(directory)
            extra = []
            peer_args = []
            secrets_env = ["DATABASE_URL=postgres://synthetic:synthetic@runtime-api:5432/synthetic",
                           "LOOM_ROOT_ADMIN_TOKEN=" + values[0], "LOOM_ENCRYPTION_KEY=" + values[1]]
            if encrypted:
                data.chmod(0o755)
                def openssl(*args):
                    subprocess.run(["openssl", *map(str,args)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                openssl("req","-x509","-newkey","rsa:2048","-nodes","-keyout",data/"ca.key","-out",data/"ca.crt","-days","1","-subj","/CN=Synthetic Loom CA","-addext","basicConstraints=critical,CA:TRUE")
                openssl("req","-new","-newkey","rsa:2048","-nodes","-keyout",data/"ssm.key","-out",data/"ssm.csr","-subj","/CN=ssm.ap-southeast-2.api.aws")
                (data/"extensions").write_text("subjectAltName=DNS:ssm.ap-southeast-2.api.aws\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n")
                openssl("x509","-req","-in",data/"ssm.csr","-CA",data/"ca.crt","-CAkey",data/"ca.key","-CAcreateserial","-out",data/"ssm.crt","-days","1","-extfile",data/"extensions")
                pem=(data/"ca.crt").read_text()
                document={"schema_version":1,"kind":"loom-private-lambda","database_url":"postgresql://synthetic:synthetic@runtime-api:5432/synthetic",
                          "database_ca_pem":pem,"database_ca_sha256":hashlib.sha256(pem.encode()).hexdigest(),"root_admin_token":values[0],"encryption_key":values[1],"key_pepper":{"mode":"derived"}}
                parameter=data/"parameter.json";parameter.touch(mode=0o600)
                parameter.write_text(json.dumps({"Parameter":{"ARN":"arn:aws:ssm:ap-southeast-2:123456789012:parameter/tandem/fixture/loom","Name":"/tandem/fixture/loom","Type":"SecureString","Version":8 if reject else 7,"Selector":":7","Value":json.dumps(document)}}))
                secrets_env=["LOOM_BOOTSTRAP_PARAMETER_ARN=arn:aws:ssm:ap-southeast-2:123456789012:parameter/tandem/fixture/loom","LOOM_BOOTSTRAP_PARAMETER_VERSION=7",
                             "AWS_ACCESS_KEY_ID=ASIASYNTHETICEXAMPLE","AWS_SECRET_ACCESS_KEY=synthetic-bootstrap-signing-key","AWS_SESSION_TOKEN=synthetic-bootstrap-session","SSL_CERT_FILE=/fixture/ca.crt"]
                extra=["--mount",f"type=bind,src={data/'ca.crt'},dst=/fixture/ca.crt,readonly"]
                peer_args=["--bootstrap-reject" if reject else "--bootstrap"]
            owned_containers.append(peer)
            command("run","-d","--name",peer,"--network",network,"--network-alias","runtime-api",
                    *(["--network-alias","ssm.ap-southeast-2.api.aws"] if encrypted else []),
                    *hardening,"--user",str(os.getuid()) if encrypted else "10001:10001","--mount",f"type=bind,src={source},dst=/fixture/peer.mjs,readonly",
                    *(["--mount",f"type=bind,src={directory},dst=/fixture-data,readonly"] if encrypted else []),
                    HELPER,"node","/fixture/peer.mjs",*peer_args,timeout=120)
            env = pathlib.Path(directory) / "runtime.env"
            env.touch(mode=0o600)
            env.write_text("\n".join([
                "AWS_LAMBDA_RUNTIME_API=runtime-api:8080", "AWS_REGION=ap-southeast-2",
                "AWS_LAMBDA_FUNCTION_NAME=synthetic", "AWS_LAMBDA_FUNCTION_MEMORY_SIZE=256",
                "AWS_LAMBDA_FUNCTION_VERSION=$LATEST", "LOOM_REQUEST_DRIVEN=true",
                *secrets_env,
                "LOOM_LAMBDA_ACCOUNT_ID=123456789012",
                'LOOM_LAMBDA_CALLER_ROLE_ARNS=["arn:aws:iam::123456789012:role/tandem-test"]',
            ]) + "\n")
            deadline = time.monotonic() + 20
            while "ready" not in command("logs", peer).stdout:
                assert time.monotonic() < deadline, "Runtime peer did not start"
                time.sleep(0.1)
            owned_containers.append(runtime)
            command("run", "-d", "--name", runtime, "--network", network, *hardening,
                    "--env-file", str(env), *extra, options.image)
            evidence = None
            deadline = time.monotonic() + 25
            while time.monotonic() < deadline:
                for line in command("logs", peer).stdout.splitlines():
                    if line.startswith("{"):
                        evidence = json.loads(line)
                if evidence is not None:
                    break
                status = json.loads(command("inspect", runtime).stdout)[0]["State"]
                if not reject:
                    assert status["Running"], "Lambda runtime exited before returning events"
                time.sleep(0.1)
            assert evidence and evidence["passed"] and evidence["responses"] == (0 if reject else 2)
            assert evidence["bootstrap_reads"] == (1 if encrypted else 0)
            if reject:
                deadline=time.monotonic()+8
                while json.loads(command("inspect",runtime).stdout)[0]["State"]["Running"]:
                    assert time.monotonic()<deadline,"Invalid bootstrap did not terminate"
                    time.sleep(0.1)
                assert json.loads(command("inspect",runtime).stdout)[0]["State"]["ExitCode"]==1
                assert "Private Lambda configuration is invalid" in command("logs",runtime).stderr
                assert json.loads(command("inspect",peer).stdout)[0]["State"]["Running"]
            assert evidence["database_connections"] == 0
            for name in owned_containers:
                logs = command("logs", name)
                assert all(value not in logs.stdout + logs.stderr for value in values)
            if not reject:
                assert json.loads(command("inspect", runtime).stdout)[0]["State"]["Running"]
            print(json.dumps({"passed": True, "image_id": image["Id"], "architecture": image["Architecture"],
                              "os": image["Os"], "uid": 10001, "read_only": True,
                              "internal_network": True, "ca_bundle": True,
                              "runtime_api_responses": evidence["responses"], "statuses": evidence["statuses"],
                              "encrypted_bootstrap": encrypted, "bootstrap_reads": evidence["bootstrap_reads"], "startup_rejected": reject,
                              "database_connections": 0, "live_ai": False, "aws_calls": False}))
    finally:
        for name in reversed(owned_containers):
            command("rm", "-f", "--volumes", name, check=False)
        if network_created:
            command("network", "rm", network, check=False)


if __name__ == "__main__":
    main()
