#!/usr/bin/env python3
"""Look up one support ticket and work out where it stands against its SLA.

This node answers a question of fact — how long has this been open, and is that too long —
and nothing more. It does not decide what to do about the answer. The caller reads
`breached` and `tier` and picks the next node itself; see the collection README.
"""

import csv
import json
import os
import sys

# Response-time commitment per plan, in minutes.
SLA_MINUTES = {
    "enterprise": 60,
    "pro": 240,
    "free": 1440,
}


def load_tickets():
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "data", "tickets.csv")
    with open(path, newline="", encoding="utf-8") as handle:
        return {row["ticket_id"]: row for row in csv.DictReader(handle)}


request = json.load(sys.stdin)
ticket_id = request["ticket_id"]

tickets = load_tickets()
if ticket_id not in tickets:
    # See the collection README: in M1 a node has no way to *refuse*, only to fail, so an
    # id that is well-formed but absent is reported as a defect (exit 20).
    print(f"no ticket {ticket_id} in tickets.csv", file=sys.stderr)
    sys.exit(1)

ticket = tickets[ticket_id]
tier = ticket["tier"]
minutes_open = int(ticket["minutes_open"])
sla_minutes = SLA_MINUTES[tier]
minutes_remaining = sla_minutes - minutes_open

json.dump(
    {
        "ticket_id": ticket_id,
        "tier": tier,
        "category": ticket["category"],
        "minutes_open": minutes_open,
        "sla_minutes": sla_minutes,
        "minutes_remaining": minutes_remaining,
        "breached": minutes_remaining < 0,
    },
    sys.stdout,
)
