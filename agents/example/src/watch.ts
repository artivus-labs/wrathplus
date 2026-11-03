// The event watcher: long-polls the client's control port and turns the game events a player
// would notice (a death, a whisper, an invite) into messages for the planner, batched so a burst
// becomes one message.

export interface GameEvent {
  seq: number;
  time: number;
  event: string;
  args: unknown[];
}

/** Events worth interrupting the planner for, each with how to say it. */
const NOTABLE: Record<string, (args: unknown[]) => string | null> = {
  PLAYER_DEAD: () => "You died.",
  PLAYER_ALIVE: () => "You are alive again (released or resurrected).",
  PLAYER_LEVEL_UP: (a) =>
    `You reached level ${a[0]}. New class skills may be waiting at your class trainer (AgentFindNpc("classtrainer")); Upkeep learns what you can afford when you open the trainer.`,
  CHAT_MSG_WHISPER: (a) => `${a[1]} whispers: ${a[0]}`,
  CHAT_MSG_PARTY: (a) => `[Party] ${a[1]}: ${a[0]}`,
  CHAT_MSG_PARTY_LEADER: (a) => `[Party leader] ${a[1]}: ${a[0]}`,
  CHAT_MSG_TEXT_EMOTE: (a) => `${a[0]} (an emote; answer with DoEmote("wave", "${a[1]}") or in chat)`,
  CHAT_MSG_EMOTE: (a) => `${a[1]} ${a[0]} (an emote)`,
  CHAT_MSG_RAID: (a) => `[Raid] ${a[1]}: ${a[0]}`,
  CHAT_MSG_RAID_LEADER: (a) => `[Raid leader] ${a[1]}: ${a[0]}`,
  CHAT_MSG_SAY: (a) => (a[1] === undefined ? null : `${a[1]} says: ${a[0]}`),
  CHAT_MSG_YELL: (a) => (a[1] === undefined ? null : `${a[1]} yells: ${a[0]}`),
  CHAT_MSG_MONSTER_SAY: (a) => `${a[1]} says: ${a[0]}`,
  CHAT_MSG_MONSTER_YELL: (a) => `${a[1]} yells: ${a[0]}`,
  CHAT_MSG_MONSTER_WHISPER: (a) => `${a[1]} whispers: ${a[0]}`,
  PARTY_INVITE_REQUEST: (a) => `${a[0]} invites you to a party (AcceptGroup() or DeclineGroup()).`,
  TRADE_SHOW: () => "Someone opened a trade window with you.",
  DUEL_REQUESTED: (a) => `${a[0]} challenges you to a duel.`,
  QUEST_COMPLETE: () => "A quest turn-in window is open.",
  UI_ERROR_MESSAGE: (a) => `Game error: ${a[0]}`,
  AGENT_LUA: (a) => (a[1] === false ? `Your Lua failed: ${a[2]} (in: ${String(a[0]).slice(0, 120)})` : null),
  AGENT_WALK_DONE: (a) =>
    a[0] === "stuck" || a[0] === "failed"
      ? `Your walk ended ${a[0]}: AgentWalk().note says why; route around it or pick another way.`
      : null,
  PLAYER_CAMPING: () => "The client is logging out (20 s): CancelLogout() to stay in the world.",
};

/** What wakes the planner: things only it should handle. The rest (combat, loot, quest windows,
 *  errors, level-ups) is the fast tier's, and reaches the planner through it. */
export const PLANNER_EVENTS = new Set([
  "PLAYER_DEAD",
  "CHAT_MSG_WHISPER",
  "CHAT_MSG_PARTY",
  "CHAT_MSG_PARTY_LEADER",
  "PARTY_INVITE_REQUEST",
  "PLAYER_LEVEL_UP",
  "CHAT_MSG_SAY",
  "CHAT_MSG_YELL",
  "CHAT_MSG_MONSTER_WHISPER",
  "CHAT_MSG_TEXT_EMOTE",
  "CHAT_MSG_EMOTE",
  "CHAT_MSG_RAID",
  "CHAT_MSG_RAID_LEADER",
  "TRADE_SHOW",
  "DUEL_REQUESTED",
  "AGENT_WALK_DONE",
  "PLAYER_CAMPING",
]);

