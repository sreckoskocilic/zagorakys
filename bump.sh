#!/bin/bash
V=$1
if [ -z "$V" ]; then
  echo "Usage: ./bump.sh <version>"
  exit 1
fi
sed -i '' "s/\"version\": \"[^\"]*\"/\"version\": \"$V\"/" package.json src-tauri/tauri.conf.json
sed -i '' "s/^version = \"[^\"]*\"/version = \"$V\"/" src-tauri/Cargo.toml
echo "Bumped to $V"
