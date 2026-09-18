// SPDX-License-Identifier: AGPL-3.0-only

//! 构建脚本：自动扫描 src/commands/ 下的 Tauri 命令，
//! 生成 register_commands.rs 到 OUT_DIR，消除手动维护的命令列表。
//!
//! 扫描策略：
//! 1. 解析 commands/mod.rs 获取顶层 pub mod 声明
//! 2. 对每个模块扫描 #[tauri::command] 函数
//! 3. 对目录模块解析子 mod.rs 获取子模块 + pub use 重导出

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

struct CommandHandler {
    /// 完整路径，如 `commands::knowledge::list_knowledge_bases`
    path: String,
    /// 条件编译属性，如 `not(mobile)`
    cfg: Option<String>,
}

fn main() {
    // manifest 策略：
    //   主程序构建 — 调用 tauri_build::build()，通过 TAURI_MANIFEST 环境变量
    //              指定 common-controls.manifest，让 tauri-build 使用自定义 manifest。
    //   测试构建 (__TAURI_WORKSPACE__=true) — 跳过 tauri_build::build()，
    //              手动通过链接器参数嵌入 manifest，因为 tauri-build 在测试模式下
    //              不会嵌入 manifest。
    //   参考: https://github.com/tauri-apps/tauri/issues/11028
    let ws_val = env::var("__TAURI_WORKSPACE__").unwrap_or_default();
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("common-controls.manifest");
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    // 必须显式声明：本脚本按该变量决定走「嵌入 manifest」还是「tauri_build::build()」分支，
    // 只监听 manifest 文件的话，cargo build 与 cargo test 来回切换会复用上一次的产物，
    // 测试 exe 因此拿不到 Common Controls v6，启动即 STATUS_ENTRYPOINT_NOT_FOUND。
    println!("cargo:rerun-if-env-changed=__TAURI_WORKSPACE__");
    eprintln!("[build.rs] __TAURI_WORKSPACE__ = '{}'", ws_val);
    eprintln!("[build.rs] manifest_path = {}", manifest_path.display());
    eprintln!("[build.rs] manifest exists = {}", manifest_path.exists());

    if ws_val == "true" {
        // 测试模式：手动嵌入 manifest
        eprintln!("[build.rs] 测试模式 (__TAURI_WORKSPACE__=true)，跳过 tauri_build::build()");
        #[cfg(all(target_os = "windows", target_env = "msvc"))]
        {
            eprintln!("[build.rs] 嵌入 manifest: {}", manifest_path.display());
            println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
            println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest_path.display());
            println!("cargo:rustc-link-arg=/STACK:8388608");
        }
    } else {
        // 主程序构建：使用 tauri-build
        eprintln!("[build.rs] 主程序构建模式，使用 tauri_build::build()");
        unsafe {
            env::set_var("TAURI_MANIFEST", &manifest_path);
        }
        tauri_build::build();

        // Windows MSVC: 增加主线程栈到 8MB（默认 1MB，不足 deep call chain）
        // /STACK 是链接器选项，需通过 rustc-link-arg 传递
        #[cfg(all(target_os = "windows", target_env = "msvc"))]
        {
            println!("cargo:rustc-link-arg=/STACK:8388608");
        }
    }

    let out_dir = env::var("OUT_DIR").unwrap();
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let commands_dir = PathBuf::from(&manifest_dir).join("src").join("commands");

    // 注册自定义 cfg 条件，消除 unexpected_cfg 警告
    println!("cargo::rustc-check-cfg=cfg(mobile)");
    println!("cargo::rustc-check-cfg=cfg(desktop)");

    // 主 crate 在 Android/iOS 目标下设置 mobile cfg 标志。
    // Tauri 的 build.rs 会在编译 tauri crate 本身时设置 mobile cfg，
    // 但不会自动传递给依赖者，这里在主 crate 级别补齐。
    let target = env::var("TARGET").unwrap_or_default();
    let is_mobile = target.contains("android")
        || (target.contains("apple")
            && (target.contains("ios")
                || target.contains("tvos")
                || target.contains("watchos")
                || target.contains("visionos")));
    if is_mobile {
        println!("cargo:rustc-cfg=mobile");
    } else {
        println!("cargo:rustc-cfg=desktop");
    }

    let mut handlers: Vec<CommandHandler> = Vec::new();

    // 1. 解析 commands/mod.rs 获取顶层模块声明
    let mod_rs_path = commands_dir.join("mod.rs");
    let top_modules = parse_mod_rs(&mod_rs_path);

    eprintln!("[build.rs] 解析到 {} 个顶层模块", top_modules.len());
    for m in &top_modules {
        eprintln!("[build.rs]   模块: {} (cfg={:?})", m.name, m.cfg);
    }

    for module_decl in &top_modules {
        let module_path = commands_dir.join(format!("{}.rs", module_decl.name));
        let module_dir = commands_dir.join(&module_decl.name);

        if module_path.exists() {
            // 单文件模块：commands::module_name::func
            eprintln!("[build.rs] 扫描单文件模块: {}", module_decl.name);
            scan_file_for_commands(
                &module_path,
                &format!("commands::{}", module_decl.name),
                module_decl.cfg.clone(),
                &mut handlers,
            );
        } else if module_dir.is_dir() {
            // 目录模块
            let sub_mod_path = module_dir.join("mod.rs");
            let (sub_mods, re_exports) = if sub_mod_path.exists() {
                parse_submodule_mod_rs(&sub_mod_path)
            } else {
                (vec![], vec![])
            };

            eprintln!(
                "[build.rs] 扫描目录模块: {} (子模块: {}, 重导出: {})",
                module_decl.name,
                sub_mods.len(),
                re_exports.len()
            );

            // 扫描 mod.rs 中直接定义的函数
            if sub_mod_path.exists() {
                let before = handlers.len();
                scan_file_for_commands(
                    &sub_mod_path,
                    &format!("commands::{}", module_decl.name),
                    module_decl.cfg.clone(),
                    &mut handlers,
                );
                if handlers.len() > before {
                    eprintln!(
                        "[build.rs]   在 {}/mod.rs 中发现 {} 个命令",
                        module_decl.name,
                        handlers.len() - before
                    );
                }
            }

            // 处理 pub mod 声明的子模块（公开）
            for sub_mod in &sub_mods {
                let sub_file = module_dir.join(format!("{}.rs", sub_mod.name));
                if sub_file.exists() {
                    let before = handlers.len();
                    scan_file_for_commands(
                        &sub_file,
                        &format!("commands::{}::{}", module_decl.name, sub_mod.name),
                        module_decl.cfg.clone(),
                        &mut handlers,
                    );
                    if handlers.len() > before {
                        eprintln!(
                            "[build.rs]   在 {}/{}.rs 中发现 {} 个命令",
                            module_decl.name,
                            sub_mod.name,
                            handlers.len() - before
                        );
                    }
                }
            }

            // 处理 pub use submodule::* 重导出
            for re_export in &re_exports {
                let sub_file = module_dir.join(format!("{}.rs", re_export.submodule));
                if sub_file.exists() {
                    let before = handlers.len();
                    scan_file_for_commands(
                        &sub_file,
                        &format!("commands::{}", module_decl.name),
                        module_decl.cfg.clone(),
                        &mut handlers,
                    );
                    if handlers.len() > before {
                        eprintln!(
                            "[build.rs]   在 {} 通过 pub use {}::* 发现 {} 个命令",
                            module_decl.name,
                            re_export.submodule,
                            handlers.len() - before
                        );
                    }
                }
            }
        }
    }

    // 2. 扫描 src/ 根目录下的命令文件（不在 commands/ 目录中的模块）
    let root_files = vec!["knowledge_integration", "tray"];
    for file_name in &root_files {
        let file_path = PathBuf::from(&manifest_dir).join("src").join(format!("{}.rs", file_name));
        if file_path.exists() {
            eprintln!("[build.rs] 扫描根目录文件: {}", file_name);
            let before = handlers.len();
            // tray 在 lib.rs 中声明为 `#[cfg(desktop)]`，其余根文件无模块级 cfg
            let root_cfg: Option<String> = if *file_name == "tray" {
                Some("desktop".to_string())
            } else {
                None
            };
            scan_file_for_commands(&file_path, file_name, root_cfg, &mut handlers);
            if handlers.len() > before {
                eprintln!(
                    "[build.rs]   在 {} 中发现 {} 个命令",
                    file_name,
                    handlers.len() - before
                );
            }
        }
    }

    // 3. 扫描 src/scheduler/ 目录（独立模块，不在 commands/ 下）
    let scheduler_dir = PathBuf::from(&manifest_dir).join("src").join("scheduler");
    if scheduler_dir.is_dir() {
        let scheduler_mod = scheduler_dir.join("mod.rs");
        eprintln!("[build.rs] 扫描 scheduler 目录");

        // 扫描 scheduler/mod.rs 中的命令
        if scheduler_mod.exists() {
            let before = handlers.len();
            scan_file_for_commands(&scheduler_mod, "scheduler", None, &mut handlers);
            if handlers.len() > before {
                eprintln!(
                    "[build.rs]   在 scheduler/mod.rs 中发现 {} 个命令",
                    handlers.len() - before
                );
            }
        }

        // 扫描 scheduler/ 下的子模块
        for sub_file in &["gate", "queue", "report", "restore"] {
            let sub_path = scheduler_dir.join(format!("{}.rs", sub_file));
            if sub_path.exists() {
                let before = handlers.len();
                scan_file_for_commands(
                    &sub_path,
                    &format!("scheduler::{}", sub_file),
                    None,
                    &mut handlers,
                );
                if handlers.len() > before {
                    eprintln!(
                        "[build.rs]   在 scheduler/{}.rs 中发现 {} 个命令",
                        sub_file,
                        handlers.len() - before
                    );
                }
            }
        }
    }

    handlers.sort_by(|a, b| a.path.cmp(&b.path));

    // 去重：同一个源文件常被扫描两次 —— ① 作为 `mod xxx;` 子模块（第 1 步的 sub_mods）
    // ② 作为 `pub use xxx::*` 重导出（第 1 步的 re_exports），两次产出的
    // path + cfg 完全相同 ⇒ 生成列表里出现重复 handler。
    // 实测（2026-09-12）：1500 条注册里 **13 条是重复**，例如
    // `commands::browser::browser_click` 连续出现两次（两条都带
    // `#[cfg(not(mobile))]`）。重复注册不会让编译失败、也不会报警，属静默冗余。
    //
    // 合并条件必须是 **path 与 cfg 都相同**：cfg 不同的同 path 条目是合法变体
    // （不同目标平台启用不同实现），删掉会丢注册。
    let before_dedup = handlers.len();
    handlers.dedup_by(|a, b| a.path == b.path && a.cfg == b.cfg);
    if handlers.len() != before_dedup {
        eprintln!(
            "[build.rs] 已合并 {} 条重复的命令注册（同一文件被模块声明与重导出各扫一次）",
            before_dedup - handlers.len()
        );
    }

    eprintln!("[build.rs] 共发现 {} 个命令", handlers.len());
    for h in handlers.iter().take(10) {
        eprintln!("[build.rs]   命令: {} (cfg={:?})", h.path, h.cfg);
    }
    if handlers.len() > 10 {
        eprintln!("[build.rs]   ... 以及其他 {} 个命令", handlers.len() - 10);
    }

    if handlers.is_empty() {
        panic!("build.rs: 未找到任何 Tauri 命令！请检查 commands/mod.rs 中的模块声明。");
    }

    let generated = generate_output(&handlers);
    let output_path = Path::new(&out_dir).join("register_commands.rs");
    fs::write(&output_path, &generated).unwrap();

    // 同时生成 src/register_commands.rs 供契约检查脚本使用
    let src_register_path = PathBuf::from(&manifest_dir).join("src").join("register_commands.rs");
    fs::write(&src_register_path, &generated).unwrap();

    println!("cargo:rerun-if-changed=src/commands");
    println!("cargo:rerun-if-changed=src/scheduler");
    println!("cargo:rerun-if-changed=src/knowledge_integration.rs");
    println!("cargo:rerun-if-changed=src/tray.rs");
    println!("cargo:rerun-if-changed=build.rs");
}

