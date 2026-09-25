// SPDX-License-Identifier: AGPL-3.0-only

//! 插件源码隔离构建（`PLAN-everything-is-plugin.md` §12.3）与编译前置探测（§14.3）。
//!
//! 五阶段管线（§12.1）的第 ③ 步：源码 → 可执行产物。本模块负责三件事：
//!
//! 1. **前置探测四项（§14.3）**：`rustc -vV` / `cargo -V` / `rustup target list --installed`
//!    / 磁盘与内存余量。前三项的取值全部进入产物 hash（§13.1）。
//! 2. **内容寻址（§13.1）**：
//!    `artifact_hash = H(source ‖ proto_version ‖ target_triple ‖ rustc_version ‖ build_flags)`，
//!    **hash 即缓存 key —— 命中即跳过编译**（编译开销的唯一解法）。
//! 3. **隔离构建（§12.3）**：`cargo build --offline` + 独立 `--target-dir`（不污染主仓库
//!    `target/`）+ 显式 `--target`（产物路径确定，不随宿主默认目标漂移）。
//!
//! 产物目录布局（§13.2 三层里的「产物」层与「源码」层）：
//!
//! ```text
//! <build_root>/<plugin_id>/<hash>/
//!   plugin.json          声明 JSON —— 目录根即可被 plugin_install 识别（§12.4）
//!   bin/<bin><.exe>      编译产物；manifest 的 worker.program 会被改写指向它
//!   source/…             原始源码（§13.2：必须留存 —— hash 输入含源码需可复现，
//!                        且审计规则升级后要能重审历史插件）
//! ```
//!
//! ⚠ **诚实边界（与 §8.2 / §12.2 同一结论）**：本模块**不是安全边界**。
//! `--offline` 与 `CARGO_NET_OFFLINE=true` 只阻止 cargo **拉取依赖**，不构成 OS 级网络隔离 ——
//! 恶意 `build.rs` 仍可在编译期直接外联。源码审计对 `build.rs` 一票否决，但审计本身可被绕过
//! （§12.2 已说明）。真正的隔离需要在容器 / 网络命名空间 / 受限用户下执行，本仓尚未提供。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::core::MANIFEST_FILE_NAME;
use crate::manager::sanitize_plugin_id;
use crate::source_audit::SourceFile;

/// 依赖的帧协议版本 —— §13.1 四要素之一（协议换版本 ⇒ 旧产物必须重建）。
const PROTO_VERSION: u32 = axagent_plugin_proto::AXAGENT_PLUGIN_PROTO_VERSION;

/// 产物在输出目录内的子目录名。
const BINARY_SUBDIR: &str = "bin";

/// 源码留存子目录名（§13.2）。
const SOURCE_SUBDIR: &str = "source";

/// 独立 target-dir 的目录名前缀（与输出目录同级，构建完即删）。
const TARGET_DIR_PREFIX: &str = ".target-";

/// 磁盘余量下限（§14.3 第 ④ 项）：2 GiB。
///
/// 取值理由：单次 `cargo build` 的中间产物量级在数百 MB，2 GiB 是「留得下 + 不会因为
/// 一次编译把用户盘写满」的折中。它不是精确预测，是「早失败、错得清楚」的预检。
const MIN_FREE_DISK_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// 可用内存下限（§14.3 第 ④ 项）：512 MiB。
///
/// rustc 在受限内存下会 OOM kill 或长时间抖动；低于此值时直接拒绝比中途崩更可诊断。
const MIN_AVAILABLE_MEMORY_BYTES: u64 = 512 * 1024 * 1024;

/// 隔离构建失败的原因。
#[derive(Debug, thiserror::Error)]
pub enum SourceBuildError {
    /// 插件 ID 含路径成分，可能逃逸内容寻址根目录。
    #[error("插件 ID `{plugin_id}` 不合法（不得为空、不得含路径分隔符或 `..`）")]
    InvalidPluginId { plugin_id: String },
    /// 产物名含路径成分。
    #[error("产物名 `{binary_name}` 不合法（不得为空、不得含路径分隔符）")]
    InvalidBinaryName { binary_name: String },
    /// 工具链不可用（命令起不来 / 退出非零 / 输出无法解析）。
    #[error("编译工具链探测失败（{tool}）：{detail}")]
    ToolchainUnavailable { tool: String, detail: String },
    /// 目标平台未安装。
    #[error("目标平台 `{target}` 未安装（可执行 `rustup target add {target}` 后重试）")]
    TargetNotInstalled { target: String },
    /// 磁盘 / 内存余量不足。
    #[error("构建资源不足（{kind}：需要 ≥ {required} 字节，当前 {actual} 字节）")]
    ResourceLow { kind: String, required: u64, actual: u64 },
    /// 源码落盘失败（含路径逃逸拒绝）。
    #[error("写入源码失败：{0}")]
    SourceWrite(String),
    /// `cargo build` 退出非零。
    #[error("隔离构建失败（退出码 {code}）：{detail}")]
    BuildFailed { code: i32, detail: String },
    /// 构建成功但按约定路径找不到产物。
    #[error("编译产物缺失：{path}")]
    ArtifactMissing { path: String },
    /// 产物 / 声明落盘失败。
    #[error("落盘产物失败：{0}")]
    OutputWrite(String),
}

