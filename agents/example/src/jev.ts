// A small client for TypeSafe's System One API (Jev): typed questions in, calibrated
// probabilities out. One endpoint, POST /v1/systemone (docs.typesafe.ai). Written here rather than
// using @typesafe-ai/sdk 0.7.0, which reads a response body twice under Bun and hides every answer.

export type Question =
  | { type: "choice"; instructions: string; criteria: Record<string, string | null> }
  | { type: "noul"; instructions: string };

export interface ChoiceAnswer {
  type: "choice";
  choice: string;
  confidence: number;
  probabilities: Record<string, number>;
}

export interface NoulAnswer {
  type: "noul";
  noul: number;
}

export type Answer = ChoiceAnswer | NoulAnswer;

export const choice = (instructions: string, criteria: Record<string, string | null>): Question => ({
  type: "choice",
  instructions,
  criteria,
});

export const noul = (instructions: string): Question => ({ type: "noul", instructions });

export interface JevResponse {
  model: string;
  answers: Record<string, Answer>;
  usage?: { input_tokens: number; output_tokens: number };
}

export class Jev {
  readonly #key: string;

  constructor(
    key: string,
    readonly model = "jev-latest",
    readonly baseUrl = "https://api.typesafe.ai",
  ) {
    this.#key = key;
  }

  async ask(state: unknown, questions: Record<string, Question>): Promise<JevResponse> {
    const response = await fetch(`${this.baseUrl}/v1/systemone`, {
      method: "POST",
      headers: { Authorization: `Bearer ${this.#key}`, "Content-Type": "application/json" },
      body: JSON.stringify({ model: this.model, state, questions }),
      signal: AbortSignal.timeout(15_000),
    });
    const text = await response.text();
    if (!response.ok) {
      throw new Error(`Jev HTTP ${response.status}: ${text.slice(0, 300)}`);
    }
    return JSON.parse(text) as JevResponse;
  }
}

export function chosen(answer: Answer | undefined): ChoiceAnswer {
  if (answer?.type !== "choice") throw new Error("Jev: expected a choice answer");
  return answer;
}

export function yes(answer: Answer | undefined): number {
  return answer?.type === "noul" ? answer.noul : 0;
}
