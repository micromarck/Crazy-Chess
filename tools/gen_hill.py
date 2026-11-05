#!/usr/bin/env python3
"""Generates "Hold the Hill" (assets/missions/hill.mission.xml).

    python3 tools/gen_hill.py

The squad holds a hilltop on a round island while zombies crawl up its
slopes, waiting 12 turns for an evac chopper. The hill is steep: the north
and east faces are cliffs (2+ levels per tile, which zombies can't climb),
the south and west faces are ramps of 1 level per tile, so the horde
funnels up those. Each zombie turn the script tops the hill up to 10-12
zombies; more gather at the island's edge, up to 30 alive in all.

Coordinates are mission coordinates: x = column, y = row from the TOP.
"""

from __future__ import annotations

import math
import os
import sys

sys.path.insert(0, os.path.dirname(__file__))
from gen_missions import Level, write  # noqa: E402

N = 40
CX, CY = 20, 20
TOP = 7            # plateau level
PLATEAU = 3.2      # plateau radius
ISLAND = 19.2      # beyond: water / void
ON_SCREEN = 12     # "on the hill": within this many tiles of the top
TURNS = 12         # evac arrives at the start of this round


def height(x: int, y: int) -> int:
    d = math.hypot(x - CX, y - CY)
    if d <= PLATEAU:
        return TOP
    steps = math.ceil(d - PLATEAU)
    # Cliffs on the north and east faces: 2 levels per tile.
    angle = math.degrees(math.atan2(-(y - CY), x - CX)) % 360  # 0 = east, 90 = north
    cliff = 20 <= angle <= 120 or angle <= 10 or angle >= 340
    drop = steps * (2 if cliff else 1)
    return max(0, TOP - drop)


def build() -> Level:
    L = Level(N, N)
    for y in range(N):
        for x in range(N):
            if math.hypot(x - CX, y - CY) > ISLAND:
                L.set(x, y, "#")
            L.heights[y][x] = height(x, y) if L.tiles[y][x] != "#" else 0

    # Sandbags around the plateau rim, with gaps.
    for a in range(0, 360, 20):
        if a % 60 == 0:
            continue
        x = round(CX + math.cos(math.radians(a)) * 3.0)
        y = round(CY - math.sin(math.radians(a)) * 3.0)
        if L.heights[y][x] == TOP:
            L.set(x, y, "h")
    # Boulders and dead trees on the slopes and the flats below (some cover,
    # some obstacles the horde has to go around).
    rocks = [(14, 25), (25, 26), (12, 19), (17, 29), (24, 31), (9, 24), (29, 24), (13, 13),
             (27, 14), (31, 19), (8, 15), (20, 33), (32, 28), (6, 20), (15, 34), (26, 8)]
    for x, y in rocks:
        if L.tiles[y][x] == "." and L.heights[y][x] < TOP:
            L.set(x, y, "H")
    scrub = [(16, 23), (23, 24), (18, 26), (11, 22), (28, 21), (22, 28), (10, 27), (30, 30), (7, 26)]
    for x, y in scrub:
        if L.tiles[y][x] == ".":
            L.set(x, y, "h")
    return L


def spawn_points(L: Level) -> list[tuple[int, int]]:
    """Walkable tiles near the island's edge, off the hill."""
    out = []
    for a in range(0, 360, 15):
        for r in (17.5, 16.5):
            x = round(CX + math.cos(math.radians(a)) * r)
            y = round(CY - math.sin(math.radians(a)) * r)
            if L.walkable(x, y) and max(abs(x - CX), abs(y - CY)) > ON_SCREEN and (x, y) not in out:
                out.append((x, y))
                break
    return out


