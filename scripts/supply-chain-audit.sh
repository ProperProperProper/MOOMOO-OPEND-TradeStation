#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "🔒 [SECURITY AUDIT] Verifying Rust Dependency Supply-Chain Policies..."

if command -v cargo-deny &> /dev/null || cargo deny --version &> /dev/null; then
    DENY_CMD="cargo deny"
else
    echo "❌ cargo-deny is required for supply-chain security checks."
    echo "   Install it with: brew install cargo-deny  (or cargo install cargo-deny)"
    exit 1
fi

echo "🔍 1. Scanning RustSec advisory database for known CVEs and vulnerabilities..."
$DENY_CMD check advisories

echo "🚫 2. Enforcing banned crate policies (vulnerable crypto & undeclared network libraries)..."
$DENY_CMD check bans

echo "📜 3. Enforcing open-source license compliance..."
$DENY_CMD check licenses

echo "📦 4. Enforcing trusted dependency registry sources..."
$DENY_CMD check sources

echo "✅ [SUPPLY-CHAIN LOCK VERIFIED] All dependency security assertions passed."
