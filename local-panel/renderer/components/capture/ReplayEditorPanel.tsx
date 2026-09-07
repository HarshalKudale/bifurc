import React, { useState, useCallback, useEffect } from "react";
import { RequestLogEntry, MockRule, ReplayResult } from "@/types";
import HeaderTable from "@/components/common/HeaderTable";
import BodyEditor from "@/components/common/BodyEditor";
import {
  KVRow, mkRowId, headersToRows, rowsToHeaders,
  b64ToText, textToB64, tryFormat, statusColor,
  METHODS, METHOD_HEX, methodColor,
} from "@/lib/utils";
import { strings } from "@/lib/strings";
import { BodyMode, contentTypeToMode, modeToContentType } from "@/lib/bodyUtils";
import { Zap } from "@/lib/icons";

interface Props {
  entry: RequestLogEntry;
  onCreateMock: (initial: Partial<MockRule>) => void;
  onClose: () => void;
}

const SKIP_HEADERS = new Set([
  "host", "proxy-connection", "connection", "content-length", "transfer-encoding",
]);

function statusTextClass(s: number): string {
  if (s < 300) return "text-signal";
  if (s < 400) return "text-amber";
  return "text-destructive";
}

export default function ReplayEditorPanel({ entry, onCreateMock, onClose }: Props) {
  const [method, setMethod]     = useState(entry.method);
  const [url, setUrl]           = useState(entry.url);
  const [reqHeaders, setReqHeaders] = useState<KVRow[]>(() =>
    headersToRows(entry.reqHeaders, SKIP_HEADERS)
  );
  const [reqBody, setReqBody] = useState(() => tryFormat(b64ToText(entry.reqBody)));
  const [reqMode, setReqMode] = useState<BodyMode>(() => contentTypeToMode(entry.reqHeaders["content-type"]));

  const [reqTab, setReqTab] = useState<"headers" | "body">("headers");
  const [resTab, setResTab] = useState<"body" | "headers">("body");
  const [resMode, setResMode] = useState<BodyMode>("json");

  const [loading, setLoading]   = useState(false);
  const [result, setResult]     = useState<ReplayResult | null>(null);
  const [error, setError]       = useState<string | null>(null);

  const resBodyText = result ? (resMode === "json" ? tryFormat(b64ToText(result.body)) : b64ToText(result.body)) : "";

  useEffect(() => {
    const h = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [onClose]);

  const handleReqModeChange = useCallback((newMode: BodyMode) => {
    setReqMode(newMode);
    const ct = modeToContentType(newMode);
    setReqHeaders((prev) => {
      const withoutCT = prev.filter((r) => r.key.toLowerCase() !== "content-type");
      if (ct) {
        return [{ id: mkRowId(), enabled: true, key: "content-type", value: ct }, ...withoutCT];
      }
      return withoutCT;
    });
  }, []);

  const handleSend = useCallback(async () => {
    setLoading(true);
    setResult(null);
    setError(null);
    try {
      const res = await window.api.replayRequest(
        method, url, rowsToHeaders(reqHeaders), textToB64(reqBody)
      );
      const ct = res.headers["content-type"];
      if (ct) setResMode(contentTypeToMode(ct));
      setResult(res);
      setResTab("body");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : strings.capture.requestFailed);
    } finally {
      setLoading(false);
    }
  }, [method, url, reqHeaders, reqBody]);

  const handleCreateMock = useCallback(() => {
    const initial: Partial<MockRule> = {
      name: "",
      method,
      urlPattern: url,
      useRegex: false,
      capturedHeaders: rowsToHeaders(reqHeaders),
      capturedBody: textToB64(reqBody),
      responseStatus: result?.status ?? 200,
      responseStatusMocked: true,
      responseHeaders: result?.headers ?? {},
      mockedResponseHeaders: [],
      responseBody: result ? (resMode === "json" ? tryFormat(b64ToText(result.body)) : b64ToText(result.body)) : "{}",
      responseBodyMocked: true,
      responseDelayMocked: true,
    };
    onCreateMock(initial);
  }, [method, url, reqHeaders, reqBody, result, resMode, onCreateMock]);

  const reqHeaderCount = reqHeaders.filter((r) => r.enabled && r.key.trim()).length;

  return (
    <div className="flex flex-col flex-1 overflow-hidden bg-surface">
      {/* Title bar */}
      <div className="flex items-center gap-3 px-5 py-3 border-b border-border flex-shrink-0">
        <span className="text-xs font-semibold text-muted-foreground uppercase tracking-widest whitespace-nowrap">
          {strings.editor.replayRequest}
        </span>
        <div className="flex-1" />
        <button
          onClick={onClose}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded border border-border bg-card hover:bg-surface-2 text-muted-foreground hover:text-foreground cursor-pointer text-xs font-medium transition-colors flex-shrink-0"
        >
          {strings.editor.back}
        </button>
      </div>

      {/* Method + URL bar */}
      <div className="px-5 py-3 border-b border-border flex-shrink-0 flex items-center gap-2">
        <div
          className="flex items-stretch rounded border border-border focus-within:border-signal transition-colors overflow-hidden flex-1"
          style={{ background: "var(--c-card)" }}
        >
          <select
            value={method}
            onChange={(e) => setMethod(e.target.value)}
            className="bg-surface-2 border-r border-border text-xs font-bold font-mono px-3 py-2.5 outline-none cursor-pointer appearance-none flex-shrink-0"
            style={{ color: methodColor(method), minWidth: 80 }}
          >
            {METHODS.map((m) => (
              <option key={m} value={m} style={{ color: methodColor(m), background: "var(--c-card)" }}>
                {m}
              </option>
            ))}
          </select>
          <input
            className="flex-1 bg-transparent px-3 py-2.5 text-sm font-mono text-foreground outline-none placeholder:text-muted-foreground min-w-0"
            placeholder={strings.requests.urlPlaceholder}
            value={url}
            onChange={(e) => setUrl(e.target.value)}
          />
        </div>
        <button
          onClick={handleSend}
          disabled={loading || !url.trim()}
          className="px-5 py-2.5 rounded bg-signal hover:bg-signal/80 disabled:opacity-40 disabled:cursor-not-allowed text-background text-xs font-semibold transition-all cursor-pointer flex-shrink-0 flex items-center gap-1.5"
        >
          {loading ? (
            <>
              <span className="inline-block w-3 h-3 border-2 border-white/30 border-t-white rounded-full animate-spin" />
              {strings.server.sending}
            </>
          ) : (
            strings.server.send
          )}
        </button>
      </div>

      {/* Two-column body */}
      <div className="flex flex-1 min-h-0 overflow-hidden">

        {/* Left: Request */}
        <div className="flex flex-col min-h-0 overflow-hidden border-r border-border" style={{ width: "50%" }}>
          <div className="flex items-center flex-shrink-0 border-b border-border bg-background/40">
            <span className="px-4 py-2.5 text-[10px] font-semibold uppercase tracking-widest text-muted-foreground border-r border-border whitespace-nowrap">
              {strings.editor.request}
            </span>
            {(["headers", "body"] as const).map((t) => (
              <button
                key={t}
                onClick={() => setReqTab(t)}
                className={`px-4 py-2.5 text-xs font-medium cursor-pointer transition-colors whitespace-nowrap ${
                  reqTab === t
                    ? "text-signal border-b-2 border-signal -mb-px"
                    : "text-muted-foreground hover:text-foreground"
                }`}
              >
                {t === "headers"
                  ? `${strings.editor.headers}${reqHeaderCount > 0 ? ` (${reqHeaderCount})` : ""}`
                  : strings.editor.body}
              </button>
            ))}
          </div>
          <div className="flex-1 overflow-y-auto min-h-0">
            {reqTab === "headers" && (
              <HeaderTable rows={reqHeaders} onChange={setReqHeaders} />
            )}
            {reqTab === "body" && (
              <BodyEditor
                value={reqBody}
                onChange={setReqBody}
                placeholder={strings.capture.requestBodyPlaceholder}
                mode={reqMode}
                onModeChange={handleReqModeChange}
              />
            )}
          </div>
        </div>

        {/* Right: Response */}
        <div className="flex flex-col min-h-0 overflow-hidden flex-1">
          <div className="flex items-center flex-shrink-0 border-b border-border bg-background/40">
            <span className="px-4 py-2.5 text-[10px] font-semibold uppercase tracking-widest text-muted-foreground border-r border-border whitespace-nowrap">
              {strings.editor.response}
            </span>
            {(["body", "headers"] as const).map((t) => (
              <button
                key={t}
                onClick={() => setResTab(t)}
                className={`px-4 py-2.5 text-xs font-medium cursor-pointer transition-colors whitespace-nowrap ${
                  resTab === t
                    ? "text-signal border-b-2 border-signal -mb-px"
                    : "text-muted-foreground hover:text-foreground"
                }`}
              >
                {t}
              </button>
            ))}
            {result && (
              <div className="ml-auto pr-4 flex items-center gap-2">
                <span
                  className="text-base font-bold font-mono"
                  style={{ color: statusColor(result.status) }}
                >
                  {result.status}
                </span>
              </div>
            )}
          </div>
          <div className="flex-1 overflow-y-auto min-h-0">
            {loading && (
              <div className="flex items-center justify-center h-full">
                <div className="flex flex-col items-center gap-2 text-muted-foreground">
                  <span className="inline-block w-5 h-5 border-2 border-muted-foreground/30 border-t-signal rounded-full animate-spin" />
                  <span className="text-xs">{strings.server.sending}</span>
                </div>
              </div>
            )}
            {error && !loading && (
              <div className="p-4">
                <div className="px-3 py-2 rounded border border-destructive/30 bg-destructive/5 text-xs text-destructive font-mono">
                  {error}
                </div>
              </div>
            )}
            {!loading && !error && !result && (
              <div className="flex items-center justify-center h-full text-center">
                <div>
                  <div className="text-3xl opacity-20 mb-2">→</div>
                  <p className="text-xs text-muted-foreground">{strings.capture.sendToSeeResponse}</p>
                </div>
              </div>
            )}
            {result && !loading && (
              <>
                {resTab === "body" && (
                  <BodyEditor
                    value={resBodyText}
                    onChange={() => {}}
                    placeholder={strings.common.emptyBody}
                    readOnly
                    mode={resMode}
                  />
                )}
                {resTab === "headers" && (
                  <HeaderTable rows={headersToRows(result.headers)} onChange={() => {}} readOnly />
                )}
              </>
            )}
          </div>
        </div>
      </div>

      {/* Footer */}
      <div className="flex items-center justify-between px-5 py-3 border-t border-border flex-shrink-0 bg-background/30">
        <div className="flex items-center gap-2">
          {result && (
            <span className={`text-xs font-mono font-semibold ${statusTextClass(result.status)}`}>
              {result.status}
            </span>
          )}
          {result && (
            <span className="text-[10px] text-muted-foreground">
              {strings.capture.headerCount.replace("{count}", String(Object.keys(result.headers).length))}
            </span>
          )}
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={onClose}
            className="px-3 py-1.5 rounded border border-border bg-card hover:bg-surface-2 text-muted-foreground text-xs font-medium transition-all cursor-pointer"
          >
            {strings.common.cancel}
          </button>
          <button
            onClick={handleCreateMock}
            className="px-4 py-1.5 rounded bg-amber/20 hover:bg-amber/30 border border-amber/30 text-amber text-xs font-semibold transition-all cursor-pointer"
            title={result ? strings.capture.createMockResponseTitle : strings.capture.createMockRequestTitle}
          >
            <Zap size={11} className="inline mr-1" /> {result ? strings.capture.mockThisResponse : strings.requests.createMock}
          </button>
        </div>
      </div>
    </div>
  );
}
