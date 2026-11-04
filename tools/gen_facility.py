#!/usr/bin/env python3
"""Generates "Ghost in the Machine" (assets/missions/facility.mission.xml).

    python3 tools/gen_facility.py

An abandoned single-storey office block run by a rogue facility AI
(VESPER, from the `facility_ai` module). The layout is drawn here so it can
be checked automatically:

* every room has at least two ways in (VESPER locks one door at a time),
* with ANY one door locked, every floor tile can still be reached from the
  insertion point,
* units, props, patrols and objectives sit on usable tiles.

Coordinates are mission coordinates: x = column, y = row from the TOP.
"""

from __future__ import annotations

import os
import sys
from collections import deque

sys.path.insert(0, os.path.dirname(__file__))
from gen_missions import Level, write  # noqa: E402

W, H = 64, 46


def build() -> Level:
    L = Level(W, H)
    L.fill(0, 0, W - 1, H - 1, "#")

    # --- building shell: two overlapping wings (not a rectangle) ----------
    # North-east wing x14-63, y0-35; south-west wing x0-49, y10-41.
    def shell(x0, y0, x1, y1):
        L.fill(x0, y0, x1, y1, ".")
    shell(14, 0, 63, 35)
    shell(0, 10, 49, 41)
    # Outer wall: any floor tile next to the outside becomes wall.
    inside = {(x, y) for y in range(H) for x in range(W) if L.tiles[y][x] == "."}
    for (x, y) in list(inside):
        for dx in (-1, 0, 1):
            for dy in (-1, 0, 1):
                n = (x + dx, y + dy)
                if n not in inside:
                    L.set(x, y, "H")
    # A notch bitten out of the north-west wing's corner (rubble outside).
    # Parking lot south of the lobby (outside, open, cars for cover).
    L.fill(0, 42, 30, 45, ".")

    def hwall(y, x0, x1):
        for x in range(x0, x1 + 1):
            L.set(x, y, "H")

    def vwall(x, y0, y1):
        for y in range(y0, y1 + 1):
            L.set(x, y, "H")

    # --- corridors ---------------------------------------------------------
    # Main east-west corridor: rows 23-24 (walls at 22 and 25).
    hwall(22, 1, 62)
    hwall(25, 1, 62)
    # North-south spine: columns 31-32 (walls at 30 and 33), y1-41.
    vwall(30, 1, 41)
    vwall(33, 1, 41)
    # Open the crossings.
    for x in (31, 32):
        L.set(x, 22, ".")
        L.set(x, 25, ".")
    for y in (23, 24):
        L.set(30, y, ".")
        L.set(33, y, ".")

    # --- north-west block (x15-29, y1-21) ---------------------------------
    # Records archive (the data) y1-9, break room + meeting rooms below.
    hwall(10, 15, 29)
    vwall(22, 11, 21)
    # West wing offices (x1-13, y11-21).
    vwall(14, 10, 21)
    vwall(7, 11, 21)
    # A tight service passage behind the archive: row 1-2 is the archive,
    # column 15 row 11-21 stays break room.

    # --- north-east open-plan office (x34-62, y1-21) ----------------------
    # Executive suite in the corner (x52-62, y1-8).
    hwall(9, 52, 62)
    vwall(51, 1, 9)

    # --- south-west: lobby (x1-20, y26-41), security + storage ------------
    vwall(21, 26, 41)
    hwall(33, 22, 29)

    # --- south-east: server hall, maintenance, loading dock ---------------
    # Server hall x34-47, y26-34; maintenance tunnel x34-48, y36-40;
    # loading dock x50-62, y26-34.
    hwall(35, 34, 48)
    vwall(48, 26, 35)
    # 1-wide service corridor between the server hall and the dock (x49),
    # walls x48 and x50.
    vwall(50, 26, 34)

    # --- doors (every room gets at least two) -----------------------------
    D = L.door
    # Archive (data): from the corridor-side meeting rooms and the spine.
    D("archive_spine", 30, 5)
    D("archive_break", 18, 10)
    D("archive_meeting", 26, 10)
    # Break room (x15-21, y11-21).
    D("break_corridor", 18, 22)
    D("break_west", 14, 16)
    D("break_meeting", 22, 16)
    # Meeting rooms (x23-29, y11-21).
    D("meeting_corridor", 26, 22)
    D("meeting_spine", 30, 15)
    # West offices.
    D("office_w1_corridor", 4, 22)
    D("office_w1_w2", 7, 16)
    D("office_w2_corridor", 11, 22)
    # Open-plan office: wide doors from the spine and the corridor.
    D("openplan_spine", 33, 8, 1, 2)
    D("openplan_spine_s", 33, 18)
    D("openplan_corridor", 40, 22, 2, 1)
    D("openplan_corridor_e", 58, 22)
    # Executive suite.
    D("exec_openplan", 51, 5)
    D("exec_south", 56, 9)
    # Lobby: two corridor doors (the front of the lobby is open to the lot).
    D("lobby_corridor", 8, 25, 2, 1)
    D("lobby_corridor_e", 17, 25)
    D("lobby_security", 21, 29)
    D("lobby_storage", 21, 37)
    # Security office (x22-29, y26-32) and storage (x22-29, y34-41).
    D("security_spine", 30, 29)
    D("security_storage", 25, 33)
    D("storage_spine", 30, 38)
    # Server hall.
    D("server_spine", 33, 29)
    D("server_corridor", 41, 25, 2, 1)
    D("server_service", 48, 30)
    D("server_maint", 40, 35)
    # Maintenance tunnel (x34-48, y36-40).
    D("maint_spine", 33, 38)
    # Service passage (x49) to the dock.
    D("service_corridor", 49, 25)
    D("dock_service", 50, 31)
    # Loading dock.
    D("dock_corridor", 56, 25, 2, 1)
    # Lobby front: an open gap to the parking lot (no door).
    for x in range(3, 19):
        L.set(x, 41, ".")
    for x in (1, 2, 19, 20):
        L.set(x, 41, "H")

    # --- furniture ---------------------------------------------------------
    def h(x, y):
        if L.tiles[y][x] == ".":
            L.set(x, y, "h")

    # Archive shelving (full-height rows with gaps: tight aisles).
    for x in (17, 20, 23, 26):
        for y in range(2, 8):
            if y != 5:
                L.set(x, y, "H")
    # Open-plan office: cubicle clusters (half cover).
    for cx in (36, 41, 46):
        for cy in (3, 8, 13, 18):
            if (cx, cy) in ((46, 3), (46, 8)):
                continue
            for dx in (0, 1, 2):
                h(cx + dx, cy)
            h(cx + 1, cy + 1)
    for cx in (53, 58):
        for cy in (12, 17):
            for dx in (0, 1, 2):
                h(cx + dx, cy)
    # Executive suite: a big desk.
    for x in (55, 56, 57, 58):
        h(x, 4)
    # Break room tables, meeting tables.
    for (x, y) in [(17, 13), (18, 13), (17, 18), (18, 18), (20, 15)]:
        h(x, y)
    for (x, y) in [(25, 13), (26, 13), (27, 13), (25, 19), (26, 19), (27, 19)]:
        h(x, y)
    # West offices: desks.
    for (x, y) in [(3, 14), (4, 14), (10, 14), (11, 14), (3, 19), (11, 19)]:
        h(x, y)
    # Lobby: reception desk, planters, seating (open space).
    for x in range(8, 13):
        h(x, 30)
    for (x, y) in [(3, 28), (18, 28), (3, 36), (18, 36), (6, 34), (7, 34), (14, 34), (15, 34)]:
        h(x, y)
    # Security office desks.
    for (x, y) in [(24, 28), (25, 28), (27, 31)]:
        h(x, y)
    # Storage: shelves.
    for (x, y) in [(23, 35), (24, 35), (27, 35), (28, 35), (23, 39), (24, 39), (27, 39), (28, 39)]:
        L.set(x, y, "H")
    # Server hall: rows of racks (full cover) with 1-tile aisles.
    for y in (27, 29, 31, 33):
        for x in range(35, 47):
            if x not in (40, 41) and not (y == 33 and x in (39, 42)):
                L.set(x, y, "H")
    # Maintenance tunnel: pipes and crates.
    for (x, y) in [(36, 37), (37, 39), (42, 37), (45, 39), (46, 37)]:
        h(x, y)
    # Loading dock: crates, forklift bay.
    for (x, y) in [(53, 28), (54, 28), (59, 28), (53, 32), (59, 32), (60, 32)]:
        h(x, y)
    # Corridor junk (some cover in the long corridor).
    for (x, y) in [(6, 23), (24, 24), (37, 23), (46, 24), (55, 23), (31, 12), (32, 34)]:
        h(x, y)
    # Parking lot: cars.
    for (x, y) in [(4, 43), (5, 43), (10, 44), (11, 44), (16, 43), (17, 43), (23, 44), (24, 44)]:
        h(x, y)
    return L


