import React from "react";

interface SectionLabelProps {
  children: React.ReactNode;
}

export default function SectionLabel({ children }: SectionLabelProps) {
  return (
    <div className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground mb-3 px-1">
      {children}
    </div>
  );
}