/// 顶层 mod.rs 中的模块声明
struct ModuleDecl {
    name: String,
    cfg: Option<String>,
}

/// 子 mod.rs 中的子模块声明
struct SubModuleDecl {
    name: String,
}

/// pub use 重导出声明
struct ReExport {
    submodule: String,
}

/// 解析顶层 mod.rs（只提取 pub mod 声明）
fn parse_mod_rs(path: &Path) -> Vec<ModuleDecl> {
    let Ok(content) = fs::read_to_string(path) else {
        return vec![];
    };

    let mut decls = Vec::new();
    let mut pending_cfg: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("#[cfg(") {
            let cfg = trimmed.trim_start_matches("#[cfg(").trim_end_matches(")]").to_string();
            pending_cfg = Some(cfg);
            continue;
        }

        if trimmed.starts_with('#') {
            continue;
        }

        // 匹配 pub mod name;
        if let Some(rest) = trimmed.strip_prefix("pub mod ") {
            let name = rest.trim_end_matches(';').trim().to_string();
            decls.push(ModuleDecl { name, cfg: pending_cfg.take() });
        } else if trimmed.starts_with("pub(crate) mod ") || trimmed.starts_with("mod ") {
            // 跳过非 pub mod 声明
            pending_cfg = None;
        } else if !trimmed.is_empty()
            && !trimmed.starts_with("use ")
            && !trimmed.starts_with("//")
            && !trimmed.starts_with("/*")
        {
            pending_cfg = None;
        }
    }

    decls
}

