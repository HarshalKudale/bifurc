import { describe, it, expect } from "vitest";
import { buildExpect, buildConsoleMock } from "@bifurc/engine/proxy/scriptContext";

/**
 * `buildExpect()` is a hand-rolled chai-like assertion library that backs
 * `lp.test(...)` in user-written API test scripts. It is ~230 lines of
 * property-getter trickery, so it is exactly the kind of code that silently
 * half-works: a broken negation or a missing alias would make a user's test suite
 * report green for the wrong reason. Every matcher is therefore checked in both
 * its passing and its failing form.
 *
 * NOTE: the built assertion function is bound to `chai` (not `expect`) so it never
 * shadows vitest's own `expect` inside a test body.
 */

/** Run `fn` and return the thrown Error, or null if it did not throw. */
function catchError(fn: () => unknown): Error | null {
    try {
        fn();
        return null;
    } catch (e) {
        return e as Error;
    }
}

describe("buildExpect — passing assertions", () => {
    it("supports the chainable readability helpers as no-ops", () => {
        const chai = buildExpect();
        chai(5).to.be.a("number");
        chai(5).to.be.an("number");
        chai(5).is.a("number");
        chai(5).that.is.a("number");
        chai(5).which.is.a("number");
        chai(5).and.is.a("number");
        chai(5).has.a("number");
        chai(5).have.a("number");
        chai(5).with.is.a("number");
        chai(5).at.is.a("number");
        chai(5).of.is.a("number");
        chai(5).same.is.a("number");
        chai(5).but.is.a("number");
        chai(5).does.is.a("number");
        chai(5).still.is.a("number");
        chai(5).also.is.a("number");
    });

    it("ok / true / false / null / undefined", () => {
        const chai = buildExpect();
        chai(1).ok;
        chai("x").ok;
        chai(true).true;
        chai(false).false;
        chai(null).null;
        chai(undefined).undefined;
    });

    it("equal / equals / eq are strict", () => {
        const chai = buildExpect();
        chai("a").equal("a");
        chai("a").equals("a");
        chai("a").eq("a");
        chai(5).equal(5);
    });

    it("eql / eqls deep-compare", () => {
        const chai = buildExpect();
        chai({ a: [1, 2] }).eql({ a: [1, 2] });
        chai({ a: [1, 2] }).eqls({ a: [1, 2] });
    });

    it("a / an type checks", () => {
        const chai = buildExpect();
        chai([]).a("array");
        chai({}).an("object");
        chai(null).a("null");
        chai("s").a("string");
        chai(1).a("number");
        chai(true).a("boolean");
        chai(() => 1).a("function");
    });

    it("include / contain / includes / contains", () => {
        const chai = buildExpect();
        chai("hello world").include("world");
        chai("hello world").contain("world");
        chai([1, 2, 3]).includes(2);
        chai([1, 2, 3]).contains(2);
        chai({ a: 1 }).include("a");
    });

    it("property", () => {
        const chai = buildExpect();
        chai({ a: 1 }).property("a");
    });

    it("length / lengthOf / lengthAbove", () => {
        const chai = buildExpect();
        chai("abc").length(3);
        chai([1, 2]).lengthOf(2);
        chai([1, 2, 3]).lengthAbove(2);
    });

    it("numeric comparisons", () => {
        const chai = buildExpect();
        chai(5).above(4);
        chai(5).greaterThan(4);
        chai(5).gt(4);
        chai(3).below(4);
        chai(3).lessThan(4);
        chai(3).lt(4);
        chai(5).least(5);
        chai(5).gte(5);
        chai(5).most(5);
        chai(5).lte(5);
    });

    it("match / matches accept a RegExp or a string pattern", () => {
        const chai = buildExpect();
        chai("abc123").match(/\d+/);
        chai("abc123").matches(/\d+/);
        chai("abc123").match("\\d+");
    });

    it("status", () => {
        const chai = buildExpect();
        chai(200).status(200);
    });

    it("empty", () => {
        const chai = buildExpect();
        chai("").empty;
        chai([]).empty;
        chai({}).empty;
    });

    it("oneOf", () => {
        const chai = buildExpect();
        chai(2).oneOf([1, 2, 3]);
    });
});

