#!/usr/bin/env node
"use strict";

// How long a ticket still inside its SLA should expect to wait.
//
// Every figure this node needs is a declared parameter rather than something it goes and
// fetches. That is deliberate: a node that takes the queue depth as an argument can be
// audited later, because the ledger records what the depth actually was. A node that reads
// the live queue itself gives an answer nobody can reconstruct tomorrow.

const { readFileSync } = require("node:fs");

const request = JSON.parse(readFileSync(0, "utf8"));
const {
  queue_depth: queueDepth,
  agents_available: agentsAvailable,
  avg_handle_minutes: avgHandleMinutes,
} = request;

console.error(`${queueDepth} waiting, ${agentsAvailable} on shift`);

// Agents work the queue in parallel, so the ticket at the back waits one handling period
// per full round ahead of it.
const rounds = Math.ceil(queueDepth / agentsAvailable);

process.stdout.write(
  JSON.stringify({
    queue_depth: queueDepth,
    agents_available: agentsAvailable,
    avg_handle_minutes: avgHandleMinutes,
    rounds,
    estimated_wait_minutes: rounds * avgHandleMinutes,
  }),
);
