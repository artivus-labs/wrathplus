// The HTTP client for the WrathPlus agent control port (crates/wrathplus-agent): a bearer token on
// every request, JSON answers, PNG screenshots.

/** A failed control-port call, with text fit to show an agent. */
export class AgentError extends Error {
  constructor(
    message: string,
    /** The HTTP status, or undefined when the port could not be reached. */
    readonly status?: number,
  ) {
    super(message);
    this.name = "AgentError";
  }
}

export interface EventsQuery {
  /** Cursor; omitted, the read starts from now. 0 is the whole retained buffer. */
  since?: number;
  wait_ms?: number;
  limit?: number;
  /** Only these events: whole names, or prefixes when they end in `_` or `*`. */
  names?: string[];
  /** Never these, in the same form. */
  exclude?: string[];
}

type Query = Record<string, number | string | undefined>;

/** Longer than the port's own 60 s game timeout, so its 504 arrives before ours. */
const REQUEST_TIMEOUT_MS = 90_000;

export class AgentClient {
  readonly baseUrl: string;
  readonly #token: string;

  constructor(baseUrl: string, token: string) {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
    this.#token = token;
  }

  /** `GET /status`: client state and, in world, the player. */
  status(): Promise<unknown> {
    return this.#json("GET", "/status");
  }

  /** `POST /lua[?budget=N]`: run a chunk in the UI's Lua VM. */
  lua(code: string, budget?: number): Promise<unknown> {
    return this.#json("POST", "/lua", { budget }, code);
  }

  /** `GET /events[?since=N][&wait_ms=M][&limit=K][&names=A,B_][&exclude=C*]`: UI events after a
   *  cursor (from now without one), long-polling, filtered by name. */
  events(query: EventsQuery = {}): Promise<unknown> {
    const { names, exclude, ...rest } = query;
    return this.#json("GET", "/events", {
      ...rest,
      names: names?.length ? names.join(",") : undefined,
      exclude: exclude?.length ? exclude.join(",") : undefined,
    });
  }

  /** `GET /files`: the agent's saved Lua modules. */
  modules(): Promise<unknown> {
    return this.#json("GET", "/files");
  }

  /** `GET /files/<name>`: one module's source. */
  async readModule(name: string): Promise<string> {
    return (await this.#request("GET", `/files/${encodeURIComponent(name)}`, {})).text();
  }

  /** `PUT /files/<name>`: save a module (does not reload it; AgentReload does). */
  saveModule(name: string, source: string): Promise<unknown> {
    return this.#json("PUT", `/files/${encodeURIComponent(name)}`, {}, source);
  }

  /** `GET /screenshot[?scale=S]`: the next rendered frame as PNG bytes. */
  async screenshot(scale?: number): Promise<Uint8Array> {
    const response = await this.#request("GET", "/screenshot", { scale });
    const type = response.headers.get("content-type") ?? "";
    if (!type.startsWith("image/png")) {
      throw new AgentError(`expected a PNG, got ${type || "no content type"}: ${await response.text()}`);
    }
    return new Uint8Array(await response.arrayBuffer());
  }

  async #json(
    method: string,
    path: string,
    query: Query = {},
    body?: string,
  ): Promise<unknown> {
    const response = await this.#request(method, path, query, body);
    const text = await response.text();
    try {
      return JSON.parse(text);
    } catch {
      throw new AgentError(`${method} ${path}: answer was not JSON: ${text.slice(0, 500)}`);
    }
  }

  async #request(
    method: string,
    path: string,
    query: Query,
    body?: string,
  ): Promise<Response> {
    const url = new URL(this.baseUrl + path);
    for (const [key, value] of Object.entries(query)) {
      if (value !== undefined) url.searchParams.set(key, String(value));
    }
    let response: Response;
    try {
      response = await fetch(url, {
        method,
        headers: {
          Authorization: `Bearer ${this.#token}`,
          ...(body === undefined ? {} : { "Content-Type": "text/plain; charset=utf-8" }),
        },
        body,
        signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
      });
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error);
      throw new AgentError(
        `could not reach the WrathPlus client at ${this.baseUrl} (${reason}). ` +
          "It may not be running, or was started without --agent-port.",
      );
    }
    if (!response.ok) throw await httpError(method, path, response);
    return response;
  }
}

async function httpError(method: string, path: string, response: Response): Promise<AgentError> {
  const text = await response.text();
  let detail = text;
  try {
    const parsed: unknown = JSON.parse(text);
    if (parsed && typeof parsed === "object" && "error" in parsed) {
      detail = String((parsed as { error: unknown }).error);
    }
  } catch {
    // Not JSON: keep the raw text.
  }
  const hint =
    response.status === 401
      ? " (check WRATHPLUS_AGENT_TOKEN against the client's WOW_AGENT_TOKEN)"
      : response.status === 504
        ? " (the game loop is busy or stalled; try again)"
        : "";
  return new AgentError(
    `${method} ${path} failed with HTTP ${response.status}: ${detail || response.statusText}${hint}`,
    response.status,
  );
}
