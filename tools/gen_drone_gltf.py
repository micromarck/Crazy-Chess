#!/usr/bin/env python3
"""Generates assets/models/drone.gltf: a small hovering drone with node animations.

It exists so the framework ships with a real glTF model that exercises the
custom-model + animation-clip pipeline without third-party assets. It is also a
reference for what the game expects from a model:

* the model faces -Z (its "eye" and gun point that way),
* the origin is on the ground at the tile centre,
* clips are separate glTF animations; their *index* is referenced from data
  (`models/drone.gltf#Animation0`), names are for humans.

Clips: 0 idle (loop), 1 walk (loop), 2 shoot, 3 hit, 4 death.

Usage:  python3 tools/gen_drone_gltf.py  (from the repository root)
"""
import base64
import json
import math
import struct
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "crates/tactics_game/assets/models/drone.gltf"

buf = bytearray()
views, accessors = [], []


def add_data(data: bytes, target=None) -> int:
    while len(buf) % 4:
        buf.append(0)
    view = {"buffer": 0, "byteOffset": len(buf), "byteLength": len(data)}
    if target:
        view["target"] = target
    buf.extend(data)
    views.append(view)
    return len(views) - 1


def accessor(values, comp_type, count, typ, minmax=False, target=None):
    fmt = {5126: "f", 5123: "H"}[comp_type]
    flat = [v for item in values for v in (item if isinstance(item, (list, tuple)) else [item])]
    view = add_data(struct.pack(f"<{len(flat)}{fmt}", *flat), target)
    acc = {"bufferView": view, "componentType": comp_type, "count": count, "type": typ}
    if minmax:
        n = len(flat) // count
        acc["min"] = [min(flat[i::n]) for i in range(n)]
        acc["max"] = [max(flat[i::n]) for i in range(n)]
    accessors.append(acc)
    return len(accessors) - 1


# --- unit cube mesh (24 verts so each face has its own normal) --------------
faces = [  # normal, then 4 corners (counter-clockwise seen from outside)
    ((1, 0, 0), [(1, -1, -1), (1, 1, -1), (1, 1, 1), (1, -1, 1)]),
    ((-1, 0, 0), [(-1, -1, 1), (-1, 1, 1), (-1, 1, -1), (-1, -1, -1)]),
    ((0, 1, 0), [(-1, 1, -1), (-1, 1, 1), (1, 1, 1), (1, 1, -1)]),
    ((0, -1, 0), [(-1, -1, 1), (-1, -1, -1), (1, -1, -1), (1, -1, 1)]),
    ((0, 0, 1), [(1, -1, 1), (1, 1, 1), (-1, 1, 1), (-1, -1, 1)]),
    ((0, 0, -1), [(-1, -1, -1), (-1, 1, -1), (1, 1, -1), (1, -1, -1)]),
]
positions, normals, indices = [], [], []
for normal, corners in faces:
    base = len(positions)
    for c in corners:
        positions.append([v * 0.5 for v in c])
        normals.append(list(normal))
    indices += [base, base + 1, base + 2, base, base + 2, base + 3]
pos_acc = accessor(positions, 5126, len(positions), "VEC3", minmax=True, target=34962)
nrm_acc = accessor(normals, 5126, len(normals), "VEC3", target=34962)
idx_acc = accessor(indices, 5123, len(indices), "SCALAR", target=34963)

materials = [
    {"name": "Hull", "pbrMetallicRoughness": {"baseColorFactor": [0.55, 0.58, 0.62, 1], "metallicFactor": 0.7, "roughnessFactor": 0.35}},
    {"name": "Eye", "pbrMetallicRoughness": {"baseColorFactor": [1, 0.2, 0.1, 1]}, "emissiveFactor": [1.0, 0.15, 0.05]},
    {"name": "Dark", "pbrMetallicRoughness": {"baseColorFactor": [0.08, 0.08, 0.1, 1], "metallicFactor": 0.5, "roughnessFactor": 0.5}},
]
meshes = [
    {"name": f"Cube{m}", "primitives": [{"attributes": {"POSITION": pos_acc, "NORMAL": nrm_acc}, "indices": idx_acc, "material": m}]}
    for m in range(3)
]

HOVER = 0.95
nodes = [
    {"name": "Drone", "children": [1, 2, 3, 4, 5], "translation": [0, HOVER, 0]},
    {"name": "Body", "mesh": 0, "scale": [0.6, 0.35, 0.6]},
    {"name": "Eye", "mesh": 1, "scale": [0.2, 0.12, 0.06], "translation": [0, 0.03, -0.31]},
    {"name": "RotorL", "mesh": 2, "scale": [0.55, 0.04, 0.12], "translation": [-0.42, 0.22, 0]},
    {"name": "RotorR", "mesh": 2, "scale": [0.55, 0.04, 0.12], "translation": [0.42, 0.22, 0]},
    {"name": "Gun", "mesh": 2, "scale": [0.08, 0.08, 0.4], "translation": [0, -0.22, -0.3]},
]


