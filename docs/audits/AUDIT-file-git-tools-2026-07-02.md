> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# 文件操作 & Git 工具缺陷审计报告

> 审计日期：2026-07-02
> 审计范围：`src-tauri/crates/tools/src/tools/` 下所有文件操作工具和 Git 工具
> 审计人员：AI 助手

---

## 总结

发现 **8 个缺陷**：4 个 P0（高危）、2 个 P1（中危）、2 个 P2（低危）。

| 严重度 | 数量 | 概要 |
|--------|------|------|
| 🔴 P0 | 4 | 框架层验证跳过、Windows 路径 bug、安全旁路 |
| 🟡 P1 | 2 | 权限检查缺失 |
| 🔵 P2 | 2 | 实现质量问题 |

---

## P0 — 高危缺陷

### #1 框架层：`execute()` 跳过 `validate()` 和 `check_permissions()`

**文件**：`src-tauri/crates/tools/src/registry.rs:1063-1101`
**严重度**：🔴 P0

**问题描述**：
`UnifiedToolRegistry::execute()` 方法在调用 `tool.call()` 之前，只做了工具启用检查、频率限制、权限策略（permission_policy）、PreToolUse Hook 这 4 层检查，**但从未调用 `tool.validate()` 和 `tool.check_permissions()`**。

```rust
// 第 1101 行 — 直接 call，没有 validate() 和 check_permissions()
match tool.call(input_val, &ctx).await {
```

`Tool` trait 在 `harness/src/tool.rs:379-473` 提供了默认的 `validate()` 实现（检查 required 字段、类型、枚举、范围），多个工具也覆盖了 `validate()` 和 `check_permissions()` 做额外的安全检查。但 execute() 根本不调用它们——这意味着所有工具的自定义输入验证和权限检查全部依赖于工具自身在 `call()` 中主动调用，否则就是摆设。

**影响**：
- `FileReadTool.validate()` 检查设备文件黑名单和敏感路径——旁路后可能读取 `/etc/shadow` 等敏感文件
- `FileWriteTool.validate()` 检查硬禁路径——旁路后可能写入系统目录
- `FileEditTool.validate()` 检查文件大小和遍历路径——旁路后可能编辑系统文件
- `GlobTool.validate()` 只检查 path 是绝对路径（影响较小）

**修复建议**：
在 `execute()` 中，`tool.call()` 之前加入 `tool.validate(&input_val, &ctx)?;`：

```rust
// 在第 1100 行附近加入
tool.validate(&input_val, &ctx).map_err(|e| {
    tracing::warn!(tool_name, "validate 失败: {e}");
    e
})?;
```

---

### #2 `validate_and_resolve_path()` 拒绝 Windows 绝对路径

**文件**：`src-tauri/crates/tools/src/tools/file_system.rs:31-33`
**严重度**：🔴 P0

**问题描述**：
`validate_and_resolve_path()` 函数判断绝对路径的逻辑是检查路径是否以 `/` 开头：

```rust
if p.is_absolute() {
    if !p.starts_with("/") {
        return Err(format!("不支持的操作系统路径: {}", path));
    }
    Ok(p.to_path_buf())
}
```

在 Windows 上，绝对路径如 `D:\OneManager\AxAgent\file.txt` 对 `Path::is_absolute()` 返回 **true**，但 `starts_with("/")` 返回 **false**。结果所有 Windows 盘符开头的绝对路径都会被拒绝。

**影响**：
- `ListDirectoryTool`、`DeleteFileTool`、`CreateDirectoryTool`、`FileExistsTool`、`GetFileInfoTool`、`MoveFileTool` — 共 **6 个工具**在 Windows 上无法使用绝对路径
- 只能通过相对路径（相对于 working_dir）访问文件

**修复建议**：
不要手动检查路径格式，直接使用解析后的 `PathBuf`：

```rust
if p.is_absolute() {
    // Windows 下 C:\foo 是绝对路径但不以 / 开头
    // Rust 的 Path::is_absolute() 已正确跨平台
    Ok(p.to_path_buf())
} else {
    let cwd = Path::new(&ctx.working_dir);
    Ok(cwd.join(p))
}
```

---

### #3 `FileEdit.check_permissions()` 缺少工作区边界检查

