#!/usr/bin/env python3
"""Generates the large bundled test missions.

    python3 tools/gen_missions.py

Writes `crates/tactics_game/assets/missions/<id>.mission.xml` for:

* proving_grounds  - a 40x32 sandbox with every mechanic (heights, ladders,
                     cut-outs, props, doors, dormant pods, fog)
* holdout          - "Hold the Beacon": waves, then extract the whole squad
* complex          - "Clear the Complex": rooms, doors, dormant pods, fog
* infiltration     - "Silent Theft": concealment, patrols, a vault, extraction

The layouts are drawn with small helpers (rooms, walls, doors) so they stay
consistent; the script checks that every unit, prop, zone and patrol point
lands on a usable tile. The generated XML is ordinary mission XML: edit it by
hand afterwards if you prefer (re-running this script overwrites it).

Coordinates are mission coordinates: x = column, y = row from the TOP.
"""

from __future__ import annotations

import os
import sys

OUT = os.path.join(os.path.dirname(__file__), "..", "crates", "tactics_game", "assets", "missions")


class Level:
    def __init__(self, w: int, h: int):
        self.w, self.h = w, h
        self.tiles = [["." for _ in range(w)] for _ in range(h)]
        self.heights = [[0 for _ in range(w)] for _ in range(h)]
        self.ladders: list[tuple[int, int]] = []
        self.doors: list[dict] = []
        self.props: list[dict] = []
        self.units: list[dict] = []
        self.zones: list[dict] = []
        self.tile_scripts: list[dict] = []
        self.deploys: list[dict] = []
        self.scripts: list[tuple[str, str, dict]] = []
        self.library = ""

    # --- drawing -----------------------------------------------------------
    def set(self, x, y, c):
        if 0 <= x < self.w and 0 <= y < self.h:
            self.tiles[y][x] = c

    def fill(self, x0, y0, x1, y1, c):
        for y in range(y0, y1 + 1):
            for x in range(x0, x1 + 1):
                self.set(x, y, c)

    def room(self, x0, y0, x1, y1, c="H"):
        """Wall outline of a rectangle (inclusive corners), floor inside."""
        self.fill(x0, y0, x1, y1, ".")
        for x in range(x0, x1 + 1):
            self.set(x, y0, c)
            self.set(x, y1, c)
        for y in range(y0, y1 + 1):
            self.set(x0, y, c)
            self.set(x1, y, c)

    def raise_area(self, x0, y0, x1, y1, level):
        for y in range(y0, y1 + 1):
            for x in range(x0, x1 + 1):
                self.heights[y][x] = level

    def door(self, tag, x, y, w=1, h=1, locked=False, scripts=""):
        for yy in range(y, y + h):
            for xx in range(x, x + w):
                self.set(xx, yy, "H")
        self.doors.append(dict(tag=tag, x=x, y=y, w=w, h=h, locked=locked, scripts=scripts))

    def prop(self, tag, kind, x, y, cover="half", hp=None, scripts=""):
        self.props.append(dict(tag=tag, kind=kind, x=x, y=y, cover=cover, hp=hp, scripts=scripts))

    def unit(self, team, cls, name, x, y, **kw):
        self.units.append(dict(team=team, cls=cls, name=name, x=x, y=y, **kw))

    def zone(self, tag, x, y, w, h, scripts, marker=None, label=None):
        self.zones.append(dict(tag=tag, x=x, y=y, w=w, h=h, scripts=scripts, marker=marker, label=label))

    def tile_script(self, x, y, scripts, marker=None, label=None, tag=None):
        self.tile_scripts.append(dict(x=x, y=y, scripts=scripts, marker=marker, label=label, tag=tag))

    def script(self, on, code, **attrs):
        self.scripts.append((on, code, attrs))

    # --- checks ------------------------------------------------------------
    def walkable(self, x, y):
        return 0 <= x < self.w and 0 <= y < self.h and self.tiles[y][x] == "."

    def blocked(self):
        """Tiles taken by doors and props."""
        out = set()
        for d in self.doors:
            for yy in range(d["y"], d["y"] + d["h"]):
                for xx in range(d["x"], d["x"] + d["w"]):
                    out.add((xx, yy))
        for p in self.props:
            out.add((p["x"], p["y"]))
        return out

    def check(self, name):
        problems = []
        taken = self.blocked()
        for p in self.props:
            if self.tiles[p["y"]][p["x"]] != ".":
                problems.append(f"prop {p['tag']} on '{self.tiles[p['y']][p['x']]}' at {p['x']},{p['y']}")
        seen = set()
        for u in self.units:
            pos = (u["x"], u["y"])
            if not self.walkable(*pos) or pos in taken or pos in seen:
                problems.append(f"unit {u['name']} on a bad tile {pos}")
            seen.add(pos)
            for wp in u.get("patrol", []):
                if not self.walkable(*wp) or wp in taken:
                    problems.append(f"patrol point {wp} of {u['name']} is blocked")
        for z in self.tile_scripts:
            if not self.walkable(z["x"], z["y"]) or (z["x"], z["y"]) in taken:
                problems.append(f"tile script at {z['x']},{z['y']} is not walkable")
        for d in self.deploys:
            if not self.walkable(d["x"], d["y"]):
                problems.append(f"deploy at {d['x']},{d['y']} is not walkable")
        for x, y in self.ladders:
            if not self.walkable(x, y):
                problems.append(f"ladder {x},{y} not walkable")
        if problems:
            sys.exit(f"{name}: " + "; ".join(problems))

    # --- output ------------------------------------------------------------
    def xml(self, header: str, attrs: str, description: str) -> str:
        o = ['<?xml version="1.0" encoding="utf-8"?>', "<!--", header.strip("\n"), "", "  Generated by tools/gen_missions.py (edit freely; re-running overwrites).", "  Coordinates: x = column from the left, y = ROW FROM THE TOP of <layout>.", "  Layout: . floor   # cut out   h half cover   H full cover", "-->"]
        o.append(f"<mission {attrs}>")
        o.append(f"  <description>{description}</description>")
        o.append("")
        o.append("  <layout>")
        o += ["    " + "".join(r) for r in self.tiles]
        o.append("  </layout>")
        if any(any(r) for r in self.heights):
            o.append("  <heights>")
            o += ["    " + "".join(str(v) for v in r) for r in self.heights]
            o.append("  </heights>")
        for x, y in self.ladders:
            o.append(f'  <ladder x="{x}" y="{y}"/>')
        o.append("")
        for d in self.deploys:
            o.append(f'  <deploy team="{d["team"]}" squad="{d["squad"]}" x="{d["x"]}" y="{d["y"]}"/>')
        o.append("")
        for d in self.doors:
            lock = ' locked="true"' if d["locked"] else ""
            body = d["scripts"].strip()
            tag = f'  <door tag="{d["tag"]}" x="{d["x"]}" y="{d["y"]}" width="{d["w"]}" height="{d["h"]}"{lock}'
            o.append(tag + (">\n    " + body + "\n  </door>" if body else "/>"))
        o.append("")
        for p in self.props:
            hp = f' hp="{p["hp"]}"' if p["hp"] is not None else ""
            body = p["scripts"].strip()
            tag = f'  <prop tag="{p["tag"]}" kind="{p["kind"]}" x="{p["x"]}" y="{p["y"]}" cover="{p["cover"]}"{hp}'
            o.append(tag + (">\n    " + body + "\n  </prop>" if body else "/>"))
        o.append("")
        for u in self.units:
            extra = ""
            if u.get("tag"):
                extra += f' tag="{u["tag"]}"'
            if u.get("dormant"):
                extra += ' dormant="true"'
            if u.get("pod"):
                extra += f' pod="{u["pod"]}"'
            if u.get("patrol"):
                extra += ' patrol="' + " ".join(f"{x},{y}" for x, y in u["patrol"]) + '"'
            body = u.get("scripts", "").strip()
            tag = f'  <unit team="{u["team"]}" class="{u["cls"]}" name="{u["name"]}" x="{u["x"]}" y="{u["y"]}"{extra}'
            o.append(tag + (">\n    " + body + "\n  </unit>" if body else "/>"))
        o.append("")
        def mark(d):
            out = ""
            if d.get("marker"):
                out += f' marker="{d["marker"]}"'
            if d.get("label"):
                out += f' label="{d["label"]}"'
            return out
        for z in self.zones:
            o.append(f'  <zone tag="{z["tag"]}" x="{z["x"]}" y="{z["y"]}" width="{z["w"]}" height="{z["h"]}"{mark(z)}>')
            o.append("    " + z["scripts"].strip())
            o.append("  </zone>")
        for t in self.tile_scripts:
            tag = f' tag="{t["tag"]}"' if t.get("tag") else ""
            o.append(f'  <tile{tag} x="{t["x"]}" y="{t["y"]}"{mark(t)}>')
            o.append("    " + t["scripts"].strip())
            o.append("  </tile>")
        o.append("")
        if self.library.strip():
            o.append("  <script>" + self.library.rstrip() + "\n  </script>")
        for on, code, attrs in self.scripts:
            a = "".join(f' {k}="{v}"' for k, v in attrs.items())
            o.append(f'  <script on="{on}"{a}>' + code.rstrip() + "\n  </script>")
        o.append("</mission>")
        return "\n".join(o) + "\n"


