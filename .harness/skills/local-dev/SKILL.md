---
name: "local-dev"
description: "Prepare local prerequisites and prove a hardware-free startup path."
---

# Local development workflow

Use for setup/startup/prerequisite diagnosis. Inputs: checkout, host OS and desktop/CLI variant.

1. Read [development](../../docs/development.md), inspect source/CI requirements and existing tools. Resolve optional checkout state through the canonical local reader.
2. Follow the CLI build/version/help sequence and synthetic firmware-info fixture; run the independent CLI probe. Record actual target, version, variant and hash observation.
3. A requested GUI run needs default features and an active desktop. Simulated screenshots follow CONTRIBUTING.md. Hardware flashing is a separate target-bound task.
4. Clean fixtures, PTYs and created processes. Preserve local execution/failure evidence in ignored state; shared guides contain portable facts.

For compiler/library failure follow prerequisites. For wrong executable version rebuild the intended target. Reuse tools/authentication and change system configuration only in the requested setup scope. State which startup path actually ran.
