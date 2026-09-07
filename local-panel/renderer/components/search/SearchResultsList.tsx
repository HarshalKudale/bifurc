import React, { forwardRef, MutableRefObject } from "react";
import { Search } from "@/lib/icons";
import { SearchSection, SearchResultItem } from "./searchUtils";
import { SearchResultItemView } from "./SearchResultItemView";

interface SearchResultsListProps {
  sections: SearchSection[];
  flatItems: SearchResultItem[];
  activeIndex: number;
  mode: "current" | "global";
  debouncedQuery: string;
  currentScreenName: string;
  itemRefs: MutableRefObject<(HTMLDivElement | null)[]>;
  onSelect: (item: SearchResultItem) => void;
  onHover: (index: number) => void;
  onMapService: (e: React.MouseEvent, port: number) => void;
}

export const SearchResultsList = forwardRef<HTMLDivElement, SearchResultsListProps>(
  ({ sections, flatItems, activeIndex, mode, debouncedQuery, currentScreenName, itemRefs, onSelect, onHover, onMapService }, ref) => {
    if (flatItems.length === 0) {
      return (
        <div ref={ref} className="flex-1 min-h-0 overflow-y-auto divide-y divide-border/30 h-full flex flex-col items-center justify-center p-8 text-center">
          <div className="inline-flex items-center justify-center w-10 h-10 rounded-full bg-surface-2 text-muted-foreground mb-3">
            <Search size={18} />
          </div>
          <div className="text-sm font-medium text-foreground">
            {debouncedQuery.trim()
              ? `No entities found matching '${debouncedQuery.trim()}'`
              : mode === "current"
              ? `No open tabs or entities in ${currentScreenName}`
              : "Type to search entities across workspace"}
          </div>
          <div className="text-xs text-muted-foreground mt-1">
            {debouncedQuery.trim()
              ? mode === "current"
                ? "Press Tab to switch to Global Search"
                : "Try searching for another name, method, or URL"
              : "Search requests, mocks, proxy rules, sockets, webhooks, and services"}
          </div>
        </div>
      );
    }

    let globalFlatIndexCounter = 0;

    return (
      <div ref={ref} className="flex-1 min-h-0 overflow-y-auto divide-y divide-border/30">
        {sections.map((sec) => (
          <div key={sec.title} className="py-2">
            <div className="px-4 py-1 flex items-center justify-between text-[11px] font-semibold tracking-wider text-muted-foreground uppercase">
              <span>{sec.title}</span>
              <span className="text-[10px] font-normal lowercase opacity-70">
                {sec.items.length} {sec.items.length === 1 ? "result" : "results"}
              </span>
            </div>
            <div className="mt-1 space-y-0.5 px-2">
              {sec.items.map((item) => {
                const thisIndex = globalFlatIndexCounter++;
                const isSelected = thisIndex === activeIndex;
                return (
                  <div
                    key={item.id + item.section}
                    ref={(el) => {
                      itemRefs.current[thisIndex] = el;
                    }}
                  >
                    <SearchResultItemView
                      item={item}
                      isSelected={isSelected}
                      globalMode={mode === "global"}
                      onSelect={() => onSelect(item)}
                      onMouseEnter={() => onHover(thisIndex)}
                      onMapService={onMapService}
                    />
                  </div>
                );
              })}
            </div>
          </div>
        ))}
      </div>
    );
  }
);
