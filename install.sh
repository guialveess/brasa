#!/usr/bin/env bash
# Installs the latest Brasa release on macOS (Apple Silicon).
#
# Usage: curl -fsSL https://raw.githubusercontent.com/guialveess/brasa/main/install.sh | sh
#
# Downloads Brasa.app from the latest GitHub release, verifies its checksum against the same
# latest.json the in-app updater reads, and installs it into /Applications (or
# ~/Applications when /Applications is not writable). The build is ad-hoc signed, not notarized
# (packaging/macos/bundle.sh), so this also clears the quarantine flag the download would
# otherwise carry: without it, the first launch needs a right-click, Open past Gatekeeper.
set -euo pipefail

owner="guialveess"
repo="brasa"
manifest_url="https://github.com/$owner/$repo/releases/latest/download/latest.json"

if [ "$(uname -s)" != "Darwin" ] || [ "$(uname -m)" != "arm64" ]; then
    echo "error: this script only installs the macOS Apple Silicon build." >&2
    echo "Other platforms: build from source, see https://github.com/$owner/$repo#building-from-source" >&2
    exit 1
fi

for tool in curl tar shasum; do
    command -v "$tool" >/dev/null 2>&1 || { echo "error: $tool is required." >&2; exit 1; }
done

echo "Fetching the latest release manifest..."
manifest=$(curl -fsSL "$manifest_url")

# A tiny manifest, no jq dependency needed: pull the two fields of the macos-aarch64 asset out
# with grep/sed, the same shape as telaio-update's manifest.rs.
url=$(printf '%s' "$manifest" | grep -o '"macos-aarch64"[^}]*"url"[^,}]*' | sed -E 's/.*"url" *: *"([^"]+)".*/\1/')
sha256=$(printf '%s' "$manifest" | grep -o '"macos-aarch64"[^}]*"sha256"[^,}]*' | sed -E 's/.*"sha256" *: *"([^"]+)".*/\1/')
version=$(printf '%s' "$manifest" | grep -o '"version" *: *"[^"]*"' | head -n 1 | sed -E 's/.*"version" *: *"([^"]+)".*/\1/')

if [ -z "$url" ] || [ -z "$sha256" ]; then
    echo "error: no macOS Apple Silicon build in the latest release." >&2
    exit 1
fi

echo "Installing Brasa $version..."
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

archive="$work/Brasa.app.tar.gz"
curl -fsSL -o "$archive" "$url"

actual_sha256=$(shasum -a 256 "$archive" | cut -d' ' -f1)
if [ "$actual_sha256" != "$sha256" ]; then
    echo "error: checksum mismatch, expected $sha256, got $actual_sha256." >&2
    exit 1
fi

tar -xzf "$archive" -C "$work"

if [ -w /Applications ]; then
    target_dir=/Applications
else
    target_dir="$HOME/Applications"
    mkdir -p "$target_dir"
fi

rm -rf "$target_dir/Brasa.app"
mv "$work/Brasa.app" "$target_dir/Brasa.app"
xattr -dr com.apple.quarantine "$target_dir/Brasa.app" 2>/dev/null || true

echo "Installed Brasa $version to $target_dir/Brasa.app"
echo "Open it from Launchpad or: open \"$target_dir/Brasa.app\""
