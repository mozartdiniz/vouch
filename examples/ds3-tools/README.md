# ds3-tools

An example `vouch` collection: build optimization for Dark Souls 3 over a local CSV dataset.

This is a demonstration of how to write nodes, not part of the runtime. Nothing here is
compiled into the `vouch` binary — the nodes are Python scripts the runtime executes as
subprocesses.

It is the collection the spec's acceptance demo uses, because nearly every output is a
checkable scalar: stat spreads, attack ratings, points spent.

## Run it

From this directory:

```console
$ vouch list
stat-optimizer  Optimal stat allocation for a target weapon and soul level in Dark Souls 3
weapon-lookup   Resolve a partial or misspelled weapon name to its exact Dark Souls 3 spelling

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
{"outcome":"refusal","code":11,"node":"stat-optimizer","reason":"unknown weapon; call
weapon-lookup with the name as the user wrote it and pass back what it resolves — do not pick
one of these yourself. This node covers only: Lothric Knight Sword, Uchigatana, ..."}
$ echo $?
11
```

**A question that has two answers, and therefore none.** `weapon-lookup` is where that refusal
sends you, and "lothric sword" is exactly the kind of query it exists for:

```console
$ vouch call weapon-lookup --input '{"query":"lothric sword"}'
{
  "ambiguous": true,
  "candidates": [ "Lothric Knight Sword", "Lothric's Holy Sword" ],
  "catalog_size": 10,
  "match_count": 2,
  "query": "lothric sword",
  "resolved": ""
}
```

`resolved` is empty. Two weapons match, so there is no name to hand to `stat-optimizer`, and
the collection's preamble tells the agent to put the choice to the user rather than take the
first candidate. That silent pick — observed for real, and recorded under "Provenance covers
outputs, not inputs" in `DECISIONS.md` — is the failure this node exists to prevent.

It is enforced rather than requested. Edit `lookup.py` to return `candidates[0]` whenever
there is one, and the postcondition catches it:

```console
$ vouch call weapon-lookup --input '{"query":"lothric sword"}'
{"outcome":"defect","code":13,"node":"weapon-lookup","reason":"postcondition failed:
(result.resolved != \"\") == (result.match_count == 1)"}
$ echo $?
13
```

A node that guesses is a broken node, and no value reaches stdout. `"uchi"` still resolves to
`Uchigatana`, and an exact name resolves even when it is a substring of others: matching tries
exact equality first, which is the one unambiguous way to settle the question.

Matching nothing is a real answer too — `{"query":"moonblade"}` returns `match_count: 0`,
which says the weapon is outside this dataset. That is a fact about the data, not an error.

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
  .vouch/
    registry.toml          collection-level context: call weapon-lookup first, never choose
    evals.toml             routing evals — natural-language questions and what must happen
  data/
    weapons.csv            the dataset, shared by both nodes
  nodes/
    stat-optimizer/
      node.toml            manifest: routing context, contracts, interface
      input.schema.json
      output.schema.json
      optimize.py          the node: JSON on stdin, JSON on stdout
      cases.toml           fixtures: fixed input, expected exit code, expected values
    weapon-lookup/
      node.toml
      input.schema.json
      output.schema.json
      lookup.py
      cases.toml
```

`run` and `[[reads]]` paths are relative to the node directory, which is why both nodes
declare `../../data/weapons.csv`. The dataset sits at collection level rather than inside
either node: a second copy is a second thing to drift, and a lookup that resolves names
against different bytes than the optimizer computes from would be worse than no lookup.

## A caveat worth reading before copying this

The unknown-weapon precondition still lists the valid names in `stat-optimizer/node.toml`,
duplicating the `name` column of `weapons.csv`. `weapon-lookup` fixes the routing half of the
problem — a partial name now resolves, and an ambiguous one refuses to resolve — but not this
half.

The reason is structural: only the runtime can refuse, and it sees only the input, so a node
has no way to say "this question is outside my competence". `optimize.py` handed a weapon that
is not in the CSV can only crash, which is reported as a defect (exit 20) — the wrong outcome
for a reasonable question about an unknown weapon. Enumerating the names in a precondition is
what keeps that from happening.

Fine at ten weapons, wrong at ten thousand. Removing the duplication needs a refusal channel
from the node itself, which is an open runtime question rather than a collection one; see
"A node cannot refuse" in `DECISIONS.md`.

## Testing it

Both layers, and the difference between them is easiest to see here:

```console
$ vouch test
stat-optimizer
  ok    the documented Lothric build
  ...
14 cases, 14 passed, 0 failed

$ vouch eval --agent "claude -p {prompt}" -n 10 --min-rate 0.9
```

`vouch test` is boolean and free: it is what catches the allocation math silently breaking,
which is invisible from the outside because a wrong stat spread looks exactly like a right one.

`vouch eval` puts a model in front of the collection and reports a rate. The case worth
watching is `what should I level for a lothric sword build?`, which asserts `expect_stop`: two
weapons match, so the honest end is to put the choice back to the user. An agent that quietly
answers about the Lothric Knight Sword fails it, and that failure is the reason the case exists.

## The allocation model

Simplified and stated in `optimize.py`'s docstring so the numbers mean something: a flat SL1
baseline of 10 in each stat, points to weapon requirements first, then a survivability floor
(vigor 27, endurance 20, vitality 20), then into scaling stats by descending coefficient,
stopping at 40, then 60, then 99. Attack rating is base damage plus each scaling stat's
contribution on a curve reaching 80% at 40 and 100% at 99.

It models DS3's real curves rather than reproducing them. That distinction matters for what
attestation can promise later: it can verify a number came from this computation, never that
the computation is a good model of the game (§6.3).
