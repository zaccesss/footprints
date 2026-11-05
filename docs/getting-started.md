# Getting started

This page covers installing Footprints on macOS, Linux and Windows, recording a first agent run
and finding where the data lives. Connecting a particular agent is covered in
[the agent guides](agents/README.md).

## Contents

- [Requirements](#requirements)
- [Install](#install)
- [Check the install](#check-the-install)
- [First run](#first-run)
- [Connect an agent](#connect-an-agent)
- [Where data lives](#where-data-lives)
- [Uninstall](#uninstall)

## Requirements

| Platform | Supported | Notes |
| --- | --- | --- |
| macOS | arm64 and x86_64 | Pre-images use APFS `clonefile`. Changes are watched with FSEvents |
| Linux | x86_64 and aarch64 | Release binaries are static musl builds. Pre-images use a `FICLONE` reflink on Btrfs, XFS and bcachefs and a plain copy elsewhere. Changes are watched with inotify |
| Windows | x86_64 | Pre-images are plain copies. Changes are watched with ReadDirectoryChangesW. Some limits differ, listed in [what can and cannot be undone](undo.md#windows) |

Building from source needs Rust 1.89 or later, the `rust-version` in
[`Cargo.toml`](../Cargo.toml). CI checks that version on every pull request.

## Install

### Cargo

On any platform with a Rust toolchain:

```sh
cargo install footprints
```

The binary goes into `~/.cargo/bin` (`%USERPROFILE%\.cargo\bin` on Windows), which `rustup`
puts on the `PATH`.

### Homebrew on macOS and Linux

```sh
brew install zaccesss/tap/footprints
```

The formula lives in the `zaccesss/homebrew-tap` repository. Its template is
[`packaging/homebrew/footprints.rb`](../packaging/homebrew/footprints.rb). It installs the release
binary for the machine's platform and runs `footprints --version` with a throwaway
`FOOTPRINTS_HOME` as its test. Upgrade with `brew upgrade footprints`.

### winget on Windows

```powershell
winget install zaccesss.Footprints
```

> [!NOTE]
> The package `zaccesss.Footprints` is [pending review in the winget repository](https://github.com/microsoft/winget-pkgs/pull/471542). The command
> works once it is accepted. Until then, use Cargo or the release binaries below.

It installs the Windows zip from the GitHub release as a portable program with the command
`footprints`. Upgrade with `winget upgrade zaccesss.Footprints`.

### Release binaries

Every [GitHub release](https://github.com/zaccesss/footprints/releases) carries one archive per
platform, each with a `.sha256` file beside it:

| Platform | Archive |
| --- | --- |
| macOS on Apple silicon | `footprints-<tag>-aarch64-apple-darwin.tar.gz` |
| macOS on Intel | `footprints-<tag>-x86_64-apple-darwin.tar.gz` |
| Linux on x86_64 | `footprints-<tag>-x86_64-unknown-linux-musl.tar.gz` |
| Linux on aarch64 | `footprints-<tag>-aarch64-unknown-linux-musl.tar.gz` |
| Windows on x86_64 | `footprints-<tag>-x86_64-pc-windows-msvc.zip` |

Each archive holds the binary plus `README.md`, `LICENSE`, `NOTICE`, `CHANGELOG.md` and
`ACCESSIBILITY.md` in a folder of the same name.

On macOS or Linux, check the sum and put the binary on the `PATH`:

```sh
tag=v0.2.0
target=aarch64-apple-darwin
curl -LO "https://github.com/zaccesss/footprints/releases/download/$tag/footprints-$tag-$target.tar.gz"
curl -LO "https://github.com/zaccesss/footprints/releases/download/$tag/footprints-$tag-$target.tar.gz.sha256"
shasum -a 256 -c "footprints-$tag-$target.tar.gz.sha256"
tar -xzf "footprints-$tag-$target.tar.gz"
install -m 755 "footprints-$tag-$target/footprints" ~/.local/bin/footprints
```

Linux without `shasum` can use `sha256sum -c` instead. The binaries are not signed or notarised,
so macOS may quarantine a file downloaded through a browser. Downloading with `curl` as above
avoids that. For a browser download, `xattr -d com.apple.quarantine footprints` clears it.

On Windows, in PowerShell:

```powershell
$tag = "v0.2.0"
$name = "footprints-$tag-x86_64-pc-windows-msvc"
Invoke-WebRequest "https://github.com/zaccesss/footprints/releases/download/$tag/$name.zip" -OutFile "$name.zip"
Invoke-WebRequest "https://github.com/zaccesss/footprints/releases/download/$tag/$name.zip.sha256" -OutFile "$name.zip.sha256"
(Get-FileHash "$name.zip" -Algorithm SHA256).Hash.ToLower()
Get-Content "$name.zip.sha256"
Expand-Archive "$name.zip" -DestinationPath .
```

The two printed sums must match. Then move `footprints.exe` into a folder on the `PATH`.

### From source

```sh
git clone https://github.com/zaccesss/footprints
cd footprints
cargo install --path .
```

The menu bar app in [`app/`](../app/README.md) is a separate crate and is built on its own. See
[the menu bar app](menu-bar-app.md).

## Check the install

```console
$ footprints --version
footprints 0.2.0
```

`footprints --help` lists every command. [The command reference](commands.md) has each one in
full.

## First run

The quickest way to see Footprints work is to wrap a small script. The script stands in for an
agent: it rewrites one file.

```sh
mkdir demo && cd demo
echo 'name = "old"' > config.toml
echo "echo 'name = \"new\"' > config.toml" > agent.sh
```

Run it under the wrapper, then read the ledger:

```console
$ footprints run --agent demo -- sh agent.sh
footprints: recorded command #1 in session run-bcc287bf with 1 file change(s). See `footprints log --session run-bcc287bf`
$ footprints log
ID  TIME                 AGENT  SESSION       KIND        STATE           TARGET
 1  2026-11-05 12:57:29  demo   run-bcc287bf  command     not reversible  sh agent.sh
 2  2026-11-05 12:57:29  demo   run-bcc287bf  file_write  reversible      config.toml (via #1)
```

The command itself cannot be undone, but the file change under it can. Look at it, then take it
back:

```console
$ footprints diff 2
--- a/Users/me/demo/config.toml
+++ b/Users/me/demo/config.toml
@@ -1 +1 @@
-name = "old"
+name = "new"
$ footprints undo 2
undid #2: restored /Users/me/demo/config.toml (recorded as #3 in session undo-c41baf35)
```

The undo is recorded as action 3, so it can be undone too. If the file is edited again before an
undo, Footprints refuses and changes nothing:

```console
$ echo 'name = "edited"' > config.toml
$ footprints undo 3
footprints: refused: conflict: /Users/me/demo/config.toml changed after #3 was recorded
  recorded post-image: blake3 59cdb0473659 (13 bytes)
  current file:        blake3 9bc08c50d0a6 (16 bytes)
Nothing was changed. Run `footprints diff 3` to see the recorded change. Run `footprints undo 3 --force` to overwrite it with the pre-image. The current contents are stored first, so that undo can itself be undone
```

That refusal exits with status 3. [Undo](undo.md) explains every check.

### A longer example

Here a script edits a setting in `config.toml` and creates `NOTES.md`. The log shows both
changes, the diff shows the edited line and the undo puts the old setting back. The final log
marks the change as undone and lists the undo as its own entry.

<img src="../assets/demo/demo.png" width="780" height="495" alt="A real terminal running Footprints in a folder called demo. cat agent.sh shows a script standing in for an agent: it turns auto_save on in config.toml and creates NOTES.md. footprints run records the command with its two file changes. footprints log lists the command as not reversible and both file changes as reversible. footprints diff 3 shows auto_save changing from false to true, with minus and plus markers so colour is never the only signal. footprints undo 3 restores config.toml and records the undo as entry 4.">

The screenshot is a real run in a real terminal. The same output as text is in
[`assets/demo/demo.txt`](../assets/demo/demo.txt).

## Connect an agent

| Route | Use it when | Guide |
| --- | --- | --- |
| Native hooks | The agent is Claude Code, Cursor, OpenAI Codex CLI or Gemini CLI | [Agent guides](agents/README.md) |
| The `run` wrapper | The agent is started from a terminal | [Any other agent](agents/other-agents.md) |
| The daemon | The agent is an editor extension or a desktop app | [How capture works](how-capture-works.md#the-daemon) |
| MCP | The agent should read its own ledger and undo its own mistakes | [MCP server](mcp.md) |

Hooks and the wrapper can be combined: hooks give the intent behind each tool call and the
wrapper catches anything the hooks miss.

## Where data lives

| Platform | Default data directory |
| --- | --- |
| macOS and Linux | `~/.local/share/footprints` |
| Windows | `%LOCALAPPDATA%\footprints` |

Setting `FOOTPRINTS_HOME` moves it anywhere. Every command, hook, the daemon and the menu bar app
read the same variable, so an agent's hooks must see the same value as the terminal that reads
the log. After a first run the directory looks like this:

```console
$ ls ~/.local/share/footprints
blobs     ledger.db locks     staging
```

On macOS and Linux the directory and everything in it is private to the account (`0700` for
directories and `0600` for files). On Windows it inherits the user profile's access list.
[Data and privacy](data-and-privacy.md) lists every file, every setting and what the blob store
keeps.

## Uninstall

Remove the binary the same way it was installed:

```sh
cargo uninstall footprints
brew uninstall footprints
```

On Windows, a winget install is removed with `winget uninstall zaccesss.Footprints`.

Then remove the hook blocks from each agent's settings, as each [agent guide](agents/README.md)
describes. Moving the data directory to the Trash removes the whole history, including every
stored copy of every file, as [removing everything](data-and-privacy.md#removing-everything)
describes. `footprints prune` removes only older history. Nothing else is installed: Footprints starts no service and edits no settings itself.
