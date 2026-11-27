#!/usr/bin/env bash
# Native Rust web build forwarder
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo xtask build-web "$@"
