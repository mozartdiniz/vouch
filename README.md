# vouch

A local, language-agnostic runtime for executing contract-checked functions. Driven by an
LLM agent, fully usable from a plain shell.

**The property it guarantees is value provenance.** Every number in an answer originates from
a function's return value rather than from a model. Nodes are allowed to do I/O — read files,
query databases, make HTTP calls. What is enforced is that the result passed its contracts,
and that its origin is recorded.

The runtime is **sound but incomplete**. It never returns an unsound answer. It may return
nothing. Every call ends in exactly one of: a value that satisfied its contracts, or a refusal
with a machine-readable reason. There is no partial or best-effort result.

## Status

**The MVP is complete.** Registry, manifests, schema validation, CEL contracts, subprocess
execution, the full exit-code taxonomy, `list` / `describe` / `call`, the routing pack, the
ledger, `vouch attest`, and both testing layers — `vouch test` and `vouch eval`.

Three example collections and a small agent loop ([`examples/ask.py`](examples/ask.py)) sit on
top of it. The spec's six-step acceptance demo has been run end to end against a real model —
`DECISIONS.md` records what it produced and the two defects it found.

[`MVP_Spec.md`](MVP_Spec.md) is what the project set out to build.
[`DECISIONS.md`](DECISIONS.md) records where the implementation departs from it and why, the
limitations that are known and open, and **where to pick the work up** — read that before
extending anything.

## Install

```console
$ cargo install --path .
```

This builds in release mode and puts `vouch` in `~/.cargo/bin`. If that directory is not on
your `PATH`, add it:

```console
$ echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.zshrc
```

`cargo build --release` alone is not enough to run `vouch` by name — it leaves the binary at
`target/release/vouch` and does not touch your `PATH`. Either install it, or invoke it by
path. `cargo uninstall vouch` removes it.

## Try it

`vouch` operates on a *collection* — a directory containing `nodes/`. This repository is the
runtime, not a collection, so point the tool at an example:

```console
$ vouch -C examples/hello-world call count-letters \
    --input '{"word":"strawberry","letter":"r"}'
{
  "count": 3,
  "letter": "r",
  "word": "strawberry",
  "word_length": 10
}
```

Ask a language model how many r's are in "strawberry" and you will often get 2 — not because
it counts badly, but because it isn't counting. The 3 above came from a four-line Python
script, and the runtime checked it against the node's postconditions before printing it.

Ask for something outside a node's competence and you get a refusal you can act on, rather
than a plausible guess:

```console
$ vouch -C examples/support-triage call escalation-cost --input '{"tier":"free","minutes_over":1440}'
{"outcome":"refusal","code":11,"node":"escalation-cost","reason":"the free tier has no SLA
commitment and therefore no credit; there is no figure to quote here"}
$ echo $?
11
```

Nothing was written to stdout. `vouch call` puts the result object on **stdout** and
everything else on **stderr**, so a caller reading stdout gets a value or gets nothing.

## Examples

[`examples/`](examples/) has three collections, in the order worth reading them:

| | | |
|---|---|---|
| [`hello-world`](examples/hello-world) | one node, two files | the node protocol and one good postcondition |
| [`support-triage`](examples/support-triage) | three nodes, Python + JavaScript, a CSV | how a collection fits together, and what a branch that ends in *no answer* looks like |
| [`ds3-tools`](examples/ds3-tools) | two nodes, real arithmetic | separate schema files, `$ref`, the acceptance walkthrough, and a lookup that refuses to resolve an ambiguous name |

Plus [`ask.py`](examples/ask.py), a small agent loop that drives any of them from plain
English — and says "I don't know" when the answer does not exist:

```console
$ ./examples/ask.py -C support-triage "what do we owe on ticket T-1006?"
→ calling triage({"ticket_id": "T-1006"})
  exit 0, contracts held: {"breached": true, "minutes_remaining": -1440, "tier": "free", ...}
→ stopping: no answer available
I don't know. T-1006 is a free-tier ticket, which carries no SLA credit, so no node can tell
me what is owed on it.
```

The ticket really has breached, by a full day. Ask a model that without tools and you will
get a number, because the question shape calls for one. There is no such number.

## Exit codes

Three families. The distinction between them is the point.

