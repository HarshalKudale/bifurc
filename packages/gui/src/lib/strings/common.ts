export const commonStrings = {
  common: {
      /** The only write button left: it creates a draft that has no server id yet. */
      create: "Create",
      /** A rename dialog's confirm — a one-shot value, not an autosaved entity. */
      rename: "Rename",
      cancel: "Cancel",
      delete: "Delete",
      close: "Close",
      edit: "Edit",
      add: "Add",
      import: "Import",
      export: "Export",
      addRow: "Add Row",
      addEntry: "Add Entry",
      format: "Format",
      noHeaders: "No headers",
      noHeadersAddRow: "No headers — click Add Entry",
      noHeadersCaptured: "No headers captured",
      noHeadersAddRowBelow: "No headers — click Add Entry below",
      emptyBody: "empty body",
      noResults: "No results.",
      key: "Key",
      value: "Value",
      sync: "Sync",
      syncing: "Syncing…",
      revert: "Revert",
      reverting: "Reverting…",
      /** Autosave indicator — the replacement for the Save button's feedback. */
      saving: "Saving…",
      saved: "Saved",
      saveFailed: "Save failed",
      /** Confirms a modal whose fields already autosaved: there is nothing left to write. */
      done: "Done",
      syncTooltip: "Sync (commit & push)",
      revertTooltip: "Revert local changes to last synced version",
      noChangesToRevert: "No local changes to revert",
      noChangesToSync: "No local changes to sync",
    },
  
    // Server status,

  server: {
      running: "Server running on port {port} — access via {hint}",
      notRunning: "Server not running",
      error: "{error}. Kill the process using that port or change the port above, then restart the server.",
      restarting: "Restarting…",
      restart: "Restart",
      sending: "Sending…",
      send: "Send",
      saving: "Saving…",
      testing: "Testing…",
      test: "Test",
    },
  
    // Mocks panel,

  binary: {
      preview: "Preview",
      hex: "Hex",
      base64: "Base64",
      copyBase64: "Copy base64",
      upload: "Upload",
      dropFile: "Drop a file here or click Upload",
      noBinaryData: "No binary data",
      fileTooLarge: "File too large ({size}). Max: 1 MB",
    },

  history: {
      title: "{kind} History",
      kindRequest: "Request",
      kindMock: "Mock",
      kindSocket: "Socket",
      kindEnvironment: "Environment",
      kindMapping: "Mapping",
      kindRule: "Rule",
      kindEntity: "Entity",
      closeHistory: "Close history",
      commit: "commit{s}",
      loading: "Loading…",
      noHistory: "No history found for this file.",
      localActor: "local",
      loadingDiff: "Loading diff…",
      noSnapshot: "No snapshot available.",
      deletedPrior: "Deleted — prior state in git history.",
      createdNoPrior: "Created — no prior state.",
      noFieldChanges: "No field changes detected.",
      hideUnchanged: "Hide {n} unchanged",
      showUnchanged: "Show {n} unchanged",
    },
  
    // Right sidebar, variables face. The two group headings mirror the resolver: the active
    // environment is overlaid on `__global__`, so the same key in both lists resolves to the
    // environment's value — `overriddenBadge` is how that is visible at all.,

  rightSidebar: {
      toggleVariables: "Environment variables",
      variablesTitle: "Variables",
      closeVariables: "Close variables",
      currentEnvGroup: "Environment",
      globalGroup: "Global",
      noActiveEnv: "No environment selected",
      noActiveEnvHint: "Choose an environment from the title bar to see its variables.",
      noVarsInEnv: "This environment has no variables.",
      noGlobalVars: "No global variables.",
      overriddenBadge: "overridden",
      overriddenByEnv: "The active environment defines this key, so its value wins.",
      copyReference: "Copy variable reference",
      emptyValue: "(empty)",
      nVariables: "{n} variable{s}",
    },
  
    // Import/export modal,

  /**
   * The git-tracking vocabulary — **one set of words for one set of actions, everywhere.**
   *
   * Before this section the same two actions were spelled four ways depending on which surface you
   * were looking at: the editors said *Sync* / *Revert*, the mapping cards said *Publish* / *Revert*,
   * the folder tree said *Commit & Push* / *Discard Changes* / *Restore*, and the footer said
   * *Publish*. `packages/gui/src/lib/entityGit.ts` decides which of these apply to an entity; these
   * strings are the only place the words are written. Do not inline a new label at a call site —
   * that is how the four vocabularies happened.
   */
  entityGit: {
      /** Untracked entity — the create action. */
      publish: "Publish",
      publishing: "Publishing…",
      publishTooltip: "Commit and push this entity for the first time",
      /** Tracked entity with local work — the update action. */
      sync: "Sync",
      syncing: "Syncing…",
      syncTooltip: "Commit and push the current state",
      /** Tracked entity whose commits are not on the remote yet. */
      pushTooltip: "Push the commits that are not on the remote yet",
      revert: "Revert",
      reverting: "Reverting…",
      revertTooltip: "Discard local changes and restore the last synced version",
      /** Folder / multi-selection verbs. */
      publishAll: "Publish All",
      syncAll: "Sync All",
      publishAllTooltip: "Commit and push every unpublished entity here",
      syncAllTooltip: "Commit and push every change here",
      history: "History",
      historyTooltip: "View the commit history of this entity",
      /**
       * The outcome of a write, said out loud.
       *
       * A successful publish used to produce **no** feedback beyond a subtle status-label change, and
       * a publish that staged nothing produced none at all — so "clicking publish does nothing" was
       * literally accurate. `publishedWithCommit` carries the **commit id**, which is the only thing
       * that proves a git operation happened; `nothingToPublish` is the case that was silent: the
       * engine returns `ok: true, commit: null` when the entity is already committed, which is correct
       * behaviour and was indistinguishable from a failure to act.
       */
      published: "Published",
      publishedWithCommit: "Published · {sha}",
      syncedWithCommit: "Synced · {sha}",
      reverted: "Reverted",
      nothingToPublish: "Nothing to publish — this entity is already committed.",
      /**
       * The card and the repository contradicting each other.
       *
       * Reported instead of "already committed" when the status map still calls the entity
       * unpublished: a Publish button sitting above the words "this entity is already committed" is a
       * bug report, not a state, and the two must not be allowed to disagree in silence.
       */
      gitStatusDisagreement: "git staged nothing, but this entity still reads as unpublished — the workspace status and the repository disagree. Reopen the workspace to re-read the status.",
      /** The proof, for a caller that wants to name the commit without a sentence around it. */
      commitLabel: "commit {sha}",
      noChangesToSync: "No local changes to sync",
      noChangesToRevert: "No local changes to revert",
      noHistoryYet: "Not published yet, so it has no history",
      /** The deleted-but-uncommitted state. */
      deleted: "Deleted",
      recover: "Recover",
      recoverTooltip: "Restore this entity from the last synced version",
      syncDelete: "Commit Delete",
      syncDeleteTooltip: "Commit the deletion to git",
      pendingDeletion: "Pending Deletion",
      pendingDeletionBody: "has been deleted locally but not yet committed. Recover it, or commit the deletion.",
      /** Status badges, in the same words on every surface. */
      stateNew: "Not published",
      stateModified: "Changes to sync",
      stateUnpushed: "Unpushed commits",
      stateDeleted: "Deleted",
      stateSynced: "Synced",
      stateUnknown: "Checking…",
      /** Delete confirmation, split by whether the deletion can be undone. */
      deleteConfirmRecoverable: "Delete {what}? The entity is removed locally but stays recoverable until you commit the deletion.",
      deleteConfirmPermanent: "Delete {what}? This entity has never been published and will be permanently removed.",
    },
} as const;
