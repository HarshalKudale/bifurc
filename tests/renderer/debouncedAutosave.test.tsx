// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import "@testing-library/jest-dom";
import { useDebouncedAutosave, AUTOSAVE_DELAY_MS } from "@/hooks/useDebouncedAutosave";

/**
 * This hook is what replaced every Save button in the UI, so these cases are the whole guard on it.
 *
 * Three properties are load-bearing and each has its own case below:
 * **the timer measures silence, not the first keystroke** (a burst of typing is one write, not one
 * per character), **arming is silent** (creating an entity must not write it straight back, or every
 * tab open would cost a redundant update), and **a draft never writes** (it has no id to write to).
 */

const DELAY = AUTOSAVE_DELAY_MS;

interface HarnessProps {
  snapshot: string;
  enabled: boolean;
  onSave: () => Promise<unknown> | unknown;
}

const renderAutosave = (initial: HarnessProps) =>
  renderHook((props: HarnessProps) => useDebouncedAutosave({ ...props, delay: DELAY }), {
    initialProps: initial,
  });

const settle = async (ms: number) => {
  await act(async () => { await vi.advanceTimersByTimeAsync(ms); });
};

beforeEach(() => { vi.useFakeTimers(); });
afterEach(() => { vi.useRealTimers(); });

describe("useDebouncedAutosave", () => {
  it("writes once, one second after the typing stops — not once per keystroke", async () => {
    const onSave = vi.fn();
    const { rerender } = renderAutosave({ snapshot: "a", enabled: true, onSave });

    // Arming must not itself write.
    expect(onSave).not.toHaveBeenCalled();

    // Four keystrokes, each landing before the previous debounce elapses.
    for (const s of ["ab", "abc", "abcd", "abcde"]) {
      rerender({ snapshot: s, enabled: true, onSave });
      await settle(DELAY - 100);
    }
    expect(onSave).not.toHaveBeenCalled();

    await settle(DELAY);
    expect(onSave).toHaveBeenCalledTimes(1);
  });

  it("never writes while disabled — a draft has no id to write to", async () => {
    const onSave = vi.fn();
    const { rerender } = renderAutosave({ snapshot: "a", enabled: false, onSave });

    rerender({ snapshot: "b", enabled: false, onSave });
    rerender({ snapshot: "c", enabled: false, onSave });
    await settle(DELAY * 3);

    expect(onSave).not.toHaveBeenCalled();
  });

  it("arming is silent: flipping enabled true does not itself write", async () => {
    const onSave = vi.fn();
    const { rerender } = renderAutosave({ snapshot: "a", enabled: false, onSave });

    // What happens when a draft is created and its tab id is swapped for the saved one.
    rerender({ snapshot: "a", enabled: true, onSave });
    await settle(DELAY * 3);

    expect(onSave).not.toHaveBeenCalled();
  });

  it("writes after being enabled only once the snapshot actually changes", async () => {
    const onSave = vi.fn();
    const { rerender } = renderAutosave({ snapshot: "a", enabled: false, onSave });
    rerender({ snapshot: "a", enabled: true, onSave });

    rerender({ snapshot: "b", enabled: true, onSave });
    await settle(DELAY);

    expect(onSave).toHaveBeenCalledTimes(1);
  });

  it("flush writes immediately and cancels the pending timer", async () => {
    const onSave = vi.fn();
    const { result, rerender } = renderAutosave({ snapshot: "a", enabled: true, onSave });
    rerender({ snapshot: "b", enabled: true, onSave });

    await act(async () => { await result.current.flush(); });
    expect(onSave).toHaveBeenCalledTimes(1);

    // The debounce that was already scheduled must not fire a second write.
    await settle(DELAY * 3);
    expect(onSave).toHaveBeenCalledTimes(1);
  });

  it("flush is a no-op when nothing has changed", async () => {
    const onSave = vi.fn();
    const { result } = renderAutosave({ snapshot: "a", enabled: true, onSave });

    await act(async () => { await result.current.flush(); });

    expect(onSave).not.toHaveBeenCalled();
  });

  it("flush is a no-op while disabled", async () => {
    const onSave = vi.fn();
    const { result, rerender } = renderAutosave({ snapshot: "a", enabled: false, onSave });
    rerender({ snapshot: "b", enabled: false, onSave });

    await act(async () => { await result.current.flush(); });

    expect(onSave).not.toHaveBeenCalled();
  });

  it("flushes a pending edit on unmount, so closing a tab cannot lose the last keystroke", async () => {
    const onSave = vi.fn();
    const { rerender, unmount } = renderAutosave({ snapshot: "a", enabled: true, onSave });
    rerender({ snapshot: "b", enabled: true, onSave });

    await act(async () => { unmount(); });

    expect(onSave).toHaveBeenCalledTimes(1);
  });

  it("does not flush on unmount while disabled", async () => {
    const onSave = vi.fn();
    const { rerender, unmount } = renderAutosave({ snapshot: "a", enabled: false, onSave });
    rerender({ snapshot: "b", enabled: false, onSave });

    await act(async () => { unmount(); });

    expect(onSave).not.toHaveBeenCalled();
  });

  it("a failed write leaves the edit pending, so a later flush retries it", async () => {
    const onSave = vi.fn().mockRejectedValueOnce(new Error("boom"));
    const { result, rerender } = renderAutosave({ snapshot: "a", enabled: true, onSave });
    rerender({ snapshot: "b", enabled: true, onSave });

    await settle(DELAY);
    expect(onSave).toHaveBeenCalledTimes(1);
    expect(result.current.error).toBe("boom");

    // The baseline must not have advanced to "b" — otherwise this flush would find nothing to do
    // and the user's edit would be silently swallowed.
    await act(async () => { await result.current.flush(); });
    expect(onSave).toHaveBeenCalledTimes(2);
  });

  it("reports saving and savedAt around a successful write, and clears the error", async () => {
    const onSave = vi.fn().mockRejectedValueOnce(new Error("boom"));
    const { result, rerender } = renderAutosave({ snapshot: "a", enabled: true, onSave });
    rerender({ snapshot: "b", enabled: true, onSave });

    await settle(DELAY);
    expect(result.current.error).toBe("boom");
    expect(result.current.savedAt).toBeNull();

    rerender({ snapshot: "c", enabled: true, onSave });
    await settle(DELAY);

    expect(result.current.error).toBeNull();
    expect(result.current.savedAt).not.toBeNull();
    expect(result.current.saving).toBe(false);
  });
});
