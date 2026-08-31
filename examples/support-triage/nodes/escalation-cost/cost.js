#!/usr/bin/env node
"use strict";

// The SLA credit owed on a ticket that missed its response-time commitment.
//
// A JavaScript node and a Python node are the same thing to the runtime: a subprocess that
// reads one JSON object from stdin and writes one JSON object to stdout. There is no SDK to
// install and nothing to import.

const { readFileSync } = require("node:fs");

// Monthly subscription per plan, in USD. The free tier has nothing to credit against, which
// is why a precondition refuses it before this node ever runs.
const MONTHLY_FEE_USD = {
  enterprise: 2000,
  pro: 200,
  free: 0,
};

const CREDIT_PERCENT_PER_HOUR = 5;
const MAX_CREDIT_PERCENT = 25;

const request = JSON.parse(readFileSync(0, "utf8"));
const { tier, minutes_over: minutesOver } = request;

// console.log would write to stdout and corrupt the payload channel (exit 21).
console.error(`crediting ${tier} for ${minutesOver} minutes over SLA`);

const hoursOver = Math.ceil(minutesOver / 60);
const creditPercent = Math.min(MAX_CREDIT_PERCENT, hoursOver * CREDIT_PERCENT_PER_HOUR);
const monthlyFee = MONTHLY_FEE_USD[tier];
const creditUsd = Math.round(monthlyFee * creditPercent) / 100;

process.stdout.write(
  JSON.stringify({
    tier,
    minutes_over: minutesOver,
    monthly_fee_usd: monthlyFee,
    credit_percent: creditPercent,
    credit_usd: creditUsd,
  }),
);
