# Decisions

`MVP_Spec.md` says what to build. The READMEs say how to use it. This file says **why the
built thing differs from the spec, and what is still unresolved** — the material that
otherwise survives only in commit messages and in whoever was in the room.

Every entry is either a decision that has been made and acted on, or an open question that
has not. Nothing here is aspirational.

---

## Status

| Milestone | State |
|---|---|
| M1 — registry, schemas, contracts, exec, exit codes, `list`/`describe`/`call` | **Done** |
| M2 — routing context fields, `describe --all --md` | Not started |
| M3 — ledger, scalar flattening, `vouch attest` | Not started |
| M4 — `vouch test`, `vouch eval` | Not started |
| M5 — demo collection, README, six-step acceptance run | Partly done — three example collections exist; the acceptance run is not automated |

Beyond M1, the repository also has three example collections and `examples/ask.py`, an agent
loop that drives them from plain English. Neither was in the spec's M1.

### Open: M3 before M2

The spec orders the milestones M1 → M5. There is now a reason to take **M3 before M2**.

`examples/ask.py` already assembles routing context by calling `describe --json` per node,
which is most of what M2's markdown pack would provide — so M2 buys less than it did before
that loop existed. Meanwhile every `ask.py` transcript has had its numbers reconciled against
the verified results *by hand*, which is precisely the job `vouch attest` exists to do.

Not decided. Recorded so the reasoning is not re-derived.

---

## Deviations from the spec

### 1. Contracts accept a `{ expr, message }` form

**Spec:** §3.1 shows `requires` and `ensures` as arrays of bare expression strings.
**Built:** either form. A contract may be a string, or a table with `expr` and an optional
`message`.

§4.2 requires that a failed precondition read as a routing correction — `unknown weapon
'lothric sword'; call weapon-lookup for valid names` rather than `invalid weapon`. A bare
expression cannot be that. The two sections of the spec are in tension, and the table form
resolves it without breaking the simple case.

When a contract has no `message`, the expression is reported instead. Honest, rarely useful.

Because TOML places subsequent bare keys inside the last-opened table, `[[requires]]` and
`[[ensures]]` sections must come after every top-level key in a manifest. This is why the
example manifests put them at the end.

### 2. A fourth outcome family: `error`, exit 1

**Spec:** §4.1 defines three families — refusal, defect, caller error.
**Built:** those three, plus `{"outcome": "error", "code": 1}` for problems with the
*collection* rather than with a call: no such node, unparseable manifest, a contract that
will not compile, a `-C` directory that does not exist.

These are not call outcomes and must not be mistaken for one. A missing node is not a refusal
— nothing was asked of anything. Folding it into an existing family would have made exit 11
mean two different things, and exit 11 is load-bearing.

### 3. `NaN` and `Infinity` are rejected at the JSON boundary, not the schema boundary

**Spec:** §8 says to "reject them at the output-schema boundary", which implies exit 12.
**Built:** they are rejected by the JSON parser, which surfaces as exit 21, a protocol
violation.

This was not a preference. `serde_json::Value` **cannot represent a non-finite number**:
`Number::from_f64` returns `None` for both, `json!(f64::NAN)` yields `Null`, and the parser
rejects `NaN`, `Infinity`, and overflowing literals like `1e400` outright. A schema-boundary
check was written first, then found to be unreachable, and deleted rather than left in place
implying a risk that does not exist.

The guarantee the spec wanted holds and is stronger than asked for — no contract can ever
evaluate against a non-finite number, because such a value cannot enter the value tree at
all. Pinned by `schema::tests::non_finite_numbers_cannot_enter_the_value_tree` and by the
`infinite` fixture node.

### 4. A `-C` / `--directory` flag

**Spec:** §4's CLI table lists commands and no global flags.
**Built:** `-C <DIR>`, matching `git -C`.

Added after `examples/` was separated from the runtime, which left the repository root
deliberately not a collection. Without it, every command needs a `cd` first.

It is a **real change of working directory**, not just a discovery hint, so relative paths in
the rest of the command line resolve from there too — `vouch -C examples/ds3-tools call
stat-optimizer --input @build.json` reads `examples/ds3-tools/build.json`. That commitment is
pinned by a test that only passes under chdir semantics.

