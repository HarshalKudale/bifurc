import React from "react";
import { cn } from "@/components/ui/cn";

interface SelectProps extends React.SelectHTMLAttributes<HTMLSelectElement> {
  error?: boolean;
  inputSize?: "sm" | "md";
}

const Select = React.forwardRef<HTMLSelectElement, SelectProps>(
  ({ error, inputSize = "md", className, children, ...props }, ref) => {
    return (
      <select
        ref={ref}
        className={cn(
          "w-full bg-card border rounded-md outline-none transition-colors cursor-pointer appearance-none",
          "focus-visible:ring-2 focus-visible:ring-signal/25 focus-visible:ring-offset-2 focus-visible:ring-offset-surface",
          inputSize === "sm" ? "min-h-8 text-sm px-2.5 py-1.5" : "min-h-10 text-sm px-3 py-2",
          "text-foreground",
          error ? "border-destructive" : "border-border focus:border-signal",
          className
        )}
        {...props}
      >
        {children}
      </select>
    );
  }
);
Select.displayName = "Select";

export default Select;
