import React from "react";

interface FormFieldProps {
  label: string;
  htmlFor?: string;
  description?: string;
  error?: string;
  children: React.ReactNode;
}

export default function FormField({ label, htmlFor, description, error, children }: FormFieldProps) {
  return (
    <div className="mb-5">
      {label && (
        <label htmlFor={htmlFor} className="block text-sm font-medium text-foreground mb-1.5">
          {label}
        </label>
      )}
      {description && <p className="text-xs text-muted-foreground mb-2">{description}</p>}
      {children}
      {error && <p className="text-xs text-destructive mt-1">{error}</p>}
    </div>
  );
}
