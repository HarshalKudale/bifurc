import React from "react";
import { ChevronDown } from "@/lib/icons";
import { strings } from "@/lib/strings";
import { TabType } from "./restTabReducer";

export interface RestCurlImportProps {
  tabType: TabType;
  showCurl: boolean;
  curlInput: string;
  onToggleShowCurl: () => void;
  onCurlChange: (v: string) => void;
}

export function RestCurlImport({
  tabType, showCurl, curlInput, onToggleShowCurl, onCurlChange
}: RestCurlImportProps) {
  if (tabType === "request") {
    return (
      <div className="px-4 flex-shrink-0 border-b border-border bg-background/30">
        <button
          onClick={onToggleShowCurl}
          className="flex items-center gap-1.5 py-2 text-[10px] font-semibold uppercase tracking-widest text-muted-foreground hover:text-foreground cursor-pointer transition-colors"
        >
          <span style={{ display: "flex", alignItems: "center", transition: "transform 0.15s ease", transform: showCurl ? "rotate(0deg)" : "rotate(-90deg)" }}>
            <ChevronDown size={10} />
          </span>
          {strings.requests.importFromCurl}
        </button>
        {showCurl && (
          <textarea
            className="w-full bg-card border border-border focus:border-signal rounded px-3 py-2 text-xs font-mono text-foreground outline-none resize-none placeholder:text-muted-foreground/50 mb-2"
            rows={3}
            placeholder={strings.requests.curlPlaceholder}
            value={curlInput}
            onChange={(e) => onCurlChange(e.target.value)}
            spellCheck={false}
          />
        )}
      </div>
    );
  } else {
    return (
      <div className="px-4 py-3 border-b border-border flex-shrink-0 bg-background/30">
        <div className="flex items-center gap-2 mb-2">
          <span className="text-[10px] font-semibold uppercase tracking-widest text-muted-foreground">{strings.mocks.importFromCurl}</span>
          <span className="text-[10px] text-muted-foreground opacity-60">{strings.mocks.importFromCurlHint}</span>
        </div>
        <textarea
          className="w-full bg-card border border-border focus:border-signal rounded px-3 py-2 text-xs font-mono text-foreground outline-none resize-none placeholder:text-muted-foreground/50 transition-colors"
          rows={3}
          placeholder={strings.mocks.curlPlaceholder}
          value={curlInput}
          onChange={(e) => onCurlChange(e.target.value)}
          spellCheck={false}
        />
      </div>
    );
  }
}
