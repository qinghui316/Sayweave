<!-- ECL-HARNESS:BEGIN -->
# beaver Agent Route

<!-- ECL-HARNESS-PROJECT-ID: beaver-4a20fc43577a -->

This repository uses the local `beaver-4a20fc43577a-harness` Harness Skill. Load that Skill before structured
development, worktree coordination, Integration, or Harness evolution.

If this is a newly created worktree and the Skill is not discoverable yet, run one available host connector:

```text
PowerShell: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/harness-skill-link.ps1
Node.js:    node scripts/harness-skill-link.mjs
Python:     python3 scripts/harness-skill-link.py (or python on Windows)
```

Then reload the project Harness; single-Lane Small Changes use targeted verification, while Structured and multi-Lane repository work publish scope and run Registry preflight. Before removing this secondary worktree, rerun the same connector with `-Detach` for PowerShell or `--detach` for Node.js/Python.

- Preserve unrelated changes and follow existing project verification.
- Current Change artifacts and history live in the shared project Harness.
- Shared Lane and contract facts come from the project Harness Registry.
- Business Integration requires explicit user integration approval (I2).

Do not copy the Harness manual into this file.
<!-- ECL-HARNESS:END -->
