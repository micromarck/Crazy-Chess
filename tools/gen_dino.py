#!/usr/bin/env python3
"""Generates "Lost Valley" (assets/missions/valley.mission.xml).

    python3 tools/gen_dino.py

A jungle valley with dinosaurs (the Dinosaurs module): recover a lost
expedition's data drive from their camp, then get everyone to the evac
clearing in the north. Raptor packs lurk in the ferns, a Hornback (2x2)
grazes by the river, and a Tyrant (3x3) crashes out of the eastern jungle
once the drive is taken (or at round 6). Trails and clearings are at least
3 tiles wide so the big ones can get around.

Coordinates are mission coordinates: x = column, y = row from the TOP.
"""

from __future__ import annotations

import math
import os
import random
import sys

sys.path.insert(0, os.path.dirname(__file__))
from gen_missions import Level, write  # noqa: E402

W, H = 46, 36


def build() -> Level:
    L = Level(W, H)
    rng = random.Random(1993)
    # A river across the valley (rows 15-16), with two fords.
    for x in range(W):
        for y in (15, 16):
            if not (8 <= x <= 12 or 30 <= x <= 35):
                L.set(x, y, "#")
    # Jungle: dense trees except on the trails, clearings and the river.
    open_areas = [
        (2, 28, 14, 35),    # landing zone (south-west)
        (16, 6, 26, 12),    # expedition camp (north-middle)
        (14, 0, 30, 4),     # evac clearing (north)
        (36, 18, 45, 26),   # eastern clearing (the Tyrant's)
        (24, 20, 34, 28),   # grazing meadow
    ]
    trails = [
        [(8, 31), (10, 22), (10, 17), (12, 13), (20, 10)],      # LZ -> west ford -> camp
        [(20, 9), (22, 3)],                                       # camp -> evac
        [(12, 30), (24, 26), (32, 22), (33, 17), (30, 11), (24, 8)],  # LZ -> meadow -> east ford -> camp
        [(33, 24), (40, 22)],                                     # meadow -> eastern clearing
    ]

    def clear(x, y):
        return any(x0 <= x <= x1 and y0 <= y <= y1 for x0, y0, x1, y1 in open_areas)

    trail_tiles = set()
    for path in trails:
        for (x0, y0), (x1, y1) in zip(path, path[1:]):
            n = max(abs(x1 - x0), abs(y1 - y0)) * 3
            for i in range(n + 1):
                cx = x0 + (x1 - x0) * i / n
                cy = y0 + (y1 - y0) * i / n
                for dx in range(-2, 3):
                    for dy in range(-2, 3):
                        trail_tiles.add((round(cx) + dx, round(cy) + dy))

    for y in range(H):
        for x in range(W):
            if L.tiles[y][x] == "#" or clear(x, y) or (x, y) in trail_tiles:
                continue
            r = rng.random()
            if r < 0.55:
                L.set(x, y, "H")      # tree
            elif r < 0.75:
                L.set(x, y, "h")      # ferns
    # Scattered cover in the squad's clearings (LZ, camp, evac): ferns,
    # logs, boulders. The meadow and the eastern clearing stay open (only a
    # fringe of ferns): the big beasts need the room.
    for i, (x0, y0, x1, y1) in enumerate(open_areas):
        big = i >= 3
        for _ in range((x1 - x0) * (y1 - y0) // (40 if big else 14)):
            x, y = rng.randint(x0, x1), rng.randint(y0, y1)
            if big and x0 < x < x1 and y0 < y < y1:
                continue  # only the edges
            L.set(x, y, "h" if big or rng.random() < 0.7 else "H")
    # The expedition camp: crates and a ruined wall around the terminal.
    for x in range(18, 25):
        L.set(x, 6, "H" if x not in (21,) else ".")
    for (x, y) in [(18, 8), (24, 8), (19, 10), (23, 10)]:
        L.set(x, y, "h")
    # A low ridge the squad can hold near the LZ.
    for y in range(29, 34):
        for x in range(2, 6):
            L.heights[y][x] = 1
    return L


def populate(L: Level):
    L.deploys.append(dict(team="player", squad="player", x=8, y=32))
    # Keep the camp, LZ, trails and spawns walkable.
    for (x, y) in [(21, 8), (21, 7), (8, 32)]:
        L.set(x, y, ".")

    def spot(x, y, side=1):
        """Nearest place where a side x side beast fits (corner = lowest x,
        lowest *grid* y = bottom row in mission coords)."""
        for r in range(0, 8):
            for dy in range(-r, r + 1):
                for dx in range(-r, r + 1):
                    cx, cy = x + dx, y + dy
                    tiles = [(cx + i, cy - j) for i in range(side) for j in range(side)]
                    if all(0 <= tx < W and 0 <= ty < H and L.tiles[ty][tx] == "." for tx, ty in tiles):
                        return cx, cy
        raise SystemExit(f"no room for {side}x{side} near {x},{y}")

    # Raptor packs in the ferns (dormant until they see or hear you).
    packs = {"west_pack": [(5, 20), (7, 19), (4, 22)], "meadow_pack": [(27, 23), (30, 25), (26, 26)], "camp_pack": [(17, 11), (25, 11)]}
    for pod, spots in packs.items():
        for i, (x, y) in enumerate(spots):
            x, y = spot(x, y)
            L.unit("enemy", "dino_raptor", f"Raptor {pod[0].upper()}{i + 1}", x, y, dormant=True, pod=pod)
            L.set(x, y, ".")
    # The Hornback grazes by the east ford.
    x, y = spot(31, 21, 2)
    L.unit("enemy", "dino_hornback", "Hornback", x, y, dormant=True, pod="grazer", patrol=[(x, y), (x - 4, y + 3), (x + 2, y + 3)])

    L.tile_script(21, 8, """<script on="enter" team="player">
      if flag("drive") == 0 {
        set_flag("drive", 1);
        message(unit_name(unit) + " has the expedition's drive. Now get everyone to the evac clearing (north)!");
        objective("Get the whole squad to the evac clearing (north)");
        zone_label("drive", "DRIVE (taken)");
        tyrant();
      }
    </script>""", marker="objective", label="EXPEDITION DRIVE", tag="drive")
    L.zone("evac", 17, 0, 9, 3, """<script on="enter" team="player">
      if flag("drive") == 1 {
        let squad = alive_count("player");
        let here = zone_count("evac", "player");
        if squad > 0 && here >= squad { message("Everyone's aboard - off the island with the drive. Mission complete!"); win(); }
        else { message(here + " of " + squad + " at the evac - everyone still alive has to make it."); }
      } else { message("Not without the drive - it's at the expedition camp."); }
    </script>""", marker="evac", label="EVAC")
    tx, ty = spot(41, 22, 3)
    L.library = f"""
    // The Tyrant arrives (once).
    fn tyrant() {{
      if flag("tyrant") == 0 {{
        set_flag("tyrant", 1);
        spawn_unit("enemy", "dino_tyrant", "Tyrant", {tx}, {ty});
        message("A roar shakes the trees in the east. Something HUGE heard that.");
      }}
    }}"""
    L.script("start", """
    set_flag("drive", 0);
    message("The expedition went silent three days ago. Their camp is north, across the river.");
    message("Raptors hunt in packs. The big ones take several tiles - they can't fit everywhere.");
    objective("Recover the expedition's drive at their camp (north, across the river)");""")
    L.script("turn_start", """
    if round == 6 { tyrant(); }
    if flag("drive") == 1 && alive_count("player") > 0 && zone_count("evac", "player") >= alive_count("player") {
      message("Everyone's aboard - off the island with the drive. Mission complete!");
      win();
    }""", team="player")
    L.script("unit_killed", """
    if unit_class(unit) == "dino_tyrant" { message("The Tyrant crashes down. The jungle goes very quiet."); }""")


def surfaces(L: Level) -> list[str]:
    rows = []
    for y in range(L.h):
        row = ""
        for x in range(L.w):
            t = L.tiles[y][x]
            if t == "#":
                row += "W"
            elif t == "H":
                row += "R" if (x * 5 + y * 3) % 7 == 0 else "T"
            elif t == "h":
                row += "G"
            elif 17 <= x <= 25 and 0 <= y <= 2:
                row += "c"
            else:
                row += "d" if (x * 7 + y * 11) % 9 == 0 else "g"
        rows.append(row)
    return rows


def main():
    L = build()
    populate(L)
    L.check("valley")
    text = L.xml(
        "  Lost Valley: a jungle with dinosaurs (Dinosaurs module). Raptors (1 tile),\n"
        "  a Hornback (2x2) and a Tyrant (3x3).",
        'id="valley" name="Lost Valley" seed="65000" fog="true" win="script"',
        "A lost expedition's camp sits across a river in a jungle valley full of dinosaurs. Recover their data drive, then get "
        "the whole squad to the evac clearing in the north. Raptors hunt in packs, a Hornback grazes by the ford, and taking "
        "the drive draws out the Tyrant. The big ones take several tiles: they need room to move.",
    )
    text = text.replace("Generated by tools/gen_missions.py", "Generated by tools/gen_dino.py")
    # Water for the river; explicit surfaces.
    text = text.replace("  </layout>", "  </layout>\n  <surfaces>\n" + "\n".join("    " + r for r in surfaces(L)) + "\n  </surfaces>", 1)
    text = text.replace("  <layout>", '  <sky top="#3d6f9a" horizon="#d9e7c0"/>\n\n  <layout>', 1)
    write("valley", text)


if __name__ == "__main__":
    main()
