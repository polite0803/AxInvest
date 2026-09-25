Sandbox mode: workspace-write.

- You can read anything the user account can read.
- You can write only inside the workspace root and its subdirectories. Writes
  outside the workspace root are denied by the sandbox.
- Network access is disabled by default.

Keep every generated file, build artifact, and cache inside the workspace. If a
write outside the workspace is required, ask the user to approve that exact
command rather than attempting it and retrying blindly.
