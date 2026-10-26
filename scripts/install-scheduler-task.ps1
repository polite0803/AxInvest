#!/usr/bin/env pwsh
# SPDX-License-Identifier: AGPL-3.0-only
<#
.SYNOPSIS
    把 AxInvest 的**无头服务端**注册为 Windows 计划任务（C 方案落地）。

.DESCRIPTION
    调度器原先只能活在 Tauri 应用进程里 ⇒ 桌面应用一关，每日定时任务
    （回放 / 反思 / 荐股 / 决策回测）就全部停止；实测长任务因此被腰斩
    （`mover_recall_analysis_batches` 连续 3 次 `interrupted`，消息统一是
    「应用重启：运行中进程退出，as-of 封闭世界无法跨进程续跑」）。

    本脚本把 `axagent-server`（`--features server` 构建）注册为**登录时启动**的
    计划任务，使调度器与桌面应用**解耦**。

    ⚠ 跨进程互斥已就位：调度器启动时抢 DB 里的 leader 租约
       （`cron_leader_lease`）⇒ 桌面应用与本服务同时在跑时**只有一个派发**。

.PARAMETER InstallPath
    `axagent-server.exe` 的绝对路径。默认取仓库的 release 构建产物。

.PARAMETER TaskName
    计划任务名。默认 `AxInvest Scheduler`。

.PARAMETER Remove
    传入则**删除**已注册的计划任务（回滚）。

.EXAMPLE
    pwsh -File scripts/install-scheduler-task.ps1
    pwsh -File scripts/install-scheduler-task.ps1 -Remove
#>
[CmdletBinding()]
param(
    [string]$InstallPath = "",
    [string]$TaskName = "AxInvest Scheduler",
    [switch]$Remove
)

$ErrorActionPreference = "Stop"

if ($Remove) {
    $existing = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    if ($existing) {
        Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
        Write-Host "[scheduler-task] removed: $TaskName" -ForegroundColor Yellow
    } else {
        Write-Host "[scheduler-task] not registered: $TaskName" -ForegroundColor Yellow
    }
    return
}

# ── 1. 定位可执行文件 ────────────────────────────────────────────────
if ([string]::IsNullOrWhiteSpace($InstallPath)) {
    $candidates = @(
        (Join-Path $PSScriptRoot "..\src-tauri\target\release\axagent-server.exe"),
        (Join-Path $PSScriptRoot "..\src-tauri\target\debug\axagent-server.exe")
    )
    $found = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $found) {
        Write-Error @"
[scheduler-task] axagent-server.exe not found. Build it first:
    cd src-tauri
    cargo build --release --features server --bin axagent-server
then re-run with -InstallPath <abs path>.
"@
    }
    $InstallPath = (Resolve-Path $found).Path
}
else {
    if (-not (Test-Path $InstallPath)) {
        Write-Error "[scheduler-task] not found: $InstallPath"
    }
    $InstallPath = (Resolve-Path $InstallPath).Path
}

Write-Host "[scheduler-task] exe: $InstallPath"

# ── 2. 注册计划任务（登录时启动，失败即重试）────────────────────────────
# ⚠ 用 `schtasks` 而非 `Register-ScheduledTask`：后者在部分 Windows 家庭版上
#   缺少「当用户登录时」+「交互式运行」组合所需的权限，而 `schtasks /IT`
#   只需普通用户权限 ⇒ 少一道提权 ⇒ 更容易落地。
$action = "schtasks.exe /Create /TN `"$TaskName`" /TR `"$InstallPath`" /SC ONLOGON /RL LIMITED /F /IT"

Write-Host "[scheduler-task] registering: $action"
Invoke-Expression $action | Out-Null

$task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
if (-not $task) {
    Write-Error "[scheduler-task] registration failed (task not found after create)"
}

Write-Host "[scheduler-task] registered: $TaskName" -ForegroundColor Green
Write-Host "[scheduler-task] state: $($task.State)" -ForegroundColor Green
Write-Host @"
[scheduler-task] notes:
  * The scheduler acquires a cross-process leader lease (table cron_leader_lease);
    the desktop app and this service may run at the same time - only one dispatches.
  * The service writes its log to stdout; run it in a console or redirect via schtasks.
"@