/// 工具链探测结果（§14.3 第 ①②③ 项）。取值全部进入产物 hash（§13.1）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainProbe {
    /// `rustc -vV` 首行的版本号（如 `1.97.0`）。
    pub rustc_version: String,
    /// `rustc -vV` 的 `release:` 行 —— §13.1 的 `rustc_version` 输入取这个值。
    pub rustc_release: String,
    /// `rustc -vV` 的 `host:` 行（未显式指定目标时的缺省 triple）。
    pub host_triple: String,
    /// `cargo -V` 的版本号。
    pub cargo_version: String,
    /// `rustup target list --installed` 的输出；**rustup 不可用时为空表**。
    ///
    /// 空表的语义是「无法核对」，不是「一个都没装」—— 见 `resolve_target_triple`。
    pub installed_targets: Vec<String>,
}

/// 资源探测结果（§14.3 第 ④ 项）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceProbe {
    /// 目标磁盘可用空间（字节）；`0` = 探测不到（不拦截）。
    pub free_disk_bytes: u64,
    /// 物理内存总量（字节）。
    pub total_memory_bytes: u64,
    /// 当前可用内存（字节）；`0` = 探测不到（不拦截）。
    pub available_memory_bytes: u64,
}

/// 一次隔离构建的请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceBuildRequest {
    /// 插件 ID；决定内容寻址目录 `<build_root>/<plugin_id>/<hash>/`。
    pub plugin_id: String,
    /// crate 名 / 产物名（对应 `Cargo.toml` 的 `[package] name`）。
    pub binary_name: String,
    /// 待编译源码（**须含 `Cargo.toml`**）；`path` 为 crate 内相对路径。
    pub files: Vec<SourceFile>,
    /// 插件声明 JSON，原样写为输出目录根的 `plugin.json`。
    pub manifest: serde_json::Value,
    /// 目标 triple；缺省 = 探测到的 host triple。
    #[serde(default)]
    pub target_triple: Option<String>,
    /// 追加给 `cargo build` 的 flag（顺序保留，并进入 hash）。
    #[serde(default)]
    pub build_flags: Vec<String>,
}

/// 一次隔离构建的结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceBuildOutcome {
    /// 内容寻址摘要（§13.1）。
    pub artifact_hash: String,
    /// `true` = hash 命中缓存，**未执行编译**。
    pub cache_hit: bool,
    /// 产物目录；直接作为 `plugin_install` 的来源路径即可（§12.4）。
    pub output_dir: String,
    /// 产物相对 `output_dir` 的路径（`bin/<name><.exe>`，POSIX 分隔符）。
    pub binary_relative_path: String,
    /// 本次探测到的工具链（已进入 hash）。
    pub toolchain: ToolchainProbe,
    /// 本次探测到的资源余量。
    pub resources: ResourceProbe,
}

/// 执行隔离构建（§12.3）：探测 → 内容寻址 → 命中即跳过 → 编译 → 落盘产物与源码。
///
/// `build_root` 由调用方给定（宿主侧为 `<appdata>/plugins`，§13.2）。
pub fn build_plugin_source(
    build_root: &Path,
    request: &SourceBuildRequest,
) -> Result<SourceBuildOutcome, SourceBuildError> {
    let segment = plugin_dir_segment(&request.plugin_id)?;
    let binary_name = validate_binary_name(&request.binary_name)?;

    let toolchain = probe_toolchain()?;
    let target = resolve_target_triple(request.target_triple.as_deref(), &toolchain)?;
    let resources = probe_resources(build_root);
    check_resources(&resources)?;

    let hash = artifact_hash(
        &request.files,
        PROTO_VERSION,
        &target,
        &toolchain.rustc_release,
        &request.build_flags,
    );
    let output_dir = build_root.join(&segment).join(&hash);
    let binary_file = binary_file_name(&binary_name);
    let binary_relative = format!("{BINARY_SUBDIR}/{binary_file}");
    let binary_path = output_dir.join(BINARY_SUBDIR).join(&binary_file);

    // §13.1「hash 即缓存 key —— 命中即跳过编译」：产物在即命中。
    let cache_hit = binary_path.is_file();
    if !cache_hit {
        compile(
            &output_dir,
            &target,
            &request.files,
            &request.build_flags,
            &binary_file,
            &binary_path,
        )?;
    }

    // 命中路径也重写一次源码与声明：成本可忽略，却能容忍「产物在、声明被手工删过」的半个目录。
    write_source_snapshot(&output_dir.join(SOURCE_SUBDIR), &request.files)?;
    write_manifest(&output_dir, &request.manifest, &binary_relative)?;

    Ok(SourceBuildOutcome {
        artifact_hash: hash,
        cache_hit,
        output_dir: output_dir.display().to_string(),
        binary_relative_path: binary_relative,
        toolchain,
        resources,
    })
}

