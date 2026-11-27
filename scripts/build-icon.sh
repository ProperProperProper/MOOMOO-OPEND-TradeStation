#!/usr/bin/env bash
# Native Rust icon build and verification forwarder
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo xtask build-icon "$@"
