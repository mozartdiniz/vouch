# Examples

Three collections, in the order worth reading them. None is part of the runtime — every node
here is an ordinary script that the `vouch` binary runs as a subprocess.

Each directory is a self-contained collection. Run commands from inside one, or point the
tool at it with `-C`:

```console
$ vouch -C examples/hello-world list
```

## [`hello-world`](hello-world) — one node, two files

Counts the letters in a word. Start here.

It exists because "how many r's are in strawberry" is the shortest demonstration of the
problem `vouch` solves: a model answering that question is not counting, and `count-letters`
is. Covers the node protocol, one precondition, and three postconditions — including the one
shape worth imitating, a postcondition that ties the output back to the input.

## [`support-triage`](support-triage) — three nodes, two languages, a CSV

A support ticket goes into a Python node that reads a CSV; depending on what comes back, one
of two JavaScript nodes answers — or neither does.

```
                       ┌── breached, paid tier ──→ escalation-cost  → credit_usd
  triage(ticket_id) ───┼── breached, free tier ──→ no answer: there is no credit
                       └── not breached ─────────→ wait-estimate    → wait_minutes
```

This is the one to read for how a collection fits together. It shows that Python and
JavaScript nodes are indistinguishable to the runtime, that **the caller does the branching**
and nodes never call each other, and — most usefully — what it looks like when a branch
correctly ends in no answer at all.

It is also honest about a limitation: a ticket id that is well-formed but missing from the
CSV surfaces as a defect rather than a refusal, and the README explains why.

## [`ds3-tools`](ds3-tools) — one node, real arithmetic

Stat allocation for a Dark Souls 3 build. Denser than the other two, and closer to what a
real collection looks like: separate `.schema.json` files, a `$ref` in the output schema, and
enough arithmetic that the postconditions are earning their keep.

Read it after the first two, for the acceptance walkthrough — a working call, a refusal, and
a deliberately broken calculation caught by its postcondition.

## [`ask.py`](ask.py) — a language model driving the collections

The smallest realistic agent loop. Ask in English; get an answer computed by nodes, or an
honest "I don't know".

```console
$ ./ask.py -C support-triage "what do we owe on ticket T-1001?"
```

It needs no API key — it shells out to `claude -p`, which uses your existing Claude Code
login. Point `VOUCH_LLM` at any command that takes a prompt to use something else.

It gets its bearings from one command — `vouch describe --all --json`, the same routing
context `describe --all --md` renders for a `CLAUDE.md`, including the collection's own
`.vouch/registry.toml` preamble. So "call triage first for any ticket question" reaches the
model as published context rather than as a rule buried in this README.

The model does two jobs, and neither is arithmetic. It picks which node to call and builds
that node's arguments, one call at a time, using the verified results of earlier calls. Then
it turns those results into a sentence. Every number in the answer came out of a node.

**What the loop is really demonstrating** is that the three exit-code families each drive a
different behaviour — which is the whole reason they are distinct:

| Outcome | Codes | What the loop does |
|---|---|---|
| Success | `0` | Record the result; let the model narrate from it |
| Refusal | `11` `14` `15` | Hand the reason back as a correction; retry or concede |
| Defect | `12` `13` `20` `21` | Stop. Report the broken node. Never answer anyway |

Its own exit codes mirror that: `0` answered, `1` no answer available, `2` a node is broken.

### Chaining, driven by the model

`triage` reads the CSV; the model reads `triage`'s output and decides what to call next.
Nodes never call each other.

```console
$ ./ask.py -C support-triage "what do we owe on ticket T-1001?"
→ calling triage({"ticket_id": "T-1001"})
  exit 0, contracts held: {"breached": true, ..., "minutes_remaining": -250, "tier": "enterprise"}
→ calling escalation-cost({"tier": "enterprise", "minutes_over": 250})
  exit 0, contracts held: {"credit_percent": 25, "credit_usd": 500, "monthly_fee_usd": 2000, ...}
→ done; narrating from verified results

T-1001 (enterprise tier, outage) has breached its 60-minute SLA — it's been open 310 minutes,
which is 250 minutes over. The escalation cost is a 25% credit on the $2000 monthly fee, or $500.
```

