"""Converts a Blitz3D (.b3d) model, as Luanti uses, to a Z-up glTF binary (.glb) in meters, with
its skeleton and named animation clips.

Luanti's b3d space is Irrlicht's: left-handed, Y up, 10 units per node. Output: right-handed,
Z up, +Y forward, 1 unit per meter, feet at z = 0. The texture isn't embedded; the mod names it.

A b3d file has one timeline. Clips are cut from it by frame ranges, as a Luanti mod's Lua file
gives them (`walk_start = 85, walk_end = 114, walk_speed = 20`), each a NAME:START-END@FPS
argument. Frames count from 0, as Luanti's do; b3d keys count from 1, which is why some tools
refuse these files. Each vertex keeps its four strongest bones.

usage: python3 -I b3d_to_glb.py in.b3d out.glb stand:35-75@20 walk:85-114@20 ...
"""
import json
import struct
import sys


def chunks(data, start, end):
    p = start
    while p + 8 <= end:
        tag = data[p:p + 4].decode("ascii")
        size = struct.unpack_from("<i", data, p + 4)[0]
        yield tag, p + 8, p + 8 + size
        p += 8 + size


def read_b3d(data):
    """Bones (name, parent, position, scale, rotation, keys), vertices, uvs, triangles and
    weights, all in b3d space. Keys are (frame, position, rotation), frames from 1."""
    bones, vertices, uvs, triangles = [], [], [], []
    weights = {}  # vertex -> [(bone, weight)]

    def node(start, end, parent):
        name_end = data.index(b"\0", start)
        name = data[start:name_end].decode("latin-1")
        p = name_end + 1
        pos = struct.unpack_from("<3f", data, p)
        scale = struct.unpack_from("<3f", data, p + 12)
        rot = struct.unpack_from("<4f", data, p + 24)
        p += 40
        me = None
        for tag, s, e in chunks(data, p, end):
            if tag == "MESH":
                mesh(s, e)
            elif tag == "BONE":
                if me is None:
                    me = len(bones)
                    bones.append(dict(name=name, parent=parent, pos=pos, scale=scale, rot=rot, keys=[]))
                for i in range((e - s) // 8):
                    v, w = struct.unpack_from("<if", data, s + i * 8)
                    if w > 0:
                        weights.setdefault(v, []).append((me, w))
            elif tag == "KEYS":
                if me is None:
                    me = len(bones)
                    bones.append(dict(name=name, parent=parent, pos=pos, scale=scale, rot=rot, keys=[]))
                flags = struct.unpack_from("<i", data, s)[0]
                stride = 4 + (12 if flags & 1 else 0) + (12 if flags & 2 else 0) + (16 if flags & 4 else 0)
                q = s + 4
                while q + stride <= e:
                    frame = struct.unpack_from("<i", data, q)[0]
                    k = q + 4
                    kpos = kscale = krot = None
                    if flags & 1:
                        kpos = struct.unpack_from("<3f", data, k)
                        k += 12
                    if flags & 2:
                        kscale = struct.unpack_from("<3f", data, k)
                        k += 12
                    if flags & 4:
                        krot = struct.unpack_from("<4f", data, k)
                    me_bone = bones[me]
                    if kscale is not None and any(abs(a - b) > 1e-4 for a, b in zip(kscale, me_bone["scale"])):
                        sys.exit(f"bone {name!r} animates its scale, which isn't supported")
                    me_bone["keys"].append((frame, kpos or me_bone["pos"], krot or me_bone["rot"]))
                    q += stride
            elif tag == "NODE":
                node(s, e, parent if me is None else me)

    def mesh(start, end):
        for tag, s, e in chunks(data, start + 4, end):  # after the brush id
            if tag == "VRTS":
                flags, sets, size = struct.unpack_from("<3i", data, s)
                q = s + 12
                stride = 12 + (12 if flags & 1 else 0) + (16 if flags & 2 else 0) + 4 * sets * size
                t_off = 12 + (12 if flags & 1 else 0) + (16 if flags & 2 else 0)
                while q + stride <= e:
                    vertices.append(struct.unpack_from("<3f", data, q))
                    uvs.append(struct.unpack_from("<2f", data, q + t_off) if sets > 0 and size >= 2 else (0.0, 0.0))
                    q += stride
            elif tag == "TRIS":
                q = s + 4
                while q + 12 <= e:
                    triangles.append(struct.unpack_from("<3i", data, q))
                    q += 12

    header = list(chunks(data, 0, len(data)))
    assert header[0][0] == "BB3D", "not a b3d file"
    _, s, e = header[0]
    for tag, cs, ce in chunks(data, s + 4, e):
        if tag == "NODE":
            node(cs, ce, -1)
    return bones, vertices, uvs, triangles, weights


# Irrlicht (x right, y up, z forward, left-handed) to Z-up right-handed (x east, y north, z up):
# swap y and z, which also flips handedness, and units to meters. A rotation seen through that
# mirror keeps its angle with its axis mirrored and negated. Blitz3D's quaternions turn the
# opposite way to the usual convention (its matrices are the transpose), which undoes the
# negation: b3d (w, x, y, z) becomes (w, x, z, y).
def point(v):
    return (v[0] / 10, v[2] / 10, v[1] / 10)


def rotation(q):
    w, x, y, z = q
    return (w, x, z, y)


def scale(s):
    return (s[0], s[2], s[1])


def quat_matrix(q):
    w, x, y, z = q
    return [
        [1 - 2 * (y * y + z * z), 2 * (x * y - w * z), 2 * (x * z + w * y)],
        [2 * (x * y + w * z), 1 - 2 * (x * x + z * z), 2 * (y * z - w * x)],
        [2 * (x * z - w * y), 2 * (y * z + w * x), 1 - 2 * (x * x + y * y)],
    ]


def trs(p, s, q):
    r = quat_matrix(q)
    return [[r[i][j] * s[j] for j in range(3)] + [p[i]] for i in range(3)] + [[0, 0, 0, 1]]


def mul(a, b):
    return [[sum(a[i][k] * b[k][j] for k in range(4)) for j in range(4)] for i in range(4)]


def inverse(m):
    a = [row[:3] for row in m[:3]]
    det = (a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
           + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]))
    c = [[(a[(j + 1) % 3][(i + 1) % 3] * a[(j + 2) % 3][(i + 2) % 3]
           - a[(j + 1) % 3][(i + 2) % 3] * a[(j + 2) % 3][(i + 1) % 3]) / det for j in range(3)] for i in range(3)]
    t = [m[i][3] for i in range(3)]
    return [c[i] + [-sum(c[i][k] * t[k] for k in range(3))] for i in range(3)] + [[0, 0, 0, 1]]


