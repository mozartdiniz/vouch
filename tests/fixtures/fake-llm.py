#!/usr/bin/env python3
"""A deterministic stand-in for a language model, for testing `examples/ask.py` offline.

`ask.py` shells out to whatever `VOUCH_LLM` names, passing the prompt as the last argument.
Pointing it here exercises the whole loop — catalog loading, decision parsing, the call, the
narration — without a network round trip or a token of spend.

It answers on keywords rather than understanding anything, which is all the control flow
under test needs.
"""

import sys

prompt = sys.argv[-1]

if "Answer the user's question" in prompt:
    # The narration turn. A real model would summarise the verified results.
    print("The letter r appears 3 times in strawberry.")

elif "capital of France" in prompt:
    # Out of scope: no node covers it, so decline without calling anything.
    print('{"stop": "no node in this collection answers geography questions"}')

elif "Calls made so far" in prompt:
    # A call has already succeeded, so there is enough to answer.
    print('{"done": true}')

else:
    print('{"call": {"node": "count-letters", "input": {"word": "strawberry", "letter": "r"}}}')
