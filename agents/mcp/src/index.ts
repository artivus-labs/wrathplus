#!/usr/bin/env bun
// Entry point: an MCP server on stdio over the WrathPlus client's agent control port.

import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { AgentClient } from "./client.ts";
import { Monitor } from "./monitor.ts";
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
  const client = new AgentClient(config.url, config.token);
  // WRATHPLUS_SCORECARD_SECONDS: how often the scorecard samples (30 s; 0 turns the scorecard and
  // the judge off). WRATHPLUS_JUDGE_DIR: where their files live (default <client home>/Agent/judge).
  const seconds = Number(process.env.WRATHPLUS_SCORECARD_SECONDS ?? 30);
  const monitor =
    Number.isFinite(seconds) && seconds > 0
      ? new Monitor({
          client,
          intervalMs: seconds * 1000,
          dir: process.env.WRATHPLUS_JUDGE_DIR,
          log: (line) => console.error(`wrathplus-agent-mcp: ${line}`),
        })
      : undefined;
  // WRATHPLUS_MCP_LONG_POLL=0: no events wait_ms (a host that wakes its agent itself).
  const server = createServer(client, {
    longPoll: process.env.WRATHPLUS_MCP_LONG_POLL !== "0",
    judging: monitor?.ready,
  });
  monitor?.start();
  // The host closing stdin ends the server: the monitor's timer would otherwise keep an orphan
  // sampling and judging beside the host's next server, two judges on one folder.
  process.stdin.once("end", () => process.exit(0));
  process.stdin.once("close", () => process.exit(0));
  await server.connect(new StdioServerTransport());
  console.error(`wrathplus-agent-mcp: serving ${config.url} on stdio`);
}
