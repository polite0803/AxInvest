// SPDX-License-Identifier: AGPL-3.0-only

//! Windows 受限令牌沙箱
//!
//! 对标 codex 的内核级沙箱语义，Windows 侧实现 `ReadOnly` / `WorkspaceWrite` 两档
//! （`DangerFullAccess` 不进沙箱路径，由调用方走直通分支）：
//!
//! ## 原理
//!
//! 1. **受限令牌（capability SID 版）**：`CreateRestrictedToken` 标志
//!    `DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED`，restricting SID 列表为
//!    「固定 capability SID → Logon → Everyone」（capability SID 的形态有硬约束，
//!    见 [`capability_sid`]：**`S-1-15-*` 段进不了 restricting 列表**）。
//!    > 选型说明：早期实现走 SAFER（`SaferCreateLevel(SAFER_LEVELID_NORMALUSER)`），
//!    > 其 restricting check 是「标准用户」语义——保留用户对自己 Profile 的写权限，
//!    > 且要求 cwd 必须世界可读。两条边界都无法在 SAFER 框架内消除（用户 SID 的
//!    > deny-only 层需要 `SeAssignPrimaryTokenPrivilege`，标准用户不持有）。
//!    > 改用 capability SID 后 restricting 列表由本模块自己掌握，两条边界同时消失。
//! 2. **写限制的机制（必须理解，否则会做无效加固）**：`WRITE_RESTRICTED` 使受限检查
//!    **只作用于写访问**：
//!    - 读：只走第一遍（令牌常规 SID）⇒ 与未沙箱进程等价，路径**不需**世界可读；
//!    - 写：需第一遍**与**第二遍（restricting SID）同时通过，第二遍只认 allow ACE。
//!
//!    ⇒ 给工作区加 capability SID 的 allow ACE 即可放行工作区写；工作区外没有任何
//!    capability ACE ⇒ 写一律被拒（**含用户自己的 Profile**）。
//!    ⇒ 派生令牌的**默认 DACL** 也必须显式改写（[`set_default_dacl`]）：它决定子进程
//!    自建对象的 ACL，继承自基础令牌的默认 DACL 不含任何 restricting SID ⇒ 子进程
//!    初始化期间的写会在第二遍被拒，直接 `0xC0000142` 退出。
//!    ⇒ 两档（`ReadOnly` / `WorkspaceWrite`）必须用**不同**的 capability SID
//!    （[`CapKind`]），否则工作区 ACE 会遗留给之后的 ReadOnly 会话。
//!    ⇒ 反向结论：在 capability SID 上挂 deny-**read** ACE **无效**——读不走第二遍。
//!    读限制只能靠「不授予读权限」的 allow-only 模型实现（codex 亦如此）。
//! 3. **restricting 列表顺序有语义**：严格按「capability → extra-restricting → Logon →
//!    Everyone」排列，照 codex 顺序抄，不要重排（Everyone 在末位是「世界可读」兜底，
//!    Logon 在 Everyone 之前用于桌面/窗口站对象）。
//! 4. `CreateProcessAsUserW` 用受限令牌启动 `cmd /d /s /c <command>`，
//!    环境块白名单重建（按名称大小写不敏感字母序排序 + 去重——Win32 硬性
//!    要求），命令行缓冲必须显式 null 终止（漏掉会导致分配器复用的堆残留
//!    拼进子进程命令行，产生随机「找不到文件」与输出乱码）。
//! 5. **私有 Window Station + Desktop**：子进程在 `lpDesktop` 指向的私有
//!    桌面运行（DACL 授 Everyone/Users GENERIC_ALL）。默认桌面
//!    `WinSta0\Default` 的 DACL 不含受限令牌的 check SID，conhost 初始化
//!    半途而废，控制台输出出现堆垃圾——私有桌面是必需项而非加固项。
//! 6. 匿名管道收集 stdout/stderr；JobObject（KILL_ON_JOB_CLOSE）兜底进程树清理。
//! 7. **网络封锁**：`network_access == false` 时按 capability SID 在 WFP 的
//!    `ALE_AUTH_CONNECT_V4/V6`（出站连接，含无连接 UDP 发送）与
//!    `ALE_RESOURCE_ASSIGNMENT_V4/V6`（套接字绑定）四层挂 block 过滤。
//!    过滤只匹配沙箱令牌（同一 capability SID），宿主与其它进程不受影响。
//!    实际强度由 [`NetworkBlock`] 如实上报（见「当前边界」第 3 条）。
//!
//! ## 当前边界（如实记录，不静默）
//!
//! 1. **世界可写目录在 `ReadOnly` 下仍可写**：Everyone 必须在 restricting 列表里
//!    才能保住「世界可读」，代价是「世界可写」也通过第二遍。codex 用
//!    world-writable 审计 + capability deny ACE 消除它，本轮不做
//!    （见 `PLAN-codex-parity-adoption.md` §4.1）。
//! 2. **reparse point 防绕过未做**：codex 另有一层 `OBJ_DONT_REPARSE` 打开目录，
//!    防「用 junction 把受限路径重定向到任意目标」，本项目无对应机制。
//! 3. **网络封锁在非提权进程里是「挂不上」而非「弱一点」** —— 本机实测
//!    （2026-09-25，非提权标准用户 `HUSTNIU\polit`）：
//!    `FwpmEngineOpen0` **成功**，而 `FwpmSubLayerAdd0`、退到内建 universal 子层后的
//!    `FwpmFilterAdd0` 均返回 `ERROR_ACCESS_DENIED`（0x00000005）⇒ WFP 对象安装需要提权。
//!    codex 的三层（WFP + Defender + 代理白名单）同样建在「提权创建的沙箱账户 / 规则」
//!    之上（审计 §8.4），本项目不建账户、不装常驻服务（计划 §8 排除），故
//!    **非提权运行时网络不会被阻断**。处理方式：`ensure_network_block()` 把这一档
//!    显式表达为 [`NetworkBlock::Unavailable`] 并记 `warn`（不假装已断网），
//!    但**不**据此拒绝启动 —— 拒绝会让非提权环境下的沙箱整体不可用，而文件系统
//!    限制（本模块的主要交付）是可交付的。以管理员身份运行时封锁自动生效。
//!    其余失败路径（非权限类）仍 fail-closed：返 `Err` ⇒ 调用方拒绝启动。
//! 4. **没有「按工作区隔离 capability SID」**：codex 为每个 cwd / 额外可写根各生成一个
//!    随机 capability SID 并落盘（`cap.rs`），从而「A 工作区的沙箱令牌写不了 B 工作区」。
//!    本模块全局共用一个固定 SID ⇒ 同一用户的沙箱可写**任意**已授权工作区
//!    （跨用户不行，理由见 [`capability_sid`] 的「安全性」）。本轮不做。
//!
//! 非 Windows 平台本模块不编译；Bash 工具侧由 cfg 分支处理。

use std::path::Path;

use axagent_harness::SandboxPolicy;

/// 沙箱化子进程：RAII 兜底——Drop 时 TerminateProcess + Job 句柄关闭
/// （KILL_ON_JOB_CLOSE 终止整个进程树），保证超时/取消不残留进程。
pub struct SandboxedChild {
    process: std::sync::Arc<RawProcess>,
    _job: std::sync::Arc<crate::job_object::JobHandle>,
    /// 管道读取线程的 mpsc 接收端（spawn 时已启动读取线程）
    stdout_rx: Option<std::sync::mpsc::Receiver<Vec<u8>>>,
    stderr_rx: Option<std::sync::mpsc::Receiver<Vec<u8>>>,
}

pub struct SandboxedOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// 进程句柄 RAII 包装（Arc 共享给等待线程）
struct RawProcess(windows_sys::Win32::Foundation::HANDLE);
// SAFETY: HANDLE 是裸句柄；只用于 WaitForSingleObject/TerminateProcess/
// GetExitCodeProcess 等线程安全的 Win32 调用。
unsafe impl Send for RawProcess {}
unsafe impl Sync for RawProcess {}

