#!/usr/bin/env python3
"""Generates "Hold the Hill" (assets/missions/hill.mission.xml) and its
extreme version "Hold the Hill: Extreme" (hill_extreme.mission.xml: a bigger,
taller hill and 2.5x the zombies).

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
import random
import sys
from dataclasses import dataclass, field

sys.path.insert(0, os.path.dirname(__file__))
from gen_missions import Level, write  # noqa: E402

@dataclass
class Hill:
    id: str
    name: str
    seed: int
    N: int = 40
    TOP: int = 7            # plateau level
    PLATEAU: float = 3.2    # plateau radius
    ISLAND: float = 19.2    # beyond: water / void
    ON_SCREEN: int = 12     # "on the hill": within this many tiles of the top
    TURNS: int = 12         # evac arrives at the start of this round
    ON_HILL: tuple = (10, 12)  # zombies kept on the hill
    CAP: int = 30           # most alive at once
    PER_TURN: int = 6       # most spawned per turn
    SCALE: float = 1.0      # layout scale of the hand-placed rocks / first wave
    EXTRA_ROCKS: int = 0
    FIRST_WAVE: int = 11
    blurb: str = ""

    @property
    def CX(self):
        return self.N // 2

    @property
    def CY(self):
        return self.N // 2


NORMAL = Hill("hill", "Hold the Hill", 51214)
EXTREME = Hill(
    "hill_extreme", "Hold the Hill: Extreme", 51215,
    N=64, TOP=9, PLATEAU=4.5, ISLAND=31.0, ON_SCREEN=18,
    ON_HILL=(25, 30), CAP=75, PER_TURN=15, SCALE=1.55, EXTRA_ROCKS=22, FIRST_WAVE=28,
    blurb="EXTREME: a bigger, taller hill and two and a half times the dead. ",
)

# Hand-placed features, as offsets from the hilltop (scaled for bigger hills).
ROCKS = [(-6, 5), (5, 6), (-8, -1), (-3, 9), (4, 11), (-11, 4), (9, 4), (-7, -7),
         (7, -6), (11, -1), (-12, -5), (0, 13), (12, 8), (-14, 0), (-5, 14), (6, -12)]
SCRUB = [(-4, 3), (3, 4), (-2, 6), (-9, 2), (8, 1), (2, 8), (-10, 7), (10, 10), (-13, 6)]
WAVE = [(-7, 7), (6, 8), (-9, 1), (9, 2), (0, 10), (-5, 11), (4, 12), (-11, 5),
        (11, 6), (-2, 13), (-8, -5)]


def height(c: Hill, x: int, y: int) -> int:
    d = math.hypot(x - c.CX, y - c.CY)
    if d <= c.PLATEAU:
        return c.TOP
    steps = math.ceil(d - c.PLATEAU)
    # Cliffs on the north and east faces: 2 levels per tile.
    angle = math.degrees(math.atan2(-(y - c.CY), x - c.CX)) % 360  # 0 = east, 90 = north
    cliff = 20 <= angle <= 120 or angle <= 10 or angle >= 340
    drop = steps * (2 if cliff else 1)
    return max(0, c.TOP - drop)


def place(c: Hill, offsets):
    return [(c.CX + round(dx * c.SCALE), c.CY + round(dy * c.SCALE)) for dx, dy in offsets]


def build(c: Hill) -> Level:
    N, CX, CY, TOP = c.N, c.CX, c.CY, c.TOP
    L = Level(N, N)
    for y in range(N):
        for x in range(N):
            if math.hypot(x - CX, y - CY) > c.ISLAND:
                L.set(x, y, "#")
            L.heights[y][x] = height(c, x, y) if L.tiles[y][x] != "#" else 0

    # Sandbags around the plateau rim, with gaps.
    for a in range(0, 360, 20):
        if a % 60 == 0:
            continue
        x = round(CX + math.cos(math.radians(a)) * (c.PLATEAU - 0.2))
        y = round(CY - math.sin(math.radians(a)) * (c.PLATEAU - 0.2))
        if L.heights[y][x] == TOP:
            L.set(x, y, "h")
    # Boulders and dead trees on the slopes and the flats below (some cover,
    # some obstacles the horde has to go around).
    rocks = place(c, ROCKS)
    rng = random.Random(c.seed)
    while len(rocks) < len(ROCKS) + c.EXTRA_ROCKS:
        a, r = rng.uniform(0, 360), rng.uniform(c.PLATEAU + 2, c.ISLAND - 2)
        rocks.append((round(CX + math.cos(math.radians(a)) * r), round(CY - math.sin(math.radians(a)) * r)))
    for x, y in rocks:
        if L.tiles[y][x] == "." and L.heights[y][x] < TOP:
            L.set(x, y, "H")
    for x, y in place(c, SCRUB):
        if L.tiles[y][x] == ".":
            L.set(x, y, "h")
    return L


def spawn_points(c: Hill, L: Level) -> list[tuple[int, int]]:
    """Walkable tiles near the island's edge, off the hill."""
    out = []
    for a in range(0, 360, 15 if c.N < 50 else 8):
        for r in (c.ISLAND - 1.7, c.ISLAND - 2.7):
            x = round(c.CX + math.cos(math.radians(a)) * r)
            y = round(c.CY - math.sin(math.radians(a)) * r)
            if L.walkable(x, y) and max(abs(x - c.CX), abs(y - c.CY)) > c.ON_SCREEN and (x, y) not in out:
                out.append((x, y))
                break
    return out


