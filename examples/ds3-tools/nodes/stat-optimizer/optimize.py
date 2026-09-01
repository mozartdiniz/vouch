#!/usr/bin/env python3
"""Optimal stat allocation for a target weapon and soul level.

Reads one JSON object on stdin, writes one JSON object on stdout. Logs go to stderr —
stdout is the payload channel and nothing else may touch it.

The allocation model is deliberately simple and stated here so the numbers mean something:

  * Every build starts from a flat SL1 baseline of 10 in each of the eight stats, so the
    available budget is `soul_level - 1` points.
  * Points go first to the weapon's requirements, then to a survivability floor
    (vigor 27, endurance 20, vitality 20), then into the scaling stats in descending
    order of scaling coefficient, stopping at 40, then 60, then 99.
  * Attack rating is base damage plus each scaling stat's contribution, where a stat
    contributes on a curve that reaches 80% of its value at 40 and 100% at 99. Failing
    the requirements applies a flat penalty.

This is a model of DS3's real curves, not a reproduction of them.
"""

import csv
import json
import os
import sys

BASELINE = 10
SOFT_CAP = 40
MAX_STAT = 99

STATS = [
    "vigor",
    "attunement",
    "endurance",
    "vitality",
    "strength",
    "dexterity",
    "intelligence",
    "faith",
]

# Stats a weapon can require and scale with, mapped to their CSV column suffix.
COMBAT_STATS = {
    "strength": "str",
    "dexterity": "dex",
    "intelligence": "int",
    "faith": "fth",
}

# Survivability floor, applied after requirements and before scaling.
FLOOR = [("vigor", 27), ("endurance", 20), ("vitality", 20)]

REQUIREMENT_PENALTY = 0.4


def load_weapons():
    path = os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "..", "..", "data", "weapons.csv"
    )
    with open(path, newline="", encoding="utf-8") as handle:
        return {row["name"]: row for row in csv.DictReader(handle)}


def saturation(stat):
    """How much of a scaling stat's contribution is realised at this level.

    Zero at the baseline, 0.80 at the soft cap, 1.0 at 99.
    """
    if stat <= BASELINE:
        return 0.0
    if stat <= SOFT_CAP:
        return (stat - BASELINE) / (SOFT_CAP - BASELINE) * 0.80
    return 0.80 + (min(stat, MAX_STAT) - SOFT_CAP) / (MAX_STAT - SOFT_CAP) * 0.20


def main():
    request = json.load(sys.stdin)
    weapon_name = request["weapon"]
    soul_level = request["soul_level"]

    weapons = load_weapons()
    if weapon_name not in weapons:
        # Reachable only if the manifest's weapon precondition has drifted from the CSV.
        # A crash is the right outcome: it is a defect in the node, not a bad question.
        print(f"weapon {weapon_name!r} is not in weapons.csv", file=sys.stderr)
        sys.exit(1)
    weapon = weapons[weapon_name]

    print(f"allocating {soul_level} SL for {weapon_name}", file=sys.stderr)

    stats = {stat: BASELINE for stat in STATS}
    budget = max(0, soul_level - 1)

    def raise_to(stat, target):
        nonlocal budget
        need = min(target, MAX_STAT) - stats[stat]
        if need <= 0 or budget <= 0:
            return
        spend = min(need, budget)
        stats[stat] += spend
        budget -= spend

    requirements = {
        stat: int(weapon[f"req_{suffix}"]) for stat, suffix in COMBAT_STATS.items()
    }
    scaling = {
        stat: float(weapon[f"scale_{suffix}"]) for stat, suffix in COMBAT_STATS.items()
    }

    for stat, required in requirements.items():
        raise_to(stat, required)
    for stat, target in FLOOR:
        raise_to(stat, target)

    # Scaling stats in descending order of coefficient, walking the caps outward so the
    # best stat reaches 40 before the second one starts.
    scaling_order = sorted(
        (stat for stat, coefficient in scaling.items() if coefficient > 0),
        key=lambda stat: -scaling[stat],
    )
    for cap in (SOFT_CAP, 60, MAX_STAT):
        for stat in scaling_order:
            raise_to(stat, cap)

    # Anything left over buys health.
    raise_to("vigor", MAX_STAT)

    requirements_met = all(stats[stat] >= req for stat, req in requirements.items())

    base = float(weapon["base_damage"])
    attack_rating = base
    for stat, coefficient in scaling.items():
        attack_rating += coefficient * base * saturation(stats[stat])
    if not requirements_met:
        attack_rating *= REQUIREMENT_PENALTY

    result = {
        "weapon": weapon_name,
        "soul_level": soul_level,
        "attack_rating": round(attack_rating, 1),
        "total_points_spent": sum(stats.values()) - BASELINE * len(STATS),
        "requirements_met": requirements_met,
        "stats": stats,
    }
    json.dump(result, sys.stdout)


if __name__ == "__main__":
    main()
