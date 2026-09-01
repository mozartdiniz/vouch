#!/bin/sh
# Calls a node that is not there, then gives up.
case "$1" in
  *"no node called"*) echo '{"stop": "I named a node that does not exist"}' ;;
  *)                  echo '{"call": {"node": "nonexistent", "input": {}}}' ;;
esac
