// Requests from the party, sorted by the fast tier before the planner hears them. Jev reads what
// was said (with a little game state) and picks what it asks for: something the built-ins can
// just do (follow, stay, come here, pick up or hand in quests, attack their target), or anything
// else. A clear request is done at once, with a few words back in the same channel; the planner
// hears about it with its next wake-up instead of being woken. Questions, chat and anything
// needing thought go to the planner as before, and so does a request Jev is unsure of.

import { choice, noul, type Jev } from "./jev.ts";

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
      return Combat.engage(me.target)`,
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

  async handle(request: Request, game: unknown): Promise<Triaged> {
    const ask = `${request.sender} said "${request.text}"`;
    const criteria: Record<string, string> = { [OTHER.name]: OTHER.when };
    for (const [name, action] of Object.entries(ACTIONS)) criteria[name] = action.when;
    let response;
    try {
      response = await this.jev.ask(
        { message: { from: request.sender, text: request.text, channel: request.channel }, game },
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