Note the `250`: the model got `minutes_remaining: -250` and flipped the sign, because the
parameter guidance told it to. It did not invent the figure.

### A refusal as a course correction

```console
$ ./ask.py -C ds3-tools "what stats should I level for a lothric sword build at soul level 120?"
→ calling stat-optimizer({"weapon": "Lothric Sword", "soul_level": 120})
  exit 11 (refusal): unknown weapon; this node covers only: Lothric Knight Sword, Uchigatana, ...
  feeding the correction back to the model
→ calling stat-optimizer({"weapon": "Lothric Knight Sword", "soul_level": 120})
  exit 0, contracts held: {"attack_rating": 190.9, ...}
```

This is what §4.2 is for. `invalid weapon` would have cost a turn and taught it nothing; a
message naming the valid spellings gets a correct retry.

### Two ways to reach "I don't know"

Sometimes the routing context is enough and no node ever runs:

```console
$ ./ask.py -C support-triage "should we hire more support agents next quarter?"
→ stopping: no answer available
I don't know. None of these nodes answer staffing or headcount questions — they only handle
per-ticket SLA status, breach credits, and wait estimates.
```

And sometimes the question looks perfectly answerable right up until it isn't:

```console
$ ./ask.py -C support-triage "what do we owe on ticket T-1006?"
→ calling triage({"ticket_id": "T-1006"})
  exit 0, contracts held: {"breached": true, "minutes_remaining": -1440, "tier": "free", ...}
→ stopping: no answer available
I don't know. T-1006 is a free-tier ticket, which carries no SLA credit, so no node can tell
me what is owed on it.
```

The ticket really has breached, by a full day. A model asked this without tools will produce
a number, because the question shape calls for one. There is no such number.

### A broken node is not a refusal

```console
$ ./ask.py -C support-triage "what is the SLA status of ticket T-9999?"
→ calling triage({"ticket_id": "T-9999"})
  exit 20 (defect): node exited with exit 1
I can't answer that. The `triage` node is broken — node exited with exit 1. That needs
reporting, not retrying.
```

No retry, no workaround, no answer. (This is the rough edge `support-triage`'s README
describes: an absent ticket *should* be a refusal, and today it is not.)

### The answer checks itself

Writing the sentence is the one step where the model produces figures of its own accord, and
so the one place a number could still be invented. Before printing anything, the loop runs
`vouch attest` over its own prose:

```console
→ done; narrating from verified results
→ attested: every figure traces to a verified result

Ticket T-1001 (enterprise tier, outage) has breached its 60-minute SLA — it's been open 310
minutes, which is 250 minutes over. What we owe is a 25% credit on the $2000 monthly fee, or $500.
```

Every numeral there — 1001, 60, 310, 250, 25, 2000, 500 — was checked against the ledger with
no model involved. Had one been wrong, the loop would have discarded the sentence and said
"I don't know" rather than show you an answer it could not stand behind.

### One thing this still does not guarantee

**Provenance is about outputs, not inputs.** `vouch` guarantees a returned value came from a
function that satisfied its contracts. It cannot know whether the *arguments* were right. If
the model passed `minutes_over: 200` instead of `250`, the credit would be computed correctly
from a wrong premise, and it would attest cleanly — because the figure really did come from a
node.

The loop avoids this by calling `triage` for the real number rather than guessing, and the
ledger records exactly what was passed so it can be audited after the fact. But the discipline
lives in how nodes are designed ("move fetches outward"), not in an enforcement mechanism.

## What they demonstrate, at a glance

| | hello-world | support-triage | ds3-tools |
|---|---|---|---|
| Nodes | 1 | 3 | 1 |
| Languages | Python | Python + JavaScript | Python |
| Reads a data file | | ✓ | ✓ |
| Inline schemas | ✓ | ✓ | |
| Separate schema files | | | ✓ |
| Refusal as a routing correction | ✓ | ✓ | ✓ |
| A branch that ends in no answer | | ✓ | |
| Caller-side branching between nodes | | ✓ | |

[`ask.py`](ask.py) runs against all three.
