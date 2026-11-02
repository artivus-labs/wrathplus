#!/usr/bin/env bun
// Entry point: an MCP server on stdio over the WrathPlus client's agent control port.

import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { AgentClient } from "./client.ts";
import { createServer } from "./tools.ts";

/** Read and check the environment; the token is never echoed. */
export function configFromEnv(env: Record<string, string | undefined>): { url: string; token: string } {
  const url = env.WRATHPLUS_AGENT_URL?.trim();
  const token = env.WRATHPLUS_AGENT_TOKEN?.trim();
  const problems: string[] = [];
  if (!url) {
    problems.push("WRATHPLUS_AGENT_URL is not set (e.g. http://127.0.0.1:17950)");
  } else if (!URL.canParse(url) || !/^https?:$/.test(new URL(url).protocol)) {
    problems.push(`WRATHPLUS_AGENT_URL is not an http(s) URL: ${url}`);
  }
  if (!token) problems.push("WRATHPLUS_AGENT_TOKEN is not set (the client's WOW_AGENT_TOKEN)");
  if (problems.length > 0) throw new Error(problems.join("; "));
  return { url: url!, token: token! };
}

if (import.meta.main) {
  let config: { url: string; token: string };
  try {
    config = configFromEnv(process.env);
  } catch (error) {
    console.error(`wrathplus-agent-mcp: ${(error as Error).message}`);
    process.exit(1);
  }
  const server = createServer(new AgentClient(config.url, config.token));
  await server.connect(new StdioServerTransport());
  console.error(`wrathplus-agent-mcp: serving ${config.url} on stdio`);
}
