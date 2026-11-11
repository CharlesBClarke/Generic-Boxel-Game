#!/usr/bin/env python3
"""Compares two builds' bench results: for each set and spot, the newest line of each build.

Usage: compare.py <results.txt> <build A> <build B>
       compare.py --self-test

Builds are named as in the results' `build` field. Only sets and spots both builds have are shown. A row whose
frame time grew by more than SLOWER is flagged, and a row where the two GPU clocks differ by more than CLOCK says so. Notes (lines starting with `#`, such as settings a build didn't know) are
printed for the two builds.
"""

import sys

SLOWER = 0.10
CLOCK = 0.05


def parse(lines):
    """Results lines as dicts: build, set, spot, ms, fps, low, window, gpu, and the line's notes."""
    rows, notes = [], []
    for line in lines:
        line = line.rstrip("\n")
        if not line.strip():
            continue
        if line.startswith("#"):
            notes.append(line)
            continue
        fields = [f.strip() for f in line.split(" | ")]
        row = {"date": fields[0]}
        for f in fields[1:]:
            for key in ("build", "set", "spot"):
                if f.startswith(key + " "):
                    row[key] = f[len(key) + 1:]
            if f.startswith("avg ") and f.endswith(" fps"):
                row["fps"] = float(f.split()[1])
            elif f.startswith("1% low "):
                row["low"] = float(f.split()[2])
            elif f.endswith(" ms") and f.split()[0].replace(".", "", 1).isdigit():
                row["ms"] = float(f.split()[0])
            elif "x" in f and f.replace("x", "", 1).isdigit():
                row["window"] = f
            elif f.startswith("gpu clock ") and "throttled " in f:
                row["mhz"] = float(f.split()[2])
        # The GPU's name is the field after the window size.
        if "window" in row:
            i = fields.index(row["window"])
            row["gpu"] = fields[i + 1] if i + 1 < len(fields) else "?"
        if all(k in row for k in ("build", "set", "spot", "ms")):
            rows.append(row)
    return rows, notes


def newest(rows, build):
    """The newest row of `build` for each (set, spot), in the order they first appear."""
    out = {}
    for row in rows:
        if row["build"] == build:
            out[(row["set"], row["spot"])] = row
    return out


def table(rows, a, b):
    """The comparison as lines of text."""
    old, new = newest(rows, a), newest(rows, b)
    if not old and not new:
        return [f"no results for {a} or {b}"]
    # Only what both builds were measured at; one build's study sets would bury the rest.
    keys = [k for k in old if k in new]
    if not keys:
        return [f"{a} and {b} share no set and spot (sets: {sorted({k[0] for k in old})} and {sorted({k[0] for k in new})})"]
    lines = [f"{'set':<12} {'spot':<12} {'A ms':>7} {'B ms':>7} {'change':>7}  {'A fps':>6} {'B fps':>6}  {'A 1%':>6} {'B 1%':>6}"]
    for key in keys:
        x, y = old.get(key), new.get(key)
        cell = lambda r, f, w: f"{r[f]:>{w}.1f}"
        change, flag = f"{'':>7}", ""
        if x and y and x["ms"] > 0:
            ratio = y["ms"] / x["ms"] - 1
            change = f"{ratio * 100:>+6.0f}%"
            flag = "  << slower" if ratio > SLOWER else ("  faster" if ratio < -SLOWER else "")
        # A GPU held to a lower clock on one side (heat, or a power budget shared with the CPU)
        # makes that side look slower than the build is.
        if "mhz" in x and "mhz" in y and abs(x["mhz"] / y["mhz"] - 1) > CLOCK:
            flag += f"  (GPU clock {x['mhz']:.0f} vs {y['mhz']:.0f} MHz)"
        lines.append(f"{key[0]:<12} {key[1]:<12} {cell(x, 'ms', 7)} {cell(y, 'ms', 7)} {change}  {cell(x, 'fps', 6)} {cell(y, 'fps', 6)}  {cell(x, 'low', 6)} {cell(y, 'low', 6)}{flag}")
    for what in ("window", "gpu"):
        seen = {r.get(what) for k in keys for r in (old[k], new[k])}
        if len(seen) > 1:
            lines.append(f"warning: the {what} differs between the runs: {', '.join(sorted(map(str, seen)))}")
    return lines


def main(argv):
    if argv[1:] == ["--self-test"]:
        return self_test()
    if len(argv) != 4:
        print(__doc__, file=sys.stderr)
        return 2
    path, a, b = argv[1:]
    with open(path) as f:
        rows, notes = parse(f)
    print(f"A = {a}\nB = {b}")
    for line in table(rows, a, b):
        print(line)
    for note in notes:
        if f"build {a} " in note or f"build {b} " in note:
            print(note)
    return 0


def self_test():
    results = """\
2026-11-10 10:00:00 | build abc1234 | release build | set shipped | spot hill | avg 50.0 fps | 1% low 40.0 fps | 20.00 ms | 1920x1080 | Intel(R) Iris(R) Xe | 1.00M triangles | 300 chunks drawn | render 512 m | -
2026-11-10 10:05:00 | build abc1234 | release build | set shipped | spot hill | avg 40.0 fps | 1% low 30.0 fps | 25.00 ms | 1920x1080 | Intel(R) Iris(R) Xe | 1.00M triangles | 300 chunks drawn | render 512 m | -
2026-11-10 11:00:00 | build work-def5678 | release build | set shipped | spot hill | avg 33.3 fps | 1% low 25.0 fps | 30.00 ms | 1920x1080 | Intel(R) Iris(R) Xe | 1.00M triangles | 300 chunks drawn | render 512 m | -
2026-11-10 11:01:00 | build work-def5678 | release build | set no_shadows | spot hill | avg 80.0 fps | 1% low 70.0 fps | 12.50 ms | 1920x1080 | Intel(R) Iris(R) Xe | 1.00M triangles | 300 chunks drawn | render 512 m | shadows.maps=0 | gpu clock 950 MHz, throttled 40% | 2 stalls
# 2026-11-10 11:01:00 | build work-def5678 | set no_shadows | didn't know: no section named [shadows]
""".splitlines()
    rows, notes = parse(results)
    assert len(rows) == 4 and len(notes) == 1, (rows, notes)
    assert rows[3]["gpu"] == "Intel(R) Iris(R) Xe" and rows[3]["window"] == "1920x1080"
    out = table(rows, "abc1234", "work-def5678")
    # The newer abc1234 line (25 ms) is the one compared: 30 / 25 is 20% slower.
    assert "+20%" in out[1] and "slower" in out[1], out
    assert len(out) == 2, "a set only one build has is left out: " + str(out)
    assert "share no set" in table(rows, "abc1234", "nobody")[0]
    rows[1]["mhz"], rows[2]["mhz"] = 900.0, 1080.0
    assert table(rows, "abc1234", "work-def5678")[1].endswith("(GPU clock 900 vs 1080 MHz)")
    rows[3]["window"] = "540x540"
    assert len(table(rows, "abc1234", "work-def5678")) == 2, "a window only an unshown set used is no warning"
    print("compare.py self-test passed")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
