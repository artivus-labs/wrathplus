// Requests from the party, sorted by the fast tier before the planner hears them. Jev reads what
// was said (with a little game state) and picks what it asks for: something the built-ins can
// just do (follow, stay, come here, pick up or hand in quests, attack their target), or anything
// else. A clear request is done at once, with a few words back in the same channel; the planner
// hears about it with its next wake-up instead of being woken. Questions, chat and anything
// needing thought go to the planner as before, and so does a request Jev is unsure of.
//
// Small talk (`SmallTalk`, below) is sorted the same way: an NPC, a bot or a stranger saying
// something to the character in passing is answered with a courtesy or ignored, not given a
// planner turn each. A remark or greeting that a spoken reply fits better is answered with one
// short line in character (`say`, written by Claude Haiku in `say.ts`), within its own caps.

import { choice, noul, type Jev } from "./jev.ts";
import { CHARACTER_LUA, memorySlice, sayLine, sayPrompt, type Character, type Speak } from "./say.ts";

export interface Request {
  sender: string;
  text: string;
  /** The chat type to answer in: "PARTY" or "WHISPER". */
  channel: "PARTY" | "WHISPER";
}

export interface Triaged {
  /** Done by the fast tier, so the planner need not be woken for it. */
  handled: boolean;
  /** For the planner: what was asked and what was done about it. */
  note: string;
  /** The message also wants a spoken answer only the planner can give. */
  reply: boolean;
}

interface Action {
  when: string;
  /** Lua that does it, given the sender's name; returns true, or false and why. */
  lua: (sender: string) => string;
  ack: string;
  done: string;
}

const q = (s: string) => JSON.stringify(s);

/** What a request can be, with the Lua that carries it out. */
export const ACTIONS: Record<string, Action> = {
  follow_me: {
    when: "They want you to follow them, come along with them, or keep up.",
    lua: (s) => `Move.companion = ${q(s)} if Agent.intents.follow_companion then return Agent.run("follow_companion") end return Move.follow(${q(s)})`,
    ack: "Following.",
    done: "followed them",
  },
  stay: {
    when: "They want you to wait, stay or hold where you are.",
    lua: () => `if Agent.intents.stay_here then return Agent.run("stay_here") end return Move.stay()`,
    ack: "Waiting here.",
    done: "is staying put",
  },
  come_here: {
    when: "They want you to come to them now (once), not to follow along.",
    lua: (s) => `return Move.to(${q(s)})`,
    ack: "On my way.",
    done: "walked over to them",
  },
  pick_up_quests: {
    when: "They want you to get, pick up or accept the quest or quests from the quest giver here.",
    lua: () => `if not Agent.intents.pick_up_quests or not Agent.available then return false, "no quest giver nearby has a quest for us" end
      for _, i in ipairs(Agent.available()) do if i.name == "pick_up_quests" then return Agent.run("pick_up_quests") end end
      return false, "no quest giver nearby has a quest for us"`,
    ack: "Getting the quest.",
    done: "is picking up the quests on offer nearby",
  },
  hand_in_quests: {
    when: "They want you to turn in or hand in finished quests here.",
    lua: () => `for _, i in ipairs(Agent.available()) do if i.name == "hand_in_quests" then return Agent.run("hand_in_quests") end end
      return false, "no quest giver nearby takes a finished quest of ours"`,
    ack: "Handing it in.",
    done: "is handing in its finished quests nearby",
  },
  assist: {
    when: "They want you to attack, help with or kill what they are fighting or targeting.",
    lua: (s) => `local me for _, u in ipairs(AgentNearby(60)) do if u.name == ${q(s)} then me = u end end
      if not me or not me.target then return false, "they have no target I can see" end
      return Combat.engage(me.target, { force = true })`,
    ack: "On it.",
    done: "attacked their target",
  },
};

const OTHER = {
  name: "something_else",
  when: "Anything else: a question, conversation, a plan to discuss, a request none of the others covers, or one that needs thought.",
};

