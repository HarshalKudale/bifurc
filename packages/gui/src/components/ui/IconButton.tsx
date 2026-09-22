import React from "react";
import { cn } from "@/components/ui/cn";

interface IconButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  icon: React.ReactNode;
}

const IconButton = React.forwardRef<HTMLButtonElement, IconButtonProps>(
  ({ icon, className, ...props }, ref) => {
    return (
      <button
        ref={ref}
        className={cn(
          "w-7 h-7 flex items-center justify-center rounded border border-border",
          "bg-card hover:bg-surface-2 text-muted-foreground hover:text-foreground transition-colors cursor-pointer",
          "disabled:opacity-40 disabled:cursor-not-allowed",
          className
        )}
        {...props}
      >
        {icon}
      </button>
    );
  }
);
IconButton.displayName = "IconButton";

export default IconButton;
