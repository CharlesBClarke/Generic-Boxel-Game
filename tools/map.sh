#!/usr/bin/env bash
# Prints a map of the code: each crate with its description from Cargo.toml, then each of its
# modules with the first line of the module's `//!` doc comment. Reads the code every time, so
# it never goes stale. Run from anywhere: tools/map.sh
set -euo pipefail
cd "$(dirname "$0")/.."

for manifest in $(find crates -name Cargo.toml -not -path '*/target/*' | sort); do
    dir=$(dirname "$manifest")
    name=$(sed -n 's/^name = "\(.*\)"/\1/p' "$manifest" | head -n1)
    desc=$(sed -n 's/^description = "\(.*\)"/\1/p' "$manifest" | head -n1)
    echo "$name ($dir)${desc:+: $desc}"
    for file in $(find "$dir/src" -name '*.rs' | sort); do
        # The module doc's first sentence, joined across wrapped lines.
        line=$(awk '/^\/\/!/ { sub(/^\/\/! ?/, ""); if ($0 == "") exit; text = text (text ? " " : "") $0; if (text ~ /\.( |$)/) exit; next } text { exit } END { print text }' "$file" \
            | sed -E 's/\.( .*)?$/./')
        echo "  ${file#"$dir/src/"}: ${line:-(no module doc)}"
    done
done
