# Feedback from a real collection

Ten things a second, non-trivial collection asked of `vouch` and did not get. Every item
below comes from `~/Dev/elden-ring-vouch` — nineteen nodes, 253 fixtures, 122 worked
questions, 24 recorded bugs, and a chat app that puts a model in charge of the parameters.
None of it is speculative: each entry names what happened, and several name the workaround
that collection had to write because the runtime offered nothing.

Recorded 7 September 2026, from a review of `src/` against that collection's `HANDOFF.md`
and `BUGS.md`. Item 10 was added the same day, found by fixing the third instance of the bug
class item 4 describes rather than by reading the code.

**Status.** Done: **10** (`43d5317`), **4** (`2deb9b3`, in the only form it can take), **6**
(`5ba5c9f`), **5** (`a6fd41f`, the judgement half), **1** (`25b0f9a`) and **7** (this commit,
minus a `--max-spend` that cannot exist). Open: **2**, **3**, **8**, **9**.

Two of the four came out different from how they were written here, and both differences are
recorded in place rather than quietly absorbed. Item 4 shrank to its structural half. Item 5
grew a distinct exit code, because a refusal that is a question is a different instruction to
a caller than a refusal that means the call was wrong.

## What is working, and should not be disturbed

Worth writing down first, because three decisions are carrying the whole design and a
refactor could quietly cost any of them.

- **`attest` involves no model.** The last mile — a model writing prose — is exactly where
  fabrication happens, and it is checked with string and number handling. Everything else in
  this space puts a second LLM there.
- **Refusal is a first-class outcome with exit-code families a caller can branch on.**
  `verify.rs::attempt` has no third path. Sound-but-incomplete is the right trade.
- **The ledger records refusals and defects, not only successes.** A highlight reel of the
  calls that worked would be useless for an audit.

Contracts have also earned their place on their own: they caught rune level being stat
sum − 79 rather than points + 1, and `weight_left` being roll-change headroom rather than
unused capacity. And `describe --all` as the entire integration story — no protocol, no
config format — is why that collection's web app is four files.

---

## 1. Attestation is set membership, not provenance — *done* (`25b0f9a`)

`commands.rs:626` does `scalars.values().copied()`. The `{entry}:{path}` keys that
`attest::ledger_scalars` just built are thrown away, so `matches_any` can only ask *does any
recorded number round to this?* — never *which one, and was it about the same thing?*

That is the hole the collection's stress test measured. It catches 23 of 24 mutations of a
real answer; the survivor is a wrong value that collides with an unrelated figure elsewhere
in the ledger. It degrades with ledger size exactly as you would expect: 19% of the integers
1–99 are already present in a two-call ledger, 96% in a day's.

**Smallest fix with the most leverage: report the matched path per numeral in `--json`.**
That turns attestation from a boolean into an audit trail. "377 matched
`result.rows[7].weight`" is visibly nonsense; "attested" is not. It requires keeping a map
instead of a `Vec<f64>` and nothing else.

Then, optionally, proximity: a numeral within a short window of the word `vigor` should
prefer a `*.vigor` path over any other. Even a weak version collapses the collision surface,
and the ordering rule already in `attest.rs` — match first, excuse second — means it can
only ever tighten the check.

## 2. `VOUCH_SESSION` defaults to the date, silently, and the ledger path is unreachable

Two halves of one problem.

`ledger.rs:29` falls back to `%Y%m%d`. That is the loosest scope the tool has, it is the
default, and nothing warns. Per item 1, it is also the scope at which attestation stops
working. **Print a line on stderr when the fallback fires**, and say the ledger's size in the
human report where a person will see it.

Worse, `sanitize()` rewrites the id it was given, so a caller cannot construct the path to
the ledger it just wrote. A session named `models-x-ai-grok-4.6` lands in
`session-models-x-ai-grok-4-6.jsonl`. That collection built the path by interpolation, missed,
`attest` exited 2, and the answer was silently downgraded to UNCHECKED — the one check that
matters, lost to a filename. The workaround was to reimplement `sanitize` in Python
(`web/engine.py::safe_session`).

**Fix: `vouch attest --session <id>`**, resolving the path internally, and print the resolved
path in the report. `--ledger` stays for naming a file directly.

## 3. The runtime cannot see omission

`ledger.rs::flatten` deliberately drops strings and booleans: *"Booleans, strings and null
carry no figure to attest against."* True for fabrication. False for the other half of the
problem — an answer that quietly leaves something out. Every figure in it attests.

That collection wrote `web/completeness.py` for this: the lookup nodes resolve what the user
named, so an answer that never mentions a resolved entity dropped something. It found real
failures. The check is completely general and it lives in one app because the runtime has
nowhere to put it.