---

## Decisions the spec left open

**The repository root is not a collection.** `src/` is the runtime, `tests/` is its tests,
`examples/` holds collections. Discovery looks for `nodes/` or `.vouch/` walking upward, and
the root has neither, so `vouch list` from the root is an error. The separation is enforced
rather than merely documented — and `tests/fixtures/` *is* a collection, so a passing test
proves the runtime found what it was pointed at rather than stumbling into it.

**Schemas may be inline or in a file.** `[input] schema = "input.schema.json"` or a JSON
Schema written directly as TOML under `[input]`. Small nodes stay at two files;
`ds3-tools` shows the other form.

**A manifest's `name` must match its directory.** The directory name is the routing key a
caller types, so a mismatch would make `describe` and `call` disagree.

**A `requires` expression referencing `result` is refused at load time.** Preconditions run
before the subprocess, so `result` does not exist and every call would refuse with exit 15 —
a manifest bug that would look like a runtime fault.

**`vouch list` shows unloadable nodes** marked as such, rather than hiding them. One broken
node must not make the rest of a collection disappear.

**Contract strength is reported by textual scan.** `describe` reports which numeric output
fields no postcondition mentions, by searching the expression sources for each field path.
This is approximate and can miss a field reached by an unusual expression. It is only ever
used to *report* strength, never to decide whether a contract held — the enforcing check
(does any `ensures` reference `result`) uses the real AST.

**Bounds belong in contracts, not schemas.** The schema checks shape; the contract checks
competence. `stat-optimizer` types `soul_level` as an integer but does not bound it, so an
out-of-range level *refuses* (exit 11, outside this node's competence) rather than being
rejected as *malformed* (exit 10).

---

## Known limitations

### A node cannot refuse

Only the runtime can refuse, and it sees only the input. A node can succeed or fail, and
failure is reported as a defect (exit 20).

So a lookup that misses — `triage` with a well-formed but absent ticket id, `stat-optimizer`
with a weapon not in the CSV — surfaces as "this node is broken", which is the wrong answer.
Nothing is broken; the question has no answer, which is a refusal.

Both example collections work around it by enumerating valid inputs in a precondition
(`input.weapon in [...]`, `input.ticket_id.startsWith("T-")`). That duplicates the data file
into the manifest. It is fine at ten weapons and wrong at ten thousand.

The general fix is a lookup node that resolves names and is called first — which is what
`stat-optimizer`'s `not_for` already points at. Whether the runtime should instead give nodes
a refusal channel (a reserved exit code, or a `{"refuse": "..."}` envelope on stdout) is
**open**. It would change the spec's claim that contract enforcement lives entirely outside
the node.

### Provenance covers outputs, not inputs

The runtime guarantees a returned value came from a function that satisfied its contracts. It
has nothing to say about whether the *arguments* were the right ones.

Observed concretely: asked "what stats for a lothric sword build", `ask.py` was refused with a
list of valid names containing both `Lothric Knight Sword` and `Lothric's Holy Sword`, picked
the first, and answered without mentioning there had been a choice. The number is real. The
reading of the question was never checked, and by the time the contract fires the
interpretation is already settled.

Mitigations that exist: `[[reads]]` records provenance, the ledger will record what was
passed, and §8.5's "move fetches outward" preference makes inputs auditable rather than
hidden. Enforcement does not exist and may not be possible.

### Nothing checks the prose

`ask.py`'s narration prompt forbids introducing figures absent from the verified results.
Forbidding is not preventing; a transposed digit in the final sentence would pass. This is
exactly what `vouch attest` is for, and it is M3.

Until then, transcripts have been reconciled by hand.

---

## Things deliberately not built

From the spec's §9 deferred list, unchanged: caching, sandboxing, declared effect sets, node
composition, warm worker pools, any server or MCP transport.

Added to that list here:

**A Makefile.** `cargo install --path .` is the install step and needs no wrapper.

**A node SDK.** A node is a subprocess reading JSON on stdin and writing JSON on stdout. The
JavaScript examples import nothing but `node:fs`. Anything that makes nodes depend on `vouch`
weakens the claim that the box is not vibe-coded.
