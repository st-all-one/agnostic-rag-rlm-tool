/**
 * arlm-pi-extension — Integração do arlm com Pi Agent
 *
 * Instalar:
 *   1. Copie este diretório para ~/.pi/extensions/arlm/
 *   2. Adicione ao package.json do Pi:
 *      { "pi": { "extensions": ["~/.pi/extensions/arlm/index.ts"] } }
 *   3. Reinicie o Pi Agent
 *
 * Ferramentas disponíveis:
 *   - rlm_context: Busca contexto do projeto
 *   - rlm_search: Busca código
 *   - rlm_run: Análise RLM recursiva
 */

import { ExtensionAPI } from "@mariozechner/pi-coding-agent";
import { execSync } from "child_process";

export default function activate(pi: ExtensionAPI) {
  // Tool de contexto
  pi.registerTool({
    name: "rlm_context",
    description: "Busca contexto relevante do projeto usando RLM",
    parameters: {
      task: { type: "string", description: "Tarefa ou pergunta" },
      format: {
        type: "string",
        enum: ["prompt", "json", "markdown"],
        default: "prompt",
      },
    },
    execute: async (params) => {
      const result = execSync(
        `arlm context "${params.task}" --project ${process.cwd()} --format ${params.format ?? "prompt"}`,
        { encoding: "utf-8" }
      );
      return result;
    },
  });

  // Tool de busca
  pi.registerTool({
    name: "rlm_search",
    description: "Busca rápida no código do projeto",
    parameters: {
      query: { type: "string", description: "Termos de busca" },
      top_k: { type: "number", default: 5 },
    },
    execute: async (params) => {
      const result = execSync(
        `arlm search "${params.query}" --project ${process.cwd()} --top-k ${params.top_k ?? 5} --format json`,
        { encoding: "utf-8" }
      );
      return JSON.parse(result);
    },
  });

  // Tool de run RLM
  pi.registerTool({
    name: "rlm_run",
    description: "Executa análise RLM recursiva",
    parameters: {
      task: { type: "string", description: "Tarefa para analisar" },
      depth: { type: "number", default: 3 },
    },
    execute: async (params) => {
      const result = execSync(
        `arlm run "${params.task}" --project ${process.cwd()} --depth ${params.depth ?? 3} --format json`,
        { encoding: "utf-8" }
      );
      return JSON.parse(result);
    },
  });
}
