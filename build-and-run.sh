#!/usr/bin/env bash
# build-and-run.sh — build the Rust core staticlib + StaapMac Swift
# shell, then launch the built app so its window appears.
#
# Intended sequence (see swift/README.md):
#   cargo build --lib            # target/debug/libstaap.a  (--debug)
#   cargo build --release --lib  # target/release/libstaap.a (default)
#   (cd swift && swift build [-c release])
#
# Launch path: the built binary directly
# (swift/.build/<config>/StaapMac), the same path the #67 fix
# verified. The app is a bundle-less SwiftPM executable, so it only
# shows its window thanks to the #67 activation fix (regular activation
# policy + activate on launch in the AppDelegate).
#
# Usage: ./build-and-run.sh [--release] [--build-only] [--help|-h]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG="debug"   # default; --release switches to a release build
BUILD_ONLY=0

usage() {
    cat <<'EOF'
Usage: ./build-and-run.sh [--release] [--build-only] [--help]

Build the Rust core staticlib and the StaapMac Swift shell,
then launch the built app so its window appears.

  (no flags)   debug build (cargo, swift default config) + launch
  --release    release build instead (cargo --release, swift -c release) + launch
  --build-only build without launching; prints the app binary path
  --help, -h   show this help and exit
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --release) CONFIG="release"; shift ;;
        --build-only) BUILD_ONLY=1; shift ;;
        --help|-h) usage; exit 0 ;;
        *) echo "build-and-run.sh: unknown argument: $1" >&2; usage >&2; exit 1 ;;
    esac
done

if [ "$CONFIG" = "release" ]; then
    echo "==> cargo build --release --lib"
    (cd "$ROOT" && cargo build --release --lib)
    echo "==> swift build -c release"
    (cd "$ROOT/swift" && swift build -c release)
    BIN="$ROOT/swift/.build/release/StaapMac"
else
    echo "==> cargo build --lib"
    (cd "$ROOT" && cargo build --lib)
    echo "==> swift build"
    (cd "$ROOT/swift" && swift build)
    BIN="$ROOT/swift/.build/debug/StaapMac"
fi

if [ "$BUILD_ONLY" = "1" ]; then
    echo "built: $BIN"
    exit 0
fi

echo "==> launching $BIN"
exec "$BIN"