impl Drop for RawProcess {
    fn drop(&mut self) {
        // SAFETY: self.0 为有效进程句柄（CreateProcessAsUserW 返回），关闭后不再使用。
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

impl Drop for SandboxedChild {
    fn drop(&mut self) {
        self.terminate();
        // _job（Arc<JobHandle>）随 Drop 释放 → 关闭 Job 句柄 → 进程树被终止
    }
}

impl SandboxedChild {
    /// 终止进程树（幂等：已退出的进程 TerminateProcess 失败无副作用）。
    pub fn terminate(&self) {
        // SAFETY: self.process.0 为有效进程句柄。
        unsafe { windows_sys::Win32::System::Threading::TerminateProcess(self.process.0, 1) };
    }

    /// 等待进程退出并收集全部输出（读端 EOF 后 WaitForSingleObject）。
    pub async fn wait_with_output(mut self) -> Result<SandboxedOutput, String> {
        let stdout_rx = self.stdout_rx.take().unwrap_or_else(|| std::sync::mpsc::channel().1);
        let stderr_rx = self.stderr_rx.take().unwrap_or_else(|| std::sync::mpsc::channel().1);
        let process = self.process.clone();
        let (tx, rx) = tokio::sync::oneshot::channel::<SandboxedOutput>();

        // 阻塞等待放在 spawn_blocking，不阻塞 tokio worker
        tokio::task::spawn_blocking(move || {
            // 读端 EOF（子进程/进程树关闭写端）后 recv 返回
            let stdout = stdout_rx.recv().unwrap_or_default();
            let stderr = stderr_rx.recv().unwrap_or_default();
            // SAFETY: process 句柄有效（Arc 保证存活到本闭包结束）。
            unsafe {
                windows_sys::Win32::System::Threading::WaitForSingleObject(
                    process.0,
                    windows_sys::Win32::System::Threading::INFINITE,
                );
            }
            let mut code: u32 = 0;
            // SAFETY: 同上。
            unsafe {
                windows_sys::Win32::System::Threading::GetExitCodeProcess(process.0, &mut code);
            }
            let _ = tx.send(SandboxedOutput { exit_code: code as i32, stdout, stderr });
        });

        rx.await.map_err(|_| "沙箱进程等待任务被取消".to_string())
    }
}

/// 以沙箱策略启动 `cmd /d /s /c <command>`。
///
/// 返回的 [`SandboxedChild`] 具备 RAII 兜底：超时/取消时 Drop 即终止进程树。
pub fn spawn_sandboxed(
    policy: &SandboxPolicy,
    command: &str,
    cwd: &Path,
) -> Result<SandboxedChild, String> {
    let cap_kind = match policy.mode {
        axagent_harness::SandboxMode::ReadOnly => CapKind::ReadOnly,
        axagent_harness::SandboxMode::WorkspaceWrite => {
            grant_workspace_write(&policy.workspace_cwd)?;
            CapKind::WorkspaceWrite
        },
        axagent_harness::SandboxMode::DangerFullAccess => {
            return Err("DangerFullAccess 不应进入沙箱路径（调用方负责走直通分支）".to_string());
        },
    };
    if !policy.network_access {
        // 只把「安装失败」当 fail-closed（返 Err ⇒ 调用方拒绝启动）；非提权导致的
        // 结构性不可用如实上报 + 记 warn，**不**据此拒绝启动——理由是拒绝会让
        // 非提权环境（桌面应用的常态）下的沙箱整体不可用，而文件系统限制这两档
        // 是可交付的。实测见模块文档「当前边界」。
        match ensure_network_block()? {
            NetworkBlock::Enforced => {},
            NetworkBlock::BestEffort => {
                tracing::warn!(
                    "沙箱网络封锁挂在内建 universal 子层（非提权进程建不了高权重子层）：\
                     过滤器已安装，但可能被系统防火墙更高权重的 allow 压过"
                );
            },
            NetworkBlock::Unavailable { reason } => {
                tracing::warn!(
                    "沙箱策略要求断网（network_access=false）但网络**未被**阻断：{reason}。\
                     以管理员身份运行本进程可自动启用封锁"
                );
            },
        }
    }
    spawn_restricted(command, cwd, cap_kind)
}

/// 环境变量白名单：重建环境块，不继承父进程完整 env（防凭据泄露）。
const ENV_WHITELIST: &[&str] = &[
    "SystemRoot",
    "ComSpec",
    "PATHEXT",
    "PATH",
    "OS",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "PROCESSOR_IDENTIFIER",
    "PROCESSOR_LEVEL",
    "PROCESSOR_REVISION",
    "COMPUTERNAME",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "CommonProgramFiles",
    "CommonProgramW6432",
    "PUBLIC",
    "windir",
];

/// Reader 端累计上限（字节）：超出后停止读取，防止恶意命令灌爆内存。
/// 命令会因管道写满而阻塞，最终由超时触发 terminate。
const PIPE_READ_CAP_BYTES: u64 = 2 * 1024 * 1024;

fn spawn_restricted(
    command: &str,
    cwd: &Path,
    cap_kind: CapKind,
) -> Result<SandboxedChild, String> {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE_FLAG_INHERIT, SetHandleInformation};
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Threading::{
        CreateProcessAsUserW, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOW,
    };

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_UNICODE_ENVIRONMENT: u32 = 0x0000_0400;
    const PIPE_SIZE: u32 = 0;

    // ── 1-2. 受限令牌（capability SID，见模块文档「原理」1-3） ──
    let spawn_token = sandbox_token(cap_kind)?;

    // ── 3. 匿名管道（stdout / stderr），句柄全部 RAII ──
    let mut out_read: HANDLE = std::ptr::null_mut();
    let mut out_write: HANDLE = std::ptr::null_mut();
    let mut err_read: HANDLE = std::ptr::null_mut();
    let mut err_write: HANDLE = std::ptr::null_mut();
    // SAFETY: 输出指针均有效。
    let ok = unsafe { CreatePipe(&mut out_read, &mut out_write, std::ptr::null(), PIPE_SIZE) };
    if ok == 0 {
        return Err("CreatePipe(stdout) 失败".to_string());
    }
    // SAFETY: 同上。
    let ok = unsafe { CreatePipe(&mut err_read, &mut err_write, std::ptr::null(), PIPE_SIZE) };
    if ok == 0 {
        return Err("CreatePipe(stderr) 失败".to_string());
    }
    let out_read = HandleGuard(out_read);
    let err_read = HandleGuard(err_read);
    let out_write = HandleGuard(out_write);
    let err_write = HandleGuard(err_write);

    // 子进程写端需可继承；父进程读端不可继承（默认即不可继承）
    // SAFETY: 句柄有效。
    let ok =
        unsafe { SetHandleInformation(out_write.raw(), HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
    if ok == 0 {
        return Err("SetHandleInformation(stdout) 失败".to_string());
    }
    // SAFETY: 句柄有效。
    let ok =
        unsafe { SetHandleInformation(err_write.raw(), HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
    if ok == 0 {
        return Err("SetHandleInformation(stderr) 失败".to_string());
    }

    // ── 4. 环境块（白名单重建，排序 + 去重） ──
    let env_block = build_env_block();

    // ── 5. 命令行：cmd /d /s /c "<command>" ──
    let cmd_path = resolve_cmd_path();
    let cmdline_str = format!("\"{cmd_path}\" /d /s /c \"{command}\"");
    // 注意：必须以 null 终止！否则 Vec 溢出容量上的堆残留（分配器复用的
    // 旧字符串，常见为 env/PATH 碎片）会拼进子进程命令行——cmd /s 模式把
    // 引号外尾随文本并入命令，造成随机的「找不到文件/路径」与输出乱码
    // （曾导致沙箱测试非确定性失败）。
    let mut cmdline: Vec<u16> = cmdline_str.encode_utf16().chain(std::iter::once(0)).collect();
    let cwd_str = cwd.to_string_lossy().to_string();
    let mut cwd_wide: Vec<u16> = cwd_str.encode_utf16().collect();
    cwd_wide.push(0);
    let appname: Vec<u16> = cmd_path.encode_utf16().chain(std::iter::once(0)).collect();

    let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    si.dwFlags = STARTF_USESTDHANDLES;
    si.hStdInput = std::ptr::null_mut();
    si.hStdOutput = out_write.raw();
    si.hStdError = err_write.raw();

    // ── 6. 私有 Window Station Desktop ──
    let station = sandbox_station()?;
    let mut desktop_wide: Vec<u16> = station.desktop_path.encode_utf16().chain([0]).collect();
    // lpDesktop 字段类型为 PWSTR（*mut u16），STARTUPINFOW 不会就地修改该缓冲区，
    // 但类型要求可变指针。默认桌面 DACL 不含受限令牌的 check SID，conhost
    // 初始化会半途而废（输出堆垃圾），私有桌面是必需项。
    si.lpDesktop = desktop_wide.as_mut_ptr();

    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    // SAFETY: 所有指针/缓冲区均在调用期间有效；cmdline 为 *mut u16（Win32 可能
    // 就地修改，传出的 Vec 所有权保留在此作用域内）。
    let ok = unsafe {
        CreateProcessAsUserW(
            spawn_token.raw(),
            appname.as_ptr(),
            cmdline.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1, // bInheritHandles：继承管道写端
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            env_block.as_ptr().cast(),
            cwd_wide.as_ptr(),
            &si,
            &mut pi,
        )
    };
    if ok == 0 {
        return Err(format!("CreateProcessAsUserW 失败（GetLastError={}）", unsafe {
            windows_sys::Win32::Foundation::GetLastError()
        }));
    }
    // SAFETY: hThread 为 CreateProcess 返回的有效线程句柄，父进程不需要。
    unsafe { CloseHandle(pi.hThread) };

    // ── 7. JobObject 兜底进程树清理 ──
    // SAFETY: pi.hProcess 是 CreateProcessAsUserW 刚返回的有效进程句柄。
    let job = match unsafe { crate::job_object::assign_job_raw(pi.hProcess) } {
        Ok(j) => j,
        Err(e) => {
            // 关联失败时手动终止，防进程泄漏
            // SAFETY: pi.hProcess 为有效进程句柄。
            unsafe { windows_sys::Win32::System::Threading::TerminateProcess(pi.hProcess, 1) };
            return Err(format!("关联 JobObject 失败: {e}"));
        },
    };
    let job = std::sync::Arc::new(job);

    // ── 8. 句柄移交 + 输出读取线程 ──
    // 读端转给 reader 线程（File 接管并负责关闭）；写端已被子进程继承，
    // 父进程必须关闭，否则读端永远收不到 EOF。守卫 forget 移交后 Drop 不再重复关闭。
    let out_read_raw = out_read.forget();
    let err_read_raw = err_read.forget();
    let out_write_raw = out_write.forget();
    let err_write_raw = err_write.forget();
    // SAFETY: 有效句柄，一次性关闭（所有权已移交，后续无人再关）。
    unsafe {
        CloseHandle(out_write_raw);
        CloseHandle(err_write_raw);
    }

    let (stdout_tx, stdout_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let (stderr_tx, stderr_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    spawn_pipe_reader(out_read_raw as isize, stdout_tx);
    spawn_pipe_reader(err_read_raw as isize, stderr_tx);

    Ok(SandboxedChild {
        process: std::sync::Arc::new(RawProcess(pi.hProcess)),
        _job: job,
        stdout_rx: Some(stdout_rx),
        stderr_rx: Some(stderr_rx),
    })
}

/// 启动阻塞读取线程：读端读到 EOF（或写端全部关闭）后把累计输出发给 tx。
fn spawn_pipe_reader(read_end: isize, tx: std::sync::mpsc::Sender<Vec<u8>>) {
    use std::io::Read;
    use std::os::windows::io::RawHandle;
    std::thread::spawn(move || {
        // SAFETY: read_end 为有效管道读句柄；File 接管后由其 Drop 关闭。
        // 句柄以 isize 传递（*mut c_void 非Send，无法直接 move 进线程）。
        let file: std::fs::File =
            unsafe { std::os::windows::io::FromRawHandle::from_raw_handle(read_end as RawHandle) };
        let mut buf = Vec::new();
        let _ = file.take(PIPE_READ_CAP_BYTES).read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
}

/// 构建白名单环境块（UTF-16，`K=V\0` 序列 + 终止 `\0`）。
///
/// Win32 硬性要求：环境块字符串必须按变量名**大小写不敏感字母序排序**
/// （CreateProcess 文档明确要求；实测乱序环境块会导致子进程环境表/堆
/// 损坏——cmd 回显追加乱路径碎片、type/dir 随机报「找不到文件/路径」）。
/// 同名变量（大小写变体，Windows 环境可能出现 `Path` 与 `PATH` 并存）只保留首个。
fn build_env_block() -> Vec<u16> {
    let whitelist: Vec<String> = ENV_WHITELIST.iter().map(|s| s.to_ascii_lowercase()).collect();
    let mut entries: Vec<String> = Vec::new();
    for (k, v) in std::env::vars() {
        if whitelist.iter().any(|w| *w == k.to_ascii_lowercase())
            && !entries
                .iter()
                .any(|e: &String| e.split('=').next().is_some_and(|n| n.eq_ignore_ascii_case(&k)))
        {
            entries.push(format!("{k}={v}"));
        }
    }
    // 大小写不敏感字母序（Win32 要求）
    entries.sort_by(|a, b| {
        a.split('=')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .cmp(&b.split('=').next().unwrap_or("").to_ascii_lowercase())
    });
    let mut block: Vec<u16> = Vec::new();
    for e in &entries {
        block.extend(e.encode_utf16());
        block.push(0);
    }
    block.push(0);
    block
}

/// 解析 cmd.exe 绝对路径（COMSPEC 优先，回退系统目录）
fn resolve_cmd_path() -> String {
    std::env::var("ComSpec").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| {
        let sys_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
        format!("{sys_root}\\System32\\cmd.exe")
    })
}

/// 私有 Window Station + Desktop（进程级单例）。
///
/// 受限令牌访问 `WinSta0\Default` 桌面时 restricted check 失败（GetLastError=5，
/// 桌面 DACL 只授予用户/SYSTEM/Administrators，不含 Everyone/Users）。
/// Chrome sandbox 同款解法：创建私有 window station + desktop，
/// DACL 授予 Everyone/Everyone-Users `GENERIC_ALL`（SDDL `D:(A;;GA;;;WD)(A;;GA;;;BU)`），
/// 子进程在该桌面上运行（CREATE_NO_WINDOW，无 UI，仅管道 IO）。
///
/// 句柄与 SD 有意泄漏（进程生命周期 = 沙箱生命周期）；station 名由系统按登录
/// 会话 LUID 自动生成（非提权进程不能指定名称），同会话多实例天然复用。
struct SandboxStation {
    /// HWINSTA（isize 存储：裸句柄非 Send，静态缓存需要）
    _hwinsta: isize,
    /// HDESK
    _hdesk: isize,
    /// `"<winsta>\\<desktop>"`，直接填入 STARTUPINFOW.lpDesktop
    desktop_path: String,
}

fn sandbox_station() -> Result<&'static SandboxStation, String> {
    static STATION: std::sync::OnceLock<Result<SandboxStation, String>> =
        std::sync::OnceLock::new();

    STATION.get_or_init(|| {
        use windows_sys::Win32::Foundation::{GENERIC_ALL, HANDLE};
        use windows_sys::Win32::Security::Authorization::
            ConvertStringSecurityDescriptorToSecurityDescriptorW;
        use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
        use windows_sys::Win32::System::StationsAndDesktops::{
            CreateDesktopW, CreateWindowStationW, GetUserObjectInformationW, GetProcessWindowStation,
            SetProcessWindowStation, UOI_NAME,
        };

        const SDDL_REVISION_1: u32 = 1;

        // 1. SDDL → SECURITY_DESCRIPTOR：Everyone + Users 均为 GENERIC_ALL
        let sddl: Vec<u16> = "D:(A;;GA;;;WD)(A;;GA;;;BU)".encode_utf16().chain([0]).collect();
        let mut sd: *mut std::ffi::c_void = std::ptr::null_mut();
        // SAFETY: sddl 以 null 结尾且调用期间有效；输出指针有效。
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err("ConvertStringSecurityDescriptorToSecurityDescriptorW 失败".to_string());
        }
        // SD 有意泄漏：window station/desktop 引用它，进程退出统一回收。
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd,
            bInheritHandle: 0,
        };

        // 2. 私有 window station
        // 注意：MSDN 规定「仅管理员组成员可以指定名称」——未提权进程带名创建
        // 必然 ERROR_ACCESS_DENIED（已实测确认）。因此传 NULL 名称，由系统按
        // 登录会话 LUID 自动命名（"Service-0x0-<luid>$"），随后查询实际站名。
        // 同一登录会话内重复调用会返回已有站点句柄，安全幂等。
        let desktop_name = "sbx";
        // SAFETY: sa 指针有效；NULL 名称表示由系统自动命名。
        let hwinsta = unsafe { CreateWindowStationW(std::ptr::null(), 0, GENERIC_ALL, &sa) };
        if hwinsta.is_null() {
            return Err(format!(
                "CreateWindowStationW 失败（GetLastError={}）",
                unsafe { windows_sys::Win32::Foundation::GetLastError() }
            ));
        }

        // 查询自动生成的站名（UOI_NAME），用于拼接 STARTUPINFOW.lpDesktop
        let mut name_buf = [0u16; 128];
        let mut name_need: u32 = 0;
        // SAFETY: hwinsta 为刚创建的有效句柄；缓冲区与长度匹配。
        let ok = unsafe {
            GetUserObjectInformationW(
                hwinsta,
                UOI_NAME,
                name_buf.as_mut_ptr().cast(),
                (name_buf.len() * 2) as u32,
                &mut name_need,
            )
        };
        if ok == 0 {
            return Err(format!(
                "GetUserObjectInformationW(UOI_NAME) 失败（GetLastError={}）",
                unsafe { windows_sys::Win32::Foundation::GetLastError() }
            ));
        }
        let name_len = name_buf.iter().position(|&c| c == 0).unwrap_or(0);
        let winsta_name = String::from_utf16_lossy(&name_buf[..name_len]);

        // 3. 切换到新 winsta 后建 desktop，随后切回
        // SAFETY: 句柄有效。
        let prev = unsafe { GetProcessWindowStation() };
        // SAFETY: 句柄有效。
        let ok = unsafe { SetProcessWindowStation(hwinsta) };
        if ok == 0 {
            return Err("SetProcessWindowStation(沙箱) 失败".to_string());
        }
        let desktop_wide: Vec<u16> = desktop_name.encode_utf16().chain([0]).collect();
        // SAFETY: 名称与 sa 有效；devmode 为 null。
        let hdesk = unsafe {
            CreateDesktopW(
                desktop_wide.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                GENERIC_ALL,
                &sa,
            )
        };
        // SAFETY: 句柄有效；无论 desktop 是否建成，都切回原 window station。
        let _ = unsafe { SetProcessWindowStation(prev) };
        if hdesk.is_null() {
            return Err(format!(
                "CreateDesktopW 失败（GetLastError={}）",
                unsafe { windows_sys::Win32::Foundation::GetLastError() }
            ));
        }

        Ok(SandboxStation {
            _hwinsta: hwinsta as HANDLE as isize,
            _hdesk: hdesk as HANDLE as isize,
            desktop_path: format!("{winsta_name}\\{desktop_name}"),
        })
    })
    .as_ref()
    .map_err(|e| e.clone())
}

/// Win32 句柄 RAII 守卫
struct HandleGuard(windows_sys::Win32::Foundation::HANDLE);

impl HandleGuard {
    fn raw(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.0
    }

    /// 放弃守卫所有权，返回裸句柄（移交子进程 / reader 线程时使用）
    fn forget(self) -> windows_sys::Win32::Foundation::HANDLE {
        let h = self.0;
        std::mem::forget(self);
        h
    }
}

impl Drop for HandleGuard {
    fn drop(&mut self) {
        // SAFETY: self.0 为有效句柄，Drop 后不再使用。
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

// ── 令牌与 SID ─────────────────────────────────────────────────────

/// capability SID 的用途档（两档各用一个独立 SID，理由见 [`capability_sid`]）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum CapKind {
    ReadOnly,
    WorkspaceWrite,
}

/// capability SID：`S-1-5-21-<a>-<b>-<c>-<d>`，五个子权威全是**编译期常量**。
///
/// ## 为什么是这个形态（实测结论，不要改回 `S-1-15-*`）
///
/// `CreateRestrictedToken` **不接受 App Package 段（`S-1-15-*`）的 SID 进入
/// restricting 列表**，且与 flags 组合无关。2026-09-25 实测（非提权标准用户，
/// 三组对照，`IsValidSid` 全部返回 1）：
///
/// | restricting 列表 | 结果 |
/// |---|---|
/// | 手工拼 `S-1-15-3-1024-<4 段>`（32 字节 / 6 子权威） | **87**（`ERROR_INVALID_PARAMETER`） |
/// | `DeriveCapabilitySidsFromName("AxAgentSandbox")` 派生的真 capability SID（48 字节 / 10 子权威 / 权威段 15） | **87** |
/// | 同位置的 `Logon`（`S-1-5-5-*`）/ `Everyone`（`S-1-1-0`） | 成功 |
///
/// ⇒ 结论：**不是「自造 vs 系统派生」的问题，而是 `S-1-15-*` 段整体进不了 restricting
/// 列表**。codex 的「capability SID」实为 `S-1-5-21-<a>-<b>-<c>-<d>`
/// （`cap.rs::make_random_cap_sid_string`）—— 在「账户域」段里自造一个不存在的域，
/// 内核照收；本模块采用同一形态。
///
/// ## 为什么是固定常量，而不是像 codex 那样随机生成 + 落盘
///
/// 本 SID 只出现在三处：沙箱令牌 restricting 列表、工作区 allow ACE、WFP 过滤条件。
/// 后两者**会持久化**（ACE 写进目录 DACL、WFP 条件按 SID 匹配）⇒ SID 必须跨次运行
/// 稳定，否则上次写下的 ACE 永久失配、WFP 条件永不命中。编译期常量即可满足，
/// 且省掉一个持久化状态文件（codex 随机 + 落盘 `codex_home/cap_sid` 是为了再支持
/// 「按工作区分 SID」的隔离，本模块不做，见模块文档「当前边界」4）。
///
/// ## 为什么分两档（`CapKind`）
///
/// `ReadOnly` 与 `WorkspaceWrite` 必须用**不同**的 capability SID。若两档共用一个，
/// `grant_workspace_write` 打在工作区上的 allow ACE 会**遗留**给之后的 `ReadOnly`
/// 会话（ACE 幂等且设计上不清理）⇒ `ReadOnly` 档就能写工作区
/// （`read_only_cannot_write_workspace` 实测复现）。codex `cap.rs` 同款做法：
/// `CapSids { workspace, readonly, .. }` 是两个独立 SID。
fn capability_sid(kind: CapKind) -> Vec<u8> {
    /// 子权威：`21`（`SECURITY_NT_NON_UNIQUE`）+ `"AxAg" | "entS" | "andb" | "ox\0"+档位`。
    const SUB_AUTHORITY_PREFIX: [u32; 4] = [21, 0x4178_4167, 0x656E_7453, 0x616E_6462];
    /// 末位子权威：`"ox"` 打底 + 档位（`0x6F78_0001` / `0x6F78_0002`）。
    const WORKSPACE_TAG: u32 = 0x6F78_0001;
    const READ_ONLY_TAG: u32 = 0x6F78_0002;

    let tag = match kind {
        CapKind::WorkspaceWrite => WORKSPACE_TAG,
        CapKind::ReadOnly => READ_ONLY_TAG,
    };
    let mut buf = Vec::with_capacity(8 + 4 * (SUB_AUTHORITY_PREFIX.len() + 1));
    buf.push(1); // Revision
    buf.push((SUB_AUTHORITY_PREFIX.len() + 1) as u8); // SubAuthorityCount
    buf.extend_from_slice(&[0, 0, 0, 0, 0, 5]); // IdentifierAuthority（大端 6 字节 = 5，即 `S-1-5-*`）
    for s in SUB_AUTHORITY_PREFIX.into_iter().chain([tag]) {
        buf.extend_from_slice(&s.to_le_bytes()); // 子权威逐个按小端追加
    }
    buf
}

/// Everyone（`WinWorldSid`）的 SID 字节。
fn everyone_sid() -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Security::{CreateWellKnownSid, WinWorldSid};
    // SECURITY_MAX_SID_SIZE = 68（WinNT.h 定义：任意 SID 的字节上限）
    let mut buf = vec![0u8; 68];
    let mut len = buf.len() as u32;
    // SAFETY: 缓冲区 68 字节（>= 任何 SID 长度）；内置权威传 null。
    let ok = unsafe {
        CreateWellKnownSid(WinWorldSid, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len)
    };
    if ok == 0 {
        return Err(format!("CreateWellKnownSid(Everyone) 失败（GetLastError={}）", unsafe {
            windows_sys::Win32::Foundation::GetLastError()
        }));
    }
    buf.truncate(len as usize);
    Ok(buf)
}

/// 从令牌取 Logon SID（`SE_GROUP_LOGON_ID` 属性的那个 SID）。
///
/// 私有 Window Station / Desktop 的对象 DACL 授 Everyone/Users，但会话级对象
/// （桌面、窗口站、部分核心对象）以 Logon SID 授权 ⇒ 必须放进 restricting 列表。
fn logon_sid(token: windows_sys::Win32::Foundation::HANDLE) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Security::{
        GetLengthSid, GetTokenInformation, TOKEN_GROUPS, TokenGroups,
    };
    // SE_GROUP_LOGON_ID（winnt.h 0xC0000000；windows-sys 未导出该常量）
    const SE_GROUP_LOGON_ID: u32 = 0xC000_0000;

    let mut need: u32 = 0;
    // SAFETY: 先取所需长度：缓冲区 null + 长度 0 是 Win32 约定用法。
    unsafe { GetTokenInformation(token, TokenGroups, std::ptr::null_mut(), 0, &mut need) };
    if need == 0 {
        return Err("GetTokenInformation(TokenGroups) 取长度失败".to_string());
    }
    let mut buf = vec![0u8; need as usize];
    // SAFETY: 缓冲区按 need 字节分配；token 为有效令牌句柄。
    let ok = unsafe {
        GetTokenInformation(token, TokenGroups, buf.as_mut_ptr().cast(), need, &mut need)
    };
    if ok == 0 {
        return Err(format!("GetTokenInformation(TokenGroups) 失败（GetLastError={}）", unsafe {
            windows_sys::Win32::Foundation::GetLastError()
        }));
    }
    // SAFETY: buf 起始即 TOKEN_GROUPS，随后是 GroupCount 个 SID_AND_ATTRIBUTES。
    let (count, groups_ptr) = unsafe {
        let groups = buf.as_ptr().cast::<TOKEN_GROUPS>();
        ((*groups).GroupCount as usize, (*groups).Groups.as_ptr())
    };
    // SAFETY: groups_ptr 指向 buf 内的 count 个 SID_AND_ATTRIBUTES。
    for entry in unsafe { std::slice::from_raw_parts(groups_ptr, count) } {
        if entry.Attributes & SE_GROUP_LOGON_ID == SE_GROUP_LOGON_ID && !entry.Sid.is_null() {
            // SAFETY: entry.Sid 为令牌内的有效 SID 指针。
            let len = unsafe { GetLengthSid(entry.Sid) } as usize;
            if len > 0 {
                // SAFETY: len 由 GetLengthSid 给出，SID 在 buf 存活期内有效。
                return Ok(
                    unsafe { std::slice::from_raw_parts(entry.Sid as *const u8, len) }.to_vec()
                );
            }
        }
    }
    Err("当前令牌中找不到 Logon SID（SE_GROUP_LOGON_ID）".to_string())
}

/// 在令牌上启用单个特权（`SE_PRIVILEGE_ENABLED`）。
///
/// `CreateRestrictedToken` 的 `DISABLE_MAX_PRIVILEGE` 会收走基础令牌的特权集，
/// 而 `CreateProcessAsUserW` 用新令牌打开映像 / cwd 时依赖
/// `SeChangeNotifyPrivilege`（遍历检查豁免）——缺它时实测
/// `GetLastError=5 ERROR_ACCESS_DENIED`。codex `token.rs` 的
/// `create_token_with_caps_from` 末尾同样显式重新启用该特权。
fn enable_privilege(
    token: windows_sys::Win32::Foundation::HANDLE,
    name: &str,
) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{GetLastError, LUID, SetLastError};
    use windows_sys::Win32::Security::{
        AdjustTokenPrivileges, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED, TOKEN_PRIVILEGES,
    };

    let name_wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
    let mut luid = LUID { LowPart: 0, HighPart: 0 };
    // SAFETY: name_wide 为 null 结尾宽串；luid 输出指针有效。
    let ok = unsafe { LookupPrivilegeValueW(std::ptr::null(), name_wide.as_ptr(), &mut luid) };
    if ok == 0 {
        return Err(format!("LookupPrivilegeValueW({name}) 失败（GetLastError={}）", unsafe {
            GetLastError()
        }));
    }
    let mut tp: TOKEN_PRIVILEGES = unsafe { std::mem::zeroed() };
    tp.PrivilegeCount = 1;
    tp.Privileges[0].Luid = luid;
    tp.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
    // SAFETY: token 为有效令牌句柄；tp 生命周期覆盖本次调用。
    unsafe { SetLastError(0) };
    let ok = unsafe {
        AdjustTokenPrivileges(token, 0, &tp, 0, std::ptr::null_mut(), std::ptr::null_mut())
    };
    if ok == 0 {
        return Err(format!("AdjustTokenPrivileges({name}) 失败（GetLastError={}）", unsafe {
            GetLastError()
        }));
    }
    // AdjustTokenPrivileges 即使「特权不在令牌里」也返回成功，必须查 LastError
    // （已按 MSDN 在调用前置零，此处非零即真失败）。
    let err = unsafe { GetLastError() };
    if err != 0 {
        return Err(format!("AdjustTokenPrivileges({name}) 未生效（GetLastError={err}）"));
    }
    Ok(())
}

/// `OWNER RIGHTS`（`S-1-3-4`）的 SID 字节。
///
/// 只在默认 DACL 里用：对象的属主始终是当前账户，属主自带隐式
/// `WRITE_DAC | READ_CONTROL`，仅靠「不授 user SID」压不住；显式写一条
/// `OWNER RIGHTS` ACE 才能把属主的隐式权限收窄（codex `token.rs`
/// `set_default_dacl` 同款做法）。
fn owner_rights_sid() -> Vec<u8> {
    // S-1-3-4：Revision=1、SubAuthorityCount=1、IdentifierAuthority=3（大端 6 字节）、
    // 子权威 [4]（小端）。
    vec![1, 1, 0, 0, 0, 0, 0, 3, 4, 0, 0, 0]
}

/// 把受限令牌的默认 DACL 改写为「本登录会话 ALL + 属主只读」。
///
/// **子进程初始化失败根因（2026-09-25 实测）**：受限令牌的访问检查是两遍制，
/// **写**访问必须在第二遍（只认 restricting 列表里 SID 的 allow ACE）也通过。派生
/// 令牌的默认 DACL 是从基础令牌继承的（`SYSTEM` / `Administrators` / 当前 user SID，
/// **不含** capability / Logon / Everyone 中的任何一个）⇒ 子进程初始化期间对自己新建
/// 对象发起的写访问在第二遍被拒，`cmd.exe` 直接以 `0xC0000142`
/// （`STATUS_DLL_INIT_FAILED`）退出。
///
/// 9 档探针实测（同桌面 / 同 env / 同命令行）：flags `0x0 / 0x4 / 0x7 / 0xD` ×
/// restricting 列表「空 / 三选组合」**全部 `0xC0000142`**；唯一把退出码变成 `0x0`
/// 的变量就是本函数。与 `lpDesktop` 无关（私有桌面与 `WinSta0\Default` 均 `0x0`）。
///
/// 返回前用 `LocalFree` 释放 `SetEntriesInAclW` 分配的 ACL（`SetTokenInformation`
/// 只复制内容，令牌不持有该缓冲）。
fn set_default_dacl(
    token: windows_sys::Win32::Foundation::HANDLE,
    logon_sid: &[u8],
) -> Result<(), String> {
    use windows_sys::Win32::Security::Authorization::{
        EXPLICIT_ACCESS_W, GRANT_ACCESS, SetEntriesInAclW, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN,
        TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{ACL, SetTokenInformation, TokenDefaultDacl};

    /// `SetTokenInformation(TokenDefaultDacl, ...)` 的入参形态（单个 `ACL*`）。
    #[repr(C)]
    struct TokenDefaultDaclInfo {
        default_dacl: *mut ACL,
    }

    const GENERIC_ALL: u32 = 0x1000_0000;
    // READ_CONTROL（winnt.h 0x00020000）
    const READ_CONTROL: u32 = 0x0002_0000;

    let owner_rights = owner_rights_sid();
    // 顺序即语义：登录会话在 restricting 列表内（放行第二遍写检查），
    // OWNER RIGHTS 紧随其后收窄属主隐式权限。
    let entries = [
        (logon_sid.as_ptr() as *mut std::ffi::c_void, GENERIC_ALL),
        (owner_rights.as_ptr() as *mut std::ffi::c_void, READ_CONTROL),
    ]
    .map(|(sid, access)| EXPLICIT_ACCESS_W {
        grfAccessPermissions: access,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: 0,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid as *mut u16,
        },
    });

    let mut dacl: *mut ACL = std::ptr::null_mut();
    // SAFETY: entries 指向本作用域内存活的 SID；dacl 输出指针有效。
    let rc = unsafe {
        SetEntriesInAclW(entries.len() as u32, entries.as_ptr(), std::ptr::null_mut(), &mut dacl)
    };
    if rc != 0 {
        return Err(format!("SetEntriesInAclW(默认 DACL) 失败（rc={rc}）"));
    }
    let mut info = TokenDefaultDaclInfo { default_dacl: dacl };
    // SAFETY: token 为有效令牌句柄（含 TOKEN_ADJUST_DEFAULT）；info 生命周期覆盖本次调用。
    let ok = unsafe {
        SetTokenInformation(
            token,
            TokenDefaultDacl,
            &mut info as *mut _ as *mut std::ffi::c_void,
            std::mem::size_of::<TokenDefaultDaclInfo>() as u32,
        )
    };
    let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    if !dacl.is_null() {
        // SAFETY: dacl 由 SetEntriesInAclW 经 LocalAlloc 分配，令牌已复制其内容。
        unsafe { windows_sys::Win32::Foundation::LocalFree(dacl as *mut std::ffi::c_void) };
    }
    if ok == 0 {
        return Err(format!("SetTokenInformation(TokenDefaultDacl) 失败（GetLastError={err}）"));
    }
    Ok(())
}

/// 构造沙箱令牌：`CreateRestrictedToken(DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED)`
/// + restricting 列表「capability（按 [`CapKind`] 分档）→ Logon → Everyone」（顺序有语义，
///   见模块文档「原理」3）。
///
/// 不 disable 任何 SID；创建后补回 `SeChangeNotifyPrivilege`（codex `token.rs`
/// `create_token_with_caps_from` 同款：`DISABLE_MAX_PRIVILEGE` 下该特权虽保留，
/// 但可能处于 disabled 状态，显式置 ENABLED 更稳），并改写默认 DACL
/// （见 [`set_default_dacl`]，漏掉这一步子进程会以 `0xC0000142` 退出）。
///
/// **spawn 失败根因（2026-09-25 实测）**：`CreateRestrictedToken` 返回的令牌句柄，
/// 其**已授予访问权来自基础令牌句柄**。基础句柄若未请求 `TOKEN_ASSIGN_PRIMARY`，
/// 新句柄同样没有 ⇒ `CreateProcessAsUserW` 访问检查失败，报
/// `GetLastError=5 ERROR_ACCESS_DENIED`。探针矩阵把「flags（0x0/0x4/0xD）×
/// restricting 列表（capability / user SID / logon+Everyone）× lpDesktop
/// （私有 / null / WinSta0\Default）」跑全 8 档**全是 5**；唯一有效变量是基础句柄
/// 掩码——补 `0x0001` 后立刻 `ok=1`（`DuplicateTokenEx(TOKEN_ALL_ACCESS)` 等效）。
/// 与 capability SID 形态、`WRITE_RESTRICTED`、私有桌面均无关。
fn sandbox_token(kind: CapKind) -> Result<HandleGuard, String> {
    use windows_sys::Win32::Foundation::{GetLastError, HANDLE};
    use windows_sys::Win32::Security::{
        CreateRestrictedToken, DISABLE_MAX_PRIVILEGE, LUA_TOKEN, SID_AND_ATTRIBUTES,
        WRITE_RESTRICTED,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut current: HANDLE = std::ptr::null_mut();
    // SAFETY: GetCurrentProcess 返回伪句柄；输出指针有效。
    // 访问掩码必须含 `TOKEN_ASSIGN_PRIMARY`（0x0001）：`CreateRestrictedToken`
    // 派生出的新令牌句柄，其**已授予访问权出自基础句柄**——基础句柄没请求该权限，
    // 新句柄就没有，`CreateProcessAsUserW` 对它做访问检查时直接
    // `GetLastError=5 ERROR_ACCESS_DENIED`（根因详载于 [`sandbox_token`] 文档）。
    // 同时含 `TOKEN_ADJUST_DEFAULT`（0x0080）：派生句柄要靠它改写默认 DACL。
    let ok = unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            0x0001 | 0x0002 | 0x0008 | 0x0020 | 0x0080,
            &mut current,
        )
    };
    if ok == 0 {
        return Err(format!("OpenProcessToken 失败（GetLastError={}）", unsafe {
            GetLastError()
        }));
    }
    let current = HandleGuard(current);

    let cap = capability_sid(kind);
    let logon = logon_sid(current.raw())?;
    let everyone = everyone_sid()?;
    let restricting = [
        SID_AND_ATTRIBUTES { Sid: cap.as_ptr() as *mut std::ffi::c_void, Attributes: 0 },
        SID_AND_ATTRIBUTES { Sid: logon.as_ptr() as *mut std::ffi::c_void, Attributes: 0 },
        SID_AND_ATTRIBUTES { Sid: everyone.as_ptr() as *mut std::ffi::c_void, Attributes: 0 },
    ];

    let mut new_token: HANDLE = std::ptr::null_mut();
    // SAFETY: 不 disable SID / 不删特权（计数 0，指针 null）；restricting 指向本作用域内
    // 3 个有效 SID（缓冲区在调用期间存活）。
    let ok = unsafe {
        CreateRestrictedToken(
            current.raw(),
            DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            restricting.len() as u32,
            restricting.as_ptr(),
            &mut new_token,
        )
    };
    if ok == 0 {
        return Err(format!("CreateRestrictedToken 失败（GetLastError={}）", unsafe {
            GetLastError()
        }));
    }
    let spawn_token = HandleGuard(new_token);
    // 顺序照 codex `token.rs`：先改默认 DACL（子进程自建对象的写检查要在第二遍通过），
    // 再补特权。
    set_default_dacl(spawn_token.raw(), &logon)?;
    // DISABLE_MAX_PRIVILEGE 清空特权集后，新令牌连目录遍历豁免都没有：
    // CreateProcessAsUserW 打开 cmd.exe / cwd 时会 ERROR_ACCESS_DENIED（实测 5）。
    enable_privilege(spawn_token.raw(), "SeChangeNotifyPrivilege")?;
    Ok(spawn_token)
}

// ── 工作区 allow ACE（WorkspaceWrite 档） ───────────────────────────

/// 给工作区打 capability SID 的 allow ACE（容器与对象均继承）。
///
/// 幂等且**不清理**：`SetEntriesInAclW` 把新 ACE 合并进现有 DACL；capability SID
/// 对非沙箱令牌无意义（不在其 SID 列表内），故 ACE 常驻不会放宽宿主权限。清理反而
/// 会与并发运行的其它沙箱互踩，且下次运行还要重打一遍。
fn grant_workspace_write(workspace: &Path) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{GENERIC_EXECUTE, GENERIC_READ, GENERIC_WRITE, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW, NO_MULTIPLE_TRUSTEE,
        SE_FILE_OBJECT, SetEntriesInAclW, SetNamedSecurityInfoW, TRUSTEE_IS_SID,
        TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{ACL, DACL_SECURITY_INFORMATION};

    // 工作区不存在则先建（ACL 无处可打；不建则 spawn 时 cmd 的 cwd 会失败）
    if !workspace.exists() {
        std::fs::create_dir_all(workspace)
            .map_err(|e| format!("创建工作区目录 {} 失败: {e}", workspace.display()))?;
    }

    let cap = capability_sid(CapKind::WorkspaceWrite);
    let mut wide: Vec<u16> = workspace.to_string_lossy().encode_utf16().chain([0]).collect();

    let mut old_dacl: *mut ACL = std::ptr::null_mut();
    let mut sd: *mut std::ffi::c_void = std::ptr::null_mut();
    // SAFETY: wide 为 null 结尾宽串；输出指针均有效。
    let rc = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut old_dacl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if rc != 0 {
        return Err(format!("GetNamedSecurityInfoW 失败（错误码 {}）", rc));
    }

    // 0x3 = CONTAINER_INHERIT_ACE | OBJECT_INHERIT_ACE（子目录与文件都继承）
    const INHERIT_ALL: u32 = 0x3;
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: GENERIC_READ | GENERIC_WRITE | GENERIC_EXECUTE,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: INHERIT_ALL,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: cap.as_ptr() as *mut u16,
        },
    };
    let mut new_dacl: *mut ACL = std::ptr::null_mut();
    // SAFETY: entry.Trustee.ptstrName 指向 cap（调用期间存活）；old_dacl 由系统分配。
    let rc = unsafe { SetEntriesInAclW(1, &entry, old_dacl, &mut new_dacl) };
    if rc != 0 {
        // SAFETY: sd 由 GetNamedSecurityInfoW 经 LocalAlloc 分配，此处释放。
        unsafe { LocalFree(sd) };
        return Err(format!("SetEntriesInAclW 失败（错误码 {}）", rc));
    }
    // SAFETY: wide 为 null 结尾宽串；new_dacl 有效。
    let rc = unsafe {
        SetNamedSecurityInfoW(
            wide.as_mut_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            new_dacl,
            std::ptr::null_mut(),
        )
    };
    // SAFETY: 两个缓冲区均由系统分配，用完即还。
    unsafe {
        LocalFree(new_dacl.cast());
        LocalFree(sd);
    }
    if rc != 0 {
        return Err(format!("SetNamedSecurityInfoW 失败（错误码 {}）", rc));
    }
    Ok(())
}

