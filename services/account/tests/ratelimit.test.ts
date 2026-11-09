import { describe, expect, test } from "bun:test";
import { clientIp, RateLimiter } from "../src/ratelimit.ts";

describe("RateLimiter", () => {
  test("allows max hits per window, then reports the wait, then recovers", () => {
    let now = 1_000;
    const rl = new RateLimiter({ max: 3, windowMs: 10_000 }, () => now);
    expect([rl.take("a"), rl.take("a"), rl.take("a")]).toEqual([0, 0, 0]);
    expect(rl.take("a")).toBe(10_000);
    now += 4_000;
    expect(rl.take("a")).toBe(6_000);
    expect(rl.take("b")).toBe(0);
    now += 6_001;
    expect(rl.take("a")).toBe(0);
  });

  test("refused hits do not extend the window", () => {
    let now = 0;
    const rl = new RateLimiter({ max: 1, windowMs: 100 }, () => now);
    rl.take("k");
    for (let i = 0; i < 10; i++) {
      now += 9;
      rl.take("k");
    }
    now = 101;
    expect(rl.take("k")).toBe(0);
  });

  test("expired keys are swept", () => {
    let now = 0;
    const rl = new RateLimiter({ max: 5, windowMs: 10 }, () => now);
    for (let i = 0; i < 999; i++) rl.take(`k${i}`);
    now = 100;
    rl.take("last");
    expect(rl.size).toBe(1);
  });
});

describe("clientIp", () => {
  test("the last X-Forwarded-For entry (added by Caddy) wins over spoofed earlier ones", () => {
    expect(clientIp(new Headers({ "x-forwarded-for": "6.6.6.6, 198.51.100.7" }), "127.0.0.1")).toBe("198.51.100.7");
    expect(clientIp(new Headers(), "203.0.113.5")).toBe("203.0.113.5");
  });
});
