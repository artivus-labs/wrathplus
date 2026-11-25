// What wakes the planner, and the message it wakes to. The launcher long-polls the client's event
// stream for these names only; everything else (combat, loot, walks) is the Lua controller's
// business, which the planner reads itself with the `events` tool when it wants to.

export interface GameEvent {
  seq: number;
  time: number;
  event: string;
  args: unknown[];
}

/** urgent: deliver now, steering a turn that is running. notable: deliver when the planner is
 *  idle. Failures never wake it: they go to the check-ins' Evidence. */
export type Urgency = "urgent" | "notable";

/** The events the launcher asks the client for. Chat in /say is only kept when it names us. */
export const WAKE_EVENTS = [
  "AGENT_ASK",
  "PLAYER_DEAD",
  "PLAYER_LEVEL_UP",
  "CHAT_MSG_WHISPER",
  "CHAT_MSG_PARTY",
  "CHAT_MSG_PARTY_LEADER",
  "CHAT_MSG_SAY",
  "CHAT_MSG_YELL",
  "CHAT_MSG_EMOTE",
  "CHAT_MSG_TEXT_EMOTE",
  "PARTY_INVITE_REQUEST",
  "TRADE_SHOW",
  "DUEL_REQUESTED",
  "AGENT_GOAL_BLOCKED",
  "AGENT_GOAL_DONE",
  "AGENT_HOLD_EXPIRED",
  // Counted for the check-ins' evidence, never waking the planner by themselves:
  "AGENT_ACTION_FAILED",
  "AGENT_PULL_REFUSED",
  // (but for a fight left "no_progress", which wakes it: see classify)
  "AGENT_COMBAT_DONE",
  "AGENT_COMBAT_REVIEW",
  "UI_ERROR_MESSAGE",
] as const;

const URGENT = new Set([
  "AGENT_ASK",
  "PLAYER_DEAD",
  "CHAT_MSG_WHISPER",
  "CHAT_MSG_PARTY",
  "CHAT_MSG_PARTY_LEADER",
  "PARTY_INVITE_REQUEST",
  "TRADE_SHOW",
  "DUEL_REQUESTED",
]);
const NOTABLE = new Set(["PLAYER_LEVEL_UP", "AGENT_GOAL_BLOCKED", "AGENT_GOAL_DONE", "AGENT_HOLD_EXPIRED"]);
const NEARBY = new Set(["CHAT_MSG_SAY", "CHAT_MSG_YELL", "CHAT_MSG_EMOTE", "CHAT_MSG_TEXT_EMOTE"]);

const str = (v: unknown): string => (v === undefined || v === null ? "" : String(v));
const clip = (s: string, n: number): string => (s.length > n ? `${s.slice(0, n - 1)}…` : s);

/** How much an event matters to the planner, or null when it does not (our own chat, a /say not
 *  meant for us). `self` is the character's name, when known. */
export function classify(e: GameEvent, self?: string): Urgency | null {
  const speaker = str(e.args[1]);
  if (e.event.startsWith("CHAT_MSG_") && self && speaker.toLowerCase() === self.toLowerCase()) return null;
  if (URGENT.has(e.event)) return "urgent";
  // A fight Combat left without progress, while still being hit: whether to fight another, run or
  // retry is the planner's call, and it cannot wait for a check-in.
  if (e.event === "AGENT_COMBAT_DONE" && str(e.args[1]) === "no_progress") return "urgent";
  if (NOTABLE.has(e.event)) return "notable";
  if (NEARBY.has(e.event)) {
    if (!self) return null;
    return str(e.args[0]).toLowerCase().includes(self.toLowerCase()) ? "notable" : null;
  }
  return null;
}

