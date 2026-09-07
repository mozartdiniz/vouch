# `vouch` — MVP Specification

> Working name. Do not call this project "deterministic anything" — see §1.2.

A local, language-agnostic runtime for executing contract-checked functions, designed to
be driven by an LLM agent but fully usable from a plain shell.

---

## 1. Purpose

### 1.1 The problem

When an LLM agent answers a question involving numbers, the numbers can come from two
places: a computation, or token prediction. Today there is no way to tell which. Tool-calling
frameworks validate the *input* to a tool against a schema and then trust whatever the tool
returns; nothing validates the return value, and nothing checks that the numbers in the
agent's final prose match the numbers its tools actually produced.

`vouch` closes both gaps.

### 1.2 The property being guaranteed

**Value provenance**, not determinism.

Every number in the final answer must originate from a function's return value rather than
from the model. This is deliberately *not* the same as determinism:

- An HTTP call to a live rates API is nondeterministic and perfectly provenanced.
- An LLM reciting a cached number from memory is deterministic and fabricated.

Nodes are therefore **allowed to do I/O** — read files, query databases, make HTTP calls.
Purity is not required and must not be enforced. What is enforced is that the result passed
contracts, and that its origin is recorded.

Avoid the word "deterministic" in docs, README, and CLI help. It invites correct-but-irrelevant
objections about the HTTP nodes.

### 1.3 The guarantee, stated precisely

> The runtime is **sound but incomplete**. It never returns an unsound answer. It may
> return nothing.

Every call ends in exactly one of: a value that satisfied its contracts, or a refusal with a
machine-readable reason. There is no third outcome and no partial/degraded/best-effort result.

### 1.4 Non-goals

- Not a graph/traversal engine. Nodes are independent; there is no inference layer.
- Not a Datalog/logic-programming system. (Explicitly avoided.)
- Not an MCP server. Plain CLI only — see §4.
- Not a router. The runtime *publishes* routing guidance; the agent decides. Never map a
  natural-language question to a node inside the tool.
- Not a workflow/orchestration engine. No chaining in the MVP.

---

## 2. Architecture

```
┌──────────────────────────────────────────────┐
│  Rust core (single static binary)            │
│                                              │
│  registry  → discovers node.toml manifests   │
│  schema    → JSON Schema validation (in/out) │
│  contracts → CEL evaluation (requires/ensures)│
│  exec      → subprocess, timeout, stdio      │
│  ledger    → append-only record of results   │
│  attest    → scalar reconciliation vs prose  │
└──────────────────────────────────────────────┘
                    │  JSON on stdin / stdout
                    ▼
        node implementation (any language)
```

**Language: Rust.** Reasons: single-binary distribution with no runtime dependency, fast
cold start (the binary is spawned per call), and mature crates for every piece below. Do not
use Python for the core.

**Nodes are subprocesses.** JSON object on stdin, JSON object on stdout. Any language.
The contract enforcement lives entirely in the Rust core, so the guarantee holds regardless
of how sloppy the node implementation is. This is the point: contributors can vibe-code the
inside of the box; the box is not vibe-coded.

### 2.1 Crates

| Concern | Crate |
|---|---|
| CLI | `clap` (derive) |
| JSON | `serde`, `serde_json` |
| JSON Schema | `jsonschema` (draft 2020-12) |
| Contracts | `cel` (from `cel-rust/cel-rust`) |
| Process + timeout | `tokio` (`process`, `time::timeout`) |
| Manifests | `toml` |
| Hashing | `sha2` |

### 2.2 Layout on disk

```
project/
  .vouch/
    registry.toml          # collection-level preamble (§5.2)
    ledger/
      session-<id>.jsonl   # append-only call records
  nodes/
    stat-optimizer/
      node.toml
      cases.toml
      optimize.py
    weapon-lookup/
      node.toml
      ...
```

---

## 3. The node contract

### 3.1 Manifest

