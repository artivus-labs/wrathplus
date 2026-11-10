// One page for every agent: each running client's live view side by side, with what its planner
// last said and did. Local only (127.0.0.1); the views carry their tokens, as the Manager's Watch
// button does.

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { agentDir, answering, loadAgents, runningOf, stateHome, tokenOf, viewOf } from "./manager.ts";

interface Entry {
  name: string;
  account: string;
  port: number;
  client: string;
  player: unknown;
  driver: { model: string; goal: string; started: string } | null;
  view: string | null;
  lines: string[];
}

/** The last lines of an agent's newest session log: what the planner thought, did and heard. */
export function recentLines(notes: string, count = 14): string[] {
  const dir = join(notes, "sessions");
  if (!existsSync(dir)) return [];
  const newest = readdirSync(dir).filter((f) => f.endsWith(".jsonl")).sort().pop();
  if (!newest) return [];
  const shown = new Set(["planner", "turn end", "fast loop", "events", "wake-up", "outside note", "harness", "compaction"]);
  const lines: string[] = [];
  for (const raw of readFileSync(join(dir, newest), "utf8").trim().split("\n").slice(-200)) {
    try {
      const entry = JSON.parse(raw) as { at: string; kind: string; text: string };
      if (shown.has(entry.kind)) lines.push(`${entry.at.slice(11, 19)} ${entry.text.split("\n[Now:")[0]!.slice(0, 240)}`);
    } catch {
      // A line being written.
    }
  }
  return lines.slice(-count);
}

async function entries(): Promise<Entry[]> {
  const home = stateHome();
  return Promise.all(
    loadAgents(home).map(async (agent) => {
      const status = await answering(agent, tokenOf(home, agent));
      const running = runningOf(home, agent);
      return {
        name: agent.name,
        account: agent.account,
        port: agent.port,
        client: status ? (status.in_world ? "in world" : String(status.state)) : "stopped",
        player: status?.player ?? null,
        driver: running ? { model: running.model, goal: running.goal, started: running.started } : null,
        view: status ? viewOf(home, agent) : null,
        lines: recentLines(join(agentDir(home, agent.account), "notes")),
      };
    }),
  );
}

const PAGE = `<!doctype html>
<html><head><meta charset="utf-8"><title>WrathPlus agents</title>
<style>
  body { margin: 0; background: #0d1117; color: #d7dde5; font: 14px/1.4 system-ui, sans-serif; }
  header { padding: 14px 20px; border-bottom: 1px solid #222a35; display: flex; gap: 16px; align-items: baseline; }
  h1 { font-size: 17px; margin: 0; }
  .muted { color: #8b96a5; }
  main { display: grid; grid-template-columns: repeat(auto-fill, minmax(560px, 1fr)); gap: 16px; padding: 16px; }
  .card { background: #151b24; border: 1px solid #222a35; border-radius: 10px; overflow: hidden; }
  .head { padding: 10px 14px; display: flex; justify-content: space-between; gap: 10px; }
  .name { font-weight: 600; }
  .pill { font-size: 12px; padding: 2px 8px; border-radius: 99px; background: #222a35; }
  .on { background: #1f4d2e; } .off { background: #4d1f1f; }
  iframe { width: 100%; aspect-ratio: 16 / 10; border: 0; background: #000; display: block; }
  .empty { aspect-ratio: 16 / 10; display: grid; place-items: center; background: #0b0f15; }
  .goal { padding: 8px 14px 0; font-size: 13px; }
  pre { margin: 0; padding: 8px 14px 12px; font: 12px/1.45 ui-monospace, monospace; white-space: pre-wrap; max-height: 220px; overflow: auto; color: #b8c2cf; }
</style></head>
<body>
<header><h1>WrathPlus agents</h1><span class="muted">bun run agent run &lt;name&gt; to start one · this page refreshes itself</span></header>
<main id="grid"></main>
<script>
const grid = document.getElementById("grid");
const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
async function refresh() {
  const agents = await (await fetch("/api/agents")).json();
  if (!agents.length) { grid.innerHTML = '<p class="muted">No agents: bun run agent add &lt;name&gt;</p>'; return; }
  for (const a of agents) {
    let card = document.getElementById("a-" + a.account);
    if (!card) {
      card = document.createElement("section");
      card.className = "card"; card.id = "a-" + a.account;
      card.innerHTML = '<div class="head"></div><div class="frame"></div><div class="goal"></div><pre></pre>';
      grid.appendChild(card);
    }
    const p = a.player || {};
    const who = p.name ? esc(p.name) + " · level " + esc(p.level) + (p.zone ? " · " + esc(p.zone) : "") : "";
    card.querySelector(".head").innerHTML =
      '<span><span class="name">' + esc(a.name) + '</span> <span class="muted">:' + a.port + " " + who + "</span></span>" +
      '<span><span class="pill ' + (a.view ? "on" : "off") + '">client ' + esc(a.client) + "</span> " +
      '<span class="pill ' + (a.driver ? "on" : "") + '">' + (a.driver ? esc(a.driver.model) : "no planner") + "</span></span>";
    const frame = card.querySelector(".frame");
    // Keep a live iframe as it is; swap only when the client starts or stops.
    if (a.view && frame.dataset.view !== a.view) { frame.dataset.view = a.view; frame.innerHTML = '<iframe src="' + esc(a.view) + '"></iframe>'; }
    if (!a.view && frame.dataset.view !== "") { frame.dataset.view = ""; frame.innerHTML = '<div class="empty muted">client not running</div>'; }
    card.querySelector(".goal").innerHTML = a.driver ? '<span class="muted">Goal:</span> ' + esc(a.driver.goal) : "";
    const pre = card.querySelector("pre");
    const atEnd = pre.scrollTop + pre.clientHeight >= pre.scrollHeight - 4;
    pre.textContent = a.lines.join("\\n");
    if (atEnd) pre.scrollTop = pre.scrollHeight;
  }
}
refresh(); setInterval(refresh, 3000);
</script>
</body></html>`;

export function serveDashboard(port: number) {
  return Bun.serve({
    port,
    hostname: "127.0.0.1",
    async fetch(request) {
      const path = new URL(request.url).pathname;
      if (path === "/api/agents") return Response.json(await entries());
      if (path === "/") return new Response(PAGE, { headers: { "Content-Type": "text/html; charset=utf-8" } });
      return new Response("Not found", { status: 404 });
    },
  });
}
