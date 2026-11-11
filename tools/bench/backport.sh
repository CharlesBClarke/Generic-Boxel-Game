#!/usr/bin/env bash
# Copies the bench into a checkout of a commit older than it: the client's `bench` module from
# this tree, a stub of its switches, and the two lines in the client's `lib.rs` that add it.
# Used by tools/bench.sh; works back to f9fbc16 (the perf recorder, 2026-11-05).
# Usage: tools/bench/backport.sh <checkout>
set -euo pipefail
here=$(cd "$(dirname "$0")/../.." && pwd)
src=${1:?usage: backport.sh <checkout>}
client=$src/crates/client/src
lib=$client/lib.rs
if [[ ! -f $client/perf.rs ]]; then
    echo "backport: $src is older than the perf recorder (f9fbc16); the bench can't run there" >&2
    exit 1
fi
cp "$here/crates/client/src/bench.rs" "$client/bench.rs"
mkdir -p "$client/bench"
cp "$here/tools/bench/switches_stub.rs" "$client/bench/switches.rs"
grep -q '^pub mod bench;' "$lib" || sed -i '0,/^pub mod /s//pub mod bench;\npub mod /' "$lib"
grep -q 'bench::BenchPlugin' "$lib" ||
    sed -i '/\.insert_resource(Time::<Fixed>::from_hz(protocol::TICK_RATE))/a\        .add_plugins(bench::BenchPlugin)' "$lib"
if ! grep -q '^pub mod bench;' "$lib" || ! grep -q 'bench::BenchPlugin' "$lib"; then
    echo "backport: couldn't find where to add the bench in $lib" >&2
    exit 1
fi