```toml
name = "stat-optimizer"
version = "0.1.0"

purpose = "Optimal stat allocation for a target weapon and soul level in Dark Souls 3"

# --- routing context (§5) ---
use_when = [
  "the question names a specific weapon or build archetype",
  "the user asks what stats to level",
]
not_for = [
  "item locations",
  "boss strategies",
  "lore questions",
]

# --- execution ---
run = ["python3", "optimize.py"]
timeout_ms = 5000

# --- declared inputs (provenance, not caching) ---
[[reads]]
path = "data/weapons.csv"

# --- interface ---
[input]
schema = "input.schema.json"     # or inline via input.properties

[params.weapon]
guidance = "Exact in-game weapon name. Call weapon-lookup first if unsure."

[params.soul_level]
guidance = "Target soul level. If the user does not say, assume 125."

[output]
schema = "output.schema.json"

# --- contracts (CEL) ---
requires = [
  "input.soul_level >= 1 && input.soul_level <= 802",
  "input.weapon != ''",
]
ensures = [
  "result.total_points_spent <= input.soul_level",
  "result.attack_rating > 0.0",
  "result.stats.strength >= 10",
]

[[examples]]
ask  = "best stats for a Lothric Knight Sword build at SL120"
call = { weapon = "Lothric Knight Sword", soul_level = 120 }
```

### 3.2 Contract language: CEL

Use CEL (Common Expression Language). It is non-Turing-complete, terminates by
construction, sandboxed, and is a Google spec with implementations in several languages —
so contributors can read a contract without learning a project-specific DSL. It is also not a
logic-programming language, which is a hard requirement here.

- `requires` expressions see `input`. Evaluated **before** the subprocess starts.
- `ensures` expressions see both `input` and `result`. Evaluated **after** the subprocess exits.

**Fail closed.** If a CEL expression errors rather than returning a boolean — undefined field,
type mismatch, division by zero — that is a *failure*, not a pass. A contract that cannot be
evaluated is a contract that did not hold. This is the single most important rule in the
codebase; if an evaluation error ever falls through as success, the entire guarantee is gone
and nobody will notice for months. Cover it with tests.

**Type handling.** Do not infer CEL types from the incoming JSON. Build the CEL environment
from the JSON Schema, which already declares `integer` vs `number` vs `string`. One source of
truth. A payload that disagrees with its schema is then a schema violation (exit 10) rather
than a mysterious contract failure. Pay specific attention to integer/float coercion — this is
the most likely source of spurious failures.

### 3.3 Contract strength

Refuse to load a node whose `ensures` list is empty or whose expressions never reference
`result`. A vacuous postcondition (`result != null`) gives an agent *more* confidence than no
postcondition while checking nothing — false assurance is worse than absent assurance.

`describe` should report contract strength (number of `ensures`, whether they reference
numeric output fields) so a consumer can weight the answer.

### 3.4 Judgement parameters

Some parameters have no right answer in the data. How much vigor a build should hold back,
which starting class to assume, whether a weapon is two-handed — the answer moves and no table
settles it.

A node has two bad options and one good one. It can **pick** a value, which presents an opinion
as a calculation. It can **require** one and say nothing more, which leaves the caller to
produce a value from nowhere — and a model asked to do that produces a different one each run:
measured on one collection, fourteen of fifteen questions where a model had to invent a
judgement drifted between repeats, against two of five where none did. Every run was
internally consistent and every figure traceable. Nothing was wrong except that the answer
moved.

The third option is to say so:

```toml
[params.vigor]
guidance = "The minimum vigor to hold back. There is no right answer in the data."
judgement = true
options = [
  { label = "balanced PvE", vigor = 40, mind = 20 },
  { label = "survivability first", vigor = 60, mind = 20 },
]
```

A call that omits it refuses with **exit 17**, the parameter named in `details.judgement`, the
guidance, and `options` carried through verbatim. `options` is free-form: only the collection
knows whether a choice is one value or a set that go together, and a caller renders them rather
than interpreting them.

