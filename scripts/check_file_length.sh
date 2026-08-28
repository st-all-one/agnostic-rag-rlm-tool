#!/usr/bin/env bash
# File-length gate (plan 021 §9): every src/*.rs file must stay at or below
# MAX_LINES production lines (blank lines and #[cfg(test)] modules excluded).
# Exceptions require an entry in ALLOWLIST with a one-line justification.
set -euo pipefail

MAX_LINES=300
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# allowlist: "path:justification" (tracked in seeds issue agnostic-rag-rlm-tool-XXXX/021.9)
# NOTE: the 9 files below were split into cohesive submodules (issue
# agnostic-rag-rlm-tool-0fc4) and removed from the allowlist once each production
# surface dropped to <= 300 lines:
#   crates/arags-llm/src/config.rs         -> config/{presets,llm_config}.rs
#   crates/arags-llm/src/backend.rs        -> backend/family/{openai,anthropic,gemini,ollama}.rs
#   crates/arags-server/src/grpc/query_cache.rs -> query_cache/{helpers,query,store,invalidate,pending}.rs
#   crates/arags-server/src/grpc/rlm.rs    -> grpc/rlm/{mod,complete,quorum}.rs
#   crates/arags-server/src/grpc/search.rs -> grpc/search/{hybrid,summary,context,query}.rs
#   crates/arags-server/src/config.rs      -> config/{exploration,rlm,embedder,search,maintenance,quorum,qa_cache,server_impl}.rs
#   crates/arags-storage/src/sqlite/chunks.rs     -> chunks/{basic,time_travel,access,content,vector}.rs
#   crates/arags-storage/src/sqlite/conn.rs       -> conn/ops.rs
#   crates/arags-storage/src/sqlite/qa_cache.rs  -> qa_cache/{types,row,store,mutate,evict,embed}.rs
ALLOWLIST=(
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