**文件**：`src-tauri/crates/tools/src/tools/file_edit.rs:111-128`
**严重度**：🔴 P0

**问题描述**：
`FileEdit.check_permissions()` 只检查了几个系统路径前缀（`/etc`、`/boot`、`/sys` 等），**完全没有工作区边界检查**：

```rust
fn check_permissions(&self, input: &Value, _ctx: &ToolContext) -> PermissionResult {
    let path = input["file_path"].as_str().unwrap_or("");
    let dangerous_prefixes = [
        "/etc", "/boot", "/sys", "/proc", "/dev",
        "C:\\Windows", "C:\\Program Files",
    ];
    for prefix in &dangerous_prefixes {
        if path.starts_with(prefix) {
            return PermissionResult::Ask(...);
        }
    }
    PermissionResult::Allow  // ← 没有 workspace 检查就 Allow
}
```

对比 `FileReadTool.check_permissions()` 和 `FileWriteTool.check_permissions()`，两者都会调用 `is_within_workspace()` 检查目标文件是否在工作区内。

**影响**：
- 可以编辑工作区外的任意文件（只要不在那几个硬编码系统前缀下），如 `/home/user/.ssh/authorized_keys`、`C:\Users\polit\.ssh\id_rsa`、`C:\Users\polit\Documents\敏感文件.txt`

**修复建议**：
参照 `FileWriteTool.check_permissions()`，加入工作区边界检查：

```rust
fn check_permissions(&self, input: &Value, ctx: &ToolContext) -> PermissionResult {
    let path = input["file_path"].as_str().unwrap_or("");
    let working_dir = std::path::Path::new(&ctx.working_dir);
    // ... 前缀检查 ...
    if !is_within_workspace(Path::new(path), working_dir) {
        return PermissionResult::Ask(format!(
            "目标 {} 不在工作区 {} 内，确认编辑？",
            path, working_dir.display()
        ));
    }
    PermissionResult::Allow
}
```

---

### #4 `file_system.rs` 所有工具无 `check_permissions()`

**文件**：`src-tauri/crates/tools/src/tools/file_system.rs`
**严重度**：🔴 P0

**问题描述**：
`ListDirectoryTool`、`DeleteFileTool`、`CreateDirectoryTool`、`FileExistsTool`、`GetFileInfoTool`、`MoveFileTool` — **6 个工具全部没有实现 `check_permissions()`**，使用 trait 默认的 `PermissionResult::Allow`。

特别是 `DeleteFileTool` 和 `MoveFileTool` 是破坏性操作（`is_destructive()` 返回 true），但没有任何权限检查。

`MoveFileTool` 可以：
- 将文件从工作区移到任意目录（跨越工作区边界）
- 覆盖已有文件而不提示

`DeleteFileTool` 可以：
- 直接删除工作区外文件（如果 `validate_and_resolve_path` 允许的话——但#2 的 bug 实际上阻止了绝对路径访问，所以这个漏洞在 Windows 上被#2 掩盖了）

**修复建议**：
为每个工具实现 `check_permissions()`，至少：
- 检查目标路径是否在工作区内（`is_within_workspace`）
- 对删除/移动等操作要求用户确认

---

## P1 — 中危缺陷

### #5 `git.rs` 所有工具无 `validate()` 和 `check_permissions()`

**文件**：`src-tauri/crates/tools/src/tools/git.rs`
**对象**：`GitStatusTool`、`GitDiffTool`、`GitCommitTool`、`GitLogTool`、`GitBranchTool`、`GitReviewTool`
**严重度**：🟡 P1

**问题描述**：
全部 6 个 Git 工具都没有覆盖 `validate()` 和 `check_permissions()` 方法。
- `GitCommitTool` 和 `GitBranchTool` 是破坏性操作（`is_destructive()=true`），但没有权限检查
- `GitCommitTool` 在 `call()` 中直接调用 `GitTools::stage_all()` 和 `GitTools::commit()` 操作 repo_path 指定的任何仓库
- `GitBranchTool` 的 `create` 和 `switch` 操作都是不可逆的

**影响**：
- 可以提交不属于工作区仓库的代码
- 可以创建、切换任意 Git 仓库的分支

**修复建议**：
为破坏性操作（GitCommit、GitBranch）实现 `check_permissions()`，检查 `repo_path` 是否在工作区内。

