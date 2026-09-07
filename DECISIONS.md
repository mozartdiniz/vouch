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
| M2 — registry preamble, `describe --all --md` | **Done** |
| M3 — ledger, scalar flattening, `vouch attest` | **Done** |
| M4 — `vouch test`, `vouch eval` | **Done** |
| M5 — demo collection, README, six-step acceptance run | **Done** — three example collections; all six steps run, steps 1–3 and 5 also pinned by tests |

Every command in §4's table exists. Beyond the milestones, the repository has three example
collections — each now carrying its own `cases.toml` fixtures and `.vouch/evals.toml` suite —
and `examples/ask.py`, an agent loop that drives them from plain English and attests its own
answers.

**M3 was taken before M2**, out of spec order, because every `ask.py` transcript was having
its numbers reconciled by hand — precisely the job `attest` exists to do. That gap is now
closed and the loop checks itself.

M2 followed rather than being skipped, because `vouch eval` needs to build the context prompt
it hands an agent, and that is exactly what `describe --all --md` generates. Building eval
first would have meant writing the same generator somewhere less reusable.

---

## Where to pick up

*Last worked on 2 September 2026. Working tree clean, `main` pushed.*

The MVP is built and demonstrated. A collection describes itself, calls are contract-checked,
every call is recorded, a written answer can be reconciled against that record, both testing
layers exist, and the acceptance demo has been run end to end against a real model.

```console
$ cargo install --path .                      # ~/.cargo/bin/vouch
$ cargo test                                  # 151 tests
$ vouch -C examples/ds3-tools test            # 14 fixture cases, no model
$ vouch -C examples/ds3-tools call weapon-lookup --input '{"query":"lothric sword"}'
$ vouch -C examples/support-triage describe --all --md
$ ./examples/ask.py -C support-triage "what do we owe on ticket T-1001?"
```

The one command that needs a model, and therefore costs tokens on every run:

```console
$ vouch -C examples/ds3-tools eval --agent 'claude -p --allowedTools "" -- {prompt}' -n 5
```

The 1 September work was, in order:

1. **`weapon-lookup` and a shared `data/`** in `examples/ds3-tools`, plus the registry
   preambles — which had never been committable, because `.gitignore` swallowed all of
   `.vouch/`. No runtime change.
2. **M4.** `verify.rs` extracted from `commands::call` first, so `test` and `eval` run the
   pipeline a real call runs; then `vouch test`; then `vouch eval`.
3. **The acceptance run** (§10), recorded below, which found and fixed two defects.

### 2 September: seven changes, all found by using it

The runtime was not extended on 2 September. It was **used**, to build a real collection of
about the size the MVP was always aiming at — `~/Dev/elden-ring-vouch`, eleven nodes over the
Elden Ring Build Planner tables — and everything below is something that only shows up when a
tool meets work it did not anticipate. This is the "use it on a real collection" move the
previous entry recommended, and it is what the section above should be read as the answer to.

| | |
|---|---|
| `7f2eba2` | `vouch test` checks a float expectation to the precision it was written to |
| `dd09b61` | `vouch eval` prints each case as it finishes |
| `cbe8b90` | the agent subprocess gets no stdin |
| `5fe9e1d` | two more node-authoring rules in §8 |
| `d2cffc7` | `vouch eval` keeps what finished when the agent quits, and says why |
| `5d5b505` | `vouch eval` shows an unattested numeral in context |
| `18f7938` | the broken-pipe panic recorded as known roughness |
| `43d5317` | a node can refuse: exit 3, reason on stderr, exit 16 to the caller |
| `2deb9b3` | collection contracts in `registry.toml`, applied by parameter |
| `5ba5c9f` | `describe --compact` and `--index`, for the context window |
| `a6fd41f` | judgement parameters: exit 17, a refusal that is a question |
| *(this commit)* | **`attest --json` says what accounted for each figure, and how firmly** |