/// 解析子目录 mod.rs，提取 pub mod 声明和 pub use 重导出
///
/// ⚠ 只把 **通配符** 重导出（`pub use sub::*`）计为「子模块命令已上提到父模块」。
///
/// 为什么排除 `pub use sub::{a, b}`（2026-09-13 修复）：
/// 该形式只把**列举的那几项**提到父模块可见，而不是"该子模块全部命令都在父模块可见"。
/// 旧实现用 `rest.contains("::{")` 把两者混同 ⇒ 只要 `sub.rs` 里有 `#[tauri::command]`，
/// 第 171 行就会用**父模块路径**（`commands::<父>::<cmd>`）注册这些命令，
/// 而它们并未被 `::{a, b}` 导出 ⇒ 编译期报
/// `cannot find __cmd__<cmd> in <父模块>` / `cannot find __tauri_command_name_<cmd>`。
/// 该报错指向 register_commands.rs（build.rs 的生成物，DO NOT EDIT），
/// 完全指不到真因 —— 典型「改到生成物上 = 假修复」。
///
/// 因此：需要上提子模块命令时，请用 `pub use sub::*;` 或逐条 `pub use sub::cmd;`。
fn parse_submodule_mod_rs(path: &Path) -> (Vec<SubModuleDecl>, Vec<ReExport>) {
    let Ok(content) = fs::read_to_string(path) else {
        return (vec![], vec![]);
    };

    let mut sub_mods = Vec::new();
    let mut re_exports = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // pub mod name;
        if let Some(rest) = trimmed.strip_prefix("pub mod ") {
            let name = rest.trim_end_matches(';').trim().to_string();
            sub_mods.push(SubModuleDecl { name });
        }
        // pub use name::*; —— 仅通配符形式才代表"该子模块命令已在父模块可见"
        else if let Some(rest) = trimmed.strip_prefix("pub use ") {
            let rest = rest.trim_end_matches(';').trim();
            if rest.ends_with("::*")
                && let Some((submod, _)) = rest.split_once("::")
            {
                re_exports.push(ReExport { submodule: submod.trim().to_string() });
            }
        }
    }

    (sub_mods, re_exports)
}

