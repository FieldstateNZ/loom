#!/usr/bin/env bash
# Produce a clean Docker build context containing ONLY committed files.
#
#   deploy/ci/docker_context.sh [git-ref] <output-dir>
#
# `git archive` exports the tree of <git-ref> (default HEAD): no .git directory,
# no untracked or ignored files, no .env*, no editor/user files — so what the
# daemon sees is exactly what is in version control. The script then asserts
# that invariant before handing the directory to `docker build`.
set -euo pipefail

if [ "$#" -eq 1 ]; then
  ref="HEAD"; out="$1"
elif [ "$#" -eq 2 ]; then
  ref="$1"; out="$2"
else
  echo "usage: $0 [git-ref] <output-dir>" >&2
  exit 2
fi

if [ -e "$out" ] && [ -n "$(ls -A "$out" 2>/dev/null)" ]; then
  echo "error: output dir $out exists and is not empty" >&2
  exit 1
fi
mkdir -p "$out"

commit="$(git rev-parse --verify "${ref}^{commit}")"
git archive --format=tar "$commit" | tar -x -C "$out"

# Invariants: nothing from .git, no env files, no untracked content.
if [ -e "$out/.git" ]; then
  echo "error: .git leaked into the build context" >&2
  exit 1
fi
leaked="$(find "$out" \( -name '.env' -o -name '.env.*' \) -print)"
if [ -n "$leaked" ]; then
  echo "error: env files in the build context:" >&2
  echo "$leaked" >&2
  exit 1
fi
expected="$(git ls-tree -r --name-only "$commit" | wc -l)"
actual="$(cd "$out" && find . \( -type f -o -type l \) | wc -l)"
if [ "$expected" -ne "$actual" ]; then
  echo "error: context file count ($actual) != tracked file count ($expected)" >&2
  exit 1
fi

echo "build context: $out"
echo "commit:        $commit"
echo "files:         $actual (tracked files only)"