**Attestation was a verdict and is now also the working.** `attest_text` built a map of ledger
path to value and then called `.values()` on it, so the strongest check in the runtime could
report *that* a figure was accounted for and never *what* accounted for it. The two readings
are very different: `512 AR` traced to `result.attack_rating` is evidence, and the same numeral
traced to `result.rows[7].weight` is a coincidence that passed.

Keeping the map costs nothing and it also makes the known weakness measurable. `accounted_by`
counts how many recorded values could explain a numeral and `ambiguous` counts the numerals
where that is more than one — which is the mutation sweep's surviving hole expressed as a
number rather than as a caveat in a document. It scales with ledger size exactly as expected:
19% of the integers 1 to 99 are present in a two-call ledger, 96% in a day's.

Writing the tests for it reproduced item 2 of the same feedback, which is the other half of
this: `attest` without `--ledger` reads the most recently modified session file rather than
the session it was told about, so three new tests running in parallel read each other's
ledgers and passed or failed on the wrong evidence. There is already a test helper that names
the ledger explicitly, written for that reason. The default is still wrong.

**A refusal that is a question.** Some parameters have no right answer in the data, and a node
has three options for them. Picking one presents an opinion as a calculation. Requiring one and
saying nothing leaves the caller to invent it — and a model asked to invent produces a
different value each run, which was measured rather than assumed: fourteen of fifteen questions
where a model had to supply a judgement drifted between repeats, against two of five where none
did. Every run was internally consistent and every figure traceable; nothing was wrong except
that the answer moved.

The third option is `judgement = true` with `options`, and a refusal at **exit 17** carrying
both. The code is separate from `11` on purpose: they are different instructions to whoever is
driving. `11` says the call was wrong — fix the arguments or route elsewhere. `17` says the
call was fine and one value in it belongs to a person nobody has asked. A caller that cannot
tell them apart either interrogates the user about genuine mistakes or quietly invents an
answer to a real question.

`options` stays free-form JSON. Only the collection knows whether a choice is one value or a
set that travel together, and a caller renders them rather than interpreting them. And they are
published in every describe shape, because a router that learns this from a refusal has already
spent a decision to find out something the pack could have told it.

**The routing pack had two readers and was written for one of them.** `--md` is pasted into a
file once. `--json` is re-sent on every routing decision an agent makes — six to sixteen per
question in the first collection to drive one — and nobody had costed that: 28,000 tokens a
decision, of which the contracts, the output schema, the `reads` and the timeouts are read by
nothing routing, `$schema` and `title` are read by a validator and a doc generator, and
`params.*.guidance` duplicates a description the schema already carries.

That collection wrote its own trimmer and got a third of it back. `--compact` is the same
trimmer, and it lands within 0.8% of the hand-rolled one on the same input, which is the
evidence that the port is faithful rather than merely similar.

`--index` came out of the same measurement from the other end: a caller choosing between
nineteen nodes needs prose, not schemas, and 12,399 characters is a different kind of object
from 78,888.

Found while wiring it: `markdown::pack` prefers the preamble's `name` and the JSON renderer
used the directory name, so one collection answered to two names depending on the flag. The
preamble wins, since it is the only one anybody chose.

**Collection contracts, and the half of the problem they turned out not to be.** The request
was to let a collection state a rule once and have it apply everywhere the parameter appears,
and the motivating example — *"any node taking `weapon` must satisfy that the weapon exists"* —
is not expressible. A CEL contract sees `input` and `result` and no custom functions are
registered, so anything requiring a lookup is out of reach and always was.

That is worth stating plainly because it re-scopes the feature. The data-dependent guards are
the previous entry's job: a node refusing on its own data is the only thing that can read a
table. What is left for a collection contract is the structural half — ranges, co-presence,
uniform postconditions — and that half is real: it is where a rule gets written into nine
manifests and forgotten in the tenth.

