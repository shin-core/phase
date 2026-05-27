#!/usr/bin/env bash
set -euo pipefail

# phase.rs onboarding bootstrap.
#
# Two axes:
#
#   Mode axis — what gets fetched/built:
#     * Full mode (default): everything an interactive human dev needs to run
#       the app in a browser, including the three Scryfall image/printing
#       sidecars consumed at runtime by the React frontend for card art.
#     * Agent mode (--agent, env PHASE_SETUP_AGENT=1): skips the Scryfall
#       sidecars. They are runtime-only image data — no Rust or frontend test
#       depends on them (the one vitest test that names them mocks `fetch`).
#       Use this for LLM-driven contributors running the docs/AI-CONTRIBUTOR.md
#       developer track — saves a multi-hundred-MB Scryfall bulk download with
#       zero impact on cargo / clippy / test / gen-card-data / coverage signal.
#
#   Build axis — whether to eagerly build WASM + card-data:
#     * Tilt mode (default when `tilt` is on PATH): skips the eager builds
#       because `tilt up` rebuilds both via the `wasm` and `card-data`
#       resources on first start. Avoids fighting Tilt for the cargo target
#       lock the moment the user starts the dev loop.
#     * Manual mode (--no-tilt or PHASE_SETUP_NO_TILT=1, or simply no `tilt`
#       on PATH): runs `build-wasm.sh` + `gen-card-data.sh` inline so the repo
#       is test-ready without Tilt.
#
# Caddy/SSL is intentionally NOT invoked — it's only needed for LAN HTTPS
# (WebRTC P2P guesting) and is gated behind `tilt up -- https`. See
# scripts/setup-ssl.sh + Caddyfile if you want it.

NO_TILT="${PHASE_SETUP_NO_TILT:-0}"
AGENT="${PHASE_SETUP_AGENT:-0}"
for arg in "$@"; do
  case "$arg" in
    --no-tilt)         NO_TILT=1 ;;
    --agent|--no-scryfall) AGENT=1 ;;
    -h|--help)
      sed -n '3,30p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unknown arg: $arg" >&2
      echo "  --agent             skip Scryfall image sidecars (LLM contributor mode)" >&2
      echo "  --no-tilt           skip Tilt detection; eager-build WASM + card-data" >&2
      echo "  -h, --help          this message" >&2
      exit 2
      ;;
  esac
done

echo "=== phase.rs Setup ==="
if [ "$AGENT" = 1 ]; then
  echo "    (mode: agent — Scryfall image fetchers skipped)"
fi
echo ""

# --- Preflight: hard tools ---
missing=()
for tool in cargo pnpm jq; do
  command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
done
if [ "${#missing[@]}" -ne 0 ]; then
  echo "ERROR: missing required tools: ${missing[*]}" >&2
  echo "  cargo: https://rustup.rs/" >&2
  echo "  pnpm:  https://pnpm.io/installation" >&2
  echo "  jq:    https://stedolan.github.io/jq/" >&2
  exit 1
fi

# --- Preflight: soft tool (tilt) ---
USE_TILT=0
if [ "$NO_TILT" != 1 ] && command -v tilt >/dev/null 2>&1; then
  USE_TILT=1
fi

FAIL=0

# --- Scryfall sidecars (skipped in agent mode) ---
# These are runtime-only image data for the React frontend. No Rust or vitest
# test depends on them — see docs/AI-CONTRIBUTOR.md and CLAUDE.md.
if [ "$AGENT" = 1 ]; then
  echo "Step 1: Skipping Scryfall sidecars (agent mode)."
else
  echo "Step 1: Fetching Scryfall sidecars (parallel)..."
  ./scripts/gen-scryfall-images.sh         & PID_IMAGES=$!
  ./scripts/gen-scryfall-token-images.sh   & PID_TOKEN_IMAGES=$!
  ./scripts/gen-scryfall-printings.sh      & PID_PRINTINGS=$!

  wait $PID_IMAGES        || FAIL=1
  wait $PID_TOKEN_IMAGES  || FAIL=1
  wait $PID_PRINTINGS     || FAIL=1
  if [ $FAIL -ne 0 ]; then
    echo "ERROR: Scryfall sidecar fetch failed." >&2
    exit 1
  fi
fi

# Comprehensive Rules — gitignored, non-fatal on failure.
if [ ! -f docs/MagicCompRules.txt ]; then
  echo ""
  echo "Fetching MTG Comprehensive Rules (local dev reference only)..."
  ./scripts/fetch-comp-rules.sh || echo "  (skipped — run ./scripts/fetch-comp-rules.sh later)"
fi

# --- Frontend deps (parallel-safe with cargo work below) ---
echo ""
echo "Step 2: Installing frontend dependencies..."
(cd client && pnpm install) &
PID_PNPM=$!

# --- Card-data + WASM ---
if [ "$USE_TILT" = 1 ]; then
  echo ""
  echo "Step 3: Tilt detected — skipping eager WASM + card-data build."
  echo "        \`tilt up\` will run both on first start via the"
  echo "        'wasm' and 'card-data' resources."
else
  echo ""
  echo "Step 3: Building WASM + card-data (parallel)..."
  ./scripts/gen-card-data.sh & PID_CARDS=$!
  ./scripts/build-wasm.sh    & PID_WASM=$!

  wait $PID_CARDS || FAIL=1
  wait $PID_WASM  || FAIL=1
fi

wait $PID_PNPM || FAIL=1
if [ $FAIL -ne 0 ]; then
  echo "ERROR: setup step failed (see logs above)." >&2
  exit 1
fi

# --- Git hooks ---
echo ""
echo "Step 4: Configuring git hooks..."
git config --local include.path ../.gitconfig

echo ""
echo "Done!"
echo ""
if [ "$AGENT" = 1 ]; then
  echo "Agent mode complete. cargo / clippy / test-engine / gen-card-data /"
  echo "coverage / semantic-audit are all dev-ready. See docs/AI-CONTRIBUTOR.md."
elif [ "$USE_TILT" = 1 ]; then
  echo "Next: run \`tilt up\` to start the dev loop (wasm + card-data + frontend)."
  echo "      Add \`-- server\` / \`-- test\` / \`-- lint\` to start optional groups."
else
  echo "Next: run \`cd client && pnpm dev\` to start the dev server."
fi
echo ""
echo "Optional: LAN HTTPS / WebRTC P2P guesting requires a Caddy reverse proxy."
echo "          See scripts/setup-ssl.sh, then \`tilt up -- https\`. Not needed for"
echo "          single-machine dev."