export class Triage {
  calls = 0;

  constructor(
    private readonly jev: Jev,
    private readonly lua: (code: string) => Promise<{ ok: boolean; values?: unknown[]; error?: string }>,
    private readonly say: (text: string, channel: string, to: string) => Promise<void>,
    /** How sure Jev must be to act without the planner. */
    private readonly confidence = 0.75,
  ) {}

  async handle(request: Request, game: unknown, plan?: string): Promise<Triaged> {
    const ask = `${request.sender} said "${request.text}"`;
    const criteria: Record<string, string> = { [OTHER.name]: OTHER.when };
    for (const [name, action] of Object.entries(ACTIONS)) criteria[name] = action.when;
    let response;
    try {
      response = await this.jev.ask(
        { message: { from: request.sender, text: request.text, channel: request.channel }, game, plan },
        {
          wants: choice(
            "A player in the game agent's party sent it this message. What does the message ask the agent to do?",
            criteria,
          ),
          reply: noul("Does the message ask a question or say something that deserves a spoken answer, beyond just doing what it asks?"),
        },
      );
      this.calls++;
    } catch (error) {
      return { handled: false, note: `${ask} (the fast loop could not sort it: ${String(error).slice(0, 80)})`, reply: true };
    }
    const wants = response.answers.wants;
    const reply = response.answers.reply;
    const wantsReply = reply?.type === "noul" ? reply.noul >= 0.5 : true;
    if (wants?.type !== "choice" || wants.choice === OTHER.name || wants.confidence < this.confidence) {
      return { handled: false, note: ask, reply: true };
    }
    const action = ACTIONS[wants.choice];
    if (!action) return { handled: false, note: ask, reply: true };
    const result = await this.lua(action.lua(request.sender));
    const ok = result.ok && result.values?.[0] !== false;
    if (!ok) {
      const why = result.error ?? String(result.values?.[1] ?? "it did not start");
      return { handled: false, note: `${ask}; the fast loop tried to (${wants.choice}) but ${why}`, reply: true };
    }
    if (!wantsReply) await this.say(action.ack, request.channel, request.sender);
    return { handled: !wantsReply, note: `${ask}; the fast loop ${action.done}`, reply: wantsReply };
  }
}

/** Something said to the character in passing: an NPC naming it, a line in /say or /yell with its
 *  name from someone outside the party (the realm's bots chatter this way), an emote aimed at it. */
export interface Chatter {
  speaker: string;
  text: string;
  kind: "npc" | "say" | "emote";
}

/** How it reads in a message to the planner. */
export function chatterLine(c: Chatter): string {
  if (c.kind === "emote") return c.text;
  return `${c.speaker} says to you: ${c.text}${c.kind === "npc" ? " (an NPC)" : ""}`;
}

interface Courtesy {
  when: string;
  lua: (speaker: string) => string;
  did: string;
}

const emote = (token: string, did: string, when: string): Courtesy => ({
  when,
  lua: (s) => `DoEmote(${q(token)}, ${q(s)}) return true`,
  did,
});
const line = (text: string, when: string): Courtesy => ({
  when,
  lua: () => `SendChatMessage(${q(text)}, "SAY") return true`,
  did: `said "${text}"`,
});

/** What the fast tier can answer small talk with by itself: Jev picks, it writes nothing. */
export const COURTESIES: Record<string, Courtesy> = {
  wave: emote("WAVE", "waved", "A greeting or a goodbye: wave back."),
  nod: emote("NOD", "nodded", "A remark, an opinion or a piece of advice that wants no more than acknowledgement: nod."),
  laugh: emote("LAUGH", "laughed", "A joke or something funny: laugh."),
  bow: emote("BOW", "bowed", "A bow, a salute or formal respect: bow back."),
  thank: emote("THANK", "thanked them", "A compliment, good wishes, a blessing or help offered in passing: thank them."),
  greet: line("Hello there.", "They greet the agent in words and a spoken greeting back fits better than a wave."),
};