# --- checks ------------------------------------------------------------------
def passable(L: Level, x, y, locked: str | None):
    if not (0 <= x < L.w and 0 <= y < L.h):
        return False
    for d in L.doors:
        if d["x"] <= x < d["x"] + d["w"] and d["y"] <= y < d["y"] + d["h"]:
            return d["tag"] != locked
    return L.tiles[y][x] == "."


def reach(L: Level, start, locked=None):
    seen = {start}
    q = deque([start])
    while q:
        x, y = q.popleft()
        for dx in (-1, 0, 1):
            for dy in (-1, 0, 1):
                n = (x + dx, y + dy)
                if n in seen or not passable(L, *n, locked):
                    continue
                # No corner cutting past walls.
                if dx and dy and not (passable(L, x + dx, y, locked) and passable(L, x, y + dy, locked)):
                    continue
                seen.add(n)
                q.append(n)
    return seen


def rooms(L: Level):
    """Floor regions with every door closed."""
    door_tiles = {(xx, yy) for d in L.doors for yy in range(d["y"], d["y"] + d["h"]) for xx in range(d["x"], d["x"] + d["w"])}
    left = {(x, y) for y in range(L.h) for x in range(L.w) if L.tiles[y][x] in ".h" and (x, y) not in door_tiles}
    out = []
    while left:
        s = left.pop()
        region = {s}
        q = deque([s])
        while q:
            x, y = q.popleft()
            for n in ((x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)):
                if n in left:
                    left.discard(n)
                    region.add(n)
                    q.append(n)
        out.append(region)
    return out, door_tiles