Two decisions inside it. Each contract is **scoped to its property's presence**, because an
unevaluable contract fails closed (§3.2) and a collection-wide rule that refuses every call
omitting an optional parameter would be worse than no rule. And `when` names the property in
the object the contract inspects — input for a precondition, result for a postcondition —
which sounds like a detail and is not: an `ensures` scoped to the input skips exactly the calls
that left the parameter out, so the node returns the value the collection said it never
returns with nothing to catch it.

They do not relax §3.3. A node still needs a postcondition of its own; the collection's is a
floor, not a substitute for a node making a claim about what it returns.

**A node had no way to say no, and it was the most expensive omission in the runtime.** It
surfaced from outside: a second collection accumulated four separate bugs (its 4, 22, 23, 24)
that were all one thing — a node with an ordinary "that is not in my table" to report, and
only two ways to report it. `exit 1` reads as `NODE_CRASHED`, a defect, which tells the caller
the node is broken and discards the rest of their question along with the part that had no
answer. The alternative is a success carrying an "it isn't there" shape, which is correct but
costs an output field, a contract, and a decision about every size invariant that assumed the
node always describes something — so it has to be re-invented per node, and it ends up in some
and not others. That is the whole mechanism behind "a guard that lives in one node is not a
guard", and it was a property of the runtime rather than of the authors.

The refusal codes that existed all belong to `vouch` and not to the node: `11` is a
precondition, which is CEL over the *input* and therefore cannot answer "is this weapon in the
catalogue" — the questions that actually need refusing, every one of which requires reading
the node's own data.

Exit **3**, not 1 or 2, because those are an uncaught exception and an argument error in most
languages. A runtime that read either as a considered refusal would turn a crash into an
answer, which is the one direction this must never fail in. The `crasher` test fixture had
been exiting 3 arbitrarily and now exits 1, which is a better model of a crash anyway.

Three are worth reading for what they say about the design rather than the fix.

**Exact float equality in fixtures was wrong.** Every expectation in that collection comes from
a spreadsheet cell recorded to seven places while the node emits full precision, as §8.3 asks.
Eight of eight generated cases failed on `543.1948141` against `543.1948140689826`. The
comparison was testing the transcription of the ground truth, not the node. Fixtures now use
the same rounding rule §6.2 already gives `attest` for prose.

**`vouch eval` was unusable on a suite that fails.** It printed nothing for six minutes across
eighteen model calls, then discarded everything if the agent died partway, and reported "the
agent command failed: " with no reason — because `claude -p` announces a usage limit on
*stdout* and leaves stderr empty. All three were hit repeatedly in one afternoon.

**An unattested numeral needs its context.** A failure line naming only the digits — "1 of 10
numerals did not come from a node: 2" — is unactionable when the case fails one run in seven.
Printing the surrounding words caught a months-old intermittent on its first occurrence
afterwards: the agent had computed a requirement gap by hand.

The §8 rules gained the two mistakes that collection made most: **a number inside a string is
invisible to the ledger**, which records numeric leaves, and **return every figure in the form
a reader will quote it in**. Six separate fabrications there were all the same shape — a node
returning a fact and leaving the reader one small sum — and every one was fixed upstream of
where it appeared rather than by checking the prose harder.

### The MVP is finished

Every milestone is done and the acceptance demo has been run end to end — see "The acceptance
run" below for what it produced and the two defects it found. What is left is deliberate
deferral rather than unfinished work:

- **Prebuilt release binaries**, held until the CLI has been used in anger (below).
- **The node refusal channel**, which is an open design question and not a task (below).
- Everything on §9's deferred list: caching, sandboxing, effect sets, composition, warm
  workers, any server or MCP transport.

That advice was taken, and the seven changes above are what it produced. The same source
suggests what is worth doing next.

