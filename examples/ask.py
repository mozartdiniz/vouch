#!/usr/bin/env python3
"""Ask a question in English; get an answer computed by nodes, or an honest "I don't know".

This is the smallest realistic agent loop around `vouch`. The point it exists to make is that
the three exit-code families each drive a different behaviour:

    refusal  (11, 14, 15)      no answer is available  → correct course, then concede
    defect   (12, 13, 20, 21)  the node is broken      → stop; report it; never answer
    success  (0)               contracts held          → let the model narrate the numbers

The model does two jobs, and neither is arithmetic. It chooses which node to call and builds
that node's arguments, one call at a time, using the results of earlier calls. Then, once the
runtime has verified everything, it turns those results into a sentence. Every number in the
answer came out of a node.

Usage:
    ./ask.py -C hello-world    "how many r's are in strawberry?"
    ./ask.py -C support-triage "what do we owe on ticket T-1001?"
    ./ask.py -C support-triage "what do we owe on ticket T-1006?"
    ./ask.py -C ds3-tools      "what should I level for a Zweihander build at SL125?" -v

The trace goes to stderr and the answer to stdout — the same split `vouch call` uses — so
`./ask.py ... > answer.txt` captures the answer alone.

Environment:
    VOUCH_BIN   the vouch binary          (default: vouch)
    VOUCH_LLM   a command taking a prompt (default: claude -p)
"""

import argparse
import json
import os
import re
import shlex
import subprocess
import sys

VOUCH = os.environ.get("VOUCH_BIN", "vouch")
LLM = os.environ.get("VOUCH_LLM", "claude -p")

# How many decisions the model may make before the loop gives up. Enough for a couple of
# chained calls plus a correction, and low enough that a confused model cannot spin.
MAX_DECISIONS = 6

# Exit codes, mirroring the outcome families this script is demonstrating.
ANSWERED = 0
NO_ANSWER = 1
BROKEN_NODE = 2


def trace(message=""):
    print(message, file=sys.stderr)


# --------------------------------------------------------------------- the runtime


def vouch(collection, *args):
    return subprocess.run([VOUCH, "-C", collection, *args], capture_output=True, text=True)


def vouch_report(proc):
    """The structured JSON object vouch puts on stderr for every non-zero exit."""
    lines = proc.stderr.strip().splitlines()
    try:
        return json.loads(lines[-1])
    except (IndexError, json.JSONDecodeError):
        return {"outcome": "error", "reason": proc.stderr.strip() or "no reason given"}


def load_catalog(collection):
    """The routing context a collection publishes about itself.

    One command gets the lot: the registry preamble — what this set of nodes covers, and how
    its nodes relate — plus each node's purpose, when to reach for it, when not to, and how
    to fill its parameters.

    Deliberately absent are the preconditions. A caller learns those the way the design
    intends: by being refused, and told why in words written to be acted on.
    """
    described = vouch(collection, "describe", "--all", "--json")
    if described.returncode != 0:
        sys.exit(f"cannot read the collection: {vouch_report(described)['reason']}")

    described = json.loads(described.stdout)
    catalog = {
        "collection": described["collection"],
        "about": described.get("description"),
        # Collection-level rules that belong to no single node — "call triage first", "these
        # figures all come from one CSV". Without these the model has to infer them.
        "notes": described.get("notes", []),
        "nodes": [
            {
                "node": node["name"],
                "purpose": node["purpose"],
                "use_when": node["use_when"],
                "not_for": node["not_for"],
                "parameters": node["params"],
                "input_schema": node["input_schema"],
                "examples": node["examples"],
            }
            for node in described["nodes"]
        ],
    }
    return catalog


# ------------------------------------------------------------------------ the model


def ask_model(prompt, verbose=False):
    if verbose:
        trace("\n--- prompt ---\n" + prompt + "\n--- end of prompt ---\n")
    proc = subprocess.run(shlex.split(LLM) + [prompt], capture_output=True, text=True)
    if proc.returncode != 0:
        sys.exit(f"the model command ({LLM}) failed: {proc.stderr.strip()}")
    return proc.stdout.strip()


def parse_json_reply(text):
    """Models sometimes wrap JSON in a fence or a sentence. Take the outermost object."""
    fenced = re.search(r"```(?:json)?\s*(.+?)```", text, re.S)
    if fenced:
        text = fenced.group(1)
    start, end = text.find("{"), text.rfind("}")
    if start == -1 or end == -1:
        raise ValueError(f"no JSON object in the model's reply: {text[:200]}")
    return json.loads(text[start : end + 1])


PLANNING_RULES = """\
You are driving a set of verified functions, called nodes, to answer a question. You are NOT
answering it yourself.

Reply with ONLY a JSON object, in one of three shapes:

  {"call": {"node": "<name>", "input": {...}}}
      Run a node. `input` must satisfy that node's input_schema exactly. Build the arguments
      from the question and from the verified results of earlier calls — never from your own
      guesses. If you need a figure you do not have, call the node that produces it first.

  {"done": true}
      The verified results so far are enough to answer the question.

  {"stop": "<one sentence>"}
      No node can answer this, or a node has told you the answer does not exist. Say what you
      cannot answer and why.

Stopping is a good answer when it is the true one. Never pick a node that is merely close,
and never fill a parameter with a number you invented — a wrong answer is worse than none.
"""


