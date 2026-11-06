#!/usr/bin/env bash
# Plays every sound file in a folder in turn, to pick candidates quickly. Kept files are copied
# into mods/base/sounds/audition/ (spaces in names become underscores; MP3 and FLAC become Ogg,
# which the game reads) and listed at the end,
# ready to paste into a `files = [...]` line in mods/base/sounds.toml.
#
#   tools/audition.sh ~/Downloads/footsteps
#
# Keys: Enter next, r replay, k keep (and go on), b back, q quit. Needs ffplay (ffmpeg).
set -euo pipefail
dir=${1:?usage: tools/audition.sh <folder>}
keep="$(dirname "$0")/../mods/base/sounds/audition"
mapfile -t files < <(find "$dir" -maxdepth 1 -type f \( -iname '*.ogg' -o -iname '*.wav' -o -iname '*.mp3' -o -iname '*.flac' \) | sort -V)
if [ ${#files[@]} -eq 0 ]; then
    echo "no sound files in $dir"
    exit 1
fi
kept=()
i=0
while [ $i -lt ${#files[@]} ]; do
    f=${files[$i]}
    printf '\n[%d/%d] %s\n' $((i + 1)) ${#files[@]} "$(basename "$f")"
    ffplay -v error -nodisp -autoexit "$f" </dev/null
    read -rsn1 -p "  Enter next, r replay, k keep, b back, q quit " key || key=q
    echo
    case $key in
        r) continue ;;
        b) i=$(( i > 0 ? i - 1 : 0 )); continue ;;
        q) break ;;
        k)
            mkdir -p "$keep"
            name=$(basename "$f" | tr ' ' '_')
            case ${name,,} in
                *.ogg | *.wav) cp "$f" "$keep/$name" ;;
                # The game reads Ogg Vorbis and WAV only.
                *) name="${name%.*}.ogg"; ffmpeg -v error -y -i "$f" -c:a libvorbis -q:a 6 "$keep/$name" ;;
            esac
            kept+=("\"sounds/audition/$name\"")
            echo "  kept as sounds/audition/$name"
            ;;
    esac
    i=$((i + 1))
done
if [ ${#kept[@]} -gt 0 ]; then
    echo
    echo "files = [$(IFS=,; echo "${kept[*]}" | sed 's/,/, /g')]"
fi