**Fix: record string leaves in the ledger too, and add `vouch attest --mentions`.** Today the
second most useful check in that project is outside the tool.

## 4. A guard that lives in one node is not a guard — *done, and re-scoped* (`2deb9b3`)

**The example this item was written around does not work.** *"Any node taking `weapon` must
satisfy that the weapon exists"* requires reading the catalogue, and a CEL contract sees
`input` and `result` with no custom functions registered. It was never expressible and no
amount of collection-level plumbing makes it so.

That splits the item cleanly. The **data-dependent** guards — bugs 4, 22, 23 and 24, every one
of which needs to consult a table — are item 10's job, and item 10 is what closes them: a node
refusing on its own data is the only thing that can read one. The **structural** half is what
`[[requires]]`/`[[ensures]]` in `registry.toml` now covers, and it is the half where a rule
gets written into nine manifests and forgotten in the tenth.

The original text follows, for the reasoning that still stands.


`requires` are per-node CEL expressions in per-node manifests. There is no way to say
something about the collection.

Bug 22 in that repository is bug 4 in two nodes written *after* bug 4 was fixed. The class is
the one the whole project exists to prevent: a well-formed number for a thing that does not
exist. A sacred seal pricing a sorcery at 798.766 — the spell buff right, the multiply right,
and the cast impossible. Contracts caught none of them, because nothing in a schema knew the
cast had to be possible, and the node that *did* know was a different node.

**This is the highest-value change in this document.** Collection-level contracts in
`.vouch/registry.toml` that apply to every node declaring a given parameter — *any node
taking `weapon` must satisfy that the weapon exists* — turn a lesson that gets re-learned
into something the runtime enforces once. A shared expression library (an `#include`, or
named contracts referenced by manifests) is the smaller version and would still have caught
bug 22.

## 5. Silent defaults are invisible to the caller and to the ledger — *done in part* (`a6fd41f`)

**Done:** the judgement half. `judgement = true` and `options` in a manifest, a refusal at exit
17 carrying both, and every describe shape publishing which parameters will ask.

**Still open:** recording the *effective* input in the ledger. A node applies its own defaults
internally, so the runtime cannot see them; the fix would be for `vouch` to apply JSON Schema
`default` values itself and record which it applied, which also removes the reason a node has
to implement defaults at all. That is a behaviour change for any collection already carrying
defaults in a schema, so it wants its own commit and its own thought.


Bugs 17 through 21 there are one bug. A JSON Schema `default` is applied with nothing saying
so, and the ledger records the input as *given*, not as *used*. On one question a model
assumed two-handing and returned 383 where one-handed is 342: internally consistent,
attested, and answering a question nobody asked.

Two fixes, both small:

- **Record the effective input alongside the given one.** An audit that cannot see which
  defaults were applied is not an audit.
- **Let a manifest mark a parameter as a judgement** — `params.<name>.judgement = true` —
  and have the runtime *refuse* when it is omitted, naming it in the reason.

The second is the interesting one. That collection's app invented an `ask` branch in its
planning prompt so a model could hand a missing judgement back to the person. But the
collection is the thing that knows a judgement is needed, and right now it has no way to say
so. This promotes `ask` from a prompt convention every integrator reinvents into a property
of the collection.

## 6. Nothing in the tool knows what a token costs — *done* (`5ba5c9f`)

`describe --all --json` on that collection is 345KB, pretty-printed, with `$schema` and
`title` repeated in every node. It is read once per session and then re-sent on every routing
decision — six to sixteen per question.

Measured there, as the planning prompt's fixed prefix:

| | chars | ≈ tokens |
|---|---|---|
| as the runtime emits it | 112,189 | 28,000 |
| after the app's own trimming | 75,711 | 18,900 |

That 33% was `$schema` and `title` removed, `params.*.guidance` folded into the schema
`description` that duplicates it, one example per node instead of all of them, and no
indentation. All of it is generic. All of it had to be written in the app.

**Ship `--compact`** (exactly that) **and `--index`** (name, purpose, `use_when`, `not_for`
only, for two-stage routing).

And note the manifest-spec bug underneath: `params.<n>.guidance` and the schema property's
own `description` are two fields for one job. Every property in every node there had both.
`markdown.rs::parameters` already has to choose between them at render time.

## 7. `vouch eval` is the command that spends money and has no controls — *done, minus one*

`--resume` and `--max-calls` are in. Stop-on-limit turned out to be `d2cffc7`, already done
before this was written. `--max-spend` in dollars is not implementable: the agent is any
command and reports a reply, not a bill, so `--max-calls` is the honest unit.


