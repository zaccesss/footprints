#!/usr/bin/env bash
# records the README demo from a real run: a small script stands in for an agent, edits a
# config file and creates a notes file under `footprints run`, then log, diff and undo are run
# on it. Prints the transcript, each command after a `$ ` prompt and its real output below it.
#
#   scripts/demo-capture.sh [path to the footprints binary] > demo.txt
#   scripts/demo-svg.py demo.txt assets/demo/demo
#
# the run happens in a fresh temporary folder with its own FOOTPRINTS_HOME, so a real ledger is
# never touched. The temporary folder is shown as /home/me in the transcript, since the real
# path names the machine's temporary directory and means nothing to a reader. Nothing else in
# the output is changed.
set -euo pipefail

bin="${1:-footprints}"
if [[ "$bin" == */* ]]; then
    bin="$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")"
fi
# pwd -P so the prefix matches the resolved path footprints prints (/private/var on macOS)
work="$(cd "$(mktemp -d)" && pwd -P)"
trap 'rm -rf "$work"' EXIT
export FOOTPRINTS_HOME="$work/ledger" NO_COLOR=1
mkdir -p "$work/home/me/demo"
cd "$work/home/me/demo"

printf 'theme = "light"\nfont_size = 14\nauto_save = false\n' > config.toml
cat > agent.sh <<'AGENT'
sed 's/auto_save = false/auto_save = true/' config.toml > config.toml.new
mv config.toml.new config.toml
printf '# Notes\n\nSaving is now automatic.\n' > NOTES.md
AGENT

# the id of the config.toml write, read from the ledger rather than assumed
write_id() {
    "$bin" log --json | python3 -c '
import json, sys
for action in json.load(sys.stdin):
    if action.get("kind") == "file_write" and str(action.get("target", "")).endswith("config.toml"):
        print(action["id"])
        break
'
}

{
    printf '$ cat agent.sh\n'
    cat agent.sh
    printf '$ footprints run --agent demo -- sh agent.sh\n'
    "$bin" run --agent demo -- sh agent.sh 2>&1
    printf '$ footprints log\n'
    "$bin" log 2>&1
    id="$(write_id)"
    printf '$ footprints diff %s\n' "$id"
    "$bin" diff "$id" 2>&1
    printf '$ footprints undo %s\n' "$id"
    "$bin" undo "$id" 2>&1
    printf '$ footprints log\n'
    "$bin" log 2>&1
} | sed "s|$work/home/me|/home/me|g"
