# Codebase Cleanup — Dead Code, Duplicates, Abstractions, File Splitting & Anti-Patterns

This plan covers all 5 cleanup goals across both **renderer/** (131 files) and **src/** (76 files). Each phase is independent and ordered by risk (lowest first).

---

## Phase 1 — Remove Dead & Orphaned Code

Safe deletions. Zero behavioral change.

### 1.1 Delete Dead Panel Files

These 7 files are imported in `panelFactory.tsx` but **never rendered** (all routing goes through the unified `RequestsPanel` / `MocksPanel`). `LogsPanel` is not even imported anywhere.

#### [DELETE] [GraphQLRequestsPanel.tsx](file:///i:/workspace/Bifurc-Worskspace/Bifurc/bifurc/renderer/panels/GraphQLRequestsPanel.tsx)
#### [DELETE] [GraphQLMocksPanel.tsx](file:///i:/workspace/Bifurc-Worskspace/Bifurc/bifurc/renderer/panels/GraphQLMocksPanel.tsx)
#### [DELETE] [GrpcRequestsPanel.tsx](file:///i:/workspace/Bifurc-Worskspace/Bifurc/bifurc/renderer/panels/GrpcRequestsPanel.tsx)
#### [DELETE] [GrpcMocksPanel.tsx](file:///i:/workspace/Bifurc-Worskspace/Bifurc/bifurc/renderer/panels/GrpcMocksPanel.tsx)
#### [DELETE] [SoapRequestsPanel.tsx](file:///i:/workspace/Bifurc-Worskspace/Bifurc/bifurc/renderer/panels/SoapRequestsPanel.tsx)
#### [DELETE] [SoapMocksPanel.tsx](file:///i:/workspace/Bifurc-Worskspace/Bifurc/bifurc/renderer/panels/SoapMocksPanel.tsx)
#### [DELETE] [LogsPanel.tsx](file:///i:/workspace/Bifurc-Worskspace/Bifurc/bifurc/renderer/panels/LogsPanel.tsx)

#### [MODIFY] [panelFactory.tsx](file:///i:/workspace/Bifurc-Worskspace/Bifurc/bifurc/renderer/lib/panelFactory.tsx)
Remove the 6 dead imports (lines 21-26).

### 1.2 Delete Dead Utility File

#### [DELETE] [applicationUtils.ts](file:///i:/workspace/Bifurc-Worskspace/Bifurc/bifurc/renderer/lib/applicationUtils.ts)
Contains `RunConfigType`, `AppProcessStatus`, etc. — **zero imports** across the entire codebase.

### 1.3 Purge Dead Strings

#### [MODIFY] [strings.ts](file:///i:/workspace/Bifurc-Worskspace/Bifurc/bifurc/renderer/lib/strings.ts)
Remove the entire `applications:` section (~120 lines, ~lines 680-800) — the "Applications" panel is disabled and these strings are never referenced.

### 1.4 Remove Dead Imports & Unused Variables (per-file)

Will use TypeScript compiler (`tsc --noUnusedLocals --noUnusedParameters`) + manual scan. Known issues:

| File | Dead code |
|------|-----------|
| `HealthBarPanel.tsx` | Unused imports: `CheckCircle2`, `AlertCircle` |
| `WorkspacePanel.tsx` | Unused import: `RefreshCw` |
| `types.ts` | Unused types: `AuthUser`, `UpdateCheckResult`; unused IPC methods: `authSignInWithEmail`, `getSubscription`, `isFirstLaunch`, `saveRunnerReport` |
| Multiple panel files | Unused destructured vars in `handleDuplicate`: `_id`, `_ca`, `_ws` |

### 1.5 Remove gRPC Stubs

#### [MODIFY] [handlers.ts](file:///i:/workspace/Bifurc-Worskspace/Bifurc/bifurc/src/ipc/handlers.ts)
`grpc:execute` and `grpc:reflect` (lines ~1127-1150) are error-return stubs. Remove if not planned for implementation; otherwise leave and annotate.

> [!IMPORTANT]
> **Decision needed**: Are the gRPC execute/reflect stubs placeholders for upcoming work, or should they be removed?

---

## Phase 2 — Abstract Duplicate Code into Shared Utils & Hooks

### 2.1 Create Generic Entity CRUD Factory (Backend)

#### [NEW] `src/ipc/handlers/entityCrudFactory.ts`
The exact same CRUD sequence (load config → validate limits → generate ID → match folders → `writeEntity()` → `upsertNameEntry()` → sync → broadcast → return) is copy-pasted **12 times** in `handlers.ts` for: Mappings, Proxy Rules, Mocks, Requests, WebSockets, Webhooks, SOAP Requests/Mocks, GraphQL Requests/Mocks, gRPC Requests/Mocks.

Create a parameterized factory:
```typescript
function registerEntityCrud<T>(kind: string, opts: {
  configKey: keyof AppConfig;
  fsDir: string;
  limitKey?: string;
}) { /* generic add/update/delete handlers */ }
```

### 2.2 Create Generic Folder Management Map (Backend)

#### [NEW] `src/ipc/handlers/folderHandlers.ts`
The `folder:add`, `folder:rename`, `folder:move`, `folder:delete` handlers use **12-level chained ternaries** mapping `kind → configKey → fsDir`. Replace with a configuration map:
```typescript
const FOLDER_CONFIG: Record<string, { configKey: string; fsDir: string }> = {
  mock: { configKey: "mockFolders", fsDir: "mocks" },
  ws:   { configKey: "wsFolders",   fsDir: "websockets" },
  // ...
};
```

### 2.3 Extract Proxy Response Formatter (Backend)

#### [NEW] `src/proxy/responseUtils.ts`
The boilerplate of stripping `HOP_BY_HOP` headers, formatting `set-cookie`, and decompressing response body is duplicated across **4 functions** in `proxyHandler.ts` (`proxyToUpstream`, `passthroughToUpstream`, `passthroughToUpstreamHttps`, `proxyWithScripts`). Extract into a single `formatProxyResponse()` utility.

### 2.4 Extract Shared Proxy Dispatch Flow (Backend)

#### [MODIFY] [server.ts](file:///i:/workspace/Bifurc-Worskspace/Bifurc/bifurc/src/proxy/server.ts)
`dispatch()` and `dispatchHttps()` contain 100+ duplicated lines for rule matching, mock resolution, script execution, and upstream forwarding. Extract shared logic into `handleProxyRouting()`.

### 2.5 Create `useProtocolEditor` Hook (Frontend)

#### [NEW] `renderer/hooks/useProtocolEditor.ts`
`RestTab`, `GrpcTab`, `GraphQLTab`, and `SoapTab` all duplicate:
- Draft persistence via `useDraftPersist`
- State initialization and `useReducer` boilerplate
- Sync/revert logic (`handleSyncClick`, `handleRevertClick`)
- Save lifecycle (`SAVE_START` → `SAVE_SUCCESS` → `SAVE_ERROR`)

Extract into a single `useProtocolEditor(protocol, reducer, stateToPayload)` hook.

### 2.6 Create `createTabReducer` Factory (Frontend)

#### [NEW] `renderer/lib/createTabReducer.ts`
The 4 tab reducers (`restTabReducer`, `grpcTabReducer`, `graphqlTabReducer`, `soapTabReducer`) independently define identical:
- Lifecycle states: `sending`, `saving`, `loading`, `resStatus`, `error`
- Generic actions: `SET_FIELD`, `LOAD_ENTITY`, `LOAD_DRAFT`, `SEND_START`, `SEND_SUCCESS`, `SEND_ERROR`, `SAVE_START`, `SAVE_SUCCESS`

Create a base reducer factory that handles the common lifecycle, allowing each protocol to only define its unique fields and actions.

### 2.7 Create `useMultiplexedEntities` Hook (Frontend)

#### [NEW] `renderer/hooks/useMultiplexedEntities.ts`
`MocksPanel` and `RequestsPanel` contain **identical** protocol-multiplexing logic: building `itemProtocolMap`, `allItemsMap`, `folderViewItems`, and switch-based CRUD dispatch for REST/GraphQL/gRPC/SOAP entities. Extract into a shared hook.

### 2.8 Consolidate Persistence Hooks

#### [MODIFY] [useDraftPersist.ts](file:///i:/workspace/Bifurc-Worskspace/Bifurc/bifurc/renderer/lib/useDraftPersist.ts)
#### [MODIFY] [usePersistedState.ts](file:///i:/workspace/Bifurc-Worskspace/Bifurc/bifurc/renderer/lib/usePersistedState.ts)
These two hooks handle overlapping persistence logic. Consolidate into a single `usePersistedState` with a `draft` mode option.

### 2.9 JSON Import/Export Factory (Backend)

#### [NEW] `src/ipc/importExport/jsonFactory.ts`
All `*-json.ts` importers and exporters share nearly identical logic, differing only by variable names and schema strings. Create a parameterized factory to eliminate ~15 near-identical files.

---

## Phase 3 — Abstract Common Components

### 3.1 Create `<EntityEditorLayout>` Component

#### [NEW] `renderer/components/common/EntityEditorLayout.tsx`
The layout of `SidebarLayout + FolderTree + TabBar + active-tab-wrapper` is duplicated across **6 panels** (Mocks, Requests, WebSockets, Webhooks, ProxyRules, Capture). Create a single layout component:
```tsx
<EntityEditorLayout
  folders={folders}
  tabs={tabs}
  activeTab={activeTab}
  onTabChange={...}
  sidebar={<FolderTree ... />}
  renderTab={(tab) => <EditorTab ... />}
