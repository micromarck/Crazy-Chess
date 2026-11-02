#!/usr/bin/env python3
"""Generates assets/characters/robot.gltf: a skinned, rigged test character.

It is exported the way a typical Blender/Mixamo character arrives, to
exercise the drop-in pipeline:

* faces +Z (the glTF convention; the game rotates it 180 degrees),
* about 2.2 units tall (the game auto-fits it to soldier height),
* a real skin: 12 joints, every body part weighted 100% to one joint,
  inverse bind matrices, one skinned mesh with two materials,
* clips named like an exporter would: "Idle", "Walk", "Fire" (-> shoot),
  "Armature|Die" (-> death, prefix stripped) and "Wave" (custom slot).

Usage:  python3 tools/gen_robot_gltf.py   (from the repository root)
"""
import base64
import json
import math
import struct
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "crates/tactics_game/assets/characters/robot.gltf"

# --- skeleton (glTF: Y up, character faces +Z, its left is +X) -------------
# name, parent index, translation relative to parent
JOINTS = [
    ("Hips", None, (0.0, 1.0, 0.0)),
    ("Spine", 0, (0.0, 0.15, 0.0)),
    ("Chest", 1, (0.0, 0.35, 0.0)),
    ("Head", 2, (0.0, 0.4, 0.0)),
    ("UpperArm.L", 2, (0.4, 0.25, 0.0)),
    ("LowerArm.L", 4, (0.0, -0.45, 0.0)),
    ("UpperArm.R", 2, (-0.4, 0.25, 0.0)),
    ("LowerArm.R", 6, (0.0, -0.45, 0.0)),
    ("UpperLeg.L", 0, (0.18, -0.05, 0.0)),
    ("LowerLeg.L", 8, (0.0, -0.45, 0.0)),
    ("UpperLeg.R", 0, (-0.18, -0.05, 0.0)),
    ("LowerLeg.R", 10, (0.0, -0.45, 0.0)),
]
GLOBAL = []
for name, parent, t in JOINTS:
    p = GLOBAL[parent] if parent is not None else (0.0, 0.0, 0.0)
    GLOBAL.append(tuple(a + b for a, b in zip(p, t)))
J = {name: i for i, (name, _, _) in enumerate(JOINTS)}

# Body parts: (joint, centre in bind pose, size, material)
PARTS = [
    ("Hips", (0, 1.0, 0), (0.5, 0.2, 0.3), 0),
    ("Spine", (0, 1.27, 0), (0.42, 0.3, 0.26), 0),
    ("Chest", (0, 1.65, 0), (0.7, 0.45, 0.38), 0),
    ("Head", (0, 2.05, 0), (0.36, 0.34, 0.34), 0),
    ("Head", (0, 2.07, 0.17), (0.28, 0.09, 0.05), 1),   # visor on the front (+Z)
    ("UpperArm.L", (0.4, 1.52, 0), (0.16, 0.45, 0.16), 0),
    ("LowerArm.L", (0.4, 1.08, 0), (0.14, 0.42, 0.14), 0),
    ("UpperArm.R", (-0.4, 1.52, 0), (0.16, 0.45, 0.16), 0),
    ("LowerArm.R", (-0.4, 1.08, 0), (0.14, 0.42, 0.14), 0),
    ("UpperLeg.L", (0.18, 0.72, 0), (0.2, 0.45, 0.2), 0),
    ("LowerLeg.L", (0.18, 0.26, 0.03), (0.18, 0.5, 0.26), 0),
    ("UpperLeg.R", (-0.18, 0.72, 0), (0.2, 0.45, 0.2), 0),
    ("LowerLeg.R", (-0.18, 0.26, 0.03), (0.18, 0.5, 0.26), 0),
]

buf = bytearray()
views, accessors = [], []


def add(data: bytes, target=None) -> int:
    while len(buf) % 4:
        buf.append(0)
    v = {"buffer": 0, "byteOffset": len(buf), "byteLength": len(data)}
    if target:
        v["target"] = target
    buf.extend(data)
    views.append(v)
    return len(views) - 1


def accessor(flat, fmt, comp, count, typ, minmax=False, target=None):
    view = add(struct.pack(f"<{len(flat)}{fmt}", *flat), target)
    a = {"bufferView": view, "componentType": comp, "count": count, "type": typ}
    if minmax:
        n = len(flat) // count
        a["min"] = [min(flat[i::n]) for i in range(n)]
        a["max"] = [max(flat[i::n]) for i in range(n)]
    accessors.append(a)
    return len(accessors) - 1


