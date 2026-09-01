#!/bin/sh
# Answers the first question and then refuses to run, the way a harness does when it hits a
# usage limit — and says so on *stdout*, with nothing on stderr, exactly as `claude -p` does.
STATE="${TMPDIR:-/tmp}/vouch-quits-$PPID"
case "$1" in
  *"unladen swallow"*)
    echo "You've hit your session limit"
    exit 1
    ;;
  *"Answer the user's question"*) echo "Doubling 21 gives 42." ;;
  *"Calls made so far"*)          echo '{"done": true}' ;;
  *)                              echo '{"call": {"node": "ok", "input": {"n": 21}}}' ;;
esac
rm -f "$STATE" 2>/dev/null