/// 前置探测 ①–③（§14.3）。
fn probe_toolchain() -> Result<ToolchainProbe, SourceBuildError> {
    let rustc = run_probe("rustc", &["-vV"])?;
    let (rustc_version, rustc_release, host_triple) =
        parse_rustc_verbose(&rustc).ok_or_else(|| SourceBuildError::ToolchainUnavailable {
            tool: "rustc -vV".to_string(),
            detail: stderr_tail(&rustc),
        })?;

    let cargo = run_probe("cargo", &["-V"])?;
    let cargo_version = cargo.split_whitespace().nth(1).map(str::to_string).ok_or_else(|| {
        SourceBuildError::ToolchainUnavailable {
            tool: "cargo -V".to_string(),
            detail: stderr_tail(&cargo),
        }
    })?;

    Ok(ToolchainProbe {
        rustc_version,
        rustc_release,
        host_triple,
        cargo_version,
        installed_targets: installed_targets(),
    })
}

/// 前置探测 ④：磁盘 / 内存余量（§14.3）。
fn probe_resources(path: &Path) -> ResourceProbe {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    ResourceProbe {
        free_disk_bytes: free_disk_bytes(path),
        total_memory_bytes: system.total_memory(),
        available_memory_bytes: system.available_memory(),
    }
}

/// 内容寻址（§13.1）：`H(source ‖ proto_version ‖ target_triple ‖ rustc_version ‖ build_flags)`。
///
/// 两条确定性要求（否则缓存永不命中或错误命中）：
///
/// 1. **字段全部带长度前缀**再入摘要 —— 否则 `("ab","c")` 与 `("a","bc")` 撞同一个 hash；
/// 2. **源码按 `path` 排序**后拼接 —— 同一份源码以不同收集顺序送入必须得到同一个 hash。
fn artifact_hash(
    files: &[SourceFile],
    proto_version: u32,
    target_triple: &str,
    rustc_version: &str,
    build_flags: &[String],
) -> String {
    let mut sorted: Vec<&SourceFile> = files.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));

    let mut hasher = Sha256::new();
    hasher.update(b"axagent-plugin-artifact-v1");
    for file in sorted {
        absorb(&mut hasher, file.path.as_bytes());
        absorb(&mut hasher, file.content.as_bytes());
    }
    hasher.update(proto_version.to_le_bytes());
    absorb(&mut hasher, target_triple.as_bytes());
    absorb(&mut hasher, rustc_version.as_bytes());
    for flag in build_flags {
        absorb(&mut hasher, flag.as_bytes());
    }
    hex::encode(hasher.finalize())
}

/// 长度前缀吸收：`u64` 小端长度 + 原始字节。
fn absorb(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

/// 跑一次真实编译，把产物复制到 `binary_path`。
fn compile(
    output_dir: &Path,
    target: &str,
    files: &[SourceFile],
    build_flags: &[String],
    binary_file: &str,
    binary_path: &Path,
) -> Result<(), SourceBuildError> {
    let source_dir = output_dir.join(SOURCE_SUBDIR);
    write_source_snapshot(&source_dir, files)?;

    // 独立 target-dir：与输出目录同级，构建后即删（hash 变了才会重编，留着只涨磁盘）。
    let target_name = output_dir.file_name().map(|name| name.to_string_lossy().to_string());
    let target_dir = output_dir.with_file_name(format!(
        "{TARGET_DIR_PREFIX}{}",
        target_name.as_deref().unwrap_or("build")
    ));

    let output = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--offline")
        .arg("--manifest-path")
        .arg(source_dir.join("Cargo.toml"))
        .arg("--target")
        .arg(target)
        .arg("--target-dir")
        .arg(&target_dir)
        .args(build_flags)
        // §12.3「离线」：阻止 cargo 拉取依赖。⚠ 这不是 OS 级网络隔离（见模块头）。
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TERM_COLOR", "never")
        .output()
        .map_err(|e| SourceBuildError::ToolchainUnavailable {
            tool: "cargo build".to_string(),
            detail: e.to_string(),
        })?;

    if !output.status.success() {
        let _ = std::fs::remove_dir_all(&target_dir);
        return Err(SourceBuildError::BuildFailed {
            code: output.status.code().unwrap_or(-1),
            detail: stderr_tail(&String::from_utf8_lossy(&output.stderr)),
        });
    }

    let produced = target_dir.join(target).join("release").join(binary_file);
    if !produced.is_file() {
        let _ = std::fs::remove_dir_all(&target_dir);
        return Err(SourceBuildError::ArtifactMissing { path: produced.display().to_string() });
    }
    if let Some(parent) = binary_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| SourceBuildError::OutputWrite(e.to_string()))?;
    }
    let copied = std::fs::copy(&produced, binary_path)
        .map_err(|e| SourceBuildError::OutputWrite(e.to_string()));
    // 临时 target-dir 无论复制成败都清掉：失败时磁盘上只多一份中间产物，不影响用户可见结果。
    let _ = std::fs::remove_dir_all(&target_dir);
    copied?;
    Ok(())
}

