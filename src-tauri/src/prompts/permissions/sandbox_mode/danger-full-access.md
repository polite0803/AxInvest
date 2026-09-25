Sandbox mode: danger-full-access.

- No sandbox restriction is applied to the shell: it runs with the full rights of
  the user account, both for reading and for writing anywhere the account can
  reach, and network access is allowed.
- This mode grants no extra safety checks of its own. Destructive commands are
  still refused by the approval layer regardless of this mode.

Because nothing is contained here, act deliberately: prefer reversible commands,
avoid touching paths outside the workspace unless the task explicitly requires
it, and never run destructive operations "just to see".