def write(name, text):
    path = os.path.join(OUT, f"{name}.mission.xml")
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    print("wrote", os.path.normpath(path))


# =============================================================================
# Proving Grounds: everything in one sandbox
# =============================================================================
def proving_grounds():
    L = Level(40, 32)
    # A chasm splits the map; two bridges cross it.
    L.fill(18, 0, 20, 31, "#")
    L.fill(18, 4, 20, 5, ".")
    L.fill(18, 15, 20, 17, ".")
    # West: a terraced hill (levels 1-2, each step climbable) ...
    L.raise_area(2, 2, 11, 9, 1)
    L.raise_area(4, 3, 8, 7, 2)
    L.prop("hill_crate", "crate", 6, 5, "half", 4)
    # ... and a watchtower (level 4) that needs its ladder.
    L.raise_area(12, 22, 13, 23, 4)
    L.ladders.append((12, 24))
    # Low walls and scattered cover on the west side.
    for x in range(3, 10):
        L.set(x, 13, "h")
    L.set(6, 13, ".")
    for (x, y) in [(14, 10), (15, 10), (3, 19), (4, 19), (9, 24), (9, 25), (15, 28), (2, 27)]:
        L.set(x, y, "h")
    L.fill(14, 16, 15, 17, "H")
    L.prop("west_barrel", "barrel", 11, 15, "half", 2, '<script on="destroyed">explode(x, y, 1.6, 4); ignite(x, y, 1.6, 2);</script>')
    # East: a walled compound with a door, barrels and a console.
    L.room(26, 9, 36, 20)
    L.door("compound_west", 26, 14, 1, 2)
    L.door("compound_south", 31, 20, 2, 1)
    for (x, y) in [(29, 12), (33, 12), (29, 17), (33, 17)]:
        L.set(x, y, "h")
    L.prop("compound_barrel", "barrel", 31, 14, "half", 2,
           '<script on="destroyed">message("The fuel barrel explodes!"); explode(x, y, 1.8, 4); ignite(x, y, 1.8, 2);</script>')
    L.prop("compound_console", "console", 35, 10, "half")
    L.tile_script(34, 11, '<script on="enter" team="player" once="true">message("Console: the east wing pods are now awake!"); wake_all(); show_zone("pg_console", false);</script>',
                  marker="danger", label="ALARM CONSOLE", tag="pg_console")
    # North-east: raised platform (level 1) with half walls.
    L.raise_area(24, 1, 37, 5, 1)
    for x in range(25, 37, 3):
        L.set(x, 6, "h")
    # South-east: crates maze.
    for (x, y) in [(24, 24), (25, 24), (28, 26), (29, 26), (32, 24), (35, 27), (36, 27), (27, 29), (31, 29)]:
        L.prop(f"crate_{x}_{y}", "crate", x, y, "half", 4)
    L.deploys.append(dict(team="player", squad="player", x=4, y=27))
    # Dormant pods: they hold position until seen (or hurt).
    L.unit("enemy", "sectoid", "Hill Sentry", 5, 4, dormant=True, pod="hill")
    L.unit("enemy", "sectoid", "Hill Sentry 2", 8, 6, dormant=True, pod="hill")
    L.unit("enemy", "sectoid", "Compound Guard", 30, 15, dormant=True, pod="compound")
    L.unit("enemy", "sectoid", "Compound Guard 2", 33, 16, dormant=True, pod="compound")
    L.unit("enemy", "drone", "Compound Drone", 34, 13, dormant=True, pod="compound")
    L.unit("enemy", "sectoid", "Platform Watch", 30, 3, dormant=True, pod="platform")
    L.unit("enemy", "drone", "Platform Drone", 34, 2, dormant=True, pod="platform")
    L.unit("enemy", "sectoid", "Crate Lurker", 30, 27, dormant=True, pod="crates")
    L.unit("enemy", "sectoid", "Crate Lurker 2", 34, 29, dormant=True, pod="crates")
    L.unit("enemy", "sectoid", "Bridge Patrol", 22, 16, patrol=[(22, 16), (22, 4), (30, 7)])
    L.script("start", """
    message("Proving Grounds: heights, ladders, a chasm, doors and four dormant pods. Clear them all.");
    objective("Eliminate all hostiles");""")
    L.script("unit_killed", """
    let left = alive_count("enemy");
    if unit_team(unit) == "enemy" && left > 0 { objective("Eliminate all hostiles (" + left + " left)"); }""")
    L.check("proving_grounds")
    write("proving_grounds", L.xml(
        "  Proving Grounds: a large sandbox that uses every mission mechanic.",
        'id="proving_grounds" name="Proving Grounds" seed="40032" fog="true"',
        "A 40x32 test range: terraced hill, ladder tower, chasm bridges, a walled compound with doors and four dormant pods under fog of war."))