Exit 17 and not 11, because they are different instructions to whoever is driving. `11` says
the call was wrong — fix the arguments or route elsewhere. `17` says the call was fine and one
value in it belongs to a person who has not been asked. A caller that cannot tell them apart
either interrogates the user about genuine mistakes or invents an answer to a real question.

Judgements are published everywhere a node is described, because a router that learns this
from a refusal has already spent a decision: `--compact` carries a `judgements` map, the
markdown pack marks the parameter *"a judgement — ask, do not invent"* and lists what to offer.

### 3.5 Collection contracts

A contract in a manifest is a contract about that node. A collection frequently needs to say
something about *every* node that takes a parameter — "a stat is 1 to 99, wherever a stat
appears" — and writing it into each manifest is a habit rather than a guard: the next node to
take that parameter gets it only if someone remembers.

`.vouch/registry.toml` takes `[[requires]]` and `[[ensures]]` entries with a `when`, an `expr`
and an optional `message`:

```toml
[[requires]]
when = "strength"
expr = "input.strength >= 1 && input.strength <= 99"
message = "stats run 1 to 99"
```

`when` names the property the contract is about, in the object that contract inspects. A
`[[requires]]` attaches to every node whose **input** schema declares it; a `[[ensures]]` to
every node whose **output** schema does. Nodes that declare neither are untouched.

Two rules make them safe to write:

- **Each is scoped to the property's presence.** An optional parameter left out would make the
  expression unevaluable, which fails closed (§3.2). That is correct for a rule an author
  wrote about their own node and wrong for one written about the collection, so the runtime
  adds the guard rather than asking every author to remember it.
- **They are checked first.** A collection-wide rule is the broader statement, and a caller who
  breaks both should hear the general one.

Collection contracts do **not** relax §3.3: a node still needs a postcondition of its own. The
collection's rule is a floor, not a substitute for a node making a claim about its own output.

What they cannot express is anything requiring the node's data. A CEL contract sees `input`
and `result` and nothing else, so "the weapon must exist" is not a contract — it is a node
refusing on its own data (exit 3, §4.1).

---

## 4. CLI surface

No MCP, no server, no daemon. A plain CLI, so a shell script or a human gets the same
access as an agent.

| Command | Behaviour |
|---|---|
| `vouch list` | Node names + one-line purpose |
| `vouch describe <node> [--json]` | Full contract, params, guidance, examples |
| `vouch describe --all --md` | Markdown pack for `CLAUDE.md` (§5.3) |
| `vouch call <node> --input @file\|-\|'{json}'` | Execute one verified call |
| `vouch test [<node>]` | Run node fixture tests (§7.1) |
| `vouch eval [--agent <cmd>] [-n N]` | Run agent routing evals (§7.2) |
| `vouch attest --ledger <f> [--text @file\|-]` | Reconcile prose against ledger (§6.2) |

`vouch call` writes the result object to **stdout** and everything else to **stderr**.

### 4.1 Exit codes

Three families. The distinction between them is the product.

**Success**
- `0` — value returned, all contracts held

