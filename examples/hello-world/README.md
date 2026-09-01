# hello-world

The smallest useful collection: one node, two files, no data.

```console
$ vouch -C examples/hello-world call count-letters --input '{"word":"strawberry","letter":"r"}'
{
  "count": 3,
  "letter": "r",
  "word": "strawberry",
  "word_length": 10
}
```

## Why counting letters

Because it is the shortest possible demonstration of the problem. Ask a language model how
many r's are in "strawberry" and you will often get 2. Not because the model is bad at
counting, but because it isn't counting at all — it is predicting what the answer looks like.

`count-letters` counts. The 3 above came from `word.lower().count(letter.lower())` in
`count.py`, and the runtime verified it before printing it.

That is the whole idea. Everything else in `vouch` is machinery for making the same
guarantee hold for numbers that matter more than this one.

## The node

Two files. `count.py` is the entire implementation:

```python
request = json.load(sys.stdin)
count = request["word"].lower().count(request["letter"].lower())
json.dump({..., "count": count, "word_length": len(request["word"])}, sys.stdout)
```

One JSON object in on stdin, one JSON object out on stdout. There is no library to import
and no framework to satisfy. A node in any language that can read stdin qualifies.

`node.toml` is the contract. The interesting half is the bottom:

```toml
[[requires]]
expr = "size(input.letter) == 1"
message = "`letter` must be exactly one character; ..."

[[ensures]]
expr = "result.count <= result.word_length"

[[ensures]]
expr = "result.word_length == size(input.word)"
```

`requires` runs **before** the node and sees only `input`. `ensures` runs **after** it exits
and sees both `input` and `result`.

That last postcondition is the one worth studying. It checks that the node measured the word
it was actually handed, rather than some other string it picked up along the way. A
postcondition is most useful when it ties the output back to the input, because that is the
join a fabricated answer cannot fake.

## Three things to try

**A precondition refusing.** Ask for a two-character "letter":

```console
$ vouch -C examples/hello-world call count-letters --input '{"word":"strawberry","letter":"rr"}'
{"outcome":"refusal","code":11,"node":"count-letters","reason":"`letter` must be exactly one
character; to count a multi-character substring, this node is not the right tool"}
$ echo $?
11
```

Exit 11 is a *refusal*: no answer is available, and the message says what to do instead.
Nothing was written to stdout.

**A postcondition catching a broken node.** In `count.py`, change `len(request["word"])` to
`len(request["word"]) + 1`. The count itself is still right, but `word_length` no longer
matches the input:

```console
{"outcome":"defect","code":13,"node":"count-letters","reason":"postcondition failed:
result.word_length == size(input.word)"}
$ echo $?
13
```

Exit 13 is a *defect*: the node is broken, and the caller should stop trusting it. Again
nothing reached stdout — a wrong answer is worse than no answer, so no value is returned.

**Corrupting the payload channel.** Change the `print(...)` in `count.py` from
`file=sys.stderr` to a plain `print(...)`. Now the log line lands on stdout ahead of the JSON:

```console
{"outcome":"defect","code":21,"node":"count-letters","reason":"stdout is not a single valid
JSON object: expected value at line 1 column 1","details":{"hint":"stdout carries the result
object and nothing else; send logs and progress to stderr","stdout":"counting 'r' in
'strawberry'\n{\"word\": \"strawberry\", ...}"}}
```

Exit 21. stdout carries the result and nothing else; every other thing your node wants to say
goes to stderr. This is the mistake every contributor makes once.

## Testing it

```console
$ vouch test
count-letters
  ok    strawberry has three rs
  ok    matching is case-insensitive
  ok    a letter that is not there counts zero
  ok    a multi-character letter is refused
  ok    an empty word is refused

5 cases, 5 passed, 0 failed
```

`cases.toml` sits beside the node. Fixed input, expected exit code, expected values at paths
rooted at `result` — no model and no network, so it runs in milliseconds and either passes or
does not.

`.vouch/evals.toml` is the other layer: the same question in plain English, put to a model.

```console
$ vouch eval --agent "claude -p {prompt}" -n 10
```

It asserts that the agent called `count-letters` with `strawberry` and `r`, and that the
sentence it wrote afterwards attests clean against the ledger — the whole pitch, end to end.
It costs tokens, so nothing in `cargo test` runs it.

## Where to go next

[`../support-triage`](../support-triage) — three nodes across two languages, a CSV, and a
caller choosing between them based on what the first node returned.
