#!/usr/bin/env bash
set -euo pipefail

# fills the SHA256_ placeholders in the Homebrew formula from the .sha256 files attached to a
# release, writing the finished formula to an output folder so the template in this repository
# keeps its placeholders.
#
#   packaging/fill-checksums.sh v0.2.0 /path/to/output
#
# needs the GitHub CLI, logged in to an account that can read zaccesss/footprints.

tag="${1:?usage: fill-checksums.sh <tag> <output folder>}"
out="${2:?usage: fill-checksums.sh <tag> <output folder>}"
here="$(cd "$(dirname "$0")" && pwd)"
formula="$here/homebrew/footprints.rb"
sums="$(mktemp -d)"
trap 'rm -rf "$sums"' EXIT

# the formula names its version in the URLs, so a tag for another version would fill the wrong
# release's sums into it
version="${tag#v}"
# dots in a version are literal, not "any character"
pattern="version \"$(printf '%s' "$version" | sed 's/\./\\./g')\""
if ! grep -q "$pattern" "$formula"; then
    printf 'The formula is not for %s. Update its version and URLs first.\n' "$tag" >&2
    exit 1
fi

gh release download "$tag" --repo zaccesss/footprints --pattern '*.sha256' --dir "$sums"

mkdir -p "$out/homebrew"
cp "$formula" "$out/homebrew/"

for target in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-musl x86_64-unknown-linux-musl; do
    file="footprints-$tag-$target.tar.gz.sha256"
    sum="$(awk '{print $1}' "$sums/$file")"
    if [ "${#sum}" -ne 64 ]; then
        printf 'No 64 character sum in %s\n' "$file" >&2
        exit 1
    fi
    # the placeholder is the target triple in upper case with hyphens as underscores
    placeholder="SHA256_$(printf '%s' "$target" | tr 'a-z-' 'A-Z_')"
    sed -i.bak "s/$placeholder/$sum/" "$out/homebrew/footprints.rb"
    rm -f "$out/homebrew/footprints.rb.bak"
done

# a placeholder sits after a quote in the formula; the header comment only names the pattern
if grep -q '"SHA256_' "$out/homebrew/footprints.rb"; then
    printf 'A placeholder is still unfilled in %s\n' "$out/homebrew/footprints.rb" >&2
    exit 1
fi
printf 'Filled %s/homebrew/footprints.rb\n' "$out"
