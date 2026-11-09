// Sliding-window rate limits in memory (one process behind Caddy). Keys are namespaced by the
// caller ("ip:1.2.3.4", "did:plc:…"). Memory is bounded by pruning expired keys as we go.

export interface Limit {
  max: number;
  windowMs: number;
}

export class RateLimiter {
  private readonly hits = new Map<string, number[]>();
  private sweeps = 0;

  constructor(
    private readonly limit: Limit,
    private readonly now: () => number = () => Date.now(),
  ) {}

  /** Records a hit and returns 0 if allowed, else the milliseconds until the next one would be. */
  take(key: string): number {
    const t = this.now();
    const since = t - this.limit.windowMs;
    const list = (this.hits.get(key) ?? []).filter((h) => h > since);
    if (list.length >= this.limit.max) {
      this.hits.set(key, list);
      return list[0]! + this.limit.windowMs - t;
    }
    list.push(t);
    this.hits.set(key, list);
    if (++this.sweeps % 1000 === 0) this.sweep(since);
    return 0;
  }

  private sweep(since: number): void {
    for (const [key, list] of this.hits) {
      if (list.every((h) => h <= since)) this.hits.delete(key);
    }
  }

  get size(): number {
    return this.hits.size;
  }
}

/** The client address: Caddy appends the peer to X-Forwarded-For, so the last entry is the real one. */
export function clientIp(headers: Headers, peer: string | undefined): string {
  const xff = headers.get("x-forwarded-for");
  if (xff) {
    const last = xff.split(",").pop()?.trim();
    if (last) return last;
  }
  return peer ?? "unknown";
}
