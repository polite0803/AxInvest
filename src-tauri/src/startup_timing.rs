// SPDX-License-Identifier: AGPL-3.0-only

//! 启动计时落盘。
//!
//! 桌面端 tracing 只写 stdout（`lib.rs` 的 `fmt().init()`，全仓无 tracing-appender），
//! release 构建又被 `windows_subsystem` 丢弃 ⇒ 已有的 `[startup] … 完成` 埋点**事后无法归因**：
//! 用户报「首屏弹出很慢」时，仓里没有任何一次真实启动的数字可查。
//!
//! 本模块把关键阶段的耗时追加写 `{axagent_home}/startup-timing.log`（每次启动截断重写，
//! 只留最近一次），使「进程启动 → 窗口即将 show」可被事后量化。写盘全程 best-effort：
//! 任何 IO 失败只丢日志，绝不影响启动。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

/// `run()` 入口时刻。以它为基准的 `since_start_ms` 就是「距进程启动」。
static PROCESS_START: OnceLock<Instant> = OnceLock::new();

/// 日志落点，`axagent_home` 就绪前为 `None`（此阶段只进 tracing）。
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// 在 `run()` 最开头调用，建立「距进程启动」的基准。重复调用无副作用。
pub fn note_process_start() {
    let _ = PROCESS_START.set(Instant::now());
}

/// 记录一个阶段：`phase` 为阶段名，`elapsed_ms` 为该阶段自身耗时。
/// 同时记 `since_start_ms`，两个数一起才分得清「这一段慢」还是「前面已经慢了很久」。
pub fn record(phase: &str, elapsed_ms: u128) {
    write_line(phase, Some(elapsed_ms));
}

/// 只关心累计值的门控点：不写 `elapsed_ms` 字段，避免它与累计值同数而被读成「本阶段耗时」。
pub fn record_since_start(phase: &str) {
    write_line(phase, None);
}

fn write_line(phase: &str, elapsed_ms: Option<u128>) {
    let since_start = PROCESS_START.get().map(|t| t.elapsed().as_millis()).unwrap_or(0);
    match elapsed_ms {
        Some(ms) => {
            tracing::info!(elapsed_ms = %ms, since_start_ms = %since_start, "[startup_timing] {phase}");
        },
        None => {
            tracing::info!(since_start_ms = %since_start, "[startup_timing] {phase}");
        },
    }
    let Some(path) = LOG_PATH.get() else { return };
    let elapsed_field = elapsed_ms.map(|ms| format!("elapsed_ms={ms}\t")).unwrap_or_default();
    let line =
        format!("{}\t{}\t{}since_start_ms={}\n", chrono_now(), phase, elapsed_field, since_start);
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
}

// ── 一次性门控 ──
// 下面两个阶段所在的命令在运行期会被反复调用（设置面板每次保存都打 get_settings），
// 无条件记录会让日志被运行期噪声淹没并无限增长 ⇒ 各用一次 `Once` 只留首条。
static FIRST_GET_SETTINGS: std::sync::Once = std::sync::Once::new();
static WINDOW_SHOW_GATE: std::sync::Once = std::sync::Once::new();

/// 事件循环可用 + 前端第一个 await 返回的时刻（`setup` 未返回时 IPC 只会排队）。
pub fn note_first_get_settings() {
    FIRST_GET_SETTINGS.call_once(|| record_since_start("ipc_first_get_settings"));
}

/// 前端 `AppInitializer` 在 `showWindow()` 前一刻打的 `apply_startup_settings`。
/// `since_start_ms` 即用户感知的「首屏弹出」时长上界（其后仅 show()+setFocus() 两个 IPC）。
pub fn note_window_show_gate() {
    WINDOW_SHOW_GATE.call_once(|| record_since_start("frontend_window_show_gate"));
}

/// 截断重写为本次启动的日志，并写一行表头。必须在 `note_process_start()` 之后调用。
pub fn init_log_file(app_dir: &Path) {
    let path = app_dir.join("startup-timing.log");
    let header = format!(
        "# AxInvest startup timing | version {}\n# {}\tprocess_start\tsince_start_ms=0\n",
        env!("CARGO_PKG_VERSION"),
        chrono_now(),
    );
    let _ = std::fs::write(&path, header);
    let _ = LOG_PATH.set(path);
}

fn chrono_now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 自检本模块的两件承诺：① 阶段行带 `elapsed_ms`、门控行不带（否则累计值会被误读成本段耗时）；
    /// ② `Once` 门控在重复调用下只落一条。`OnceLock` 是进程级单例 ⇒ 全测试体只此一个用例，
    /// 不拆成多个，避免并行线程互相污染基准与落点。
    #[test]
    fn startup_timing_lines_carry_the_right_fields() {
        note_process_start();
        let first = *PROCESS_START.get().expect("基准应已建立");
        note_process_start();
        assert_eq!(first, *PROCESS_START.get().unwrap(), "重复 note_process_start 不得移动基准");

        let dir =
            std::env::temp_dir().join(format!("axagent-startup-timing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("临时目录应可建");
        init_log_file(&dir);

        record("unit_phase", 7);
        note_first_get_settings();
        note_first_get_settings();
        note_window_show_gate();
        note_window_show_gate();

        let content =
            std::fs::read_to_string(dir.join("startup-timing.log")).expect("日志应已落盘");
        assert!(content.contains("unit_phase\telapsed_ms=7\tsince_start_ms="), "{content}");
        assert!(content.contains("ipc_first_get_settings\tsince_start_ms="), "{content}");
        assert!(
            !content.contains("ipc_first_get_settings\telapsed_ms="),
            "门控行不该带 elapsed_ms: {content}"
        );
        let gate_lines =
            content.lines().filter(|l| l.contains("frontend_window_show_gate")).count();
        assert_eq!(gate_lines, 1, "重复门控只应落一条，实得 {gate_lines} 行:\n{content}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