def populate(L: Level):
    L.deploys.append(dict(team="player", squad="player", x=CX, y=CY))
    # The first wave, already climbing.
    first = [(13, 27), (26, 28), (11, 21), (29, 22), (20, 30), (15, 31), (24, 32), (9, 25),
             (31, 26), (18, 33), (12, 15)]
    for i, (x, y) in enumerate(first):
        L.unit("enemy", "zombie", f"Shambler {i + 1}", x, y)

    pts = spawn_points(L)
    pts_rhai = "[" + ", ".join(f"[{x}, {y}]" for x, y in pts) + "]"
    lo, hi = CX - ON_SCREEN, CY - ON_SCREEN
    L.zone("evac", CX - 1, CY - 1, 3, 3, """<script on="enter" team="player">
      if flag("evac") == 1 {
        let squad = alive_count("player");
        let aboard = zone_count("evac", "player");
        if squad > 0 && aboard >= squad { message("Everyone's aboard - the chopper lifts off the hill. Mission complete!"); win(); }
        else { message(aboard + " of " + squad + " aboard - everyone still alive has to make it."); }
      }
    </script>""", marker="evac", label=f"EVAC (TURN {TURNS})")

    L.library = f"""
    // Keeps 10-12 zombies on the hill; more gather at the shore, up to 30.
    fn horde_tide() {{
      let alive = alive_count("enemy");
      let on_hill = count_in_area({lo}, {hi}, {2 * ON_SCREEN + 1}, {2 * ON_SCREEN + 1}, "enemy");
      let want = rand(10, 12);
      let n = want - on_hill;
      // The shore keeps filling up even when the hill is full.
      if n < 2 {{ n = 2; }}
      if n > 30 - alive {{ n = 30 - alive; }}
      if n > 6 {{ n = 6; }}
      let pts = {pts_rhai};
      for i in 0..n {{
        let p = pts[rand(0, pts.len() - 1)];
        spawn_unit("enemy", "zombie", "Shambler", p[0], p[1]);
      }}
      if n > 0 && on_hill < 10 {{ message("More of them are crawling out of the treeline..."); }}
    }}"""
    L.script("start", f"""
    set_flag("evac", 0);
    message("Hold the hilltop. The chopper is {TURNS} turns out.");
    message("They can't climb the cliffs to the north and east - watch the south and west slopes.");
    objective("Survive until the evac chopper arrives (turn {TURNS})");""")
    L.script("turn_start", "horde_tide();", team="enemy")
    L.script("turn_start", f"""
    let left = {TURNS} - round;
    if left > 0 {{
      objective("Survive until the evac chopper arrives: " + left + (if left == 1 {{ " turn" }} else {{ " turns" }}));
      if left <= 3 {{ message("Evac in " + left + (if left == 1 {{ " turn!" }} else {{ " turns!" }})); }}
    }} else {{
      if flag("evac") == 0 {{
        set_flag("evac", 1);
        message("The chopper is down on the pad! Everyone aboard!");
        zone_label("evac", "EVAC - BOARD NOW");
        objective("Get everyone still alive onto the evac pad");
      }}
      let squad = alive_count("player");
      if squad > 0 && zone_count("evac", "player") >= squad {{
        message("Everyone's aboard - the chopper lifts off the hill. Mission complete!");
        win();
      }}
    }}""", team="player")


def surfaces(L: Level) -> list[str]:
    """Looks: grass slopes, a dirt plateau with a concrete pad, boulders and
    dead trees, sandbags on the rim, scrub on the slopes."""
    rows = []
    for y in range(L.h):
        row = ""
        for x in range(L.w):
            t, h = L.tiles[y][x], L.heights[y][x]
            if t == "#":
                row += "."
            elif t == "H":
                row += "T" if (x * 7 + y * 3) % 3 == 0 else "R"
            elif t == "h":
                row += "S" if h == TOP else "G"
            elif abs(x - CX) <= 1 and abs(y - CY) <= 1:
                row += "c"
            elif h == TOP:
                row += "d"
            elif h == 0 and (x * 13 + y * 7) % 5 == 0:
                row += "d"
            else:
                row += "g"
        rows.append(row)
    return rows


def main():
    L = build()
    populate(L)
    L.check("hill")
    text = L.xml(
        "  Hold the Hill: the squad holds a hilltop while zombies crawl up its\n"
        "  slopes, for 12 turns until the evac chopper lands. Cliffs on the north\n"
        "  and east faces, ramps to the south and west.",
        'id="hill" name="Hold the Hill" seed="51214" fog="false" win="script"',
        "The squad is cut off on a hilltop at dusk with the dead crawling up the slopes. Hold for 12 turns until the evac "
        "chopper lands on the pad, then get everyone aboard. The north and east faces are sheer cliffs the zombies can't "
        "climb; they come up the south and west slopes, 10-12 at a time, and more keep gathering below.",
    )
    text = text.replace("Generated by tools/gen_missions.py", "Generated by tools/gen_hill.py")
    text = text.replace("  </heights>", "  </heights>\n  <surfaces>\n" + "\n".join("    " + r for r in surfaces(L)) + "\n  </surfaces>", 1)
    text = text.replace("  <layout>", '  <sky top="#1d2238" horizon="#c46a3c"/>\n\n  <layout>', 1)
    write("hill", text)


if __name__ == "__main__":
    main()
