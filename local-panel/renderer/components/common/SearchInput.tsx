import React from "react";
import { Search } from "@/lib/icons";
import { strings } from "@/lib/strings";
import { Input } from "@/components/ui";

interface Props {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
}

export default function SearchInput({ value, onChange, placeholder = strings.sidebar.searchPlaceholderDefault }: Props) {
  return (
    <div className="relative">
      <span className="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground pointer-events-none select-none flex items-center"><Search size={14} /></span>
      <Input
        inputSize="md"
        aria-label={placeholder}
        className="w-56 pl-9 pr-3"
        placeholder={placeholder}
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
    </div>
  );
}
