#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOOKS_DIR="$ROOT_DIR/.git/hooks"

mkdir -p "$HOOKS_DIR"

# Pre-push hook: runs full supply-chain security audit before allowing any git push
cat > "$HOOKS_DIR/pre-push" << 'EOF'
#!/usr/bin/env bash
set -euo pipefail

echo "🔒 [GIT HOOK: PRE-PUSH] Running supply-chain security audit..."
./scripts/supply-chain-audit.sh
EOF

# Pre-commit hook: runs supply-chain audit before git commit
cat > "$HOOKS_DIR/pre-commit" << 'EOF'
#!/usr/bin/env bash
set -euo pipefail

# Only run supply-chain audit if Cargo.toml, Cargo.lock, or deny.toml changed
if git diff --cached --name-only | grep -E "(Cargo\.toml|Cargo\.lock|deny\.toml)" > /dev/null; then
    echo "🔒 [GIT HOOK: PRE-COMMIT] Dependencies or policy changed. Running supply-chain audit..."
    ./scripts/supply-chain-audit.sh
fi
EOF

chmod +x "$HOOKS_DIR/pre-push" "$HOOKS_DIR/pre-commit" "$ROOT_DIR/scripts/supply-chain-audit.sh" "$ROOT_DIR/scripts/verify-hardened-runtime.sh"
echo "✅ Git pre-commit and pre-push security hooks installed."