/>
```

### 3.2 Create `<MasterDetailLayout>` Component

#### [NEW] `renderer/components/common/MasterDetailLayout.tsx`
`ProxyRulesPanel` and `CapturePanel` both use a "list on left, detail on right" pattern. Standardize into a shared layout.

### 3.3 Create `<ProtocolEditorLayout>` Component

#### [NEW] `renderer/components/common/ProtocolEditorLayout.tsx`
`RestTab`, `GrpcTab`, `GraphQLTab`, and `SoapTab` all render the same structural layout: `EditorTitleBar + TabStrip + two-pane-resizable + BottomBar`. Extract into a shared component.

### 3.4 Extract `<ProxyRuleForm>` Component

#### [NEW] `renderer/components/rules/ProxyRuleForm.tsx`
`ProxyRuleDetailsPanel` and `RuleTab` contain near-identical form state management (`stateFromRule`), validation (`validate`), and save handlers. Extract the shared form fields into `<ProxyRuleForm>`.

### 3.5 Consolidate Token Hint Components

#### [MODIFY] [EnvVarHint.tsx](file:///i:/workspace/Bifurc-Worskspace/Bifurc/bifurc/renderer/components/editor/EnvVarHint.tsx)
#### [MODIFY] [RandomizerHint.tsx](file:///i:/workspace/Bifurc-Worskspace/Bifurc/bifurc/renderer/components/editor/RandomizerHint.tsx)
These share dropdown/token-insertion UI logic. Create a generalized `<TokenHint>` base component with type-specific renderers.

### 3.6 Move Misplaced Files

| Current Location | New Location | Reason |
|---|---|---|
| `lib/useDraftPersist.ts` | `hooks/useDraftPersist.ts` | Hook belongs in hooks/ |
| `lib/useEntityTabs.ts` | `hooks/useEntityTabs.ts` | Hook belongs in hooks/ |
| `lib/usePersistedState.ts` | `hooks/usePersistedState.ts` | Hook belongs in hooks/ |
| `lib/useSidebarVisibility.ts` | `hooks/useSidebarVisibility.ts` | Hook belongs in hooks/ |
| `lib/useTheme.ts` | `hooks/useTheme.ts` | Hook belongs in hooks/ |
| `lib/useWebSocket.ts` | `hooks/useWebSocket.ts` | Hook belongs in hooks/ |
| `components/editor/HeaderTable.tsx` | `components/common/HeaderTable.tsx` | Generic key-value editor |
| `components/editor/BodyEditor.tsx` | `components/common/BodyEditor.tsx` | Generic body editor wrapper |

---

## Phase 4 — Enforce 300-Line File Limit

**34 files** currently exceed 300 lines. After Phases 1-3, many will naturally shrink. The remaining over-limit files and their splitting strategy:

### Renderer files (27 over limit)

| File | Lines | Splitting strategy |
|------|-------|--------------------|
| `strings.ts` | 1460 | Remove dead `applications` section (~120 lines). Split remaining by domain: `strings/panels.ts`, `strings/editor.ts`, `strings/sidebar.ts`, etc. |
| `FolderTree.tsx` | 924 | Extract `renderNode` → `FolderTreeNode.tsx` (React.memo). Extract `renderItem` → `FolderTreeItem.tsx`. Move modal dialogs (Move/Delete/Rename) to `sidebar/modals/`. |
| `WebSocketsPanel.tsx` | 776 | Migrate to `useEntityTabs` hook + `EntityEditorLayout`. Shrinks ~400 lines. |
| `WebhooksPanel.tsx` | 764 | Same as WebSockets — migrate to shared hooks/layout. |
| `MocksPanel.tsx` | 747 | Extract multiplexing logic to `useMultiplexedEntities` hook. Use `EntityEditorLayout`. |
| `RequestsPanel.tsx` | 745 | Same as Mocks — shared hook + layout. |
| `App.tsx` | 725 | Extract: `WorkspaceProvider` (context), `ConfigSyncProvider` (effects), `AppLayout` (structure). |
| `EditorTab.tsx` | 656 | Extract: protocol-specific sub-editors into separate files. Extract tab toolbar into its own component. |
| `types.ts` | 623 | Remove dead types. Split into domain files: `types/entities.ts`, `types/ipc.ts`, `types/config.ts`. |
| `HealthBarPanel.tsx` | 621 | Extract `<ServiceHealthCard>` row component. Extract health check logic into `useHealthCheck` hook. |
| `searchUtils.ts` | 542 | Split monolithic `searchEntities` into: `searchRequests`, `searchMocks`, `searchRules`, etc. |
| `GrpcTab.tsx` | 513 | Use `useProtocolEditor` hook + `ProtocolEditorLayout`. |
| `RestTab.tsx` | 492 | Same — shared hook + layout. |
| `restTabReducer.ts` | 473 | Use `createTabReducer` factory. Only keep REST-specific fields. |
| `CollectionRunner.tsx` | 438 | Extract `useCollectionRunner` hook. Split view: `RunnerConfig`, `RunnerProgress`, `RunnerResults`. |
| `GraphQLTab.tsx` | 427 | Use `useProtocolEditor` + layout. |
| `SearchModal.tsx` | 416 | Extract `SearchResultsList.tsx`. |
| `SoapTab.tsx` | 415 | Use `useProtocolEditor` + layout. |
| `WorkspacePanel.tsx` | 412 | Extract `<WorkspaceCard>` sub-component. |
| `ProxyRulesPanel.tsx` | 398 | Use `MasterDetailLayout` + `ProxyRuleForm`. |
| `ProxyRuleDetailsPanel.tsx` | 392 | Use shared `ProxyRuleForm`. |
| `RuleTab.tsx` | 367 | Use shared `ProxyRuleForm`. |
| `CapturePanel.tsx` | 358 | Use `MasterDetailLayout`. Extract filter bar. |
| `EnvironmentsPanel.tsx` | 340 | Extract `<EnvVariableTable>` sub-component. |
| `ImportExportModal.tsx` | 339 | Extract step components to `modals/import-export/`. |
| `AuditLogPanel.tsx` | 338 | Extract `<AuditLogRow>` (React.memo) component. |
| `WorkspaceSelector.tsx` | 306 | Extract workspace dropdown content into sub-component. |

### Backend files (7 over limit)

| File | Lines | Splitting strategy |
|------|-------|--------------------|
| `handlers.ts` | 1909 | **Biggest offender.** Split into modular directory `src/ipc/handlers/`: `index.ts` (registry), `system.ts`, `entities.ts` (CRUD factory), `graphql.ts`, `grpc.ts`, `soap.ts`, `folders.ts`, `tls.ts`, `sync.ts`, `runner.ts`. |
| `server.ts` | 469 | Extract shared dispatch into `handleProxyRouting`. Move TLS logic to `tlsProxy.ts`. |
| `proxyHandler.ts` | 451 | Extract `formatProxyResponse` util. Reduces to ~300. |
| `scriptExecutor.ts` | 422 | Extract script context builder and sandbox setup. |
| `workspaceFs.ts` | 419 | Split into `workspaceFs/read.ts` and `workspaceFs/write.ts`. |
| `postman.ts` | 352 | Extract shared conversion utils. |
| `syncManager.ts` | 339 | Extract git operations into `gitOps.ts`. |

---

## Phase 5 — Fix React Anti-Patterns

### 5.1 Inline Function/Object Creation in JSX

| File | Issue | Fix |
|------|-------|-----|
| `MocksPanel`, `RequestsPanel`, `WebhooksPanel`, `WebSocketsPanel` | `<TabBar tabs={...map(() => ...)}` recreated every render | Memoize tab array with `useMemo`; wrap render functions in `useCallback` |
| `HealthBarPanel` | `onRefresh={() => checkService(svc)}` inside `.map()` | Pass `serviceId` prop to `<ServiceHealthCard>`, let the card call the handler |
| `FolderTree` | Massive inline `renderNode` and `renderItem` functions | Extract to standalone `React.memo` components |
| `FolderTree` | Inline `style={{ position: "absolute", left: ... }}` objects | Extract to `useMemo` or CSS classes |
| `SearchModal` | Inline `onClick` / `ref` callbacks in `.map()` | Extract to `<SearchResultItem>` component with `React.memo` |
| `WorkspaceSelector` | `style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}` | Extract to constant |

### 5.2 Missing Memoization

| File | Issue | Fix |
|------|-------|-----|
| `AuditLogPanel` | `rowContent` built as massive inline JSX inside `.map()` | Extract `<AuditLogRow>` with `React.memo` |
| `ProxyRulesPanel` | `columns` in `useMemo` depends on closures over changing state | Use `useCallback` for handlers, stabilize references |
| All protocol tabs | Reducer dispatch creates new function refs each render | Already stable from `useReducer`, but event handlers wrapping dispatch need `useCallback` |

### 5.3 State Management Issues

| File | Issue | Fix |
|------|-------|-----|
| `App.tsx` | Massive monolithic state with complex `useEffect` chains for config sync | Extract into `ConfigSyncProvider` context with stable callbacks |
| `WebSocketsPanel`, `WebhooksPanel` | Manual tab management duplicating `useEntityTabs` logic | Migrate to existing `useEntityTabs` hook |

---

## Verification Plan

### Automated Tests
```bash
# TypeScript compilation (zero errors baseline)
npx tsc --noEmit

