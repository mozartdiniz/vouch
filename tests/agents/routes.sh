#!/bin/sh
# Calls the node, then narrates a figure that came from it.
case "$1" in
  *"Answer the user's question"*) echo "Doubling 21 gives 42." ;;
  *"Calls made so far"*)          echo '{"done": true}' ;;
  *)                              echo '{"call": {"node": "ok", "input": {"n": 21}}}' ;;
esac
