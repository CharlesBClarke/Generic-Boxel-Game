#!/usr/bin/env bash
# Bakes the base mod's generated sounds into mods/base/sounds/, so the game only ever plays
# files: Ogg Vorbis, or WAV for loops, since Vorbis trims a little and a loop must keep its
# exact length. Needs ffmpeg with libvorbis. Re-run after changing a recipe here.
#
#   tools/make_sounds.sh                  # what the game uses
#   tools/make_sounds.sh --extras <folder>  # also the recipes tried and not used, into <folder>
#
# What the game uses:
#   powerdown.ogg   the robot shutting down: a buzzy tone falling away.
#   wind.wav        rushing air, looped, for moving fast.
#   printer.wav     a real printer's whine, looped (temporary; see below).
# Extras, tried and not used (kept to try again):
#   whoosh_00N.ogg  a tool's swing: pink noise through a band-pass, swelling and dying away
#                   quickly, each variation centered a little differently.
#   stepper.wav     the neck's motor, like a 3D printer's stepper: a whine at its step rate
#                   (400 Hz), each step a sharp pulse, with a metallic resonance and a faint
#                   microstepping ripple. A one-second loop of whole cycles, so it repeats
#                   without a click; the client changes its pitch with turn speed.
#   chirp.ogg       a pick-up blip: a quick rising sine.
set -euo pipefail
out="$(dirname "$0")/../mods/base/sounds"
extras=""
if [ "${1:-}" = "--extras" ]; then
    extras=${2:?usage: tools/make_sounds.sh --extras <folder>}
    mkdir -p "$extras"
fi

# whoosh <path> <band center Hz> <seconds>
whoosh() {
    local file=$1 center=$2 length=$3
    ffmpeg -v error -y -f lavfi -i "anoisesrc=color=pink:duration=$length:sample_rate=44100:seed=$center" \
        -af "highpass=f=$((center / 2)),lowpass=f=$((center * 3)),bandpass=f=$center:width_type=q:w=0.8,volume=eval=frame:volume='pow(sin(PI*pow(t/$length,0.6)),2)*2.5'" \
        -ac 1 -c:a libvorbis -q:a 4 "$file"
}

if [ -n "$extras" ]; then
    whoosh "$extras/whoosh_000.ogg" 700 0.22
    whoosh "$extras/whoosh_001.ogg" 900 0.2
    whoosh "$extras/whoosh_002.ogg" 1100 0.18
    # stepper: 2 s generated, the first second dropped so the filters have settled, leaving 400
    # whole cycles that loop cleanly.
    step="mod(400*t,1)"
    ffmpeg -v error -y -f lavfi \
        -i "aevalsrc='(0.5*exp(-7*$step)+0.25*sin(2*PI*400*t)+0.14*sin(2*PI*800*t)+0.08*sin(2*PI*1200*t))*(1+0.12*sin(2*PI*100*t))':s=48000:d=2" \
        -af "highpass=f=180,equalizer=f=2600:t=q:w=2.5:g=7,lowpass=f=7000,atrim=start=1:end=2,asetpts=PTS-STARTPTS,volume=0.6" \
        -ac 1 -c:a pcm_s16le "$extras/stepper.wav"

    # chirp.ogg: a robot's pick-up blip, a sine rising from 700 to 1800 Hz in 80 ms, dying away.
    rise="2*PI*700*0.08/log(1800/700)*(pow(1800/700,t/0.08)-1)"
    ffmpeg -v error -y -f lavfi -i "aevalsrc='(0.7*sin($rise)+0.15*sin(2*$rise))*exp(-t*25)':s=48000:d=0.12" \
        -af "volume=0.8" -ac 1 -c:a libvorbis -q:a 6 "$extras/chirp.ogg"
fi

# powerdown.ogg: the robot shutting down when knocked out. A buzzy tone falling from 600 to
# 60 Hz over 0.9 s (an exponential sweep, so it sounds even), fading as it goes.
sweep="2*PI*600*0.9/log(0.1)*(pow(0.1,t/0.9)-1)"
ffmpeg -v error -y -f lavfi \
    -i "aevalsrc='(0.6*sin($sweep)+0.25*sgn(sin($sweep))+0.1*sin(3*$sweep))*(1-t/0.9)':s=48000:d=0.9" \
    -af "lowpass=f=5000,volume=0.7" -ac 1 -c:a libvorbis -q:a 6 "$out/powerdown.ogg"

# wind.wav: rushing air for moving fast, from the whoosh's recipe: pink noise through a wide
# band-pass, 2.25 s generated and its last quarter second crossfaded (equal power) into its
# first, so the 2 s loop repeats without a seam. The client sets its loudness and pitch by speed.
ffmpeg -v error -f lavfi -i "anoisesrc=color=pink:duration=2.25:sample_rate=48000:seed=7" \
    -af "highpass=f=250,lowpass=f=2500,bandpass=f=800:width_type=q:w=0.6" -ac 1 -f f32le - | python3 -I -c '
import struct, sys, wave
raw = sys.stdin.buffer.read()
a = struct.unpack("%df" % (len(raw) // 4), raw)
length, fade = 96000, 12000
out = list(a[:length])
for i in range(fade):
    w = i / fade
    out[i] = a[i] * w ** 0.5 + a[length + i] * (1 - w) ** 0.5
gain = 10 ** (-3 / 20) / max(abs(v) for v in out)
with wave.open(sys.argv[1], "wb") as f:
    f.setnchannels(1)
    f.setsampwidth(2)
    f.setframerate(48000)
    f.writeframes(b"".join(struct.pack("<h", int(max(-1, min(1, v * gain)) * 32767)) for v in out))
' "$out/wind.wav"

# printer.wav: a real 3D printer's steady whine (about 1180 Hz), looped. TEMPORARY, for
# experimenting: the source isn't licensed for shipping; replace it with our own recording
# before release. Baked only when the source is given:
#
#   PRINTER_SOURCE=~/Projects/boxel-sound-packs/downloads/hammy01-3d-printer-283745.mp3 tools/make_sounds.sh
#
# Takes 2.25 s from 2 s in, mono, and crossfades (equal power) its last quarter second into its first, so the
# 2 s loop repeats without a seam, then raises it to -3 dB peak.
if [ -n "${PRINTER_SOURCE:-}" ]; then
    ffmpeg -v error -ss 2 -t 2.25 -i "$PRINTER_SOURCE" -ac 1 -ar 48000 -f f32le - | python3 -I -c '
import struct, sys, wave
raw = sys.stdin.buffer.read()
a = struct.unpack("%df" % (len(raw) // 4), raw)
rate, length, fade = 48000, 96000, 12000
if len(a) < length + fade:
    sys.exit("printer source too short")
out = list(a[:length])
for i in range(fade):
    # Equal power, so the noise holds its level through the fade.
    w = i / fade
    out[i] = a[i] * w ** 0.5 + a[length + i] * (1 - w) ** 0.5
peak = max(abs(v) for v in out)
gain = 10 ** (-3 / 20) / peak
with wave.open(sys.argv[1], "wb") as f:
    f.setnchannels(1)
    f.setsampwidth(2)
    f.setframerate(rate)
    f.writeframes(b"".join(struct.pack("<h", int(max(-1, min(1, v * gain)) * 32767)) for v in out))
' "$out/printer.wav"
fi

echo "baked into $out"
