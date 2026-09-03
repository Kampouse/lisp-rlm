#!/usr/bin/env bash
# board.sh — full verification board in one command: battery + gauntlet + twins.
# Serialized (mock state is a shared /tmp file otherwise), and each leg runs on
# an isolated NEAR_MOCK_STATE so concurrent sessions can't corrupt results.
# Exit 0 only when all three legs are green.
set -euo pipefail
cd "$(dirname "$0")/.."
# Artifacts persist under target/board/ (state file still per-run isolated).
BOARD_DIR="target/board"
mkdir -p "$BOARD_DIR"
STATE_DIR="$(mktemp -d)"
trap 'rm -rf "$STATE_DIR"' EXIT

echo "── build"
cargo build --release --bin near-mock --bin compile >/dev/null

echo "── battery"
export NEAR_MOCK_STATE="$STATE_DIR/battery.bin"
cargo test --release > "$BOARD_DIR/battery.log" 2>&1 || true
python3 scripts/board_sum.py "$BOARD_DIR/battery.log"
grep -E '^error|FAILED|panicked' "$BOARD_DIR/battery.log" | head -5 || true

echo "── gauntlet"
unset NEAR_MOCK_STATE   # runners bring their own isolated state
./projects/nostr-gov-lisp/tests/run-gauntlet.sh > "$BOARD_DIR/gauntlet.log" 2>&1 || {
  grep -E '✗|pass /' "$BOARD_DIR/gauntlet.log" | head -20
  exit 1
}
grep 'pass /' "$BOARD_DIR/gauntlet.log"

echo "── twins"
./projects/nostr-gov-lisp/tests/diff-ts-full.sh > "$BOARD_DIR/twins.log" 2>&1 || {
  head -30 "$BOARD_DIR/twins.log"
  exit 1
}
head -2 "$BOARD_DIR/twins.log"

echo "board: all green (logs in target/board/)"
