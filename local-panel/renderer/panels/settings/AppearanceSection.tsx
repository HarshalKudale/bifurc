import React, { useState, useMemo } from "react";
import { strings } from "@/lib/strings";
import { SectionLabel, SectionCard, Switch } from "@/components/ui";
import { enabledPanels, ALWAYS_VISIBLE_PANELS, PanelEntry } from "@/lib/panelRegistry";
import { ChevronDown, ChevronRight } from "@/lib/icons";

interface AppearanceSectionProps {
  sidebarVisibility: Record<string, boolean>;
  onSidebarVisibilityChange: (id: string, visible: boolean) => void;
}

export default function AppearanceSection({
  sidebarVisibility,
  onSidebarVisibilityChange,
}: AppearanceSectionProps) {
  const [expanded, setExpanded] = useState(false);

  // Group panels by section, preserving order
  const groups = useMemo(() => {
    const map = new Map<string, PanelEntry[]>();
    for (const entry of enabledPanels) {
      const list = map.get(entry.section) ?? [];
      list.push(entry);
      map.set(entry.section, list);
    }
    return Array.from(map.entries()).map(([section, items]) => ({ section, items }));
  }, []);

  const visibleCount = enabledPanels.filter(
    (e) => ALWAYS_VISIBLE_PANELS.includes(e.id) || sidebarVisibility[e.id] !== false
  ).length;

  return (
    <section>
      <SectionLabel>{strings.settings.sectionAppearance}</SectionLabel>

      <SectionCard>
        <button
          onClick={() => setExpanded((v) => !v)}
          className="flex items-center gap-4 px-5 py-4 w-full text-left cursor-pointer hover:bg-card transition-colors"
        >
          <div className="flex-1 min-w-0">
            <div className="text-sm font-medium text-foreground">{strings.settings.sidebarPanels}</div>
            <div className="text-xs text-muted-foreground mt-0.5">
              {strings.settings.sidebarPanelsDesc} (
              {strings.settings.sidebarPanelsShown
                .replace("{visibleCount}", String(visibleCount))
                .replace("{n}", String(enabledPanels.length))}
              )
            </div>
          </div>
          <span className="flex-shrink-0 text-muted-foreground">
            {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          </span>
        </button>

        {expanded && (
          <div className="border-t border-border">
            {groups.map(({ section, items }) => (
              <div key={section}>
                <div className="text-[10px] font-semibold uppercase tracking-widest text-muted-foreground px-5 pt-3 pb-1">
                  {section}
                </div>
                {items.map((entry) => {
                  const isAlwaysVisible = ALWAYS_VISIBLE_PANELS.includes(entry.id);
                  const isVisible = isAlwaysVisible || sidebarVisibility[entry.id] !== false;
                  return (
                    <div key={entry.id} className="flex items-center gap-4 px-5 py-2.5">
                      <div className="flex-1 min-w-0">
                        <div className="text-sm text-foreground">{entry.label}</div>
                        {isAlwaysVisible && (
                          <div className="text-[10px] text-muted-foreground">
                            {strings.settings.alwaysVisible}
                          </div>
                        )}
                      </div>
                      <Switch
                        checked={isVisible}
                        ariaLabel={entry.label}
                        onChange={(v) => onSidebarVisibilityChange(entry.id, v)}
                        disabled={isAlwaysVisible}
                      />
                    </div>
                  );
                })}
              </div>
            ))}
          </div>
        )}
      </SectionCard>
    </section>
  );
}
