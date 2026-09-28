/**
 * Where the workspace stands relative to its remote — the missing half of "is this entity synced?".
 *
 * ## Why this file exists
 *
 * `getWorkspaceSyncStatus()` answers *"does this file differ from HEAD?"*. That is not the question
 * the UI needs to ask. The publish model has **three** resting states, not two:
 *
 * | on disk            | HEAD        | remote       | what the user should be offered |
 * | ------------------ | ----------- | ------------ | ------------------------------- |
 * | not in the index   | —           | —            | **Publish** (this is a create)  |
 * | differs from HEAD  | differs     | —            | **Sync** + **Revert**           |
 * | == HEAD            | == remote   | == HEAD      | nothing — it is synced          |
 * | == HEAD            | ahead       | behind HEAD  | **Sync** only — push it         |
 *
 * The fourth row is the one that was missing. A file committed locally but never pushed is `clean`
 * as far as `git status` is concerned, so the UI showed **no button at all** and the entity could
 * not be pushed from its own row — the footer's panel-level publish was disabled too, because it
 * counts non-`clean` entries under the panel's prefix. That is the "publish/sync is inconsistent"
 * report, and it is a *missing dimension* rather than a wrong branch.
 *
 * ## Local only — deliberately, and the trade-off is real
 *
 * `ahead` is measured against **`refs/remotes/origin/<branch>`**, the last remote state this clone
 * knows about. Nothing here touches the network: no `fetch`, no `ls-remote`. Three reasons, in order
 * of weight:
 *
 *  1. **It is called on every status refresh** — which is every entity write, every publish, every
 *     config save. A network round trip on that path would make the UI wait on the internet to draw
 *     a pencil icon.
 *  2. **This sandbox's `git fetch` is not trustworthy.** It reports success and does not write the
 *     remote-tracking ref (see the standing note in `~/.workbuddy-ai/MEMORY.md`), so an `ahead`
 *     derived from a fresh fetch would be *wrong* here in a way that looks like a product bug.
 *  3. `autoSync` already polls `getRemoteHead()` on its own schedule. A status refresh is not the
 *     place to duplicate that.
 *
 * The consequence, stated plainly so it is not mistaken for a bug: **`ahead` can be stale after
 * someone else pushes.** If another machine pushed to the same branch, this clone still believes it
 * is ahead until the next pull/fetch. The remedy is the pull path, not a fetch here.
 *
 * ## A repo with no `origin` is never "ahead"
 *
 * `hasRemote: false` ⇒ `ahead: false`, so a purely local workspace offers neither Sync nor Revert on
 * a `clean` entity — there is nowhere to push, so "committed and pushed" is already true. Note that
 * `publishEntities()` still commits in that case; it just skips the push. The distinction the UI
 * draws is between *a change to publish* (a diff) and *a commit to push* (ahead), and neither exists
 * here.
 */
import { getGit } from "../store/gitStore";

export interface WorkspaceGitState {
  /** An `origin` remote is configured. */
  hasRemote: boolean;
  /** The checked-out branch, or `null` on an unborn/detached HEAD. */
  branch: string | null;
  /** HEAD has commits that `refs/remotes/origin/<branch>` does not. */
  ahead: boolean;
  /** `refs/remotes/origin/<branch>` has commits that HEAD does not. */
  behind: boolean;
}

const NO_GIT_STATE: WorkspaceGitState = {
  hasRemote: false,
  branch: null,
  ahead: false,
  behind: false,
};

/**
 * Read the workspace's position relative to its remote. Never throws — a workspace with no repo, a
 * broken repo, or no git at all reports `NO_GIT_STATE` rather than rejecting, because every caller
 * is a status refresh whose failure has nowhere to go.
 */
export async function getWorkspaceGitState(wsId: string): Promise<WorkspaceGitState> {
  const g = getGit(wsId);

  try {
    const remotes = (await g.raw(["remote"]))
      .split(/\r?\n/)
      .map((r) => r.trim())
      .filter(Boolean);
    if (!remotes.includes("origin")) return NO_GIT_STATE;

    let branch: string | null = null;
    try {
      const head = (await g.raw(["rev-parse", "--abbrev-ref", "HEAD"])).trim();
      // `--abbrev-ref HEAD` prints the literal "HEAD" when detached, which is not a branch name.
      if (head && head !== "HEAD") branch = head;
    } catch {
      // Unborn branch — no commits yet, so there is nothing to be ahead of.
    }
    if (!branch) return { hasRemote: true, branch: null, ahead: false, behind: false };

    // Does `refs/remotes/origin/<branch>` exist?
    //
    // **Tested by output, not by exit code.** `git rev-parse --verify --quiet <missing-ref>` exits
    // **0 with empty output** on this git (measured), so a `try { … } catch { /* never pushed */ }`
    // around it never fires — and the next line, `rev-list origin/<branch>...HEAD`, then throws
    // `ambiguous argument`, which the outer catch turns into `NO_GIT_STATE`. The visible symptom was
    // the opposite of the intent: a repo with a remote and nothing pushed reported **no remote at
    // all**, so the UI offered no Sync for the one state that most needs it. Caught by
    // `tests/sync/gitState.test.ts`.
    let remoteRef = "";
    try {
      remoteRef = (await g.raw(["rev-parse", "--verify", "--quiet", `refs/remotes/origin/${branch}`])).trim();
    } catch {
      remoteRef = "";
    }
    if (!remoteRef) {
      // `origin` exists but this branch has never been pushed, so every commit is unpushed.
      return { hasRemote: true, branch, ahead: true, behind: false };
    }

    // `--left-right` prints "<behind>\t<ahead>" for `A...B`: commits reachable from A only, then
    // from B only. Order matters and is easy to invert — this is A=origin, B=HEAD.
    const counts = (await g.raw(["rev-list", "--count", "--left-right", `origin/${branch}...HEAD`])).trim();
    const [behindRaw, aheadRaw] = counts.split(/\s+/);
    return {
      hasRemote: true,
      branch,
      ahead: Number(aheadRaw) > 0,
      behind: Number(behindRaw) > 0,
    };
  } catch {
    return NO_GIT_STATE;
  }
}
