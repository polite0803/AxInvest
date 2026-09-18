// SPDX-License-Identifier: AGPL-3.0-only

//! 应用配置持久化命令
//!
//! 提供前端 appConfigStore 的后端持久化支持。

use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::storage as storage_err;
use axagent_agent_macro::agent_command;
use tauri::State;

#[agent_command(domain = settings, safety = Safe, call_mode = StateOnly, description = "获取应用配置")]
#[tauri::command]
pub async fn get_app_config(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let db = state.harness.db();
    match axagent_dao::repo::settings::get_setting(db, "app_config").await {
        Ok(Some(json_str)) => {
            serde_json::from_str(&json_str).map_err(|e| format!("解析配置失败: {}", e))
        },
        Ok(None) => Ok(serde_json::json!({})),
        Err(e) => Err(ErrorResponse::new(storage_err::READ_FILE_FAILED)
            .with_detail(format!("读取配置失败: {}", e))
            .into()),
    }
}

#[agent_command(domain = settings, safety = Caution, call_mode = StateInput, description = "保存应用配置")]
#[tauri::command]
pub async fn save_app_config(
    state: State<'_, AppState>,
    config: serde_json::Value,
) -> Result<(), String> {
    let db = state.harness.db();
    let json_str = serde_json::to_string(&config).map_err(|e| format!("序列化配置失败: {}", e))?;
    axagent_dao::repo::settings::set_setting(db, "app_config", &json_str)
        .await
        .map_err(|e| format!("保存配置失败: {}", e))?;

    // 立即把 `maxIterations` 推送到 SessionManager。
    //
    // 若只写 DB 不注入，用户改完设置必须**重启应用**才生效 —— 这正是修复前
    // 的静默失效形态（铁律 #6：可配置 ≠ 可被优化）。放在此处而非新增独立命令，
    // 是因为 `save_app_config` 是全项目唯一的配置保存路径，内联注入可以
    // **结构性排除**「新增了保存路径但忘了同步注入」这类缺口。
    //
    // `None`（字段缺失 / 非数字 / 0）⇒ 传 0 表示清除覆盖，回退复杂度推导。
    state
        .agent_session_manager
        .set_max_iterations_override(extract_max_iterations(&config).unwrap_or(0));
    Ok(())
}

/// 从 `app_config` JSON 中提取最大迭代次数的私有 helper。
///
/// 仅接受 `1..=100_000` 的正整数 —— `0`、负数、小数与非数字一律视为「未配置」。
/// 上界与前端控件量程（`SettingsPanel` 的 `max={100}`）解耦：后端放宽到
/// 100_000 以便未来调整面板量程时无需改后端，仅拒绝明显病态的取值。
fn extract_max_iterations(config: &serde_json::Value) -> Option<usize> {
    let raw = config.get("maxIterations")?;
    let n = raw.as_u64()?;
    match n {
        0 => None,
        n if n <= 100_000 => Some(n as usize),
        _ => None,
    }
}

/// 读取 `app_config` 设置并解析为 JSON。数据库无配置或解析失败时返回 `None`。
///
/// 抽为独立 helper 以避免 `read_self_improvement_flags` / `read_max_iterations`
/// 各自重复「读 setting + parse」两步逻辑（AGENTS.md 第 12 条）。
async fn read_app_config_json(
    db: &axagent_harness::DatabaseConnection,
) -> Option<serde_json::Value> {
    let json_str = match axagent_dao::repo::settings::get_setting(db, "app_config").await {
        Ok(Some(s)) => s,
        _ => return None,
    };
    serde_json::from_str(&json_str).ok()
}

/// 从数据库读取用户配置的最大迭代次数（供 wiring 层启动时注入 SessionManager）。
///
/// 返回 `None` 表示用户未配置 ⇒ 调用方应保留「未覆盖」状态，
/// 让运行期回退到 `dynamic_max_iterations()` 的复杂度推导。
pub async fn read_max_iterations(db: &axagent_harness::DatabaseConnection) -> Option<usize> {
    extract_max_iterations(&read_app_config_json(db).await?)
}

