import React from "react";
import { cn } from "@/components/ui/cn";

interface SectionCardProps {
  children: React.ReactNode;
  className?: string;
}

export default function SectionCard({ children, className }: SectionCardProps) {
  return (
    <div className={cn("bg-surface border border-border rounded-lg overflow-hidden", className)}>
      {children}
    </div>
  );
}
