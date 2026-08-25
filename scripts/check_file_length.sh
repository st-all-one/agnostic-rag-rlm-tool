#!/usr/bin/env bash
# File-length gate (plan 021 §9): every src/*.rs file must stay at or below
# MAX_LINES production lines (blank lines and #[cfg(test)] modules excluded).
# Exceptions require an entry in ALLOWLIST with a one-line justification.
set -euo pipefail

MAX_LINES=300
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# allowlist: "path:justification" (tracked in seeds issue agnostic-rlm-rs-XXXX/021.9)
ALLOWLIST=(
  "crates/arags-llm/src/config.rs:LlmConfig builder + per-family presets are one cohesive unit (021.9)"
  "crates/arags-llm/src/backend.rs:family request builders/parsers to be split into backend/family/* (021.9)"
  "crates/arags-server/src/grpc/query_cache.rs:plan-017 handler flow kept linear for auditability; helpers already extracted (021.9)"
  "crates/arags-server/src/grpc/rlm.rs:RLM RPC handlers grouped per proto service surface (021.9)"
  "crates/arags-server/src/grpc/search.rs:search+summary+context handlers share hybrid_search plumbing (021.9)"
  "crates/arags-server/src/config.rs:server.toml schema sections belong together; tests already externalized (021.9)"
  "crates/arags-storage/src/sqlite/chunks.rs:single table CRUD; splitting would fragment transactional seams (021.9)"
  "crates/arags-storage/src/sqlite/conn.rs:pool+PRAGMAs must stay adjacent to guarantee init order (021.9)"
  "crates/arags-storage/src/sqlite/qa_cache.rs:qa_cache lifecycle (store/hit/evict/invalidate) mirrors one SQLite dataset (021.9)"
)

violations=0
checked=0

while IFS= read -r -d '' file; do
    rel="${file#"$ROOT"/}"
    # Production lines: stop counting at the test-module marker.
    count=$(awk '/^#\[cfg\(test\)\]/{exit} {n++} END{print n+0}' "$file")
    # Discount blank lines inside the counted region.
    blanks=$(awk '/^#\[cfg\(test\)\]/{exit} /^[[:space:]]*$/{n++} END{print n+0}' "$file")
    prod=$((count - blanks))
    checked=$((checked + 1))
    [ "$prod" -le "$MAX_LINES" ] && continue

    allowed=0
    for entry in "${ALLOWLIST[@]}"; do
        path="${entry%%:*}"
        [ "$rel" = "$path" ] && allowed=1 && break
    done
    if [ "$allowed" -eq 1 ]; then
        echo "ALLOWED  $rel ($prod lines, in allowlist)"
    else
        echo "VIOLATION $rel ($prod production lines > $MAX_LINES)"
        violations=$((violations + 1))
    fi
done < <(find "$ROOT/crates" -path '*/src/*' -name '*.rs' -not -path '*/target/*' -print0)

echo "checked $checked files"
if [ "$violations" -gt 0 ]; then
    echo "FAIL: $violations file(s) exceed the ${MAX_LINES}-line limit."
    echo "Split the module by concern, or add a justified allowlist entry in scripts/check_file_length.sh."
    exit 1
fi
echo "OK: all source files within the ${MAX_LINES}-line limit."
