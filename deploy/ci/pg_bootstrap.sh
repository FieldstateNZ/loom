#!/usr/bin/env bash
# Runs INSIDE a PostgreSQL container as the `postgres` user. Initialises a
# throwaway cluster under /tmp and starts the server in the foreground.
#
# This deliberately does not rely on the image's own entrypoint/bootstrap
# (the official `postgres` image's docker-entrypoint.sh, or CloudNativePG's
# operator-driven bootstrap): it only needs `initdb` and `postgres` binaries,
# so the same path works for the official engine image and for CNPG images.
# The CNPG 18 image has no ENTRYPOINT at all (CMD bash, USER 26 = postgres,
# /usr/lib/postgresql/18/bin on PATH), so the official image's
# POSTGRES_PASSWORD-style bootstrap would simply not happen there.
#
# Required env: PG_BOOTSTRAP_PASSWORD (the superuser password; synthetic, CI only).
set -euo pipefail

: "${PG_BOOTSTRAP_PASSWORD:?PG_BOOTSTRAP_PASSWORD is required}"

if ! command -v initdb >/dev/null 2>&1; then
  for d in /usr/lib/postgresql/*/bin; do
    [ -x "$d/initdb" ] && PATH="$d:$PATH"
  done
  export PATH
fi
command -v initdb >/dev/null || { echo "initdb not found in image" >&2; exit 1; }

echo "running as uid=$(id -u) ($(id -un 2>/dev/null || echo '?'))"
echo "os-release: $(grep -E '^PRETTY_NAME=' /etc/os-release || true)"
initdb --version
postgres --version

PGDATA="$(mktemp -d /tmp/loom-pgdata.XXXXXX)"
pwfile="$(mktemp /tmp/loom-pgpw.XXXXXX)"
printf '%s\n' "$PG_BOOTSTRAP_PASSWORD" > "$pwfile"

initdb -D "$PGDATA" -U postgres \
  --auth-local=trust --auth-host=scram-sha-256 --pwfile="$pwfile" \
  --encoding=UTF8 --locale=C.UTF-8
rm -f "$pwfile"

{
  echo "host all all 0.0.0.0/0 scram-sha-256"
  echo "host all all ::/0 scram-sha-256"
} >> "$PGDATA/pg_hba.conf"

# Ephemeral: durability off for speed, sockets under /tmp so no system
# directory needs to be writable.
exec postgres -D "$PGDATA" \
  -c listen_addresses='*' \
  -c unix_socket_directories=/tmp \
  -c max_connections=300 \
  -c fsync=off \
  -c synchronous_commit=off