**Refusal** — no answer is available; the agent should try a different approach
- `11` — precondition failed (the question is outside this node's competence)
- `14` — execution timeout
- `15` — contract could not be evaluated (fail-closed, §3.2)
- `16` — the node refused: it ran, looked at its own data, and there is no answer
- `17` — a judgement parameter is missing (§3.4): the call was fine, one value in it is a
  person's to make, and `details` carries the parameter and its options

A node refuses by **exiting 3** with its reason on stderr. That is the only way it can say
"I understood the question and the answer does not exist", and it is a thing most nodes
eventually need to say — a name that is not in the table, a combination the data forbids.
`11` is the runtime refusing on the node's behalf from a precondition, which is CEL over the
*input* and therefore cannot consult the node's data; `16` is the node refusing after it has.

Exit 3 and not 1 or 2: those are an uncaught exception and an argument error in most
languages, and reading either as a considered refusal would turn a crash into an answer.
Every other non-zero status remains `20`.

**Defect** — the node is broken; the agent should stop trusting it and report it
- `12` — output failed its JSON Schema
- `13` — postcondition failed
- `20` — node crashed / non-zero exit (any status but 0 and 3)
- `21` — node protocol violation (stdout was not a single valid JSON object)

**Caller error**
- `10` — input failed its JSON Schema

Every non-zero exit emits a JSON object on stderr: `{ "outcome": "refusal"|"defect"|"caller_error",
"code": 11, "node": "...", "reason": "..." }`.

### 4.2 Refusal messages are routing corrections

Write precondition failure messages as instructions to a reader who can act on them.

- Bad: `invalid weapon`
- Good: `unknown weapon 'lothric sword'; call weapon-lookup for valid names`

The agent reads this and retries correctly. This is where context (advisory) and contracts
(enforcing) meet: context makes the right call *likely*, preconditions make the wrong call
*fail informatively*.

---

## 5. Context and routing

### 5.1 Why it exists

Users need to instruct the agent how to consume the collection: "use A for this kind of
question, B for that kind." That guidance lives in the manifests and is *published*, never
executed.

`not_for` does disproportionate work — negative examples reduce misrouting more than
positive ones, and they prevent the agent burning a turn on a refusal it doesn't understand.

### 5.2 Registry preamble

`.vouch/registry.toml` carries collection-level context: what this set of nodes covers as a
whole, and explicit disambiguation between nodes that overlap.

```toml
name = "ds3-tools"
description = "Dark Souls 3 build optimization over a local CSV dataset."
notes = [
  "All numbers come from patch 1.15 data files.",
  "Use stat-optimizer for allocation questions; weapon-lookup only resolves names.",
]
```

### 5.3 The markdown pack

`vouch describe --all --md` emits the registry preamble plus every node's purpose,
`use_when`, `not_for`, parameter guidance, and examples as markdown. The user pastes it
into `CLAUDE.md`, or the agent runs the command at session start. That is the entire
integration story — no protocol, no config file format to learn.

### 5.4 Sized for a context window

The pack is written once into a file. `describe --all --json` is different: an agent driving a
collection re-sends it on **every routing decision**, six to sixteen per question in the first
collection to be measured, where it was 28,000 tokens each time. Most of what it carries is
not read by anything routing, and a caller that noticed had to write its own trimmer.

`--compact` is that trimmer. It emits, with no whitespace: node name, purpose, `use_when`,
`not_for`, the input schema, and **one** example.

- `$schema` and `title` are dropped — a validator and a doc generator read them.
- `params.*.guidance` is folded into the schema property's `description`, keeping both texts
  where they differ. They are two fields for one job, and this is the largest single saving.
- Contracts, the output schema, `reads` and `timeout_ms` are left out, for the reason §5.3
  already gives: enforcing rather than advisory.

`--index` goes further and emits only name, purpose, `use_when` and `not_for` — enough to pick
a node, nothing to call one with. It is for a caller that wants a shortlist before reading any
schema, and it drops the collection notes, which would dwarf it.

Both imply JSON. Measured on that collection: 360,258 characters to 78,888 with `--compact`
and 12,399 with `--index`.

---

## 6. The ledger and attestation

This is the part nobody else ships, and the reason the project is worth building.

### 6.1 Ledger

Every `vouch call` appends one JSON line to `.vouch/ledger/session-<id>.jsonl`:

```json
{
  "ts": "2026-08-30T14:02:11Z",
  "node": "stat-optimizer",
  "version": "0.1.0",
  "input": { "weapon": "Lothric Knight Sword", "soul_level": 120 },
  "reads": [ { "path": "data/weapons.csv", "sha256": "9f2a..." } ],
  "outcome": "ok",
  "code": 0,
  "result": { "attack_rating": 512.4, "stats": { "strength": 16, "dexterity": 40 } },
  "scalars": { "result.attack_rating": 512.4, "result.stats.strength": 16 }
}
```

`reads` is the audit record — which file, which URL, which timestamp produced this number.
It is *not* a cache key; caching is out of scope for the MVP.

`scalars` is a flattened map of every numeric leaf in the result. Compute it in the core.

### 6.2 `vouch attest`

The runtime can guarantee its own output. It cannot guarantee what the agent writes
afterward — transposed digits, wrong units, a total the model "helpfully" recomputed, a
figure carried over from an earlier turn. All the contract work sits upstream of where
fabrication actually happens.

`vouch attest` closes it. Pipe the agent's final text in; it extracts every numeral and checks
each against the session ledger. **No LLM involved** — pure string/number reconciliation, a
few hundred lines.

The check is *set membership*: does any recorded value round to this numeral? So a figure
matched by exactly the value it is about and a figure matched by something unrelated are both
"matched", and the verdict alone cannot tell them apart. `--json` therefore reports
`accounted`: every matched numeral with the ledger paths that account for it, and how many
there were. A numeral more than one value could explain is counted in `ambiguous`, and the
human report says so.

That does not close the gap — nothing numeric can — but it makes it legible. `512 AR`
accounted for by `result.attack_rating` reads differently from the same numeral accounted for
by `result.rows[7].weight`, and `ambiguous` is the measure of how much the ledger's size is
doing the work: in a two-call ledger 19% of the integers 1 to 99 are already present, in a
day's 96%. Scoping `VOUCH_SESSION` per conversation is what keeps that number down.

Normalization rules to implement:
- Strip thousands separators: `1,234.5` matches `1234.5`
- Percent forms: `12.3%` matches `0.123` and `12.3`
- Rounding: a prose number matches if it equals a ledger scalar rounded to the prose
  number's precision (`512` matches `512.4`)
- Units suffixes are stripped before comparison (`40 kg`, `512 AR`)

Ignore-list (configurable, to keep false positives down):
- Numbers that appear verbatim in the user's own question
- Four-digit years
- Integers 0–10 appearing without a unit (ordinals, list counts)

Output: exit `0` if every numeral is accounted for; exit `1` with a list of unmatched numerals
and their positions otherwise. An agent harness can run this as a post-step and regenerate on
failure.

### 6.3 Known limit — state it in the docs

Attestation covers scalars. It verifies that the numbers are real; it cannot verify that the
*advice* is good. A workout plan's sets, reps, loads, and volume totals attest fine; "is this a
good program" is a judgment the ledger cannot check. Never let a green check imply more
than it checked.

---

## 7. Testing

Two layers. Do not conflate them — one is boolean, the other is a rate.

### 7.1 Node tests (`vouch test`)

`cases.toml` beside each node. Fixed input, expected output or expected exit code.
Deterministic, fast, ordinary unit testing. This is what catches the scaling math silently
breaking.

```toml
[[case]]
name = "known LKS build"
input = { weapon = "Lothric Knight Sword", soul_level = 120 }
expect_code = 0
expect = { "result.stats.dexterity" = 40 }

[[case]]
name = "rejects impossible soul level"
input = { weapon = "Lothric Knight Sword", soul_level = 9999 }
expect_code = 11
```

### 7.2 Agent evals (`vouch eval`)

Natural-language question in; assert that the right node was called with the right params
and that the prose attests clean. Because there is a model in the loop this is a **pass rate**,
not a pass — run each case N times and report `9/10`.

```toml
[[eval]]
ask = "what should I level for a Lothric Knight Sword build at SL120?"
expect_node = "stat-optimizer"
expect_params = { weapon = "Lothric Knight Sword", soul_level = 120 }
attest = true
```

Stay agent-agnostic: `vouch eval --agent "claude -p {prompt}" -n 10`. The runtime shells
out to whatever harness the user has.

This gives a regression signal on the failure mode that is otherwise invisible: reword a
`use_when`, routing accuracy drops from 95% to 60%, and without the eval nobody finds out.

---

## 8. Design rules for node authors

Put these in the README. They are the rules people will violate first.

1. **Return computed results, never raw rows.** If a node hands back 400 rows and lets the
   model do the arithmetic, every guarantee evaporates at the last step. The function does
   the math; the model only narrates. This is the central rule.
2. **stdout is the payload channel.** Logs, prints, and progress go to stderr. Every
   language's default logger writes to stdout and every contributor will corrupt the channel
   on day one — fail with a clear message (exit 21) rather than a parse error.
3. **Say no by exiting 3, and say why on stderr.** Ask of every node: *can this say the answer
   does not exist?* Nearly all of them need to — a name that is not in the table, a
   combination the data forbids — and the node is the only thing that can tell, because a
   precondition cannot read the node's data. The two ways of not having exit 3 are both
   damaging: `exit 1` tells the caller the node is broken and discards the rest of their
   question, and a success-shaped "it isn't there" has to be re-invented per node, so it ends
   up in some and not others. A guard that lives in one node is not a guard.
4. **Emit full precision with explicit units**, and prefer a small flat result object over a
   nested blob. Every hop the model makes through your output is an opportunity to fabricate.
5. **Give each returned value a stable key.** Attestation and evals both depend on it.
6. **Move fetches outward where practical.** A node that takes a rate as a declared parameter
   is more auditable than one that silently uses whatever the market was doing at call time.
   Not a rule, a preference.

Additionally, in the core: canonicalize JSON (sorted keys) before hashing anything, and decide
explicitly what happens to `NaN` and `Infinity`, which are not JSON and which numeric nodes
will eventually emit. Reject them at the output-schema boundary.

---

## 9. MVP scope

### In

- Registry discovery, manifest parsing, `list` / `describe` / `describe --all --md`
- JSON Schema validation on input and output
- CEL `requires` / `ensures` with fail-closed evaluation
- Subprocess execution with timeout
- Full exit-code taxonomy with structured stderr
- Ledger append + scalar flattening
- `vouch attest`
- `vouch test` (node fixtures)
- `vouch eval` (agent routing + attestation, pass rate)
- One complete demo node collection (§10)

### Deferred

- Caching / content-addressing (was justified by determinism; no longer the goal)
- Sandboxing (network/filesystem isolation) — nodes do I/O by design; revisit as a safety
  feature, not a correctness one
- Declared effect sets with deny-by-default
- Node composition / chaining. If added later: **refusal is absorbing** — any node refuses,
  the chain refuses, no partial results
- Warm worker pools for subprocess startup latency. At one call per agent turn, 150ms is
  invisible; do not optimize it yet
- Any web UI, server, or MCP transport

---

## 10. Acceptance demo

Build the Dark Souls 3 collection first. It is the strongest demo because nearly every output
is a checkable scalar — stat spreads, attack ratings, breakpoints.

Prove, in order:

1. `vouch call stat-optimizer` returns a number that satisfies its postconditions.
2. Change an input so the precondition fails → exit 11, informative message, **no value**.
3. Break the node's math so a postcondition fails → exit 13, flagged as a defect, no value
   leaks to the caller.
4. Paste `describe --all --md` into `CLAUDE.md`; ask Claude Code a natural-language build
   question; it routes correctly and returns the computed number.
5. Hand-edit one digit in the agent's answer; `vouch attest` catches it.
6. Ask the same question of a bare LLM with no tools; it produces a confident, plausible,
   wrong number.

Step 6 next to step 4 is the pitch.

---

## 11. Milestones

| # | Deliverable |
|---|---|
| M1 | Registry, manifests, schema validation, CEL contracts, subprocess exec, exit codes, `list`/`describe`/`call` |
| M2 | Routing context fields, `describe --all --md` |
| M3 | Ledger + scalar flattening + `vouch attest` |
| M4 | `vouch test` and `vouch eval` |
| M5 | DS3 demo collection + README + the six-step acceptance run |

M1 is a complete, useful tool on its own. Ship it before starting M2.