import { expect, test } from "bun:test";
import { Jev } from "../src/jev.ts";
import { Policy, WAIT } from "../src/policy.ts";

test("each tick asks Jev with the plan and state, runs the chosen intent once, and reports a stall", async () => {
  const lua: string[] = [];
  const asked: { state: { plan: string }; questions: Record<string, { criteria?: Record<string, string> }> }[] = [];
  const stuck = 0.9;
  const server = Bun.serve({
    port: 0,
    hostname: "127.0.0.1",
    async fetch(request) {
      const url = new URL(request.url);
      if (url.pathname === "/lua") {
        const code = await request.text();
        lua.push(code);
        return Response.json({ ok: true, values: code.includes("AgentNearby") ? [{ level: 1, nearby: [] }] : [1] });
      }
      if (url.pathname === "/v1/systemone") {
        asked.push((await request.json()) as (typeof asked)[number]);
        return Response.json({
          model: "jev-test",
          answers: {
            next: { type: "choice", choice: "talk", confidence: 0.9, probabilities: { talk: 0.9, wait: 0.1 } },
            stuck: { type: "noul", noul: stuck },
          },
        });
      }
      return new Response("no", { status: 404 });
    },
  });
  const base = server.url.href.replace(/\/$/, "");
  const stalled: string[] = [];
  const policy = new Policy({ url: base, token: "t", jev: new Jev("k", "jev-test", base), onStalled: (s) => stalled.push(s) });
  expect(await policy.tick()).toBeNull(); // no plan yet: nothing asked
  policy.update("Talk to the quest giver.", [{ name: "talk", when: "A quest giver is near.", lua: "AgentInteract('0x1')" }]);
  for (let i = 0; i < 5; i++) await policy.tick();
  server.stop(true);
  expect(asked[0]?.state.plan).toBe("Talk to the quest giver.");
  expect(Object.keys(asked[0]?.questions.next?.criteria ?? {})).toEqual(["talk", WAIT.name]);
  // The intent ran once: the next ticks fall inside its 3 s repeat window.
  expect(lua.filter((c) => c === "AgentInteract('0x1')")).toHaveLength(1);
  expect(policy.decisions.map((d) => d.ran)).toEqual([true, false, false, false, false]);
  expect(stalled).toHaveLength(1);
  expect(stalled[0]).toContain("stalled");

});

test("Jev errors carry the status and body", async () => {
  const server = Bun.serve({ port: 0, hostname: "127.0.0.1", fetch: () => Response.json({ detail: "nope" }, { status: 403 }) });
  const jev = new Jev("k", "jev-test", server.url.href.replace(/\/$/, ""));
  await expect(jev.ask({}, {})).rejects.toThrow("Jev HTTP 403");
  server.stop(true);
});

test("intents and decisions registered by the agent's own Lua are offered to Jev and carried out", async () => {
  const lua: string[] = [];
  let asked: { questions: Record<string, { criteria?: Record<string, string> }> } | undefined;
  const server = Bun.serve({
    port: 0,
    hostname: "127.0.0.1",
    async fetch(request) {
      const url = new URL(request.url);
      if (url.pathname === "/lua") {
        const code = await request.text();
        lua.push(code);
        const state = {
          registered: [{ name: "pull_wolf", when: "A wolf is near and we are healthy." }],
          decisions: [{ id: 7, name: "flee_or_fight", question: "Fight on?", options: { fight: "Stay", flee: "Run" } }],
        };
        return Response.json({ ok: true, values: code.includes("AgentNearby") ? [state] : [true] });
      }
      asked = (await request.json()) as typeof asked;
      return Response.json({
        model: "jev-test",
        answers: {
          next: { type: "choice", choice: "pull_wolf", confidence: 0.8, probabilities: {} },
          stuck: { type: "noul", noul: 0.1 },
          decision_7: { type: "choice", choice: "flee", confidence: 0.7, probabilities: {} },
        },
      });
    },
  });
  const base = server.url.href.replace(/\/$/, "");
  const policy = new Policy({ url: base, token: "t", jev: new Jev("k", "jev-test", base), onStalled: () => {} });
  const decision = await policy.tick();
  server.stop(true);
  // No set_plan at all: the registered intent alone is enough to ask.
  expect(Object.keys(asked?.questions.next?.criteria ?? {})).toEqual(["pull_wolf", "wait"]);
  expect(asked?.questions.decision_7?.criteria).toEqual({ fight: "Stay", flee: "Run" });
  expect(lua).toContain('return Agent.resolve(7, "flee")');
  expect(lua).toContain('return Agent.run("pull_wolf")');
  expect(decision?.ran).toBe(true);
});
