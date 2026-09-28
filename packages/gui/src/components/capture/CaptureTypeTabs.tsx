import React from "react";
import { strings } from "@/lib/strings";
import { CaptureType } from "./captureUtils";

export type TypeFilter = CaptureType | "all";

const TABS: { key: TypeFilter; label: string }[] = [
  { key: "all", label: strings.capture.typeAll },
  { key: "xhr", label: strings.capture.typeXhr },
  { key: "doc", label: strings.capture.typeDoc },
  { key: "css", label: strings.capture.typeCss },
  { key: "js", label: strings.capture.typeJs },
  { key: "font", label: strings.capture.typeFont },
  { key: "img", label: strings.capture.typeImg },
  { key: "media", label: strings.capture.typeMedia },
  { key: "other", label: strings.capture.typeOther },
];

interface Props {
  active: TypeFilter;
  counts: Record<TypeFilter, number>;
  onChange: (t: TypeFilter) => void;
}

export default function CaptureTypeTabs({ active, counts, onChange }: Props) {
  return (
    <div className="flex items-center gap-1 px-3 py-1.5 border-b border-border flex-shrink-0 overflow-x-auto">
      {TABS.map((t) => {
        const n = counts[t.key] ?? 0;
        const isActive = active === t.key;
        return (
          <button
            key={t.key}
            onClick={() => onChange(t.key)}
            className={`px-2.5 py-1 rounded text-xs font-medium whitespace-nowrap transition-colors cursor-pointer ${
              isActive ? "bg-signal/15 text-signal" : "text-muted-foreground hover:text-foreground hover:bg-card"
            }`}
          >
            {t.label}
            {t.key !== "all" && n > 0 && <span className="ml-1 text-[10px] text-muted-foreground">{n}</span>}
          </button>
        );
      })}
    </div>
  );
}
