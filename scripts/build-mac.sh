#!/usr/bin/env bash
# Builds the Clickr Mac app with the Dioxus CLI and ad-hoc signs it, so macOS can
# remember its Bluetooth and Accessibility permissions.
#
# Usage: scripts/build-mac.sh
# Requires: cargo install dioxus-cli@0.7.10
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# Run from the workspace root with -p: from inside crates/clickr-app, dx 0.7.10 fails to
# resolve the workspace's default members.
dx bundle --platform macos --release -p clickr-app

APP="$ROOT/target/dx/clickr-app/bundle/macos/macos/ClickrApp.app"
codesign --force --deep --sign - "$APP"
codesign --verify "$APP"

echo "Done. Open $APP"
