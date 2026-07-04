# Deploying Loom

Loom ships as a single **stateless** container (`loom-server`, built by the
repo's [`Dockerfile`](../Dockerfile)) that fronts an **external PostgreSQL**
database. This directory contains a Helm chart plus ready-made value profiles
for two targets:

| Target | Profile | Database |
| ------ | ------- | -------- |
| **k3s staging** (a modest ~16 GB home server) | [`chart/loom/values-staging.yaml`](chart/loom/values-staging.yaml) | Bundled dev PostgreSQL (in-cluster, single pod) |
| **DOKS production** (DigitalOcean Kubernetes) | [`chart/loom/values-production.yaml`](chart/loom/values-production.yaml) | External **managed** PostgreSQL |

The chart lives at [`chart/loom`](chart/loom). Its full configuration surface is
documented inline in [`chart/loom/values.yaml`](chart/loom/values.yaml).

---

## Contents

- [Concepts you must know first](#concepts-you-must-know-first)
- [Create the Secret](#create-the-secret)
- [Staging: k3s (single node)](#staging-k3s-single-node)
- [Production: DOKS + managed PostgreSQL](#production-doks--managed-postgresql)
- [SSE / streaming ingress notes](#sse--streaming-ingress-notes)
- [Migrations](#migrations)
- [Scaling and the per-replica limiter caveat](#scaling-and-the-per-replica-limiter-caveat)
- [Verifying a deployment](#verifying-a-deployment)

---

## Concepts you must know first

**Secrets are never in values files.** The chart reads all sensitive material
from an existing Kubernetes `Secret` that *you* create out-of-band and reference
by name (`secrets.existingSecret`). The keys the server needs:

| Env var | Purpose | Required |
| ------- | ------- | -------- |
| `DATABASE_URL` | PostgreSQL connection URL | Yes (unless the bundled DB is enabled, which supplies it) |
| `LOOM_ENCRYPTION_KEY` | AES-256-GCM key for stored credentials — **64 hex chars** | Yes |
| `LOOM_ROOT_ADMIN_TOKEN` | Bearer token guarding `/admin` | Yes |
| `LOOM_KEY_PEPPER` | Virtual-key HMAC pepper | Optional (derived from the encryption key when unset) |

> **The encryption key is durable state.** Everything encrypted at rest (tenant
> provider credentials) is tied to it. Rotating or losing it makes stored
> secrets unrecoverable. Generate it once and keep it safe:
>
> ```sh
> openssl rand -hex 32          # LOOM_ENCRYPTION_KEY (32 bytes -> 64 hex chars)
> openssl rand -base64 24       # LOOM_ROOT_ADMIN_TOKEN
> ```

**Migrations run on boot.** `config.runMigrations: true` (the default) makes the
server apply schema migrations itself at startup — no migration `Job` or
init-container. A `startupProbe` gives that first boot room before liveness
kicks in. See [Migrations](#migrations).

**Health endpoints.** Liveness → `/healthz` (process up), readiness → `/readyz`
(PostgreSQL reachable). Both are wired by the chart.

---

## Create the Secret

Pick a namespace (examples use `loom`) and create it, then the Secret.

**Staging (bundled DB)** — only the app secrets are needed; `DATABASE_URL` comes
from the in-cluster database:

```sh
kubectl create namespace loom

kubectl -n loom create secret generic loom-secrets \
  --from-literal=LOOM_ENCRYPTION_KEY="$(openssl rand -hex 32)" \
  --from-literal=LOOM_ROOT_ADMIN_TOKEN="$(openssl rand -base64 24)"
```

**Production (external managed DB)** — include `DATABASE_URL`:

```sh
kubectl create namespace loom

kubectl -n loom create secret generic loom-secrets \
  --from-literal=DATABASE_URL='postgres://doadmin:PASS@db-host:25060/loom?sslmode=require' \
  --from-literal=LOOM_ENCRYPTION_KEY="$(openssl rand -hex 32)" \
  --from-literal=LOOM_ROOT_ADMIN_TOKEN="$(openssl rand -base64 24)"
```

> Prefer sealed-secrets / external-secrets / SOPS in real environments; the
> chart only needs the resulting `Secret` to exist by the name you configure.

---

## Staging: k3s (single node)

Assumes a running [k3s](https://k3s.io) node. k3s bundles Traefik as the default
ingress; the staging profile's example annotations target **ingress-nginx** — if
you keep Traefik, adjust the ingress section (see
[SSE notes](#sse--streaming-ingress-notes)).

1. Create the Secret (bundled-DB variant above).
2. Install the chart with the staging profile:

   ```sh
   helm upgrade --install loom deploy/chart/loom \
     -n loom \
     -f deploy/chart/loom/values-staging.yaml
   ```

   This enables the bundled single-pod PostgreSQL (`postgresql.enabled=true`),
   small resource envelopes (~96–256 Mi), and an example ingress at
   `loom.staging.local`. Point that hostname at the node (e.g. `/etc/hosts`) or
   edit it to your own.

3. [Verify](#verifying-a-deployment).

The bundled PostgreSQL is **dev/staging only**: one pod, one PVC, no HA, no
backups. It is perfect for a home server and unacceptable for production.

---

## Production: DOKS + managed PostgreSQL

1. **Provision a managed PostgreSQL** (DigitalOcean → Databases → PostgreSQL).
   Create a database named `loom`, and copy the connection URI (it already
   includes `?sslmode=require`). Restrict its trusted sources to your DOKS
   cluster.

2. **Create the Secret** (external-DB variant above) with `DATABASE_URL` set to
   that URI.

3. **Ensure an ingress controller + TLS.** Install `ingress-nginx` and
   `cert-manager` if you have not already:

   ```sh
   helm repo add ingress-nginx https://kubernetes.github.io/ingress-nginx
   helm upgrade --install ingress-nginx ingress-nginx/ingress-nginx \
     -n ingress-nginx --create-namespace
   # cert-manager per its docs; the production profile references a
   # cluster-issuer named "letsencrypt-prod".
   ```

4. **Install Loom** with the production profile, pinning an image tag:

   ```sh
   helm upgrade --install loom deploy/chart/loom \
     -n loom --create-namespace \
     -f deploy/chart/loom/values-production.yaml \
     --set image.tag=v0.1.0
   ```

   The production profile keeps `postgresql.enabled=false` (external DB),
   sources `DATABASE_URL` from your Secret, and configures an ingress at
   `loom.example.com` with the SSE-safe annotations and cert-manager TLS. Edit
   the host to your domain.

5. [Verify](#verifying-a-deployment).

Images are published to `ghcr.io/pukekos/loom` by the
[`release` workflow](../.github/workflows/release.yml) on every `v*` git tag.

---

## SSE / streaming ingress notes

Loom streams turn responses with **Server-Sent Events**. A proxy that buffers
responses or applies a short read timeout will stall tokens or drop long
streams. The chart's ingress defaults carry the correct **ingress-nginx**
annotations:

```yaml
nginx.ingress.kubernetes.io/proxy-buffering: "off"        # stream, don't buffer
nginx.ingress.kubernetes.io/proxy-request-buffering: "off"
nginx.ingress.kubernetes.io/proxy-read-timeout: "3600"    # allow long streams
nginx.ingress.kubernetes.io/proxy-send-timeout: "3600"
```

The two things that matter for any proxy:

1. **Response buffering OFF** — tokens must flush as they arrive.
2. **Long read/send timeout** — minutes, not the default seconds.

**Traefik (k3s default):** Traefik streams responses without buffering by
default, so the main risk is timeouts. Raise
`serversTransport.forwardingTimeouts.responseHeaderTimeout` (and idle timeouts
on the entrypoint) to a few minutes. Set `ingress.className: traefik` and drop
the nginx-specific annotations.

**Cloud L7 load balancers** (incl. the DO LB in front of ingress-nginx) also have
idle timeouts — raise them to match if you terminate long streams there.

---

## Migrations

Default: **on-boot.** With `config.runMigrations: true` the server runs
`sqlx` migrations at startup under an advisory lock, so concurrent replicas are
safe and idempotent. This is why a `startupProbe` (default 150 s budget) guards
the first boot — a fresh database's migration pass must finish before liveness
probing begins.

If you prefer to gate migrations (e.g. run them from a single controlled
rollout and keep application pods read-only against schema), set
`config.runMigrations: false` on the app release and run one migrating rollout
separately. There is no separate migration Job in the chart by design; the
on-boot path is the supported default and matches the container's behavior in
`docker-compose`.

---

## Scaling and the per-replica limiter caveat

`replicaCount` is configurable and horizontal scaling for **throughput** is
safe (the server is stateless; all durable state is in PostgreSQL).

**However — issue #10:** Loom's rate limiters and budget caches are
**in-process**, so they are enforced **per replica**. With `replicaCount: N`:

- The effective global rate limit is roughly `configured_limit × N`, because
  each replica only sees its share of traffic.
- Budget ceilings are ultimately reconciled against PostgreSQL, but the
  fast-path cache is local, so brief bursts can exceed a tenant's limit before
  reconciliation.

Until a shared/distributed limiter lands, **run a single replica when strict
per-tenant rate/budget enforcement matters.** Both staging and production
profiles default to `replicaCount: 1` for this reason. The chart's `NOTES.txt`
re-emits this warning whenever you install with more than one replica.

**The batch poll worker is safe under `replicas > 1`.** Each replica runs a
worker, but a batch job's `created → provider` submission is an **atomic claim**
(`UPDATE … WHERE status = 'created'`, flipping the job to `submitting`) — only
the replica that wins the claim submits, so a job is never submitted to the
provider twice (no duplicate provider batches, no double billing). A
cancellation that races an in-flight submission is likewise safe: the job is
moved to `canceling` rather than finalised locally, so a completing submission
cannot resurrect a canceled batch into a running one. The multi-replica caveat
above is therefore only about the in-process rate/budget limiter, **not** about
batch submission. (One residual edge: a replica that crashes in the narrow
window between provider-submit and recording the result leaves the job in
`submitting`; the worker never re-submits such a job — preserving exactly-once —
so it awaits operator reconciliation rather than risking a double submit.)

---

## Verifying a deployment

```sh
# Roll out and wait.
kubectl -n loom rollout status deploy/loom

# Port-forward and probe health.
kubectl -n loom port-forward svc/loom 8080:80
curl -s localhost:8080/healthz   # -> ok
curl -s localhost:8080/readyz    # -> {"status":"ready"}  (once PostgreSQL is reachable)

# Inspect what was rendered without applying (great for reviews / CI parity):
helm template loom deploy/chart/loom -f deploy/chart/loom/values-staging.yaml \
  --set postgresql.auth.password=example | less
```

`/readyz` returning non-200 means the server is up but cannot reach PostgreSQL —
check `DATABASE_URL`, the managed-DB trusted sources, and (staging) that the
bundled PostgreSQL pod is `Running`.

### Chart CI

The [`helm` CI job](../.github/workflows/ci.yml) runs `helm lint` and
`helm template` against the default, staging, and production profiles on every
push/PR, then validates the rendered manifests with
[`ci/validate_render.py`](ci/validate_render.py) (required kinds present, and —
critically — that `DATABASE_URL` / `LOOM_ENCRYPTION_KEY` / `LOOM_ROOT_ADMIN_TOKEN`
are always sourced via `secretKeyRef`, never rendered as literal values).
