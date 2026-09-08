import React from "react";
import { Panel, PanelEntry } from "@/lib/panelRegistry";
import NavItem from "@/components/sidebar/NavItem";
import { Settings } from "@/lib/icons";
import { strings } from "@/lib/strings";

interface Props {
    entries: PanelEntry[];
    activePanel: Panel;
    onPanelSelect: (id: Panel) => void;
    badges: Partial<Record<Panel, number | undefined>>;
    collapsed?: boolean;
}

export default function AppSidebar({
    entries,
    activePanel,
    onPanelSelect,
    badges,
}: Props) {
    return (
        <div className="flex flex-col h-full overflow-hidden">
            {/* Flat list of individual nav items (scrollable) */}
            <div className="flex-1 overflow-y-auto overflow-x-hidden p-1.5 flex flex-col items-center gap-1.5">
                {entries.map((n) => (
                    <NavItem
                        key={n.id}
                        id={n.id}
                        label={n.label}
                        icon={n.icon}
                        active={activePanel === n.id}
                        badge={badges[n.id]}
                        onClick={() => onPanelSelect(n.id)}
                    />
                ))}
            </div>

            {/* Bottom pinned section - only shows global settings */}
            <div className="flex-shrink-0 border-t border-border bg-surface p-1.5 flex flex-col items-center relative w-full">
                <button
                    type="button"
                    className={`w-full min-h-[40px] py-1 px-1 flex flex-col items-center justify-center rounded-lg transition-colors cursor-pointer select-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-signal/35 ${
                        activePanel === "settings"
                            ? "bg-signal/15 text-signal border border-signal/25"
                            : "text-muted-foreground hover:text-foreground hover:bg-surface-2/70 border border-transparent"
                    }`}
                    onClick={() => onPanelSelect("settings")}
                    title={strings.nav.settings}
                    aria-label={strings.nav.settings}
                    style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
                >
                    <Settings size={18} />
                    <span className="text-[9px] font-medium leading-tight mt-0.5 tracking-tight truncate max-w-full">
                        {strings.nav.settings}
                    </span>
                </button>
            </div>
        </div>
    );
}