// ── 网络封锁（WFP） ─────────────────────────────────────────────────

use windows_sys::Win32::NetworkManagement::WindowsFilteringPlatform::{
    FWPM_LAYER_ALE_AUTH_CONNECT_V4, FWPM_LAYER_ALE_AUTH_CONNECT_V6,
    FWPM_LAYER_ALE_RESOURCE_ASSIGNMENT_V4, FWPM_LAYER_ALE_RESOURCE_ASSIGNMENT_V6,
};
use windows_sys::core::GUID;

/// 自建 WFP 子层键（固定值 ⇒ 过滤器标识稳定、重装幂等）。
const WFP_SUBLAYER_KEY: GUID = GUID::from_u128(0x0a1b_7f10_5c3d_4e21_9f88_1c2d_3e4f_5a60);
/// 过滤器 key：`2 档 CapKind × 4 层`，索引 = 档位序号 × 4 + 层序号。
///
/// 两档都要挂：两个 capability SID 不同（见 [`CapKind`]），只封一档会让「先跑过另一档」
/// 的会话漏网。本进程内只装一次（会话缓存），故必须一次把两档都封上。
const WFP_FILTER_KEYS: [GUID; 8] = [
    // 档位 0（ReadOnly）× 4 层
    GUID::from_u128(0x0a1b_7f11_5c3d_4e21_9f88_1c2d_3e4f_5a61),
    GUID::from_u128(0x0a1b_7f12_5c3d_4e21_9f88_1c2d_3e4f_5a62),
    GUID::from_u128(0x0a1b_7f13_5c3d_4e21_9f88_1c2d_3e4f_5a63),
    GUID::from_u128(0x0a1b_7f14_5c3d_4e21_9f88_1c2d_3e4f_5a64),
    // 档位 1（WorkspaceWrite）× 4 层
    GUID::from_u128(0x0a1b_7f21_5c3d_4e21_9f88_1c2d_3e4f_5a65),
    GUID::from_u128(0x0a1b_7f22_5c3d_4e21_9f88_1c2d_3e4f_5a66),
    GUID::from_u128(0x0a1b_7f23_5c3d_4e21_9f88_1c2d_3e4f_5a67),
    GUID::from_u128(0x0a1b_7f24_5c3d_4e21_9f88_1c2d_3e4f_5a68),
];
/// 四层：出站连接（TCP connect / 无连接 UDP 发送）与套接字绑定（bind / listen）。
const WFP_FILTER_LAYERS: [GUID; 4] = [
    FWPM_LAYER_ALE_AUTH_CONNECT_V4,
    FWPM_LAYER_ALE_AUTH_CONNECT_V6,
    FWPM_LAYER_ALE_RESOURCE_ASSIGNMENT_V4,
    FWPM_LAYER_ALE_RESOURCE_ASSIGNMENT_V6,
];

