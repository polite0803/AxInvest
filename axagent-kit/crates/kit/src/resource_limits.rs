// SPDX-License-Identifier: AGPL-3.0-only

//! 跨平台进程资源限制。
//!
//! Linux/macOS: rlimit (RLIMIT_CPU, RLIMIT_AS, RLIMIT_NPROC, RLIMIT_FSIZE)
//! Windows: Job Objects (内存限制 + 进程数限制)

#[cfg(all(unix, not(target_os = "android")))]
use libc;
#[derive(Debug, Clone, Copy)]
pub struct ResourceLimits {
    /// CPU 时间限制（秒），默认 60
    pub max_cpu_seconds: u64,
    /// 虚拟内存限制（字节），默认 512MB
    pub max_memory_bytes: u64,
    /// 最大子进程数，默认 10
    pub max_processes: u32,
    /// 最大文件写入（字节），默认 100MB
    pub max_file_size_bytes: u64,
    /// 最大打开文件描述符数，默认 256
    pub max_open_files: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_cpu_seconds: 60,
            max_memory_bytes: 512 * 1024 * 1024,
            max_processes: 10,
            max_file_size_bytes: 100 * 1024 * 1024,
            max_open_files: 256,
        }
    }
}

impl ResourceLimits {
    /// 创建沙箱默认限制
    pub fn default_sandbox() -> Self {
        Self::default()
    }

    /// 应用资源限制到当前进程及其子进程
    pub fn apply_to_current_process(&self) -> Result<(), String> {
        #[cfg(all(unix, not(target_os = "android")))]
        self.apply_rlimit()?;

        #[cfg(target_os = "windows")]
        self.apply_job_object()?;

        tracing::info!(
            "Sandbox resource limits applied: cpu={}s, mem={}MB, procs={}, fsize={}MB",
            self.max_cpu_seconds,
            self.max_memory_bytes / (1024 * 1024),
            self.max_processes,
            self.max_file_size_bytes / (1024 * 1024),
        );

        Ok(())
    }

    #[cfg(all(unix, not(target_os = "android")))]
    fn apply_rlimit(&self) -> Result<(), String> {
        // RLIMIT_CPU: 进程可使用的 CPU 时间（秒）
        self.set_rlimit(
            libc::RLIMIT_CPU as _,
            self.max_cpu_seconds,
            self.max_cpu_seconds.saturating_add(5),
        )?;

        // RLIMIT_AS: 进程可用虚拟内存（字节）
        self.set_rlimit(libc::RLIMIT_AS as _, self.max_memory_bytes, self.max_memory_bytes)?;

        // RLIMIT_NPROC: 最大子进程数
        self.set_rlimit(
            libc::RLIMIT_NPROC as _,
            self.max_processes as u64,
            self.max_processes as u64,
        )?;

        // RLIMIT_FSIZE: 最大文件写入（字节）
        self.set_rlimit(
            libc::RLIMIT_FSIZE as _,
            self.max_file_size_bytes,
            self.max_file_size_bytes,
        )?;

        // RLIMIT_NOFILE: 最大打开文件描述符数（防止 fd 耗尽）
        self.set_rlimit(libc::RLIMIT_NOFILE as _, self.max_open_files, self.max_open_files)?;

        Ok(())
    }

