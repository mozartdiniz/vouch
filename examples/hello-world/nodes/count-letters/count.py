#!/usr/bin/env python3
"""Count how many times a letter appears in a word.

The whole node protocol is these few lines: read one JSON object from stdin, write one JSON
object to stdout. Anything else you want to say goes to stderr.
"""

import json
import sys

request = json.load(sys.stdin)
word = request["word"]
letter = request["letter"]

count = word.lower().count(letter.lower())

# stdout is the payload channel. Logs go to stderr, or the runtime rejects the call as a
# protocol violation (exit 21).
print(f"counting {letter!r} in {word!r}", file=sys.stderr)

json.dump(
    {
        "word": word,
        "letter": letter,
        "count": count,
        "word_length": len(word),
    },
    sys.stdout,
)