FACES = [
    ((1, 0, 0), [(1, -1, -1), (1, 1, -1), (1, 1, 1), (1, -1, 1)]),
    ((-1, 0, 0), [(-1, -1, 1), (-1, 1, 1), (-1, 1, -1), (-1, -1, -1)]),
    ((0, 1, 0), [(-1, 1, -1), (-1, 1, 1), (1, 1, 1), (1, 1, -1)]),
    ((0, -1, 0), [(-1, -1, 1), (-1, -1, -1), (1, -1, -1), (1, -1, 1)]),
    ((0, 0, 1), [(1, -1, 1), (1, 1, 1), (-1, 1, 1), (-1, -1, 1)]),
    ((0, 0, -1), [(-1, -1, -1), (-1, 1, -1), (1, 1, -1), (1, -1, -1)]),
]

primitives = []
for material in (0, 1):
    pos, nrm, jnt, wgt, idx = [], [], [], [], []
    for joint, c, size, mat in PARTS:
        if mat != material:
            continue
        for normal, corners in FACES:
            base = len(pos) // 3
            for corner in corners:
                pos += [c[i] + corner[i] * size[i] / 2 for i in range(3)]
                nrm += list(normal)
                jnt += [J[joint], 0, 0, 0]
                wgt += [1.0, 0.0, 0.0, 0.0]
            idx += [base, base + 1, base + 2, base, base + 2, base + 3]
    n = len(pos) // 3
    primitives.append({
        "attributes": {
            "POSITION": accessor(pos, "f", 5126, n, "VEC3", True, 34962),
            "NORMAL": accessor(nrm, "f", 5126, n, "VEC3", target=34962),
            "JOINTS_0": accessor(jnt, "H", 5123, n, "VEC4", target=34962),
            "WEIGHTS_0": accessor(wgt, "f", 5126, n, "VEC4", target=34962),
        },
        "indices": accessor(idx, "H", 5123, len(idx), "SCALAR", target=34963),
        "material": material,
    })

# Inverse bind matrices: the bind pose has no rotation, so each is a
# translation by minus the joint's global position (column-major).
ibm = []
for g in GLOBAL:
    ibm += [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, -g[0], -g[1], -g[2], 1]
ibm_acc = accessor(ibm, "f", 5126, len(GLOBAL), "MAT4")

nodes = []
for i, (name, parent, t) in enumerate(JOINTS):
    node = {"name": name, "translation": list(t)}
    kids = [k for k, (_, p, _) in enumerate(JOINTS) if p == i]
    if kids:
        node["children"] = kids
    nodes.append(node)
MESH_NODE = len(nodes)
nodes.append({"name": "RobotMesh", "mesh": 0, "skin": 0})


def q(axis, deg):
    h = math.radians(deg) / 2
    s = math.sin(h)
    return [axis[0] * s, axis[1] * s, axis[2] * s, math.cos(h)]


X, Y, Z = (1, 0, 0), (0, 1, 0), (0, 0, 1)
animations = []


def anim(name, rot=None, trans=None):
    """rot/trans: {joint: [(time, value), ...]}; rotation values are (axis, degrees)."""
    samplers, channels = [], []
    for path, tracks in (("rotation", rot or {}), ("translation", trans or {})):
        for joint, keys in tracks.items():
            times = [k[0] for k in keys]
            if path == "rotation":
                vals = [v for _, (axis, deg) in keys for v in q(axis, deg)]
                typ = "VEC4"
            else:
                vals = [v for _, t in keys for v in t]
                typ = "VEC3"
            t_acc = accessor(times, "f", 5126, len(times), "SCALAR", True)
            v_acc = accessor(vals, "f", 5126, len(times), typ)
            samplers.append({"input": t_acc, "output": v_acc, "interpolation": "LINEAR"})
            channels.append({"sampler": len(samplers) - 1, "target": {"node": J[joint], "path": path}})
    animations.append({"name": name, "samplers": samplers, "channels": channels})


