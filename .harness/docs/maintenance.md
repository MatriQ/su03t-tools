# Harness maintenance

The authored input is [harness.json](../harness.json); [workspace.json](../workspace.json) records choices and registry. Language is English, hosts are Codex/Claude Code, layout is centralized. AGENTS.md is regular; CLAUDE.md is a relative symlink. Supporting procedures are entry-point-routed.

## Refresh and review

1. Read required rules, affected guides and implementation/workflow sources. CONTRIBUTING.md retains standing authority.
2. Edit the spec's prose and source/provenance mappings; use the installed harness-init Skill to preview/apply here. Do not suppress drift by changing the receipt/runtime.
3. Review final bytes, register implementation/scripts with --source, and use the workspace helper's --review-document on real symlink targets. Inventory refresh alone is not prose review.
4. Run portable integrity and strict workspace/generation checks. A fresh clone can run the checker without the original Skill or this machine's ignored context.

~~~bash
node .harness/runtime/check.mjs --workspace .
~~~

New AI prose belongs in .harness/docs/. The user kept README.md, README.en.md and CONTRIBUTING.md at the root and moved protocol.md/validation.md here. Keep callers and Linux archive paths aligned; preserve historical validation claims.

## Local context

workspace.local.md is the sole optional machine binding/readiness entry. Parse it with:

~~~bash
node .harness/runtime/local-context.mjs --workspace .
~~~

A fresh clone has no established binding. Use the installed workspace helper's plan/apply with member tools in init mode, this checkout's actual path and saved choices. Rebind after moving directories; do not copy personal paths/readiness from another machine. Keep raw reports under ignored local-evidence/ and link them locally. Never publish credentials or private device logs.

## Activation and evidence

[Codex reads AGENTS.md at startup](https://learn.chatgpt.com/docs/agent-configuration/agents-md); restart its session after an entry-point change. Claude Code loads CLAUDE.md, resolving the same content here. This centralized layout generates no native settings/hooks/custom-agent registration.

Keep artifact integrity, host loading/trust, application execution and hosted CI separate. Node version is not host activation; a local pass is not hosted CI; PTY success is not physical flashing. Use the installed Skill's doctor with target-bound [probes](../resources/probes.py) and retain reports locally. Replay representative index questions from the final entry point; missing key mappings are documentation gaps.
