import React from "react";
import { cn } from "@/components/ui/cn";

interface SidebarFooterProps {
  children: React.ReactNode;
  className?: string;
}

export default function SidebarFooter({ children, className }: SidebarFooterProps) {
  return (
    <div className={cn("px-2 py-1.5 border-t border-border flex items-center gap-1 flex-shrink-0", className)}>
      {children}
    </div>
  );
}