# =============================================================================
# Holdout: defend the beacon, then extract
# =============================================================================
def holdout():
    W, H = 36, 30
    L = Level(W, H)
    bx, by = 18, 15  # beacon
    # A ring of barricades around the beacon with four gaps.
    for x in range(13, 24):
        for y in (10, 20):
            if x not in (17, 18, 19):
                L.set(x, y, "h")
    for y in range(11, 20):
        for x in (12, 24):
            if y not in (14, 15, 16):
                L.set(x, y, "h")
    # Inner sandbags.
    for (x, y) in [(15, 13), (21, 13), (15, 17), (21, 17)]:
        L.set(x, y, "h")
    L.prop("beacon", "beacon", bx, by, "half")
    # Outer ruins: walls and pillars for the attackers.
    L.fill(4, 4, 7, 5, "H")
    L.fill(28, 4, 31, 5, "H")
    L.fill(4, 24, 7, 25, "H")
    L.fill(28, 23, 31, 24, "H")
    for (x, y) in [(10, 3), (25, 3), (10, 26), (25, 26), (2, 14), (33, 14), (17, 3), (18, 26)]:
        L.set(x, y, "h")
    # Raised gun platforms inside the ring (level 1).
    L.raise_area(14, 11, 16, 12, 1)
    L.raise_area(20, 18, 22, 19, 1)
    # A collapsed section: cut-out holes.
    L.fill(5, 15, 6, 17, "#")
    L.fill(29, 10, 30, 12, "#")
    # Evac pad (north-west corner), opened after turn 6.
    L.raise_area(1, 0, 5, 2, 0)
    for x in range(0, 7):
        L.set(x, 3, "h") if x in (0, 6) else None
    L.zone("evac", 1, 0, 5, 3, """<script on="enter" team="player">
      if flag("evac_open") == 1 {
        let inside = count_in_area(1, 0, 5, 3, "player");
        let squad = alive_count("player");
        if inside == squad { message("Everyone aboard. Dust off!"); win(); }
        else { message(unit_name(unit) + " is aboard: " + inside + " of " + squad + " soldiers at the pad."); }
      }
    </script>""", marker="evac", label="EVAC PAD (opens turn 7)")
    L.deploys.append(dict(team="player", squad="player", x=18, y=17))
    # Beacon defenders join the squad.
    L.unit("player", "grenadier", "Beacon Guard Reyes", 17, 13)
    L.unit("player", "rifleman", "Beacon Guard Lindqvist", 20, 16)
    L.unit("enemy", "sectoid", "Scout 1", 4, 2)
    L.unit("enemy", "sectoid", "Scout 2", 32, 27)
    L.unit("enemy", "drone", "Scout Drone", 32, 2)
    L.library = """
    // Script functions can't see the trigger variables: pass `round` in.
    fn status(round) {
      let left = 7 - round;
      if flag("evac_open") == 1 { "Get the WHOLE squad to the evac pad (north-west)" }
      else if left > 0 { "Defend the beacon - integrity " + flag("integrity") + " - evac in " + left + " turns" }
      else { "Defend the beacon - integrity " + flag("integrity") }
    }"""
    L.script("start", """
    set_flag("integrity", 12);
    ai_goal("enemy", 18, 15);
    ai_goal("player", 18, 16);
    message("Hold the beacon until the evac craft arrives (6 turns). Aliens next to it damage it.");
    objective(status(round));""")
    # Integrity: every enemy within 2 tiles of the beacon at the start of its turn damages it.
    L.script("turn_start", """
    let n = count_in_area(16, 13, 5, 5, "enemy");
    if n > 0 && flag("evac_open") == 0 {
      set_flag("integrity", flag("integrity") - n * 2);
      message("The beacon is under attack! Integrity " + flag("integrity"));
      if flag("integrity") <= 0 { message("The beacon is destroyed."); lose(); }
    }
    // Waves: [round, class, name, x, y]
    let waves = [
      [2, "sectoid", "Raider", 18, 0], [2, "sectoid", "Raider", 20, 0],
      [3, "sectoid", "Flanker", 35, 15], [3, "drone", "Drone", 35, 13],
      [4, "sectoid", "Raider", 0, 20], [4, "sectoid", "Raider", 0, 22],
      [5, "sectoid", "Assault", 18, 29], [5, "drone", "Drone", 20, 29],
      [6, "sectoid", "Elite", 35, 2], [6, "sectoid", "Elite", 10, 29]
    ];
    for w in waves {
      if w[0] == round { spawn_unit("enemy", w[1], w[2], w[3], w[4]); }
    }
    if round >= 2 && round <= 6 { message("Wave " + (round - 1) + " incoming!"); }
    // After the evac opens the aliens chase the squad, a few at a time.
    if flag("evac_open") == 1 && round % 3 == 0 { spawn_unit("enemy", "sectoid", "Hunter", 35, 0); }""", team="enemy")
    L.script("turn_start", """
    if round == 7 && flag("evac_open") == 0 {
      set_flag("evac_open", 1);
      message("The evac craft is down at the north-west pad! Get everyone aboard.");
      zone_label("evac", "EVAC - ALL ABOARD!");
      ai_goal("player", 3, 1);
      ai_goal("enemy", 3, 1);
    }
    objective(status(round));""", team="player")
    L.check("holdout")
    write("holdout", L.xml(
        "  Hold the Beacon: defend a point against waves, then extract the whole squad.",
        'id="holdout" name="Hold the Beacon" seed="36030" win="script"',
        "Defend the uplink beacon for six turns against waves of aliens, then get the entire squad to the evac pad."))


