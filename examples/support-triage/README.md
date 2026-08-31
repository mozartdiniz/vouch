# support-triage

Three nodes, two languages, one CSV. A ticket goes in, and depending on what the first node
finds, a different second node answers — or none does.

```
                             ┌── breached, paid tier ──→ escalation-cost  → credit_usd
  triage(ticket_id) ─────────┼── breached, free tier ──→ no answer: there is no credit
                             └── not breached ─────────→ wait-estimate    → wait_minutes
```

| Node | Language | Answers |
|---|---|---|
| `triage` | Python | Where does this ticket stand against its SLA? |
| `escalation-cost` | JavaScript | What does the missed SLA cost us? |
| `wait-estimate` | JavaScript | How much longer will this person wait? |

The runtime does not care which is which. A node is a subprocess that reads one JSON object
from stdin and writes one JSON object to stdout; `triage.py` and `cost.js` are the same kind
of thing to it. Neither has a dependency on `vouch`, and there is nothing to install.

## Who does the branching

**Not the tool.** `triage` does not call `escalation-cost`. Nodes never call each other, and
the runtime never picks a node for you.

What happens instead is that `triage` returns a fact — `breached: true`, `tier: "free"` — and
whoever is holding that result decides what to do with it. That "whoever" is an agent, a
shell script, or you. Each manifest publishes `use_when` and `not_for` to make the decision
an easy one:

```toml
# escalation-cost/node.toml
use_when = [ "triage reported breached = true and the tier is pro or enterprise" ]
not_for  = [ "tickets that are still within their SLA — call wait-estimate instead" ]
```

That guidance is advisory — it makes the right call *likely*. The preconditions below are
what make a wrong call *fail informatively*. The two work as a pair, and the second one is
what the guarantee rests on.

## Walking the branches

### The main path: a breached enterprise ticket

```console
$ vouch -C examples/support-triage call triage --input '{"ticket_id":"T-1001"}'
{
  "breached": true,
  "category": "outage",
  "minutes_open": 310,
  "minutes_remaining": -250,
  "sla_minutes": 60,
  "ticket_id": "T-1001",
  "tier": "enterprise"
}
```

Breached, and the tier is paid. So the credit question has an answer:

```console
$ vouch -C examples/support-triage call escalation-cost \
    --input '{"tier":"enterprise","minutes_over":250}'
{
  "credit_percent": 25,
  "credit_usd": 500,
  "minutes_over": 250,
  "monthly_fee_usd": 2000,
  "tier": "enterprise"
}
```

### The healthy path

`T-1002` comes back with `breached: false` and 145 minutes to spare, so the credit question
does not apply and the wait question does:

```console
$ vouch -C examples/support-triage call wait-estimate \
    --input '{"queue_depth":12,"agents_available":4,"avg_handle_minutes":20}'
{ "estimated_wait_minutes": 60, "rounds": 3, ... }
```

### The branch with no answer

`T-1006` is a free-tier ticket, 1440 minutes past a 1440-minute SLA. It is breached, so the
obvious next step is `escalation-cost`. There is no answer there:

```console
$ vouch -C examples/support-triage call escalation-cost --input '{"tier":"free","minutes_over":1440}'
{"outcome":"refusal","code":11,"node":"escalation-cost","reason":"the free tier has no SLA
commitment and therefore no credit; there is no figure to quote here"}
$ echo $?
11
```

This is the case the whole design exists for. A model asked "what do we owe on T-1006" will
happily produce a number, because a number is what the question shape calls for. The honest
answer is that no such number exists, and exit 11 with an empty stdout is how the runtime
says so. **No answer is a valid outcome.** The runtime is sound but incomplete: it never
returns an unsound answer, and it may return nothing.

### Routed to the wrong node

Ask `escalation-cost` about a ticket that never breached, and it does not just say no — it
says where to go:

```console
$ vouch -C examples/support-triage call escalation-cost --input '{"tier":"pro","minutes_over":-145}'
{"outcome":"refusal","code":11,"node":"escalation-cost","reason":"this ticket has not
breached its SLA, so there is no credit to compute — call wait-estimate instead"}
```

A refusal message is written for a reader who can act on it. An agent reads that and retries
correctly on the next turn; `invalid input` would have cost it a turn and taught it nothing.

### A precondition standing in for a crash

`wait-estimate` divides the queue by the number of agents. With zero agents that is a
division by zero, which would make the node return `Infinity` and get rejected as broken. A
precondition catches it one step earlier, where it is a question rather than a fault:

```console
$ vouch -C examples/support-triage call wait-estimate \
    --input '{"queue_depth":12,"agents_available":0,"avg_handle_minutes":20}'
{"outcome":"refusal","code":11,"node":"wait-estimate","reason":"no agents are on shift, so
there is no wait to estimate — that is a staffing question, not a queue one"}
```

Refusal, not defect. Nothing is wrong with the node; the question just has no answer.

## Postconditions worth copying

`triage` checks its own arithmetic against itself:

```toml
[[ensures]]
expr = "result.minutes_remaining == result.sla_minutes - result.minutes_open"

[[ensures]]
expr = "result.breached == (result.minutes_remaining < 0)"
```

The first catches the subtraction silently breaking. The second catches `breached` drifting
out of step with the number it is supposed to summarise — the kind of bug that survives code
review because both fields look right on their own.

Try it: in `triage.py`, change `minutes_remaining < 0` to `minutes_open > 0`. Every ticket
still returns plausible-looking JSON, and every call now fails with exit 13.

## Two details you may trip over

**`minutes_over` is not `minutes_remaining`.** `triage` reports `-250`; `escalation-cost`
wants `250`. The sign flip is deliberate, so that `input.minutes_over > 0` reads as the
plain-English claim it is. It is written down in the parameter guidance, which is where a
caller will look.

**`credit_usd` comes back as `500`, not `500.0`.** JavaScript does not distinguish them, so
the JSON says `500`. The output schema says `number`, and the runtime builds its contract
types from the schema rather than from the payload — so `result.credit_usd <=
result.monthly_fee_usd * 0.25` compares doubles, as written. Had the types been inferred from
the JSON instead, this contract would be comparing an integer against a float for no reason
the manifest could explain.

## The known rough edge

Ask `triage` for a well-formed ticket id that is not in the CSV:

```console
$ vouch -C examples/support-triage call triage --input '{"ticket_id":"T-9999"}'
{"outcome":"defect","code":20,"node":"triage","reason":"node exited with exit 1", ...}
```

Exit 20 — a *defect*, meaning "this node is broken". That is the wrong answer. Nothing is
broken; the ticket simply does not exist, which is a refusal.

It happens because only the runtime can refuse, and the runtime only sees the input. A
precondition cannot open `tickets.csv`, and a node has no channel for saying "outside my
competence" — it can only succeed or fail. Enumerating every valid id in a precondition works
at eight tickets and not at eight thousand.

This is a real limitation of the current design rather than an oversight in the example, and
it is worth understanding before you build a collection that depends on lookups.

## Layout

```
support-triage/
  nodes/
    triage/
      node.toml
      triage.py
      data/tickets.csv     declared under [[reads]] for provenance
    escalation-cost/
      node.toml
      cost.js
    wait-estimate/
      node.toml
      wait.js
```

Each node's `run` command and `[[reads]]` paths are relative to its own directory. Schemas
here are written inline in `node.toml`; `../ds3-tools` uses separate `.schema.json` files
instead. Both work.
