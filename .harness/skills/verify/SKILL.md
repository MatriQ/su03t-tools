---
name: "verify"
description: "Select meaningful unit, PTY, installer and package checks for a change."
---

# Verification workflow

Use after a change or for regression reproduction. Inputs: diff, expected behavior, OS and feature variant.

1. Read [testing](../../docs/testing.md) and [validation limits](../../docs/validation.md). Choose checks by affected behavior and inspect fixture/target boundaries.
2. Run native commands from the right checkout; build before passing a binary to PTY scripts. Account for CARGO_TARGET_DIR and variants sharing the debug binary.
3. For transfers distinguish Writing, WaitingAck, acknowledged data and final EOT ACK. Preserve changed-file refusal, B8 receive-only, retries and cancellation.
4. For docs/harness run portable integrity and freshness review; for moved docs inspect README/archive destinations. Installer checks use temporary prefixes.
5. Report exact commands/outcomes and retain first failures/retries. Broaden checks only for a new change, failure or unresolved concern.

Simulated transfers do not establish physical flashing. Host activation/hosted CI are separate from artifact checks. Link relevant implementation/tests and keep private raw evidence local.