**`describe` could flag the figures a node leaves a caller to compute.** Six fabrications in one
collection were all a node returning a fact and leaving one small step — a truncation, a
subtraction, a gap. Each was found individually by an eval, after the fact, which is reactive:
a node author has no way to ask "which of my outputs will a reader have to do arithmetic on?"
`describe` already reports which numeric fields no postcondition mentions; the signature here
looks similar — a `number` field with no integer twin, a pair of fields whose difference a
reader will obviously want. It is a guess that it can be detected well enough to be worth
saying, and it should be tried on a collection that has already been through the eval loop
rather than designed in the abstract.

**The eval suite is the slowest part of the loop by a wide margin.** Thirteen cases at three
runs is over a hundred model calls and half an hour, and a usage limit ended three separate
runs mid-suite. `--only <case>` and resuming a cut-short run would both have paid for
themselves several times over in one afternoon.

### Also outstanding

`tests/examples.rs` has not shrunk now that the examples carry their own fixtures. It was
going to, and it should not: the two files assert different things. `cases.toml` pins what a
node does; `tests/examples.rs` pins the figures and refusal messages the READMEs *quote*, which
is the prose that drifts. The overlap is real and it is cheaper than the drift.

### Known roughness

**`vouch <command> | head` can panic on a broken pipe.** Rust ignores SIGPIPE, so once `head`
exits, the next `println!` fails and panics — "failed printing to stdout: Broken pipe". Seen
once on `vouch list | head -3` against a nine-node collection; it is a race and does not
reproduce reliably. The fix is to restore the default SIGPIPE handler at startup, which needs
`libc` as a direct dependency (it is already in the tree under tokio). Not done yet: it is
cosmetic, it costs a dependency, and piping to `head` is something a person does interactively
rather than something a script depends on.

### Decisions waiting on a human

- **Should a node be able to refuse?** See "A node cannot refuse" below. It would change the
  spec's claim that contract enforcement lives entirely outside the node, so it is not a
  change to make casually.
- **Prebuilt release binaries.** `cargo install --path .` needs a Rust toolchain, which sits
  oddly with §2's "single static binary with no runtime dependency" pitch. Deferred
  deliberately until the tool has been used in anger for a while — a release process pins a
  CLI surface, and pinning one that is still moving costs more than it saves.

---

## The acceptance run

*Run 1 September 2026, against `claude -p` (Claude Code 2.1.252).*

§10's six steps, in order, plus `vouch eval` against a real model for the first time. Both are
recorded here because a demo nobody has executed is a claim, not a demonstration — and because
this one found two defects.

### The six steps

| Step | Result |
|---|---|
| 1. `stat-optimizer` returns a number satisfying its postconditions | Pass — AR 190.9 at SL120. Pinned by `tests/examples.rs` and `cases.toml`. |
| 2. Precondition fails → exit 11, informative message, no value | Pass. Pinned by tests. |
| 3. Break the math → exit 13, defect, no value leaks | Pass. Pinned by tests. |
| 4. Paste the pack into a `CLAUDE.md`; ask Claude Code a build question | Pass — see below. |
| 5. Hand-edit one digit; `vouch attest` catches it | Pass — 2 of 11 numerals reported with their positions. |
| 6. Ask a bare model the same question | Pass, in the sense the spec means: a confident, plausible, unverifiable answer. |

**Step 4.** `examples/ds3-tools/CLAUDE.md` is the generated pack, and it is committed so the
run is reproducible; a test asserts it has not drifted from `describe --all --md`. Asked "what
stats should I level for a Lothric Knight Sword build at soul level 120, and what attack rating
does that give?", Claude Code called `stat-optimizer` once, with the right arguments, and
answered with AR 190.9, 119 points spent, and the full stat spread. `190.9` appears nowhere in
the `CLAUDE.md`, and the session ledger holds the call it came from.

**Step 5.** Attesting that answer against its own session:

```
ledger: .vouch/ledger/session-acceptance-step4.jsonl (1 entry, 11 scalars)
clean: 11 numerals checked, 11 matched, 0 ignored
```

