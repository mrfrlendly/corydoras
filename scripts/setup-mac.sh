#!/usr/bin/env bash
# Builds Corydoras on macOS, installs the app in ~/Applications and the MCP
# server in ~/.local/bin, then opens the app.
#
#   bash scripts/setup-mac.sh                    build and install
#   bash scripts/setup-mac.sh --register-claude  also add the MCP server to Claude Desktop
#
# The script changes nothing outside the project, ~/Applications/Corydoras.app,
# ~/.local/bin/corydoras-mcp and, once, a "Corydoras Local Signing" certificate
# in the login keychain (it keeps the Screen Recording grant across builds),
# unless you pass --register-claude.

set -euo pipefail

REGISTER_CLAUDE=0
for arg in "$@"; do
  case "$arg" in
    --register-claude) REGISTER_CLAUDE=1 ;;
    *) echo "Unknown option: $arg"; exit 2 ;;
  esac
done

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
say() { printf '\n\033[1;38;5;208m▸ %s\033[0m\n' "$1"; }
fail() { printf '\n\033[1;31m✗ %s\033[0m\n' "$1"; exit 1; }

# ---------------------------------------------------------------- toolchains
say "1/6  Check the toolchains"
[[ "$(uname -s)" == "Darwin" ]] || fail "This script is for macOS."
xcode-select -p >/dev/null 2>&1 || fail "Xcode Command Line Tools are missing. Run: xcode-select --install"

