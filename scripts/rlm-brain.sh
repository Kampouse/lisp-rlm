#!/bin/bash
# rlm-brain.sh — run a mutator lisp script with the BEST available brain.
#
# Solver worlds stay on the local qwen shim (cheap, always-on). The
# MUTATOR lane (tactics / dream tasks / action proposals) gets whatever
# frontier model is configured in $WS/data/rlm/dream/brain.env:
#
#   RLM_BRAIN_BASE=https://api.z.ai/api/coding/paas/v4   (OpenAI-compatible)
#   RLM_BRAIN_KEY=sk-...
#   RLM_BRAIN_MODEL=glm-4.7-flash
#
# No file / empty values → falls back to the local shim. The brain used
# is stamped to data/rlm/dream/last-brain.txt for trace headers.
REPO="$(cd "$(dirname "$0")/.." && pwd)"
WS="/Users/asil/.openclaw/workspace"
ENVF="$WS/data/rlm/dream/brain.env"
SCRIPT="$1"

BASE="http://127.0.0.1:8765/v1"; KEY="local-shim"; MODEL="qwen3.5-4b"; TAG="local:qwen3.5-4b"
if [ -s "$ENVF" ]; then
  set -a; . "$ENVF"; set +a
  if [ -n "${RLM_BRAIN_BASE:-}" ] && [ -n "${RLM_BRAIN_KEY:-}" ] && [ -n "${RLM_BRAIN_MODEL:-}" ]; then
    BASE="$RLM_BRAIN_BASE"; KEY="$RLM_BRAIN_KEY"; MODEL="$RLM_BRAIN_MODEL"; TAG="brain:$MODEL"
  fi
fi
echo "$TAG" > "$REPO/data/rlm/dream/last-brain.txt"

cd "$REPO" && exec env RLM_API_BASE="$BASE" RLM_API_KEY="$KEY" RLM_MODEL="$MODEL" \
  perl -e 'alarm 600; exec @ARGV' ./target/release/lisp-run "$SCRIPT"