Every figure in the prose traced to one call. Changing `190.9` to `190.8` and one stat from
`42` to `43` produced `UNATTESTED: 2 of 11`, with line and column for each.

**Step 6.** The same question to a bare model, no tools and no pack, produced a longer and more
confident answer: a full stat table, four infusion comparisons, and AR figures of 395, 425, 430
and "past 500". Attested against the same ledger: **20 of 36 numerals could not be accounted
for.**

The honest reading of that, which matters more than the pitch: those numbers are not
necessarily *wrong about Dark Souls 3*. `weapons.csv` is a simplified model, so 190.9 is right
about this dataset and 395 may well be closer to the real game. What the two answers actually
differ in is whether anything can be checked. The bare answer closes with "AR figures are from
memory, ±5 — verify in-game before committing respec points", which is the model correctly
describing its own epistemic position and is exactly the sentence a reader skips. The step 4
answer carries no such hedge because it does not need one.

So step 6 next to step 4 is the pitch, but the pitch is *provenance*, not correctness — the
same line §6.3 draws. A green attestation says every figure came from a function. It never says
the function is a good model of the world.

### `vouch eval` against a real model

`--agent 'claude -p --allowedTools "" -- {prompt}'`, five runs per case:

| Collection | Result |
|---|---|
| `hello-world` | 10/10 |
| `support-triage` | 25/25 (after the fix below; 4/5 before it) |
| `ds3-tools` | 25/25 |

The ambiguity case was expected to be the one that failed. It did not: in all five runs the
agent called `weapon-lookup` with "lothric sword", got two candidates and `resolved: ""`, and
declined to choose — which is what the preamble asks for and what `expect_stop` asserts.

The `--allowedTools ""` is worth keeping: without it the agent has Claude Code's own tools and
may go and read the collection instead of routing through the published context, which measures
something other than the pack.

### What the run found

**A node that made a model do arithmetic.** `support-triage` failed its first case, 4/5. The
agent answered "250 minutes past its SLA" and `attest` refused the 250 — correctly, because
`triage` returned `minutes_remaining: -250` and no node had ever produced `250`. The negation
was the model's. Worse, `escalation-cost`'s parameter guidance *told* it to negate: "this is
the negation of triage's minutes_remaining".

`triage` now returns `minutes_over` alongside `minutes_remaining`, tied together by a
postcondition. The suite went to 25/25. The general rule, which is §8.1 in a sharper form and
now sits in the collection's README: **return every figure in the form a caller will quote it
in.** A node that leaves the reader one small sum has handed that sum to a model, and the sum is
where fabrication lives. It also shrinks the "provenance covers outputs, not inputs" gap below
— `escalation-cost`'s argument is now copied from a result rather than computed from one.

**Two bugs in one line of `attest`.** Step 6's answer contained `±5`, and numeral extraction
panicked: it looked one *byte* back from a digit to check for a currency prefix, and that byte
was inside the `±`. The same line could never have matched `£`, `€` or `¥` either, since none
of them is one byte — so currency detection had silently worked for `$` alone since it was
written. Both are fixed by looking at the preceding *character*, and pinned by a test.

Neither defect was reachable from the example collections or the fixtures. Both needed real
prose from a real model, which is the argument for running the demo rather than describing it.

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

### 6. `vouch eval` gains `expect_stop`, `--min-rate`, and a suite location

**Spec:** §7.2 shows `[[eval]]` with `ask`, `expect_node`, `expect_params` and `attest`, run as
`vouch eval --agent "claude -p {prompt}" -n 10`.
**Built:** that, plus three things it does not mention.

**`expect_stop`.** The spec's four fields can only assert that an answer was produced
correctly. The case this project cares most about — the question whose honest answer is "there
isn't one" — is unwritable with them: `what do we owe on ticket T-1006?` has no figure, and an
eval that cannot assert "the agent declined" cannot measure the behaviour the README leads
with. `expect_stop` asserts the run ended in a decline. It is rejected alongside `attest`,
because a run that stops writes no answer to check.