/** One line a person would read for an event. */
export function describe(e: GameEvent): string {
  const a = e.args;
  switch (e.event) {
    case "AGENT_ASK": {
      const context = a[2] === undefined || a[2] === null ? "" : ` Context: ${clip(JSON.stringify(a[2]), 600)}.`;
      const digest = Array.isArray(a[3]) && a[3].length ? ` Recent: ${a[3].slice(-10).map(str).join(" | ")}.` : "";
      return `Your Lua asks (ask ${str(a[0])}): ${str(a[1])}.${context}${digest} Answer with Agent.answer(${str(a[0])}, value) in lua.`;
    }
    case "PLAYER_DEAD":
      return "You died.";
    case "PLAYER_LEVEL_UP":
      return `You reached level ${str(a[0])}.`;
    case "CHAT_MSG_WHISPER":
      return `${str(a[1])} whispers: ${clip(str(a[0]), 300)}`;
    case "CHAT_MSG_PARTY":
    case "CHAT_MSG_PARTY_LEADER":
      return `[Party] ${str(a[1])}: ${clip(str(a[0]), 300)}`;
    case "CHAT_MSG_SAY":
      return `${str(a[1])} says: ${clip(str(a[0]), 300)}`;
    case "CHAT_MSG_YELL":
      return `${str(a[1])} yells: ${clip(str(a[0]), 300)}`;
    case "CHAT_MSG_EMOTE":
      return `${str(a[1])} ${clip(str(a[0]), 200)} (emote)`;
    case "CHAT_MSG_TEXT_EMOTE":
      return `${clip(str(a[0]), 200)} (emote)`;
    case "PARTY_INVITE_REQUEST":
      return `${str(a[0])} invites you to a party (AcceptGroup() or DeclineGroup()).`;
    case "TRADE_SHOW":
      return "Someone opened a trade with you (CancelTrade() to refuse).";
    case "DUEL_REQUESTED":
      return `${str(a[0])} challenges you to a duel (AcceptDuel() or CancelDuel()).`;
    case "AGENT_GOAL_BLOCKED":
      return `Goal ${str(a[0])} is blocked: ${clip(str(a[1]), 300)}`;
    case "AGENT_GOAL_DONE":
      return `Goal ${str(a[0])} ended (${a[1] ? "ok" : "not ok"}).`;
    case "AGENT_HOLD_EXPIRED":
      if (a[2]) return `Your hold "${str(a[0])}" ended with its walk (${str(a[2])}) after ${str(a[1])} s: hold again or Goals.release().`;
      return `Your hold "${str(a[0])}" ran out after ${str(a[1])} s: hold again or Goals.release().`;
    case "AGENT_ACTION_FAILED":
      return `Goal action ${str(a[1])} (goal ${str(a[0])}) failed: ${clip(str(a[2]), 200)}`;
    case "AGENT_COMBAT_DONE":
      if (str(a[1]) === "no_progress") return `Combat left ${str(a[0])} without progress: ${clip(str(a[2]), 400)}`;
      return `${e.event} ${clip(JSON.stringify(a), 200)}`;
    default:
      return `${e.event} ${clip(JSON.stringify(a), 200)}`;
  }
}

/** The pending events, held until they are delivered. Repeats of one line collapse to a count. */
export class Inbox {
  #items: { urgency: Urgency; line: string; count: number }[] = [];

  add(e: GameEvent, self?: string): Urgency | null {
    const urgency = classify(e, self);
    if (!urgency) return null;
    const line = describe(e);
    const same = this.#items.find((i) => i.line === line);
    if (same) same.count += 1;
    else this.#items.push({ urgency, line, count: 1 });
    return urgency;
  }

  get size(): number {
    return this.#items.length;
  }

  has(urgency: Urgency): boolean {
    return this.#items.some((i) => i.urgency === urgency);
  }

  /** The lines, most urgent first, and empty the inbox. */
  take(): string[] {
    const order: Record<Urgency, number> = { urgent: 0, notable: 1 };
    const lines = [...this.#items]
      .sort((a, b) => order[a.urgency] - order[b.urgency])
      .map((i) => (i.count > 1 ? `${i.line} (x${i.count})` : i.line));
    this.#items = [];
    return lines;
  }
}

/** A compact line from the client's /status, for the top of a wake message. */
export function statusLine(status: unknown): string {
  const s = (status ?? {}) as Record<string, unknown>;
  const p = s.player as Record<string, unknown> | null | undefined;
  if (!s.in_world || !p) return `Client: ${str(s.state) || "unknown"}, not in the world.`;
  const where = [p.zone, p.subzone].filter((x) => x).map(str).join(" / ");
  const health = p.health_max ? ` Health ${str(p.health)}/${str(p.health_max)}, power ${str(p.power)}/${str(p.power_max)}.` : "";
  const flags = [p.dead ? "dead" : "", p.combat ? "in combat" : ""].filter((x) => x).join(", ");
  return `${str(p.name)}, level ${str(p.level)} ${str(p.class_name ?? p.class)}, ${where}.${health}${flags ? ` (${flags})` : ""}`;
}

/** What went wrong (and how fights ended) between two check-ins, counted, so a check-in starts
 *  from evidence rather than a glance: the controller's failures as the game and the built-ins
 *  report them. The launcher only counts; what to make of them is the planner's call. */
export class Evidence {
  #counts = new Map<string, number>();
  #outcomes = new Map<string, number>();
  #review: string | undefined;

