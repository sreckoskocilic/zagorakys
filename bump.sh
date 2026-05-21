#!/bin/bash
set -e
V=$1
if [ -z "$V" ]; then
  echo "Usage: ./bump.sh <version>"
  exit 1
fi
if ! echo "$V" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "Error: version must be X.Y.Z (got '$V')"
  exit 1
fi
sed -i '' "s/\"version\": \"[^\"]*\"/\"version\": \"$V\"/" package.json src-tauri/tauri.conf.json
sed -i '' "s/^version = \"[^\"]*\"/version = \"$V\"/" src-tauri/Cargo.toml
npm install --package-lock-only --silent
(cd src-tauri && cargo update -p zagorakys --quiet)
echo "Bumped to $V"