**`--min-rate`, default 1.0.** §7.2 is explicit that this is a rate rather than a pass, and a
command that fails on any single flake cannot be used in the CI job the rate exists for. The
default is still 1.0, so a suite is strict until someone deliberately loosens it.

**The suite lives at `.vouch/evals.toml`.** The spec does not say where. Beside the registry
preamble, because both are properties of the collection rather than of any one node — unlike
`cases.toml`, which §2.2 puts beside the node it tests. `--file` overrides it.

### 7. Both testing commands take `attest`'s exit convention

**Spec:** §7 does not give exit codes for `test` or `eval`.
**Built:** 0 everything passed, 1 something failed, 2 the run could not happen — the same split
§6.2 gives `attest`, for the same reason. A failing fixture is a *finding* about the collection;
a missing suite or an agent command that will not start is a failure of the command. For a
checking tool that distinction is the whole value: a run that never happened must not be
indistinguishable from a run that found nothing wrong.

Two consequences follow from taking that seriously:

**A run that checked nothing is an error, not a pass.** `vouch test` over a collection with no
`cases.toml` anywhere exits 2 rather than reporting success over zero cases. This is the same
argument §3.3 makes about vacuous postconditions — false assurance is worse than absent
assurance — applied to the tool that reports the assurance.

**A node that will not load is a failure, not a skip.** `describe --all` omits unloadable nodes
and names them on stderr, because publishing a node an agent cannot call would only invite a
failure. `vouch test` does the opposite and counts them as failures, because it is being asked
whether the collection is sound and the answer is no. One broken node still does not abort the
run: the loadable nodes' cases are reported first.

### 8. `vouch eval` keeps its ledger in memory

**Spec:** §7.2 says the prose must attest clean; §6.1 says every call appends to
`.vouch/ledger/`.
**Built:** an eval's calls are recorded in memory and attested against there. Nothing reaches
the ledger directory, and `vouch test` writes nothing at all.

Both are rehearsals rather than calls anyone is entitled to quote a number from. Writing them
to the session ledger would let a figure that only ever appeared in a fixture or an eval account
for a numeral in a real answer later — the same loosening `VOUCH_SESSION` exists to prevent
(a smaller ledger is a stricter check), arrived at from the other direction. It also keeps a
stale eval file from becoming `attest`'s default "newest session".

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

### Testing

**Expected values are addressed by the ledger's path grammar.** `expect = { "result.stats.dexterity" = 40 }`,
rooted at `result` and spelled the way `ledger::scalars` spells a scalar. `value_at` lives
beside `scalars` in `ledger.rs` so that one place defines the grammar rather than three that
nearly agree, and a unit test asserts every path `scalars` writes can be read back by
`value_at`. A path without the `result.` prefix is rejected when the file loads.

**Numbers compare numerically, and a float to the precision it was written to.** TOML's `40`
matches a node's `40.0`, because the spelling of a number must not decide a fixture. A float
expectation is checked after rounding the found value to as many decimals as the expectation
carries — the same rule §6.2 gives `attest` for prose, and for the same reason.

This started as exact float equality, on the argument that a node given fixed input returns a
fixed value. Real use killed it. Building the Elden Ring collection, every expectation came
from a Build Planner spreadsheet cell recorded to seven places, while the node emits full
precision as §8.3 asks: `543.1948141` against `543.1948140689826`, eight times over. Exact
comparison there tests the transcription of the ground truth, not the node. Writing more digits
demands more, and an integer expectation has no decimals to round to, so it stays exact.

**`expect_params` is a subset, not an equality, and matches any call in the run.** A case pins
what matters and leaves the rest free, so adding an optional parameter does not break every
eval. Matching any call rather than the first means an agent that reaches the right call *after*
reading a refusal counts as having routed correctly — which is exactly what §4.2 claims a
refusal is for, so an eval that penalised it would be measuring against the design.

