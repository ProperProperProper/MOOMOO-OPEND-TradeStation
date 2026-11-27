#!/usr/bin/env bash
# Native Rust bundle forwarder
set -euo pipefail
cd "$(dirname "$0")/.."
bin_arg=""
if [ $# -gt 0 ]; then
    bin_arg="--bin $1"
fi
exec cargo xtask bundle-macos $bin_arg
