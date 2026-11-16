// Usage from a Messages API response: a JSON body, or a server-sent event stream.

import type { Usage } from "./pricing.ts";

const FIELDS = ["input_tokens", "output_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"] as const;

function merge(into: Usage, from: unknown): void {
  if (!from || typeof from !== "object") return;
  const source = from as Record<string, unknown>;
  for (const field of FIELDS) {
    const value = source[field];
    if (typeof value === "number") into[field] = value;
  }
  const breakdown = source.cache_creation;
  if (breakdown && typeof breakdown === "object") into.cache_creation = breakdown as Usage["cache_creation"];
}

/** `usage` of a non-streaming response body, or null when there is none. */
export function usageFromJson(body: unknown): Usage | null {
  if (!body || typeof body !== "object" || !("usage" in body)) return null;
  const usage: Usage = {};
  merge(usage, (body as { usage: unknown }).usage);
  return usage;
}

/**
 * Accumulates usage from a streamed response. `message_start` carries the prompt counts;
 * each `message_delta` carries cumulative counts (output, and on newer API versions input too),
 * so later values replace earlier ones.
 */
export class StreamUsage {
  private readonly decoder = new TextDecoder();
  private buffer = "";
  private readonly usage: Usage = {};
  private seen = false;

  push(chunk: Uint8Array): void {
    this.buffer += this.decoder.decode(chunk, { stream: true });
    let newline = this.buffer.indexOf("\n");
    while (newline >= 0) {
      this.line(this.buffer.slice(0, newline).replace(/\r$/, ""));
      this.buffer = this.buffer.slice(newline + 1);
      newline = this.buffer.indexOf("\n");
    }
  }

  private line(line: string): void {
    if (!line.startsWith("data:")) return;
    let event: { type?: string; message?: { usage?: unknown }; usage?: unknown };
    try {
      event = JSON.parse(line.slice(5).trim());
    } catch {
      return;
    }
    if (event.type === "message_start") {
      merge(this.usage, event.message?.usage);
      this.seen = true;
    } else if (event.type === "message_delta") {
      merge(this.usage, event.usage);
      this.seen = true;
    }
  }

  /** Usage seen so far, or null when the stream carried none. */
  result(): Usage | null {
    if (this.buffer) this.line(this.buffer.replace(/\r$/, ""));
    this.buffer = "";
    return this.seen ? { ...this.usage } : null;
  }
}

/**
 * Passes a response stream through unchanged and calls `done` exactly once with its usage:
 * at the end, on an upstream error, or when the client goes away (whatever was billed so far).
 */
export function meteredStream(body: ReadableStream<Uint8Array>, done: (usage: Usage | null) => void) {
  const reader = body.getReader();
  const usage = new StreamUsage();
  let finished = false;
  const finish = () => {
    if (finished) return;
    finished = true;
    done(usage.result());
  };
  return new ReadableStream<Uint8Array>({
    async pull(controller) {
      try {
        const { done: end, value } = await reader.read();
        if (end) {
          finish();
          controller.close();
          return;
        }
        usage.push(value);
        controller.enqueue(value);
      } catch (error) {
        finish();
        controller.error(error);
      }
    },
    async cancel(reason) {
      finish();
      await reader.cancel(reason);
    },
  });
}
