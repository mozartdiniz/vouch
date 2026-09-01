#!/bin/sh
# Gets it wrong first, reads the refusal, and fixes the argument — the routing correction of
# §4.2, which is the behaviour worth measuring.
case "$1" in
  *"Answer the user's question"*)  echo "Doubling 21 gives 42." ;;
  *"Calls made so far"*)           echo '{"done": true}' ;;
  *"rejected by the runtime"*)     echo '{"call": {"node": "ok", "input": {"n": 21}}}' ;;
  *)                               echo '{"call": {"node": "ok", "input": {"n": -1}}}' ;;
esac