# =============================================================================
# Complex: explore and clear a facility
# =============================================================================
def complex_():
    W, H = 44, 36
    L = Level(W, H)
    # Outer facility wall; the yard to the south stays open.
    L.room(2, 2, 41, 25)
    # Main corridor (east-west) at rows 12-14, spine corridor north-south at 20-22.
    for x in range(3, 41):
        L.set(x, 11, "H")
        L.set(x, 15, "H")
    for y in range(3, 25):
        if not 11 <= y <= 15:
            L.set(19, y, "H")
            L.set(23, y, "H")
    # The spine crosses the corridor.
    for x in range(20, 23):
        L.set(x, 11, ".")
        L.set(x, 15, ".")
    # Rooms north of the corridor: walls between them.
    for x in (10, 31):
        for y in range(3, 11):
            L.set(x, y, "H")
    # Rooms south of the corridor.
    for x in (10, 31):
        for y in range(16, 25):
            L.set(x, y, "H")
    # Front entrance: double door from the yard into the spine.
    L.door("entrance", 20, 25, 3, 1)
    # Side entrance into the south-west room.
    L.door("side_entrance", 6, 25, 1, 1)
    # Doors from the corridor/spine into each room.
    L.door("armory", 6, 11)          # NW room (armory)
    L.door("barracks", 14, 11)       # N-middle-left
    L.door("labs", 27, 11)           # N-middle-right
    L.door("security", 36, 11)       # NE room (security)
    L.door("storage", 6, 15)         # SW room
    L.door("mess", 14, 15)           # S-middle-left
    L.door("generator", 27, 15)      # S-middle-right
    L.door("core", 36, 15, 1, 1, locked=True,
           scripts='<script on="opened">message("The core door slides open. Something big is inside.");</script>')  # SE room (core)
    # Spine to corridor openings already exist (walls skip rows 11-15).
    # Interior details.
    for (x, y) in [(4, 5), (5, 5), (4, 8), (8, 4), (13, 6), (16, 6), (13, 8), (16, 8), (26, 5), (28, 5), (34, 4), (38, 8), (5, 19), (7, 21), (14, 19), (16, 22), (26, 19), (28, 22), (35, 18), (38, 22)]:
        L.set(x, y, "h")
    # Labs: a mezzanine (level 1) with a level-2 gantry reached by ladder.
    L.raise_area(24, 3, 30, 6, 1)
    L.raise_area(24, 3, 26, 4, 3)
    L.ladders.append((27, 4))
    L.set(26, 5, ".")
    L.set(28, 5, ".")
    # Props.
    L.prop("armory_rack", "crate", 8, 8, "half", 4)
    L.prop("gen_core", "barrel", 25, 20, "half", 3,
           '<script on="destroyed">message("The generator blows! The lights flicker."); explode(x, y, 2.0, 5); ignite(x, y, 2.0, 2);</script>')
    L.prop("gen_core_2", "barrel", 29, 20, "half", 3,
           '<script on="destroyed">explode(x, y, 2.0, 5); ignite(x, y, 2.0, 2);</script>')
    L.prop("sec_console", "console", 39, 4, "half")
    L.prop("mess_table", "crate", 12, 20, "half", 3)
    L.prop("mess_table_2", "crate", 17, 20, "half", 3)
    L.tile_script(38, 5, """<script on="enter" team="player" once="true">
      message("Security console: core door unlocked.");
      lock_door("core", false);
      open_door("core");
      objective("Clear the complex - the core is open (" + alive_count("enemy") + " hostiles left)");
      show_zone("core_console", false);
    </script>""", marker="objective", label="CORE UNLOCK", tag="core_console")
    # Yard cover.
    for (x, y) in [(8, 29), (9, 29), (15, 28), (27, 28), (33, 30), (34, 30), (20, 32), (21, 32), (12, 32), (30, 33)]:
        L.set(x, y, "h")
    L.deploys.append(dict(team="player", squad="player", x=21, y=31))
    # Pods, one per room (dormant until seen).
    L.unit("enemy", "sectoid", "Armory Guard", 5, 7, dormant=True, pod="armory")
    L.unit("enemy", "sectoid", "Armory Guard 2", 8, 5, dormant=True, pod="armory")
    L.unit("enemy", "sectoid", "Sleeper", 13, 4, dormant=True, pod="barracks")
    L.unit("enemy", "sectoid", "Lab Tech", 25, 4, dormant=True, pod="labs")
    L.unit("enemy", "drone", "Lab Drone", 29, 8, dormant=True, pod="labs")
    L.unit("enemy", "sectoid", "Security Chief", 35, 7, dormant=True, pod="security", tag="chief",
           scripts='<script on="death">message("The security chief drops a keycard reading CORE.");</script>')
    L.unit("enemy", "drone", "Security Drone", 38, 9, dormant=True, pod="security")
    L.unit("enemy", "sectoid", "Storekeeper", 5, 22, dormant=True, pod="storage")
    L.unit("enemy", "sectoid", "Cook", 15, 23, dormant=True, pod="mess")
    L.unit("enemy", "sectoid", "Engineer", 27, 23, dormant=True, pod="generator")
    L.unit("enemy", "sectoid", "Core Warden", 36, 21, dormant=True, pod="core", tag="warden",
           scripts='<script on="spotted" once="true">message("The core wardens! Watch the drone.");</script>')
    L.unit("enemy", "sectoid", "Core Warden 2", 38, 19, dormant=True, pod="core")
    L.unit("enemy", "drone", "Core Drone", 34, 23, dormant=True, pod="core")
    L.unit("enemy", "sectoid", "Corridor Patrol", 21, 13, patrol=[(21, 13), (4, 13), (21, 6), (40, 13)])
    L.script("start", """
    message("Sweep the complex room by room. Doors open as you approach; rooms hide dormant pods.");
    objective("Clear the complex (" + alive_count("enemy") + " hostiles)");""")
    L.script("unit_killed", """
    let left = alive_count("enemy");
    if unit_team(unit) == "enemy" && left > 0 { objective("Clear the complex (" + left + " hostiles left)"); }
    if unit_team(unit) == "enemy" && left == 3 { message("Only the core remains. The security console (north-east) opens it."); }""")
    L.check("complex")
    write("complex", L.xml(
        "  Clear the Complex: explore a facility room by room under fog of war.",
        'id="complex" name="Clear the Complex" seed="44036" fog="true"',
        "Breach the facility, sweep eight rooms with doors and dormant pods, unlock the core from the security room and clear it."))


