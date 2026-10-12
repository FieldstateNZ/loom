#!/usr/bin/env bash
# Start an ephemeral PostgreSQL server from an arbitrary PostgreSQL image using
# an explicit initdb + postgres path (deploy/ci/pg_bootstrap.sh), wait for it,
# and print provenance (image digest, server version, OS) in Markdown.
#
#   PG_BOOTSTRAP_PASSWORD=... deploy/ci/ephemeral_postgres.sh <image-ref> <container-name> <host-port>
#
# Pulls anonymously — no registry login is performed or required. Intended for
# CI compatibility checks against synthetic data only.
set -euo pipefail

image="${1:?image ref}"
name="${2:?container name}"
port="${3:?host port}"
: "${PG_BOOTSTRAP_PASSWORD:?PG_BOOTSTRAP_PASSWORD is required}"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

docker pull --quiet "$image"

docker run -d --name "$name" \
  --user postgres \
  --entrypoint /bin/bash \
  -e PG_BOOTSTRAP_PASSWORD \
  -p "127.0.0.1:${port}:5432" \
  -v "$here/pg_bootstrap.sh:/pg_bootstrap.sh:ro" \
  "$image" /pg_bootstrap.sh >/dev/null

# Deliberately expand these expressions in the container, not on the runner.
# shellcheck disable=SC2016
pgbin='PATH="$PATH:$(ls -d /usr/lib/postgresql/*/bin 2>/dev/null | tail -n1)"'
ready=0
for _ in $(seq 1 60); do
  if docker exec "$name" bash -c "$pgbin; pg_isready -h /tmp -U postgres" >/dev/null 2>&1; then
    ready=1; break
  fi
  if [ "$(docker inspect -f '{{.State.Running}}' "$name")" != "true" ]; then
    break
  fi
  sleep 1
done
if [ "$ready" -ne 1 ]; then
  echo "PostgreSQL did not become ready; container logs:" >&2
  docker logs "$name" >&2 || true
  exit 1
fi

version="$(docker exec "$name" bash -c "$pgbin; psql -h /tmp -U postgres -Atc 'select version()'")"
version_num="$(docker exec "$name" bash -c "$pgbin; psql -h /tmp -U postgres -Atc 'show server_version_num'")"
os_release="$(docker exec "$name" bash -c "grep -E '^PRETTY_NAME=' /etc/os-release" || true)"

{
  echo "## Ephemeral PostgreSQL: \`$image\`"
  echo
  echo '```'
  echo "image ref:        $image"
  docker image inspect "$image" --format 'image id:         {{.Id}}
repo digests:     {{json .RepoDigests}}'
  echo "server version:   $version"
  echo "server_version_num: $version_num"
  echo "container os:     $os_release"
  echo "bootstrap:        explicit initdb + postgres (deploy/ci/pg_bootstrap.sh), image entrypoint NOT used"
  echo '```'
  echo
  echo "Container bootstrap log:"
  echo
  echo '```'
  docker logs "$name" 2>&1 | head -n 40
  echo '```'
} | tee -a "${GITHUB_STEP_SUMMARY:-/dev/null}"

echo "ready on 127.0.0.1:${port}"
