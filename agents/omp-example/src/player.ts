#!/usr/bin/env bun
// The launcher: one long-lived OMP session (RPC mode) plays the character through the WrathPlus
// MCP server, and is woken by the game. It long-polls the client's event stream for what needs the
// planner (an Agent.ask, a death, a whisper, an invite, a level-up), prompts the session with it,
// checks in when nothing has happened for a while, has a reflector subagent review the scorecard and
// ledger every few hours, and starts a fresh session now and then (never mid-trial). Each model
// call's usage goes to usage.jsonl.

import { appendFileSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { AgentClient } from "../../mcp/src/client.ts";
import { digestLines, flushNotices, NOTICED, NoticeBatch, passedLua } from "./notice.ts";
import { type Frame, OmpRpc } from "./rpc.ts";
import { ompCli, ompEnv, prepareWorkDir, type Settings, settingsFromEnv, trialRunning } from "./setup.ts";
import { appendJsonl, totals, usageFromFrame } from "./usage.ts";
import { classify, Evidence, type GameEvent, Inbox, Repeats, statusLine, WAKE_EVENTS, type WakeReason, wakeMessage } from "./wake.ts";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;

class Player {
  readonly client: AgentClient;
  readonly inbox = new Inbox();
  readonly evidence = new Evidence();
  readonly repeats: Repeats;
  readonly notices: NoticeBatch;
  /** Notice.passed's cursor: where the next check-in's digest reads from. */
  noticeCursor = 0;
  readonly usagePath: string;
  readonly logPath: string;
  rpc: OmpRpc | undefined;
  session: string | undefined;
  self: string | undefined;
  busy = false;
  turnStarted = 0;
  aborting = false;
  lastCheckin = Date.now();
  lastReflect = Date.now();
  sessionStarted = Date.now();
  pendingPrompt: string | undefined;
  stopping = false;
  /** Continue the last session; off for a first start asked to be fresh. */
  resume: boolean;

  constructor(readonly s: Settings) {
    this.client = new AgentClient(s.agentUrl, s.token);
    this.repeats = new Repeats(s.repeatMinutes * MINUTE);
    this.notices = new NoticeBatch(s.noticeSeconds * 1000);
    this.usagePath = join(s.stateDir, "usage.jsonl");
    this.logPath = join(s.stateDir, "launcher.log");
    mkdirSync(s.stateDir, { recursive: true, mode: 0o700 });
    this.resume = !s.fresh;
  }

  log(line: string): void {
    const text = `${new Date().toISOString()} ${line}`;
    console.log(text);
    appendFileSync(this.logPath, `${text}\n`);
  }

  // -- OMP ---------------------------------------------------------------------------------------

  async startOmp(): Promise<void> {
    prepareWorkDir(this.s);
    const cmd = [
      process.execPath,
      ompCli(this.s.exampleDir),
      "--mode",
      "rpc",
      "--no-extensions",
      "--no-title",
      "--model",
      this.s.model,
      "--thinking",
      this.s.thinking,
      ...(this.resume ? ["--continue"] : []),
      ...(this.s.maxCost ? ["--max-cost", String(this.s.maxCost)] : []),
    ];
    const rpc = new OmpRpc({
      cmd,
      cwd: this.s.workDir,
      env: ompEnv(this.s, process.env),
      onFrame: (f) => this.onFrame(f),
      onLog: (line) => this.log(`omp ${line}`),
    });
    this.rpc = rpc;
    await rpc.start();
    await rpc.send({ type: "set_subagent_subscription", level: "events" });
    await rpc.send({
      type: "set_event_filter",
      events: ["agent_start", "agent_end", "message_end", "auto_compaction_end", "notice", "retry_fallback_applied"],
    });
    const state = (await rpc.send({ type: "get_state" })).data as Record<string, unknown> | undefined;
    this.session = typeof state?.sessionId === "string" ? state.sessionId : undefined;
    const messages = typeof state?.messageCount === "number" ? state.messageCount : 0;
    this.log(`omp ready: session ${this.session ?? "?"} (${messages} messages), model ${this.s.model}`);
    this.pendingPrompt = await this.message(messages > 0 ? "restart" : "start", []);
    this.resume = true;
    void rpc.exited.then((code) => this.onExit(code));
  }

  async onExit(code: number): Promise<void> {
    this.busy = false;
    this.rpc = undefined;
    if (this.stopping) return;
    this.log(`omp exited (${code}); restarting in 15 s`);
    await Bun.sleep(15_000);
    try {
      await this.startOmp();
    } catch (error) {
      this.log(`omp restart failed: ${(error as Error).message}`);
      void this.onExit(-1);
    }
  }

  onFrame(f: Frame): void {
    const usage = usageFromFrame(f, this.session);
    if (usage) appendJsonl(this.usagePath, usage);
    switch (f.type) {
      case "message_end":
        this.logMessage(f.message);
        break;
      case "prompt_result":
        this.busy = false;
        this.aborting = false;
        this.log(`turn ended: ${String(f.status)}${f.error ? ` ${JSON.stringify(f.error).slice(0, 300)}` : ""} (${Math.round((Date.now() - this.turnStarted) / 1000)} s)`);
        void this.sessionTotals();
        break;
      case "auto_compaction_end":
      case "notice":
      case "retry_fallback_applied":
        this.log(`omp ${f.type}: ${JSON.stringify(f).slice(0, 300)}`);
        break;
      default:
        break;
    }
  }

  logMessage(message: unknown): void {
    const m = (message ?? {}) as { role?: string; content?: unknown };
    if (m.role !== "assistant" || !Array.isArray(m.content)) return;
    for (const part of m.content as Record<string, unknown>[]) {
      if (part.type === "text" && typeof part.text === "string" && part.text.trim()) this.log(`says: ${part.text.trim().slice(0, 400)}`);
      if (part.type === "toolCall") this.log(`calls ${String(part.name)} ${JSON.stringify(part.arguments ?? {}).slice(0, 200)}`);
    }
  }

  /** The session's running totals (subagents included), as OMP counts them. */
  async sessionTotals(): Promise<void> {
    try {
      const stats = (await this.rpc?.send({ type: "get_session_stats" }, 10_000))?.data;
      if (stats) appendJsonl(this.usagePath, { ts: new Date().toISOString(), kind: "session_stats", session: this.session, stats });
    } catch (error) {
      this.log(`get_session_stats: ${(error as Error).message}`);
    }
  }

  async prompt(message: string): Promise<void> {
    const rpc = this.rpc;
    if (!rpc) return;
    this.busy = true;
    this.turnStarted = Date.now();
    this.log(`prompt: ${message.split("\n")[0]?.slice(0, 200)}`);
    try {
      const r = await rpc.send({ type: "prompt", message });
      if ((r.data as { agentInvoked?: boolean } | undefined)?.agentInvoked === false) this.busy = false;
    } catch (error) {
      this.busy = false;
      this.log(`prompt failed: ${(error as Error).message}`);
    }
  }

  async message(reason: WakeReason, lines: string[]): Promise<string> {
    let status = "Client status unknown (the control port did not answer).";
    try {
      const raw = (await this.client.status()) as { player?: { name?: string } | null };
      if (raw.player?.name) this.self = raw.player.name;
      status = statusLine(raw);
    } catch {
      // The client may be restarting; the planner will see that itself.
    }
    const evidence =
      reason === "checkin" || reason === "reflect" ? [...this.repeats.take(), ...this.evidence.take(), ...(await this.passed())] : [];
    return wakeMessage(reason, lines, status, this.s.checkinMinutes, evidence);
  }

  // -- The game's events -------------------------------------------------------------------------

  async pollEvents(): Promise<void> {
    let since: number | undefined;
    while (!this.stopping) {
      try {
        const read = (await this.client.events({
          since,
          names: [...WAKE_EVENTS],
          wait_ms: 25_000,
          limit: 100,
        })) as { events: GameEvent[]; next: number };
        // A cursor that went backwards is a restarted client: start again from now.
        if (since !== undefined && read.next < since) {
          since = undefined;
          continue;
        }
        since = read.next;
        for (const e of read.events) {
          this.evidence.add(e);
          // What the character passed waits for its batch: one wake per window, not one each.
          if (e.event === NOTICED) {
            this.notices.add(e, Date.now());
            continue;
          }
          // A notable event repeating a recent wake (the same goal blocked again) is only counted.
          if (this.repeats.hold(e, classify(e, this.self), Date.now())) {
            this.log(`event ${e.event} (repeat, counted for the check-in)`);
            continue;
          }
          const urgency = this.inbox.add(e, this.self);
          if (urgency) this.log(`event ${e.event} (${urgency})`);
        }
      } catch (error) {
        since = undefined;
        this.log(`events: ${(error as Error).message.slice(0, 200)}`);
        await Bun.sleep(10_000);
      }
    }
  }

  /** Every notice since the last check-in, muted or not, counted by kind (the client keeps them:
   *  Notice.passed), for the check-in's evidence. */
  async passed(): Promise<string[]> {
    try {
      const { lines, next } = digestLines(await this.client.lua(passedLua(this.noticeCursor)));
      if (next !== undefined) this.noticeCursor = next;
      return lines;
    } catch {
      return [];
    }
  }

  // -- The schedule ------------------------------------------------------------------------------

  /** Whether the judge is mid-trial, from its ledger on disk (where the MCP server keeps it). */
  async ledgerBusy(): Promise<boolean> {
    try {
      let dir = this.s.judgeDir;
      if (!dir) {
        const listing = (await this.client.modules()) as { folder?: unknown };
        if (typeof listing.folder !== "string") return true;
        // The MCP server's default: `judge` beside the modules folder (agents/mcp/src/monitor.ts).
        dir = join(listing.folder, "..", "judge");
      }
      const path = join(dir, "ledger.jsonl");
      return existsSync(path) && trialRunning(readFileSync(path, "utf8"));
    } catch {
      return true;
    }
  }

  async tick(): Promise<void> {
    const rpc = this.rpc;
    if (!rpc) return;
    const now = Date.now();
    if (this.busy) {
      if (this.s.turnMaxMinutes > 0 && !this.aborting && now - this.turnStarted > this.s.turnMaxMinutes * MINUTE) {
        this.aborting = true;
        this.log(`turn over ${this.s.turnMaxMinutes} min: aborting it`);
        await rpc.send({ type: "abort" }).catch((e: Error) => this.log(`abort: ${e.message}`));
      } else if (this.inbox.has("urgent")) {
        const message = await this.message("events", this.inbox.take());
        this.log("steering the running turn with urgent events");
        await rpc.send({ type: "steer", message }).catch((e: Error) => this.log(`steer: ${e.message}`));
      }
      return;
    }
    const noticed = flushNotices(this.notices, this.inbox, now);
    if (noticed) this.log(`noticed ${noticed} (batched)`);
    if (this.pendingPrompt) {
      const message = this.pendingPrompt;
      this.pendingPrompt = undefined;
      return this.prompt(message);
    }
    if (this.inbox.has("urgent") || this.inbox.has("notable")) return this.prompt(await this.message("events", this.inbox.take()));
    if (this.s.reflectHours > 0 && now - this.lastReflect > this.s.reflectHours * HOUR) {
      this.lastReflect = now;
      return this.prompt(await this.message("reflect", this.inbox.take()));
    }
    if (this.s.sessionHours > 0 && now - this.sessionStarted > this.s.sessionHours * HOUR && !(await this.ledgerBusy())) {
      this.sessionStarted = now;
      this.log("starting a fresh session");
      try {
        await rpc.send({ type: "new_session" });
        const state = (await rpc.send({ type: "get_state" })).data as Record<string, unknown> | undefined;
        this.session = typeof state?.sessionId === "string" ? state.sessionId : this.session;
      } catch (error) {
        this.log(`new_session: ${(error as Error).message}`);
      }
      return this.prompt(await this.message("start", this.inbox.take()));
    }
    // Every checkinMinutes, whatever else woke it meanwhile: the turn for improving the controller.
    if (this.s.checkinMinutes > 0 && now - this.lastCheckin > this.s.checkinMinutes * MINUTE) {
      const message = await this.message("checkin", this.inbox.take());
      this.lastCheckin = now;
      return this.prompt(message);
    }
  }

  async run(): Promise<void> {
    this.log(`launcher: state ${this.s.stateDir}, client ${this.s.agentUrl}`);
    const before = totals(this.usagePath);
    this.log(`usage so far: ${before.calls} calls, $${before.cost.toFixed(2)}`);
    await this.startOmp();
    void this.pollEvents();
    const stop = async () => {
      this.stopping = true;
      this.log("stopping");
      await this.rpc?.stop();
      process.exit(0);
    };
    process.on("SIGINT", stop);
    process.on("SIGTERM", stop);
    process.on("SIGHUP", stop);
    for (;;) {
      try {
        await this.tick();
      } catch (error) {
        this.log(`tick: ${(error as Error).message}`);
      }
      await Bun.sleep(5_000);
    }
  }
}

if (import.meta.main) {
  const [major = 0, minor = 0, patch = 0] = Bun.version.split(".").map(Number);
  if (major * 1e6 + minor * 1e3 + patch < 1_004_004) {
    console.error(`omp needs Bun 1.4.4 or later; this is ${Bun.version}. Run through ./run.sh, which uses this example's own Bun.`);
    process.exit(1);
  }
  let settings: Settings;
  try {
    settings = settingsFromEnv(process.env);
  } catch (error) {
    console.error(`wrathplus-omp-agent: ${(error as Error).message}`);
    process.exit(1);
  }
  await new Player(settings).run();
}