| | Code | Meaning |
|---|---|---|
| **Success** | `0` | Value returned, all contracts held |
| **Refusal** — no answer is available; try a different approach | `11` | Precondition failed (outside this node's competence) |
| | `14` | Execution timeout |
| | `15` | A contract could not be evaluated (fail-closed) |
| **Defect** — the node is broken; stop trusting it and report it | `12` | Output failed its JSON Schema |
| | `13` | Postcondition failed |
| | `20` | Node crashed or exited non-zero |
| | `21` | Protocol violation (stdout was not a single JSON object) |
| **Caller error** | `10` | Input failed its JSON Schema |
| **Error** — a problem with the collection, not the call | `1` | Node missing, manifest unparseable, contract won't compile |

Every non-zero exit emits one JSON object on stderr:

```json
{ "outcome": "refusal", "code": 11, "node": "stat-optimizer", "reason": "..." }
```

`vouch attest`, `vouch test` and `vouch eval` share their own convention, because a finding is
not a failure of the command: `0` everything checked out, `1` something did not, `2` the check
could not be run at all. Keeping those apart matters most for a checking tool — a run that
could not happen must never look like a run that found nothing wrong.

## Telling an agent what a collection can do

A collection describes itself in one command:

```console
$ vouch describe --all --md
```

That emits markdown you paste into a `CLAUDE.md`, or that an agent runs at session start:
each node's purpose, when to reach for it, when *not* to, its parameters with types and
guidance, and worked examples — plus a short note on how to call a node and what each exit
family means. That is the whole integration story. There is no protocol and no config format
to learn.

Contracts are deliberately left out. They are enforcing rather than advisory, and a caller
does not need to read a precondition to make a good call — if they get it wrong the refusal
says so in words written to be acted on.

### The registry preamble

Some things are true of a collection rather than of any one node. Those go in
`.vouch/registry.toml`, and lead the pack:

```toml
name = "support-triage"
description = "SLA status, breach credits and queue waits, computed over a local CSV."

notes = [
  "Call triage first for any question about a specific ticket. The other nodes take their arguments from what it returns.",
  "Only pro and enterprise plans carry an SLA commitment. A free-tier breach has no credit to compute — say so rather than quoting zero.",
]
```

"Call triage first" belongs to no single node's `use_when`. Without a preamble it lives only
in a README that no agent reads. The file is optional; a collection of well-described nodes
works without one.

`describe --all --json` gives the same material to a program — [`examples/ask.py`](examples/ask.py)
uses it to build its context in one call.

## The ledger and attestation

The runtime can guarantee its own output. It cannot guarantee what gets written afterwards —
a transposed digit, a wrong unit, a total the model helpfully recomputed. All the contract
work sits upstream of where fabrication actually happens.

Every call appends one line to `.vouch/ledger/session-<id>.jsonl`, whatever the outcome:

```json
{ "ts": "2026-08-31T13:02:54Z", "node": "stat-optimizer", "version": "0.1.0",
  "input": { "weapon": "Lothric Knight Sword", "soul_level": 120 },
  "reads": [ { "path": "../../data/weapons.csv", "sha256": "8933b2cd…", "bytes": 596 } ],
  "outcome": "ok", "code": 0,
  "result": { "attack_rating": 190.9, "stats": { "strength": 42 } },
  "scalars": { "result.attack_rating": 190.9, "result.stats.strength": 42 } }
```

`reads` is the audit record — which bytes produced this number — not a cache key. `scalars`
is every numeric leaf of the result, flattened, which is what attestation checks against.

Then pipe the prose in. **No model is involved**; it is string and number reconciliation:

```console
$ vouch attest --text "$ANSWER" --question "$QUESTION"
ledger: .vouch/ledger/session-demo.jsonl (1 entry, 11 scalars)
clean: 11 numerals checked, 11 matched, 0 ignored

$ vouch attest --text "${ANSWER/190.9/190.8}" --question "$QUESTION"
UNATTESTED: 1 of 11 numerals did not come from the ledger

  line 1, column 290: 190.8
    …t spread yields an attack rating of 190.8.
$ echo $?
1
```

`512` matches a recorded `512.4`, since rounding is not fabrication. `1,234.5` matches
`1234.5`, and `12.3%` matches both `12.3` and `0.123`. Numbers from the user's own question,
bare years, and small bare integers are excused — but only *after* matching has been tried,
so an ignore rule can never suppress a figure that genuinely came from a node.

Inputs are recorded but do not count as verified unless you pass `--include-inputs`. An agent
chose them, so treating them as provenanced would launder a fabricated argument into an
attested figure.

Set `VOUCH_SESSION` once per conversation. A smaller ledger is a stricter check.

### What it looks like on a real answer

Asked a build question with nothing but the routing pack in its `CLAUDE.md`, Claude Code called
one node and wrote a stat spread from what came back:

```console
$ vouch -C examples/ds3-tools attest --ledger .vouch/ledger/session-acceptance-step4.jsonl \
    --text @answer.txt --question @question.txt
ledger: .vouch/ledger/session-acceptance-step4.jsonl (1 entry, 11 scalars)
clean: 11 numerals checked, 11 matched, 0 ignored
```

The same question to a bare model, no tools, produced a longer and more confident answer — a
full stat table, four infusion comparisons, attack ratings of 395, 425, 430 and "past 500".
Against the same ledger, **20 of its 36 numerals could not be accounted for**.

Read that carefully, because the honest version is narrower than the exciting one: those
figures are not necessarily *wrong about Dark Souls 3*. `weapons.csv` is a simplified model. The
difference is that one answer can be checked and the other cannot — the bare one closes with
"AR figures are from memory, ±5", which is the model describing its own position accurately and
is the sentence every reader skips.

### What it does not check

Attestation covers scalars. It verifies that the numbers are real, not that the advice is
good. A workout plan's sets, reps and volume totals attest fine; "is this a good program" is
a judgement no ledger can settle. Never let a green check imply more than it checked.

The same limit applies to everything above: a green check says every figure came from a
function. It never says the function is a good model of the world.

## Testing a collection

Two layers, and conflating them is the mistake worth avoiding: one is boolean, the other is a
rate.

### `vouch test` — node fixtures

`cases.toml` beside a node. Fixed input, expected exit code, expected values. No model, no
network, nothing to average:

```toml
[[case]]
name = "the documented Lothric build"
input = { weapon = "Lothric Knight Sword", soul_level = 120 }
expect = { "result.attack_rating" = 190.9, "result.stats.dexterity" = 60 }

[[case]]
name = "an impossible soul level is refused"
input = { weapon = "Uchigatana", soul_level = 9999 }
expect_code = 11
```

```console
$ vouch -C examples/ds3-tools test
stat-optimizer
  ok    the documented Lothric build
  ok    an impossible soul level is refused
  ...

14 cases, 14 passed, 0 failed
```

Paths in `expect` are rooted at `result` and spelled exactly as the ledger spells its scalars,
so one path grammar covers a fixture, a ledger entry and an attestation report. A float
expectation is checked **to the precision it was written to** — `543.1948141` passes against
`543.1948140689826` — the same rule attestation gives prose. Expectations get copied from
wherever the ground truth lives, a spreadsheet cell or another implementation's printout;
demanding that such a figure also reproduce IEEE noise tests the transcription, not the node.
Write more digits to demand more. An integer expectation stays exact. A case runs the
identical pipeline a real call runs — schema, contracts, subprocess, schema, contracts — minus
the ledger, because a fixture is a rehearsal and not a call anyone may quote a number from. A
case that expects a non-zero exit may not also expect values; there is no result to read them
from, and saying so when the file loads beats a puzzling failure later.

Two things deliberately fail rather than passing quietly: a node that will not load is reported
as a failure of the collection rather than skipped, and a run that found no `cases.toml` at all
exits 2 instead of reporting success. A checking tool that passes having checked nothing is the
same false assurance the contract-strength gate exists to prevent.

### `vouch eval` — agent routing

Natural-language question in; assert the right node was called with the right parameters and
that the prose written from the results attests clean. There is a model in the loop, so this is
a **pass rate**, not a pass:

```toml
[[eval]]
ask = "what should I level for a Lothric Knight Sword build at SL120?"
expect_node = "stat-optimizer"
expect_params = { weapon = "Lothric Knight Sword", soul_level = 120 }
attest = true

# Two weapons match "lothric sword", so the honest end is to put the choice back to the user.
[[eval]]
ask = "what should I level for a lothric sword build?"
expect_node = "weapon-lookup"
expect_stop = true
```

```console
$ vouch -C examples/ds3-tools eval --agent "claude -p {prompt}" -n 10 --min-rate 0.9
suite: .vouch/evals.toml (2 cases, 10 runs each)

what should I level for a Lothric Knight Sword build at SL120?
  10/10 runs passed
    [answered: stat-optimizer]
what should I level for a lothric sword build?
  8/10 runs passed
    [stopped: weapon-lookup]
    [answered: weapon-lookup → stat-optimizer]
    expected the agent to decline; it answered

48/50 runs passed (96%); the floor is 90%
```

Each case prints as it finishes rather than at the end: a suite is minutes of model calls, and
a run that has to be waited out in silence is one nobody interrupts when the first case is
already going wrong. `--json` still emits one document.

If the agent stops working partway — a usage limit, a dropped connection — the run fails with
exit 2 and no rate, because a partial rate is not a rate. It still reports the cases that
finished and how far it got, since those cost real model calls, and it surfaces whatever the
agent said even when the agent said it on stdout.

The suite lives in `.vouch/evals.toml`, beside the preamble, because routing is a property of
the collection rather than of any one node. `--agent` takes any command that accepts a prompt
and prints a reply — `{prompt}` is substituted where it appears, and appended as a final
argument when it does not — so the runtime never depends on a particular harness.

This is the regression signal on the failure mode that is otherwise invisible: reword a
`use_when`, watch routing accuracy fall from 95% to 60%, and without an eval nobody finds out
until a user does. The report names the route each run took, not only the rate, because a rate
that has dropped is unactionable without knowing where the agent went instead.

`expect_stop` asserts that the agent declined. It is not in the spec, and it is the case this
project cares most about: the question whose honest answer is "there isn't one".

Run against Claude Code at five runs per case, the three example suites currently score 10/10,
25/25 and 25/25. The first real run of `support-triage` scored 4/5, and the failure was worth
having: the agent had negated a number rather than quoting one. That story is in `DECISIONS.md`
under "The acceptance run", and the fix — a node returning the same fact in the form a caller
will actually quote — is the most useful thing either testing layer has produced.

The eval's calls are held in memory rather than written to `.vouch/ledger/`. An eval is a
rehearsal too, and a figure that only ever appeared in one must never be able to account for a
numeral in a real answer later.

Each run costs tokens, so `vouch eval` is not part of `cargo test`.

## Writing a node

A node is a subprocess: one JSON object in on stdin, one JSON object out on stdout, in any
language. Contract enforcement lives entirely in the runtime, so the guarantee holds
regardless of how the node is implemented. You can vibe-code the inside of the box. The box is
not vibe-coded.

Rules, in the order people violate them:

1. **Return computed results, never raw rows.** If a node hands back 400 rows and lets the
   model do the arithmetic, every guarantee evaporates at the last step. The function does the
   math; the model only narrates. This is the central rule.
2. **stdout is the payload channel.** Logs, prints, and progress go to stderr. Every
   language's default logger writes to stdout, and a stray log line is a protocol violation
   (exit 21), not a parse error you get to debug.
3. **Emit full precision with explicit units**, and prefer a small flat result object over a
   nested blob. Every hop the model makes through your output is a chance to fabricate.
4. **Give each returned value a stable key, and give every figure a numeric one.** A number
   inside a string is invisible to the ledger, which records numeric leaves — a node returning
   `"split": "409/0/411/0/0"` has published four figures that attestation cannot account for,
   so a caller quoting one gets flagged for a number the node really did produce. Keep the
   display string if it is how the game writes it, and return the numbers beside it.
5. **Return every figure in the form a reader will quote it in.** If the game shows a
   truncated `259` and you return `259.576275`, the caller does the truncating, and a figure
   a model computed is a figure no node produced. The same goes for a difference the reader
   obviously wants: return it. Every small sum you leave to the caller is handed to a model,
   and that is where fabrication lives.
6. **Move fetches outward where practical.** A node that takes a rate as a declared parameter
   is more auditable than one that silently uses whatever the market was doing at call time.
   A preference, not a rule.

### Contracts must not be vacuous

The runtime **refuses to load** a node whose `ensures` list is empty, or whose postconditions
never reference `result`. A postcondition like `result != null` gives an agent *more*
confidence than no postcondition while checking nothing, and false assurance is worse than
absent assurance.

`vouch describe` reports contract strength — how many postconditions there are, and which
numeric output fields none of them mention — so a consumer can weight the answer:

```
Contract strength: 3 postconditions, covering 3/11 numeric output fields
  unchecked: result.soul_level, result.stats.attunement, ...
```

### Contracts fail closed

`requires` sees `input` and runs before the subprocess. `ensures` sees `input` and `result`
and runs after it exits.

If a CEL expression *errors* rather than returning a boolean — undefined field, type mismatch,
division by zero — that is a failure, not a pass. A contract that cannot be evaluated is a
contract that did not hold, and the call refuses with exit 15. An expression returning a
non-boolean is treated the same way; truthiness is never a verdict.

CEL types come from the JSON Schema, not from the incoming payload, so `integer` and `number`
mean what the schema says they mean and a payload that disagrees is a schema violation
(exit 10) rather than a mysterious contract failure.

### Manifest

See `examples/ds3-tools/nodes/stat-optimizer/node.toml` for a complete example. Contracts take
either form:

```toml
ensures = ["result.attack_rating > 0.0"]
```

```toml
[[requires]]
expr = "input.soul_level >= 1 && input.soul_level <= 802"
message = "soul level must be between 1 and 802"
```

Prefer the second for preconditions. **A failed precondition is a routing correction** — the
message is written for a reader who can act on it. `unknown weapon 'lothric sword'; call
weapon-lookup for valid names` gets a correct retry; `invalid weapon` does not.

Because TOML puts subsequent bare keys inside the last-opened table, `[[requires]]` and
`[[ensures]]` sections go at the end of the manifest.

## What a collection looks like

```
my-tools/
  nodes/
    stat-optimizer/
      node.toml            manifest: routing context, contracts, interface
      input.schema.json
      output.schema.json
      optimize.py          the node: JSON on stdin, JSON on stdout
      data/weapons.csv     declared under [[reads]] for provenance
```

`vouch` finds the collection by walking up from the working directory looking for `nodes/` or
`.vouch/`. A node's `run` command and its `[[reads]]` paths are relative to the node
directory.

`-C <DIR>` runs as if vouch had been started in `<DIR>`, matching `git -C`. It is a real
change of working directory, not just a discovery hint, so relative paths on the rest of the
command line resolve from there too:

```console
$ vouch -C examples/ds3-tools call stat-optimizer --input @build.json
```

reads `examples/ds3-tools/build.json`.

## What this repository looks like

The runtime and the things that merely *use* it are kept apart:

```
src/                 the runtime — the only thing compiled into the binary
  error.rs             exit-code taxonomy, structured stderr
  registry.rs          collection discovery, the .vouch/registry.toml preamble
  manifest.rs          node.toml parsing, contract-strength gate
  schema.rs            JSON Schema validation
  contracts.rs         CEL evaluation, fail-closed
  exec.rs              subprocess, timeout, stdio protocol boundary
  verify.rs            one attempt at a call: the pipeline, minus ledger and stdout
  ledger.rs            append-only call record, scalar flattening, reads hashing
  attest.rs            numeral extraction and reconciliation — no model involved
  markdown.rs          the routing pack
  cases.rs             cases.toml — node fixtures
  eval.rs              the agent loop, the suite, and how a run is judged
  commands.rs          list / describe / call / test / eval / attest

tests/               the runtime's own tests
  exit_codes.rs        one test per exit code
  ledger_attest.rs     the ledger, and prose reconciliation
  routing_pack.rs      the preamble and the markdown pack — mostly what it omits
  directory.rs         the -C flag
  examples.rs          the examples still do what their READMEs say
  node_cases.rs        vouch test — including what it refuses to call a pass
  agent_evals.rs       vouch eval, driven by fake agents so no model is needed
  fixtures/nodes/      throwaway nodes, each broken in one specific way
  agents/              fake agents: fixed replies, one per behaviour worth measuring

examples/            complete collections, for reading and copying
  hello-world/         one Python node
  support-triage/      three nodes across Python and JavaScript, over a CSV
  ds3-tools/           Dark Souls 3 build optimization, and name resolution for it
  ask.py               an agent loop that drives any of them
```

Nothing under `examples/` is compiled into the binary or required by the test suite; the
nodes there are ordinary subprocesses in whatever language their author chose. Nothing under
`tests/fixtures/` is meant to be copied — those nodes exist to fail.

## Tests

```console
$ cargo test
```

`tests/exit_codes.rs` pins the whole taxonomy against the fixture nodes. Two properties are
asserted throughout: a non-zero exit never writes a value to stdout, and every non-zero exit
emits one structured JSON object on stderr.

Those tests run against `tests/fixtures/`, which is itself a collection — the repository root
deliberately is not one, so a test that passes proves the runtime found what it was pointed
at rather than stumbling into it.

`tests/examples.rs` covers the example collections instead, asserting the exit codes, refusal
messages, and specific numbers their READMEs quote. Documentation that nothing executes is
documentation that drifts. The JavaScript nodes are skipped with a note if `node` is not
installed.

The example collections also carry their own `cases.toml`, and `cargo test` runs them: a
regression in an example node fails the runtime's test suite. `vouch eval` needs a model and
costs tokens, so what `cargo test` covers there is the machinery — the loop, the corrections,
the attestation of the final prose — driven by the fixed-reply agents in `tests/agents/`, plus
a check that every committed eval suite still loads and names nodes that exist.
