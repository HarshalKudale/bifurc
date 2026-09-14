import React, { memo } from "react";
import { Folder, ArrowRight } from "@/lib/icons";
import { methodColor, methodBg } from "@/lib/utils";
import { SearchResultItem, getPanelDisplayName } from "./searchUtils";
import { Panel } from "@/lib/panelRegistry";

interface SearchResultItemViewProps {
  item: SearchResultItem;
  isSelected: boolean;
  globalMode: boolean;
  onSelect: () => void;
  onMouseEnter: () => void;
  onMapService: (e: React.MouseEvent, port: number) => void;
}

function SearchResultItemViewComponent({
  item,
  isSelected,
  globalMode,
  onSelect,
  onMouseEnter,
  onMapService,
}: SearchResultItemViewProps) {
  return (
    <div
      onClick={onSelect}
      onMouseEnter={onMouseEnter}
      className={`flex items-center gap-3 px-3 py-2 rounded-lg cursor-pointer transition-colors text-xs select-none ${
        isSelected
          ? "bg-signal/15 border-l-2 border-signal text-foreground pl-[10px]"
          : "hover:bg-surface-2 text-foreground/90 pl-3"
      }`}
    >
      {item.method ? (
        <span
          className="px-1.5 py-0.5 rounded font-mono font-bold text-[10px] flex-shrink-0 uppercase"
          style={{
            color: methodColor(item.method),
            backgroundColor: methodBg(item.method),
          }}
        >
          {item.method}
        </span>
      ) : (
        <span className="px-1.5 py-0.5 rounded font-mono font-bold text-[10px] bg-surface-2 text-muted-foreground flex-shrink-0">
          {item.entityType.toUpperCase()}
        </span>
      )}

      <div className="flex-1 min-w-0 flex flex-col justify-center">
        <div className="flex items-center gap-2">
          <span className="font-medium text-foreground truncate">{item.title}</span>
          {item.isDraft && (
            <span className="text-[9px] px-1 py-0.2 rounded bg-amber/15 text-amber border border-amber/30">
              Draft
            </span>
          )}
        </div>
        {item.subtitle && (
          <div className="text-[11px] text-muted-foreground truncate font-mono">
            {item.subtitle}
          </div>
        )}
      </div>

      {item.folderName && (
        <span className="flex items-center gap-1 text-[10px] text-muted-foreground bg-surface-2 px-1.5 py-0.5 rounded flex-shrink-0">
          <Folder size={10} />
          <span className="truncate max-w-[100px]">{item.folderName}</span>
        </span>
      )}

      {item.entityType === "service" && item.original?.port && (
        <div className="flex items-center gap-1.5 flex-shrink-0">
          {item.isMapped ? (
            <span className="px-1.5 py-0.5 text-[10px] font-medium rounded bg-emerald-500/15 text-emerald-400 border border-emerald-500/30">
              Mapped
            </span>
          ) : (
            <button
              type="button"
              onClick={(e) => onMapService(e, item.original.port)}
              title={`Map localhost:${item.original.port}`}
              className="px-2 py-0.5 text-[11px] font-semibold rounded bg-signal/15 hover:bg-signal/25 text-signal border border-signal/30 hover:border-signal/50 transition-all cursor-pointer flex items-center gap-1"
            >
              <span>Map</span>
              <ArrowRight size={10} />
            </button>
          )}
        </div>
      )}

      {item.isOpenTab ? (
        <span className="px-1.5 py-0.5 rounded text-[10px] font-semibold bg-signal/20 text-signal border border-signal/30 flex-shrink-0">
          OPEN TAB
        </span>
      ) : globalMode ? (
        <span className="px-1.5 py-0.5 rounded text-[10px] bg-surface-2 text-muted-foreground flex-shrink-0">
          {getPanelDisplayName(item.panel)}
        </span>
      ) : null}

      {isSelected && (
        <div className="flex items-center text-muted-foreground flex-shrink-0">
          <kbd className="px-1 py-0.5 text-[9px] bg-card border border-border rounded font-mono">
            ↵
          </kbd>
        </div>
      )}
    </div>
  );
}

export const SearchResultItemView = memo(SearchResultItemViewComponent);