# Facing +Z: swinging a hanging limb forward is a NEGATIVE rotation about X.
anim("Idle", rot={
    "Chest": [(0, (X, 0)), (1, (X, -3)), (2, (X, 0))],
    "Head": [(0, (Y, 0)), (0.7, (Y, 12)), (1.4, (Y, -12)), (2, (Y, 0))],
    "UpperArm.L": [(0, (Z, 8)), (1, (Z, 11)), (2, (Z, 8))],
    "UpperArm.R": [(0, (Z, -8)), (1, (Z, -11)), (2, (Z, -8))],
})
anim("Walk", rot={
    "UpperLeg.L": [(0, (X, -30)), (0.4, (X, 30)), (0.8, (X, -30))],
    "UpperLeg.R": [(0, (X, 30)), (0.4, (X, -30)), (0.8, (X, 30))],
    "LowerLeg.L": [(0, (X, 5)), (0.2, (X, 10)), (0.4, (X, 5)), (0.6, (X, 55)), (0.8, (X, 5))],
    "LowerLeg.R": [(0, (X, 5)), (0.2, (X, 55)), (0.4, (X, 5)), (0.6, (X, 10)), (0.8, (X, 5))],
    "UpperArm.L": [(0, (X, 25)), (0.4, (X, -25)), (0.8, (X, 25))],
    "UpperArm.R": [(0, (X, -25)), (0.4, (X, 25)), (0.8, (X, -25))],
    "Chest": [(0, (Y, 6)), (0.4, (Y, -6)), (0.8, (Y, 6))],
}, trans={
    "Hips": [(0, (0, 1.0, 0)), (0.2, (0, 0.96, 0)), (0.4, (0, 1.0, 0)), (0.6, (0, 0.96, 0)), (0.8, (0, 1.0, 0))],
})
anim("Fire", rot={
    "UpperArm.R": [(0, (X, 0)), (0.15, (X, -88)), (0.3, (X, -72)), (0.45, (X, -88)), (0.6, (X, 0))],
    "UpperArm.L": [(0, (X, 0)), (0.15, (X, -70)), (0.45, (X, -70)), (0.6, (X, 0))],
    "Chest": [(0, (X, 0)), (0.3, (X, 8)), (0.6, (X, 0))],
})
anim("Armature|Die", rot={
    "Hips": [(0, (X, 0)), (0.4, (X, -15)), (1.2, (X, 85))],
    "UpperLeg.L": [(0, (X, 0)), (1.2, (X, -80))],
    "UpperLeg.R": [(0, (X, 0)), (1.2, (X, -70))],
    "Head": [(0, (X, 0)), (1.2, (X, 25))],
}, trans={
    "Hips": [(0, (0, 1.0, 0)), (0.4, (0, 0.85, 0)), (1.2, (0, 0.28, 0))],
})
anim("Wave", rot={
    "UpperArm.L": [(0, (Z, 0)), (0.3, (Z, 160)), (0.5, (Z, 135)), (0.7, (Z, 160)), (0.9, (Z, 135)), (1.2, (Z, 0))],
    "Head": [(0, (Z, 0)), (0.3, (Z, -8)), (0.9, (Z, -8)), (1.2, (Z, 0))],
})

gltf = {
    "asset": {"version": "2.0", "generator": "TBSium tools/gen_robot_gltf.py"},
    "scene": 0,
    "scenes": [{"name": "Robot", "nodes": [0, MESH_NODE]}],
    "nodes": nodes,
    "meshes": [{"name": "RobotMesh", "primitives": primitives}],
    "skins": [{"name": "Armature", "joints": list(range(len(JOINTS))), "inverseBindMatrices": ibm_acc, "skeleton": 0}],
    "materials": [
        {"name": "Plating", "pbrMetallicRoughness": {"baseColorFactor": [0.85, 0.55, 0.15, 1], "metallicFactor": 0.6, "roughnessFactor": 0.4}},
        {"name": "Visor", "pbrMetallicRoughness": {"baseColorFactor": [0.2, 0.9, 1.0, 1]}, "emissiveFactor": [0.2, 1.0, 1.2]},
    ],
    "animations": animations,
    "accessors": accessors,
    "bufferViews": views,
    "buffers": [{"byteLength": len(buf), "uri": "data:application/octet-stream;base64," + base64.b64encode(bytes(buf)).decode()}],
}
OUT.parent.mkdir(parents=True, exist_ok=True)
OUT.write_text(json.dumps(gltf, indent=1))
print(f"wrote {OUT}: {len(JOINTS)} joints, {len(animations)} clips: {[a['name'] for a in animations]}")