/// `ReActConfig` 的**用户可配置子集**（P1-E 扩展，2026-09-12）。
///
/// 只收录「用户能理解 + 影响可感知行为」的字段。**刻意不暴露**以下字段 ——
/// 它们属引擎内部实现细节，暴露了没人能判断该填什么，只是多一个改坏引擎的入口：
///
/// - `max_depth` / `reflection_threshold` / `adaptive_reflection`：内部启发式参数
/// - `enable_analyzing`：内部阶段开关，关掉会让 `analyze` 阶段静默跳过
/// - `agent_role`：应由 AgentProfile 决定，手工覆盖会与 profile 冲突
/// - `checkpoint_interval`：与 `checkpoint_enabled` 配对，固定 10 足够
/// - `token_budget_enabled` / `token_budget_limit`：预算与模型上下文窗口耦合，
///   填小会中断长任务；装配点已收敛为确定值（见 `init/state.rs` 的注释）
/// - `self_improvement_enabled` / `final_output_reflection`：已由
///   `SelfImprovementFlags` 单独承接（见上方 `read_self_improvement_flags`）
/// - `self_improvement_loop_config`：复杂结构体，非标量
///
/// 所有字段都是 `Option`：`None` = 用户未配置 ⇒ **保留引擎默认值**，
/// 而不是用某个「合理的默认」覆盖（那会让引擎默认值再也改不动）。
#[derive(Debug, Clone, Default)]
pub struct ReactEngineOverrides {
    pub timeout_secs: Option<u64>,
    pub max_retry_attempts: Option<usize>,
    pub max_repeated_calls: Option<usize>,
    pub max_no_progress_iterations: Option<usize>,
    pub min_quality_threshold: Option<u8>,
    pub cycle_detection_enabled: Option<bool>,
    pub verification_enabled: Option<bool>,
    pub reflection_enabled: Option<bool>,
    pub checkpoint_enabled: Option<bool>,
}

impl ReactEngineOverrides {
    /// 把已配置的字段覆盖到 `config` 上（未配置的保持引擎默认）。
    pub fn apply_to(&self, config: &mut axagent_agent::reasoning_state::ReActConfig) {
        if let Some(v) = self.timeout_secs {
            config.timeout_secs = v;
        }
        if let Some(v) = self.max_retry_attempts {
            config.max_retry_attempts = v;
        }
        if let Some(v) = self.max_repeated_calls {
            config.max_repeated_calls = v;
        }
        if let Some(v) = self.max_no_progress_iterations {
            config.max_no_progress_iterations = v;
        }
        if let Some(v) = self.min_quality_threshold {
            config.min_quality_threshold = v;
        }
        if let Some(v) = self.cycle_detection_enabled {
            config.cycle_detection_enabled = v;
        }
        if let Some(v) = self.verification_enabled {
            config.verification_enabled = v;
        }
        if let Some(v) = self.reflection_enabled {
            config.enable_reflection = v;
        }
        if let Some(v) = self.checkpoint_enabled {
            config.checkpoint_enabled = v;
        }
    }
}

/// 从 `obj[key]` 提取正整数并校验闭区间 `[min, max]`。
///
/// 越界 / 类型不符 / 键缺失一律返回 `None`（= 未配置）。每个字段必须给**独立量程**：
/// 超时填 0 会让引擎立即超时，重试填 100_000 等于死循环 —— 量程是「改坏」的
/// 第二道防线（第一道是前端控件的 `min`/`max`，但前端可被绕过 / 直接改 DB）。
fn extract_bounded_u64(obj: &serde_json::Value, key: &str, min: u64, max: u64) -> Option<u64> {
    let n = obj.get(key)?.as_u64()?;
    (min..=max).contains(&n).then_some(n)
}

fn extract_bool(obj: &serde_json::Value, key: &str) -> Option<bool> {
    obj.get(key)?.as_bool()
}

/// 从数据库读取 `ReActConfig` 的用户覆盖项（供 wiring 层装配 `ReActEngine` 时应用）。
///
/// 读的是 `app_config.reactEngine` 子对象。整块缺失 ⇒ 全部 `None` ⇒ 引擎走默认值，
/// 与「用户从未打开过这个面板」的语义一致。
pub async fn read_react_engine_overrides(
    db: &axagent_harness::DatabaseConnection,
) -> ReactEngineOverrides {
    let Some(value) = read_app_config_json(db).await else {
        return ReactEngineOverrides::default();
    };
    let Some(engine) = value.get("reactEngine") else {
        return ReactEngineOverrides::default();
    };

    ReactEngineOverrides {
        // 下限 10s：低于此值的超时会让任何真实 LLM 调用必然失败。
        // 上限 3600s：超过 1 小时的单次运行已失去意义（外层还有会话级取消）。
        timeout_secs: extract_bounded_u64(engine, "timeoutSecs", 10, 3_600),
        max_retry_attempts: extract_bounded_u64(engine, "maxRetryAttempts", 0, 10)
            .map(|v| v as usize),
        // 下限 1：0 会让「重复调用检测」对第一次调用就报警。
        max_repeated_calls: extract_bounded_u64(engine, "maxRepeatedCalls", 1, 100)
            .map(|v| v as usize),
        max_no_progress_iterations: extract_bounded_u64(engine, "maxNoProgressIterations", 1, 100)
            .map(|v| v as usize),
        // 与 `Reflector::QualityMetrics` 的 0-10 分制对齐。
        min_quality_threshold: extract_bounded_u64(engine, "minQualityThreshold", 0, 10)
            .map(|v| v as u8),
        cycle_detection_enabled: extract_bool(engine, "cycleDetectionEnabled"),
        verification_enabled: extract_bool(engine, "verificationEnabled"),
        reflection_enabled: extract_bool(engine, "reflectionEnabled"),
        checkpoint_enabled: extract_bool(engine, "checkpointEnabled"),
    }
}

