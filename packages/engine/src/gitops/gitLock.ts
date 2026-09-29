/**
 * Surviving `.git/index.lock` — the stale one that made publish silently fail.
 *
 * ## The failure this exists for
 *
 * Every git command that touches the index takes `.git/index.lock` for the duration of that command.
 * If the process is **killed** mid-write — the app closed during a push, the shell killed — the lock is
 * never released, and from then on every `git add` fails with
 * `Unable to create '…/.git/index.lock': File exists.` while `git status` keeps reading happily. So the
 * entity looks untracked (correctly!), the Publish button is offered (correctly!), and the publish
 * fails for a reason nobody can see.
 *
 * Measured on a real workspace 2026-09-28. `publishEntities` was swallowing the `git add` failure in a
 * bare `catch {}`, so it reported `commit: null` and the UI said *"this entity is already committed"* —
 * a claim the card's own status contradicted. The lock had been there since an earlier crash.
 *
 * ## Retry first, and why that is not a workaround
 *
 * A lock held by a **live** process is released in milliseconds on a config repo this size, so a
 * transient collision resolves on its own. The app's *own* source of them is gone — `GIT_OPTIONAL_LOCKS=0`
 * in `getGit` stops the status poll from taking an optional lock to refresh the index — so what remains
 * is another git process (an editor, a terminal) doing something.
 *
 * ## Then remove it, because the alternative is an app that cannot publish at all
 *
 * After five attempts over ~2 s the lock is not transient: no live git process holds the index for two
 * seconds on a repo of this size. The alternative is a permanently broken Publish button with no
 * recovery except a terminal — which is exactly the state this was reported from. The index is
 * rebuildable (`git reset`), so removing a live process's lock is a **recoverable** risk; leaving a
 * stale one in place is an unrecoverable feature.
 *
 * The path comes from git's own message and is then **re-validated** — basename `index.lock`, and
 * inside this workspace's `.git/` — so a message we did not anticipate cannot make us unlink something
 * arbitrary.
 */
import * as fs from "fs";
import * as path from "path";
import { wsDir } from "../workspace/workspaceFs";

const INDEX_LOCK = /index\.lock/i;
const LOCK_PATH = /Unable to create '([^']+)'/i;

/**
 * Attempts before the lock is treated as stale. Five, spaced 80/160/240/320 ms — long enough for a
 * transient collision to clear, short enough that a genuinely stale lock does not stall a click.
 */
const ATTEMPTS = 5;

const delay = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * Run a git **write**, retrying while another process holds the index.
 *
 * Only `index.lock` failures are retried; anything else is rethrown immediately, because retrying a
 * permission error or a bad pathspec just delays the report.
 */
export async function withIndexLockRetry<T>(wsId: string, fn: () => Promise<T>): Promise<T> {
  for (let attempt = 1; ; attempt++) {
    try {
      return await fn();
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      if (!INDEX_LOCK.test(message)) throw e;
      if (attempt < ATTEMPTS) {
        await delay(80 * attempt);
        continue;
      }
      // Every retry hit the same lock. Remove it and give the operation one more chance — if it fails
      // again the error is real and the caller reports it.
      removeStaleIndexLock(wsId, message);
      return await fn();
    }
  }
}

/**
 * Remove the lock git told us it could not create — **only** if it really is this workspace's index
 * lock. Exported for its own test: the guard is the part worth pinning, since the consequence of
 * getting it wrong is deleting a file we were never pointed at.
 */
export function removeStaleIndexLock(wsId: string, gitMessage: string): boolean {
  const claimed = gitMessage.match(LOCK_PATH)?.[1];
  if (!claimed) return false;

  const gitDir = path.resolve(wsDir(wsId), ".git");
  const lock = path.resolve(claimed);
  // Basename first: a message we did not anticipate must not be able to name any other file.
  if (path.basename(lock) !== "index.lock") return false;
  if (!lock.startsWith(gitDir + path.sep)) return false;

  try {
    fs.unlinkSync(lock);
    return true;
  } catch {
    return false; // already gone, or not ours to remove
  }
}
