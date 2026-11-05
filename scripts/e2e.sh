#!/usr/bin/env bash
# one real end-to-end run of a built binary: a script edits, creates and deletes files under
# the wrapper and under the daemon, then log, diff and undo must put back the exact bytes.
# Everything happens in a fresh temporary folder with FOOTPRINTS_HOME inside it.
#
#   scripts/e2e.sh target/release/footprints
set -euo pipefail

bin="${1:?usage: e2e.sh <path to the footprints binary>}"
bin="$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
export FOOTPRINTS_HOME="$work/home" NO_COLOR=1
mkdir -p "$work/project"
cd "$work/project"

printf 'line one\nline two\n' > edit.txt
printf '\000\001\002\377binary' > doomed.bin
cp edit.txt "$work/edit.orig"
cp doomed.bin "$work/doomed.orig"

cat > "$work/agent.sh" <<'AGENT'
printf 'line one\nline 2\n' > edit.txt
mkdir -p sub
printf 'brand new\n' > sub/created.txt
rm doomed.bin
AGENT

echo "== run"
"$bin" run --agent e2e --session e2e -- sh "$work/agent.sh"
echo "== log"
"$bin" log --session e2e
id_of() { "$bin" log --session e2e --kind "$1" --json | grep '"id"' | head -1 | tr -dc '0-9'; }
edit_id="$(id_of file_write)"
echo "== diff #$edit_id"
"$bin" diff "$edit_id" | tee "$work/diff.txt"
grep -q -- '-line two' "$work/diff.txt"
grep -q -- '+line 2' "$work/diff.txt"
echo "== undo the session"
"$bin" undo --session e2e
cmp edit.txt "$work/edit.orig"
cmp doomed.bin "$work/doomed.orig"
test ! -e sub/created.txt
echo "wrapper: bytes restored"

echo "== daemon"
"$bin" daemon --root . --snapshot --seconds 6 2> "$work/daemon.err" &
daemon=$!
for _ in $(seq 1 100); do
    grep -q 'daemon recording changes' "$work/daemon.err" 2>/dev/null && break
    sleep 0.1
done
sleep 1
printf 'changed by anyone\n' > edit.txt
printf 'made by anyone\n' > other.txt
rm doomed.bin
wait "$daemon"
cat "$work/daemon.err"
session="$(grep -o 'daemon-[0-9a-f]*' "$work/daemon.err" | head -1)"
"$bin" log --session "$session"
"$bin" undo --session "$session"
cmp edit.txt "$work/edit.orig"
cmp doomed.bin "$work/doomed.orig"
test ! -e other.txt
echo "daemon: bytes restored"
echo "e2e passed"
