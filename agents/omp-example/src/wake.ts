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
 *  idle. minor: ride along with the next wake, or wake alone after a quiet spell. */
export type Urgency = "urgent" | "notable" | "minor";

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
  "AGENT_ACTION_FAILED",
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
  if (NOTABLE.has(e.event)) return "notable";
  if (NEARBY.has(e.event)) {
    if (!self) return null;
    return str(e.args[0]).toLowerCase().includes(self.toLowerCase()) ? "notable" : null;
  }
  if (e.event === "AGENT_ACTION_FAILED") return "minor";
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
      return `Your hold "${str(a[0])}" ran out after ${str(a[1])} s: hold again or Goals.release().`;
    case "AGENT_ACTION_FAILED":
      return `Goal action ${str(a[1])} (goal ${str(a[0])}) failed: ${clip(str(a[2]), 200)}`;
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
    const order: Record<Urgency, number> = { urgent: 0, notable: 1, minor: 2 };
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

export type WakeReason = "events" | "checkin" | "start" | "restart" | "reflect";

/** The message the planner wakes to. */
export function wakeMessage(reason: WakeReason, lines: string[], status: string, minutesQuiet?: number): string {
  const head: Record<WakeReason, string> = {
    start: "A new session starts. Read your notes and skills, check status and the scorecard, then get your controller playing.",
    restart: "The launcher restarted and this session resumes. Check status and your controller, then carry on.",
    events: "Something happened in the game.",
    checkin: `Check-in: nothing has needed you for ${minutesQuiet ?? "a while"} minutes. Glance at status, goals and the scorecard; fix what is off, or improve one thing in your controller, then end your turn.`,
    reflect:
      "Reflection is due. Dispatch the `reflector` agent with the task tool (a fresh context with the same game tools) and wait for its report; then act on it: proposals it made are queued with the judge, notes it wrote are in notes/. Keep your own turn short.",
  };
  const body = lines.length ? `\n\nEvents since your last turn:\n${lines.map((l) => `- ${l}`).join("\n")}` : "";
  return `[launcher ${new Date().toISOString()}] ${head[reason]}\n${status}${body}`;
}