def apply(m, v):
    return tuple(sum(m[i][k] * v[k] for k in range(3)) + m[i][3] for i in range(3))


def parse_clip(arg):
    name, rest = arg.split(":")
    frames, fps = rest.split("@")
    start, end = frames.split("-")
    return name, int(start), int(end), float(fps)


def main(src, dst, clip_args):
    bones, vertices, uvs, triangles, weights = read_b3d(open(src, "rb").read())
    clips = [parse_clip(a) for a in clip_args]

    out_v = [point(v) for v in vertices]
    low = min(v[2] for v in out_v)
    out_v = [(x, y, z - low) for x, y, z in out_v]
    out_uv = [(u, v) for u, v in uvs]  # glTF and b3d both count rows from the top
    # Swapping two axes mirrors the triangles, so swap their winding back.
    out_tris = [(a, c, b) for a, b, c in triangles]

    # Each bone's local transform in Z-up meters. Roots also drop to the feet.
    local = []
    for b in bones:
        p = point(b["pos"])
        if b["parent"] < 0:
            p = (p[0], p[1], p[2] - low)
        local.append((p, scale(b["scale"]), rotation(b["rot"])))
    world = []
    for b, (p, s, q) in zip(bones, local):
        m = trs(p, s, q)
        world.append(m if b["parent"] < 0 else mul(world[b["parent"]], m))

    # Self-check: a bone's bind pose in the new space must carry its vertices where the old
    # space's did, and every vertex's bones must agree on where it sits.
    def old_world(i):
        b = bones[i]
        m = trs(b["pos"], b["scale"], (b["rot"][0], -b["rot"][1], -b["rot"][2], -b["rot"][3]))
        return m if b["parent"] < 0 else mul(old_world(b["parent"]), m)
    for i in range(len(bones)):
        probe = (1.0, 2.0, 3.0)
        a = apply(world[i], probe)
        # The probe is in bone space, which is mirrored too: (x, y, z) here is (x, z, y)*10 there.
        o = point(apply(old_world(i), (probe[0] * 10, probe[2] * 10, probe[1] * 10)))
        o = (o[0], o[1], o[2] - low)
        assert max(abs(x - y) for x, y in zip(a, o)) < 1e-3, f"bone {bones[i]['name']!r} doesn't map across"

    joints, wts = [], []
    for v in range(len(vertices)):
        ws = sorted(weights.get(v, []), key=lambda bw: -bw[1])[:4]
        if not ws:
            sys.exit(f"vertex {v} isn't bound to any bone")
        total = sum(w for _, w in ws)
        ws = [(b, w / total) for b, w in ws] + [(0, 0.0)] * (4 - len(ws))
        joints.append([b for b, _ in ws])
        wts.append([w for _, w in ws])

    # Binary buffer.
    blob = bytearray()
    views, accessors = [], []

    def add(data, target=None):
        while len(blob) % 4:
            blob.append(0)
        view = {"buffer": 0, "byteOffset": len(blob), "byteLength": len(data)}
        if target:
            view["target"] = target
        blob.extend(data)
        views.append(view)
        return len(views) - 1

    def accessor(values, comp, kind, fmt, target=None, bounds=False):
        flat = [x for v in values for x in (v if isinstance(v, (list, tuple)) else (v,))]
        view = add(struct.pack(f"<{len(flat)}{fmt}", *flat), target)
        acc = {"bufferView": view, "componentType": comp, "count": len(values), "type": kind}
        if bounds:
            n = len(values[0]) if isinstance(values[0], (list, tuple)) else 1
            col = [[v[i] if n > 1 else v for v in values] for i in range(n)]
            acc["min"] = [min(c) for c in col]
            acc["max"] = [max(c) for c in col]
        accessors.append(acc)
        return len(accessors) - 1

    FLOAT, USHORT, UINT = 5126, 5123, 5125
    position_acc = accessor(out_v, FLOAT, "VEC3", "f", 34962, bounds=True)
    uv_acc = accessor(out_uv, FLOAT, "VEC2", "f", 34962)
    joints_acc = accessor(joints, USHORT, "VEC4", "H", 34962)
    weights_acc = accessor(wts, FLOAT, "VEC4", "f", 34962)
    index_acc = accessor([i for t in out_tris for i in t], UINT, "SCALAR", "I", 34963)
    # Inverse bind matrices, column-major.
    ibm = [[inverse(m)[r][c] for c in range(4) for r in range(4)] for m in world]
    ibm_acc = accessor(ibm, FLOAT, "MAT4", "f")

    nodes = [{"name": "mesh", "mesh": 0, "skin": 0}]
    for i, (b, (p, s, q)) in enumerate(zip(bones, local)):
        node = {"name": b["name"], "translation": list(p), "rotation": [q[1], q[2], q[3], q[0]], "scale": list(s)}
        children = [j + 1 for j, c in enumerate(bones) if c["parent"] == i]
        if children:
            node["children"] = children
        nodes.append(node)
    roots = [i + 1 for i, b in enumerate(bones) if b["parent"] < 0]

    animations = []
    for name, start, end, fps in clips:
        samplers, channels = [], []
        times = [(f - start) / fps for f in range(start, end + 1)]
        time_acc = accessor(times, FLOAT, "SCALAR", "f", bounds=True)
        for i, b in enumerate(bones):
            keys = {frame - 1: (kp, kr) for frame, kp, kr in b["keys"]}
            if not keys:
                continue
            missing = [f for f in range(start, end + 1) if f not in keys]
            if missing:
                sys.exit(f"clip {name}: bone {b['name']!r} has no key at frame {missing[0]}")
            trans = []
            for f in range(start, end + 1):
                p = point(keys[f][0])
                trans.append((p[0], p[1], p[2] - low) if b["parent"] < 0 else p)
            rots = []
            for f in range(start, end + 1):
                w, x, y, z = rotation(keys[f][1])
                # Keep neighbors on the same side so blending takes the short way.
                if rots and sum(a * c for a, c in zip(rots[-1], (x, y, z, w))) < 0:
                    w, x, y, z = -w, -x, -y, -z
                rots.append((x, y, z, w))
            for path, values in (("translation", trans), ("rotation", rots)):
                out = accessor(values, FLOAT, "VEC3" if path == "translation" else "VEC4", "f")
                samplers.append({"input": time_acc, "output": out, "interpolation": "LINEAR"})
                channels.append({"sampler": len(samplers) - 1, "target": {"node": i + 1, "path": path}})
        animations.append({"name": name, "samplers": samplers, "channels": channels})

    gltf = {
        "asset": {"version": "2.0", "generator": f"b3d_to_glb.py from {src.split('/')[-1]}"},
        "scene": 0,
        "scenes": [{"nodes": [0] + roots}],
        "nodes": nodes,
        "meshes": [{"primitives": [{
            "attributes": {"POSITION": position_acc, "TEXCOORD_0": uv_acc, "JOINTS_0": joints_acc, "WEIGHTS_0": weights_acc},
            "indices": index_acc,
        }]}],
        "skins": [{"joints": [i + 1 for i in range(len(bones))], "inverseBindMatrices": ibm_acc}],
        "animations": animations,
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{"byteLength": len(blob)}],
    }
    while len(blob) % 4:
        blob.append(0)
    js = json.dumps(gltf, separators=(",", ":")).encode()
    js += b" " * (-len(js) % 4)
    total = 12 + 8 + len(js) + 8 + len(blob)
    with open(dst, "wb") as f:
        f.write(struct.pack("<4sII", b"glTF", 2, total))
        f.write(struct.pack("<I4s", len(js), b"JSON"))
        f.write(js)
        f.write(struct.pack("<I4s", len(blob), b"BIN\0"))
        f.write(blob)
    xs, ys, zs = zip(*out_v)
    print(f"{len(out_v)} vertices, {len(out_tris)} triangles, {len(bones)} bones, {len(clips)} clips; "
          f"bounds x {min(xs):.2f}..{max(xs):.2f} y {min(ys):.2f}..{max(ys):.2f} z 0..{max(zs):.2f}")


if __name__ == "__main__":
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2], sys.argv[3:])