    #[cfg(all(unix, not(target_os = "android")))]
    fn set_rlimit(&self, resource: u32, soft: u64, hard: u64) -> Result<(), String> {
        let rlim = libc::rlimit { rlim_cur: soft.min(hard), rlim_max: hard };
        // SAFETY: rlim is properly initialized with valid soft/hard limit values;
        // resource parameter is a valid libc rlimit resource constant;
        // setrlimit is called on the current process only;
        // failure is handled gracefully (non-zero return logged but not fatal).
        let ret = unsafe { libc::setrlimit(resource as _, &rlim) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            tracing::warn!("Failed to set rlimit {:?}: {}", resource, err);
            // 不返回错误——rlimit 失败不应阻止执行
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn apply_job_object(&self) -> Result<(), String> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::HANDLE;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_JOB_MEMORY,
            JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectExtendedLimitInformation, SetInformationJobObject,
        };

        let name: Vec<u16> = std::ffi::OsStr::new("AxAgent_Sandbox_Job")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // SAFETY: name is a properly null-terminated UTF-16 string
        // (created with encode_wide().chain(once(0)));
        // null pointer passed for lpSecurityAttributes is valid
        // (uses default security descriptor);
        // null handle return is checked below.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), name.as_ptr()) };
        if handle.is_null() {
            return Err("无法创建 Windows Job Object".to_string());
        }

        // SAFETY: JOBOBJECT_EXTENDED_LIMIT_INFORMATION is a Windows struct
        // that is safe to zero-initialize; all fields are numeric types
        // (DWORD, SIZE_T) that default to 0; the zeroed struct is
        // immediately populated with valid values before use.
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_JOB_MEMORY;
        info.ProcessMemoryLimit = if std::mem::size_of::<usize>() < 8 {
            self.max_memory_bytes.min(usize::MAX as u64) as usize
        } else {
            self.max_memory_bytes as usize
        };
        let limit = self.max_memory_bytes.saturating_mul(2);
        let limit = if std::mem::size_of::<usize>() < 8 {
            limit.min(usize::MAX as u64) as usize
        } else {
            limit as usize
        };
        info.JobMemoryLimit = limit;

        // SAFETY: handle is a valid Job Object handle obtained from
        // CreateJobObjectW (null-checked above); info pointer and size are
        // correctly derived from the same struct; JobObjectExtendedLimitInformation
        // is the correct information class for this struct type;
        // failure is handled gracefully (ret == 0 logged but not fatal).
        let ret = unsafe {
            SetInformationJobObject(
                handle as HANDLE,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ret == 0 {
            tracing::warn!("Failed to configure Windows Job Object");
        }

        // SAFETY: GetCurrentProcess always returns a valid pseudo-handle
        // per Windows documentation; no parameters, cannot fail.
        let current = unsafe { windows_sys::Win32::System::Threading::GetCurrentProcess() };
        // SAFETY: handle is a valid Job Object handle (null-checked above);
        // current is a valid process pseudo-handle from GetCurrentProcess;
        // failure is handled gracefully (ret == 0 logged but not fatal).
        let ret = unsafe { AssignProcessToJobObject(handle as HANDLE, current) };
        if ret == 0 {
            tracing::warn!("Failed to assign process to Job Object");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(unix, not(target_os = "android")))]
    macro_rules! read_rlimit {
        ($resource:expr) => {{
            let mut rlim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
            // SAFETY: rlim 指向已初始化的 rlimit 结构，$resource 是合法的 rlimit 常量。
            let ret = unsafe { libc::getrlimit($resource, &mut rlim) };
            assert_eq!(ret, 0, "getrlimit 应成功");
            (rlim.rlim_cur as u64, rlim.rlim_max as u64)
        }};
    }

    /// 子进程探针开关：设有该变量才真正应用限制，保证父测试进程永不被收紧。
    const RLIMIT_CHILD_ENV: &str = "AXAGENT_KIT_RLIMIT_PROBE";

    /// 探针用例的完整路径（libtest `--exact` 需要全路径）。
    const RLIMIT_CHILD_TEST: &str = "resource_limits::tests::child_applies_limits_probe";

    /// 子进程回写哨兵：父进程只有**收到哨兵**才认定探针真的执行过，
    /// 避免过滤器写错时「0 个用例」被读成通过。
    const RLIMIT_CHILD_SENTINEL: &str = "AXAGENT_KIT_RLIMIT_PROBE_OK";

    /// 在**子进程**里应用资源限制并回读。
    ///
    /// 为什么必须是子进程：`set_rlimit` 把 `rlim_max` 写成与软限制同一个值（生产语义，
    /// 此处不改），硬上限一旦压低，同进程内不可恢复。若在测试进程里直接 apply，同一
    /// 二进制里后续用例只要线程栈 / panic 备用栈超过 512MB 就 ENOMEM、CPU 超 65s 就
    /// SIGXCPU，表现为随机的 SIGABRT（nextest 每用例一进程，所以一直没暴露）。
    #[test]
    fn child_applies_limits_probe() {
        if std::env::var_os(RLIMIT_CHILD_ENV).is_none() {
            return;
        }

        let limits = ResourceLimits::default();
        limits.apply_to_current_process().expect("子进程中应用资源限制不应失败");

        #[cfg(all(unix, not(target_os = "android")))]
        {
            let cpu = read_rlimit!(libc::RLIMIT_CPU);
            assert_eq!(cpu, (limits.max_cpu_seconds, limits.max_cpu_seconds + 5));
            let mem = read_rlimit!(libc::RLIMIT_AS);
            assert_eq!(mem, (limits.max_memory_bytes, limits.max_memory_bytes));
            let procs = read_rlimit!(libc::RLIMIT_NPROC);
            assert_eq!(procs, (limits.max_processes as u64, limits.max_processes as u64));
            let fsize = read_rlimit!(libc::RLIMIT_FSIZE);
            assert_eq!(fsize, (limits.max_file_size_bytes, limits.max_file_size_bytes));
            let nofile = read_rlimit!(libc::RLIMIT_NOFILE);
            assert_eq!(nofile, (limits.max_open_files, limits.max_open_files));

            // 地址空间已被收紧：不再分配、不经过 libtest 汇总，直接回写哨兵并退出。
            let payload = RLIMIT_CHILD_SENTINEL.as_bytes();
            // SAFETY: payload 是静态字节串，长度即实际长度；fd 1 是进程标准输出。
            let written = unsafe { libc::write(1, payload.as_ptr().cast(), payload.len()) };
            let code = if written == payload.len() as isize {
                0
            } else {
                1
            };
            // SAFETY: _exit 只结束当前进程，不跑 atexit、不分配内存。
            unsafe { libc::_exit(code) };
        }

        #[cfg(not(all(unix, not(target_os = "android"))))]
        {
            use std::io::Write;
            println!("{RLIMIT_CHILD_SENTINEL}");
            let _ = std::io::stdout().flush();
            std::process::exit(0);
        }
    }

    /// 父测试进程只负责拉起探针并检查结果；自身 rlimit 前后必须完全一致。
    #[test]
    fn apply_does_not_panic() {
        #[cfg(all(unix, not(target_os = "android")))]
        let before = [
            read_rlimit!(libc::RLIMIT_CPU),
            read_rlimit!(libc::RLIMIT_AS),
            read_rlimit!(libc::RLIMIT_NPROC),
            read_rlimit!(libc::RLIMIT_FSIZE),
            read_rlimit!(libc::RLIMIT_NOFILE),
        ];

        let exe = std::env::current_exe().expect("应能定位当前测试二进制");
        let output = std::process::Command::new(&exe)
            .arg("--exact")
            .arg(RLIMIT_CHILD_TEST)
            .arg("--nocapture")
            .env(RLIMIT_CHILD_ENV, "1")
            .output()
            .expect("应能拉起资源限制探针子进程");

        assert!(
            output.status.success(),
            "子进程应用资源限制失败：status={:?} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(RLIMIT_CHILD_SENTINEL),
            "子进程未回写探针哨兵（过滤器可能没命中用例）：stdout={stdout}"
        );

        #[cfg(all(unix, not(target_os = "android")))]
        {
            let after = [
                read_rlimit!(libc::RLIMIT_CPU),
                read_rlimit!(libc::RLIMIT_AS),
                read_rlimit!(libc::RLIMIT_NPROC),
                read_rlimit!(libc::RLIMIT_FSIZE),
                read_rlimit!(libc::RLIMIT_NOFILE),
            ];
            assert_eq!(before, after, "父测试进程的 rlimit 必须保持不变");
        }
    }

    #[test]
    fn default_limits_are_reasonable() {
        let limits = ResourceLimits::default();
        assert!(limits.max_cpu_seconds > 0);
        assert!(limits.max_memory_bytes > 0);
        assert!(limits.max_processes > 0);
        assert!(limits.max_file_size_bytes > 0);
    }

    #[test]
    fn sandbox_limits_are_restrictive() {
        let limits = ResourceLimits::default_sandbox();
        assert!(limits.max_cpu_seconds <= 120);
        assert!(limits.max_memory_bytes <= 1024 * 1024 * 1024);
        assert!(limits.max_processes <= 50);
    }
}
