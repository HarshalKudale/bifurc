import React, { useState } from "react";
import { RequestLogEntry } from "@/types";
import { X } from "@/lib/icons";
import { IconButton } from "@/components/ui";
import CodeEditor from "@/components/common/CodeEditor";
import BinaryViewer from "@/components/common/BinaryViewer";
import { isBinaryContentType } from "@/lib/bodyUtils";
import { strings } from "@/lib/strings";
import CaptureHeaderTable from "./CaptureHeaderTable";
import {
  b64ToText, tryFormat, ctToLang, statusColor, fmtTime, fmtDur,
  getHeader, fulfilledBy, fulfilledColor, resBodySize,
} from "./captureUtils";

function Pane({ headers, body }: { headers: Record<string, string>; body: string }) {
  const ct = getHeader(headers, "content-type").toLowerCase();
  const isBinary = isBinaryContentType(ct);
  const lang = ctToLang(ct);
  const bodyText = isBinary ? "" : b64ToText(body);
  const displayBody = lang === "json" ? tryFormat(bodyText) : bodyText;

  if (isBinary && body) {
    return (
      <div className="flex-1 overflow-hidden">
        <BinaryViewer data={body} contentType={ct.split(";")[0].trim()} />
      </div>
    );
  }
  if (!displayBody) {
    return <p className="font-mono text-[11px] text-muted-foreground italic p-3">{strings.capture.emptyBody}</p>;
  }
  return <CodeEditor value={displayBody} readOnly language={lang} className="h-full" />;
}

function TimingRow({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between px-3 py-2 border-b border-border/40 last:border-0">
      <span className="text-[11px] text-muted-foreground">{label}</span>
      <span className="text-[11px] font-mono text-foreground">{value}</span>
    </div>
  );
}

type DetailTab = "headers" | "payload" | "response" | "timing";

interface Props {
  entry: RequestLogEntry;
  onClose: () => void;
}

export default function CaptureDetail({ entry, onClose }: Props) {
  const [tab, setTab] = useState<DetailTab>("headers");

  const tabs: { key: DetailTab; label: string }[] = [
    { key: "headers", label: strings.capture.tabHeaders },
    { key: "payload", label: strings.capture.tabPayload },
    { key: "response", label: strings.capture.tabResponse },
    { key: "timing", label: strings.capture.tabTiming },
  ];

  return (
    <div className="flex flex-col flex-1 min-h-0 overflow-hidden bg-surface border-l border-border">
      {/* Header */}
      <div className="flex items-center gap-3 px-3 py-2 border-b border-border flex-shrink-0">
        <span className="font-mono text-xs font-semibold text-signal">{entry.method}</span>
        <span className="font-mono text-xs text-muted-foreground truncate flex-1" title={entry.url}>{entry.url}</span>
        {entry.status !== null && (
          <span className={`font-mono text-xs font-bold ${statusColor(entry.status)}`}>{entry.status}</span>
        )}
        {entry.durationMs !== null && (
          <span className="font-mono text-[10px] text-muted-foreground">{fmtDur(entry.durationMs)}</span>
        )}
        <IconButton icon={<X size={14} />} onClick={onClose} title={strings.capture.closeDetail} />
      </div>

      {/* Tabs */}
      <div className="flex items-center gap-1 px-3 pt-2 border-b border-border flex-shrink-0">
        {tabs.map((t) => (
          <button
            key={t.key}
            onClick={() => setTab(t.key)}
            className={`px-3 py-1 text-[11px] font-medium cursor-pointer transition-colors rounded-t ${
              tab === t.key ? "bg-card text-foreground border border-border border-b-card" : "text-muted-foreground hover:text-foreground"
            }`}
          >
            {t.label}
          </button>
        ))}
      </div>

      {/* Content */}
      <div className="flex-1 min-h-0 overflow-auto">
        {tab === "headers" && (
          <div>
            <div className="px-3 py-1.5 text-[10px] font-semibold uppercase tracking-widest text-muted-foreground bg-card/40">
              {strings.capture.detailRequestHeaders}
            </div>
            <CaptureHeaderTable headers={entry.reqHeaders} />
            <div className="px-3 py-1.5 text-[10px] font-semibold uppercase tracking-widest text-muted-foreground bg-card/40 border-t border-border">
              {strings.capture.detailResponseHeaders}
            </div>
            <CaptureHeaderTable headers={entry.resHeaders} />
          </div>
        )}
        {tab === "payload" && <Pane headers={entry.reqHeaders} body={entry.reqBody} />}
        {tab === "response" && <Pane headers={entry.resHeaders} body={entry.resBody} />}
        {tab === "timing" && (
          <div>
            <TimingRow label={strings.capture.timingStarted} value={fmtTime(entry.ts)} />
            <TimingRow label={strings.capture.timingDuration} value={fmtDur(entry.durationMs)} />
            <TimingRow label={strings.capture.timingStatus} value={<span className={statusColor(entry.status)}>{entry.status ?? "—"}</span>} />
            <TimingRow label={strings.capture.timingFulfilledBy} value={<span className={fulfilledColor(entry.via)}>{fulfilledBy(entry.via)}</span>} />
            <TimingRow label={strings.capture.timingSize} value={resBodySize(entry)} />
          </div>
        )}
      </div>
    </div>
  );
}
