// The background loop: sample the scorecard and advance the judge every `intervalMs`. The judge's
// folder is set up once the client has said where the agent's modules live (`GET /files`), and is
// refused if it lies inside that folder, where a module path could reach it.

import { isAbsolute, join, relative, resolve } from "node:path";
import type { AgentClient } from "./client.ts";
import { Judge } from "./judge.ts";
import { SampleStore, Sampler } from "./scorecard.ts";
import type { Judging } from "./tools.ts";

/** The judge's folder: `WRATHPLUS_JUDGE_DIR`, else `judge` beside the modules folder (in the
 *  client's home, `<home>/Agent/judge`). An error when it lies inside the modules folder. */
export function judgeDir(modulesFolder: string, configured?: string): string {
  const dir = resolve(configured?.trim() || join(modulesFolder, "..", "judge"));
  const rel = relative(resolve(modulesFolder), dir);
  if (rel === "" || (!rel.startsWith("..") && !isAbsolute(rel))) {
    throw new Error(`the judge's folder ${dir} is inside the agent's modules folder ${modulesFolder}; choose another (WRATHPLUS_JUDGE_DIR)`);
  }
  return dir;
}

export interface MonitorOptions {
  client: AgentClient;
  intervalMs: number;
  /** WRATHPLUS_JUDGE_DIR, if set. */
  dir?: string;
  log?: (line: string) => void;
}

export class Monitor {
  #judging: Judging | undefined;
  #why = "the scorecard starts once the client answers (GET /files)";
  #timer: ReturnType<typeof setInterval> | undefined;
  #busy = false;

  constructor(readonly options: MonitorOptions) {}

  /** For the tools: the scorecard and judge, or why they are not there yet. */
  readonly ready = (): Judging | string => this.#judging ?? this.#why;

  async #setup(): Promise<void> {
    const listing = (await this.options.client.modules()) as { folder?: unknown };
    if (typeof listing.folder !== "string") throw new Error("GET /files named no modules folder");
    const dir = judgeDir(listing.folder, this.options.dir);
    const store = new SampleStore(join(dir, "characters"));
    const sampler = new Sampler(this.options.client, { store });
    const maxGapMs = Math.max(3 * this.options.intervalMs, 90_000);
    const judge = new Judge({ dir, port: this.options.client, store, latest: () => sampler.last, maxGapMs });
    await judge.recover();
    this.#judging = { judge, store, sampler, maxGapMs };
    this.options.log?.(`scorecard and judge in ${dir}`);
  }

  async tick(): Promise<void> {
    if (this.#busy) return;
    this.#busy = true;
    try {
      if (!this.#judging) {
        try {
          await this.#setup();
        } catch (error) {
          this.#why = `the scorecard is not set up: ${error instanceof Error ? error.message : String(error)}`;
          return;
        }
      }
      await this.#judging!.sampler.tick();
      await this.#judging!.judge.tick();
    } catch (error) {
      this.options.log?.(`judge: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      this.#busy = false;
    }
  }

  start(): void {
    if (this.#timer) return;
    void this.tick();
    this.#timer = setInterval(() => void this.tick(), this.options.intervalMs);
  }

  stop(): void {
    clearInterval(this.#timer);
    this.#timer = undefined;
  }
}
