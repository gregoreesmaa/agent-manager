#!/bin/sh
# Live converse proof for the Linux shell (issue #63): spawn, pump for
# output, write, resize against a fake `muse` on PATH — the same hermetic
# trick as the core's `public_spawn_success_path` test, so no real agent
# and no live config are touched. No display needed.
#
# Usage: smoke_live.sh <path-to-staap-gtk-smoke>
set -eu

SMOKE="$1"
FAKEBIN="$(mktemp -d)"
trap 'rm -rf "$FAKEBIN"' EXIT INT TERM

cat >"$FAKEBIN/muse" <<'EOF'
#!/bin/sh
echo fake-muse-ready-63
sleep 30
EOF
chmod 755 "$FAKEBIN/muse"

# Hermetic HOME + config: discovery/persistence must degrade to empty and
# never touch the developer's live files.
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$FAKEBIN" "$SCRATCH"' EXIT INT TERM

PATH="$FAKEBIN:$PATH" \
  HOME="$SCRATCH" \
  STAAP_CONFIG="$SCRATCH/config.json" \
  "$SMOKE" --smoke-live
