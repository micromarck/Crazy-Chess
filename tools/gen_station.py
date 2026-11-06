#!/usr/bin/env python3
"""Generates "Dead Signal" (assets/missions/station.mission.xml).

    python3 tools/gen_station.py

The relay station Halcyon Deep went silent. The squad docks and works its way
through it section by section - docking bay, crew quarters, hydroponics,
medical, command - looking for the crew's logs. Each section's log holds the
override for the next bulkhead; the moment someone steps through, the
station's quarantine protocol seals the bulkhead behind them at the end of
the turn and vents the section: **anyone still behind it dies**.

The symbiotes (Symbiotes module): Crawlers, infected Hosts, mind-controlling
Puppeteers, an Amalgam (2x2) and, on the bridge, the Choir (2x2). Their vines
spread through the station every round and burn - so do they. Seeing them,
reading the logs and losing friends wears the squad's sanity down (Disturbed,
Unhinged: they may snap; at 100 they're lost to the Choir).

Coordinates are mission coordinates: x = column, y = row from the TOP.
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(__file__))
from gen_missions import Level, write  # noqa: E402
from gen_dino_missions import footprint, spot  # noqa: E402

W, H = 97, 37
SPINE = (17, 21)                 # the main corridor's rows (5 wide)
DOOR_ROWS = (18, 20)             # bulkhead doors: rows 18-20 (3 tall)
BULKHEADS = [16, 36, 59, 76]     # bulkhead columns, between sections 1|2|3|4|5

# Floor areas: (x0, y0, x1, y1), inclusive.
FLOORS = [
    # 1. Docking bay: the hangar and the dockmaster's office.
    (1, 6, 15, 31), (8, 1, 15, 4), (12, 5, 12, 5),
    # 2. Crew quarters: the corridor, cabins north and south, the mess hall.
    (17, 17, 35, 21),
    (18, 9, 22, 14), (24, 9, 28, 14), (30, 9, 34, 14),
    (20, 15, 20, 16), (26, 15, 26, 16), (32, 15, 32, 16),
    (18, 24, 22, 29), (24, 24, 28, 29), (30, 24, 34, 29),
    (20, 22, 20, 23), (26, 22, 26, 23), (32, 22, 32, 23),
    (18, 1, 34, 6), (21, 7, 21, 8), (31, 7, 31, 8),
    # 3. Hydroponics: one great overgrown hall, the botanist's office above it.
    (37, 5, 58, 32), (51, 0, 58, 2), (55, 3, 55, 4),
    # 4. Medical: a narrow corridor between labs and wards.
    (60, 17, 75, 21),
    (60, 8, 66, 14), (69, 8, 75, 14), (63, 15, 63, 16), (72, 15, 72, 16),
    (60, 24, 66, 30), (69, 24, 75, 30), (63, 22, 63, 23), (72, 22, 72, 23),
    (67, 10, 68, 10), (67, 27, 68, 27),
    # 5. Command: the bridge (the Choir) and the escape pods.
    (77, 4, 95, 32), (84, 0, 88, 3),
]
# Bulkhead door tiles.
for xb in BULKHEADS:
    FLOORS.append((xb, DOOR_ROWS[0], xb, DOOR_ROWS[1]))

LOGS = [
    # (x, y, title, text, insanity)
    (13, 2, "DOCKMASTER'S LOG",
     "Dockmaster Reyes, day 41: The survey drone came back with something stuck to its hull. Kept it in quarantine bay 2. "
     "Bulkhead override for the quarters is 7-7-1-Delta. If you're reading this, don't open bay 2.", 4),
    (32, 27, "CABIN 6 - PERSONAL LOG",
     "Ensign Okafor: Mara stopped talking yesterday. She just sits and hums the same five notes. Everyone in the mess hall is "
     "humming it now. I hum it too when I forget not to. Override for hydroponics: 3-0-3-Gamma.", 6),
    (55, 1, "BOTANIST'S LOG",
     "Dr. Vasquez: It isn't a plant. It grows along the decking a hand's width a day and it BURNS - the only thing that "
     "slows it is fire. I flooded the east planters with fuel. Override for medical: 9-1-2-Sigma.", 6),
    (73, 28, "MEDICAL OFFICER'S LOG",
     "Dr. Lin: The infected don't die. They join. Their minds go somewhere - into the big one on the bridge, the Choir. "
     "It can reach out and take you if it sees you. Keep your heads down. Command override: 0-0-0-Omega.", 8),
    (92, 6, "CAPTAIN'S LOG",
     "Captain Hale: I'm sealing myself on the bridge with it. If anyone finds this - the escape pods are behind me. Burn it. "
     "Burn all of it. And don't listen to the singing.", 8),
]
POD = (91, 16, 4, 7)     # the escape pod zone (x, y, w, h)


def in_rect(x, y, r):
    x0, y0, x1, y1 = r
    return x0 <= x <= x1 and y0 <= y <= y1


def build() -> Level:
    L = Level(W, H)
    floor = {(x, y) for r in FLOORS for x in range(r[0], r[2] + 1) for y in range(r[1], r[3] + 1)}
    for y in range(H):
        for x in range(W):
            if (x, y) in floor:
                continue
            near = any((x + dx, y + dy) in floor for dx in (-1, 0, 1) for dy in (-1, 0, 1))
            L.set(x, y, "H" if near else "#")
    # --- 1. Docking bay: crates, a shuttle's landing struts, the airlock.
    for (x, y) in [(5, 9), (6, 9), (5, 10), (10, 12), (11, 12), (4, 26), (5, 26), (9, 27), (10, 27), (12, 24), (13, 9), (3, 14)]:
        L.set(x, y, "h")
    for (x, y) in [(8, 14), (8, 15), (12, 28), (13, 28)]:
        L.set(x, y, "H")
    # --- 2. Quarters: bunks and lockers in the cabins, tables in the mess.
    for cx in (18, 24, 30):
        L.set(cx, 10, "h"); L.set(cx + 4, 13, "h"); L.set(cx, 28, "h"); L.set(cx + 4, 25, "h")
    for x in range(20, 33, 4):
        L.set(x, 3, "h"); L.set(x + 1, 3, "h")
    # --- 3. Hydroponics: rows of planters (low cover) with gaps.
    for x in range(40, 57, 4):
        for y in list(range(7, 15)) + list(range(24, 31)):
            if y not in (10, 11, 27):
                L.set(x, y, "h")
    # --- 4. Medical: beds and cabinets.
    for (x, y) in [(61, 9), (61, 11), (61, 13), (74, 9), (74, 11), (70, 13), (61, 25), (61, 27), (61, 29), (65, 29), (70, 25), (74, 26)]:
        L.set(x, y, "h")
    # --- 5. Command: consoles in rings round the bridge.
    for (x, y) in [(80, 8), (81, 8), (80, 28), (81, 28), (89, 9), (90, 9), (89, 27), (90, 27), (79, 13), (79, 23), (93, 12), (93, 25)]:
        L.set(x, y, "h")
    for x, y, *_ in LOGS:
        L.set(x, y, ".")
    return L


def populate(L: Level):
    L.deploys.append(dict(team="player", squad="player", x=3, y=19))
    taken = set()

    def put(cls, name, x, y, side=1, **kw):
        x, y = spot(L, x, y, side, taken)
        taken.update(footprint(x, y, side))
        L.unit("enemy", cls, name, x, y, dormant=kw.pop("dormant", True), **kw)
        return x, y

    # 1. Docking bay: a few hosts by the cargo, crawlers in the dark.
    put("symb_host", "Deckhand", 11, 9, pod="dock")
    put("symb_host", "Loader", 4, 28, pod="dock")
    put("symb_crawler", "Crawler", 13, 26, pod="dock")
    # 2. Quarters: the crew in their cabins; a Puppeteer in the mess.
    for i, (x, y) in enumerate([(20, 12), (26, 11), (32, 12), (20, 26), (26, 27)]):
        put("symb_host", f"Crewman {i + 1}", x, y, pod=f"cabin{i}")
    put("symb_puppeteer", "Puppeteer", 27, 4, pod="mess")
    put("symb_crawler", "Crawler", 23, 2, pod="mess")
    put("symb_crawler", "Crawler", 33, 5, pod="mess")
    # 3. Hydroponics: crawlers everywhere, the Amalgam among the planters.
    for i, (x, y) in enumerate([(39, 8), (43, 13), (47, 28), (51, 9), (55, 26), (45, 31), (53, 15)]):
        put("symb_crawler", f"Crawler {i + 1}", x, y, pod=f"hydro{i % 3}")
    put("symb_amalgam", "The Amalgam", 49, 21, side=2, pod="amalgam")
    put("symb_host", "Gardener", 57, 30, pod="hydro0")
    # 4. Medical: patients who got up again, two Puppeteers.
    for i, (x, y) in enumerate([(63, 10), (71, 12), (64, 27), (72, 26)]):
        put("symb_host", f"Patient {i + 1}", x, y, pod=f"ward{i}")
    put("symb_puppeteer", "Puppeteer", 73, 10, pod="lab_n")
    put("symb_puppeteer", "Puppeteer", 66, 26, pod="lab_s")
    put("symb_crawler", "Crawler", 68, 19, pod="ward0")
    # 5. Command: the Choir, its hosts and an honour guard.
    put("symb_choir", "The Choir", 85, 20, side=2, tag="choir", pod="choir")
    for i, (x, y) in enumerate([(82, 12), (82, 26), (88, 14), (88, 25)]):
        put("symb_host", f"Bridge Crew {i + 1}", x, y, pod="bridge")
    put("symb_puppeteer", "Puppeteer", 91, 28, pod="bridge")

    # Supplies along the way.
    for tag, kind, x, y in [("med1", "supply_medkit", 9, 3), ("ammo1", "supply_ammo", 33, 26), ("nade1", "supply_grenade", 18, 5),
                            ("ammo2", "supply_ammo", 52, 1), ("med2", "supply_medkit", 74, 29), ("med3", "supply_medkit", 62, 13),
                            ("ammo3", "supply_ammo", 79, 31)]:
        L.prop(tag, kind, x, y, cover="none")
    # Fuel tanks in hydroponics and on the bridge: the botanist's answer.
    for i, (x, y) in enumerate([(42, 19), (50, 7), (54, 30), (46, 24), (80, 20), (86, 6)]):
        L.prop(f"fuel{i}", "barrel", x, y, cover="half", hp=2,
               scripts='<script on="destroyed">message("A fuel tank goes up!"); explode(x, y, 2.0, 4); ignite(x, y, 2.5, 3);</script>')
    # Consoles beside each log terminal (cover, and something to look at).
    for i, (x, y, *_r) in enumerate(LOGS):
        cx = x + 1 if L.walkable(x + 1, y) else x - 1
        L.prop(f"console{i}", "console", cx, y, cover="half")

    # Bulkheads: locked until that section's log is found.
    for i, xb in enumerate(BULKHEADS, start=1):
        L.door(f"bh{i}", xb, DOOR_ROWS[0], 1, DOOR_ROWS[1] - DOOR_ROWS[0] + 1, locked=True)
        # Stepping into the next section arms the seal.
        L.zone(f"sec{i + 1}", xb + 1, SPINE[0] - 1, 3, SPINE[1] - SPINE[0] + 3, f"""<script on="enter" team="player">
      if flag("seal{i}") == 0 {{
        set_flag("seal{i}", 1);
        message("QUARANTINE PROTOCOL: bulkhead {i} seals at the end of this turn. Anyone left behind it is lost!");
        zone_label("bh{i}_mark", "SEALING");
      }}
    </script>""")
        L.zone(f"bh{i}_mark", xb, DOOR_ROWS[0], 1, 3, "<script on=\"enter\"></script>", marker="objective", label=f"BULKHEAD {i} (LOCKED)")

    # The logs.
    for i, (x, y, title, text, mad) in enumerate(LOGS, start=1):
        if i < len(LOGS):
            after = f'''lock_door("bh{i}", false);
        zone_label("bh{i}_mark", "BULKHEAD {i} (OPEN)");
        objective("Logs found: " + flag("logs") + " of {len(LOGS)}. Bulkhead {i} is unlocked - push on (east)");'''
        else:
            after = '''objective("Get everyone still yourself into the escape pods (east of the bridge)");
        zone_label("pod", "ESCAPE PODS - BOARD");'''
        L.tile_script(x, y, f"""<script on="enter" team="player">
      if flag("log{i}") == 0 {{
        set_flag("log{i}", 1);
        set_flag("logs", flag("logs") + 1);
        message("{title}: {text}");
        add_insanity(unit, {mad});
        zone_label("log{i}", "{title} (read)");
        {after}
      }}
    </script>""", marker="objective", label=title, tag=f"log{i}")

    px, py, pw, ph = POD
    L.zone("pod", px, py, pw, ph, """<script on="enter" team="player">
      if flag("log5") == 1 {
        let squad = alive_count("player");
        let here = zone_count("pod", "player");
        if squad > 0 && here >= squad { message("The pods launch. Halcyon Deep falls away behind you, singing. Mission complete."); win(); }
        else { message(here + " of " + squad + " in the pods - everyone still themselves has to make it."); }
      } else { message("The pods won't launch without the captain's release code - it's in her log on the bridge."); }
    </script>""", marker="evac", label="ESCAPE PODS (LOCKED)")

    xs = "[" + ", ".join(str(x) for x in BULKHEADS) + "]"
    L.library = f"""
    // The quarantine protocol: seal bulkhead i (at column xb) and vent everything behind it.
    fn seal(i, xb) {{
      let lost = 0;
      for id in units("player") {{
        if unit_x(id) <= xb {{
          message(unit_name(id) + " was caught behind bulkhead " + i + ". Through the port you watch the section vent to space.");
          kill(id);
          lost += 1;
        }}
      }}
      close_door("bh" + i);
      lock_door("bh" + i, true);
      zone_label("bh" + i + "_mark", "BULKHEAD " + i + " (SEALED)");
      if lost > 0 {{
        for id in units("player") {{ add_insanity(id, 10 * lost); }}
      }} else {{
        message("Bulkhead " + i + " slams shut behind you. The section beyond it vents to space.");
      }}
    }}"""
    L.script("start", """
    set_flag("logs", 0);
    set_vine_growth(5);
    // The growth: a little in the bay, more in the quarters, the hydroponics hall
    // choked with it, and the bridge.
    plant_vines(12, 26, 2.0);
    plant_vines(26, 3, 2.5);
    plant_vines(22, 26, 1.5);
    plant_vines(44, 10, 3.0);
    plant_vines(52, 26, 3.5);
    plant_vines(47, 19, 3.0);
    plant_vines(64, 28, 2.0);
    plant_vines(86, 20, 4.0);
    message("Halcyon Deep Relay. No answer to hails for nine days. Life support is still running.");
    message("Find the crew's logs - each one has the override for the next bulkhead. Watch for growth on the deck: it spreads, and it burns.");
    objective("Find the dockmaster's log (docking bay, north-east office)");""")
    L.script("turn_start", f"""
    let xs = {xs};
    for i in 1..5 {{
      if flag("seal" + i) == 1 {{
        set_flag("seal" + i, 2);
        seal(i, xs[i - 1]);
      }}
    }}
    // The station whispers.
    if round % 5 == 0 {{
      message("The lights flicker. Somewhere in the walls, a hundred voices hum five notes.");
      for id in units("player") {{ add_insanity(id, 2); }}
    }}""", team="enemy")
    L.script("turn_start", """
    let squad = alive_count("player");
    if flag("log5") == 1 && squad > 0 && zone_count("pod", "player") >= squad {
      message("The pods launch. Halcyon Deep falls away behind you, singing. Mission complete.");
      win();
    }""", team="player")
    L.script("unit_killed", """
    if unit_class(unit) == "symb_choir" {
      message("The Choir shrieks with a hundred voices and falls silent. In your head, at last: quiet.");
      for id in units("player") { add_insanity(id, -25); }
      set_vine_growth(0);
    }""")


def surfaces(L: Level) -> list[str]:
    rows = []
    for y in range(L.h):
        row = ""
        for x in range(L.w):
            t = L.tiles[y][x]
            if t == "#":
                row += "."
            elif t in "Hh":
                row += "m"          # bulkheads, panels and crates: bare metal
            elif 37 <= x <= 58 and 5 <= y <= 32:
                row += "d" if (x * 7 + y * 3) % 5 else "g"     # hydroponics soil
            elif SPINE[0] <= y <= SPINE[1]:
                row += "c"
            else:
                row += "m"
        rows.append(row)
    return rows


def main():
    L = build()
    populate(L)
    L.check("station")
    text = L.xml(
        "  Dead Signal: explore the abandoned relay station Halcyon Deep for the crew's\n"
        "  logs while the symbiotes (Symbiotes module) close in. Each log unlocks the\n"
        "  next bulkhead; each bulkhead seals behind you - anyone left behind dies.",
        'id="station" name="Dead Signal" seed="77031" fog="true" win="script"',
        "The relay station Halcyon Deep went silent nine days ago. Work through it section by section - docking bay, crew "
        "quarters, hydroponics, medical, the bridge - and find the crew's logs: each holds the override for the next bulkhead. "
        "The quarantine protocol seals every bulkhead behind you at the end of the turn you pass it and vents the section: "
        "anyone left behind dies. The symbiotes take minds, rage when hurt and spread their vines across the deck every "
        "round - but they burn. Watch your squad's sanity: the disturbed lose aim, the unhinged may snap, and the lost "
        "belong to the Choir. Read the captain's log on the bridge, then get to the escape pods.",
    )
    text = text.replace("Generated by tools/gen_missions.py", "Generated by tools/gen_station.py")
    text = text.replace("  </layout>", "  </layout>\n  <surfaces>\n" + "\n".join("    " + r for r in surfaces(L)) + "\n  </surfaces>", 1)
    text = text.replace("  <layout>", '  <sky top="#02030a" horizon="#0c1226"/>\n\n  <layout>', 1)
    write("station", text)


if __name__ == "__main__":
    main()