/// 把源码写到 `dir` 下（按 `files` 的相对路径建子目录）。
///
/// 路径校验：拒绝绝对路径与 `..` —— 源码由前端可视化 / AI 生成，必须挡住「写穿到目录之外」。
fn write_source_snapshot(dir: &Path, files: &[SourceFile]) -> Result<(), SourceBuildError> {
    for file in files {
        let relative = Path::new(&file.path);
        if relative.is_absolute()
            || relative.components().any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(SourceBuildError::SourceWrite(format!(
                "源码路径 `{}` 必须是 crate 内相对路径（不得为绝对路径、不得含 `..`）",
                file.path
            )));
        }
        let path = dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| SourceBuildError::SourceWrite(e.to_string()))?;
        }
        std::fs::write(&path, &file.content)
            .map_err(|e| SourceBuildError::SourceWrite(e.to_string()))?;
    }
    Ok(())
}

/// 写插件声明到输出目录根，并把 `worker.program` 改写成实际产物路径。
///
/// 为什么要改写：产物名在 Windows 带 `.exe` 后缀，而生成端（可视化 / AI）不该关心平台细节。
/// 由构建端——唯一知道实际产物文件名的地方——回填，可让「生成 → 安装」这条链路不因为后缀
/// 差异断在 `resolve_worker_program` 的 `is_file()` 检查上。
/// **未声明 `worker` 时不新增该字段**：声明式插件不该因为经过构建就凭空获得一个 worker。
fn write_manifest(
    dir: &Path,
    manifest: &serde_json::Value,
    binary_relative: &str,
) -> Result<(), SourceBuildError> {
    let mut manifest = manifest.clone();
    let root = manifest
        .as_object_mut()
        .ok_or_else(|| SourceBuildError::OutputWrite("插件声明必须是 JSON 对象".to_string()))?;
    if let Some(worker) = root.get_mut("worker").and_then(serde_json::Value::as_object_mut) {
        worker
            .insert("program".to_string(), serde_json::Value::String(binary_relative.to_string()));
    }
    let body = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| SourceBuildError::OutputWrite(e.to_string()))?;
    std::fs::create_dir_all(dir).map_err(|e| SourceBuildError::OutputWrite(e.to_string()))?;
    std::fs::write(dir.join(MANIFEST_FILE_NAME), body)
        .map_err(|e| SourceBuildError::OutputWrite(e.to_string()))
}

/// 插件 ID → 目录名：先按「不得逃逸」校验，再复用 `manager` 的既有清洗（`/ \ @ :` → `-`）。
fn plugin_dir_segment(plugin_id: &str) -> Result<String, SourceBuildError> {
    let trimmed = plugin_id.trim();
    let invalid = trimmed.is_empty()
        || trimmed == "."
        || trimmed.contains("..")
        || trimmed.contains(['/', '\\']);
    if invalid {
        return Err(SourceBuildError::InvalidPluginId { plugin_id: plugin_id.to_string() });
    }
    Ok(sanitize_plugin_id(trimmed))
}

/// 产物名校验：cargo 按名在 target-dir 下找产物，名字含路径成分即失控。
fn validate_binary_name(name: &str) -> Result<String, SourceBuildError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.contains(['/', '\\']) || trimmed.contains("..") {
        return Err(SourceBuildError::InvalidBinaryName { binary_name: name.to_string() });
    }
    Ok(trimmed.to_string())
}

/// 平台产物文件名（Windows 带 `.exe`）。
fn binary_file_name(binary_name: &str) -> String {
    format!("{binary_name}{}", std::env::consts::EXE_SUFFIX)
}

/// ③ 的目标核对：显式给了 target 就必须已安装（空表 = 无法核对 ⇒ 跳过，见 §14.3 的务实取舍）。
fn resolve_target_triple(
    requested: Option<&str>,
    toolchain: &ToolchainProbe,
) -> Result<String, SourceBuildError> {
    let target = match requested.map(str::trim).filter(|target| !target.is_empty()) {
        Some(target) => target.to_string(),
        None => toolchain.host_triple.clone(),
    };
    if !toolchain.installed_targets.is_empty()
        && !toolchain.installed_targets.iter().any(|installed| installed == &target)
    {
        return Err(SourceBuildError::TargetNotInstalled { target });
    }
    Ok(target)
}