/// 网络封锁的**实际强度**（如实上报，调用方不得从 `policy.network_access` 反推）。
///
/// 分档依据是实测：WFP 对象（子层 / 过滤器）的安装**需要提权**，而本沙箱刻意
/// 走非提权受限令牌（不建本地账户，常驻 SYSTEM 服务见计划 §8 排除）。故在非提权
/// 进程里，网络封锁有一档可判定的结构性缺失，必须显式表达而不是假装成功。
#[derive(Debug, Clone)]
pub enum NetworkBlock {
    /// 自建高权重子层（0x8000）安装成功：block 在仲裁中优先于系统防火墙的 allow。
    Enforced,
    /// 只能挂在系统内建 universal 子层（权重最低）：过滤器确实装上并生效，但若
    /// 系统防火墙对同一条流存在更高权重的 allow，本 block 会被压过。
    BestEffort,
    /// 结构性不可用：**非提权进程无权安装 WFP 对象**（实测 `FwpmSubLayerAdd0` /
    /// `FwpmFilterAdd0` 返回 `ERROR_ACCESS_DENIED` 0x00000005）。此时网络**未**被
    /// 阻断 —— 调用方不得据此声称已断网。
    Unavailable { reason: String },
}

/// 安装（进程内一次性）按 capability SID 阻断网络的 WFP 过滤器。
///
/// 结果是**实际强度**而非意图，见 [`NetworkBlock`]；调用方不得从
/// `policy.network_access == false` 反推「已断网」。结论进程内缓存一次。
fn ensure_network_block() -> Result<NetworkBlock, String> {
    static INSTALLED: std::sync::OnceLock<Result<NetworkBlock, String>> =
        std::sync::OnceLock::new();
    INSTALLED.get_or_init(wfp_install_block_filters).clone()
}

