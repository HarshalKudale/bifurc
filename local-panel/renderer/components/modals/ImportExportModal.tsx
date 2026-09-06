import React, { useEffect, useReducer } from "react";
import Modal from "@/components/common/Modal";
import { strings } from "@/lib/strings";
import {
  ImportExportEntityKind, CollisionStrategy,
  ImportExportFormatsMap, ImportExportFormatDef,
  ExportRequest, PreflightRequest, ImportRequest,
} from "@/types";
import { ALL_KINDS, StepSelectKind, StepSelectFormat, StepCollision, StepDone } from "./import-export/Steps";
import { Step } from "./import-export/types";

interface State {
  step: Step;
  kind: ImportExportEntityKind | null;
  format: string | null;
  collisionStrategy: CollisionStrategy;
}

type Action =
  | { type: "selectKind"; kind: ImportExportEntityKind }
  | { type: "selectFormat"; format: string }
  | { type: "setStrategy"; strategy: CollisionStrategy }
  | { type: "working" }
  | { type: "collision"; filePath: string; itemCount: number; collisionCount: number }
  | { type: "done"; ok: boolean; message: string }
  | { type: "back" }
  | { type: "reset" };

function reducer(state: State, action: Action): State {
  switch (action.type) {
    case "selectKind":
      return { ...state, kind: action.kind, format: null, step: { name: "selectFormat" } };
    case "selectFormat":
      return { ...state, format: action.format };
    case "setStrategy":
      return { ...state, collisionStrategy: action.strategy };
    case "working":
      return { ...state, step: { name: "working" } };
    case "collision":
      return { ...state, step: { name: "collision", filePath: action.filePath, itemCount: action.itemCount, collisionCount: action.collisionCount } };
    case "done":
      return { ...state, step: { name: "done", ok: action.ok, message: action.message } };
    case "back":
      if (state.step.name === "selectFormat") return { ...state, step: { name: "selectKind" } };
      if (state.step.name === "collision") return { ...state, step: { name: "selectFormat" } };
      return state;
    case "reset":
      return initialState;
    default:
      return state;
  }
}

const initialState: State = {
  step: { name: "selectKind" },
  kind: null,
  format: null,
  collisionStrategy: "keep",
};

interface Props {
  open: boolean;
  mode: "import" | "export";
  wsId: string;
  onClose(): void;
  onImportDone?(): void;
}

export default function ImportExportModal({ open, mode, wsId, onClose, onImportDone }: Props) {
  const [state, dispatch] = useReducer(reducer, initialState);
  const [formats, setFormats] = React.useState<ImportExportFormatsMap>({});

  useEffect(() => {
    if (open) {
      dispatch({ type: "reset" });
      window.api.getImportExportFormats().then(setFormats).catch(() => setFormats({}));
    }
  }, [open]);

  const title = mode === "export" ? strings.importExport.exportData : strings.importExport.importData;
  const kindFormats: ImportExportFormatDef[] = state.kind
    ? (formats[state.kind] ?? []).filter((f) =>
      mode === "export" ? f.supportsExport : f.supportsImport,
    )
    : [];

  async function handleExport() {
    if (!state.kind || !state.format) return;
    dispatch({ type: "working" });
    const req: ExportRequest = { kind: state.kind, format: state.format, wsId };
    const res = await window.api.exportData(req);
    if (res.canceled) { dispatch({ type: "reset" }); return; }
    dispatch({ type: "done", ok: res.ok, message: res.ok ? strings.importExport.exportComplete : (res.error ?? strings.importExport.exportFailed) });
  }

  async function handleImportPreflight() {
    if (!state.kind || !state.format) return;
    dispatch({ type: "working" });
    const req: PreflightRequest = { kind: state.kind, format: state.format, wsId };
    const res = await window.api.preflightImport(req);
    if (res.canceled) { dispatch({ type: "reset" }); return; }
    if (!res.ok) {
      dispatch({ type: "done", ok: false, message: res.error ?? strings.importExport.couldNotReadFile });
      return;
    }
    const collisionCount = res.collisionIds?.length ?? 0;
    if (collisionCount > 0) {
      dispatch({ type: "collision", filePath: res.filePath!, itemCount: res.itemCount ?? 0, collisionCount });
    } else {
      await applyImport(res.filePath!, "keep");
    }
  }

  async function applyImport(filePath: string, strategy: CollisionStrategy) {
    if (!state.kind || !state.format) return;
    dispatch({ type: "working" });
    const req: ImportRequest = { kind: state.kind, format: state.format, wsId, filePath, collisionStrategy: strategy };
    const res = await window.api.importData(req);
    if (res.ok) {
      onImportDone?.();
      const n = res.imported ?? 0;
      const imported = strings.importExport.importedItems.replace("{n}", String(n)).replace("{s}", n !== 1 ? "s" : "");
      const skipped = res.skipped ? strings.importExport.skippedSuffix.replace("{n}", String(res.skipped)) : "";
      dispatch({ type: "done", ok: true, message: `${imported}${skipped}.` });
    } else {
      dispatch({ type: "done", ok: false, message: res.error ?? strings.importExport.importFailed });
    }
  }

  function handleClose() {
    dispatch({ type: "reset" });
    onClose();
  }

  return (
    <>
      <Modal open={open} title={title} onClose={handleClose}>
        {state.step.name === "selectKind" && (
          <StepSelectKind kinds={ALL_KINDS} onSelect={(k) => dispatch({ type: "selectKind", kind: k })} />
        )}
        {state.step.name === "selectFormat" && state.kind && (
          <StepSelectFormat
            mode={mode}
            kind={state.kind}
            formats={kindFormats}
            selected={state.format}
            onSelect={(f) => dispatch({ type: "selectFormat", format: f })}
            onBack={() => dispatch({ type: "back" })}
            onConfirm={mode === "export" ? handleExport : handleImportPreflight}
          />
        )}
        {state.step.name === "collision" && (
          <StepCollision
            step={state.step as Extract<Step, { name: "collision" }>}
            strategy={state.collisionStrategy}
            onSetStrategy={(s) => dispatch({ type: "setStrategy", strategy: s })}
            onBack={() => dispatch({ type: "back" })}
            onConfirm={() => {
              const s = state.step as Extract<Step, { name: "collision" }>;
              applyImport(s.filePath, state.collisionStrategy);
            }}
          />
        )}
        {state.step.name === "working" && (
          <div className="flex items-center gap-3 py-6">
            <span className="inline-block w-5 h-5 border-2 border-signal border-t-transparent rounded-full animate-spin flex-shrink-0" />
            <span className="text-sm text-muted-foreground">{mode === "export" ? strings.importExport.exporting : strings.importExport.importing}</span>
          </div>
        )}
        {state.step.name === "done" && (
          <StepDone step={state.step as Extract<Step, { name: "done" }>} onClose={handleClose} />
        )}
      </Modal>
    </>
  );
}
