import { describe, it, expect, beforeEach } from "vitest";
import { CommandRegistry } from "@/commands/registry";
import { bus } from "@/eventBus";

// `CommandRegistry` validates against the real, frozen `@bifurc/protocol` schemas — no mocking
// needed, per work item 7's design (an engine command is unreachable unless the protocol package
// already knows about it).

describe("CommandRegistry", () => {
  let registry: CommandRegistry;
  const ctx = { bus };

  beforeEach(() => {
    registry = new CommandRegistry();
  });

  it("registers and invokes a no-params command", () => {
    registry.register("config.get", () => ({ port: 8080 }));
    expect(registry.invoke("config.get", {}, ctx)).toEqual({ port: 8080 });
  });

  it("passes the parsed, validated payload to the handler", () => {
    let received: unknown;
    registry.register("env.setActive", (params) => {
      received = params;
      return { ok: true };
    });
    registry.invoke("env.setActive", { id: "env-1" }, ctx);
    expect(received).toEqual({ id: "env-1" });
  });

  it("accepts a null value where the schema allows it", () => {
    registry.register("env.setActive", (params: any) => params);
    expect(registry.invoke("env.setActive", { id: null }, ctx)).toEqual({ id: null });
  });

  it("throws when invoking a payload that fails schema validation", () => {
    registry.register("env.setActive", () => ({ ok: true }));
    // `id` is required (string | null) — omitting it must fail `EnvSetActiveParams.safeParse`.
    expect(() => registry.invoke("env.setActive", {}, ctx)).toThrow(/Invalid payload/);
  });

  it("throws when invoking a command with no registered handler", () => {
    expect(() => registry.invoke("config.get", {}, ctx)).toThrow(/No handler registered/);
  });

  it("throws when registering the same command twice", () => {
    registry.register("config.get", () => ({}));
    expect(() => registry.register("config.get", () => ({}))).toThrow(/already registered/);
  });

  it("returns whatever the handler returns, sync value or Promise, unchanged", async () => {
    registry.register("config.get", () => Promise.resolve({ port: 42 }));
    const result = registry.invoke("config.get", {}, ctx);
    expect(result).toBeInstanceOf(Promise);
    await expect(result).resolves.toEqual({ port: 42 });
  });

  it("list() reports every registered command name", () => {
    registry.register("config.get", () => ({}));
    registry.register("env.setActive", () => ({ ok: true }));
    expect(registry.list().sort()).toEqual(["config.get", "env.setActive"].sort());
  });

  it("isRegistered() is false for a known-but-unregistered command and an unknown string", () => {
    expect(registry.isRegistered("config.get")).toBe(false);
    expect(registry.isRegistered("not.a.real.command")).toBe(false);
    registry.register("config.get", () => ({}));
    expect(registry.isRegistered("config.get")).toBe(true);
  });
});
