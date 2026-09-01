#!/bin/sh
# Routes correctly and then writes a number no node produced — the failure `attest` exists
# for, and the one an eval must catch even though every call succeeded.
case "$1" in
  *"Answer the user's question"*) echo "Doubling 21 gives 43." ;;
  *"Calls made so far"*)          echo '{"done": true}' ;;
  *)                              echo '{"call": {"node": "ok", "input": {"n": 21}}}' ;;
esac
