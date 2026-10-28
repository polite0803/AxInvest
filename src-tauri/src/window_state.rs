// SPDX-License-Identifier: AGPL-3.0-only

use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PersistedWindowState {
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub maximized: bool,
    #[serde(default)]
    pub fullscreen: bool,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
}

pub fn window_state_path(axagent_home: &Path) -> PathBuf {
    axagent_home.join("window-state.json")
}

pub fn load_window_state(axagent_home: &Path) -> Option<PersistedWindowState> {
    let path = window_state_path(axagent_home);
    let json = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

pub fn save_window_state(axagent_home: &Path, state: PersistedWindowState) -> io::Result<()> {
    std::fs::create_dir_all(axagent_home)?;
    let json = serde_json::to_vec_pretty(&state).map_err(io::Error::other)?;
    std::fs::write(window_state_path(axagent_home), json)
}

pub fn clamp_window_state_to_monitor(
    state: PersistedWindowState,
    monitor_width: f64,
    monitor_height: f64,
) -> PersistedWindowState {
    // 下限必须同样参与 max()：逻辑分辨率很小的屏（如 1280×800 @150% ⇒ 逻辑高 533，
    // 乘 0.9 后低于 480）会让 `f64::clamp` 的 min > max 而 panic。本函数在 setup
    // 同步路径上，panic 等于应用启动即崩。
    let min_width = 640.0;
    let min_height = 480.0;
    let max_width = (monitor_width * 0.9).max(min_width);
    let max_height = (monitor_height * 0.9).max(min_height);
    PersistedWindowState {
        width: state.width.clamp(min_width, max_width),
        height: state.height.clamp(min_height, max_height),
        maximized: state.maximized,
        fullscreen: state.fullscreen,
        x: state.x.map(|v| v.clamp(0.0, (monitor_width - 100.0).max(0.0))),
        y: state.y.map(|v| v.clamp(0.0, (monitor_height - 100.0).max(0.0))),
    }
}

/// 决定本次启动要应用的窗口状态。
///
/// `saved` 是上次退出时保存的状态；`current_size` 是窗口按 `tauri.conf.json`
/// 创建后的实际尺寸（逻辑像素）；`monitor` 是当前显示器的逻辑尺寸。
///
/// 首次安装没有保存状态，但**同样要按显示器收敛**：150% 缩放的 1080p 屏逻辑
/// 可用高度只有约 680px，配置里的 800px 高会把窗口底部顶到屏幕外，表现为
/// 「输入框/状态栏看不见」——而开发机分辨率更大，永远看不到这一条。
///
/// 返回 `None` 表示拿不到任何尺寸依据，调用方应保持窗口创建时的状态。
pub fn resolve_window_state(
    saved: Option<PersistedWindowState>,
    current_size: Option<(f64, f64)>,
    monitor: Option<(f64, f64)>,
) -> Option<PersistedWindowState> {
    let base = match (saved, current_size) {
        (Some(state), _) => state,
        (None, Some((width, height))) => PersistedWindowState {
            width,
            height,
            maximized: false,
            fullscreen: false,
            // 首次安装没有位置 ⇒ 交给调用方 center()，避免落在屏幕角落
            x: None,
            y: None,
        },
        (None, None) => return None,
    };
    Some(match monitor {
        Some((width, height)) => clamp_window_state_to_monitor(base, width, height),
        None => base,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        PersistedWindowState, clamp_window_state_to_monitor, load_window_state,
        resolve_window_state, save_window_state,
    };

    #[test]
    fn round_trips_window_state_in_axagent_home() {
        let test_dir = std::env::temp_dir().join(format!(
            "axagent-window-state-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time before unix epoch")
                .as_nanos()
        ));

        std::fs::create_dir_all(&test_dir).expect("failed to create temp dir");
        let state = PersistedWindowState {
            width: 1440.0,
            height: 960.0,
            maximized: true,
            fullscreen: false,
            x: Some(100.0),
            y: Some(50.0),
        };

        save_window_state(&test_dir, state).expect("failed to save window state");

        let restored = load_window_state(&test_dir).expect("failed to load saved window state");
        assert_eq!(restored, state);

        let _ = std::fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn loads_legacy_state_without_new_fields() {
        let test_dir = std::env::temp_dir().join(format!(
            "axagent-window-state-legacy-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time before unix epoch")
                .as_nanos()
        ));

        std::fs::create_dir_all(&test_dir).expect("failed to create temp dir");
        let legacy_json = r#"{"width":1440.0,"height":960.0}"#;
        std::fs::write(test_dir.join("window-state.json"), legacy_json)
            .expect("failed to write legacy json");

        let restored = load_window_state(&test_dir).expect("failed to load legacy state");
        assert_eq!(restored.width, 1440.0);
        assert_eq!(restored.height, 960.0);
        assert!(!restored.maximized);
        assert!(!restored.fullscreen);
        assert!(restored.x.is_none());
        assert!(restored.y.is_none());

        let _ = std::fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn clamps_oversized_window_state_to_visible_monitor_bounds() {
        let clamped = clamp_window_state_to_monitor(
            PersistedWindowState {
                width: 2200.0,
                height: 1600.0,
                maximized: false,
                fullscreen: false,
                x: Some(2000.0),
                y: Some(1500.0),
            },
            1512.0,
            982.0,
        );

        assert!((clamped.width - 1360.8).abs() < f64::EPSILON);
        assert!((clamped.height - 883.8).abs() < 1e-9);
        // x clamped to monitor_width - 100
        assert!((clamped.x.expect("测试：x 应已设置") - 1412.0).abs() < f64::EPSILON);
        // y clamped to monitor_height - 100
        assert!((clamped.y.expect("测试：y 应已设置") - 882.0).abs() < f64::EPSILON);
    }

    #[test]
    fn first_run_shrinks_default_size_to_small_monitor() {
        // 1080p @150% ⇒ 逻辑 1280×720；配置的 1200×800 高度越界
        let resolved = resolve_window_state(None, Some((1200.0, 800.0)), Some((1280.0, 720.0)))
            .expect("首次运行应给出目标尺寸");

        assert!((resolved.width - 1152.0).abs() < 1e-9, "宽度应收到显示器 90%");
        assert!((resolved.height - 648.0).abs() < 1e-9, "高度应收到显示器 90%");
        assert!(
            resolved.x.is_none() && resolved.y.is_none(),
            "首次运行没有位置，应交给 center() 而不是落到 (0,0)"
        );
        assert!(!resolved.maximized && !resolved.fullscreen);
    }

    #[test]
    fn first_run_keeps_default_size_on_large_monitor() {
        let resolved = resolve_window_state(None, Some((1200.0, 800.0)), Some((2560.0, 1440.0)))
            .expect("首次运行应给出目标尺寸");

        assert!((resolved.width - 1200.0).abs() < 1e-9);
        assert!((resolved.height - 800.0).abs() < 1e-9);
    }

    #[test]
    fn tiny_monitor_does_not_panic_when_max_below_min() {
        // 显示器逻辑高 400 ⇒ 0.9 系数后 360 < 下限 480，旧实现会在 f64::clamp 处 panic
        let resolved = resolve_window_state(None, Some((1200.0, 800.0)), Some((800.0, 400.0)))
            .expect("首次运行应给出目标尺寸");

        assert!(resolved.width >= 640.0 - 1e-9);
        assert!(resolved.height >= 480.0 - 1e-9);
        assert!(resolved.x.is_none() && resolved.y.is_none());
    }

    #[test]
    fn saved_position_survives_when_size_is_known() {
        let saved = PersistedWindowState {
            width: 1000.0,
            height: 700.0,
            maximized: false,
            fullscreen: false,
            x: Some(50.0),
            y: Some(30.0),
        };

        let resolved =
            resolve_window_state(Some(saved), Some((1200.0, 800.0)), Some((1920.0, 1080.0)))
                .expect("有保存状态时应给出目标尺寸");

        assert!((resolved.width - 1000.0).abs() < 1e-9, "保存的尺寸不应被窗口创建尺寸顶替");
        assert!((resolved.height - 700.0).abs() < 1e-9);
        assert_eq!(resolved.x, Some(50.0));
        assert_eq!(resolved.y, Some(30.0));
    }

    #[test]
    fn resolve_returns_none_without_any_size_hint() {
        assert!(
            resolve_window_state(None, None, Some((1280.0, 720.0))).is_none(),
            "既无保存状态又拿不到窗口尺寸时应保持窗口创建时的状态"
        );
    }
}
