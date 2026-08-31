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

Milestone 1: registry, manifests, schema validation, CEL contracts, subprocess execution, the
exit-code taxonomy, and `list` / `describe` / `call`. The ledger, `attest`, `test`, `eval`,
and the markdown routing pack are not built yet.

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
| [`ds3-tools`](examples/ds3-tools) | one node, real arithmetic | separate schema files, `$ref`, the acceptance walkthrough |

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
4. **Give each returned value a stable key.** Attestation and evals will both depend on it.
5. **Move fetches outward where practical.** A node that takes a rate as a declared parameter
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
  registry.rs          collection discovery
  manifest.rs          node.toml parsing, contract-strength gate
  schema.rs            JSON Schema validation
  contracts.rs         CEL evaluation, fail-closed
  exec.rs              subprocess, timeout, stdio protocol boundary
  commands.rs          list / describe / call

tests/               the runtime's own tests
  exit_codes.rs        one test per exit code
  directory.rs         the -C flag
  examples.rs          the examples still do what their READMEs say
  fixtures/nodes/      throwaway nodes, each broken in one specific way

examples/            complete collections, for reading and copying
  hello-world/         one Python node
  support-triage/      three nodes across Python and JavaScript, over a CSV
  ds3-tools/           Dark Souls 3 build optimization
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
