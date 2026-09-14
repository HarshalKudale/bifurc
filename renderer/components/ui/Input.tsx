import React from "react";
import { cn } from "@/components/ui/cn";

interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {
  error?: boolean;
  inputSize?: "sm" | "md";
}

const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ error, inputSize = "md", className, ...props }, ref) => {
    return (
      <input
        ref={ref}
        className={cn(
          "w-full bg-card border rounded-md outline-none transition-colors",
          "placeholder:text-muted-foreground/70",
          "focus-visible:ring-2 focus-visible:ring-signal/25 focus-visible:ring-offset-2 focus-visible:ring-offset-surface",
          inputSize === "sm" ? "min-h-8 text-sm px-2.5 py-1.5" : "min-h-10 text-sm px-3 py-2",
          "text-foreground",
          error ? "border-destructive" : "border-border focus:border-signal",
          className
        )}
        {...props}
      />
    );
  }
);
Input.displayName = "Input";

export default Input;
