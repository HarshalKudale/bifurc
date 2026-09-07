import React from "react";

interface PanelHeaderProps {
  title: string;
  subtitle?: React.ReactNode;
  actions?: React.ReactNode;
}

export default function PanelHeader({ title, subtitle, actions }: PanelHeaderProps) {
  return (
    <div className="px-6 py-4 border-b border-border flex flex-wrap items-start gap-3 flex-shrink-0 md:flex-nowrap md:items-center">
      <div className="min-w-0 flex-1">
        <h1 className="text-lg font-semibold text-foreground">{title}</h1>
        {subtitle && <p className="text-xs text-muted-foreground mt-0.5">{subtitle}</p>}
      </div>
      {actions && <div className="ml-auto flex items-center gap-2 flex-wrap">{actions}</div>}
    </div>
  );
}
