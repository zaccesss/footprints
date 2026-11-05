# Packaging

How Footprints reaches crates.io, Homebrew and winget after a release. All three are handled by
[`release.yml`](../.github/workflows/release.yml) once the binaries are attached, so a release
needs nothing else by hand. The steps below are the fallback for when a job was skipped or
failed. The [workflows README](../.github/workflows/README.md#homebrew-and-winget) lists the
secrets each job needs.

Before any of these, the release must exist: the tag `vX.Y.Z` pushed, the GitHub release made from
it and `release.yml` finished, so the five archives and their `.sha256` files are attached.
`gh release view vX.Y.Z --repo zaccesss/footprints` lists them.

## In this folder

| File | Does |
| --- | --- |
| [`homebrew/footprints.rb`](homebrew/footprints.rb) | The formula for the tap, with a `SHA256_` placeholder per platform |
| [`winget/`](winget) | The three winget manifests (version, installer and default locale) in manifest schema 1.12.0, with one placeholder for the Windows zip |
| [`fill-checksums.sh`](fill-checksums.sh) | Downloads the `.sha256` files of a release and writes the formula and the manifests with the sums filled in to a folder of choice |

The files here keep their placeholders. Each names one version in its URLs and `fill-checksums.sh`
refuses a tag for any other version. The release workflow moves its own copy to the tag's version
first. A manual run needs the version bumped in the files beforehand: the four URLs and the
`version` line in the formula, plus `PackageVersion`, the zip path, `InstallerUrl` and
`ReleaseNotesUrl` in the manifests.

## Homebrew

The tap is `zaccesss/homebrew-tap`, which Homebrew reads as `zaccesss/tap`. The formula installs
the release archive for the platform rather than building from source, so it needs the SHA-256 of
each of the four Unix archives.

The release workflow's `homebrew` job updates the tap itself. It writes the formula to the tap's
`main` as a Zac Bot commit, which the tap's ruleset allows for the admin role, so no pull request
counts on the maintainer's profile. Its `TAP_TOKEN` is a fine-grained token from the account that
owns the tap, limited to `zaccesss/homebrew-tap` with Contents read and write. When the job is skipped
or warns that the update failed, update the tap by hand the same way:

1. Fill the formula from the release files:

   ```sh
   packaging/fill-checksums.sh vX.Y.Z /tmp/packaging-X.Y.Z
   ```

2. Copy `/tmp/packaging-X.Y.Z/homebrew/footprints.rb` to `Formula/footprints.rb` in a checkout
   of the tap, then check it there before committing:

   ```sh
   brew install --formula ./Formula/footprints.rb
   brew test footprints
   brew style ./Formula/footprints.rb
   ```

3. Commit it to the tap's `main` as `feat: Footprints X.Y.Z`.

## winget

The package is `zaccesss.Footprints`. The installer is the Windows zip from the release,
installed as a portable program with the command alias `footprints`. The first version is
[pending review in the winget repository](https://github.com/microsoft/winget-pkgs/pull/471542).
Once it is accepted, the release workflow submits every later version itself.

1. Fill the manifests from the release files:

   ```sh
   packaging/fill-checksums.sh vX.Y.Z /tmp/packaging-X.Y.Z
   ```

2. Copy the three files from `/tmp/packaging-X.Y.Z/winget/` into a fork of
   [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) under
   `manifests/z/zaccesss/Footprints/X.Y.Z/`.
3. Validate and test them on a Windows machine:

   ```powershell
   winget validate --manifest manifests\z\zaccesss\Footprints\X.Y.Z
   winget install --manifest manifests\z\zaccesss\Footprints\X.Y.Z
   footprints --version
   ```

4. Open a pull request to microsoft/winget-pkgs. Their checks and review usually take a day or
   two and the package is live once it merges.
