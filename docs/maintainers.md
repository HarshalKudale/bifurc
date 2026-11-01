# Maintainers

Repository settings, and the owner's checklist for going public and releasing. Every placeholder marked
`TODO` followed by `(owner):` in the repositories must be gone before **Release** runs: its first job refuses to
start while one is left (`git grep -n 'TODO[(]owner[)]:'` lists them).

## External setup

- [ ] Turn on GitHub Sponsors for `HarshalKudale` (needs Stripe and a bank account; approval can take days). Fix the
  handle in `.github/FUNDING.yml`, the README and the website.
- [ ] Create the Ko-fi page and fix the handle everywhere.
- [ ] Install CLA Assistant (cla-assistant.io) on `bifurc` and `bifurc-extension`, linking to `CLA.md`. Put its badge in
  the README.
- [ ] Create the `security@bifurc.app` alias, or switch `SECURITY.md` to `contact@bifurc.app`.
- [ ] Record the legacy download count of `bifurc-releases` and commit it to the website (`LEGACY_DOWNLOADS` in
  `site.ts`):

  ```bash
  gh api repos/HarshalKudale/bifurc-releases/releases --paginate --jq '[.[].assets[] | select(.name != "stable.json") | .download_count] | add'
  ```

- [ ] Add the repo link to the store listings: Microsoft Store, Chrome Web Store, Firefox AMO, Product Hunt.

## Repository settings (both public repos, after flipping, unless noted)

- [ ] **Self-hosted runners:** Settings, Actions, Runners: remove every self-hosted runner from `bifurc` and from
  `bifurc-extension`, and uninstall the runner services on the machines.
- [ ] **Actions, General:** require approval for all external contributors; workflow permissions "Read repository
  contents"; uncheck "Allow GitHub Actions to create and approve pull requests".
- [ ] **Environment `release`** (app): add the secrets `BIFURC_POSTHOG_KEY`, `BIFURC_POSTHOG_HOST` and
  `BIFURC_GITHUB_CLIENT_ID`; set yourself as required reviewer; allow only the `master` branch. Delete the old repo
  variables of the same names.
- [ ] **Rulesets** "Protect master" (and "Protect release/**" on the app): restrict deletions, block force pushes,
  require linear history, require a PR with 1 approval, dismiss stale approvals, require review from Code Owners,
  require conversation resolution, require the status checks `all-green` and `license/cla` with branches up to date.
  Bypass: Repository admin, "for pull requests only", so you can merge your own PRs. On a personal-account repo only
  collaborators with Write access can merge. To add a member later, give them Write and add them to
  `.github/CODEOWNERS`.
- [ ] Turn on private vulnerability reporting, Dependabot alerts and security updates, and secret scanning with push
  protection.
- [ ] Turn on Discussions.
- [ ] Set the About fields: description, website `https://bifurc.app`, and topics (`api`, `proxy`, `mock-server`,
  `http-debugging`, `rust`, `gpui`, `mcp`, `developer-tools`, `localhost`).
- [ ] Delete the extension repo's old `https-changes` branch (identical to `master`).
- [ ] In your own clone of `bifurc`, move the old commit hook out of the repo: it is removed from the tree, so copy
  it from the commit before its removal to `~/.config/git/hooks/bifurc/commit-msg` and run
  `git config core.hooksPath ~/.config/git/hooks/bifurc` in that clone only.

## Go-live order

1. Every chunk is merged into `release/0.6.5`, gitleaks is clean over the full history of both repos, and no
   placeholder is left.
2. Merge `release/0.6.5` into `master`.
3. **Make `bifurc` and `bifurc-extension` public** (Settings, Change visibility). Apply the settings above.
4. Run **Release** with `0.6.5` and "Everything is open", then follow [releasing.md](releasing.md): Partner Center,
   delete the unsigned MSIX, publish.
5. Run the site's **Publish release** for `v0.6.5`.
6. Verify: `install.sh` on macOS and Linux installs 0.6.5; a 0.6.2 portable zip sees 0.6.5 and its download link
   resolves through `bifurc.app/dl/...`; a 0.6.2 macOS or Linux install updates through `update.apply`; the badge
   endpoint returns legacy plus new downloads.
7. After Store certification: run `scripts/attach-signed-msix.ps1`, then **Publish release** again with
   `store_version=0.6.5`.
8. Delete `HarshalKudale/bifurc-releases`, once 0.6.5 is published here, the site serves the new manifest, a 0.6.2
   install has been seen to pick up 0.6.5, and the legacy count is recorded.
9. Start the list submissions.