def populate(c: Hill, L: Level):
    CX, CY, ON_SCREEN, TURNS = c.CX, c.CY, c.ON_SCREEN, c.TURNS
    L.deploys.append(dict(team="player", squad="player", x=CX, y=CY))
    # The first wave, already climbing (the south and west slopes and flats).
    first = [p for p in place(c, WAVE) if L.walkable(*p)]
    rng = random.Random(c.seed + 1)
    tries = 0
    while len(first) < c.FIRST_WAVE and tries < 5000:
        tries += 1
        a, r = rng.uniform(150, 330), rng.uniform(c.PLATEAU + 3, c.ISLAND - 3)
        p = (round(CX + math.cos(math.radians(a)) * r), round(CY - math.sin(math.radians(a)) * r))
        if L.walkable(*p) and p not in first:
            first.append(p)
    for i, (x, y) in enumerate(first[: c.FIRST_WAVE]):
        L.unit("enemy", "zombie", f"Shambler {i + 1}", x, y)

    pts = spawn_points(c, L)
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
    // Keeps {c.ON_HILL[0]}-{c.ON_HILL[1]} zombies on the hill; more gather at the shore, up to {c.CAP}.
    fn horde_tide() {{
      let alive = alive_count("enemy");
      let on_hill = count_in_area({lo}, {hi}, {2 * ON_SCREEN + 1}, {2 * ON_SCREEN + 1}, "enemy");
      let want = rand({c.ON_HILL[0]}, {c.ON_HILL[1]});
      let n = want - on_hill;
      // The shore keeps filling up even when the hill is full.
      if n < {max(2, c.PER_TURN // 3)} {{ n = {max(2, c.PER_TURN // 3)}; }}
      if n > {c.CAP} - alive {{ n = {c.CAP} - alive; }}
      if n > {c.PER_TURN} {{ n = {c.PER_TURN}; }}
      let pts = {pts_rhai};
      for i in 0..n {{
        let p = pts[rand(0, pts.len() - 1)];
        spawn_unit("enemy", "zombie", "Shambler", p[0], p[1]);
      }}
      if n > 0 && on_hill < {c.ON_HILL[0]} {{ message("More of them are crawling out of the treeline..."); }}
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


def surfaces(c: Hill, L: Level) -> list[str]:
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
                row += "S" if h == c.TOP else "G"
            elif abs(x - c.CX) <= 1 and abs(y - c.CY) <= 1:
                row += "c"
            elif h == c.TOP:
                row += "d"
            elif h == 0 and (x * 13 + y * 7) % 5 == 0:
                row += "d"
            else:
                row += "g"
        rows.append(row)
    return rows


def generate(c: Hill):
    L = build(c)
    populate(c, L)
    L.check(c.id)
    text = L.xml(
        f"  {c.name}: the squad holds a hilltop while zombies crawl up its\n"
        f"  slopes, for {c.TURNS} turns until the evac chopper lands. Cliffs on the north\n"
        "  and east faces, ramps to the south and west.",
        f'id="{c.id}" name="{c.name}" seed="{c.seed}" fog="false" win="script"',
        f"{c.blurb}The squad is cut off on a hilltop at dusk with the dead crawling up the slopes. Hold for {c.TURNS} turns until the evac "
        "chopper lands on the pad, then get everyone aboard. The north and east faces are sheer cliffs the zombies can't "
        f"climb; they come up the south and west slopes, {c.ON_HILL[0]}-{c.ON_HILL[1]} at a time, and more keep gathering below (up to {c.CAP}).",
    )
    text = text.replace("Generated by tools/gen_missions.py", "Generated by tools/gen_hill.py")
    text = text.replace("  </heights>", "  </heights>\n  <surfaces>\n" + "\n".join("    " + r for r in surfaces(c, L)) + "\n  </surfaces>", 1)
    text = text.replace("  <layout>", '  <sky top="#1d2238" horizon="#c46a3c"/>\n\n  <layout>', 1)
    write(c.id, text)


def main():
    generate(NORMAL)
    generate(EXTREME)


if __name__ == "__main__":
    main()
