import React from "react";

const colorVar: Record<string, string> = {
  green:  "var(--c-signal)",
  yellow: "var(--c-amber)",
  red:    "var(--c-destructive)",
  accent: "var(--c-signal)",
};

interface ActiveDotProps {
  active: boolean;
  color?: "green" | "yellow" | "red" | "accent";
  size?: "sm" | "md";
  className?: string;
}

export default function ActiveDot({ active, color = "green", size = "sm", className }: ActiveDotProps) {
  const px = size === "sm" ? 8 : 10;
  const bg = active ? colorVar[color] : "var(--c-muted-foreground)";
  const shadow = active ? `0 0 5px ${colorVar[color]}` : "none";
  const opacity = active ? 1 : 0.45;
  return (
    <span
      className={className}
      style={{
        display: "inline-block",
        flexShrink: 0,
        width: px,
        height: px,
        borderRadius: "50%",
        background: bg,
        boxShadow: shadow,
        opacity,
        transition: "background 0.15s, box-shadow 0.15s, opacity 0.15s",
      }}
    />
  );
}
