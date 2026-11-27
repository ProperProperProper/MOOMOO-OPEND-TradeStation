#!/usr/bin/env bash
# Native Rust cross-compilation check forwarder
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo xtask check-cross "$@"
