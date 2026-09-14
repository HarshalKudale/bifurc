import React from "react";
import { cn } from "@/components/ui/cn";

export type BadgeVariant = "accent" | "green" | "red" | "yellow" | "neutral";

interface BadgeProps {
  variant: BadgeVariant;
  dot?: boolean;
  className?: string;
  children: React.ReactNode;
}

const variantClasses: Record<BadgeVariant, string> = {
  accent:  "bg-signal/10 text-signal",
  green:   "bg-signal/10 text-signal",
  red:     "bg-destructive/10 text-destructive",
  yellow:  "bg-amber/10 text-amber",
  neutral: "bg-surface-2 text-muted-foreground",
};

export default function Badge({ variant, dot, className, children }: BadgeProps) {
  return (
    <span className={cn(
      "inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium",
      variantClasses[variant],
      className
    )}>
      {dot && "●"}
      {children}
    </span>
  );
}
