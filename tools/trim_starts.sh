#!/usr/bin/env bash
# Cuts the dead air off the start of sound files, in place: each starts 2 ms before its sound
# does (the first point at a tenth of its peak), with a 1 ms fade-in so the cut doesn't click. Files
# already within 10 ms are left alone. TRIM_LEVEL sets the share of the peak that counts as
# the start (default 0.1). Prints each file's dead air before and after.
#
#   tools/trim_starts.sh mods/base/sounds/audition/*.ogg
#
# Ogg files are re-encoded (Vorbis, quality 6); WAV files stay WAV. Needs ffmpeg and python3.
set -euo pipefail
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Milliseconds before the first sample at TRIM_LEVEL of the peak.
onset() {
    ffmpeg -v error -i "$1" -ac 1 -ar 48000 -f f32le - | python3 -I -c '
import struct, sys
raw = sys.stdin.buffer.read()
a = struct.unpack("%df" % (len(raw) // 4), raw)
level = float(sys.argv[1])
peak = max((abs(x) for x in a), default=0) or 1
print("%.1f" % (next(i for i, x in enumerate(a) if abs(x) >= level * peak) / 48.0))' "${TRIM_LEVEL:-0.1}"
}

for f in "$@"; do
    before=$(onset "$f")
    if python3 -I -c "import sys; sys.exit(0 if float(sys.argv[1]) <= 10 else 1)" "$before"; then
        printf '%7s ms  kept     %s\n' "$before" "$f"
        continue
    fi
    start=$(python3 -I -c "import sys; print(max(0.0, float(sys.argv[1]) - 2) / 1000)" "$before")
    case ${f,,} in
        *.wav) codec=(-c:a pcm_s16le) ;;
        *) codec=(-c:a libvorbis -q:a 6) ;;
    esac
    out="$tmp/$(basename "$f")"
    ffmpeg -v error -y -i "$f" -af "atrim=start=$start,asetpts=PTS-STARTPTS,afade=t=in:d=0.001" "${codec[@]}" "$out"
    mv "$out" "$f"
    printf '%7s ms  -> %4s ms  %s\n' "$before" "$(onset "$f")" "$f"
done