**A reply the loop cannot act on is a failed run, not a failed command.** A model that answers
with prose instead of JSON, or names a node that does not exist, is the failure an eval exists
to count. An agent command that will not *start*, by contrast, aborts with exit 2: there is no
rate to report, and reporting `0/10` would blame the collection for a broken harness.

**The eval loop is `examples/ask.py` in Rust, deliberately.** Same planning rules, same
two-turn shape, same treatment of refusals as corrections and defects as stopping conditions.
The context it hands the agent is `markdown::pack` — the routing pack a user pastes into a
`CLAUDE.md`. Building a second, eval-only description would measure a context nobody ships,
which is the one thing an eval must not do.

### The routing pack

**The pack omits the contracts.** §5.3 lists what goes in — purpose, `use_when`, `not_for`,
parameter guidance, examples — and preconditions are not on it. That is the right call for a
reason worth writing down: contracts are *enforcing*, not advisory. A caller does not need to
read a precondition to make a good call, because getting it wrong produces a refusal written
to be acted on (§4.2). Publishing them would bloat the context an agent carries every turn,
and would blur the line between the advisory layer and the enforcing one.

**Parameters are rendered one line each, not as a JSON Schema.** `` `soul_level` (integer,
required) — Target soul level… `` is what a caller needs; the full schema is noise in a
context window. `describe <node> --json` still has it for anyone who wants more.

**The pack includes a usage section** covering how to invoke a node and what each exit family
calls for. Not in §5.3's list, but a pack that describes nodes without saying how to call one
is a catalogue rather than "the entire integration story", and the exit families are the part
a reader most needs to act on differently.

**Unloadable nodes are omitted from the pack and named on stderr.** A node that fails the
strength gate cannot be called, so advertising it to an agent would only invite a failure.
`vouch list` still shows it, because a person debugging a collection needs to see it.

**A missing preamble is fine; a malformed one is an error.** A collection of well-described
nodes works without `.vouch/registry.toml`. But silently ignoring context an author
deliberately wrote is worse than refusing to start.

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

### The collection on disk

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

**A dataset two nodes read belongs to the collection, not to either node.** `ds3-tools` keeps
`weapons.csv` at `data/weapons.csv` and both nodes declare `../../data/weapons.csv`. `run` and
`[[reads]]` paths stay relative to the node directory, which is what makes a node runnable from
anywhere, so the `../../` is the cost of not having a second copy. It is the right cost: a
lookup resolving names against different bytes than the optimizer computes from would be worse
than no lookup, and two files with the same contents are two files that will differ eventually.

**`.vouch/registry.toml` is committed; `.vouch/ledger/` is not.** The original `.gitignore`
ignored all of `.vouch/`, which meant the preamble this file argues for could never reach a
repository — the one quoted in the root README as `support-triage`'s had never existed. Ledgers
are per-session and per-machine and stay ignored; a preamble is authored context and is source.

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

`ds3-tools`' `weapon-lookup` node fixes the *routing* half — a partial name resolves, an
ambiguous one does not — but not this half. The enumeration is still there, because it is what
stops an unknown weapon from crashing `optimize.py` into a defect. Whether the runtime should
give nodes a refusal channel is still **open**, and the shape of the choice is below.

#### What the two architectures actually differ on

**Today.** The runtime is the sole author of every outcome. It decides from things it can see
without trusting the node: the input schema, preconditions over `input`, the exit status, the
shape of stdout, the output schema, postconditions over `result`. The node's entire vocabulary
is *one JSON object on stdout* (success) or *anything else* (defect). Three consequences
follow, and they are the whole of the difference:

- "No answer exists for this well-formed input" is **inexpressible by the node**. It has to be
  decided in advance, from the input alone, which means every fact needed to judge competence
  must be restated as a CEL precondition — i.e. lifted out of the data and into the manifest.
  That is exactly the duplication above, and it is not a wart on the design, it is the design.