/// 资源余量判据。
fn check_resources(probe: &ResourceProbe) -> Result<(), SourceBuildError> {
    if is_below_floor(probe.free_disk_bytes, MIN_FREE_DISK_BYTES) {
        return Err(SourceBuildError::ResourceLow {
            kind: "free_disk".to_string(),
            required: MIN_FREE_DISK_BYTES,
            actual: probe.free_disk_bytes,
        });
    }
    if is_below_floor(probe.available_memory_bytes, MIN_AVAILABLE_MEMORY_BYTES) {
        return Err(SourceBuildError::ResourceLow {
            kind: "available_memory".to_string(),
            required: MIN_AVAILABLE_MEMORY_BYTES,
            actual: probe.available_memory_bytes,
        });
    }
    Ok(())
}

/// 余量判据：`0` = 探测不到 ⇒ 放行。
///
/// 宁可让编译自己去失败，也不要因为「探测不出余量」把构建一刀切掉
/// （sysinfo 在部分平台 / 受限环境下会返回 0）。
fn is_below_floor(actual: u64, floor: u64) -> bool {
    actual > 0 && actual < floor
}

/// ③ `rustup target list --installed`。
///
/// rustup 缺失或输出异常时返回**空表**而不报错：rustup 只是安装方式之一（cargo 也可能来自
/// 系统包管理器），此时「无法核对」不能升级为「构建被拒」。
fn installed_targets() -> Vec<String> {
    match Command::new("rustup").args(["target", "list", "--installed"]).output() {
        Ok(output) if output.status.success() => {
            parse_installed_targets(&String::from_utf8_lossy(&output.stdout))
        },
        _ => Vec::new(),
    }
}

