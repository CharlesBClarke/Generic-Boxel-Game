#!/usr/bin/env bash
# Benchmarks builds of the game, unattended: each build is started once per settings set in
# tools/bench/bench.txt, stands at each spot in it, measures the frame rate and quits (see the
# client's `bench` module). Results go to ~/.local/share/boxel/bench/results.txt, a screenshot
# of each spot to ~/.local/share/boxel/bench/shots/<build>/.
#
#   tools/bench.sh [--quick] [--sets a,b] [REV...]   bench each REV (default: the working tree)
#   tools/bench.sh --compare A B                     compare saved results of two builds
#   tools/bench.sh --clean                           remove the checkouts kept for old commits
#   tools/bench.sh --study FILE [REV...]             bench FILE's sets (such as tools/bench/shadows.txt)
#                                                    at bench.txt's spots, in bench.txt's world
#
# REV is `work` (the working tree, as edited) or any commit (`master`, `d02e0d4`). A commit is
# built in its own checkout under ~/.cache/boxel/bench/; one older than the bench gets it
# copied in by tools/bench/backport.sh (back to f9fbc16). With two or more REVs, the first is
# compared with each of the others at the end. --quick runs only the `shipped` set.
#
# The game's window holds the screen while it runs: leave the machine alone until it's done.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
bench_file=$repo/tools/bench/bench.txt
cache=${XDG_CACHE_HOME:-$HOME/.cache}/boxel/bench
out_dir=${XDG_DATA_HOME:-$HOME/.local/share}/boxel/bench
results=$out_dir/results.txt