/// 缺陷1修复:从数据库读取前端 FeatureFlag,返回自改进循环相关的两个 flag 值。
///
/// 供 wiring 层(`init/state.rs`)在构造 `AppState` 时调用,把前端开关
/// 桥接到 `SessionManager::set_self_improvement_flags()`:
/// - `self_improvement_enabled` ← `features.selfImprovingLoop`
/// - `final_output_reflection` ← `features.finalOutputReflection`
///
/// 数据库无配置或解析失败时返回全 false 的默认值,保持向后兼容。
pub async fn read_self_improvement_flags(
    db: &axagent_harness::DatabaseConnection,
) -> axagent_agent::SelfImprovementFlags {
    let Some(value) = read_app_config_json(db).await else {
        return axagent_agent::SelfImprovementFlags::default();
    };
    let features = value.get("features");
    axagent_agent::SelfImprovementFlags {
        self_improvement_enabled: features
            .and_then(|f| f.get("selfImprovingLoop"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        final_output_reflection: features
            .and_then(|f| f.get("finalOutputReflection"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    }
}

/// 缺陷1修复:Tauri 命令,前端切换 selfImprovingLoop / finalOutputReflection 后调用,
/// 即时更新 SessionManager 的 flags(无需重启应用)。
///
/// 前端 `appConfigStore.toggleFeature` 在 saveConfig 后调用本命令,
/// 把最新 flag 值推送到后端 SessionManager。
#[agent_command(domain = settings, safety = Caution, call_mode = StateInput, description = "设置自我改进标志")]
#[tauri::command]
pub async fn set_self_improvement_flags(
    state: State<'_, AppState>,
    self_improvement_enabled: bool,
    final_output_reflection: bool,
) -> Result<(), String> {
    let flags =
        axagent_agent::SelfImprovementFlags { self_improvement_enabled, final_output_reflection };
    state.agent_session_manager.set_self_improvement_flags(flags).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `maxIterations` 的取值边界。这些断言是「用户改了不生效 / 改坏就崩」
    /// 两类故障的第一道防线：只有落在 `1..=100_000` 的正整数才被视为有效配置，
    /// 其余一律返回 `None` ⇒ 调用方回退到复杂度推导（而非把 0 当成上限写进引擎，
    /// 那会导致 `Max iterations (0) reached` 立即停摆）。
    #[test]
    fn extract_max_iterations_accepts_only_positive_integers_in_range() {
        let cases: &[(serde_json::Value, Option<usize>)] = &[
            // 正常值
            (serde_json::json!({ "maxIterations": 30 }), Some(30)),
            (serde_json::json!({ "maxIterations": 1 }), Some(1)),
            (serde_json::json!({ "maxIterations": 100 }), Some(100)),
            (serde_json::json!({ "maxIterations": 100_000 }), Some(100_000)),
            // 0 = 未配置哨兵（前端清空输入框会传 0/undefined）
            (serde_json::json!({ "maxIterations": 0 }), None),
            // 缺字段 / null
            (serde_json::json!({}), None),
            (serde_json::json!({ "maxIterations": null }), None),
            // 类型不对：字符串、浮点、负数一律拒绝（`as_u64` 对后两者返回 None）
            (serde_json::json!({ "maxIterations": "30" }), None),
            (serde_json::json!({ "maxIterations": 1.5 }), None),
            (serde_json::json!({ "maxIterations": -5 }), None),
            // 超上界
            (serde_json::json!({ "maxIterations": 100_001 }), None),
        ];
        for (input, expected) in cases {
            assert_eq!(
                extract_max_iterations(input),
                *expected,
                "input = {input} 时预期 {expected:?}"
            );
        }
    }

    /// 键名必须是 camelCase `maxIterations` —— 前端 `appConfigStore` 传的是
    /// `config.maxIterations`（`saveConfig` 的 payload）。写成 snake_case 会
    /// 静默落空（`get()` 返回 `None`），且与「未配置」不可区分。
    #[test]
    fn extract_max_iterations_rejects_snake_case_key() {
        assert_eq!(extract_max_iterations(&serde_json::json!({ "max_iterations": 30 })), None);
    }

    /// `extract_bounded_u64` 的闭区间语义。每个字段的量程都是「改坏」防线：
    /// 超时 0 秒 ⇒ 必定立即超时；重试 10 万次 ⇒ 事实上的死循环。
    #[test]
    fn extract_bounded_u64_is_closed_interval() {
        let v = serde_json::json!({ "k": 10 });
        assert_eq!(extract_bounded_u64(&v, "k", 10, 20), Some(10), "下界应包含");
        assert_eq!(extract_bounded_u64(&v, "k", 1, 9), None, "低于下界应拒绝");
        assert_eq!(extract_bounded_u64(&v, "k", 10, 20), Some(10));
        assert_eq!(extract_bounded_u64(&serde_json::json!({ "k": 20 }), "k", 10, 20), Some(20));
        assert_eq!(extract_bounded_u64(&serde_json::json!({ "k": 21 }), "k", 10, 20), None);
        // 缺键 / null / 字符串 / 小数 / 负数（`as_u64` 对后两者返回 None）
        assert_eq!(extract_bounded_u64(&serde_json::json!({}), "k", 0, 100), None);
        assert_eq!(extract_bounded_u64(&serde_json::json!({ "k": null }), "k", 0, 100), None);
        assert_eq!(extract_bounded_u64(&serde_json::json!({ "k": "10" }), "k", 0, 100), None);
        assert_eq!(extract_bounded_u64(&serde_json::json!({ "k": 1.5 }), "k", 0, 100), None);
        assert_eq!(extract_bounded_u64(&serde_json::json!({ "k": -1 }), "k", 0, 100), None);
    }

    /// `apply_to` 必须**只覆盖已配置（`Some`）的字段**。
    ///
    /// 若实现成「无条件赋值」，一个只填了超时的用户会把其余 8 个字段
    /// 一起打回默认值 —— 表面上没问题（值恰好等于默认），但当引擎默认值
    /// 在未来变更时，用户配置就会把它悄悄改回去（配置比代码更「长寿」）。
    #[test]
    fn apply_to_only_touches_configured_fields() {
        // 先制造一份与默认不同的状态，模拟「引擎侧已有非默认值」
        // （用结构体初始化而非逐字段赋值：clippy `field_reassign_with_default` 是 `-D warnings` 门禁）
        let mut config = axagent_agent::reasoning_state::ReActConfig {
            max_repeated_calls: 42,
            reflection_threshold: 7,
            ..Default::default()
        };

        let overrides = ReactEngineOverrides {
            timeout_secs: Some(600),
            checkpoint_enabled: Some(true),
            ..Default::default()
        };
        overrides.apply_to(&mut config);

        assert_eq!(config.timeout_secs, 600, "已配置字段应被覆盖");
        assert!(config.checkpoint_enabled, "已配置字段应被覆盖");
        assert_eq!(config.max_repeated_calls, 42, "未配置字段不得被改动");
        assert_eq!(config.reflection_threshold, 7, "未暴露字段更不得被改动");
    }

    /// 键名必须是 camelCase（前端 `appConfigStore` 传的是 `reactEngine.timeoutSecs`）。
    /// 写成 snake_case 会静默落空 —— 与「未配置」不可区分，正是铁律 #6 的形态。
    #[test]
    fn react_engine_overrides_reject_snake_case_keys() {
        let engine = serde_json::json!({
            "timeout_secs": 600,
            "max_retry_attempts": 5,
            "cycle_detection_enabled": false
        });
        assert_eq!(extract_bounded_u64(&engine, "timeoutSecs", 10, 3_600), None);
        assert_eq!(extract_bounded_u64(&engine, "maxRetryAttempts", 0, 10), None);
        assert_eq!(extract_bool(&engine, "cycleDetectionEnabled"), None);

        // camelCase 才生效
        let ok = serde_json::json!({ "timeoutSecs": 600, "cycleDetectionEnabled": false });
        assert_eq!(extract_bounded_u64(&ok, "timeoutSecs", 10, 3_600), Some(600));
        assert_eq!(extract_bool(&ok, "cycleDetectionEnabled"), Some(false));
    }
}
