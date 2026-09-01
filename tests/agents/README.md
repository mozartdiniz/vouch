# Fake agents

Stand-ins for a model, so `cargo test` can exercise `vouch eval`'s machinery — the loop, the
corrections, the in-memory ledger, the attestation of the final prose — without spending a
token or depending on a network.

Each script takes the prompt as its single argument and replies on stdout, branching on which
turn of the loop it is being asked for. They are not a model and prove nothing about routing;
what they prove is that a routing result would be reported honestly.

They drive `tests/fixtures`, whose `ok` node doubles `n`.