# Existing unit tests must pass
npm run test

# E2E tests
npm run test:e2e
```

### Manual Verification
- After Phase 1: confirm app starts, all panels render, no console errors
- After Phase 2-3: confirm all CRUD operations work (add/edit/delete entities)
- After Phase 4: verify `find . -name "*.ts" -o -name "*.tsx" | xargs wc -l | sort -rn | head -20` shows no file > 300 lines
- After Phase 5: verify no visual regressions in panel rendering

---

## Execution Order

> [!IMPORTANT]
> Each phase will be executed sequentially and verified before moving to the next.

| Phase | Risk | Estimated files touched |
|-------|------|------------------------|
| Phase 1 — Dead code removal | 🟢 Lowest | ~15 files deleted/modified |
| Phase 2 — Shared utils/hooks | 🟡 Medium | ~25 new/modified files |
| Phase 3 — Common components | 🟡 Medium | ~20 new/modified files |
| Phase 4 — File splitting | 🟠 Medium-High | ~35 files split |
| Phase 5 — Anti-pattern fixes | 🟢 Low | ~15 files modified |

> [!WARNING]
> Phase 4 (file splitting) has the highest number of file changes. It should be done **after** Phases 2-3 which will naturally reduce many file sizes through abstraction.

## Open Questions

> [!IMPORTANT]
> 1. **gRPC stubs**: Should `grpc:execute` and `grpc:reflect` handler stubs in `handlers.ts` be removed as dead code, or kept as placeholders for upcoming gRPC execution support?
> 2. **IPC dead methods**: `authSignInWithEmail`, `getSubscription`, `isFirstLaunch`, `saveRunnerReport` are declared on the `Window` type but appear unused. Are these planned features or safe to remove?
> 3. **Import/Export factory**: Converting ~15 nearly-identical JSON import/export files into a factory is a high-value but medium-risk refactor. Should this be included in this pass or deferred?
