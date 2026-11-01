#!/bin/sh
# Prepare the locked llama.cpp sys crate for FreeBSD and print Cargo's patch config.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
lock="$root/Cargo.lock"
version=$(awk '$0 == "name = \"llama-cpp-sys-2\"" { getline; gsub(/version = |"/, ""); print; exit }' "$lock")
[ -n "$version" ] || exit 0

cargo fetch --locked --manifest-path "$root/Cargo.toml" >/dev/null
src=
for candidate in "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/"llama-cpp-sys-2-$version"; do
    if [ -d "$candidate" ]; then src=$candidate; break; fi
done
[ -n "$src" ] || { echo "llama-cpp-sys-2-$version is missing from the Cargo registry" >&2; exit 1; }
dest="$root/target/llama-cpp-sys-2-freebsd"
rm -rf "$dest"
mkdir -p "$root/target"
cp -R "$src" "$dest"
(cd "$dest" && patch -s -p1 < "$root/packaging/llama/llama-cpp-sys-2-freebsd.patch") >&2

# Cargo needs a path source for the patched crate; the wrapper restores this lockfile after use.
awk '
    $0 == "name = \"llama-cpp-sys-2\"" { inside = 1 }
    inside && /^(source|checksum) = / { next }
    /^$/ { inside = 0 }
    { print }
' "$lock" > "$lock.tmp"
mv "$lock.tmp" "$lock"

echo "patch.crates-io.llama-cpp-sys-2.path=\"$dest\""
