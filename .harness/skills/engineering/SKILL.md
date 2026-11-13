---
name: "engineering"
description: "Implement a change while preserving firmware, serial and UI contracts."
---

# Engineering workflow

Use for a requested code/behavior change. Inputs: expected behavior, affected platform/variant and a reproducible symptom or acceptance criterion.

1. Read [required rules](../../rules/engineering.md), [architecture](../../docs/architecture.md) and [CONTRIBUTING](../../../CONTRIBUTING.md). Locate the entry point, worker/core boundary and relevant tests.
2. Establish before/after behavior. Keep firmware framing, presets and presentation separate. An available port does not authorize physical operation.
3. Implement a cohesive change and update affected guides/callers, preserving Chinese/English behavior and original RX bytes.
4. Follow [verify](../verify/SKILL.md) for meaningful checks. Retain failures and actual hardware/OS limits.
5. Review the diff and report behavior, sources, commands/results and remaining acceptance. Follow existing task publication scope; add no invented approval gate or history rewrite.

If source contradicts prose, update the authored spec and refresh using [maintenance](../../docs/maintenance.md). Return exact evidence, not an inventory-based behavior claim.