/** The say step's criterion: offered only with a writer and while its caps allow. */
export const SAY_WHEN =
  "A remark or a greeting that a short spoken reply in character fits better than an emote: someone talking to the agent, wanting a few friendly words back, not a decision.";

const GREETING = /\b(hi|hello|hey|greetings|well met|good (?:morning|day|evening)|bye|goodbye|farewell)\b/i;

const IGNORE = "Idle chatter that wants no answer (a remark to nobody in particular, a complaint, a rumour, a repeat of what was just said), or an answer was given moments ago.";
const NEEDS_THOUGHT =
  "A real request, question, invitation, offer or warning that deserves a proper answer or a decision: an invitation to group, a trade or help offered or asked for, a question about what the agent is doing, directions that matter to its quests.";

export interface SmallTalkOptions {
  /** The fast tier: with it, Jev answers or ignores small talk itself. */
  jev?: Jev;
  lua?: (code: string) => Promise<{ ok: boolean; values?: unknown[]; error?: string }>;
  now?: () => number;
  /** Without the fast tier, small talk wakes the planner at most this often (ms, default 10 min);
   *  the rest rides along with its next wake-up. */
  wakeEveryMs?: number;
  /** One answer to a speaker, then none for this long (ms, default 2 min). */
  perSpeakerMs?: number;
  /** How sure Jev must be to act (or to wake the planner) rather than let the line ride along. */
  confidence?: number;
  /** The say step's writer (`haikuSpeak`); without it, or without Jev, there is no say step. */
  speak?: Speak;
  /** MEMORY.md, whose tail flavours a said line. */
  memory?: () => string;
  /** At most one say per this long (ms, default 30 s), and `sayPerHour` (default 20) an hour. */
  sayGapMs?: number;
  sayPerHour?: number;
  /** A line not written within this long is given up for an emote (ms, default 10 s). */
  sayTimeoutMs?: number;
}

/** What became of a batch of small talk. */
export interface Sorted {
  /** Lines that wake the planner now. */
  wake: string[];
  /** Lines for its next wake-up, with what the fast tier did about each. */
  later: string[];
}

/** Small talk never costs a planner turn each: with the fast tier Jev waves, nods, says hello or
 *  ignores it (and passes on what really needs the planner); without it, the lines are batched
 *  into the next wake-up, and wake the planner by themselves at most once per `wakeEveryMs`. */
export class SmallTalk {
  calls = 0;
  #answered = new Map<string, number>();
  #wokeAt = -Infinity;
  #said: number[] = [];
  #chat: string[] = [];

  constructor(readonly options: SmallTalkOptions = {}) {}