  add(e: GameEvent): void {
    const a = e.args;
    const bump = (m: Map<string, number>, k: string) => m.set(k, (m.get(k) ?? 0) + 1);
    switch (e.event) {
      case "UI_ERROR_MESSAGE":
        return void bump(this.#counts, `game error "${clip(str(a[0]), 120)}"`);
      case "AGENT_ACTION_FAILED":
        return void bump(this.#counts, `goal action ${str(a[1])} failed: ${clip(str(a[2]), 160)}`);
      case "AGENT_PULL_REFUSED":
        return void bump(this.#counts, `pull refused: ${clip(str(a[1]), 160)}`);
      case "AGENT_COMBAT_DONE":
        return void bump(this.#outcomes, str(a[1]) || "?");
      case "AGENT_COMBAT_REVIEW":
        this.#review = clip(str(a[0]), 600);
        return;
      case "PLAYER_DEAD":
        return void bump(this.#outcomes, "you died");
      default:
        return;
    }
  }

  /** The tally, most frequent first, and start a new one. */
  take(top = 8): string[] {
    const lines = [...this.#counts.entries()]
      .sort((x, y) => y[1] - x[1])
      .slice(0, top)
      .map(([k, n]) => `${n}x ${k}`);
    if (this.#outcomes.size) lines.unshift(`fights: ${[...this.#outcomes.entries()].map(([k, n]) => `${n} ${k}`).join(", ")}`);
    if (this.#review) lines.push(`latest fight review: ${this.#review}`);
    this.#counts.clear();
    this.#outcomes.clear();
    this.#review = undefined;
    return lines;
  }
}

/** What makes a notable event the same as an earlier one: its name and the arguments that say
 *  what it is about (a hold's reason, not how long it waited). */
function repeatKey(e: GameEvent): string {
  const args = e.event === "AGENT_HOLD_EXPIRED" ? e.args.slice(0, 1) : e.args;
  return `${e.event} ${JSON.stringify(args)}`;
}

/** How a repeat is shown in the check-in's evidence. */
function repeatLabel(e: GameEvent): string {
  const a = e.args;
  switch (e.event) {
    case "AGENT_GOAL_BLOCKED":
      return `${e.event} ${str(a[0])}: '${clip(str(a[1]), 120)}'`;
    case "AGENT_HOLD_EXPIRED":
      return `${e.event} '${clip(str(a[0]), 120)}'`;
    default:
      return clip(describe(e), 160);
  }
}

/** Notable events that repeat one that woke the planner within `windowMs` (the same goal blocked
 *  for the same reason, the same hold running out) do not wake it again: they are counted and shown
 *  in the next check-in's evidence. Urgent events are never held back. A window of 0 is off. */
export class Repeats {
  #woke = new Map<string, number>();
  #held = new Map<string, { label: string; count: number }>();

  constructor(readonly windowMs: number) {}

  /** Whether `e` repeats a recent wake and is held back (and counted); else it is recorded as a
   *  wake when notable. */
  hold(e: GameEvent, urgency: Urgency | null, now: number): boolean {
    if (urgency !== "notable" || this.windowMs <= 0) return false;
    const key = repeatKey(e);
    const last = this.#woke.get(key);
    if (last !== undefined && now - last < this.windowMs) {
      const held = this.#held.get(key) ?? { label: repeatLabel(e), count: 0 };
      held.count += 1;
      this.#held.set(key, held);
      return true;
    }
    this.#woke.set(key, now);
    for (const [k, t] of this.#woke) if (now - t >= this.windowMs) this.#woke.delete(k);
    return false;
  }

  /** The repeats held back since the last take, most frequent first, and start a new count. */
  take(): string[] {
    const minutes = Math.round(this.windowMs / 60_000);
    const lines = [...this.#held.values()]
      .sort((x, y) => y.count - x.count)
      .map((h) => `${h.label} ×${h.count} (repeats within ${minutes} min of a wake; not woken for)`);
    this.#held.clear();
    return lines;
  }
}

export type WakeReason = "events" | "checkin" | "start" | "restart" | "reflect";

/** The message the planner wakes to. */
export function wakeMessage(reason: WakeReason, lines: string[], status: string, minutesQuiet?: number, evidence: string[] = []): string {
  const head: Record<WakeReason, string> = {
    start: "A new session starts. Read your notes and skills, check status and the scorecard, then get your controller playing.",
    restart: "The launcher restarted and this session resumes. Check status and your controller, then carry on.",
    events: "Something happened in the game.",
    checkin: `Check-in (every ${minutesQuiet ?? "few"} minutes): your turn to improve the controller. Take the most costly recurring failure in the evidence below, or the scorecard's weakest objective or need. Study it: Goals.log and Goals.list(), Rotation.stats, the module or built-in involved (lua_doc), recent events. Then write or change a module for it (propose_module when it changes how you play), or write in notes/journal.md why you are not changing anything and what you would need to. "No changes" without that note is not an answer. See the improve-the-controller skill.`,
    reflect:
      "Reflection is due. Dispatch the `reflector` agent with the task tool (a fresh context with the same game tools) and wait for its report; then act on it: proposals it made are queued with the judge, notes it wrote are in notes/. Keep your own turn short.",
  };
  const body = lines.length ? `\n\nEvents since your last turn:\n${lines.map((l) => `- ${l}`).join("\n")}` : "";
  const seen =
    reason === "checkin" || reason === "reflect"
      ? `\n\nEvidence since the last check-in:\n${evidence.length ? evidence.map((l) => `- ${l}`).join("\n") : "- (nothing failed, and no fights ended)"}`
      : "";
  return `[launcher ${new Date().toISOString()}] ${head[reason]}\n${status}${body}${seen}`;
}
