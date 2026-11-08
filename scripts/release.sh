#!/usr/bin/env bash
# Bumps the workspace version, commits, tags and pushes: the one thing that starts
# .github/workflows/release.yml, which builds, packages and publishes the GitHub release.
#
# Usage: scripts/release.sh <version>   (for example: scripts/release.sh 0.10.0)
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: $0 <version>" >&2
    echo "example: $0 0.10.0" >&2
    exit 2
fi

version=$1
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
    echo "error: version '$version' is not X.Y.Z or X.Y.Z-prerelease" >&2
    exit 2
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

if [[ -n "$(git status --porcelain)" ]]; then
    echo "error: working tree is not clean, commit or stash first." >&2
    exit 1
fi

branch=$(git rev-parse --abbrev-ref HEAD)
if [[ "$branch" != "main" ]]; then
    echo "error: on branch '$branch', releases are tagged from main." >&2
    exit 1
fi

if git rev-parse "v$version" >/dev/null 2>&1; then
    echo "error: tag v$version already exists." >&2
    exit 1
fi

echo "Bumping the workspace version to $version..."
awk -v new_version="$version" '
    /^\[workspace\.package\]/ { section = 1 }
    /^\[/ && !/^\[workspace\.package\]/ { section = 0 }
    section && /^version *=/ { print "version = \"" new_version "\""; next }
    { print }
' Cargo.toml > Cargo.toml.new
mv Cargo.toml.new Cargo.toml

echo "Checking the workspace still builds..."
cargo check --workspace --quiet

git add Cargo.toml Cargo.lock
git commit -m "Release $version"
git tag "v$version"

echo "Pushing main and the tag..."
git push origin main
git push origin "v$version"

origin=$(git remote get-url origin | sed -E 's#.*github\.com[:/]##; s#\.git$##')
echo
echo "Pushed v$version. The build is running at:"
echo "  https://github.com/$origin/actions"
echo "Once it finishes, the release is published at:"
echo "  https://github.com/$origin/releases/tag/v$version"
