import React from "react";
import { cn } from "@/components/ui/cn";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type ButtonSize = "sm" | "md";

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant: ButtonVariant;
  size?: ButtonSize;
  icon?: React.ReactNode;
}

const variantClasses: Record<ButtonVariant, string> = {
  primary:   "bg-signal hover:bg-signal/80 text-background font-medium shadow-sm",
  secondary: "border border-border bg-card hover:bg-surface-2 text-muted-foreground hover:text-foreground font-medium",
  ghost:     "text-muted-foreground hover:text-foreground hover:bg-card",
  danger:    "text-destructive hover:bg-destructive/10",
};

const sizeClasses: Record<ButtonSize, string> = {
  sm: "min-h-8 px-3 py-1.5 text-xs",
  md: "min-h-10 px-4 py-2 text-sm",
};

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ variant, size = "md", icon, children, className, ...props }, ref) => {
    return (
      <button
        ref={ref}
        className={cn(
          "inline-flex items-center justify-center rounded-md transition-all cursor-pointer whitespace-nowrap",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-signal/35 focus-visible:ring-offset-2 focus-visible:ring-offset-surface",
          "disabled:opacity-40 disabled:cursor-not-allowed",
          variantClasses[variant],
          sizeClasses[size],
          icon && children && "flex items-center gap-1.5",
          className
        )}
        {...props}
      >
        {icon}
        {children}
      </button>
    );
  }
);
Button.displayName = "Button";

export default Button;
