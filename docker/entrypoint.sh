#!/bin/sh
set -e

# 1) Sobe o Ollama em background.
ollama serve &
OLLAMA_PID=$!

# 2) Aguarda ficar saudavel.
for _ in $(seq 1 60); do
  if curl -fsS http://127.0.0.1:11434/api/tags >/dev/null 2>&1; then break; fi
  sleep 2
done

# 3) Garante o modelo de embedding (bakeado no build; pull em runtime se preciso).
ollama pull all-minilm || true

# 4) Mantem o Ollama vivo; encerra tudo junto no sinal de saida.
trap 'kill $OLLAMA_PID 2>/dev/null || true' EXIT TERM INT

# 5) arags-server em foreground (PID 1 do container).
exec arags-server
