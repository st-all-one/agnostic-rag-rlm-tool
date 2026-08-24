/**
 * arags-pi-extension — Integração do arags com Pi Agent
 *
 * Instalar:
 *   1. Copie este diretório para ~/.pi/extensions/arags/
 *   2. Adicione ao package.json do Pi:
 *      { "pi": { "extensions": ["~/.pi/extensions/arags/index.ts"] } }
 *   3. Reinicie o Pi Agent
 *
 * Ferramentas disponíveis:
 *   - arags_search: Busca híbrida (BM25 + semântica) no código
 *   - arags_context: Contexto do projeto em formato prompt
 *   - arags_query: Pergunta analítica com digest QA via LLM local
 */

import { ExtensionAPI } from "@mariozechner/pi-coding-agent";
import { execSync } from "child_process";

export default function activate(pi: ExtensionAPI) {
  // Tool de busca
  pi.registerTool({
    name: "arags_search",
    description: "Busca híbrida (BM25 + semântica) no código do projeto via RAG server",
    parameters: {
      query: { type: "string", description: "Termos de busca" },
      top_k: { type: "number", default: 10 },
    },
    execute: async (params) => {
      const result = execSync(
        `arags search "${params.query}" --project ${process.cwd()} --top-k ${params.top_k ?? 10} --format jsonl`,
        { encoding: "utf-8" }
      );
      return JSON.parse(result);
    },
  });

  // Tool de contexto
  pi.registerTool({
    name: "arags_context",
    description: "Recupera contexto relevante do projeto em formato prompt via busca híbrida",
    parameters: {
      task: { type: "string", description: "Tarefa ou pergunta" },
    },
    execute: async (params) => {
      const result = execSync(
        `arags search "${params.task}" --project ${process.cwd()} --format text`,
        { encoding: "utf-8" }
      );
      return result;
    },
  });

  // Tool de query analítica (QA digest via LLM local do usuário)
  pi.registerTool({
    name: "arags_query",
    description: "Pergunta analítica ao projeto; digest QA via LLM local com cache server-side",
    parameters: {
      question: { type: "string", description: "Pergunta a analisar" },
    },
    execute: async (params) => {
      const result = execSync(
        `arags query "${params.question}" --qa`,
        { encoding: "utf-8" }
      );
      return JSON.parse(result);
    },
  });
}
