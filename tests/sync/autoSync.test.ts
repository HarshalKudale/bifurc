import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const mockGetSyncConfig = vi.fn();
const mockSyncPull = vi.fn();
const mockGetRemoteHead = vi.fn();

vi.mock("@/sync/syncManager", () => ({
    getSyncConfig: (...args: any[]) => mockGetSyncConfig(...args),
    syncPull: (...args: any[]) => mockSyncPull(...args),
    getRemoteHead: (...args: any[]) => mockGetRemoteHead(...args),
}));

import { startAutoSync, stopAutoSync, stopAllAutoSync, updateLastKnownHead } from "@/sync/autoSync";

describe("autoSync", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        mockGetSyncConfig.mockReturnValue({ autoSync: true, remote: "origin", branch: "main" });
        mockSyncPull.mockResolvedValue({ ok: true, updated: false });
        mockGetRemoteHead.mockResolvedValue("abc123");
    });

    afterEach(() => {
        stopAllAutoSync();
        vi.useRealTimers();
        vi.resetAllMocks();
    });

    describe("startAutoSync()", () => {
        it("does nothing when sync config has autoSync disabled", () => {
            mockGetSyncConfig.mockReturnValue({ autoSync: false });
            startAutoSync("ws1");
            vi.advanceTimersByTime(60000);
            expect(mockGetRemoteHead).not.toHaveBeenCalled();
        });

        it("does nothing when sync config is null", () => {
            mockGetSyncConfig.mockReturnValue(null);
            startAutoSync("ws1");
            vi.advanceTimersByTime(60000);
            expect(mockGetRemoteHead).not.toHaveBeenCalled();
        });

        it("polls remote head on interval", async () => {
            startAutoSync("ws1");
            // First tick seeds lastKnownRemoteHead
            await vi.advanceTimersByTimeAsync(30_000);
            expect(mockGetRemoteHead).toHaveBeenCalledWith("ws1");
        });

        it("calls syncPull when remote head changes", async () => {
            startAutoSync("ws1");
            // Let the seed promise resolve
            await vi.advanceTimersByTimeAsync(1);
            // First poll - same head, no pull
            mockGetRemoteHead.mockResolvedValue("abc123");
            await vi.advanceTimersByTimeAsync(30_000);
            expect(mockSyncPull).not.toHaveBeenCalled();
            // Second poll - different head, should pull
            mockGetRemoteHead.mockResolvedValue("def456");
            await vi.advanceTimersByTimeAsync(30_000);
            expect(mockSyncPull).toHaveBeenCalledWith("ws1");
        });
    });

    describe("stopAutoSync()", () => {
        it("stops polling for a workspace", async () => {
            startAutoSync("ws1");
            await vi.advanceTimersByTimeAsync(1);
            stopAutoSync("ws1");
            mockGetRemoteHead.mockClear();
            await vi.advanceTimersByTimeAsync(60_000);
            expect(mockGetRemoteHead).not.toHaveBeenCalled();
        });

        it("does not throw for non-existent workspace", () => {
            expect(() => stopAutoSync("nonexistent")).not.toThrow();
        });
    });

    describe("stopAllAutoSync()", () => {
        it("stops all active pollers", async () => {
            startAutoSync("ws1");
            startAutoSync("ws2");
            await vi.advanceTimersByTimeAsync(1);
            stopAllAutoSync();
            mockGetRemoteHead.mockClear();
            await vi.advanceTimersByTimeAsync(60_000);
            expect(mockGetRemoteHead).not.toHaveBeenCalled();
        });
    });

    describe("updateLastKnownHead()", () => {
        it("updates head without triggering a pull on next tick", async () => {
            startAutoSync("ws1");
            await vi.advanceTimersByTimeAsync(1);
            // Update head to match what remote will return
            updateLastKnownHead("ws1", "newhead");
            mockGetRemoteHead.mockResolvedValue("newhead");
            await vi.advanceTimersByTimeAsync(30_000);
            expect(mockSyncPull).not.toHaveBeenCalled();
        });

        it("does not throw for non-existent workspace", () => {
            expect(() => updateLastKnownHead("nonexistent", "sha")).not.toThrow();
        });
    });
});