- A refusal is **uncounterfeitable**. A node cannot manufacture one, so §2's claim that
  contract enforcement lives entirely outside the node holds literally, not approximately.
- Every byte that reaches a caller as a value has passed a JSON Schema. There is no channel
  through which a node's own prose reaches an agent.

**With a refusal channel.** Two spellings. A reserved exit code is the obvious one and the
worse one: a node that exits 11 by accident — a library's `sys.exit`, a shell wrapper passing
through a status — launders a defect into a refusal, silently. An envelope on stdout
(`{"refuse": {"reason": "..."}}`) is much harder to emit by accident and is checked at the
same protocol boundary that already rejects everything else, so that is the form to build.

What it costs, concretely:

- `exec::run` stops returning `Result<Json>` and starts returning a three-way outcome —
  value, refusal, defect. Every caller of it gains a branch.
- A refusal **skips the output schema and the postconditions**, because there is no result to
  check. So the reason string is the first thing the runtime has ever handed a caller without
  validating it. Mitigation: schema-constrain the envelope itself (a `reason` string, bounded
  length, perhaps a `retry_with` field), so the channel is narrow even though its contents are
  the node's.
- It wants its own exit code — 16, say, in the refusal family — rather than reusing 11. The
  ledger and `vouch test` both need to tell "your input was outside the declared competence"
  apart from "the node ran, did its reads, and found nothing". They are different facts about
  the collection, and collapsing them makes exit 11 mean two things.
- Such a refusal *should* be recorded in the ledger, unlike a precondition failure: the
  subprocess ran and its `[[reads]]` are the audit record of a lookup that genuinely missed.

**What is not at stake: soundness.** A refusing node returns no value, so it cannot return an
unsound one. §1.3's guarantee is untouched either way.

**What is at stake: the visibility of a broken node.** Today, a node that throws is a defect,
and an agent is told to stop trusting it. Give nodes a refusal channel and a catch-all
exception handler that refuses turns every bug in that node into "I don't know" — permanently
plausible, permanently invisible. The runtime becomes *more incomplete* in a way nobody can
measure, and the signal an agent uses to report a broken tool is gone.

That is the trade in one line: **a refusal channel buys the ability to say "no answer" from
inside the data, and spends the ability to tell a node that has no answer apart from a node
that is broken.**

### Provenance covers outputs, not inputs

The runtime guarantees a returned value came from a function that satisfied its contracts. It
has nothing to say about whether the *arguments* were the right ones.

Observed concretely: asked "what stats for a lothric sword build", `ask.py` was refused with a
list of valid names containing both `Lothric Knight Sword` and `Lothric's Holy Sword`, picked
the first, and answered without mentioning there had been a choice. The number is real. The
reading of the question was never checked, and by the time the contract fires the
interpretation is already settled.

`ds3-tools`' `weapon-lookup` node is the narrow fix for that specific case, and it is worth
being precise about how much it fixes. The node makes the ambiguity a *value*: `"lothric
sword"` returns both candidates with `resolved` empty, and the postcondition `(result.resolved
!= "") == (result.match_count == 1)` makes a node that picks one a defect (exit 13) rather
than a plausible answer. So the *collection* can no longer produce a resolved name it did not
earn. What remains advisory is the agent's obligation to ask the user which weapon they meant
instead of choosing between the two candidates itself — that lives in the registry preamble,
which is published rather than enforced. The runtime cannot check it, because by then the
choice is an input again.

The general shape of the mitigation is worth copying: where a question can have two readings,
return both and resolve neither, and write a postcondition that makes resolving one without
grounds a defect. It converts an unenforceable claim about interpretation into an enforceable
claim about a return value.

Other mitigations that exist: `[[reads]]` records provenance, the ledger records what was
passed, and §8.5's "move fetches outward" preference makes inputs auditable rather than
hidden. General enforcement does not exist and may not be possible.

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
