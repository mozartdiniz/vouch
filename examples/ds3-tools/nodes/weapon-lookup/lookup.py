#!/usr/bin/env python3
"""Resolve a partial or misspelled weapon name to its exact in-game spelling.

Reads one JSON object on stdin, writes one JSON object on stdout. Logs go to stderr —
stdout is the payload channel and nothing else may touch it.

Matching, in order:

  * Normalise both sides: lowercase, drop apostrophes, reduce every other punctuation
    run to a space. "Lothric's Holy Sword" and "lothrics holy sword" are the same string.
  * An exact normalised match wins outright, even when the query is also a substring of
    other names. That is the one clear way to remove ambiguity, so it is applied first:
    "Lothric Knight Sword" resolves, and never reports "Lothric's Holy Sword" alongside it.
  * Otherwise every weapon whose normalised name contains all of the query's tokens is a
    candidate.

**Ambiguity is the answer, not a failure.** When more than one weapon matches, the node
returns every candidate and leaves `resolved` empty. The caller must put the choice to the
user rather than taking the first name — that silent pick is precisely the failure this node
exists to prevent. A query that matches nothing resolves to nothing either: the weapon is
outside the dataset, which is a real answer about the data and not an error.
"""

import csv
import json
import os
import sys


def catalog():
    path = os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "..", "..", "data", "weapons.csv"
    )
    with open(path, newline="", encoding="utf-8") as handle:
        return [row["name"] for row in csv.DictReader(handle)]


def normalize(text):
    """Lowercase, apostrophe-free, single-spaced. Comparison happens only on this form."""
    text = text.lower().replace("'", "").replace("’", "")
    return " ".join("".join(c if c.isalnum() else " " for c in text).split())


def resolve(query, names):
    target = normalize(query)

    # An exact name is unambiguous by construction, whatever else it is a substring of.
    exact = [name for name in names if normalize(name) == target]
    if exact:
        return exact

    tokens = target.split()
    if not tokens:
        # No tokens would make the all() below vacuously true and match the whole
        # catalogue. Punctuation is not a query.
        return []
    return sorted(
        name for name in names if all(token in normalize(name) for token in tokens)
    )


def main():
    request = json.load(sys.stdin)
    query = request["query"]

    names = catalog()
    candidates = resolve(query, names)
    print(f"{query!r} matched {len(candidates)} of {len(names)}", file=sys.stderr)

    result = {
        "query": query,
        # Empty rather than null: a caller that reads this field gets a string either way,
        # and an empty one is never mistaken for a name.
        "resolved": candidates[0] if len(candidates) == 1 else "",
        "match_count": len(candidates),
        "candidates": candidates,
        "ambiguous": len(candidates) > 1,
        "catalog_size": len(names),
    }
    json.dump(result, sys.stdout)


if __name__ == "__main__":
    main()
