# Engineering constraints

Scope: firmware, UI/CLI, diagnostics, packaging and harness. [CONTRIBUTING](../../CONTRIBUTING.md) is the standing authority; [protocol](../docs/protocol.md) and [validation](../docs/validation.md) define behavior/evidence limits.

- Use synthetic transports/PTys in tests. Never silently open attached USB hardware, send firmware, install drivers or manipulate power. Actual hardware work needs a selected target and scope already authorized by the task.
- Hash exact fixed-buffer image bytes before port open; preserve changed-file refusal. Filename/hash screening does not prove compatibility.
- Progress counts acknowledged bytes. Preserve Writing/WaitingAck and final EOT ACK. CCC does not identify the model; transfer success does not prove readback, boot or voice functions.
- Separate application presets from firmware framing. Enforce B8 receive-only in the core; connect passively unless TX/trigger is explicit.
- Preserve Chinese/English UI and log reading positions. Display clearing retains progress/counters/raw RX/recordings as applicable. Do not translate original device data, paths or hashes.
- Public documentation covers SU-03T firmware transfer and generic serial diagnostics. Use generic examples and omit application-specific project names and configurations.
- Keep proprietary firmware, vendor tools, credentials and private logs out of commits/screenshots. Ignored vendor analysis is optional, never a build prerequisite or a release asset.
- Follow the task's publication scope, existing identities and focused commit style. Preserve published assets and tags; changed released bytes require a new version.
- New AI project prose belongs under .harness/docs/ with updated navigation/sources. Reuse the user's keep/move choices. These instructions are not a native permission policy or hook.

## Conditional Notion integration

When a task calls Notion, retain the user-provided header contract exactly: Authorization: Bearer $NOTION_API_KEY; Notion-Version: 2022-06-28; Content-Type: application/json. Do not use another Authorization format. Keep the token value in the environment/credential store. This conditional instruction does not make Notion an application dependency.