def check_layout(L: Level, start):
    problems = []
    floor = {(x, y) for y in range(L.h) for x in range(L.w) if L.tiles[y][x] == "."}
    for d in L.doors:
        # A door panel needs floor on two opposite sides.
        x, y = d["x"], d["y"]
        ok = (L.walkable(x - 1, y) and L.walkable(x + d["w"], y)) or (L.walkable(x, y - 1) and L.walkable(x, y + d["h"]))
        if not ok:
            problems.append(f"door {d['tag']} at {x},{y} doesn't join two spaces")
    base = reach(L, start)
    missing = floor - base
    if missing:
        problems.append(f"{len(missing)} floor tiles unreachable, e.g. {sorted(missing)[:5]}")
    for d in L.doors:
        cut = floor - reach(L, start, locked=d["tag"])
        if cut:
            problems.append(f"locking {d['tag']} cuts off {len(cut)} tiles, e.g. {sorted(cut)[:3]}")
    regions, door_tiles = rooms(L)
    for r in regions:
        if len(r) < 4:
            continue
        exits = set()
        for d in L.doors:
            tiles = [(xx, yy) for yy in range(d["y"], d["y"] + d["h"]) for xx in range(d["x"], d["x"] + d["w"])]
            if any((tx + dx, ty + dy) in r for tx, ty in tiles for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1))):
                exits.add(d["tag"])
        open_edge = any(not (0 <= x < L.w - 1 and 0 < y < L.h - 1) for x, y in r)
        if len(exits) < 2 and not open_edge:
            x, y = min(r)
            problems.append(f"room near {x},{y} has only {sorted(exits)}")
    if problems:
        sys.exit("facility layout: " + "\n  ".join(problems))