if ! command -v cargo >/dev/null 2>&1 && [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi
command -v cargo >/dev/null 2>&1 || fail "Rust is missing. Install it from https://rustup.rs"

RUST_MINOR="$(rustc --version | sed -E 's/rustc 1\.([0-9]+).*/\1/')"
if (( RUST_MINOR < 85 )); then
  echo "Rust $(rustc --version) is too old (1.85 or newer is necessary). Updating the stable toolchain…"
  rustup update stable
fi
echo "Rust:  $(rustc --version)"

if ! command -v node >/dev/null 2>&1; then
  for p in /opt/homebrew/bin /usr/local/bin "$HOME/.nvm/versions/node"/*/bin; do
    [[ -x "$p/node" ]] && export PATH="$p:$PATH" && break
  done
fi
command -v node >/dev/null 2>&1 || fail "Node.js is missing. Install it with: brew install node   (or from https://nodejs.org)"
NODE_MAJOR="$(node -p 'process.versions.node.split(".")[0]')"
(( NODE_MAJOR >= 20 )) || fail "Node.js $(node --version) is too old. Version 20 or newer is necessary."
echo "Node:  $(node --version)"

# ---------------------------------------------------------------- frontend packages
say "2/6  Install the frontend packages"
if [[ -f package-lock.json ]]; then npm ci --no-audit --no-fund; else npm install --no-audit --no-fund; fi

# ---------------------------------------------------------------- MCP server
say "3/6  Build the MCP server"
cargo build --release -p corydoras-mcp
mkdir -p "$HOME/.local/bin"
# Remove first: overwriting a running or cached signed binary in place makes macOS
# kill the new file (exit 137) because the old file's signature is still cached.
rm -f "$HOME/.local/bin/corydoras-mcp"
cp target/release/corydoras-mcp "$HOME/.local/bin/corydoras-mcp"
chmod 755 "$HOME/.local/bin/corydoras-mcp"
echo "Installed: $HOME/.local/bin/corydoras-mcp"

# ---------------------------------------------------------------- app
say "4/6  Build Corydoras.app (the first build takes a few minutes)"
npx tauri build --bundles app

APP_SRC="$ROOT/target/release/bundle/macos/Corydoras.app"
[[ -d "$APP_SRC" ]] || fail "The build did not make $APP_SRC"

# Sign with a local certificate. Tauri's ad-hoc signature changes with every
# build, and macOS ties privacy grants (Screen Recording for captures) to it,
# so each new build lost the grant while System Settings still showed it on.
# A certificate signature keeps the same identity from build to build.
SIGN_ID="Corydoras Local Signing"
if ! security find-certificate -c "$SIGN_ID" >/dev/null 2>&1; then
  echo "Make the signing certificate \"$SIGN_ID\" in the login keychain (once)…"
  TMPD="$(mktemp -d)"
  cat > "$TMPD/cfg" <<'EOF'
[req]
distinguished_name=dn
prompt=no
x509_extensions=ext
[dn]
CN=Corydoras Local Signing
[ext]
basicConstraints=critical,CA:false
keyUsage=critical,digitalSignature
extendedKeyUsage=critical,codeSigning
EOF
  P12_PASS="$(uuidgen)"
  /usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 7300 -config "$TMPD/cfg" \
    -keyout "$TMPD/key.pem" -out "$TMPD/cert.pem" >/dev/null 2>&1 || { rm -rf "$TMPD"; fail "Cannot make the signing certificate."; }
  /usr/bin/openssl pkcs12 -export -inkey "$TMPD/key.pem" -in "$TMPD/cert.pem" -name "$SIGN_ID" \
    -out "$TMPD/id.p12" -passout "pass:$P12_PASS" || { rm -rf "$TMPD"; fail "Cannot make the signing certificate."; }
  security import "$TMPD/id.p12" -k "$HOME/Library/Keychains/login.keychain-db" -P "$P12_PASS" -T /usr/bin/codesign \
    || { rm -rf "$TMPD"; fail "Cannot add the signing certificate to the login keychain."; }
  rm -rf "$TMPD"
fi
echo "Sign with \"$SIGN_ID\" (if macOS asks for the keychain, click Always Allow)…"
codesign --force --deep --sign "$SIGN_ID" "$APP_SRC" || fail "Signing failed."
NEW_REQ="$(codesign -d -r- "$APP_SRC" 2>&1 | sed -n 's/^designated => //p')"

say "5/6  Install the app in ~/Applications"
mkdir -p "$HOME/Applications"
# A grant made for a differently signed copy (an ad-hoc build) no longer matches:
# clear it so that macOS asks once, cleanly, instead of at every capture.
OLD_REQ=""
[[ -d "$HOME/Applications/Corydoras.app" ]] && OLD_REQ="$(codesign -d -r- "$HOME/Applications/Corydoras.app" 2>&1 | sed -n 's/^designated => //p')"
if [[ "$OLD_REQ" != "$NEW_REQ" ]]; then
  tccutil reset ScreenCapture com.travisforsyth.corydoras >/dev/null 2>&1 || true
  echo "The signature changed: macOS will ask once for Screen Recording at the next capture."
fi
if [[ -d "$HOME/Applications/Corydoras.app" ]]; then
  osascript -e 'tell application "Corydoras" to quit' >/dev/null 2>&1 || true
  rm -rf "$HOME/Applications/Corydoras.app"
fi
ditto "$APP_SRC" "$HOME/Applications/Corydoras.app"
xattr -dr com.apple.quarantine "$HOME/Applications/Corydoras.app" 2>/dev/null || true
echo "Installed: $HOME/Applications/Corydoras.app"

# ---------------------------------------------------------------- Claude Desktop (optional)
LIB="$(python3 -c 'import json,os;p=os.path.expanduser("~/Library/Application Support/Corydoras/config.json");print(json.load(open(p)).get("library_path") or "") if os.path.exists(p) else print("")' 2>/dev/null || true)"
if (( REGISTER_CLAUDE )); then
  say "6/6  Register the MCP server in Claude Desktop"
  CFG="$HOME/Library/Application Support/Claude/claude_desktop_config.json"
  mkdir -p "$(dirname "$CFG")"
  [[ -f "$CFG" ]] && cp "$CFG" "$CFG.bak-corydoras-$(date +%Y%m%d%H%M%S)" && echo "Backup: $CFG.bak-corydoras-*"
  python3 - "$CFG" "$HOME/.local/bin/corydoras-mcp" "$LIB" <<'PY'
import json, os, sys
cfg, cmd, lib = sys.argv[1], sys.argv[2], sys.argv[3]
data = {}
if os.path.exists(cfg):
    with open(cfg) as f:
        txt = f.read().strip()
        data = json.loads(txt) if txt else {}
servers = data.setdefault("mcpServers", {})
# No --library: the server reads the library that the app uses, also after a move.
entry = {"command": cmd, "args": []}
servers["corydoras"] = entry
with open(cfg, "w") as f:
    json.dump(data, f, indent=2)
print("Added mcpServers.corydoras to", cfg)
PY
  echo "Restart Claude Desktop to load the server."
else
  say "6/6  MCP server"
  echo "To register it in Claude Desktop, run this script again with --register-claude,"
  echo "or copy the JSON from Corydoras > Settings > Agents."
fi

say "Done. Opening Corydoras."
open "$HOME/Applications/Corydoras.app"
