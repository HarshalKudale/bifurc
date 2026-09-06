import React from "react";

interface Props {
  id: string;
  label: string;
  icon: React.ReactNode;
  active: boolean;
  badge?: number;
  collapsed?: boolean;
  onClick: () => void;
}

export default function NavItem({ id, label, icon, active, badge, onClick }: Props) {
  // Support both ID and normalized label for data-testid
  const testId = `nav-${id ?? label.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "")}`;

  return (
    <button
      type="button"
      data-testid={testId}
      onClick={onClick}
      title={label}
      aria-label={label}
      className={`group relative flex flex-col items-center justify-center w-full min-h-[58px] py-2 px-1 rounded-lg transition-all duration-150 cursor-pointer select-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-signal/35 ${
        active
          ? "bg-signal/15 text-signal border border-signal/25"
          : "text-muted-foreground hover:bg-surface-2/70 hover:text-foreground border border-transparent"
      }`}
    >
      {/* Active left indicator bar */}
      {active && (
        <span
          className="absolute left-0 top-1.5 bottom-1.5 w-[3px] rounded-r bg-signal"
          aria-hidden="true"
        />
      )}

      {/* Big icon on top */}
      <span
        className={`flex items-center justify-center flex-shrink-0 mb-1 transition-colors [&>svg]:w-5 [&>svg]:h-5 ${
          active ? "text-signal" : "text-muted-foreground group-hover:text-foreground"
        }`}
      >
        {icon}
      </span>

      {/* Title below icon */}
      <span
        className={`text-[10px] font-medium leading-tight text-center truncate max-w-full px-0.5 tracking-tight transition-colors ${
          active ? "text-signal font-semibold" : "text-muted-foreground group-hover:text-foreground"
        }`}
      >
        {label}
      </span>

      {/* Top-right badge count */}
      {badge !== undefined && badge > 0 && (
        <span
          className="absolute top-1 right-1 min-w-[15px] h-3.5 px-1 rounded-full text-[9px] font-bold flex items-center justify-center bg-signal text-background shadow-sm leading-none"
        >
          {badge}
        </span>
      )}
    </button>
  );
}