quick=0 sets="" compare=0 study="" revs=()
while (($#)); do
    case $1 in
        --quick) quick=1 ;;
        --sets) sets=${2:?--sets needs a list}; shift ;;
        --compare) compare=1 ;;
        --study) study=${2:?--study needs a file}; shift ;;
        --clean)
            for dir in "$cache"/src/*/; do
                [[ -d $dir ]] && git -C "$repo" worktree remove --force "$dir" && echo "removed $dir"
            done
            git -C "$repo" worktree prune
            exit 0 ;;
        -h|--help) sed -n '2,/^set -e/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
        -*) echo "unknown option $1 (see --help)" >&2; exit 2 ;;
        *) revs+=("$1") ;;
    esac
    shift
done
((${#revs[@]})) || revs=(work)

# A study: bench.txt up to its sets (the world and the spots), then the study's own sets. A
# [bench] section in the study overrides bench.txt's timings or window.
if [[ -n $study ]]; then
    mkdir -p "$cache"
    combined=$cache/study-$(basename "$study")
    { sed '/^\[set /,$d' "$bench_file"; echo; cat "$study"; } >"$combined"
    bench_file=$combined
fi

# The build's name in the results: `work-<commit>` (with `+edited` if changed) for the working
# tree, the short commit otherwise.
label() {
    if [[ $1 == work ]]; then
        local head; head=$(git -C "$repo" rev-parse --short HEAD)
        if [[ -n $(git -C "$repo" status --porcelain --untracked-files=no) ]]; then echo "work-$head+edited"; else echo "work-$head"; fi
    else
        git -C "$repo" rev-parse --short "$1^{commit}"
    fi
}

# A build name as written in the results, from a REV or a name.
resolve() {
    if [[ $1 == work ]]; then
        grep -o 'build work-[^ ]*' "$results" 2>/dev/null | tail -n1 | cut -d' ' -f2 || true
    elif git -C "$repo" rev-parse -q --verify "$1^{commit}" >/dev/null 2>&1; then
        git -C "$repo" rev-parse --short "$1^{commit}"
    else
        echo "$1"
    fi
}

compare() {
    python3 -I "$repo/tools/bench/compare.py" "$results" "$(resolve "$1")" "$(resolve "$2")"
}

if ((compare)); then
    ((${#revs[@]} == 2)) || { echo "--compare needs two builds" >&2; exit 2; }
    compare "${revs[0]}" "${revs[1]}"
    exit 0
fi

if [[ -n $sets ]]; then
    IFS=, read -r -a set_list <<<"$sets"
elif ((quick)); then
    set_list=(shipped)
else
    mapfile -t set_list < <(sed -n 's/^\[set \(.*\)\]\s*$/\1/p' "$bench_file")
fi

# Builds REV and sets `src` (the checkout it runs in, whose mods it reads) and `bin`.
build() {
    local rev=$1 name target
    mkdir -p "$cache/bin"
    if [[ $rev == work ]]; then
        src=$repo
        (cd "$src" && cargo build --release -p game)
        target=$repo/target
        bin=$cache/bin/$(label work)
    else
        local short stamp; short=$(label "$rev")
        src=$cache/src/$short
        bin=$cache/bin/$short
        if [[ ! -d $src ]]; then
            mkdir -p "$cache/src"
            git -C "$repo" worktree add --detach "$src" "$short" >/dev/null
        fi
        # A commit older than the bench gets this tree's bench copied in. The stamp says which
        # bench a kept binary was built with: a commit's own, or a copy of these files.
        if git -C "$repo" cat-file -e "$short:crates/client/src/bench.rs" 2>/dev/null; then
            stamp=own
        else
            stamp=$(cat "$repo/crates/client/src/bench.rs" "$repo/tools/bench/switches_stub.rs" "$repo/tools/bench/backport.sh" | sha1sum | cut -d' ' -f1)
        fi
        if [[ -x $bin && $(cat "$bin.stamp" 2>/dev/null) == "$stamp" ]]; then
            echo "   already built"
            return
        fi
        [[ $stamp == own ]] || "$repo/tools/bench/backport.sh" "$src"
        # The checkouts share a target folder so the libraries are built once, but cargo names a
        # crate's output the same in every checkout and trusts file dates: without this it would
        # reuse another commit's crates. Marked changed, this checkout's crates are built again.
        find "$src/crates" "$src/Cargo.toml" "$src/Cargo.lock" -type f -exec touch {} +
        (cd "$src" && CARGO_TARGET_DIR=$cache/target cargo build --release -p game)
        target=$cache/target
    fi
    name=$(sed -n '/^\[\[bin\]\]/,/^name/ s/^name = "\(.*\)"/\1/p' "$src/crates/game/Cargo.toml" | head -n1)
    # A copy, so building the next commit in the shared target folder can't replace it.
    cp "$target/release/${name:-game}" "$bin"
    [[ $rev == work ]] || echo "$stamp" >"$bin.stamp"
}

labels=()
start=$(date +%s)
for rev in "${revs[@]}"; do
    build_name=$(label "$rev")
    labels+=("$build_name")
    echo "== $build_name: building"
    build "$rev"
    for set in "${set_list[@]}"; do
        echo "== $build_name: set $set"
        run=$(mktemp -d "$cache/run.XXXXXX")
        mkdir -p "$run/data" "$out_dir"
        status=0
        (cd "$src" && env XDG_DATA_HOME="$run/data" BOXEL_TUNING="$run/tuning.txt" \
            BOXEL_BENCH="$bench_file" BOXEL_BENCH_SET="$set" BOXEL_BENCH_OUT="$results" \
            BOXEL_BENCH_BUILD="$build_name" BOXEL_BENCH_SHOTS="$out_dir/shots/$build_name" \
            timeout 15m "$bin" >"$run/log" 2>&1) || status=$?
        # Settings this build didn't know, from its warnings about the tuning file.
        unknown=$(sed 's/\x1b\[[0-9;]*m//g' "$run/log" | grep -F "$run/tuning.txt: " | sed "s|.*$run/tuning.txt: ||" | paste -sd ';' - || true)
        if [[ -n $unknown ]]; then
            echo "# $(date '+%F %T') | build $build_name | set $set | didn't know: $unknown" | tee -a "$results"
        fi
        if ((status)); then
            echo "!! $build_name set $set failed (exit $status); its log: $run/log" >&2
            tail -n 20 "$run/log" >&2
        else
            grep -F "| build $build_name | " "$results" | grep -F "| set $set |" | tail -n "$(grep -c '^\[spot ' "$bench_file")" | cut -d'|' -f5-8
            rm -rf "$run"
        fi
    done
done
echo "== done in $(( $(date +%s) - start )) s; results in $results"
for other in "${labels[@]:1}"; do
    echo
    python3 -I "$repo/tools/bench/compare.py" "$results" "${labels[0]}" "$other"
done
