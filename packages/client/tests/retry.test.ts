/**
 * The retry policy — P5 work item 5.
 *
 * The load-bearing cases are the two that a code-only policy gets wrong in the dangerous
 * direction, so they are named explicitly: a **retryable code on a non-idempotent command**
 * (`TIMEOUT` on `entity.create`) must **not** be retried, and the same code on a read must be.
 */

import { describe, expect, it } from "vitest";
import { EngineError, ErrorCode, COMMANDS, type ErrorCodeValue } from "@bifurc/protocol";
import {
  assertRetryPolicyIsTotal,
  backoffDelayMs,
  DEFAULT_RETRY_POLICY,
  isIdempotentCommand,
  isSafeToRetry,
  withRetry,
  IDEMPOTENT_COMMANDS,
  NON_IDEMPOTENT_COMMANDS,
  ALL_COMMANDS,
  type RetryPolicy,
} from "../src/retry";

describe("the retry policy partitions the protocol's commands", () => {
  it("classifies every command, with no overlap", () => {
    expect(() => assertRetryPolicyIsTotal()).not.toThrow();
  });

  it("covers the real command list", () => {
    // Guards the guard: an empty COMMANDS would make the partition trivially true.
    expect(ALL_COMMANDS.length).toBeGreaterThan(80);
    expect(ALL_COMMANDS).toEqual(Object.keys(COMMANDS));
  });

  it("has no command in both sets", () => {
    const overlap = [...IDEMPOTENT_COMMANDS].filter((c) => NON_IDEMPOTENT_COMMANDS.has(c));
    expect(overlap).toEqual([]);
  });

  it("would notice an unclassified command", () => {
    // The assertion is only worth having if it can fail. `entity.create` is the canonical
    // non-idempotent command, so removing it from its set must trip the guard.
    const removed = NON_IDEMPOTENT_COMMANDS.has("entity.create");
    expect(removed).toBe(true);

    const known = new Set([...IDEMPOTENT_COMMANDS, ...NON_IDEMPOTENT_COMMANDS]);
    known.delete("entity.create");
    expect(ALL_COMMANDS.filter((c) => !known.has(c))).toContain("entity.create");
  });
});

describe("isSafeToRetry — the code half AND the command half", () => {
  it("refuses to retry a mutation on a retryable code, because the mutation may have landed", () => {
    // The case this whole module exists for.
    expect(isSafeToRetry("entity.create", ErrorCode.TIMEOUT)).toBe(false);
    expect(isSafeToRetry("blob.put", ErrorCode.ENGINE_ERROR)).toBe(false);
    expect(isSafeToRetry("import.commit", ErrorCode.UPSTREAM_FAILED)).toBe(false);
  });

  it("allows retrying a read on a retryable code", () => {
    expect(isSafeToRetry("server.status", ErrorCode.TIMEOUT)).toBe(true);
    expect(isSafeToRetry("entity.list", ErrorCode.ENGINE_ERROR)).toBe(true);
  });

  it("refuses a non-retryable code even on an idempotent command", () => {
    // `BAD_REQUEST` means the payload was wrong; replaying it changes nothing.
    expect(isSafeToRetry("server.status", ErrorCode.BAD_REQUEST)).toBe(false);
    expect(isSafeToRetry("entity.update", ErrorCode.BAD_REQUEST)).toBe(false);
    expect(isSafeToRetry("entity.update", ErrorCode.UNAUTHORIZED)).toBe(false);
    expect(isSafeToRetry("entity.update", ErrorCode.FORBIDDEN)).toBe(false);
  });

  it("treats an unknown command as not retryable", () => {
    // Fail safe: an unclassified command must not be auto-retried.
    expect(isIdempotentCommand("something.new")).toBe(false);
    expect(isSafeToRetry("something.new", ErrorCode.TIMEOUT)).toBe(false);
  });
});

// ── applying the policy ──────────────────────────────────────────────────────

/**
 * A policy whose `sleep` records instead of waiting. The default sleeps for real, so a test of the
 * backoff sequence would otherwise take 150ms per case and be flaky under load.
 */
function recordingPolicy(attempts = 3): { policy: RetryPolicy; slept: number[] } {
  const slept: number[] = [];
  return {
    policy: { attempts, baseDelayMs: 50, maxDelayMs: 1_000, sleep: async (ms) => void slept.push(ms) },
    slept,
  };
}

/** An `EngineError`, i.e. the only shape `withRetry` will consider retrying. */
const fail = (code: ErrorCodeValue): (() => never) => () => {
  throw new EngineError(code, "boom");
};