No `--resume`, no `--max-spend`, and a provider limit fails every remaining case in turn
rather than stopping the run — one pattern there produced ten "errors" in fifteen seconds,
all the same session limit, each looking like a case that had been tried.

That collection wrote `scripts/run_battery.py` and `scripts/compare_models.py`, and both
learned all three lessons separately and outside the tool. It is also why its eval suite is
still the original 16 cases against nineteen nodes, with eight nodes never once in front of a
live model: the command that would measure routing is the command that can eat an account.

**`--resume`, `--max-spend`, and stop-on-limit belong in `eval`.** The runners have the code.

## 8. `vouch test` will happily pin a bug

`cases.rs` takes any `expect_code` and treats them all alike. There is no distinction between
an expected *refusal* (11, 14, 15) — a node correctly saying no — and an expected *defect*
(20, 21) — a node crashing.

A fixture there asserted that `buff-stack` exiting 20 on an unknown buff name was correct.
250 green fixtures then affirmed that bug until a model tripped over it in the app. The
fixture was not wrong about what the code did; it was wrong about what the code should do,
and the suite had no way to notice.

**Fix: warn on any case expecting 20 or 21.** Pinning a crash is nearly always pinning a
defect. A one-line note in the report would have caught this one.

## 10. A node has no way to say no — *done* (`43d5317`)

This is the root cause of items 4's whole family, and it was found by fixing the third
instance of it rather than by reading the code.

A node can do exactly two things: exit 0 with a value on stdout, or exit non-zero. Every
non-zero exit becomes `NODE_CRASHED` (20) — a **defect**, which `verify.rs` treats as a
broken node. There is no exit code meaning *"I understood the question and the answer does
not exist"*.

But that is the single most common thing a well-written node needs to say. The refusal codes
that do exist (11, 14, 15) all belong to the runtime: preconditions, timeouts, unevaluable
contracts. A precondition is a CEL expression over the input, so it cannot answer *is this
weapon in the catalogue* or *is this item in the effect table* — the questions that actually
need refusing, every one of which requires reading the node's own data.

So a node author who wants to refuse has two options, and both are bad:

1. `sys.exit(1)`, which reads as a defect. The caller is told the node is broken, the loop
   stops, and the user gets nothing — including nothing about the rest of their question.
   That is bug 23 in the collection, and its `item-effect` had been doing it since it was
   written.
2. Return a success carrying an "it isn't there" shape. This is what `weapon-lookup` does and
   it is the better answer, but it costs an output-schema field, a contract, and a decision
   about every size invariant that assumed the node always describes something. It has to be
   re-invented per node, which is precisely why it is in some nodes and not others.

**Give a node a refusal exit code** — a documented status (say 3) that `exec::run` maps to a
refusal with the node's stderr as the reason, alongside the existing crash path. Then option
1 becomes correct instead of wrong, "no answer exists" is one line in any language, and the
distinction the exit-code families already promise callers is one a node can actually make.

This also sharpens item 8: a fixture expecting 20 is nearly always pinning a bug, and right
now it is the only way to pin a legitimate refusal a node raises itself.

## 9. Broken-pipe panic

`vouch <cmd> | head` can panic. Already recorded in `DECISIONS.md` as known roughness; noted
here only so the list is complete.

---

## What is left

Four are done. These six are not, in the order they are worth doing:

1. **Item 1** — matched paths in `attest --json`. `commands.rs` builds a map of
   `{entry}:{path}` to value and then calls `.values()` on it. Keeping it turns the strongest
   check in the tool from a boolean into evidence: *"377 matched `result.rows[7].weight`"* is
   visibly nonsense where "attested" is not.
2. **Item 7** — `eval --resume`, `--max-spend`, stop-on-limit. It is the command that spends
   money and the only one with no controls, which is why the collection that needed it most
   has a suite that has not been run end to end.
3. **Item 2** — `attest --session`, and a warning when the session id falls back to the date.
   Two halves of one silent failure.
4. **Item 8** — warn when a fixture pins exit 20 or 21. It would have caught two fixtures that
   were affirming bugs, one of them for as long as its node existed.
5. **Item 3** — strings in the ledger and `attest --mentions`, so the runtime can see an
   answer that left something out.
6. **Item 9** — the broken-pipe panic.

The remainder of item 5 belongs here too: applying JSON Schema defaults in the runtime and
recording which were applied, which is the only way the ledger can show what a node actually
used rather than what it was handed.

Items 2, 8 and 9 are each an afternoon and each closes a way to be silently wrong, which is
the failure mode this runtime is for.
