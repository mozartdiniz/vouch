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
