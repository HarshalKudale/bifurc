import React from "react";
import { cn } from "@/components/ui/cn";

export interface TableColumn<T> {
  key: string;
  header?: string;
  width?: string;
  align?: "left" | "center" | "right";
  headerClassName?: string;
  render: (row: T, index: number) => React.ReactNode;
}

interface DataTableProps<T> {
  columns: TableColumn<T>[];
  data: T[];
  rowKey: (row: T) => string;
  onRowClick?: (row: T) => void;
  rowClassName?: (row: T) => string;
  emptyState?: React.ReactNode;
  stickyHeader?: boolean;
  compact?: boolean;
  className?: string;
}

export default function DataTable<T>({
  columns,
  data,
  rowKey,
  onRowClick,
  rowClassName,
  emptyState,
  stickyHeader,
  compact,
  className,
}: DataTableProps<T>) {
  const cellPad = compact ? "px-3 py-1.5" : "px-4 py-2.5";
  const headerFontSize = compact ? "text-[10px]" : "text-[11px]";

  if (data.length === 0 && emptyState) {
    return <>{emptyState}</>;
  }

  return (
    <div className={cn("bg-surface border border-border rounded-lg overflow-hidden", className)}>
      <table className="w-full border-collapse">
        <thead className={cn(stickyHeader && "sticky top-0 bg-background z-10")}>
          <tr className="border-b border-border">
            {columns.map((col) => (
              <th
                key={col.key}
                className={cn(
                  col.align === "center" && "text-center",
                  col.align === "right" && "text-right",
                  (!col.align || col.align === "left") && col.header && "text-left",
                  col.width,
                  col.header && `${headerFontSize} font-semibold uppercase tracking-wider text-muted-foreground ${cellPad}`,
                  !col.header && cellPad,
                  col.headerClassName
                )}
              >
                {col.header}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {data.map((row, index) => (
            <tr
              key={rowKey(row)}
              className={cn(
                "border-b border-border/50 last:border-0 hover:bg-card transition-colors",
                onRowClick && "cursor-pointer",
                rowClassName?.(row)
              )}
              onClick={onRowClick ? () => onRowClick(row) : undefined}
            >
              {columns.map((col) => (
                <td
                  key={col.key}
                  className={cn(
                    cellPad,
                    col.align === "center" && "text-center",
                    col.align === "right" && "text-right"
                  )}
                >
                  {col.render(row, index)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