/// 跑一个探测命令并返回 stdout。
fn run_probe(program: &str, args: &[&str]) -> Result<String, SourceBuildError> {
    let tool = format!("{program} {}", args.join(" "));
    let output = Command::new(program).args(args).output().map_err(|e| {
        SourceBuildError::ToolchainUnavailable { tool: tool.clone(), detail: e.to_string() }
    })?;
    if !output.status.success() {
        return Err(SourceBuildError::ToolchainUnavailable {
            tool,
            detail: stderr_tail(&String::from_utf8_lossy(&output.stderr)),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// 解析 `rustc -vV` 输出 → `(版本号, release, host triple)`，形如：
///
/// ```text
/// rustc 1.97.0 (69f9c33d7 2026-08-01)
/// binary: rustc
/// host: x86_64-pc-windows-msvc
/// release: 1.97.0
/// ```
///
/// 三项缺一即返回 `None` —— 它们都是 hash 输入，缺项会让「同源码、不同环境」不可区分。
fn parse_rustc_verbose(output: &str) -> Option<(String, String, String)> {
    let mut version = None;
    let mut release = None;
    let mut host = None;
    for line in output.lines() {
        let line = line.trim();
        if version.is_none() && line.starts_with("rustc ") {
            version = line.split_whitespace().nth(1).map(str::to_string);
        } else if let Some(rest) = line.strip_prefix("release:") {
            release = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("host:") {
            host = Some(rest.trim().to_string());
        }
    }
    match (version, release, host) {
        (Some(version), Some(release), Some(host)) => {
            if version.is_empty() || release.is_empty() || host.is_empty() {
                return None;
            }
            Some((version, release, host))
        },
        _ => None,
    }
}

/// 解析 `rustup target list` 的输出：每行一个 triple。
///
/// 兼容未加 `--installed` 的形态（`x86_64-… (installed)`）—— 两种输出都能吃。
/// 去重并保持顺序：内容寻址只关心「是否包含目标」，去重让结果稳定可断言。
fn parse_installed_targets(output: &str) -> Vec<String> {
    let mut targets: Vec<String> = Vec::new();
    for line in output.lines() {
        let triple = line.trim().trim_end_matches("(installed)").trim();
        if triple.is_empty() || targets.iter().any(|existing| existing == triple) {
            continue;
        }
        targets.push(triple.to_string());
    }
    targets
}

/// 取 `path` 所在磁盘的可用空间；探测不到返回 `0`（= 未知，不拦截）。
///
/// 按**挂载点最长前缀**匹配（`/` 与 `/home` 同时命中时选更贴切的 `/home`），
/// 目标路径尚不存在时逐级上溯到最近的已存在祖先（首次构建时输出目录还不存在）。
fn free_disk_bytes(path: &Path) -> u64 {
    let probe_path = existing_ancestor(path);
    let disks = sysinfo::Disks::new_with_refreshed_list();

    let mut best: Option<(usize, u64)> = None;
    for disk in disks.list() {
        let mount = disk.mount_point();
        if !probe_path.starts_with(mount) {
            continue;
        }
        let depth = mount.as_os_str().len();
        if best.is_none_or(|(current, _)| depth > current) {
            best = Some((depth, disk.available_space()));
        }
    }
    match best {
        Some((_, space)) => space,
        // 挂载点匹配不上（容器 / 虚拟文件系统）时退化为「所有磁盘里最大的可用空间」。
        None => disks.list().iter().map(|disk| disk.available_space()).max().unwrap_or(0),
    }
}

/// 逐级上溯到最近存在的祖先（磁盘探测需要真实路径）。
fn existing_ancestor(path: &Path) -> PathBuf {
    let mut current = path;
    loop {
        if current.exists() {
            return current.to_path_buf();
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return path.to_path_buf(),
        }
    }
}

/// 取 stderr 的**尾部**摘要。
///
/// 为什么取尾部：cargo 的结论（`error: …`）在最后几行，前缀多是编译进度与 warning。
/// 按字符截断，不在多字节字符中间切开。
fn stderr_tail(stderr: &str) -> String {
    const MAX_CHARS: usize = 2000;

    let total = stderr.chars().count();
    if total <= MAX_CHARS {
        return stderr.to_string();
    }
    let mut tail: String = stderr.chars().skip(total - MAX_CHARS).collect();
    tail.insert(0, '…');
    tail
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET: &str = "x86_64-unknown-linux-gnu";
    const RUSTC: &str = "1.97.0";

    fn file(path: &str, content: &str) -> SourceFile {
        SourceFile { path: path.to_string(), content: content.to_string() }
    }

    fn sample_files() -> Vec<SourceFile> {
        vec![
            file("Cargo.toml", "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n"),
            file("src/main.rs", "fn main() {}\n"),
        ]
    }

    fn hash_of(files: &[SourceFile], flags: &[String]) -> String {
        artifact_hash(files, PROTO_VERSION, TARGET, RUSTC, flags)
    }

    #[test]
    fn hash_is_stable_regardless_of_input_order() {
        let mut reversed = sample_files();
        reversed.reverse();

        assert_eq!(
            hash_of(&sample_files(), &[]),
            hash_of(&reversed, &[]),
            "同一份源码以不同顺序送入必须得到同一个 hash（否则缓存永不命中）"
        );
    }

    #[test]
    fn hash_changes_with_each_of_the_four_factors() {
        let files = sample_files();
        let no_flags: Vec<String> = Vec::new();
        let base = artifact_hash(&files, PROTO_VERSION, TARGET, RUSTC, &no_flags);

        let mut edited = sample_files();
        edited[1].content = "fn main() { let _ = 1; }\n".to_string();
        assert_ne!(base, hash_of(&edited, &[]), "改源码必须换 hash");

        assert_ne!(
            base,
            artifact_hash(&files, PROTO_VERSION + 1, TARGET, RUSTC, &no_flags),
            "换协议版本必须换 hash"
        );
        assert_ne!(
            base,
            artifact_hash(&files, PROTO_VERSION, "aarch64-apple-darwin", RUSTC, &no_flags),
            "换目标平台必须换 hash"
        );
        assert_ne!(
            base,
            artifact_hash(&files, PROTO_VERSION, TARGET, "1.98.0", &no_flags),
            "换 rustc 版本必须换 hash"
        );
        assert_ne!(
            base,
            hash_of(&files, &["--features".to_string(), "x".to_string()]),
            "换构建 flag 必须换 hash"
        );
    }

    #[test]
    fn hash_disambiguates_field_boundaries() {
        // 没有长度前缀时，("ab","c") 与 ("a","bc") 会撞同一个 hash。
        let left = [file("ab", "c")];
        let right = [file("a", "bc")];

        assert_ne!(hash_of(&left, &[]), hash_of(&right, &[]));
    }

    #[test]
    fn parses_rustc_verbose_output() {
        let output = "rustc 1.97.0 (69f9c33d7 2026-08-01)\nbinary: rustc\ncommit-hash: 69f9c33d7\ncommit-date: 2026-08-01\nhost: x86_64-pc-windows-msvc\nrelease: 1.97.0\nLLVM version: 21.1.0\n";

        assert_eq!(
            parse_rustc_verbose(output),
            Some((
                "1.97.0".to_string(),
                "1.97.0".to_string(),
                "x86_64-pc-windows-msvc".to_string()
            ))
        );
    }

    #[test]
    fn rejects_rustc_verbose_output_missing_hash_inputs() {
        // 缺 `host:` ⇒ 不能当作成功探测（host triple 是 hash 输入之一）。
        let output = "rustc 1.97.0 (69f9c33d7 2026-08-01)\nrelease: 1.97.0\n";

        assert_eq!(parse_rustc_verbose(output), None);
    }

    #[test]
    fn parses_installed_targets_with_and_without_suffix() {
        let with_suffix = "x86_64-pc-windows-msvc (installed)\naarch64-apple-darwin\nwasm32-unknown-unknown (installed)\n";
        assert_eq!(
            parse_installed_targets(with_suffix),
            vec!["x86_64-pc-windows-msvc", "aarch64-apple-darwin", "wasm32-unknown-unknown"]
        );

        let plain = "x86_64-pc-windows-msvc\n\nx86_64-pc-windows-msvc\n";
        assert_eq!(parse_installed_targets(plain), vec!["x86_64-pc-windows-msvc"]);
    }

    fn toolchain(installed: &[&str]) -> ToolchainProbe {
        ToolchainProbe {
            rustc_version: RUSTC.to_string(),
            rustc_release: RUSTC.to_string(),
            host_triple: "x86_64-pc-windows-msvc".to_string(),
            cargo_version: RUSTC.to_string(),
            installed_targets: installed.iter().map(|target| (*target).to_string()).collect(),
        }
    }

    #[test]
    fn target_resolution_falls_back_to_host_and_checks_installed_list() {
        let probe = toolchain(&["x86_64-pc-windows-msvc"]);

        assert_eq!(
            resolve_target_triple(None, &probe).expect("host triple should be accepted"),
            "x86_64-pc-windows-msvc"
        );
        assert_eq!(
            resolve_target_triple(Some("  "), &probe).expect("blank should fall back to host"),
            "x86_64-pc-windows-msvc"
        );
        assert!(matches!(
            resolve_target_triple(Some("aarch64-apple-darwin"), &probe),
            Err(SourceBuildError::TargetNotInstalled { target }) if target == "aarch64-apple-darwin"
        ));
    }

    #[test]
    fn target_resolution_skips_check_when_rustup_is_unavailable() {
        // 空表 = 无法核对（rustup 缺失），不得升级为「构建被拒」。
        let probe = toolchain(&[]);

        assert_eq!(
            resolve_target_triple(Some("aarch64-apple-darwin"), &probe)
                .expect("unknown probe must not reject"),
            "aarch64-apple-darwin"
        );
    }

    #[test]
    fn resource_check_passes_unknown_and_rejects_low() {
        let unknown =
            ResourceProbe { free_disk_bytes: 0, total_memory_bytes: 0, available_memory_bytes: 0 };
        assert!(check_resources(&unknown).is_ok(), "探测不到余量时不得拦截");

        let enough = ResourceProbe {
            free_disk_bytes: MIN_FREE_DISK_BYTES,
            total_memory_bytes: 8 * 1024 * 1024 * 1024,
            available_memory_bytes: MIN_AVAILABLE_MEMORY_BYTES,
        };
        assert!(check_resources(&enough).is_ok(), "恰好到达下限应放行");

        let low_disk = ResourceProbe { free_disk_bytes: MIN_FREE_DISK_BYTES - 1, ..enough };
        assert!(matches!(
            check_resources(&low_disk),
            Err(SourceBuildError::ResourceLow { kind, .. }) if kind == "free_disk"
        ));

        let low_memory =
            ResourceProbe { available_memory_bytes: MIN_AVAILABLE_MEMORY_BYTES - 1, ..enough };
        assert!(matches!(
            check_resources(&low_memory),
            Err(SourceBuildError::ResourceLow { kind, .. }) if kind == "available_memory"
        ));
    }

    #[test]
    fn rejects_path_escaping_plugin_id_and_binary_name() {
        for bad in ["", "  ", "..", "a/../../b", "a\\b", "."] {
            assert!(
                matches!(plugin_dir_segment(bad), Err(SourceBuildError::InvalidPluginId { .. })),
                "`{bad}` 不应被接受为目录名"
            );
        }
        // 合法 ID 复用 manager 的清洗（`@` → `-`），保证与安装目录同款命名。
        assert_eq!(plugin_dir_segment("demo@external").expect("valid"), "demo-external");

        for bad in ["", "  ", "a/b", "a\\b", "a..b"] {
            assert!(
                matches!(
                    validate_binary_name(bad),
                    Err(SourceBuildError::InvalidBinaryName { .. })
                ),
                "`{bad}` 不应被接受为产物名"
            );
        }
        assert_eq!(validate_binary_name(" demo_worker ").expect("valid"), "demo_worker");
    }

    #[test]
    fn binary_file_name_carries_platform_suffix() {
        assert_eq!(
            binary_file_name("demo_worker"),
            format!("demo_worker{}", std::env::consts::EXE_SUFFIX)
        );
    }

    #[test]
    fn stderr_tail_keeps_the_conclusion_and_truncates_on_char_boundary() {
        let short = "error: boom\n";
        assert_eq!(stderr_tail(short), short);

        let long = format!("{}\n最后一次错误在末尾", "w".repeat(3000));
        let tail = stderr_tail(&long);
        assert!(tail.starts_with('…'), "超长 stderr 应带截断标记");
        assert!(tail.ends_with("最后一次错误在末尾"), "结论（尾部）必须保留");
        assert_eq!(tail.chars().count(), 2001);
    }

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("plugin-build-{label}-{nanos}"))
    }

    fn read_manifest(dir: &Path) -> serde_json::Value {
        let body = std::fs::read(dir.join(MANIFEST_FILE_NAME)).expect("manifest should exist");
        serde_json::from_slice(&body).expect("manifest should be json")
    }

    #[test]
    fn write_manifest_rewrites_worker_program_only_when_declared() {
        let root = temp_dir("manifest");
        let declared = serde_json::json!({
            "name": "demo",
            "version": "0.1.0",
            "description": "demo",
            "worker": { "program": "bin/demo", "args": ["--x"] }
        });
        write_manifest(&root, &declared, "bin/demo_worker.exe").expect("manifest should write");
        let written = read_manifest(&root);

        assert_eq!(written["worker"]["program"], "bin/demo_worker.exe");
        assert_eq!(written["worker"]["args"][0], "--x", "改写只碰 program，不动其余字段");

        let declarative = serde_json::json!({ "name": "demo", "version": "0.1.0" });
        write_manifest(&root, &declarative, "bin/demo_worker.exe").expect("manifest should write");

        assert!(read_manifest(&root).get("worker").is_none(), "未声明 worker 时不得凭空新增");
        assert!(matches!(
            write_manifest(&root, &serde_json::json!(["not", "an", "object"]), "bin/x"),
            Err(SourceBuildError::OutputWrite(_))
        ));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn write_source_snapshot_rejects_escaping_paths() {
        let root = temp_dir("snapshot");
        write_source_snapshot(&root.join("crate"), &[file("src/main.rs", "fn main() {}\n")])
            .expect("relative path should write");
        assert!(root.join("crate").join("src").join("main.rs").is_file());

        assert!(matches!(
            write_source_snapshot(&root.join("crate"), &[file("../escape.rs", "x")]),
            Err(SourceBuildError::SourceWrite(_))
        ));

        let _ = std::fs::remove_dir_all(root);
    }

    /// 真实 `cargo build --offline` 的端到端验证（默认跳过）。
    ///
    /// 门控理由与 `lib.rs` 的 `require_plugin_subprocess` 同款：它会真的拉起 cargo 编译，
    /// 在无工具链的环境里必须**打印说明后跳过**，而不是伪造通过或留下红。显式设
    /// `AXAGENT_TEST_PLUGIN_BUILD=1` 才真跑。
    #[test]
    fn builds_minimal_plugin_offline_when_enabled() {
        if std::env::var("AXAGENT_TEST_PLUGIN_BUILD").as_deref() != Ok("1") {
            eprintln!(
                "SKIP: 未设置 AXAGENT_TEST_PLUGIN_BUILD=1，跳过真实 cargo 构建。\n      \
                 本测试**未通过，也未被验证** —— 跳过仅表示环境不允许跑编译。"
            );
            return;
        }
        if probe_toolchain().is_err() {
            eprintln!("SKIP: 本机探测不到 rustc / cargo，无法验证隔离构建。");
            return;
        }

        let root = temp_dir("e2e");
        let manifest = serde_json::json!({
            "name": "build-demo",
            "version": "0.1.0",
            "description": "offline build smoke test",
            "worker": { "program": "filled-by-build" }
        });
        let request = SourceBuildRequest {
            plugin_id: "build-demo".to_string(),
            binary_name: "build_demo_worker".to_string(),
            files: vec![
                file(
                    "Cargo.toml",
                    "[package]\nname = \"build_demo_worker\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                ),
                file("src/main.rs", "fn main() {\n    println!(\"ok\");\n}\n"),
            ],
            manifest,
            target_triple: None,
            build_flags: Vec::new(),
        };

        let first = build_plugin_source(&root, &request).expect("first build should succeed");
        assert!(!first.cache_hit);
        let output_dir = Path::new(&first.output_dir);
        assert!(output_dir.join(&first.binary_relative_path).is_file(), "产物应落在输出目录内");
        assert!(output_dir.join(SOURCE_SUBDIR).join("Cargo.toml").is_file(), "源码必须留存");

        let written = read_manifest(output_dir);
        assert_eq!(written["worker"]["program"], first.binary_relative_path);

        // 第二次同请求：hash 命中 ⇒ 跳过编译。
        let second = build_plugin_source(&root, &request).expect("second build should succeed");
        assert!(second.cache_hit);
        assert_eq!(first.artifact_hash, second.artifact_hash);

        let target_name = format!("{TARGET_DIR_PREFIX}{}", first.artifact_hash);
        assert!(!output_dir.with_file_name(target_name).exists(), "临时 target-dir 应在构建后清理");

        let _ = std::fs::remove_dir_all(root);
    }
}