---

### #6 `workspace.rs` 路径穿越防护不充分

**文件**：`src-tauri/crates/tools/src/tools/workspace.rs:52-58`
**严重度**：🟡 P1

**问题描述**：
`WorkspaceReadTool` 和 `WorkspaceWriteTool` 使用字符串替换方式做路径净化：

```rust
let safe_name = filename
    .replace("..", "")
    .replace("\\", "")
    .replace("/", "");
```

这种净化可以被绕过。例如：
- 输入 `....//foo` → 先替换 `..` 为 `""` → `..` + `/foo` → 最终路径穿越
- 或者 `....\\foo` 在 Windows 上可能穿越

**影响**：
可能读取/写入工作区外的文件。

**修复建议**：
使用 `std::path::Path::canonicalize()` 解析并验证结果路径仍以 workspace_path 为前缀：

```rust
let file_path = Path::new(workspace_path).join(&filename);
let canonical = file_path.canonicalize()
    .map_err(|_| "非法路径")?;
if !canonical.starts_with(workspace_path) {
    return Err("路径穿越检测");
}
```

---

## P2 — 低危缺陷

### #7 `grep.rs fallback_search` 忽略 context 参数

**文件**：`src-tauri/crates/tools/src/tools/grep.rs:177`
**严重度**：🔵 P2

**问题描述**：
`fallback_search()` 函数接收 `_context: u32` 参数，但完全未使用（下划线开头表示故意忽略）。当系统没有 `rg` 或 `grep` 命令时，回退搜索不会显示上下文行。

```rust
fn fallback_search(
    pattern: &str,
    search_path: &str,
    case_insensitive: bool,
    _context: u32,          // ← 未使用
    head_limit: usize,
) -> Result<ToolResult, ToolError> {
```

**修复建议**：
在回退搜索中实现上下文行支持，参考 `rg` 的 `-C` 行为。

---

### #8 `worktree.rs` `default_branch()` 用 `git branch -a` 查找默认分支低效

**文件**：`src-tauri/crates/tools/src/tools/worktree.rs:33-53`
**严重度**：🔵 P2

**问题描述**：
`default_branch()` 执行 `git branch -a` 列出所有分支（可能包含数百个远程跟踪分支），然后检查是否包含 `main` 或 `master`。

`git branch -a` 在有多个 remote 的仓库中会输出大量分支，在大型仓库上可能很慢（尤其是需要 fetch 时）。

**修复建议**：
使用更精确的命令：`git symbolic-ref refs/remotes/origin/HEAD` 或 `git rev-parse --abbrev-ref HEAD` 并回退到 `main`。

---

## 缺陷总表

| # | 严重度 | 文件 | 工具 | 问题概要 |
|---|--------|------|------|----------|
| 1 | 🔴 P0 | `registry.rs:1101` | **全部工具** | `execute()` 跳过 `validate()` 和 `check_permissions()` |
| 2 | 🔴 P0 | `file_system.rs:31-33` | 6 个文件系统工具 | Windows 绝对路径被拒绝 |
| 3 | 🔴 P0 | `file_edit.rs:111-128` | `FileEditTool` | 缺少工作区边界检查 |
| 4 | 🔴 P0 | `file_system.rs` (全文件) | 6 个文件系统工具 | 无 `check_permissions()` |
| 5 | 🟡 P1 | `git.rs` (全文件) | 6 个 Git 工具 | 无 `validate()` 和 `check_permissions()` |
| 6 | 🟡 P1 | `workspace.rs:52-58` | `WorkspaceRead/Write` | 路径穿越防护不充分 |
| 7 | 🔵 P2 | `grep.rs:177` | `Grep fallback_search` | context 参数未使用 |
| 8 | 🔵 P2 | `worktree.rs:33-53` | `EnterWorktreeTool` | 默认分支查找低效 |

---

## 修复优先级建议

1. **立即修复** (#1, #2)：框架层验证缺失 + Windows 路径 bug 导致工具链基本不可用
2. **尽快修复** (#3, #4)：严重的安全漏洞，允许越界读写文件
3. **规划修复** (#5, #6)：安全加固，防止误操作
4. **低优先** (#7, #8)：功能完整性和性能改进
