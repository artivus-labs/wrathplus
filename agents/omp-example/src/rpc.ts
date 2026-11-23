// A long-lived `omp --mode rpc` process: JSONL commands on stdin, responses and session events on
// stdout (see OMP's docs/rpc.md). One process holds the planner's session, and with it the
// WrathPlus MCP server, whose scorecard sampler and judge must keep running between turns.

import type { Subprocess } from "bun";

export type Frame = Record<string, unknown> & { type?: string; id?: string };

/** Split a byte stream into JSON lines; a line that is not JSON is passed to `bad`. */
export class LineDecoder {
  #buffer = "";
  #decoder = new TextDecoder();

  push(chunk: Uint8Array, frame: (f: Frame) => void, bad: (line: string) => void): void {
    this.#buffer += this.#decoder.decode(chunk, { stream: true });
    let at: number;
    while ((at = this.#buffer.indexOf("\n")) >= 0) {
      const line = this.#buffer.slice(0, at).trim();
      this.#buffer = this.#buffer.slice(at + 1);
      if (!line) continue;
      try {
        frame(JSON.parse(line) as Frame);
      } catch {
        bad(line);
      }
    }
  }
}

export interface RpcOptions {
  /** The command line, e.g. [bun, omp's cli.js, "--mode", "rpc", ...]. */
  cmd: string[];
  cwd: string;
  env: Record<string, string | undefined>;
  /** Every frame that is not a response to a command of ours. */
  onFrame: (frame: Frame) => void;
  /** stderr lines and undecodable stdout. */
  onLog: (line: string) => void;
}

export class OmpRpc {
  #proc: Subprocess<"pipe", "pipe", "pipe"> | undefined;
  #pending = new Map<string, { resolve: (f: Frame) => void; reject: (e: Error) => void }>();
  #next = 0;
  #ready: Promise<void> | undefined;
  readonly exited: Promise<number>;
  #exit!: (code: number) => void;

  constructor(readonly options: RpcOptions) {
    this.exited = new Promise((resolve) => (this.#exit = resolve));
  }

  /** Start the process and wait for its `ready` frame. */
  start(): Promise<void> {
    const proc = Bun.spawn(this.options.cmd, {
      cwd: this.options.cwd,
      env: this.options.env,
      stdin: "pipe",
      stdout: "pipe",
      stderr: "pipe",
    });
    this.#proc = proc;
    let ready!: () => void;
    this.#ready = new Promise((resolve) => (ready = resolve));
    const out = new LineDecoder();
    void (async () => {
      for await (const chunk of proc.stdout) {
        out.push(
          chunk,
          (f) => {
            if (f.type === "ready") return ready();
            if (f.type === "response" && typeof f.id === "string" && this.#pending.has(f.id)) {
              const p = this.#pending.get(f.id)!;
              this.#pending.delete(f.id);
              if (f.success === false) p.reject(new Error(`${String(f.command)}: ${String(f.error)}`));
              else p.resolve(f);
              // A prompt's response only admits it; its prompt_result still follows.
              if (f.command !== "prompt" && f.command !== "abort_and_prompt") return;
            }
            this.options.onFrame(f);
          },
          (line) => this.options.onLog(`stdout: ${line.slice(0, 500)}`),
        );
      }
    })();
    void (async () => {
      const decoder = new TextDecoder();
      for await (const chunk of proc.stderr) {
        for (const line of decoder.decode(chunk, { stream: true }).split("\n")) {
          if (line.trim()) this.options.onLog(`stderr: ${line.slice(0, 500)}`);
        }
      }
    })();
    void proc.exited.then((code) => {
      for (const p of this.#pending.values()) p.reject(new Error(`omp exited (${code})`));
      this.#pending.clear();
      this.#exit(code);
    });
    return Promise.race([
      this.#ready,
      this.exited.then((code) => {
        throw new Error(`omp exited (${code}) before it was ready`);
      }),
    ]);
  }

  /** Send a command and wait for its response (for a prompt: its admission). */
  send(command: Record<string, unknown>, timeoutMs = 120_000): Promise<Frame> {
    const proc = this.#proc;
    if (!proc) return Promise.reject(new Error("omp is not running"));
    const id = `l${++this.#next}`;
    const done = new Promise<Frame>((resolve, reject) => {
      this.#pending.set(id, { resolve, reject });
      setTimeout(() => {
        if (this.#pending.delete(id)) reject(new Error(`${String(command.type)}: no response in ${timeoutMs} ms`));
      }, timeoutMs);
    });
    proc.stdin.write(`${JSON.stringify({ ...command, id })}\n`);
    proc.stdin.flush();
    return done;
  }

  /** Close stdin (OMP drains and exits), then kill it if it lingers. */
  async stop(graceMs = 10_000): Promise<void> {
    const proc = this.#proc;
    if (!proc) return;
    try {
      proc.stdin.end();
    } catch {
      // Already closed.
    }
    const timer = setTimeout(() => proc.kill("SIGTERM"), graceMs);
    await this.exited;
    clearTimeout(timer);
  }
}