describe("withRetry — applying the policy", () => {
  it("does not sleep or retry when the first attempt succeeds", async () => {
    const { policy, slept } = recordingPolicy();
    let calls = 0;

    const result = await withRetry("server.status", async () => {
      calls += 1;
      return "ok";
    }, policy);

    expect(result).toBe("ok");
    expect(calls).toBe(1);
    expect(slept).toEqual([]);
  });

  it("retries a read and succeeds on the second attempt", async () => {
    const { policy, slept } = recordingPolicy();
    let calls = 0;

    const result = await withRetry("server.status", async () => {
      calls += 1;
      if (calls === 1) throw new EngineError(ErrorCode.TIMEOUT, "slow");
      return "ok";
    }, policy);

    expect(result).toBe("ok");
    expect(calls).toBe(2);
    expect(slept).toEqual([50]);
  });

  it("gives up after `attempts` and rethrows the last error", async () => {
    const { policy, slept } = recordingPolicy(3);
    let calls = 0;

    await expect(
      withRetry("server.status", async () => {
        calls += 1;
        throw new EngineError(ErrorCode.TIMEOUT, "still slow");
      }, policy),
    ).rejects.toThrow("still slow");

    // 3 attempts = 2 waits, not 3: there is no sleep after the final failure.
    expect(calls).toBe(3);
    expect(slept).toEqual([50, 100]);
  });

  it("does not retry a mutation, even on a retryable code", async () => {
    // The case the whole module exists for, now asserted through the loop rather than the predicate:
    // a `TIMEOUT` means we do not know whether the entity was created.
    const { policy, slept } = recordingPolicy();
    let calls = 0;

    await expect(
      withRetry("entity.create", async () => {
        calls += 1;
        throw new EngineError(ErrorCode.TIMEOUT, "unknown outcome");
      }, policy),
    ).rejects.toThrow("unknown outcome");

    expect(calls).toBe(1);
    expect(slept).toEqual([]);
  });

  it("does not retry a non-retryable code", async () => {
    const { policy } = recordingPolicy();
    let calls = 0;

    await expect(
      withRetry("server.status", async () => {
        calls += 1;
        throw new EngineError(ErrorCode.BAD_REQUEST, "malformed");
      }, policy),
    ).rejects.toThrow("malformed");

    expect(calls).toBe(1);
  });

  it("does not retry anything that is not an EngineError", async () => {
    // All four transports reject with a real `EngineError`; anything else is a transport bug, and
    // retrying a bug is guessing.
    const { policy } = recordingPolicy();
    let calls = 0;

    await expect(
      withRetry("server.status", async () => {
        calls += 1;
        throw new Error("a raw Error, not an EngineError");
      }, policy),
    ).rejects.toThrow("a raw Error");

    expect(calls).toBe(1);
  });

  it("stops retrying once the transport is known to be closed", async () => {
    /**
     * The third term. Every transport rejects a request on a closed transport with `ENGINE_ERROR`,
     * which is a *retryable* code — so without this guard the loop would make three attempts and
     * wait 150ms against a transport it already knows is gone.
     */
    const { policy, slept } = recordingPolicy();
    let calls = 0;
    let closed = false;

    await expect(
      withRetry("server.status", async () => {
        calls += 1;
        closed = true; // the transport dies during the first attempt
        throw new EngineError(ErrorCode.ENGINE_ERROR, "this transport has been closed.");
      }, policy, () => closed),
    ).rejects.toThrow("has been closed");

    expect(calls).toBe(1);
    expect(slept).toEqual([]);
  });

  it("attempts: 1 disables retrying entirely", async () => {
    const { policy, slept } = recordingPolicy(1);
    let calls = 0;

    await expect(
      withRetry("server.status", async () => {
        calls += 1;
        throw new EngineError(ErrorCode.TIMEOUT, "nope");
      }, policy),
    ).rejects.toThrow("nope");

    expect(calls).toBe(1);
    expect(slept).toEqual([]);
  });
});

describe("backoffDelayMs — exponential, capped", () => {
  const policy = DEFAULT_RETRY_POLICY;

  it("doubles from the base and stops at the cap", () => {
    expect(backoffDelayMs(1, policy)).toBe(50);
    expect(backoffDelayMs(2, policy)).toBe(100);
    expect(backoffDelayMs(3, policy)).toBe(200);
    expect(backoffDelayMs(4, policy)).toBe(400);
    expect(backoffDelayMs(5, policy)).toBe(800);
    // 1600 would exceed the 1s cap.
    expect(backoffDelayMs(6, policy)).toBe(1_000);
    expect(backoffDelayMs(30, policy)).toBe(1_000);
  });
});
