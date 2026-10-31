"""Converts a Blitz3D (.b3d) mesh, as Luanti uses, to a Z-up OBJ in meters, in its rest pose.

Bones and animation are ignored; vertices are taken as stored, moved by the mesh node's own
transform and its parents'. Luanti's b3d space is Irrlicht's: left-handed, Y up, 10 units per
node. Output: right-handed, Z up, +Y forward, 1 unit per meter, feet at z = 0.

usage: python3 -I b3d_to_obj.py in.b3d out.obj
"""
import struct
import sys


def quat_rotate(q, v):
    w, x, y, z = q
    # v' = q v q*
    tx = 2 * (y * v[2] - z * v[1])
    ty = 2 * (z * v[0] - x * v[2])
    tz = 2 * (x * v[1] - y * v[0])
    return (
        v[0] + w * tx + (y * tz - z * ty),
        v[1] + w * ty + (z * tx - x * tz),
        v[2] + w * tz + (x * ty - y * tx),
    )


class Reader:
    def __init__(self, data):
        self.data = data

    def chunks(self, start, end):
        p = start
        while p + 8 <= end:
            tag = self.data[p:p + 4].decode("ascii")
            size = struct.unpack_from("<i", self.data, p + 4)[0]
            yield tag, p + 8, p + 8 + size
            p += 8 + size

    def string(self, p):
        e = self.data.index(b"\0", p)
        return self.data[p:e].decode("latin-1"), e + 1


def main(src, dst):
    data = open(src, "rb").read()
    r = Reader(data)
    meshes = []  # (positions, uvs, triangles)

    def node(start, end, transforms):
        name, p = r.string(start)
        pos = struct.unpack_from("<3f", data, p)
        scale = struct.unpack_from("<3f", data, p + 12)
        rot = struct.unpack_from("<4f", data, p + 24)
        p += 40
        chain = transforms + [(pos, scale, rot)]
        for tag, s, e in r.chunks(p, end):
            if tag == "MESH":
                mesh(s, e, chain)
            elif tag == "NODE":
                node(s, e, chain)

    def apply(chain, v):
        for pos, scale, rot in reversed(chain):
            v = (v[0] * scale[0], v[1] * scale[1], v[2] * scale[2])
            v = quat_rotate(rot, v)
            v = (v[0] + pos[0], v[1] + pos[1], v[2] + pos[2])
        return v

    def mesh(start, end, chain):
        p = start + 4  # brush id
        positions, uvs, tris = [], [], []
        for tag, s, e in r.chunks(p, end):
            if tag == "VRTS":
                flags, sets, size = struct.unpack_from("<3i", data, s)
                q = s + 12
                stride = 12 + (12 if flags & 1 else 0) + (16 if flags & 2 else 0) + 4 * sets * size
                while q + stride <= e:
                    v = struct.unpack_from("<3f", data, q)
                    t = q + 12 + (12 if flags & 1 else 0) + (16 if flags & 2 else 0)
                    uv = struct.unpack_from("<2f", data, t) if sets > 0 and size >= 2 else (0.0, 0.0)
                    positions.append(apply(chain, v))
                    uvs.append(uv)
                    q += stride
            elif tag == "TRIS":
                q = s + 4
                while q + 12 <= e:
                    tris.append(struct.unpack_from("<3i", data, q))
                    q += 12
        meshes.append((positions, uvs, tris))

    header = list(r.chunks(0, len(data)))
    assert header[0][0] == "BB3D", "not a b3d file"
    _, s, e = header[0]
    for tag, cs, ce in r.chunks(s + 4, e):
        if tag == "NODE":
            node(cs, ce, [])

    # Irrlicht (x right, y up, z forward, left-handed) to Z-up right-handed (x east, y north,
    # z up): swap y and z, which also flips the handedness. Then units to meters.
    out_v, out_t, out_f = [], [], []
    for positions, uvs, tris in meshes:
        base = len(out_v)
        for (x, y, z), (u, v) in zip(positions, uvs):
            out_v.append((x / 10, z / 10, y / 10))
            out_t.append((u, 1 - v))
        for a, b, c in tris:
            # Swapping two axes mirrors the triangles, so swap their winding back.
            out_f.append((base + a, base + c, base + b))
    low = min(v[2] for v in out_v)
    out_v = [(x, y, z - low) for x, y, z in out_v]
    xs, ys, zs = zip(*out_v)
    with open(dst, "w") as f:
        f.write(f"# Converted from {src.split('/')[-1]} by b3d_to_obj.py: Z up, meters, feet at z = 0, facing +Y.\n")
        f.write(f"# Bounds: x {min(xs):.3f} to {max(xs):.3f}, y {min(ys):.3f} to {max(ys):.3f}, z 0 to {max(zs):.3f}\n")
        for x, y, z in out_v:
            f.write(f"v {x:.5f} {y:.5f} {z:.5f}\n")
        for u, v in out_t:
            f.write(f"vt {u:.5f} {v:.5f}\n")
        for a, b, c in out_f:
            f.write(f"f {a + 1}/{a + 1} {b + 1}/{b + 1} {c + 1}/{c + 1}\n")
    print(f"{len(out_v)} vertices, {len(out_f)} triangles, bounds x {min(xs):.2f}..{max(xs):.2f} y {min(ys):.2f}..{max(ys):.2f} z 0..{max(zs):.2f}")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
