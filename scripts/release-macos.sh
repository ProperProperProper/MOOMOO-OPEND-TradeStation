#!/usr/bin/env bash
# Native Rust release packager forwarder
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo xtask release-macos "$@"
