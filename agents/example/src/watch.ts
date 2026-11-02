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
  PLAYER_LEVEL_UP: (a) => `You reached level ${a[0]}.`,
  CHAT_MSG_WHISPER: (a) => `${a[1]} whispers: ${a[0]}`,
  CHAT_MSG_PARTY: (a) => `[Party] ${a[1]}: ${a[0]}`,
  CHAT_MSG_PARTY_LEADER: (a) => `[Party leader] ${a[1]}: ${a[0]}`,
  CHAT_MSG_RAID: (a) => `[Raid] ${a[1]}: ${a[0]}`,
  CHAT_MSG_RAID_LEADER: (a) => `[Raid leader] ${a[1]}: ${a[0]}`,
  CHAT_MSG_SAY: (a) => (a[1] === undefined ? null : `${a[1]} says: ${a[0]}`),
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
  "CHAT_MSG_SAY",
  "CHAT_MSG_RAID",
  "CHAT_MSG_RAID_LEADER",
  "TRADE_SHOW",
  "DUEL_REQUESTED",
  "AGENT_WALK_DONE",
  "PLAYER_CAMPING",
]);

/** Turn a batch of raw events into lines, skipping the ones a player wouldn't notice. */
export function describe(events: GameEvent[], self?: string, kinds?: Set<string>): string[] {
  const lines: string[] = [];
  for (const e of events) {
    // Your own chat comes back too; it isn't news.
    if (self && e.event.startsWith("CHAT_MSG_") && e.args[1] === self) continue;
    if (kinds && !kinds.has(e.event)) continue;
    // Said nearby: the planner hears it only when it is addressed by name (bots chatter in /say).
    if (kinds && e.event === "CHAT_MSG_SAY" && !(self && String(e.args[0]).toLowerCase().includes(self.toLowerCase()))) continue;
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
      const lines = describe(batch.events, options.self?.(), PLANNER_EVENTS);
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
