import React from "react";
import PanelHeader from "@/components/layout/PanelHeader";

interface PanelLayoutProps {
  title: string;
  subtitle?: React.ReactNode;
  actions?: React.ReactNode;
  children: React.ReactNode;
  noPadding?: boolean;
}

export default function PanelLayout({ title, subtitle, actions, children, noPadding }: PanelLayoutProps) {
  return (
    <div className="flex flex-col flex-1 overflow-hidden">
      <PanelHeader title={title} subtitle={subtitle} actions={actions} />
      <div className={`flex-1 overflow-y-auto ${noPadding ? "" : "px-4 py-5 md:px-6 md:py-6"}`}>
        {children}
      </div>
    </div>
  );
}
