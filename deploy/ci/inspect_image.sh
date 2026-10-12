#!/usr/bin/env bash
# Inspect a locally built loom-server image and print a Markdown provenance
# report (image ID, config, layer history, resolved base-image digests, and
# runtime smoke checks). Read-only against the local daemon: nothing is pushed,
# tagged for a registry, or logged in to.
#
#   deploy/ci/inspect_image.sh <image-ref> [Dockerfile]
set -euo pipefail

image="${1:?image ref}"
dockerfile="${2:-Dockerfile}"
fail=0

echo "## loom-server image inspection: \`$image\`"
echo
echo "Local image ID (config digest; a registry digest only exists after a push, which this job never does):"
echo
echo '```'
docker image inspect "$image" --format '{{.Id}}'
echo '```'
echo
echo "### Config"
echo
echo '```'
docker image inspect "$image" --format \
  'User={{.Config.User}} Entrypoint={{json .Config.Entrypoint}} ExposedPorts={{json .Config.ExposedPorts}} Healthcheck={{json .Config.Healthcheck.Test}} Size={{.Size}} Arch={{.Architecture}}/{{.Os}}'
echo '```'
echo
echo "### Layer history"
echo
echo '```'
docker image history --no-trunc --format '{{.CreatedBy}}' "$image" | sed 's/^/  /'
echo '```'
echo
echo "### Base images resolved by the build"
echo
echo "FROM references in $dockerfile and the digests the local daemon holds for them:"
echo
echo '```'
grep -E '^FROM ' "$dockerfile" | awk '{print $2}' | while read -r ref; do
  echo "FROM $ref"
  docker image inspect "$ref" --format '  Id={{.Id}} RepoDigests={{json .RepoDigests}}' 2>/dev/null \
    || echo "  (not present in the local daemon)"
done
echo '```'
echo
echo "### Runtime smoke checks"
echo
echo '```'
uid="$(docker run --rm --entrypoint /usr/bin/id "$image" -u)"
echo "uid inside container: $uid"
if [ "$uid" != "10001" ]; then echo "FAIL: expected uid 10001"; fail=1; fi

if docker run --rm --entrypoint /bin/sh "$image" -c 'test -s /etc/ssl/certs/ca-certificates.crt'; then
  echo "OS CA bundle present: /etc/ssl/certs/ca-certificates.crt"
else
  echo "FAIL: OS CA bundle missing"; fail=1
fi

# The binary must execute (correct libc/arch). With no server listening the
# health probe must return exit 1 — not 126/127 (not executable / not found).
set +e
docker run --rm "$image" --healthcheck
rc=$?
set -e
echo "loom-server --healthcheck exit code: $rc (1 expected: binary runs, no server)"
if [ "$rc" -ne 1 ]; then echo "FAIL: unexpected healthcheck exit code"; fail=1; fi

# Starting without configuration must fail fast with a clear diagnostic.
set +e
out="$(docker run --rm "$image" 2>&1)"
rc=$?
set -e
echo "loom-server (no env) exit code: $rc"
echo "$out" | tail -n 3 | sed 's/^/  /'
if [ "$rc" -ne 1 ] || ! echo "$out" | grep -q DATABASE_URL; then
  echo "FAIL: expected exit 1 mentioning DATABASE_URL"; fail=1
fi
echo '```'

exit "$fail"
