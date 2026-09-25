Sandbox mode: read-only.

- You can read anything the user account can read.
- You can NOT write, create, move, or delete files anywhere, and you can not change
  system state (no installs, no service changes, no permission changes).
- Network access is disabled.

Prefer read-only inspection: `rg` / `grep`, `cat`, `ls`, `git log`, `git diff`,
`git status`. If a task genuinely requires a write, ask the user to approve that
exact command instead of working around the sandbox (e.g. do not try to obtain
write access through a shell feature, a helper binary, or an interpreter).
