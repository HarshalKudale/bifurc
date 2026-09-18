# Handler census — raw output

Captured 2026-09-15, before any P1+ change. Reproduce with the commands in
[`../13-checklist.md`](../13-checklist.md) § "If you do nothing else today".

> **Paths below were updated to current file names** on 2026-09-16 (`src/ipc/importExport/index.ts`
> → `src/ipc/importExportHandlers.ts`), so that a reader who greps a path from this document finds
> something. The **counts and line numbers are the 2026-09-15 snapshot** and are deliberately not
> re-measured — the document's value is that it is a fixed before-picture.

## `handlers.txt` — `ipcMain.handle` sites

```
grep -rn "ipcMain.handle" src --include=*.ts
```

**112 lines — matches the expected count exactly.** By file:

| Count | File |
|---:|---|
| 20 | `src/ipc/handlers/coreHandlers.ts` |
| 19 | `src/ipc/handlers/syncHandlers.ts` |
| 14 | `src/ipc/handlers/systemHandlers.ts` |
| 10 | `src/ipc/applicationHandlers.ts` |
| 8 | `src/ipc/handlers/grpcHandlers.ts` |
| 7 | `src/ipc/handlers/tlsHandlers.ts` |
| 6 | `src/ipc/handlers/runnerHandlers.ts` |
| 5 | `src/ipc/handlers/soapHandlers.ts` |
| 5 | `src/ipc/handlers/graphqlHandlers.ts` |
| 5 | `src/ipc/handlers/crudHandlers.ts` |
| 4 | `src/ipc/importExportHandlers.ts` |
| 4 | `src/ipc/handlers/folderHandlers.ts` |
| 3 | `src/ipc/handlers/entityCrudFactory.ts` (generates ~36 of the 112 at runtime) |
| 2 | `src/main.ts` |

## `coupling.txt` — Electron coupling outside `main.ts`

```
grep -rn "BrowserWindow\|app.getPath\|dialog\.\|shell\." src --include=*.ts | grep -v src/main.ts
```

**42 lines — one more than the ~27 the plan estimated.** The plan's figure counted *sites that
actually need mediating*; this grep also catches `import` statements and type annotations. Two
adjustments:

1. **One false positive.** `src/proxy/service-discovery.ts:13` matches on the literal Windows path
   `"...\\WindowsPowerShell\\v1.0\\powershell.exe"` — the `shell.` is inside a string, not an
   Electron API. **Real count: 41.**
2. Several of the 41 are `import { BrowserWindow }` lines whose only use is the broadcast pattern,
   which P2 replaces with the EventBus. The genuinely distinct work items are:

| File | What couples it | P2 disposition |
|---|---|---|
| `src/ipc/handlers.ts` | 4× `BrowserWindow.getAllWindows()` broadcast | EventBus |
| `src/ipc/handlers/coreHandlers.ts` | 1× broadcast + `@/main` import cycle | EventBus; invert the cycle |
| `src/ipc/handlers/entityCrudFactory.ts` | 2× broadcast | EventBus |
| `src/ipc/handlers/folderHandlers.ts` | 1× broadcast | EventBus |
| `src/ipc/handlers/syncHandlers.ts` | 1× broadcast + 1× `showSaveDialog` | EventBus; dialog → blob layer (P3) |
| `src/ipc/handlers/systemHandlers.ts` | 1× broadcast, 1× `shell.openExternal`, 3× `showOpenDialog`, 1× `showSaveDialog`, 1× `getFocusedWindow` | split: dialogs/shell → CLIENT, rest → ENGINE |
| `src/ipc/handlers/tlsHandlers.ts` | 1× `showSaveDialog`, 2× `showOpenDialog` | blob layer (P3); `installCA` → CLIENT |
| `src/ipc/handlers/runnerHandlers.ts` | 1× `showSaveDialog` | blob layer (P3) |
| `src/ipc/importExportHandlers.ts` | 1× `showSaveDialog`, 1× `showOpenDialog` | blob layer (P3) |
| `src/proxy/webhookServer.ts` | 1× broadcast | EventBus |
| `src/companion/companionServer.ts` | 2× `BrowserWindow.getAllWindows()` | EventBus; also generalises into the transport (P4) |
| `src/applications/processSpawner.ts` | holds a `mainWindow` reference | drop the reference; use the EventBus |
| `src/store/appSettings.ts` | `app.getPath("userData")` | inject the data dir (P2) |
| `src/store/workspaceFs.ts` | `app.getPath("userData")` | inject the data dir (P2) |

Every `dialog.*` site above is an **egress/ingress channel** already enumerated in P3 § "Rewire 5
egress channels / 4 ingress channels". The two `app.getPath` sites are the whole of the data-dir
coupling.