  #now(): number {
    return this.options.now?.() ?? Date.now();
  }

  async handle(items: Chatter[], game?: unknown): Promise<Sorted> {
    const sorted: Sorted = { wake: [], later: [] };
    const unsorted: Chatter[] = [];
    for (const item of items) {
      if (!this.options.jev || !this.options.lua) unsorted.push(item);
      else if (!(await this.#sort(item, game, sorted))) unsorted.push(item);
      // The last few lines, for the next say's context.
      this.#chat = [...this.#chat, chatterLine(item)].slice(-6);
    }
    if (!unsorted.length) return sorted;
    // Nobody to sort these: one wake-up per `wakeEveryMs` at most, the rest ride along.
    const now = this.#now();
    if (now - this.#wokeAt >= (this.options.wakeEveryMs ?? 600_000)) {
      this.#wokeAt = now;
      sorted.wake.push(...unsorted.map((c) => `${chatterLine(c)} (small talk: a short reply or an emote is welcome, not required)`));
    } else {
      sorted.later.push(...unsorted.map(chatterLine));
    }
    return sorted;
  }

  /** Let Jev sort one line; false when it could not (the caller batches it). */
  async #sort(item: Chatter, game: unknown, sorted: Sorted): Promise<boolean> {
    const jev = this.options.jev!;
    const lua = this.options.lua!;
    const now = this.#now();
    const text = chatterLine(item);
    const recently = now - (this.#answered.get(item.speaker) ?? -Infinity) < (this.options.perSpeakerMs ?? 120_000);
    const criteria: Record<string, string> = { ignore: IGNORE, needs_thought: NEEDS_THOUGHT };
    // Answered a moment ago: one courtesy is enough, so only ignore or the planner are offered.
    if (!recently) for (const [name, c] of Object.entries(COURTESIES)) criteria[name] = c.when;
    if (!recently && this.#maySay(now)) criteria.say = SAY_WHEN;
    let answer;
    try {
      const response = await jev.ask(
        { said: { from: item.speaker, text: item.text, kind: item.kind }, game },
        {
          small_talk: choice(
            "Someone in the game world said this to a game-playing agent in passing: an NPC, a bot or a player outside its party (state.said). What should the agent do about it, as a polite player busy with its own goals would?",
            criteria,
          ),
        },
      );
      this.calls++;
      answer = response.answers.small_talk;
    } catch {
      return false;
    }
    if (answer?.type !== "choice") return false;
    const sure = answer.confidence >= (this.options.confidence ?? 0.6);
    if (answer.choice === "needs_thought") {
      // Unsure that it needs the planner: it reads it with its next wake-up, at no extra turn.
      if (sure) sorted.wake.push(`${text} (the fast loop judged this needs you)`);
      else sorted.later.push(text);
      return true;
    }
    if (answer.choice === "say" && sure && criteria.say) {
      const said = await this.#say(item, now);
      if (said) {
        this.#answered.set(item.speaker, now);
        sorted.later.push(`${text} (the fast loop said "${said}" to ${item.speaker})`);
        return true;
      }
    }
    // A say that could not be written falls back to the emote that fits.
    const fallback = answer.choice === "say" && criteria.say ? (GREETING.test(item.text) ? "wave" : "nod") : answer.choice;
    const courtesy = recently ? undefined : COURTESIES[fallback];
    if (!courtesy || !sure) {
      sorted.later.push(`${text} (ignored)`);
      return true;
    }
    const result = await lua(courtesy.lua(item.speaker)).catch(() => ({ ok: false }));
    if (result.ok) this.#answered.set(item.speaker, now);
    sorted.later.push(`${text} (the fast loop ${result.ok ? courtesy.did : "could not answer"})`);
    return true;
  }

  /** Whether a say is allowed now: a writer, and under both caps. */
  #maySay(now: number): boolean {
    if (!this.options.speak) return false;
    this.#said = this.#said.filter((t) => now - t < 3_600_000);
    const last = this.#said.at(-1) ?? -Infinity;
    return now - last >= (this.options.sayGapMs ?? 30_000) && this.#said.length < (this.options.sayPerHour ?? 20);
  }

  /** Write and say one line; null on any failure, a timeout or an empty line. */
  async #say(item: Chatter, now: number): Promise<string | null> {
    const lua = this.options.lua!;
    const abort = new AbortController();
    const timeout = setTimeout(() => abort.abort(), this.options.sayTimeoutMs ?? 10_000);
    try {
      const me = await lua(CHARACTER_LUA);
      const character = (me.ok ? me.values?.[0] : undefined) as Character | undefined;
      const prompt = sayPrompt({
        character: character ?? {},
        speaker: item.speaker,
        text: item.text,
        kind: item.kind,
        chat: this.#chat.slice(-4),
        memory: memorySlice(this.options.memory?.() ?? ""),
      });
      const written = await Promise.race([
        this.options.speak!(prompt, abort.signal),
        new Promise<never>((_, reject) => abort.signal.addEventListener("abort", () => reject(new Error("say timed out")))),
      ]);
      const line = sayLine(written);
      if (!line) return null;
      const result = await lua(`SendChatMessage(${q(line)}, "SAY") return true`);
      if (!result.ok) return null;
      this.#said.push(now);
      return line;
    } catch {
      return null;
    } finally {
      clearTimeout(timeout);
    }
  }
}