/// 扫描单个文件中的 #[tauri::command] 或 #[command] 函数
/// `module_cfg` 为模块级条件编译（来自 commands/mod.rs 的 `#[cfg(...)]` 或根文件声明），
/// 当函数自身无独立 cfg 时继承之，确保移动端（mobile）构建不会引用被排除的模块。
fn scan_file_for_commands(
    file_path: &Path,
    module_prefix: &str,
    module_cfg: Option<String>,
    handlers: &mut Vec<CommandHandler>,
) {
    let Ok(content) = fs::read_to_string(file_path) else {
        return;
    };

    // 同时支持 #[tauri::command] 和 #[command]
    let has_tauri_cmd = content.contains("#[tauri::command]");
    let has_cmd = content.contains("#[command]");
    if !has_tauri_cmd && !has_cmd {
        return;
    }

    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    let mut pending_cfg: Option<String> = None;
    let mut found_in_file = 0;
    let file_name = file_path.file_name().unwrap_or_default().to_string_lossy();

    while i < lines.len() {
        let line = lines[i].trim();

        if line.starts_with("#[cfg(") {
            let cfg_content = line.trim_start_matches("#[cfg(").trim_end_matches(")]").to_string();
            pending_cfg = Some(cfg_content);
            i += 1;
            continue;
        }

        // 检查是否为 pub async fn / pub fn 定义
        if line.starts_with("pub async fn ") || line.starts_with("pub fn ") {
            // 向前搜索命令注解。
            //
            // ⚠ **必须能穿过 doc 注释块** —— 这是本函数的头号陷阱。
            // `#[tauri::command]` 与 `pub fn` 之间夹 `///` 文档注释是合法且常见的
            // 写法，而旧实现遇到第一行「非属性、非空行」就 `break`，`///` 恰好
            // 落在这一支 ⇒ 这类命令**永远不被收录**。
            //
            // 致命之处在于它**每次构建都自动复现**：build.rs 会把生成的列表
            // `fs::write` 回 `src/register_commands.rs`（见本文件末尾），所以
            // 「手工补一行」在下次构建时会被静默覆盖掉 —— 缺陷看起来像"修好了"，
            // 实际只是被撤销前的假象。
            //
            // 生产实证（2026-09-12）：`commands::memory::rebuild_memory_index`
            // 的注解与 fn 之间夹 10 行文档注释（`memory.rs:562`→`573`），因此从未
            // 注册；契约检查 a 段报「已定义但未注册到 generate_handler!（前端
            // invoke 会 404）」，而前端有 2 个调用点（`MemorySettings.tsx`）+
            // 1 个 store 调用点。构建快照时间线可作为独立佐证：
            // 该行在 09-02 起的所有快照中都存在，12 日 19:52 快照仍在，
            // 21:08 快照已消失 —— 消失时点与源码 mtime（=构建时刻）重合。
            //
            // 窗口给到 64 行而非 10 行：10 行连一个中等长度的文档注释都覆盖不了，
            // 且窗口一旦耗尽同样是静默漏收录。
            let mut is_cmd = false;
            for j in 1..=64 {
                if j > i {
                    break;
                }
                let prev = lines[i - j].trim();

                // 空行与注释（行注释 / 文档注释 / 块注释）一律跳过，**不终止搜索**
                if prev.is_empty()
                    || prev.starts_with("//")
                    || prev.starts_with("/*")
                    || prev.starts_with('*')
                {
                    continue;
                }

                // 检查是否为命令注解
                if prev == "#[tauri::command]" || prev == "#[command]" {
                    is_cmd = true;
                    break;
                }

                // 如果遇到 #[ 开头的属性行，检查是否为命令注解
                if prev.starts_with('#') {
                    // 跳过 #[cfg(...)] 属性
                    if prev.starts_with("#[cfg(") {
                        continue;
                    }
                    // 跳过 #[agent_command(...)] 等自定义属性
                    if prev.starts_with("#[agent_command") {
                        continue;
                    }
                    // 其他 #[ 开头的行（如 #[derive(...)]、#[allow(...)] 等）
                    // 检查是否为命令注解的开始
                    if prev.contains("tauri::command") || prev == "#[command]" {
                        is_cmd = true;
                        break;
                    }
                    // 其他属性行，继续搜索
                    continue;
                }

                // 如果是属性的一部分（包含括号或等号），继续搜索
                if prev.contains('(')
                    || prev.contains(')')
                    || prev.contains('=')
                    || prev.contains(',')
                {
                    continue;
                }

                // 遇到非属性行，停止搜索
                break;
            }

            if is_cmd {
                let func_line = line
                    .strip_prefix("pub async fn ")
                    .or_else(|| line.strip_prefix("pub fn "))
                    .unwrap_or(line);
                let func_name = func_line
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect::<String>();

                if !func_name.is_empty() {
                    let full_path = format!("{}::{}", module_prefix, func_name);
                    let func_cfg = pending_cfg.take();
                    // 合并模块级 cfg 与函数级 cfg：
                    // - 函数无独立 cfg → 继承模块级 cfg（覆盖移动端被排除的模块）
                    // - 二者都有且不同 → all(module, func)
                    let effective_cfg = match (func_cfg, module_cfg.clone()) {
                        (Some(f), Some(m)) if f == m => Some(f),
                        (Some(f), Some(m)) => Some(format!("all({}, {})", m, f)),
                        (Some(f), None) => Some(f),
                        (None, Some(m)) => Some(m),
                        (None, None) => None,
                    };
                    handlers.push(CommandHandler { path: full_path, cfg: effective_cfg });
                    found_in_file += 1;
                }
            } else if has_tauri_cmd || has_cmd {
                // 调试：如果文件包含命令注解但没匹配到，输出日志
                let func_line = line
                    .strip_prefix("pub async fn ")
                    .or_else(|| line.strip_prefix("pub fn "))
                    .unwrap_or(line);
                let func_name = func_line
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect::<String>();
                if !func_name.is_empty() && found_in_file == 0 && i < 20 {
                    eprintln!(
                        "[build.rs]     调试 {}: 检查函数 '{}' - 前5行:",
                        file_name, func_name
                    );
                    for j in 1..=5 {
                        if j <= i {
                            let prev = lines[i - j].trim();
                            eprintln!(
                                "[build.rs]       行 {}: '{}' (是空: {}, 匹配tauri: {}, 匹配command: {})",
                                i - j,
                                prev,
                                prev.is_empty(),
                                prev == "#[tauri::command]",
                                prev == "#[command]"
                            );
                        }
                    }
                }
            }
        }

        if !line.is_empty()
            && !line.starts_with("use ")
            && !line.starts_with("//")
            && !line.starts_with("/*")
            && !line.starts_with("pub ")
        {
            pending_cfg = None;
        }

        i += 1;
    }

    if found_in_file > 0 {
        eprintln!(
            "[build.rs]     {}: 发现 {} 个命令",
            file_path.file_name().unwrap_or_default().to_string_lossy(),
            found_in_file
        );
    }

    // ── 自检：本文件「声明」的命令注解数 vs 「收录」的命令数 ────────────────
    //
    // 本解析器是**文本启发式**的，任何它跳不过的写法都会让命令静默漏收录，
    // 而漏收录的后果是前端 `invoke` 直接 404（`cargo check` / `tsc` / 单测
    // 三者都不会报警）。所以这里把差集显式喊出来，不让它只体现在运行期。
    //
    // 判据用**整行精确相等**而不是 `contains`：文档注释里提到
    // `#[tauri::command]` 时，若用 contains 会凭空多算，把警告变成噪音。
    let declared = lines
        .iter()
        .filter(|l| {
            let l = l.trim();
            l == "#[tauri::command]" || l == "#[command]"
        })
        .count();
    if declared != found_in_file {
        eprintln!(
            "[build.rs] ⚠ 警告: {} 声明了 {} 个命令注解，但只收录 {} 个 —— \
             未被收录的命令会导致前端 invoke 404（契约检查 a 段会 fail）。\
             请检查这些命令的注解与 fn 之间是否有本解析器跳不过的语句。",
            file_path.file_name().unwrap_or_default().to_string_lossy(),
            declared,
            found_in_file
        );
    }
}

fn generate_output(handlers: &[CommandHandler]) -> String {
    let mut out = String::new();

    out.push_str("// Auto-generated by build.rs — DO NOT EDIT\n");
    out.push_str("// 手动修改将被下一次构建覆盖\n\n");

    out.push_str("#[allow(unused_macros)]\n");
    out.push_str("macro_rules! register_all_commands {\n");
    out.push_str("    () => {\n");
    out.push_str("        tauri::generate_handler![\n");

    for (i, h) in handlers.iter().enumerate() {
        if let Some(cfg) = &h.cfg {
            out.push_str(&format!("            #[cfg({})]\n", cfg));
        }
        out.push_str(&format!("            {}", h.path));
        if i < handlers.len() - 1 {
            out.push(',');
        }
        out.push('\n');
    }

    out.push_str("        ]\n");
    out.push_str("    };\n");
    out.push_str("}\n");

    out
}