describe("buildExpect — failing assertions", () => {
    it("throws a descriptive error on a failed match", () => {
        const chai = buildExpect();
        const err = catchError(() => chai(5).equal(6));
        expect(err).toBeInstanceOf(Error);
        expect(err!.message).toContain("5");
        expect(err!.message).toContain("6");
    });

    it("throws when ok is used on a falsy value", () => {
        const chai = buildExpect();
        expect(catchError(() => chai(0).ok)).toBeInstanceOf(Error);
    });

    it("throws when a type check fails", () => {
        const chai = buildExpect();
        expect(catchError(() => chai("s").a("number"))).toBeInstanceOf(Error);
    });

    it("throws when a deep-equal fails", () => {
        const chai = buildExpect();
        expect(catchError(() => chai({ a: 1 }).eql({ a: 2 }))).toBeInstanceOf(Error);
    });

    it("throws when include fails", () => {
        const chai = buildExpect();
        expect(catchError(() => chai("abc").include("z"))).toBeInstanceOf(Error);
    });

    it("throws when property is absent", () => {
        const chai = buildExpect();
        expect(catchError(() => chai({ a: 1 }).property("b"))).toBeInstanceOf(Error);
    });

    it("throws when length is wrong", () => {
        const chai = buildExpect();
        expect(catchError(() => chai("abc").length(2))).toBeInstanceOf(Error);
    });

    it("throws when a numeric comparison fails", () => {
        const chai = buildExpect();
        expect(catchError(() => chai(1).above(2))).toBeInstanceOf(Error);
        expect(catchError(() => chai(5).below(2))).toBeInstanceOf(Error);
    });

    it("throws when match fails", () => {
        const chai = buildExpect();
        expect(catchError(() => chai("abc").match(/\d/))).toBeInstanceOf(Error);
    });

    it("throws when status is wrong", () => {
        const chai = buildExpect();
        expect(catchError(() => chai(404).status(200))).toBeInstanceOf(Error);
    });

    it("throws when empty is used on a non-empty value", () => {
        const chai = buildExpect();
        expect(catchError(() => chai("x").empty)).toBeInstanceOf(Error);
        expect(catchError(() => chai([1]).empty)).toBeInstanceOf(Error);
        expect(catchError(() => chai({ a: 1 }).empty)).toBeInstanceOf(Error);
    });

    it("throws when oneOf fails", () => {
        const chai = buildExpect();
        expect(catchError(() => chai(9).oneOf([1, 2]))).toBeInstanceOf(Error);
    });

    it("a matcher used in the middle of a chain still asserts", () => {
        // Guards against a matcher returning early / being swallowed by a getter.
        const chai = buildExpect();
        expect(catchError(() => chai(1).to.be.a("number").equal(2))).toBeInstanceOf(Error);
    });

    it("throws when a getter-style matcher is read on a mismatching value", () => {
        const chai = buildExpect();
        // `.ok` / `.empty` / `.true` are getters, so they only run when read.
        expect(catchError(() => chai(null).ok)).toBeInstanceOf(Error);
        expect(catchError(() => chai(0).true)).toBeInstanceOf(Error);
        expect(catchError(() => chai(1).null)).toBeInstanceOf(Error);
    });
});

describe("buildExpect — .not negation", () => {
    it("inverts each matcher", () => {
        const chai = buildExpect();
        chai(0).not.ok;
        chai(false).not.true;
        chai(true).not.false;
        chai(1).not.null;
        chai(1).not.undefined;
        chai(5).not.equal(6);
        chai({ a: 1 }).not.eql({ a: 2 });
        chai("s").not.a("number");
        chai("abc").not.include("z");
        chai({ a: 1 }).not.property("b");
        chai("abc").not.length(2);
        chai(1).not.above(2);
        chai(5).not.below(2);
        chai(1).not.least(2);
        chai(5).not.most(4);
        chai("abc").not.match(/\d/);
        chai(404).not.status(200);
        chai("x").not.empty;
        chai(9).not.oneOf([1, 2]);
    });

    it("still throws when the negated assertion is actually true", () => {
        const chai = buildExpect();
        expect(catchError(() => chai(1).not.ok)).toBeInstanceOf(Error);
        expect(catchError(() => chai(true).not.true)).toBeInstanceOf(Error);
        expect(catchError(() => chai(5).not.equal(5))).toBeInstanceOf(Error);
        expect(catchError(() => chai("abc").not.include("b"))).toBeInstanceOf(Error);
        expect(catchError(() => chai("").not.empty)).toBeInstanceOf(Error);
    });

    it("double negation behaves like the plain matcher", () => {
        const chai = buildExpect();
        chai(1).not.not.ok;
        chai(5).not.not.equal(5);
    });
});

describe("buildConsoleMock", () => {
    it("captures log/warn/error/info into the log array", () => {
        const logs: string[] = [];
        const c = buildConsoleMock(logs);
        c.log("a", "b");
        c.info("i");
        c.warn("w");
        c.error("e");
        expect(logs).toEqual(["a b", "i", "[WARN] w", "[ERROR] e"]);
    });

    it("JSON-stringifies non-string arguments", () => {
        const logs: string[] = [];
        buildConsoleMock(logs).log({ a: 1 }, [1, 2], 3);
        expect(logs).toEqual(['{"a":1} [1,2] 3']);
    });

    it("appends rather than replacing previous entries", () => {
        const logs: string[] = [];
        const c = buildConsoleMock(logs);
        c.log("one");
        c.log("two");
        expect(logs).toEqual(["one", "two"]);
    });
});
