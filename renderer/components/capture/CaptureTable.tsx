import React from "react";
import { RequestLogEntry } from "@/types";
import { Square, CheckSquare, Ban } from "@/lib/icons";
import { strings } from "@/lib/strings";
import { blockKey } from "@/lib/blocks";
import {
  statusColor, fmtTime, fmtDur, urlName, deriveType, fulfilledBy, fulfilledColor, resBodySize,
} from "./captureUtils";

const TYPE_LABEL: Record<ReturnType<typeof deriveType>, string> = {
  xhr: strings.capture.typeXhr,
  doc: strings.capture.typeDoc,
  css: strings.capture.typeCss,
  js: strings.capture.typeJs,
  font: strings.capture.typeFont,
  img: strings.capture.typeImg,
  media: strings.capture.typeMedia,
  other: strings.capture.typeOther,
};

interface Props {
  entries: RequestLogEntry[];
  selectedIds: Set<string>;
  activeId: string | null;
  blockedKeys: Set<string>;
  onRowClick: (entry: RequestLogEntry, ev: React.MouseEvent) => void;
  onToggleCheck: (id: string, ev: React.MouseEvent) => void;
  onToggleAll: () => void;
  onContextMenu: (entry: RequestLogEntry, ev: React.MouseEvent) => void;
}

const TH = "text-left px-3 py-2 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground";

export default function CaptureTable({
  entries, selectedIds, activeId, blockedKeys, onRowClick, onToggleCheck, onToggleAll, onContextMenu,
}: Props) {
  const allSelected = entries.length > 0 && entries.every((e) => selectedIds.has(e.id));

  return (
    <table className="w-full border-collapse">
      <thead className="sticky top-0 bg-background z-10">
        <tr className="border-b border-border">
          <th className="px-3 py-2 w-8">
            <button
              onClick={onToggleAll}
              className="flex items-center justify-center text-muted-foreground hover:text-signal transition-colors cursor-pointer"
              title={strings.capture.selectAll}
            >
              {allSelected ? <CheckSquare size={13} /> : <Square size={13} />}
            </button>
          </th>
          <th className={TH}>{strings.capture.colName}</th>
          <th className={`${TH} w-14`}>{strings.capture.colMethod}</th>
          <th className={`${TH} w-12`}>{strings.capture.colStatus}</th>
          <th className={`${TH} w-20`}>{strings.capture.colType}</th>
          <th className={`${TH} w-24`}>{strings.capture.colFulfilledBy}</th>
          <th className={`${TH} w-16 text-right`}>{strings.capture.colSize}</th>
          <th className={`${TH} w-28`}>{strings.capture.colTime}</th>
        </tr>
      </thead>
      <tbody>
        {entries.map((e) => {
          const checked = selectedIds.has(e.id);
          const isActive = e.id === activeId;
          const blocked = blockedKeys.has(blockKey(e.method, e.url));
          return (
            <tr
              key={e.id}
              onClick={(ev) => onRowClick(e, ev)}
              onContextMenu={(ev) => onContextMenu(e, ev)}
              className={`border-b border-border/30 cursor-pointer transition-colors ${
                isActive ? "bg-signal/10" : checked ? "bg-card/60 hover:bg-surface" : "hover:bg-surface"
              }`}
            >
              <td className="px-3 py-1.5" onClick={(ev) => { ev.stopPropagation(); onToggleCheck(e.id, ev); }}>
                <span className={`flex items-center justify-center ${checked ? "text-signal" : "text-muted-foreground"} hover:text-signal`}>
                  {checked ? <CheckSquare size={13} /> : <Square size={13} />}
                </span>
              </td>
              <td className={`px-3 py-1.5 max-w-[320px] ${blocked ? "text-muted-foreground" : "text-foreground"}`}>
                <span className="flex items-center gap-1.5 min-w-0" title={blocked ? `${strings.capture.blockedPrefix} ${e.url}` : e.url}>
                  {blocked && <Ban size={11} className="flex-shrink-0 text-destructive" />}
                  <span className={`block truncate ${blocked ? "line-through" : ""}`}>{urlName(e.url)}</span>
                </span>
              </td>
              <td className="px-3 py-1.5 text-signal whitespace-nowrap">{e.method}</td>
              <td className={`px-3 py-1.5 whitespace-nowrap font-semibold ${statusColor(e.status)}`}>{e.status ?? "—"}</td>
              <td className="px-3 py-1.5 text-muted-foreground whitespace-nowrap">{TYPE_LABEL[deriveType(e)]}</td>
              <td className={`px-3 py-1.5 whitespace-nowrap ${fulfilledColor(e.via)}`}>{fulfilledBy(e.via)}</td>
              <td className="px-3 py-1.5 text-right text-muted-foreground whitespace-nowrap">{resBodySize(e)}</td>
              <td className="px-3 py-1.5 text-muted-foreground whitespace-nowrap">
                <span className="text-foreground">{fmtTime(e.ts)}</span>
                <span className="ml-2">{fmtDur(e.durationMs)}</span>
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
