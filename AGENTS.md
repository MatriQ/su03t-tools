<!-- workspace-harness-kit:configuration:start -->
# SU-03T Tools engineering harness

Use the selected workspace and its existing instructions. Resolve member checkout paths through the established local mapping.

AGENTS.md is the canonical entry point; other host entry points link to it. Read the relevant procedures and rules below. These are entry-point-routed instructions, not registered native commands, automatically discovered Skills or agents.

## Documentation location

All new project documentation written by AI, including plans, designs, analyses, validation reports and handoff notes, must live under `.harness/docs/`. Organize it by topic and update that directory's index. Name repository-local files and workflows by purpose, without repeating the repository or product name; add a component qualifier only when needed to distinguish workflows. Do not create these documents or duplicate copies at the repository root or elsewhere. AGENTS.md and host entry points contain rules and navigation; tool-required configuration, Skills and runtime files retain their prescribed locations.

When existing documents are found elsewhere, list their current paths, proposed destinations and affected references, then ask the user which to move and which to keep. Reuse an explicit prior choice. Do not move or delete unselected originals; link to documents kept in place. For a selected move, preserve content, check collisions, repair relative links and callers, then verify sources and documentation completeness.

For machine checkout paths, environment, tools or personal access context, first read the [optional local entry](.harness/docs/workspace.local.md). Files matching *.local.md are ignored and maintained by local configuration, outside shared generation. Use `node .harness/runtime/local-context.mjs --workspace .` for the shared parser and identity checks. On a new machine, supply bindings and reverify readiness; do not inherit another machine's observations. Keep credential values in environment variables or existing credential stores.

- [.harness/docs/index.md](.harness/docs/index.md)
- [.harness/docs/architecture.md](.harness/docs/architecture.md)
- [.harness/docs/development.md](.harness/docs/development.md)
- [.harness/docs/testing.md](.harness/docs/testing.md)
- [.harness/docs/release.md](.harness/docs/release.md)
- [.harness/docs/troubleshooting.md](.harness/docs/troubleshooting.md)
- [.harness/docs/maintenance.md](.harness/docs/maintenance.md)
- Implement a change while preserving firmware, serial and UI contracts. — [engineering](.harness/skills/engineering/SKILL.md)
- Prepare local prerequisites and prove a hardware-free startup path. — [local-dev](.harness/skills/local-dev/SKILL.md)
- Select meaningful unit, PTY, installer and package checks for a change. — [verify](.harness/skills/verify/SKILL.md)
- Prepare, publish or recover a versioned release and verify delivered packages. — [release](.harness/skills/release/SKILL.md)
- Required guidance: [engineering](.harness/rules/engineering.md).

Configuration and local verification: [harness specification](.harness/harness.json). Run `node .harness/runtime/check.mjs --workspace .` after changes. A passing check does not establish agent activation or deployment readiness.
<!-- workspace-harness-kit:configuration:end -->