def plan(question, catalog, steps, correction=None, verbose=False):
    prompt = [
        PLANNING_RULES,
        "\nThe collection you are working with:\n",
        json.dumps(catalog, indent=2),
        f"\n\nThe user asked: {question}\n",
    ]

    if steps:
        prompt.append("\nCalls made so far, and their verified results:\n")
        for step in steps:
            prompt.append(
                f"  {step['node']}({json.dumps(step['input'])})\n"
                f"    → {json.dumps(step['result'])}\n"
            )

    if correction:
        prompt.append(
            "\nYour last call was rejected by the runtime.\n"
            f"  you called: {correction['node']}({json.dumps(correction['input'])})\n"
            f"  the runtime said: {correction['reason']}\n\n"
            "That message is a correction you can act on. Fix the arguments, call a different "
            "node, or stop if there is genuinely no answer to be had.\n"
        )

    return parse_json_reply(ask_model("".join(prompt), verbose))


def narrate(question, steps, verbose=False):
    verified = "\n".join(
        f"From {step['node']}:\n{json.dumps(step['result'], indent=2)}" for step in steps
    )
    prompt = f"""\
Answer the user's question in one to three plain sentences, using ONLY the values in the
verified results below.

Do not calculate anything. Do not introduce any number that does not appear below. If the
results do not contain what was asked for, say so plainly.

The user asked: {question}

{verified}
"""
    return ask_model(prompt, verbose)


# --------------------------------------------------------------------- attestation


def attested(collection, prose, question):
    """Check the model's sentences against the ledger before showing them to anyone.

    Everything upstream of here is enforced: schemas, contracts, exit codes. This is the one
    step where the model writes figures of its own accord, and so the one place a number can
    still be invented — a transposed digit, a total it helpfully recomputed. `vouch attest`
    reconciles every numeral against what the nodes actually returned. No model is involved
    in the check.
    """
    proc = vouch(collection, "attest", "--text", prose, "--question", question)

    if proc.returncode == 0:
        trace("→ attested: every figure traces to a verified result")
        return True

    if proc.returncode == 1:
        trace("→ ATTESTATION FAILED — the answer contains figures no node produced:")
        for line in proc.stderr.strip().splitlines()[1:]:
            trace(f"  {line}")
        print(
            "I don't know. I had verified results, but the answer written from them "
            "contained figures the ledger cannot account for, so I am not repeating it."
        )
        return False

    # Exit 2: attestation could not run. Refusing to answer on a technicality would be worse
    # than saying so, but the answer must not be presented as checked either.
    trace(f"→ could not attest: {proc.stderr.strip()}")
    trace("  the answer below is UNCHECKED")
    return True


# ------------------------------------------------------------------------- the loop


def answer(question, collection, verbose=False):
    catalog = load_catalog(collection)
    names = [n["node"] for n in catalog["nodes"]]
    trace(f"→ {catalog['collection']}: {len(names)} node(s) — {', '.join(names)}")

    steps = []
    correction = None

    for _ in range(MAX_DECISIONS):
        decision = plan(question, catalog, steps, correction, verbose)
        correction = None

        # The model declined. This is the honest out-of-scope path — and when it follows a
        # refusal, it is the model relaying a "no" the runtime established.
        if "stop" in decision:
            trace("→ stopping: no answer available")
            print(f"I don't know. {decision['stop']}")
            return NO_ANSWER

        if decision.get("done"):
            if not steps:
                print("I don't know. Nothing was computed to answer this.")
                return NO_ANSWER
            trace("→ done; narrating from verified results")
            prose = narrate(question, steps, verbose)

            # The model has now written numbers of its own accord, which is the one place in
            # this whole flow where a figure could be invented. Check it before printing.
            if not attested(collection, prose, question):
                return NO_ANSWER

            trace()
            print(prose)
            return ANSWERED

        call = decision["call"]
        node, node_input = call["node"], call.get("input", {})
        trace(f"→ calling {node}({json.dumps(node_input)})")

        proc = vouch(collection, "call", node, "--input", json.dumps(node_input))

        if proc.returncode == 0:
            result = json.loads(proc.stdout)
            trace(f"  exit 0, contracts held: {json.dumps(result)}")
            steps.append({"node": node, "input": node_input, "result": result})
            continue

        report = vouch_report(proc)
        outcome, reason = report.get("outcome"), report.get("reason", "")
        trace(f"  exit {proc.returncode} ({outcome}): {reason}")

        # A defect means the node is broken. Retrying is pointless and answering anyway
        # would be dishonest, so the loop stops here rather than working around it.
        if outcome == "defect":
            print(
                f"I can't answer that. The `{node}` node is broken — {reason}. "
                "That needs reporting, not retrying."
            )
            return BROKEN_NODE

        # A refusal, or an input the schema rejected, is a correction. Hand it back.
        trace("  feeding the correction back to the model")
        correction = {"node": node, "input": node_input, "reason": reason}

    print("I don't know. I ran out of attempts before reaching a verified answer.")
    return NO_ANSWER


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("question", help="a question, in plain English")
    parser.add_argument(
        "-C",
        "--collection",
        default="support-triage",
        help="collection directory (default: support-triage)",
    )
    parser.add_argument(
        "-v", "--verbose", action="store_true", help="show the prompts sent to the model"
    )
    args = parser.parse_args()

    collection = args.collection
    if not os.path.isdir(collection):
        collection = os.path.join(os.path.dirname(os.path.abspath(__file__)), args.collection)

    return answer(args.question, collection, args.verbose)


if __name__ == "__main__":
    sys.exit(main())
