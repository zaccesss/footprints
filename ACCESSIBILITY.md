# Accessibility

Footprints is a command-line tool. Its output is what people read when something has gone wrong
with their files, so it has to be clear without colour, at any font size and through a screen
reader.

## The promise

- **Words carry the meaning, never colour alone.** The `STATE` column says `reversible`,
  `not reversible`, `pending` or `undone` in words. Diff lines start with `+` or `-`. Refusals
  start with `footprints: refused:` and errors with `footprints: error:`.
- **Colour is optional.** Colour is only used when standard output is a terminal. It is turned off
  by `NO_COLOR` set to any value (see [no-color.org](https://no-color.org)), by `TERM=dumb` and by
  the `--no-color` flag on every command. Piped and redirected output never has escape codes.
- **Plain characters only.** Tables are aligned with spaces. There are no box-drawing characters,
  emoji, spinners or progress bars, so a braille display or a screen reader's line mode reads each
  row in order.
- **Nothing moves.** No output animates or redraws itself. No command waits for a key press.
- **Machine-readable output.** `log`, `show` and `undo` take `--json` for anyone who prefers their
  own viewer or a script.

## Colours used

Footprints uses only the 16 standard ANSI colours, so the terminal's own theme decides the actual
shade and contrast.

| Where | Colour | Also shown by |
| --- | --- | --- |
| Table headers and diff file names | Bold | Position at the top |
| Added diff lines | Green | A leading `+` |
| Removed diff lines | Red | A leading `-` |
| Diff hunk headers | Cyan | A leading `@@` |
| `not reversible` in the log | Yellow | The words themselves |

> [!WARNING]
> Red and green diff lines can look alike with some colour vision types. The `+` and `-` markers
> always carry the meaning. Set `NO_COLOR=1` if the colours get in the way.

## Known limits

- Long commands are clipped to 72 characters in `footprints log`, ending with `...`. The full
  command is in `footprints show <id>`.
- Paths in the log are relative to the current directory where possible. A long absolute path can
  still wrap on a narrow terminal.
- Times are shown in the machine's local time zone as `YYYY-MM-DD HH:MM:SS`.

Many of these choices are preferences. Open an issue or a pull request if a change would help
others.

## The shared statement

> [!NOTE]
> The shared accessibility statement is in
> [zaccesss/accessibility](https://github.com/zaccesss/accessibility) and on
> [isaacadjei.me/accessibility](https://isaacadjei.me/accessibility). This file takes precedence
> where the two differ.
