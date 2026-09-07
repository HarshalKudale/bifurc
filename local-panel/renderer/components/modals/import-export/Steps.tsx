import React from "react";
import { ImportExportEntityKind, ImportExportFormatDef, CollisionStrategy } from "@/types";
import { strings } from "@/lib/strings";
import { Button } from "@/components/ui";
import { Step } from "./types";

export const KIND_LABELS: Record<ImportExportEntityKind, string> = {
  workspace: strings.importExport.kindWorkspace,
  requests: strings.importExport.kindRequests,
  mocks: strings.importExport.kindMocks,
  environments: strings.importExport.kindEnvironments,
  mappings: strings.importExport.kindMappings,
  proxyRules: strings.importExport.kindProxyRules,
  websockets: strings.importExport.kindWebsockets,
  webhooks: strings.importExport.kindWebhooks,
};

export const KIND_DESC: Record<ImportExportEntityKind, string> = {
  workspace: strings.importExport.descWorkspace,
  requests: strings.importExport.descRequests,
  mocks: strings.importExport.descMocks,
  environments: strings.importExport.descEnvironments,
  mappings: strings.importExport.descMappings,
  proxyRules: strings.importExport.descProxyRules,
  websockets: strings.importExport.descWebsockets,
  webhooks: strings.importExport.descWebhooks,
};

export const ALL_KINDS: ImportExportEntityKind[] = [
  "workspace", "requests", "mocks", "environments",
  "mappings", "proxyRules", "websockets", "webhooks",
];

const COLLISION_LABELS: Record<CollisionStrategy, string> = {
  keep: strings.importExport.collisionKeepLabel,
  override: strings.importExport.collisionOverrideLabel,
  new: strings.importExport.collisionNewLabel,
};
const COLLISION_DESC: Record<CollisionStrategy, string> = {
  keep: strings.importExport.collisionKeepDesc,
  override: strings.importExport.collisionOverrideDesc,
  new: strings.importExport.collisionNewDesc,
};

export function StepSelectKind({ kinds, onSelect }: { kinds: ImportExportEntityKind[]; onSelect(k: ImportExportEntityKind): void }) {
  return (
    <div>
      <p className="text-xs text-muted-foreground mb-3">{strings.importExport.whatToWorkWith}</p>
      <div className="grid grid-cols-2 gap-2">
        {kinds.map((k) => (
          <button
            key={k}
            onClick={() => onSelect(k)}
            className="text-left px-4 py-3 rounded border border-border bg-card hover:bg-surface-2 hover:border-signal/40 transition-colors"
          >
            <div className="text-sm font-medium text-foreground">{KIND_LABELS[k]}</div>
            <div className="text-xs text-muted-foreground mt-0.5 line-clamp-1">{KIND_DESC[k]}</div>
          </button>
        ))}
      </div>
    </div>
  );
}

export function StepSelectFormat({
  mode, kind, formats, selected, onSelect, onBack, onConfirm,
}: {
  mode: "import" | "export";
  kind: ImportExportEntityKind;
  formats: ImportExportFormatDef[];
  selected: string | null;
  onSelect(f: string): void;
  onBack(): void;
  onConfirm(): void;
}) {
  return (
    <div>
      <p className="text-xs text-muted-foreground mb-3">
        {mode === "export" ? strings.importExport.chooseExportFormatFor : strings.importExport.chooseImportFormatFor}{" "}
        <span className="text-foreground font-medium">{KIND_LABELS[kind]}</span>
      </p>
      {formats.length === 0 ? (
        <p className="text-sm text-muted-foreground py-4 text-center">{strings.importExport.noFormatsAvailable}</p>
      ) : (
        <div className="space-y-1.5">
          {formats.map((f) => (
            <label
              key={f.id}
              className={`flex items-center gap-3 px-4 py-3 rounded border cursor-pointer transition-colors ${selected === f.id
                ? "border-signal/50 bg-signal/10"
                : "border-border bg-card hover:bg-surface-2"
                }`}
            >
              <input
                type="radio"
                name="format"
                value={f.id}
                checked={selected === f.id}
                onChange={() => onSelect(f.id)}
                className="accent-signal"
              />
              <div>
                <div className="text-sm font-medium text-foreground">{f.label}</div>
                <div className="text-xs text-muted-foreground">.{f.extensions.join(", .")}</div>
              </div>
            </label>
          ))}
        </div>
      )}
      <div className="flex justify-between mt-5 pt-4 border-t border-border">
        <Button variant="secondary" onClick={onBack}>{strings.importExport.back}</Button>
        <Button
          variant="primary"
          onClick={onConfirm}
          disabled={!selected}
        >
          {mode === "export" ? strings.importExport.exportEllipsis : strings.importExport.chooseFile}
        </Button>
      </div>
    </div>
  );
}

export function StepCollision({
  step, strategy, onSetStrategy, onBack, onConfirm,
}: {
  step: Extract<Step, { name: "collision" }>;
  strategy: CollisionStrategy;
  onSetStrategy(s: CollisionStrategy): void;
  onBack(): void;
  onConfirm(): void;
}) {
  return (
    <div>
      <div className="bg-amber/10 border border-amber/30 rounded px-4 py-3 mb-4">
        <p className="text-sm text-amber font-medium">
          {strings.importExport.collisionSummary.replace("{count}", String(step.collisionCount)).replace("{total}", String(step.itemCount))}
        </p>
      </div>
      <p className="text-xs text-muted-foreground mb-3">{strings.importExport.howConflictsHandled}</p>
      <div className="space-y-2">
        {(["keep", "override", "new"] as CollisionStrategy[]).map((s) => (
          <label key={s} className={`flex items-start gap-3 px-4 py-3 rounded border cursor-pointer transition-colors ${strategy === s ? "border-signal/50 bg-signal/10" : "border-border bg-card hover:bg-surface-2"
            }`}>
            <input
              type="radio"
              name="collision"
              value={s}
              checked={strategy === s}
              onChange={() => onSetStrategy(s)}
              className="mt-0.5 accent-signal"
            />
            <div>
              <div className="text-sm font-medium text-foreground">{COLLISION_LABELS[s]}</div>
              <div className="text-xs text-muted-foreground">{COLLISION_DESC[s]}</div>
            </div>
          </label>
        ))}
      </div>
      <div className="flex justify-between mt-5 pt-4 border-t border-border">
        <Button variant="secondary" onClick={onBack}>{strings.importExport.back}</Button>
        <Button variant="primary" onClick={onConfirm}>{strings.importExport.import}</Button>
      </div>
    </div>
  );
}

export function StepDone({ step, onClose }: { step: Extract<Step, { name: "done" }>; onClose(): void }) {
  return (
    <div>
      <div className={`px-4 py-3 rounded border text-sm mb-5 ${step.ok ? "border-signal/30 bg-signal/10 text-signal" : "border-destructive/30 bg-destructive/10 text-destructive"
        }`}>
        {step.message}
      </div>
      <div className="flex justify-end">
        <Button variant="primary" onClick={onClose}>{strings.common.close}</Button>
      </div>
    </div>
  );
}
