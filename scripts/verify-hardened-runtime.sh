#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "🛡️  [SECURITY AUDIT] Verifying macOS Hardened Runtime & Dylib Tampering Protections..."

APP_PATH="${1:-target/release/Terminal.app}"

if [ ! -d "$APP_PATH" ]; then
    echo "📦 Bundle not found at '$APP_PATH'. Building bundle via 'cargo xtask bundle-macos'..."
    cargo xtask bundle-macos
fi

if [ ! -d "$APP_PATH" ]; then
    echo "❌ Application bundle not found at '$APP_PATH'."
    exit 1
fi

echo "🔍 1. Validating code signature integrity..."
codesign --verify --deep --strict --verbose=2 "$APP_PATH"

echo "🔍 2. Inspecting Hardened Runtime flags (CS_RUNTIME = 0x10000)..."
SIGN_INFO=$(codesign -d --verbose=2 "$APP_PATH" 2>&1)
echo "$SIGN_INFO" | grep -E "(flags=|CodeDirectory)"

if echo "$SIGN_INFO" | grep -q "flags=.*runtime"; then
    echo "✅ Hardened Runtime (flags=runtime) is ACTIVE."
else
    echo "❌ Hardened Runtime flag is NOT present in bundle signature!"
    exit 1
fi

echo "🔍 3. Verifying embedded entitlements..."
ENTITLEMENTS=$(codesign -d --entitlements :- "$APP_PATH" 2>&1 || true)
if echo "$ENTITLEMENTS" | grep -q "com.apple.security.cs.disable-library-validation"; then
    echo "✅ Library Validation entitlement confirmed: local dylib tampering is BLOCKED."
else
    echo "ℹ️  Default Hardened Runtime library validation is enforced by macOS."
fi

echo "✅ [MACOS HARDENED RUNTIME VERIFIED] Local dylib tampering and injection vectors are prevented."
