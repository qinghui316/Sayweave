<!-- ECL-HARNESS:BEGIN -->
# beaver Claude Route

<!-- ECL-HARNESS-PROJECT-ID: beaver-4a20fc43577a -->

Use the local `beaver-4a20fc43577a-harness` Skill for structured project work, Change handling, worktree
coordination, Integration, and Harness evolution. Keep detailed rules in the shared Skill rather
than duplicating them here.

If this is a newly created worktree and the Skill is not discoverable yet, run one available host connector:

```text
PowerShell: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/harness-skill-link.ps1
Node.js:    node scripts/harness-skill-link.mjs
Python:     python3 scripts/harness-skill-link.py (or python on Windows)
```

Then reload the project Harness; single-Lane Small Changes use targeted verification, while Structured and multi-Lane repository work publish scope and run Registry preflight. Before removing this secondary worktree, rerun the same connector with `-Detach` for PowerShell or `--detach` for Node.js/Python.
<!-- ECL-HARNESS:END -->
