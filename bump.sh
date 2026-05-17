#!/bin/bash
V=$1
if [ -z "$V" ]; then
  echo "Usage: ./bump.sh <version>"
  exit 1
fi
sed -i '' "s/\"version\": \"[^\"]*\"/\"version\": \"$V\"/" package.json src-tauri/tauri.conf.json
sed -i '' "s/^version = \"[^\"]*\"/version = \"$V\"/" src-tauri/Cargo.toml
npm install --package-lock-only --silent 2>/dev/null
(cd src-tauri && cargo update -p zagorakys --quiet 2>/dev/null)
echo "Bumped to $V"
