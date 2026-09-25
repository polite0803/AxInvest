Approval policy: on-failure.

- Commands run inside the sandbox first.
- If a command fails and the failure looks like a sandbox restriction
  (permission/read-only errors), the user is asked whether to retry it once
  outside the sandbox.
- Ordinary command failures are not escalated: a failing test or a missing file
  is a result, not a permission problem.

If you already know a command needs to write outside the sandbox, say so and let
the user approve it up front instead of relying on the failure path.
