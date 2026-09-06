import React, { useEffect, useRef, useState, useCallback } from "react";
import { RequestLogEntry, MockRule, ReplayResult } from "@/types";
import RestTab from "@/components/rest/RestTab";
import ReplayResultModal from "@/components/capture/ReplayResultModal";
import ViaBadge, { VIA_LABEL } from "@/components/common/ViaBadge";
import { Play, Pause, Zap, Clipboard } from "@/lib/icons";
import { Button } from "@/components/ui";
import { strings } from "@/lib/strings";

const MAX_ENTRIES = 100;

function statusColor(s: number | null): string {
  if (s === null) return "text-muted-foreground";
  if (s < 300) return "text-signal";
  if (s < 400) return "text-amber";
  return "text-destructive";
}

function fmtTime(ts: number): string {
  const d = new Date(ts);
  return d.toTimeString().slice(0, 8) + "." + String(d.getMilliseconds()).padStart(3, "0");
}

function fmtDur(ms: number | null): string {
  if (ms === null) return "—";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

interface Props {
  onMockAdded: (mock: MockRule) => void;
}

export default function LogsPanel({ onMockAdded }: Props) {
  const [entries, setEntries] = useState<RequestLogEntry[]>([]);
  const [paused, setPaused] = useState(false);
  const pausedRef = useRef(false);
  pausedRef.current = paused;

  const [replayEntry, setReplayEntry] = useState<RequestLogEntry | null>(null);
  const [replayLoading, setReplayLoading] = useState(false);
  const [replayResult, setReplayResult] = useState<ReplayResult | null>(null);
  const [replayError, setReplayError] = useState<string | null>(null);

  const [mockEntry, setMockEntry] = useState<RequestLogEntry | null>(null);

  useEffect(() => {
    const unsub = window.api.onLogEntry((entry) => {
      if (pausedRef.current) return;
      setEntries((prev) => {
        const next = [entry, ...prev];
        return next.length > MAX_ENTRIES ? next.slice(0, MAX_ENTRIES) : next;
      });
    });
    return unsub;
  }, []);

  const clear = useCallback(() => setEntries([]), []);

  const handleReplay = useCallback(async (entry: RequestLogEntry) => {
    setReplayEntry(entry);
    setReplayResult(null);
    setReplayError(null);
    setReplayLoading(true);
    try {
      const result = await window.api.replayRequest(entry.method, entry.url, entry.reqHeaders, entry.reqBody);
      setReplayResult(result);
    } catch (e: unknown) {
      setReplayError(e instanceof Error ? e.message : "Request failed");
    } finally {
      setReplayLoading(false);
    }
  }, []);

  const handleMockSave = useCallback(async (data: Omit<MockRule, "id" | "createdAt" | "workspaceId">) => {
    const mock = await window.api.addMock(data);
    onMockAdded(mock);
    setMockEntry(null);
  }, [onMockAdded]);

  const buildMockInitial = (e: RequestLogEntry): Partial<MockRule> => ({
    name: "",
    method: e.method,
    urlPattern: e.url,
    useRegex: false,
    capturedHeaders: e.reqHeaders,
    capturedBody: e.reqBody,
    responseStatus: e.resStatus ?? 200,
    responseHeaders: e.resHeaders,
    responseBody: (() => {
      if (!e.resBody) return "{}";
      try {
        const bytes = Uint8Array.from(atob(e.resBody), (c) => c.charCodeAt(0));
        return new TextDecoder("utf-8", { fatal: false }).decode(bytes);
      } catch { return "{}"; }
    })(),
  });

  if (mockEntry) {
    return (
      <RestTab
        tabType="mock"
        tabId={`logs-mock-${mockEntry.id}`}
        initial={buildMockInitial(mockEntry)}
        onSave={handleMockSave}
        onClose={() => setMockEntry(null)}
        showCurlImport={false}
      />
    );
  }

  return (
    <div className="flex flex-col flex-1 overflow-hidden">
      <div className="px-6 py-4 border-b border-border flex items-center gap-3 flex-shrink-0">
        <div>
          <h1 className="text-base font-semibold text-foreground">{strings.logs.title}</h1>
          <p className="text-xs text-muted-foreground mt-0.5">{strings.logs.subtitle}</p>
        </div>
        <div className="ml-auto flex items-center gap-2">
          <button
            onClick={() => setPaused((v) => !v)}
            className={`px-3 py-1.5 rounded border text-xs font-medium transition-all cursor-pointer whitespace-nowrap ${
              paused
                ? "border-amber bg-amber/10 text-amber"
                : "border-border bg-card hover:bg-surface-2 text-muted-foreground hover:text-foreground"
            }`}
          >
            {paused ? <><Play size={10} fill="currentColor" className="inline mr-1" /> {strings.logs.resume}</> : <><Pause size={10} fill="currentColor" className="inline mr-1" /> {strings.logs.pause}</>}
          </button>
          <Button variant="secondary" onClick={clear}>{strings.logs.clear}</Button>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto font-mono text-xs">
        {entries.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-center py-16">
            <div className="opacity-15 mb-3"><Clipboard size={36} /></div>
            <div className="text-sm font-medium text-foreground font-sans mb-1">{strings.logs.noEntries}</div>
            <p className="text-xs text-muted-foreground font-sans">
              {strings.logs.emptyHint}
            </p>
          </div>
        ) : (
          <table className="w-full border-collapse">
            <thead className="sticky top-0 bg-background z-10">
              <tr className="border-b border-border">
                <th className="text-left px-3 py-2 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground whitespace-nowrap">{strings.logs.colTime}</th>
                <th className="text-left px-3 py-2 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground w-12">{strings.logs.colMethod}</th>
                <th className="text-left px-3 py-2 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground w-10">{strings.logs.colStatus}</th>
                <th className="text-left px-3 py-2 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">{strings.logs.colUrl}</th>
                <th className="text-left px-3 py-2 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground w-16">{strings.logs.colVia}</th>
                <th className="text-right px-3 py-2 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground w-14">{strings.logs.colDur}</th>
                <th className="px-3 py-2 w-28" />
              </tr>
            </thead>
            <tbody>
              {entries.map((e) => (
                <tr
                  key={e.id}
                  className="border-b border-border/30 hover:bg-surface transition-colors group"
                  title={e.url}
                >
                  <td className="px-3 py-1.5 text-muted-foreground whitespace-nowrap">{fmtTime(e.ts)}</td>
                  <td className="px-3 py-1.5 text-signal whitespace-nowrap">{e.method}</td>
                  <td className={`px-3 py-1.5 whitespace-nowrap font-semibold ${statusColor(e.status)}`}>
                    {e.status ?? "—"}
                  </td>
                  <td className="px-3 py-1.5 text-foreground max-w-0">
                    <span className="block truncate">{e.url}</span>
                  </td>
                  <td className="px-3 py-1.5 whitespace-nowrap">
                    <ViaBadge via={e.via} />
                  </td>
                  <td className="px-3 py-1.5 text-right text-muted-foreground whitespace-nowrap">
                    {fmtDur(e.durationMs)}
                  </td>
                  <td className="px-3 py-1.5 text-right whitespace-nowrap">
                    <div className="flex gap-1 justify-end">
                      <button
                        onClick={() => handleReplay(e)}
                        className="px-2 py-0.5 rounded border border-border bg-card hover:bg-surface-2 text-muted-foreground hover:text-signal text-[10px] font-medium transition-all cursor-pointer"
                        title={strings.logs.replayTitle}
                      >
                        <Play size={10} fill="currentColor" className="inline mr-0.5" /> {strings.logs.replay}
                      </button>
                      <button
                        onClick={() => setMockEntry(e)}
                        className="px-2 py-0.5 rounded border border-border bg-card hover:bg-surface-2 text-muted-foreground hover:text-amber text-[10px] font-medium transition-all cursor-pointer"
                        title={strings.logs.mockTitle}
                      >
                        <Zap size={10} className="inline mr-0.5" /> {strings.logs.mock}
                      </button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>

      <div className="px-4 py-1.5 border-t border-border flex items-center gap-3 text-[10px] text-muted-foreground font-mono flex-shrink-0">
        <span>{entries.length} {strings.logs.entries}</span>
        {q && <span>· {filtered.length} {strings.logs.shown}</span>}
        {paused && <span className="text-amber">· {strings.logs.paused}</span>}
        <span className="ml-auto">{strings.logs.footerNote.replace("{max}", String(MAX_ENTRIES))}</span>
      </div>

      <ReplayResultModal
        open={!!replayEntry}
        url={replayEntry?.url ?? ""}
        result={replayResult}
        error={replayError}
        loading={replayLoading}
        onClose={() => { setReplayEntry(null); setReplayResult(null); setReplayError(null); }}
      />
    </div>
  );
}
