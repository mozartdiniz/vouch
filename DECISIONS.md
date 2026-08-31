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
| M3 — ledger, scalar flattening, `vouch attest` | **Done** |
| M4 — `vouch test`, `vouch eval` | Not started |
| M5 — demo collection, README, six-step acceptance run | Partly done — three example collections exist; steps 1–3 and 5 are covered by tests, steps 4 and 6 have never been run |

Beyond the milestones, the repository has three example collections and `examples/ask.py`, an
agent loop that drives them from plain English and attests its own answers.

**M3 was taken before M2**, out of spec order. `ask.py` already assembles routing context by
calling `describe --json` per node, which is most of what M2's markdown pack would provide, so
M2 bought less than it once did. Meanwhile every `ask.py` transcript was having its numbers
reconciled by hand — precisely the job `attest` exists to do. That gap is now closed and the
loop checks itself.

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

### 5. `attest` has its own exit convention

**Spec:** §6.2 says exit 0 if every numeral is accounted for, exit 1 otherwise.
**Built:** that, plus exit **2** when the check itself could not run.

Exit 1 under §6.2 means "unmatched numerals found", which is a *finding* — the command
worked. That collides with the `error` family above, where 1 means the command failed. An
unreadable ledger returning 1 would be indistinguishable from a clean run finding problems,
which is the worst possible confusion for a checking tool. Its own failures moved to 2.

---

## Decisions the spec left open

### The ledger

**Sessions come from `VOUCH_SESSION`, falling back to the date.** The spec names the file
`session-<id>.jsonl` without saying where the id comes from. A harness should set the variable
once per conversation; that is what makes attestation tight, since a numeral can only be
accounted for by a call in *this* session and a smaller ledger is a stricter check. The date
fallback keeps a plain shell usable at the cost of a looser scope. Ids are sanitised, because
they reach the filesystem.

**A ledger write failure is fatal on the success path, and a warning everywhere else.** §1.2
makes recording part of the guarantee — the result passed its contracts *and* its origin is
recorded — so if it cannot be recorded there is nothing to vouch for, and nothing reaches
stdout. On a failing path the original refusal or defect matters more to the caller than a
write that did not happen, so it is reported as a warning and the original error stands.

**Refusals and defects are recorded too.** The spec's example entry shows `outcome: "ok"`, but
carrying `outcome` and `code` fields only makes sense if they vary. A ledger is an account of a
session, not a highlight reel of the calls that worked. Preconditions are the exception: they
fail before the subprocess runs, so there is nothing to record and no entry is written.

**`scalars` holds result values only**, matching the spec's example exactly. Inputs are in the
entry verbatim and can be admitted to a check with `--include-inputs`, but not by default — an
agent chose them, so counting them as verified would launder a fabricated argument into an
attested figure.

### Attestation

**Matching runs before the ignore rules.** A numeral is checked against the ledger first, and
only if that fails is it tested for being a year, a small integer, or a number from the user's
question. This ordering is what makes the ignore list safe: an over-broad rule can only soften
a miss, never suppress a figure that genuinely came from a node.

**Unit detection is a deliberately conservative heuristic.** A unit is a currency symbol in
front, or `%`, or a short word (≤4 characters, not a common function word) right after. `40 kg`
and `512 AR` count; `3 of them` and `2026 there` do not. It exists only to keep the ignore
rules from excusing measured quantities, and given the ordering above, a missed unit costs a
little detection in a narrow band while a spurious one would disable the ignore rules on
ordinary prose. The second failure is much worse, so the heuristic errs toward "not a unit".

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

### Attestation covers scalars, not judgement

This one is by design and is stated in §6.3, but it bears repeating where people will read it.

`attest` verifies that the numbers in an answer are real. It cannot verify that the answer is
*good*. A workout plan's sets, reps and volume totals attest cleanly; "is this a good
programme" is a judgement no ledger can settle. A stat spread attests cleanly whether or not
it is a sensible build.

A green check means every figure traces to a function's return value. It means nothing more,
and it must never be presented as meaning more.

Two narrower gaps in the same area:

**Non-numeric claims are unchecked.** A sentence can attest perfectly while attributing the
right number to the wrong thing — swap two stat names and every numeral still matches.

**A looser session is a weaker check.** Without `VOUCH_SESSION`, the ledger covers a whole
day, so a numeral from an unrelated earlier call can account for a figure in today's answer.
It is a false negative, never a false positive.

---

## Things deliberately not built

From the spec's §9 deferred list, unchanged: caching, sandboxing, declared effect sets, node
composition, warm worker pools, any server or MCP transport.

Added to that list here:

**A Makefile.** `cargo install --path .` is the install step and needs no wrapper.

**A node SDK.** A node is a subprocess reading JSON on stdin and writing JSON on stdout. The
JavaScript examples import nothing but `node:fs`. Anything that makes nodes depend on `vouch`
weakens the claim that the box is not vibe-coded.