def quat(axis, degrees):
    half = math.radians(degrees) / 2
    s = math.sin(half)
    x, y, z = axis
    return [x * s, y * s, z * s, math.cos(half)]


def mul(a, b):  # quaternion product a*b
    ax, ay, az, aw = a
    bx, by, bz, bw = b
    return [aw * bx + ax * bw + ay * bz - az * by, aw * by - ax * bz + ay * bw + az * bx,
            aw * bz + ax * by - ay * bx + az * bw, aw * bw - ax * bx - ay * by - az * bz]


animations = []


def animation(name, channels):
    """channels: list of (node, path, times, values)."""
    samplers, chans = [], []
    for node, path, times, values in channels:
        t_acc = accessor(times, 5126, len(times), "SCALAR", minmax=True)
        typ = "VEC4" if path == "rotation" else "VEC3"
        v_acc = accessor(values, 5126, len(values), typ)
        samplers.append({"input": t_acc, "output": v_acc, "interpolation": "LINEAR"})
        chans.append({"sampler": len(samplers) - 1, "target": {"node": node, "path": path}})
    animations.append({"name": name, "samplers": samplers, "channels": chans})


def spin(period, length, steps_per_turn=8):
    n = int(length / period * steps_per_turn)
    times = [i * length / n for i in range(n + 1)]
    return times, [quat((0, 1, 0), -360 * t / period) for t in times]


# 0 idle: gentle bob + spinning rotors (2 s loop)
times = [0, 1, 2]
rt, rv = spin(0.5, 2.0)
animation("idle", [
    (0, "translation", times, [[0, HOVER, 0], [0, HOVER + 0.1, 0], [0, HOVER, 0]]),
    (0, "rotation", [0, 2], [quat((1, 0, 0), 0)] * 2),
    (3, "rotation", rt, rv),
    (4, "rotation", rt, rv),
])
# 1 walk: lean forward, faster rotors (1 s loop)
rt, rv = spin(0.25, 1.0)
animation("walk", [
    (0, "translation", [0, 0.5, 1], [[0, HOVER + 0.05, 0], [0, HOVER + 0.12, 0], [0, HOVER + 0.05, 0]]),
    (0, "rotation", [0, 1], [quat((1, 0, 0), -15)] * 2),
    (3, "rotation", rt, rv),
    (4, "rotation", rt, rv),
])
# 2 shoot: recoil backwards and tilt up (0.5 s)
animation("shoot", [
    (0, "translation", [0, 0.08, 0.5], [[0, HOVER, 0], [0, HOVER + 0.03, 0.28], [0, HOVER, 0]]),
    (0, "rotation", [0, 0.08, 0.5], [quat((1, 0, 0), 0), quat((1, 0, 0), 12), quat((1, 0, 0), 0)]),
    (5, "scale", [0, 0.08, 0.5], [[0.08, 0.08, 0.4], [0.14, 0.14, 0.5], [0.08, 0.08, 0.4]]),
])
# 3 hit: wobble (0.45 s)
animation("hit", [
    (0, "rotation", [0, 0.1, 0.25, 0.45],
     [quat((0, 0, 1), 0), quat((0, 0, 1), 25), quat((0, 0, 1), -15), quat((0, 0, 1), 0)]),
    (0, "translation", [0, 0.45], [[0, HOVER, 0], [0, HOVER, 0]]),
])
# 4 death: sputter, tip over and drop (1.2 s); holds the last frame
tilt = [mul(quat((1, 0, 0), a), quat((0, 0, 1), b)) for a, b in [(0, 0), (10, -20), (35, 15), (80, 30)]]
animation("death", [
    (0, "rotation", [0, 0.3, 0.7, 1.2], tilt),
    (0, "translation", [0, 0.3, 0.7, 1.2], [[0, HOVER, 0], [0, HOVER + 0.15, 0], [0, 0.5, 0.1], [0, 0.18, 0.2]]),
])

gltf = {
    "asset": {"version": "2.0", "generator": "tactics tools/gen_drone_gltf.py"},
    "scene": 0,
    "scenes": [{"name": "Drone", "nodes": [0]}],
    "nodes": nodes,
    "meshes": meshes,
    "materials": materials,
    "animations": animations,
    "accessors": accessors,
    "bufferViews": views,
    "buffers": [{"byteLength": len(buf), "uri": "data:application/octet-stream;base64," + base64.b64encode(bytes(buf)).decode()}],
}
OUT.parent.mkdir(parents=True, exist_ok=True)
OUT.write_text(json.dumps(gltf, indent=1))
print(f"wrote {OUT} ({len(buf)} bytes of binary data, {len(animations)} animations)")
