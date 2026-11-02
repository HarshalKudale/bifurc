# Releasing

1. Bump the version on the release branch (`[workspace.package].version` in `Cargo.toml`, then `cargo update -w`), and merge to `master`.
2. Run **Release** (Actions) from `master` with the version and a title. It builds Windows, Linux and macOS and
   attaches everything to a **draft** release, with `SHA256SUMS.txt` and build provenance. The job needs the
   `release` environment's approval.
3. Download `Bifurc-<v>-UNSIGNED-for-store-upload.msix` from the draft and submit it in Partner Center.
4. Delete that asset from the draft: `gh release delete-asset v<v> Bifurc-<v>-UNSIGNED-for-store-upload.msix -y`.
5. Publish the draft. macOS and Linux users get it right away; **don't wait for the Store.**
6. In `bifurc-studio` (the private website repo), run **Publish release** with the tag and a summary. It writes
   `stable.json` and the site's version file and opens a PR.
7. Once Store certification is done and the update is live:
   1. Download the signed package by hand from store.rg-adguard.net (product id `9NCB4FDZZQPV`, ring Retail).
   2. Run `scripts/attach-signed-msix.ps1 -Tag v<v> -Path <file>`. It checks the signature, name and version
      first.
   3. Run **Publish release** again with `store_version=<v>`.

`winget download --source msstore` was tried: it needs a Microsoft Entra ID sign-in with an administrator role,
so the package is downloaded by hand instead.

Verify a download:

```
sha256sum -c SHA256SUMS.txt --ignore-missing
gh attestation verify <file> --repo HarshalKudale/bifurc
```