fn wfp_install_block_filters() -> Result<NetworkBlock, String> {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::NetworkManagement::WindowsFilteringPlatform::{
        FWP_ACTION_BLOCK, FWP_CONDITION_VALUE0, FWP_EMPTY, FWP_MATCH_EQUAL, FWP_SID, FWP_VALUE0,
        FWPM_ACTION0, FWPM_CONDITION_ALE_USER_ID, FWPM_DISPLAY_DATA0, FWPM_FILTER_CONDITION0,
        FWPM_FILTER0, FWPM_SESSION_FLAG_DYNAMIC, FWPM_SESSION0, FWPM_SUBLAYER_UNIVERSAL,
        FWPM_SUBLAYER0, FwpmEngineClose0, FwpmEngineOpen0, FwpmFilterAdd0, FwpmSubLayerAdd0,
    };
    use windows_sys::Win32::Security::SID;
    use windows_sys::Win32::System::Rpc::RPC_C_AUTHN_WINNT;

    /// `ERROR_ACCESS_DENIED`（winerror.h 5）—— 非提权进程安装 WFP 对象的返回码。
    const ERROR_ACCESS_DENIED: u32 = 5;

    // WFP 要求 `displayData.name` 非空，否则 `FwpmSubLayerAdd0` / `FwpmFilterAdd0`
    // 直接返回 `FWP_E_NULL_DISPLAY_NAME`（0x80320023）—— 实测踩过：失败与权限无关，
    // 只差这一个名字。缓冲区在本函数内对所有调用存活。
    let display_name: Vec<u16> =
        "AxAgent sandbox network block".encode_utf16().chain([0]).collect();
    let make_display = || FWPM_DISPLAY_DATA0 {
        name: display_name.as_ptr() as *mut u16,
        description: std::ptr::null_mut(),
    };

    // 动态会话：过滤器生命周期 = 本进程会话，进程退出即自动清除，不残留系统配置。
    let session = FWPM_SESSION0 { flags: FWPM_SESSION_FLAG_DYNAMIC, ..Default::default() };
    let mut engine: HANDLE = std::ptr::null_mut();
    // SAFETY: session 有效；engine 输出指针有效；本机会话无需认证身份。
    let rc = unsafe {
        FwpmEngineOpen0(
            std::ptr::null(),
            RPC_C_AUTHN_WINNT,
            std::ptr::null(),
            &session,
            &mut engine,
        )
    };
    if rc != 0 {
        return Err(format!("FwpmEngineOpen0 失败（0x{rc:08X}）"));
    }

    // 目标子层：优先自建高权重子层（weight 0x8000，高于系统防火墙 ⇒ 仲裁中
    // block 优先于其 allow）；非提权进程建不了子层，退到内建 universal 子层
    // （过滤器仍生效，但权重最低 —— 强度差别见 NetworkBlock::BestEffort）。
    let mut sub_layer_key = WFP_SUBLAYER_KEY;
    let mut status = NetworkBlock::Enforced;

    // 自建子层 weight = 0x8000（高于系统防火墙子层）⇒ 仲裁中 block 优先于其 allow。
    let sublayer = FWPM_SUBLAYER0 {
        subLayerKey: WFP_SUBLAYER_KEY,
        displayData: make_display(),
        flags: 0,
        providerKey: std::ptr::null_mut(),
        providerData: Default::default(),
        weight: 0x8000,
    };
    // SAFETY: sublayer 有效；安全描述符传 null 由系统分配默认值。
    let rc = unsafe { FwpmSubLayerAdd0(engine, &sublayer, std::ptr::null_mut()) };
    if rc != 0 {
        if rc != ERROR_ACCESS_DENIED {
            // SAFETY: engine 为刚成功打开的有效句柄。
            unsafe { FwpmEngineClose0(engine) };
            return Err(format!("FwpmSubLayerAdd0 失败（0x{rc:08X}）"));
        }
        // 实测：非提权进程 FwpmEngineOpen0 成功、FwpmSubLayerAdd0 返回
        // ERROR_ACCESS_DENIED（0x00000005）—— WFP 对象安装需要提权。
        sub_layer_key = FWPM_SUBLAYER_UNIVERSAL;
        status = NetworkBlock::BestEffort;
    }

    // 两档 capability SID 各挂一遍（理由见 WFP_FILTER_KEYS 注释）。
    let caps = [capability_sid(CapKind::ReadOnly), capability_sid(CapKind::WorkspaceWrite)];
    for (kind_index, cap) in caps.iter().enumerate() {
        for (layer_index, layer) in WFP_FILTER_LAYERS.iter().enumerate() {
            let filter_key = &WFP_FILTER_KEYS[kind_index * WFP_FILTER_LAYERS.len() + layer_index];
            // 命名 union 分支只能靠赋值语句写，故先以字面量建初值再补 union 分支
            // （字面量初值不会被 clippy::field_reassign_with_default 判违规）。
            let mut condition_value =
                FWP_CONDITION_VALUE0 { r#type: FWP_SID, ..Default::default() };
            condition_value.Anonymous.sid = cap.as_ptr() as *mut SID;
            let condition = FWPM_FILTER_CONDITION0 {
                fieldKey: FWPM_CONDITION_ALE_USER_ID,
                matchType: FWP_MATCH_EQUAL,
                conditionValue: condition_value,
            };

            let filter = FWPM_FILTER0 {
                filterKey: *filter_key,
                displayData: make_display(),
                layerKey: *layer,
                subLayerKey: sub_layer_key,
                weight: FWP_VALUE0 { r#type: FWP_EMPTY, ..Default::default() },
                numFilterConditions: 1,
                filterCondition: &condition as *const _ as *mut FWPM_FILTER_CONDITION0,
                action: FWPM_ACTION0 { r#type: FWP_ACTION_BLOCK, ..Default::default() },
                ..Default::default()
            };

            let mut id: u64 = 0;
            // SAFETY: filter / condition 在调用期间存活；cap 缓冲区存活；id 输出指针有效。
            let rc = unsafe { FwpmFilterAdd0(engine, &filter, std::ptr::null_mut(), &mut id) };
            if rc != 0 {
                // SAFETY: engine 为有效句柄；整批失败即撤销本次会话的全部过滤器。
                unsafe { FwpmEngineClose0(engine) };
                if rc == ERROR_ACCESS_DENIED {
                    // 连 universal 子层也装不上 ⇒ 结构性不可用，如实上报（不假装已断网，
                    // 也不据此拒绝启动：拒绝会让非提权环境下的沙箱整体不可用）。
                    return Ok(NetworkBlock::Unavailable {
                        reason: format!(
                            "非提权进程无权安装 WFP 过滤器（层 0x{:08X} 返回 ERROR_ACCESS_DENIED）",
                            layer.data1
                        ),
                    });
                }
                return Err(format!(
                    "FwpmFilterAdd0（层 0x{:08X}）失败（0x{rc:08X}）",
                    layer.data1
                ));
            }
        }
    }
    // engine 句柄**有意不关闭**：动态会话的过滤器生命周期 = 会话生命周期，
    // 关句柄等于当场撤销封锁。持有到进程退出，由系统回收（同 sandbox_station 的做法）。
    Ok(status)
}

// ── 测试（Windows only） ──────────────────────────────────────────
//
// 三件改造各自的可实测取证（`PLAN-codex-parity-adoption.md` §7 O-5：三件各自
// 必须配可实测的集成测试，不允许只靠 clippy 过门禁）：
// - ① 令牌换代：`sandboxed_echo_works` / `sandboxed_can_read_system_dir` /
//   `sandboxed_cannot_write_user_profile`
// - ② 工作区 allow ACE：`workspace_write_inside_allowed` /
//   `workspace_write_outside_denied` / `read_only_cannot_write_workspace`
// - ③ 网络封锁：`network_block_status_is_truthful`（真断网 / 结构性不可用二者必居其一）

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use axagent_harness::{SandboxMode, SandboxPolicy};

    /// 文件系统类断言用的策略：`network_access = true` ⇒ **不触发 WFP 安装**，
    /// 把令牌 / allow ACE 的断言与网络封锁解耦（本机非提权时 WFP 装不上，
    /// 见模块文档「当前边界」3；不解除耦合会让整个测试模块的红绿取决于网络项）。
    fn fs_policy(mode: SandboxMode, cwd: impl Into<std::path::PathBuf>) -> SandboxPolicy {
        SandboxPolicy { mode, workspace_cwd: cwd.into(), network_access: true }
    }

    /// 只读档基准（`C:\Windows` 让「写系统目录被拒」有一个天然不可写的目标目录）。
    fn read_only_policy() -> SandboxPolicy {
        fs_policy(SandboxMode::ReadOnly, "C:\\Windows")
    }

    /// 测试用工作区（`WorkspaceWrite` 的 allow ACE 打在它上面）。
    ///
    /// **每个测试一个独立子目录**（`<pid>_<测试名>`），理由两条（都实测踩过）：
    /// ① 共享同一目录时，`WorkspaceWrite` 打上的 allow ACE 与写下的文件会污染
    /// `read_only_cannot_write_workspace` 的断言（ACE 幂等不清理）；
    /// ② 跨次运行遗留的文件同名，会让「文件不应存在」断言在第二次运行起恒假。
    fn temp_workspace(test: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join("axagent_sandbox_ws_probe")
            .join(format!("{}_{test}", std::process::id()))
    }

    /// ① 新受限令牌能 spawn 且基本命令可用。
    ///
    /// 这是令牌换代的**首要回归点**：受限令牌子进程必须在初始化阶段活下来。
    ///
    /// 三类实测失败都锁在这里（都是 2026-09-25 定位的）：
    /// - `GetLastError=5`（spawn 就失败）：基础令牌句柄漏了 `TOKEN_ASSIGN_PRIMARY`；
    /// - `0xC0000142`（spawn 成功但子进程初始化失败）：漏了 [`set_default_dacl`]；
    /// - 控制台输出堆垃圾：漏了私有 Window Station / Desktop（模块文档「原理」5）。
    #[tokio::test]
    async fn sandboxed_echo_works() {
        let policy = read_only_policy();
        let child = spawn_sandboxed(&policy, "echo hello_sandbox", &policy.workspace_cwd)
            .expect("受限令牌 spawn 应成功");
        let output = child.wait_with_output().await.expect("等待输出失败");
        assert_eq!(
            output.exit_code,
            0,
            "echo 应退出码 0，stderr={:?}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("hello_sandbox"), "stdout 应包含回显: {stdout}");
    }

    /// ① 读路径只走第一遍（常规 SID）⇒ 与未沙箱进程等价，系统文件照常可读。
    #[tokio::test]
    async fn sandboxed_can_read_system_dir() {
        let policy = read_only_policy();
        let child = spawn_sandboxed(&policy, "type C:\\Windows\\win.ini", &policy.workspace_cwd)
            .expect("受限令牌 spawn 应成功");
        let output = child.wait_with_output().await.expect("等待输出失败");
        assert_eq!(
            output.exit_code,
            0,
            "type 应成功，stdout={:?} stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("[fonts]"),
            "应能读到 win.ini 内容"
        );
    }

    /// ① 不能写系统目录（写路径第二遍只认 allow ACE）。
    #[tokio::test]
    async fn sandboxed_cannot_write_system_dir() {
        let policy = read_only_policy();
        let cmd = "echo blocked > \"C:\\Windows\\axagent_sandbox_write_probe.txt\"";
        let child =
            spawn_sandboxed(&policy, cmd, &policy.workspace_cwd).expect("受限令牌 spawn 应成功");
        let output = child.wait_with_output().await.expect("等待输出失败");
        assert_ne!(
            output.exit_code,
            0,
            "写入系统目录必须失败（受限令牌 deny 生效），stdout={:?}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            !std::path::Path::new("C:\\Windows\\axagent_sandbox_write_probe.txt").exists(),
            "探测文件不应被创建"
        );
    }

    /// ① **令牌换代的核心收益**：用户自己的 Profile 也不能写。
    ///
    /// SAFER（Basic User）保留「标准用户对自己 Profile 的写权限」，这条边界在
    /// SAFER 框架内无法消除；capability SID 版下 Profile 里没有任何 capability
    /// allow ACE ⇒ 写一律被拒（模块文档「原理」2）。
    #[tokio::test]
    async fn sandboxed_cannot_write_user_profile() {
        let profile = std::env::var("USERPROFILE").expect("USERPROFILE 未设置");
        let target = format!("{profile}\\axagent_sandbox_profile_probe.txt");
        let policy = read_only_policy();
        let cmd = format!("echo blocked > \"{target}\"");
        let child = spawn_sandboxed(&policy, &cmd, &policy.workspace_cwd).expect("spawn 应成功");
        let output = child.wait_with_output().await.expect("等待输出失败");
        assert_ne!(
            output.exit_code,
            0,
            "写入用户 Profile 必须失败（capability SID 无 allow ACE），stdout={:?}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(!std::path::Path::new(&target).exists(), "Profile 探测文件不应被创建: {target}");
    }

    /// ② `WorkspaceWrite`：工作区被打了 capability allow ACE ⇒ 区内写成功，
    /// 且文件继承 ACE（`INHERIT_ALL`），子目录同样可写。
    #[tokio::test]
    async fn workspace_write_inside_allowed() {
        let ws = temp_workspace("write_inside");
        std::fs::create_dir_all(&ws).expect("建工作区失败");
        let policy = fs_policy(SandboxMode::WorkspaceWrite, ws.clone());
        let file = ws.join("probe.txt");
        let cmd = format!("echo allowed > \"{}\"", file.display());
        let child = spawn_sandboxed(&policy, &cmd, &ws).expect("spawn 应成功");
        let output = child.wait_with_output().await.expect("等待输出失败");
        assert_eq!(
            output.exit_code,
            0,
            "工作区内写应成功，stdout={:?} stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(file.exists(), "工作区内探测文件应被创建: {}", file.display());
        let _ = std::fs::remove_file(&file);

        // 子目录继承：验证 INHERIT_ALL（0x3）确实生效
        let sub = ws.join("nested");
        std::fs::create_dir_all(&sub).expect("建子目录失败");
        let sub_file = sub.join("probe.txt");
        let cmd = format!("echo nested > \"{}\"", sub_file.display());
        let child = spawn_sandboxed(&policy, &cmd, &ws).expect("spawn 应成功");
        let output = child.wait_with_output().await.expect("等待输出失败");
        assert_eq!(
            output.exit_code,
            0,
            "子目录写应成功（ACE 继承），stderr={:?}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(sub_file.exists(), "子目录探测文件应被创建");
        let _ = std::fs::remove_file(&sub_file);
        let _ = std::fs::remove_dir(&sub);
    }

    /// ② `WorkspaceWrite`：工作区**外**写仍被拒（allow ACE 只打在工作区）。
    #[tokio::test]
    async fn workspace_write_outside_denied() {
        let ws = temp_workspace("write_outside");
        std::fs::create_dir_all(&ws).expect("建工作区失败");
        let policy = fs_policy(SandboxMode::WorkspaceWrite, ws.clone());
        let cmd = "echo blocked > \"C:\\Windows\\axagent_sandbox_ws_outside_probe.txt\"";
        let child = spawn_sandboxed(&policy, cmd, &ws).expect("spawn 应成功");
        let output = child.wait_with_output().await.expect("等待输出失败");
        assert_ne!(
            output.exit_code,
            0,
            "工作区外写必须失败，stdout={:?}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            !std::path::Path::new("C:\\Windows\\axagent_sandbox_ws_outside_probe.txt").exists(),
            "工作区外探测文件不应被创建"
        );
    }

    /// ② 档位区分的核心：`ReadOnly` **不打** allow ACE ⇒ 连自己的工作区目录也不能写。
    ///
    /// 工作区目录必须是**本测试专属**的（`temp_workspace("readonly")`）：`ReadOnly` 与
    /// `WorkspaceWrite` 的 capability SID 已分档，但若复用 `WorkspaceWrite` 打过 ACE
    /// 的目录，断言就失去意义（`grant_workspace_write` 幂等不清理）。
    #[tokio::test]
    async fn read_only_cannot_write_workspace() {
        let ws = temp_workspace("readonly");
        std::fs::create_dir_all(&ws).expect("建工作区失败");
        let policy = fs_policy(SandboxMode::ReadOnly, ws.clone());
        let file = ws.join("probe_readonly.txt");
        let cmd = format!("echo blocked > \"{}\"", file.display());
        let child = spawn_sandboxed(&policy, &cmd, &ws).expect("spawn 应成功");
        let output = child.wait_with_output().await.expect("等待输出失败");
        assert_ne!(
            output.exit_code,
            0,
            "ReadOnly 档下工作区内写也必须失败（无 allow ACE），stdout={:?}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(!file.exists(), "ReadOnly 档不应写下任何文件");
    }

    /// ③ 网络封锁：**实际强度必须如实**，且 `Enforced` / `BestEffort` 两档都必须
    /// 真断网（否则「挂上了过滤器」只是自陈）。
    ///
    /// 本机回环监听器是唯一可判定的探针 —— 不依赖外网，离线/CI 环境同样可判。
    /// 自校验设计：先用同一条命令在**沙箱外**跑一遍作为对照组，对照组不成功
    /// （例如 curl 不可用）就跳过 —— 否则「沙箱内连不上」不构成封锁证据。
    ///
    /// 必须多线程 runtime：对照组是**阻塞**调用，单线程 runtime 上它会把
    /// 监听器任务一起堵死，导致对照组超时失败（实测踩过）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn network_block_status_is_truthful() {
        use tokio::io::AsyncWriteExt;

        let listener =
            tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("绑定回环端口失败");
        let port = listener.local_addr().expect("取本地地址失败").port();
        tokio::spawn(async move {
            loop {
                if let Ok((mut sock, _)) = listener.accept().await {
                    // 回最小 HTTP 响应，让 curl 以退出码 0 判定「连接成功」。
                    // curl 取完响应即断开，写 / 关闭失败是预期噪声：显式吞掉并
                    // 继续服务后续连接（H 棘轮不许新增 `let _ = <fallible>.await;`）。
                    if sock
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                        .await
                        .is_err()
                    {
                        continue;
                    }
                    if sock.shutdown().await.is_err() {
                        continue;
                    }
                }
            }
        });

        let cmd = format!("curl -s -m 5 -o NUL http://127.0.0.1:{port}/probe");

        // 对照组：沙箱外必须连得上（证明监听器与 curl 都可用）
        let direct = std::process::Command::new("cmd")
            .args(["/d", "/s", "/c", &cmd])
            .output()
            .expect("对照组启动失败");
        if !direct.status.success() {
            eprintln!(
                "跳过 network_block_status_is_truthful：对照组未成功（探针不可用），\
                 stderr={:?}",
                String::from_utf8_lossy(&direct.stderr)
            );
            return;
        }

        let status = ensure_network_block().expect("非权限类安装失败应 fail-closed（返 Err）");
        // 实验组：真实策略（`network_access = false`）下跑同一条探针命令
        let policy = SandboxPolicy::read_only("C:\\Windows");
        let child = spawn_sandboxed(&policy, &cmd, &policy.workspace_cwd)
            .expect("网络策略相关的 spawn 应成功");
        let output = child.wait_with_output().await.expect("等待输出失败");
        let sandboxed_ok = output.exit_code == 0;

        match status {
            NetworkBlock::Enforced | NetworkBlock::BestEffort => {
                assert!(
                    !sandboxed_ok,
                    "封锁已安装（{status:?}）却仍能连上回环监听器 ⇒ 过滤器被绕过或条件不匹配。\
                     stdout={:?} stderr={:?}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            },
            NetworkBlock::Unavailable { reason } => {
                // 如实记录当下约束（非提权 ⇒ 装不上 WFP 对象）：只允许权限类原因，
                // 且明确断言「网络确实没被断」—— 把限制写成断言，防止后人误以为已隔离。
                assert!(
                    reason.contains("ERROR_ACCESS_DENIED"),
                    "Unavailable 的原因必须是权限类（实测 ERROR_ACCESS_DENIED）: {reason}"
                );
                assert!(
                    sandboxed_ok,
                    "Unavailable 档下网络未被阻断（这是已知限制，故断言其确实放行）"
                );
            },
        }
    }

    /// 环境驱动探针（#[ignore]，仅实验诊断用）：AXAGENT_PROBE_CWD /
    /// AXAGENT_PROBE_CMD 控制 cwd 与命令。
    #[tokio::test]
    #[ignore]
    async fn sandbox_env_probe() {
        let cwd = std::env::var("AXAGENT_PROBE_CWD").unwrap_or_else(|_| "C:\\Windows".into());
        let cmd = std::env::var("AXAGENT_PROBE_CMD").unwrap_or_else(|_| "echo hi".into());
        let policy = read_only_policy();
        let child =
            spawn_sandboxed(&policy, &cmd, &std::path::PathBuf::from(&cwd)).expect("spawn 应成功");
        let output = child.wait_with_output().await.expect("等待输出失败");
        println!("cmd={cmd:?} cwd={cwd:?}");
        println!("exit={}", output.exit_code);
        println!("stdout={:?}", String::from_utf8_lossy(&output.stdout));
        println!("stderr={:?}", String::from_utf8_lossy(&output.stderr));
    }
}
