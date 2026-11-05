#!/usr/bin/env bash
set -euo pipefail

# fills the SHA256_ placeholders in the Homebrew formula and the winget installer manifest from
# the .sha256 files attached to a release, writing the finished files to an output folder so the
# templates in this repository keep their placeholders.
#
#   packaging/fill-checksums.sh v0.2.0 /path/to/output
#   packaging/fill-checksums.sh v0.2.0 /path/to/output /folder/of/sha256/files
#
# needs the GitHub CLI, logged in to an account that can read zaccesss/footprints, unless a
# folder of .sha256 files is given as the third argument. The release workflow passes the files
# it already downloaded that way. The same route tests the script before a release exists.

usage='usage: fill-checksums.sh <tag> <output folder> [folder of .sha256 files]'
tag="${1:?$usage}"
out="${2:?$usage}"
here="$(cd "$(dirname "$0")" && pwd)"
formula="$here/homebrew/footprints.rb"
installer="zaccesss.Footprints.installer.yaml"

if [ -n "${3:-}" ]; then
    sums="$3"
else
    sums="$(mktemp -d)"
    trap 'rm -rf "$sums"' EXIT
    gh release download "$tag" --repo zaccesss/footprints --pattern '*.sha256' --dir "$sums"
fi

# the files name their version in the URLs, so a tag for another version would fill the wrong
# release's sums into them
version="${tag#v}"
# dots in a version are literal, not "any character"
escaped="$(printf '%s' "$version" | sed 's/\./\\./g')"
if ! grep -q "version \"$escaped\"" "$formula"; then
    printf 'The formula is not for %s. Update its version and URLs first.\n' "$tag" >&2
    exit 1
fi
for file in "$here/winget/"*.yaml; do
    if ! grep -q "^PackageVersion: $escaped\$" "$file"; then
        printf '%s is not for %s. Update its PackageVersion and URLs first.\n' "$file" "$tag" >&2
        exit 1
    fi
done

mkdir -p "$out/homebrew" "$out/winget"
cp "$formula" "$out/homebrew/"
cp "$here/winget/"*.yaml "$out/winget/"

for target in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-musl x86_64-unknown-linux-musl x86_64-pc-windows-msvc; do
    case "$target" in
        *windows*) file="footprints-$tag-$target.zip.sha256" ;;
        *) file="footprints-$tag-$target.tar.gz.sha256" ;;
    esac
    sum="$(awk '{print $1}' "$sums/$file")"
    if [ "${#sum}" -ne 64 ]; then
        printf 'No 64 character sum in %s\n' "$file" >&2
        exit 1
    fi
    # the placeholder is the target triple in upper case with hyphens as underscores
    placeholder="SHA256_$(printf '%s' "$target" | tr 'a-z-' 'A-Z_')"
    case "$target" in
        *windows*)
            # winget manifests carry the sum in upper case, as winget's own tools write it
            upper="$(printf '%s' "$sum" | tr 'a-f' 'A-F')"
            sed -i.bak "s/$placeholder/$upper/" "$out/winget/$installer"
            rm -f "$out/winget/$installer.bak" ;;
        *)
            sed -i.bak "s/$placeholder/$sum/" "$out/homebrew/footprints.rb"
            rm -f "$out/homebrew/footprints.rb.bak" ;;
    esac
done

# a placeholder sits after a quote in the formula and after a colon in the manifest; the header
# comments only name the pattern
if grep -rqE '"SHA256_|: SHA256_' "$out/homebrew" "$out/winget"; then
    printf 'A placeholder is still unfilled in %s\n' "$out" >&2
    exit 1
fi
printf 'Filled %s/homebrew/footprints.rb and %s/winget/\n' "$out" "$out"