def populate(L: Level):
    L.deploys.append(dict(team="player", squad="player", x=12, y=43))

    # Props.
    L.prop("vesper_core", "console", 41, 28, "half")
    L.prop("archive_terminal", "console", 21, 5, "half")
    L.prop("dock_crate", "crate", 61, 33, "half", 4)

    # --- enemies: mostly small security bots, a few big Wardens roaming ---
    def u(cls, name, x, y, **kw):
        L.unit("enemy", cls, name, x, y, **kw)

    # The boss: VESPER's own chassis in the server hall.
    u("vesper_prime", "VESPER Prime", 41, 30, tag="vesper", dormant=True, pod="server")
    u("fac_secbot", "Rack Sentinel", 37, 28, dormant=True, pod="server")
    u("fac_secbot", "Rack Sentinel 2", 44, 32, dormant=True, pod="server")

    # Wardens (large) patrol the long corridors.
    u("fac_warden", "Warden W-1", 5, 24, patrol=[(5, 24), (28, 23), (28, 24)])
    u("fac_warden", "Warden W-2", 60, 23, patrol=[(60, 23), (35, 24), (35, 23)])
    u("fac_warden", "Warden W-3", 31, 3, patrol=[(31, 3), (32, 20), (31, 39), (32, 20)])

    # Security bots in pods around the building.
    u("fac_secbot", "Lobby Bot", 10, 33, dormant=True, pod="lobby")
    u("fac_secbot", "Desk Bot", 26, 30, dormant=True, pod="security")
    u("fac_drone", "Camera Drone", 23, 27, dormant=True, pod="security")
    u("fac_secbot", "Stock Bot", 26, 37, dormant=True, pod="storage")
    u("fac_secbot", "Office Bot", 4, 17, dormant=True, pod="west")
    u("fac_secbot", "Office Bot 2", 10, 12, dormant=True, pod="west")
    u("fac_secbot", "Cafe Bot", 19, 19, dormant=True, pod="break")
    u("fac_secbot", "Minutes Bot", 27, 16, dormant=True, pod="meeting")
    u("fac_drone", "Meeting Drone", 24, 12, dormant=True, pod="meeting")
    u("fac_secbot", "Archivist", 19, 3, dormant=True, pod="archive")
    u("fac_secbot", "Archivist 2", 25, 8, dormant=True, pod="archive")
    u("fac_drone", "Archive Drone", 28, 2, dormant=True, pod="archive")
    u("fac_secbot", "Cubicle Bot", 38, 5, dormant=True, pod="openplan")
    u("fac_secbot", "Cubicle Bot 3", 48, 10, patrol=[(48, 10), (48, 20), (35, 20), (35, 11)])
    u("fac_drone", "Floor Drone", 56, 15, dormant=True, pod="openplan")
    u("fac_secbot", "Exec Guard", 57, 6, dormant=True, pod="exec")
    u("fac_secbot", "Dock Bot", 55, 30, dormant=True, pod="dock")
    u("fac_secbot", "Dock Bot 2", 60, 27, dormant=True, pod="dock")
    u("fac_secbot", "Tunnel Bot", 44, 38, patrol=[(44, 38), (35, 38), (47, 38)])

    # --- objectives ---------------------------------------------------------
    L.tile_script(21, 6, """<script on="enter" team="player">
      if flag("data") == 0 {
        set_flag("data", 1);
        set_flag("carrier", unit + 1);
        message(unit_name(unit) + " has the research data. Get everyone to the loading dock (east)!");
        objective(unit_name(unit) + ": carry the data to the loading dock - everyone has to make it");
        zone_label("data", "DATA (taken)");
        show_zone("extraction", true);
        vesper_lockdown();
      }
    </script>""", marker="objective", label="RESEARCH DATA", tag="data")
    L.tile_script(41, 29, """<script on="enter" team="player">
      if flag("vesper_purged") == 0 {
        vesper_purge();
        zone_label("core_access", "CORE (purged)");
      }
    </script>""", marker="objective", label="VESPER CORE", tag="core_access")

    L.zone("extraction", 54, 29, 5, 3, """<script on="enter" team="player">
      if flag("data") == 1 {
        evac(zone_count("extraction", "player"), alive_count("player"), "The data is out and the whole squad with it. Mission complete!");
      } else {
        message("No data yet - the research archive is in the north-west.");
      }
    </script>""", marker="evac", label="EXTRACTION")

    L.library = """
    fn evac(inside, squad, done) {
      if squad > 0 && inside >= squad { message(done); win(); }
      else { message(inside + " of " + squad + " soldiers at the dock - everyone still alive has to make it."); }
    }

    // Taking the data: VESPER boots its spare machines.
    fn vesper_lockdown() {
      if flag("vesper_on") == 0 { return; }
      message("VESPER: 'That data is not yours.' Lockdown - spare machines are booting up.");
      spawn_unit("enemy", "fac_secbot", "Lockdown Bot", 61, 23);
      spawn_unit("enemy", "fac_secbot", "Lockdown Bot 2", 36, 39);
    }"""
    L.script("start", """
    set_flag("data", 0);
    vesper_init(unit_by_tag("vesper"), round);
    message("The facility is dark, but something is still awake. VESPER, the building's AI, is watching through every camera.");
    message("VESPER can lock one door at a time (red) and jump into another machine every 3 turns. Destroy the machine it's in and every bot freezes for 2 turns.");
    objective("Find the research data in the archive (north-west)");""")
    L.script("turn_start", "vesper_turn(round);", team="enemy")
    L.script("turn_start", """
    if flag("data") == 1 && alive_count("player") > 0 && zone_count("extraction", "player") >= alive_count("player") {
      message("The data is out and the whole squad with it. Mission complete!");
      win();
    }""", team="player")
    L.script("unit_killed", """
    if unit + 1 == flag("carrier") && flag("data") == 1 {
      set_flag("data", 0);
      set_flag("carrier", 0);
      message(unit_name(unit) + " is down and VESPER wipes the drive! The archive terminal still holds a copy.");
      objective("Download the research data again from the archive terminal (north-west)");
      zone_label("data", "RESEARCH DATA (copy)");
    }""")


def main():
    L = build()
    check_layout(L, (12, 43))
    populate(L)
    L.check("facility")
    text = L.xml(
        "  Ghost in the Machine: an abandoned office block run by a rogue AI, VESPER\n"
        "  (classes and boss logic in assets/modules/facility_ai.ron). Get the data,\n"
        "  get out. Every room has two or more doors: VESPER locks one at a time.",
        'id="facility" name="Ghost in the Machine" seed="90417" fog="true" win="script"',
        "An abandoned research office, flat and dark: open-plan floors, cramped archives and server aisles. "
        "Its AI, VESPER, still runs the building - it locks doors in your face and jumps between its machines. "
        "Find the research data in the archive, then get the whole squad to the loading dock. "
        "Destroy the machine VESPER is in to freeze every bot for 2 turns; purge its core in the server hall to stop it jumping.",
    )
    text = text.replace("Generated by tools/gen_missions.py", "Generated by tools/gen_facility.py")
    # Night sky through the skylights.
    text = text.replace("  <layout>", '  <sky top="#0b1020" horizon="#2a3346"/>\n\n  <layout>', 1)
    write("facility", text)


if __name__ == "__main__":
    main()