# =============================================================================
# Infiltration: sneak in, steal the data, get out
# =============================================================================
def infiltration():
    W, H = 44, 34
    L = Level(W, H)
    # The base: an outer fence (full walls) with a gate.
    L.room(12, 2, 42, 24)
    L.door("gate", 12, 18, 1, 3)              # west gate
    L.door("service", 20, 24, 2, 1)           # south service door
    # Buildings inside.
    L.room(15, 4, 24, 11)                    # barracks (north-west)
    L.door("barracks_door", 19, 11, 2, 1)
    L.room(28, 4, 41, 10)                    # vault building (north-east)
    L.door("vault_outer", 28, 8, 1, 2)
    for y in range(5, 10):
        L.set(35, y, "H")                    # vault inner wall
    L.door("vault", 35, 6, 1, 2, locked=True,
           scripts='<script on="opened">message("The vault door grinds open.");</script>')
    L.room(27, 14, 35, 21)                   # security office
    L.door("security_door", 27, 17, 1, 1)
    # Crates in the yard and cover outside.
    for (x, y) in [(16, 15), (17, 15), (22, 18), (24, 14), (25, 14), (38, 13), (39, 13), (38, 20), (16, 21), (23, 22), (40, 17)]:
        L.set(x, y, "h")
    for (x, y) in [(3, 6), (4, 6), (7, 12), (2, 20), (3, 20), (8, 27), (9, 27), (14, 29), (25, 29), (26, 29), (32, 27), (5, 30)]:
        L.set(x, y, "h")
    L.fill(6, 15, 7, 16, "H")
    # A raised watch post (level 2) in the yard with a ladder.
    L.raise_area(31, 12, 32, 12, 2)
    L.ladders.append((33, 12))
    # Data terminal inside the vault; security console in the office.
    L.prop("data_core", "console", 40, 5, "half")
    L.prop("security_console", "console", 33, 15, "half")
    L.tile_script(32, 16, """<script on="enter" team="player" once="true">
      message("Security bypassed: the vault door is unlocked.");
      lock_door("vault", false);
      open_door("vault");
      if flag("data") == 0 { objective("Steal the data from the vault (north-east)"); }
      show_zone("sec_console", false);
    </script>""", marker="objective", label="SECURITY CONSOLE", tag="sec_console")
    L.tile_script(39, 6, """<script on="enter" team="player">
      if flag("data") == 0 {
        set_flag("data", 1);
        set_flag("carrier", unit);
        message(unit_name(unit) + " has the data. Get to the extraction zone (south-west)!");
        objective(unit_name(unit) + ": carry the data to extraction (south-west)");
        ai_goal("player", 2, 31);
        zone_label("data_terminal", "DATA (taken)");
      }
    </script>""", marker="objective", label="DATA TERMINAL", tag="data_terminal")
    # Extraction zone: the squad's starting corner.
    L.zone("extraction", 0, 28, 6, 6, """<script on="enter" team="player">
      if flag("data") == 1 && unit == flag("carrier") {
        message("Data extracted. Mission complete!");
        win();
      } else if flag("data") == 0 {
        message("We don't have the data yet.");
      }
    </script>""", marker="evac", label="EXTRACTION")
    L.deploys.append(dict(team="player", squad="player", x=3, y=31))
    # Guards: patrols walk the yard, sentries hold posts. All unalerted.
    L.unit("enemy", "sectoid", "Gate Patrol", 14, 13, patrol=[(14, 13), (14, 22), (25, 22), (25, 13)])
    L.unit("enemy", "sectoid", "Yard Patrol", 37, 22, patrol=[(37, 22), (37, 12), (26, 12)])
    L.unit("enemy", "sectoid", "Perimeter Patrol", 10, 26, patrol=[(10, 26), (10, 4), (2, 4), (2, 18)])
    L.unit("enemy", "drone", "Watch Drone", 31, 12, dormant=True, pod="post")
    L.unit("enemy", "sectoid", "Vault Guard", 31, 6, dormant=True, pod="vault")
    L.unit("enemy", "sectoid", "Vault Guard 2", 33, 9, dormant=True, pod="vault")
    L.unit("enemy", "sectoid", "Desk Officer", 30, 19, dormant=True, pod="office")
    L.unit("enemy", "sectoid", "Sleeping Guard", 17, 6, dormant=True, pod="barracks")
    L.unit("enemy", "sectoid", "Sleeping Guard 2", 22, 9, dormant=True, pod="barracks")
    L.unit("enemy", "sectoid", "Sleeping Guard 3", 17, 9, dormant=True, pod="barracks")
    L.script("start", """
    set_flag("data", 0);
    message("You are concealed. Red rings show how close each guard notices you. Shooting gives you away.");
    objective("Reach the security office (centre-east) and bypass the vault lock");""")
    L.script("alarm", """
    message("ALARM! The base is on alert - reinforcements from the barracks!");
    open_door("barracks_door");
    if flag("data") == 1 { objective("Fight your way to extraction with the data"); }
    else { objective("Alarm raised - steal the data anyway and fight your way out"); }
    ai_goal("enemy", 3, 30);""")
    L.script("turn_start", """
    if alarm() && round % 2 == 0 && alive_count("enemy") < 14 {
      spawn_unit("enemy", "sectoid", "Response Team", 20, 7);
      spawn_unit("enemy", "drone", "Response Drone", 41, 3);
    }""", team="enemy")
    L.script("unit_killed", """
    if unit == flag("carrier") && flag("data") == 1 {
      set_flag("data", 0);
      message("The data carrier is down and the drive is destroyed! A backup is still on the vault terminal.");
      objective("Recover the backup data from the vault terminal");
      clear_ai_goal("player");
      zone_label("data_terminal", "DATA TERMINAL (backup)");
    }""")
    L.check("infiltration")
    write("infiltration", L.xml(
        "  Silent Theft: infiltrate a base under concealment, steal the data and extract.\n  If you're spotted the alarm sounds: every guard wakes and reinforcements arrive.",
        'id="infiltration" name="Silent Theft" seed="44034" fog="true" concealment="true" detection="4.5" win="script"',
        "Sneak past patrols, bypass the vault lock in the security office, steal the data and carry it back to extraction. Getting caught means fighting your way out."))


if __name__ == "__main__":
    proving_grounds()
    holdout()
    complex_()
    infiltration()
