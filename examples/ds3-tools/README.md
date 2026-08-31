# ds3-tools

An example `vouch` collection: build optimization for Dark Souls 3 over a local CSV dataset.

This is a demonstration of how to write nodes, not part of the runtime. Nothing here is
compiled into the `vouch` binary — the node is a Python script the runtime executes as a
subprocess.

It is the collection the spec's acceptance demo uses, because nearly every output is a
checkable scalar: stat spreads, attack ratings, points spent.

## Run it

From this directory:

```console
$ vouch list
stat-optimizer  Optimal stat allocation for a target weapon and soul level in Dark Souls 3

$ vouch describe stat-optimizer

$ vouch call stat-optimizer --input '{"weapon":"Lothric Knight Sword","soul_level":120}'
{
  "attack_rating": 190.9,
  "requirements_met": true,
  "soul_level": 120,
  "stats": { "strength": 42, "dexterity": 60, "vigor": 27, ... },
  "total_points_spent": 119,
  "weapon": "Lothric Knight Sword"
}
```

`vouch` locates a collection by walking up from the working directory looking for `nodes/`, so
commands run from here or anywhere beneath it. From the repository root, point the tool at
this directory instead:

```console
$ vouch -C examples/ds3-tools list
```

## What it demonstrates

**A value that satisfied its contracts.** The call above returns exit 0. Every number in it
came from `optimize.py` reading `data/weapons.csv`, not from a model.

**A refusal you can act on** (§4.2). Preconditions carry the message the caller sees, so a
wrong call fails informatively rather than silently producing a plausible number:

```console
$ vouch call stat-optimizer --input '{"weapon":"lothric sword","soul_level":120}'
{"outcome":"refusal","code":11,"node":"stat-optimizer","reason":"unknown weapon; this node
covers only: Lothric Knight Sword, Uchigatana, ... Names must match in-game spelling exactly."}
$ echo $?
11
```

**A defect that leaks no value.** Break the arithmetic in `optimize.py` — change
`budget = max(0, soul_level - 1)` to `soul_level * 2` — and the postcondition
`result.total_points_spent <= input.soul_level` catches it:

```console
$ vouch call stat-optimizer --input '{"weapon":"Lothric Knight Sword","soul_level":120}'
{"outcome":"defect","code":13,"node":"stat-optimizer","reason":"postcondition failed: ..."}
$ echo $?
13
```

The rejected result is reported on stderr for debugging and never written to stdout. A caller
reading stdout gets nothing, which is the point: no answer beats a wrong one.

**Where bounds belong.** `input.schema.json` types `soul_level` as an integer but does not
bound it. The 1–802 range lives in a precondition instead, so an out-of-range level *refuses*
(exit 11, "outside this node's competence") rather than being rejected as *malformed*
(exit 10). The schema checks shape; contracts check competence.

## Layout

```
ds3-tools/
  nodes/
    stat-optimizer/
      node.toml            manifest: routing context, contracts, interface
      input.schema.json
      output.schema.json
      optimize.py          the node: JSON on stdin, JSON on stdout
      data/weapons.csv     declared under [[reads]] for provenance
```

`run` and `[[reads]]` paths are relative to the node directory.

## A caveat worth reading before copying this

The unknown-weapon precondition lists the valid names in `node.toml`, duplicating the `name`
column of `weapons.csv`. That is deliberate but not ideal: only the runtime can refuse, so a
node has no way to say "this question is outside my competence" — it can only crash, which
would be reported as a defect (exit 20) and is the wrong outcome for a reasonable question
about an unknown weapon.

This is fine at ten weapons and would not be at a thousand. The general fix is a
`weapon-lookup` node that resolves names, which is what the manifest's `not_for` and the
refusal message both point a caller toward.

## The allocation model

Simplified and stated in `optimize.py`'s docstring so the numbers mean something: a flat SL1
baseline of 10 in each stat, points to weapon requirements first, then a survivability floor
(vigor 27, endurance 20, vitality 20), then into scaling stats by descending coefficient,
stopping at 40, then 60, then 99. Attack rating is base damage plus each scaling stat's
contribution on a curve reaching 80% at 40 and 100% at 99.

It models DS3's real curves rather than reproducing them. That distinction matters for what
attestation can promise later: it can verify a number came from this computation, never that
the computation is a good model of the game (§6.3).
