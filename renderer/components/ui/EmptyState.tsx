import React from "react";
import { cn } from "@/components/ui/cn";

interface EmptyStateProps {
  icon: React.ReactNode;
  title: string;
  description?: React.ReactNode;
  action?: React.ReactNode;
  className?: string;
  fill?: boolean;
}

export default function EmptyState({ icon, title, description, action, className, fill }: EmptyStateProps) {
  return (
    <div className={cn(
      "flex flex-col items-center justify-center text-center",
      fill ? "h-full py-16" : "bg-surface border border-border rounded-lg py-16",
      className
    )}>
      <div className="opacity-15 mb-3">{icon}</div>
      <div className="text-sm font-medium text-foreground mb-1">{title}</div>
      {description && <p className="text-xs text-muted-foreground">{description}</p>}
      {action && <div className="mt-4">{action}</div>}
    </div>
  );
}
