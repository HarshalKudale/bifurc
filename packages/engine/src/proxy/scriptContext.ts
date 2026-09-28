import * as vm from "vm";
import { TestResultEntry } from "./scriptExecutor";

/**
 * Build a chai-like `expect(value)` assertion function.
 * Throws an Error with a descriptive message on assertion failure.
 */
export function buildExpect() {
  return function expect(actual: unknown) {
    function assertionError(msg: string): Error {
      return new Error(msg);
    }

    const buildChain = (negated: boolean) => {
      const chain: Record<string, unknown> = {};

      // Chainable language helpers (no-ops for readability)
      const passthrough = () => chain;
      Object.defineProperty(chain, "to", { get: () => chain });
      Object.defineProperty(chain, "be", { get: () => chain });
      Object.defineProperty(chain, "been", { get: () => chain });
      Object.defineProperty(chain, "is", { get: () => chain });
      Object.defineProperty(chain, "that", { get: () => chain });
      Object.defineProperty(chain, "which", { get: () => chain });
      Object.defineProperty(chain, "and", { get: () => chain });
      Object.defineProperty(chain, "has", { get: () => chain });
      Object.defineProperty(chain, "have", { get: () => chain });
      Object.defineProperty(chain, "with", { get: () => chain });
      Object.defineProperty(chain, "at", { get: () => chain });
      Object.defineProperty(chain, "of", { get: () => chain });
      Object.defineProperty(chain, "same", { get: () => chain });
      Object.defineProperty(chain, "but", { get: () => chain });
      Object.defineProperty(chain, "does", { get: () => chain });
      Object.defineProperty(chain, "still", { get: () => chain });
      Object.defineProperty(chain, "also", { get: () => chain });

      // .not — flips the negation
      Object.defineProperty(chain, "not", { get: () => buildChain(!negated) });

      // .ok — truthy check
      Object.defineProperty(chain, "ok", {
        get: () => {
          const pass = !!actual;
          if (negated ? pass : !pass) {
            throw assertionError(`expected ${JSON.stringify(actual)} ${negated ? "not " : ""}to be truthy`);
          }
          return chain;
        },
      });

      // .true / .false / .null / .undefined
      Object.defineProperty(chain, "true", {
        get: () => {
          const pass = actual === true;
          if (negated ? pass : !pass) throw assertionError(`expected ${JSON.stringify(actual)} ${negated ? "not " : ""}to be true`);
          return chain;
        },
      });
      Object.defineProperty(chain, "false", {
        get: () => {
          const pass = actual === false;
          if (negated ? pass : !pass) throw assertionError(`expected ${JSON.stringify(actual)} ${negated ? "not " : ""}to be false`);
          return chain;
        },
      });
      Object.defineProperty(chain, "null", {
        get: () => {
          const pass = actual === null;
          if (negated ? pass : !pass) throw assertionError(`expected ${JSON.stringify(actual)} ${negated ? "not " : ""}to be null`);
          return chain;
        },
      });
      Object.defineProperty(chain, "undefined", {
        get: () => {
          const pass = actual === undefined;
          if (negated ? pass : !pass) throw assertionError(`expected ${JSON.stringify(actual)} ${negated ? "not " : ""}to be undefined`);
          return chain;
        },
      });

      // .equal(val)
      chain.equal = chain.equals = chain.eq = (expected: unknown) => {
        const pass = actual === expected;
        if (negated ? pass : !pass) {
          throw assertionError(`expected ${JSON.stringify(actual)} ${negated ? "not " : ""}to equal ${JSON.stringify(expected)}`);
        }
        return chain;
      };

      // .eql(val) — deep equal
      chain.eql = chain.eqls = (expected: unknown) => {
        const pass = JSON.stringify(actual) === JSON.stringify(expected);
        if (negated ? pass : !pass) {
          throw assertionError(`expected ${JSON.stringify(actual)} ${negated ? "not " : ""}to deeply equal ${JSON.stringify(expected)}`);
        }
        return chain;
      };

      // .a(type) / .an(type)
      chain.a = chain.an = (type: string) => {
        let pass = false;
        const t = type.toLowerCase();
        if (t === "array") pass = Array.isArray(actual);
        else if (t === "object") pass = typeof actual === "object" && actual !== null && !Array.isArray(actual);
        else if (t === "null") pass = actual === null;
        else pass = typeof actual === t;
        if (negated ? pass : !pass) {
          throw assertionError(`expected ${JSON.stringify(actual)} ${negated ? "not " : ""}to be a(n) ${type}`);
        }
        return chain;
      };

      // .include(val) / .contain(val) / .includes(val) / .contains(val)
      chain.include = chain.contain = chain.includes = chain.contains = (val: unknown) => {
        let pass = false;
        if (typeof actual === "string" && typeof val === "string") pass = actual.includes(val);
        else if (Array.isArray(actual)) pass = actual.includes(val);
        else if (typeof actual === "object" && actual !== null && typeof val === "string") pass = val in (actual as object);
        if (negated ? pass : !pass) {
          throw assertionError(`expected ${JSON.stringify(actual)} ${negated ? "not " : ""}to include ${JSON.stringify(val)}`);
        }
        return chain;
      };

      // .property(key)
      chain.property = (key: string) => {
        const pass = typeof actual === "object" && actual !== null && key in (actual as object);
        if (negated ? pass : !pass) {
          throw assertionError(`expected object ${negated ? "not " : ""}to have property "${key}"`);
        }
        return chain;
      };

      // .length(n) / .lengthOf(n)
      chain.length = chain.lengthOf = (n: number) => {
        const len = (actual as any)?.length;
        const pass = len === n;
        if (negated ? pass : !pass) {
          throw assertionError(`expected length ${len} ${negated ? "not " : ""}to equal ${n}`);
        }
        return chain;
      };

      // .lengthAbove(n) — alias for length greater than
      chain.lengthAbove = (n: number) => {
        const len = (actual as any)?.length;
        const pass = typeof len === "number" && len > n;
        if (negated ? pass : !pass) {
          throw assertionError(`expected length ${len} ${negated ? "not " : ""}to be above ${n}`);
        }
        return chain;
      };

      // .above(n) / .greaterThan(n) / .gt(n)
      chain.above = chain.greaterThan = chain.gt = (n: number) => {
        const pass = typeof actual === "number" && actual > n;
        if (negated ? pass : !pass) {
          throw assertionError(`expected ${actual} ${negated ? "not " : ""}to be above ${n}`);
        }
        return chain;
      };

      // .below(n) / .lessThan(n) / .lt(n)
      chain.below = chain.lessThan = chain.lt = (n: number) => {
        const pass = typeof actual === "number" && actual < n;
        if (negated ? pass : !pass) {
          throw assertionError(`expected ${actual} ${negated ? "not " : ""}to be below ${n}`);
        }
        return chain;
      };

      // .least(n) / .gte(n)
      chain.least = chain.gte = (n: number) => {
        const pass = typeof actual === "number" && actual >= n;
        if (negated ? pass : !pass) {
          throw assertionError(`expected ${actual} ${negated ? "not " : ""}to be at least ${n}`);
        }
        return chain;
      };

      // .most(n) / .lte(n)
      chain.most = chain.lte = (n: number) => {
        const pass = typeof actual === "number" && actual <= n;
        if (negated ? pass : !pass) {
          throw assertionError(`expected ${actual} ${negated ? "not " : ""}to be at most ${n}`);
        }
        return chain;
      };

      // .match(regex)
      chain.match = chain.matches = (re: RegExp | string) => {
        const regex = typeof re === "string" ? new RegExp(re) : re;
        const pass = typeof actual === "string" && regex.test(actual);
        if (negated ? pass : !pass) {
          throw assertionError(`expected "${actual}" ${negated ? "not " : ""}to match ${regex}`);
        }
        return chain;
      };

      // .status(code) — shorthand for checking response status
      chain.status = (code: number) => {
        const pass = actual === code;
        if (negated ? pass : !pass) {
          throw assertionError(`expected status ${actual} ${negated ? "not " : ""}to be ${code}`);
        }
        return chain;
      };

      // .empty
      Object.defineProperty(chain, "empty", {
        get: () => {
          let pass = false;
          if (typeof actual === "string" || Array.isArray(actual)) pass = (actual as any).length === 0;
          else if (typeof actual === "object" && actual !== null) pass = Object.keys(actual).length === 0;
          if (negated ? pass : !pass) {
            throw assertionError(`expected ${JSON.stringify(actual)} ${negated ? "not " : ""}to be empty`);
          }
          return chain;
        },
      });

      // .oneOf(list)
      chain.oneOf = (list: unknown[]) => {
        const pass = list.includes(actual);
        if (negated ? pass : !pass) {
          throw assertionError(`expected ${JSON.stringify(actual)} ${negated ? "not " : ""}to be one of ${JSON.stringify(list)}`);
        }
        return chain;
      };

      return chain;
    };

    return buildChain(false);
  };
}

export function buildConsoleMock(logs: string[]) {
  return {
    log: (...args: unknown[]) => { logs.push(args.map((a) => typeof a === "string" ? a : JSON.stringify(a)).join(" ")); },
    warn: (...args: unknown[]) => { logs.push("[WARN] " + args.map((a) => typeof a === "string" ? a : JSON.stringify(a)).join(" ")); },
    error: (...args: unknown[]) => { logs.push("[ERROR] " + args.map((a) => typeof a === "string" ? a : JSON.stringify(a)).join(" ")); },
    info: (...args: unknown[]) => { logs.push(args.map((a) => typeof a === "string" ? a : JSON.stringify(a)).join(" ")); },
  };
}
