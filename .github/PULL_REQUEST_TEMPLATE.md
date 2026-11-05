## What does this PR do?

A short, clear description of the change in your own words.

## Which parts does it touch?

- [ ] Ledger or blob store (`src/store.rs`, `src/blob.rs`)
- [ ] Capture (`src/hook.rs`, `src/run.rs`, `src/snapshot.rs`)
- [ ] Undo (`src/undo.rs`)
- [ ] Output, diff or export (`src/output.rs`, `src/diff.rs`, `src/export.rs`)
- [ ] MCP server (`src/mcp.rs`)
- [ ] Docs, CI or templates

## Related issue

Closes #

## AI use

**Required.** Tick one, following the [AI policy](https://github.com/zaccesss/ai-policy). Dictation, translation and spelling tools do not count.

- [ ] None
- [ ] Assisted: I wrote it and AI helped with parts
- [ ] Generated: AI wrote most or all of it and I reviewed and tested every part

**Required.** Tick this too:

- [ ] I wrote this title and description myself and I understand every change in it.

If you ticked Assisted or Generated, say which parts and how you checked them:

## Checklist

- [ ] The change is focused and does one thing.
- [ ] `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.
- [ ] A change to capture or undo has a test that checks the restored bytes.
- [ ] Tests use temporary directories and a temporary `FOOTPRINTS_HOME` only.
- [ ] Output still reads correctly with `NO_COLOR=1`.
- [ ] The README, ACCESSIBILITY.md and CHANGELOG.md are updated if behaviour changed.
- [ ] I have read the [contributing guide](https://github.com/zaccesss/footprints/blob/main/CONTRIBUTING.md) and agree to the [code of conduct](https://github.com/zaccesss/footprints/blob/main/CODE_OF_CONDUCT.md).
- [ ] This does not disclose a security issue (those go through [SECURITY.md](https://github.com/zaccesss/footprints/blob/main/SECURITY.md) privately).