/** Game errors that keep coming back (the same red line `times` times within `windowMs`): the
 *  planner hears each once per `quietMs`, with the count. One error is noise; a stream of
 *  "You are too far away!" means its Lua is doing something wrong. */
export class RepeatedErrors {
  #seen = new Map<string, number[]>();
  #told = new Map<string, number>();
  constructor(
    readonly times = 3,
    readonly windowMs = 15_000,
    readonly quietMs = 60_000,
  ) {}

  notice(events: GameEvent[], now = Date.now()): string[] {
    const out: string[] = [];
    for (const e of events) {
      if (e.event !== "UI_ERROR_MESSAGE" || typeof e.args[0] !== "string") continue;
      const text = e.args[0];
      const recent = (this.#seen.get(text) ?? []).filter((t) => now - t < this.windowMs);
      recent.push(now);
      this.#seen.set(text, recent);
      if (recent.length >= this.times && now - (this.#told.get(text) ?? -Infinity) >= this.quietMs) {
        this.#told.set(text, now);
        out.push(`Game error, ${recent.length} times in ${Math.round(this.windowMs / 1000)}s: "${text}". Your Lua keeps doing something the game refuses; fix it.`);
      }
    }
    return out;
  }
}

/** Turn a batch of raw events into lines, skipping the ones a player wouldn't notice. */
/** Speech that is not to us: other players' /say and /yell, NPC chatter. */
const NEARBY = new Set(["CHAT_MSG_SAY", "CHAT_MSG_YELL", "CHAT_MSG_MONSTER_SAY", "CHAT_MSG_MONSTER_YELL"]);

/** /say or /yell meant for us: from the party, or naming us. */
function addressed(e: GameEvent, self?: string, party?: Set<string>): boolean {
  const sender = String(e.args[1] ?? "");
  const mentions = self !== undefined && String(e.args[0]).toLowerCase().includes(self.toLowerCase());
  return mentions || (party?.has(sender.toLowerCase()) ?? false);
}

/** An NPC talking to us by name ("City folk never understand that, Agenttwo."). */
function npcToUs(e: GameEvent, self?: string): boolean {
  return (e.event === "CHAT_MSG_MONSTER_SAY" || e.event === "CHAT_MSG_MONSTER_YELL")
    && self !== undefined && String(e.args[0]).toLowerCase().includes(self.toLowerCase());
}

/** What was said nearby but not to us, to pass on with the next wake-up rather than wake for.
 *  With `npcReplies`, an NPC naming us is not overheard: it wakes the planner (describe). */
export function overheard(events: GameEvent[], self?: string, party?: Set<string>, npcReplies = true): string[] {
  const lines: string[] = [];
  for (const e of events) {
    if (!NEARBY.has(e.event) || (self && e.args[1] === self)) continue;
    if ((e.event === "CHAT_MSG_SAY" || e.event === "CHAT_MSG_YELL") && addressed(e, self, party)) continue;
    if (npcReplies && npcToUs(e, self)) continue;
    const line = NOTABLE[e.event]?.(e.args);
    if (line) lines.push(line);
  }
  return lines;
}

export function describe(events: GameEvent[], self?: string, kinds?: Set<string>, party?: Set<string>, npcReplies = true): string[] {
  const lines: string[] = [];
  for (const e of events) {
    // An NPC that names us may get an answer, for flavour.
    if (kinds && npcReplies && npcToUs(e, self)) {
      lines.push(`${e.args[1]} says to you: ${e.args[0]} (an NPC; a reply in /say is welcome, not required)`);
      continue;
    }
    // Your own chat comes back too; it isn't news.
    if (self && e.event.startsWith("CHAT_MSG_") && e.args[1] === self) continue;
    if (kinds && !kinds.has(e.event)) continue;
    // Said nearby: the planner is woken only when it is meant for it, by the party or by name (bots
    // chatter in /say); the rest is overheard.
    if (kinds && (e.event === "CHAT_MSG_SAY" || e.event === "CHAT_MSG_YELL") && !addressed(e, self, party)) continue;
    // An emote counts when aimed at us ("Pallytest waves at you.", or our name in a custom one);
    // our own come back as "You wave at ...".
    if (kinds && (e.event === "CHAT_MSG_TEXT_EMOTE" || e.event === "CHAT_MSG_EMOTE")) {
      const text = String(e.args[0]);
      const atUs = /\byou\b/i.test(text) || (self !== undefined && text.toLowerCase().includes(self.toLowerCase()));
      if (!atUs || text.startsWith("You ") || (self && e.args[1] === self)) continue;
    }
    const say = NOTABLE[e.event];
    const line = say?.(e.args);
    if (line) lines.push(line);
  }
  // A repeated error in a burst is one line.
  return lines.filter((line, i) => lines.indexOf(line) === i);
}

export interface WatchOptions {
  url: string;
  token: string;
  /** The character's own name, so its chat is not reported back to it. */
  self?: () => string | undefined;
  /** The party's names (lower case): their /say is meant for us. */
  party?: () => Set<string>;
  /** Speech nearby that is not to us, to hand over with the next wake-up. */
  onOverheard?: (lines: string[]) => void;
  /** Wake the planner when an NPC addresses the character by name (default true). */
  npcReplies?: boolean;
  /** Gather lines for this long after the first before sending (ms). */
  batchMs?: number;
  onMessage: (text: string) => void;
  /** Every batch of raw events, for the fast tier. */
  onEvents?: (events: GameEvent[]) => void;
  signal: AbortSignal;
}

/** Poll until aborted. Network errors back off and retry: the client may be restarting. */
export async function watch(options: WatchOptions): Promise<void> {
  const { url, token, signal, onMessage } = options;
  const batchMs = options.batchMs ?? 3000;
  let since = 0;
  let pending: string[] = [];
  let flushAt = 0;
  const errors = new RepeatedErrors();
  // Start from now: old events are not news.
  try {
    const head = await read(url, token, 1_000_000_000, 0, signal);
    since = head.next;
  } catch {
    // The first long-poll below starts from 0 instead.
  }
  while (!signal.aborted) {
    try {
      const wait = pending.length ? Math.max(0, flushAt - Date.now()) : 25_000;
      const batch = await read(url, token, since, wait, signal);
      since = batch.next;
      if (batch.events.length) options.onEvents?.(batch.events);
      const party = options.party?.();
      const npc = options.npcReplies ?? true;
      const lines = [...describe(batch.events, options.self?.(), PLANNER_EVENTS, party, npc), ...errors.notice(batch.events)];
      const heard = overheard(batch.events, options.self?.(), party, npc);
      if (heard.length) options.onOverheard?.(heard);
      if (lines.length && !pending.length) flushAt = Date.now() + batchMs;
      pending.push(...lines);
      if (batch.missed) pending.push(`(${batch.missed} game events were missed.)`);
      if (pending.length && Date.now() >= flushAt) {
        onMessage(`Game events:\n- ${pending.join("\n- ")}`);
        pending = [];
      }
    } catch (error) {
      if (signal.aborted) return;
      await Bun.sleep(2000);
    }
  }
}

async function read(url: string, token: string, since: number, wait: number, signal: AbortSignal) {
  const response = await fetch(`${url}/events?since=${since}&wait_ms=${Math.round(wait)}&limit=2000`, {
    headers: { Authorization: `Bearer ${token}` },
    signal,
  });
  if (!response.ok) throw new Error(`events: HTTP ${response.status}`);
  return (await response.json()) as { events: GameEvent[]; next: number; missed: number };
}
