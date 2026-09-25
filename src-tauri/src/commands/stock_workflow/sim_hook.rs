//! 决策落库后的「仿真验证」挂钩 —— **单一实现源**。
//!
//! ## 为什么是挂钩，而不是 DAG 节点
//!
//! 模板里的 `sim-verify` 节点是 `enabled: false` **且不连任何边**的「图示节点」
//! （两条约束的必要性见 `stock_analysis_setup/seed_stock_analysis.rs` 节点定义处
//! 注释）。因此仿真**不进 DAG 调度**，而是由本模块在**决策持久化之后**触发：
//!
//! - 挂在 `workflow-completed` emit 之后（`core.rs`）⇒ **不占用工作流执行时长**；
//! - 此刻 action / positionPct 已由 portfolio-risk-gate + quality-gate 定稿，
//!   仿真只写补充信息、**不产出任何决策字段**，全图无节点消费它
//!   ⇒ 结果物理上不可能回灌决策（「不阻滞决策」由数据依赖方向保证，不靠约定）。
//!
//! ## 失败结果的契约（两条路径必须同构）
//!
//! 写进快照的值有两种形态，**`simOk` 是唯一判据**：
//!
//! ```text
//! 成功：{ simOk: true,  stockCode, referencePrice, … }        ← McSimResult 序列化体
//! 失败：{ simOk: false, code, category, detail? }             ← ErrorResponse 序列化体
//! ```
//!
//! 为什么失败体复用 `ErrorResponse` 而不是自由文本：
//! ① 前端已有统一翻译层（`src/lib/errorI18n.ts` 按 `t("error.${code}")` 查表），
//!    同一形状 ⇒ 前端零特殊分支；自由文本在日/德/法/俄界面无法翻译。
//! ② 错误码定义在 `commands/error_code.rs::stock_sim`，由
//!    `scripts/check-errorcode-alignment.mjs` 强制与 11 语言 `error` 段对齐。
//! ③ 规范要求错误码**平铺在顶层 `error` 段**，禁止散落到业务子段
//!    （AGENTS.md「后端错误码 i18n 规范」第 6 条）。
//!
//! `detail` 只放技术串（变量名 + 值），业务原因一律由 `code` 表达。
//!
//! ## 为什么必须由两个入口共用
//!
//! `rerun_decision`（重跑决策）会改决策但**不重写快照**。若它不重跑仿真，
//! 前端会把上一版决策的仿真结论当成新决策的结论展示 —— 决策变了而补充信息没变，
//! 是最典型的一种误导。历史教训：同一逻辑两套实现必然漂移，故两者共用本函数。

use crate::commands::error::{ErrorCategory, ErrorResponse};
use crate::commands::error_code::stock_sim as sim_err;
use axagent_harness::IpcEventName;
// 快照读写**经 dao**（`axagent_dao::repo::stock_analysis_snapshot`），命令层不得直连
// `axagent_entities` / `sea_orm` —— 分层门禁 `commands-no-direct-db` 的判据。
// `DatabaseConnection` 也走 dao 暴露的出口（`crates/dao/src/db.rs` 明文：消费者
// 只需 `use axagent_*::DatabaseConnection`），不再引用第三方 crate 的路径。
use axagent_dao::db::DatabaseConnection;
use axagent_dao::repo::stock_analysis_snapshot;
use tauri::Emitter;

/// 构造失败体骨架：`ErrorResponse` 序列化体（`code` + `category` + `detail`）。
///
/// `detail` 只放技术串，**不放面向用户的整句文案** —— 用户看到的是
/// `error.${code}` 的翻译；`detail` 仅在前端缺译时兜底，以及用于日志排查。
fn err_value(code: &str, category: ErrorCategory, detail: impl Into<String>) -> serde_json::Value {
    serde_json::to_value(ErrorResponse::new(code).with_category(category).with_detail(detail))
        .unwrap_or_else(|_| serde_json::json!({ "code": code }))
}

/// 给任意对象补 `simOk: false` —— 成功/失败的唯一判据。
///
/// 单独成函数是因为两条路径都要贴这个标记：宿主失败体已是 `ErrorResponse` 形状，
/// 只需贴标记；而本地构造的失败体还要先经 [`err_value`]。
///
/// 按值解构而非 `as_object_mut()`：后者会让 `v` 在可变借用期内，无法在分支里
/// 把 `v` 移出去返回。
fn mark_failed(v: serde_json::Value) -> serde_json::Value {
    let mut obj = match v {
        serde_json::Value::Object(m) => m,
        // 非对象（理论上不可达）：退化成最小可识别失败体，但同样要贴 simOk 判据，
        // 不返回半成品。
        _ => {
            let mut v = err_value(
                sim_err::RESULT_PARSE_FAILED,
                ErrorCategory::Unrecoverable,
                "non-object payload",
            );
            v["simOk"] = serde_json::Value::Bool(false);
            return v;
        },
    };
    obj.insert("simOk".to_string(), serde_json::Value::Bool(false));
    serde_json::Value::Object(obj)
}

/// 决策已定稿后触发仿真：跑蒙特卡洛 → 写回 `blackboard_snapshot` → emit 事件。
///
/// **不阻塞调用方**（`tokio::spawn` + `spawn_blocking`）。
///
/// - `reference_price_yuan`：调用方若已拿到实时行情就传 `Some(价)`（元）；
///   传 `None` 时回退到快照里 `t-scoring` 记录的现价 —— `rerun_decision` 没有新的
///   行情输入，它重算用的就是快照里那份数据，**从快照取价才是同口径**的。
pub(crate) fn spawn_simulation_after_decision(
    db: DatabaseConnection,
    app: tauri::AppHandle,
    analysis_id: String,
    stock_code: String,
    reference_price_yuan: Option<f64>,
) {
    tokio::spawn(async move {
        let _ = run_simulation_after_decision(
            &db,
            Some(&app),
            &analysis_id,
            &stock_code,
            reference_price_yuan,
        )
        .await;
    });
}

/// 仿真的**实际执行体**：跑 → 写回 → （可选）emit。返回写进快照的那份结果。
///
/// 抽成独立函数而不是内联进 `tokio::spawn`，是为了让「无 AppHandle 的调用方」
/// 也能复用（并拿到返回值）。
pub(crate) async fn run_simulation_after_decision(
    db: &DatabaseConnection,
    app: Option<&tauri::AppHandle>,
    analysis_id: &str,
    stock_code: &str,
    reference_price_yuan: Option<f64>,
) -> serde_json::Value {
    // ── 1. 定参考价 ────────────────────────────────────────────────────────
    let price = match reference_price_yuan {
        Some(p) if p.is_finite() && p > 0.0 => Some(p),
        _ => read_snapshot(db, analysis_id).await.and_then(|snap| snapshot_reference_price(&snap)),
    };

    // ── 2. 跑仿真（纯 CPU：无 IO、无 LLM ⇒ spawn_blocking）────────────────
    // 取不到参考价时**不跑**，但仍产出失败体，让第 3 步无条件写回
    //（理由见第 3 步注释：失败也必须覆盖，否则旧结论会冒充新结论）。
    //
    // ⚠ 所有异常都必须**产出带错误码的失败体**，不允许提前 return：
    //   提前 return 会跳过第 3、4 步 ⇒ 前端既刷新不到、也看不到失败，只能继续
    //   展示上一版结论。「静默无变化」比「明确失败」危险得多。
    let sim_value = match price {
        Some(p) => {
            let code_for_calc = stock_code.to_string();
            match tokio::task::spawn_blocking(move || {
                crate::market_sim_service::run_mc_preset(&code_for_calc, p, "stress")
            })
            .await
            {
                Ok(raw) => match serde_json::from_str::<serde_json::Value>(&raw) {
                    // 成功：宿主返回 McSimResult 序列化体（含 `simOk`）
                    Ok(v) if v.get("simOk").is_some() => v,
                    // 失败：宿主已返回 `ErrorResponse` 形状（含 `code`）⇒ 原样透传，
                    // 只补 `simOk:false`。两条路径共用同一套错误码，无需再映射一次
                    //（上一版在此把宿主的自由文本改写成自造的小写 reason，那正是
                    //  第二套错误码体系的来源）。
                    Ok(v) if v.get("code").is_some() => mark_failed(v),
                    // 既无 `simOk` 也无 `code` ⇒ 载荷无法识别。**不猜**，按解析失败记，
                    // 并把原载荷留在 detail 里供排查。
                    Ok(v) => {
                        tracing::warn!("[sim-verify] 宿主返回无法识别的载荷: {v}");
                        mark_failed(err_value(
                            sim_err::RESULT_PARSE_FAILED,
                            ErrorCategory::Unrecoverable,
                            format!("host payload has neither simOk nor code: {v}"),
                        ))
                    },
                    Err(e) => {
                        tracing::warn!("[sim-verify] 仿真结果 JSON 解析失败: {e}");
                        mark_failed(err_value(
                            sim_err::RESULT_PARSE_FAILED,
                            ErrorCategory::Unrecoverable,
                            format!("JSON parse failed: {e}"),
                        ))
                    },
                },
                // 运行时关闭导致任务被取消 ≠ 任务 panic。两者必须给不同错误码：
                // 都报「仿真执行失败」的话，用户拿到一个查不到原因的失败。
                Err(e) if e.is_cancelled() => {
                    tracing::warn!("[sim-verify] 仿真任务被取消（运行时关闭？）");
                    mark_failed(err_value(
                        sim_err::CANCELLED,
                        ErrorCategory::Retryable,
                        "spawn_blocking join cancelled",
                    ))
                },
                Err(e) => {
                    tracing::warn!("[sim-verify] 仿真任务异常终止: {e}");
                    mark_failed(err_value(
                        sim_err::EXEC_FAILED,
                        ErrorCategory::Unrecoverable,
                        format!("spawn_blocking join error: {e}"),
                    ))
                },
            }
        },
        None => mark_failed(err_value(
            sim_err::NO_REFERENCE_PRICE,
            ErrorCategory::Validation,
            "no realtime price, and no t-scoring currentPrice in snapshot",
        )),
    };

    // ── 3. 写回快照（**无条件覆盖**，失败也覆盖）──────────────────────────
    // 「无条件」是刻意的：若仅在成功时写回，`rerun_decision` 之后旧结论会留在
    // 快照里冒充新结论。因此失败同样写入 `simOk:false`，让 UI 明确显示「不可用」，
    // 而不是继续展示上一版决策的数字。
    match read_snapshot(db, analysis_id).await {
        Some(mut snap) => {
            if let Some(obj) = snap.as_object_mut() {
                obj.insert("sim-verify".to_string(), serde_json::json!({ "result": sim_value }));
            }
            match stock_analysis_snapshot::write_blackboard_snapshot(db, analysis_id, &snap).await {
                // 0 行 = 目标不存在 ⇒ **不得报「已写回」**。原实现只看 `Ok/Err`，会把
                // 「影响 0 行」打成 success 日志（日志显示成功、库里没有数据）——
                // 本文件头已立据：「静默无变化」比「明确失败」危险得多。
                Ok(0) => tracing::warn!(
                    "[sim-verify] 快照写回影响 0 行（analysis={analysis_id} 不存在？）—— 结果未落库"
                ),
                Ok(_) => tracing::info!(
                    "[sim-verify] 仿真结果已写回快照: analysis={analysis_id}, simOk={}",
                    sim_value.get("simOk").and_then(|v| v.as_bool()).unwrap_or(false)
                ),
                Err(e) => tracing::warn!("[sim-verify] 快照写回失败: {e}"),
            }
        },
        // 记录读不到：既不写回也不 emit（已经打过日志），但把结果返回给调用方，
        // 免得同步调用方以为「什么都没发生」。
        None => return sim_value,
    }

    // ── 4. emit：让已打开的分析页即时刷新，无需轮询 ────────────────────────
    if let Some(app) = app {
        if let Err(e) = app.emit(
            IpcEventName::SimulationReady.as_str(),
            serde_json::json!({
                "analysisId": analysis_id,
                "stockCode": stock_code,
                "simulation": sim_value,
            }),
        ) {
            tracing::warn!("[sim-verify] emit simulation-ready 失败: {e}");
        }
    }

    sim_value
}

/// 读 `stock_analyses.blackboard_snapshot` 并解析为 JSON 对象。
///
/// 三态语义（记录在+可解析 / 记录在+空损坏 / 记录不存在）由 dao 提供，
/// 见 `axagent_dao::repo::stock_analysis_snapshot` 模块文档。本函数只负责
/// **在命令层补日志**，然后压成调用方需要的两态：
///
/// - 记录存在但快照为空/损坏 ⇒ 空对象 `{}`（后续原样写回，不丢其他字段）；
/// - 记录不存在或查询失败 ⇒ `None`（调用方据此放弃写回）。
async fn read_snapshot(db: &DatabaseConnection, analysis_id: &str) -> Option<serde_json::Value> {
    match stock_analysis_snapshot::read_blackboard_snapshot(db, analysis_id).await {
        Ok(Some(snap)) => Some(snap),
        Ok(None) => {
            tracing::warn!("[sim-verify] 分析记录 {analysis_id} 不存在，跳过写回");
            None
        },
        Err(e) => {
            tracing::warn!("[sim-verify] 读取分析记录失败: {e}");
            None
        },
    }
}

/// 从 `blackboard_snapshot` 里取「决策当时用的现价」（单位：**元**）。
///
/// 取值路径与模板里 `sim-verify` 节点的 `input_mapping`
/// （`current_price ← t-scoring.result.content.currentPrice`）**保持同一口径** ——
/// 两处不一致会让「仿真看到的价」和「决策看到的价」对不上，结论不可对照。
///
/// key 写法按历史顺序逐个兜底（与 `decision.rs::extract_score_json` 的
/// `t-scoring` / `_raw.t-scoring` / `t-scoring.result` 三路一致）。
fn snapshot_reference_price(snap: &serde_json::Value) -> Option<f64> {
    /// 同一容器内允许的两种字段命名（Rust 侧 camelCase 为准，snake_case 兼容旧快照）。
    fn pick(v: &serde_json::Value) -> Option<f64> {
        v.get("currentPrice").or_else(|| v.get("current_price")).and_then(|x| x.as_f64())
    }

    let node = snap
        .get("t-scoring")
        .or_else(|| snap.get("_raw.t-scoring"))
        .or_else(|| snap.get("t-scoring.result"))?;
    let result = node.get("result").unwrap_or(node);

    // `content` 在新版快照里是对象，在历史版本里是 JSON 字符串 ⇒ 字符串要二次解析，
    // 否则 `content.currentPrice` 永远取不到（这是「字段在但读成空」的经典成因）。
    let parsed;
    let content = match result.get("content") {
        Some(serde_json::Value::String(s)) => {
            parsed = serde_json::from_str::<serde_json::Value>(s).ok();
            parsed.as_ref()
        },
        other => other,
    };

    content.and_then(pick).or_else(|| pick(result)).filter(|p| p.is_finite() && *p > 0.0)
}

#[cfg(test)]
mod tests {
    use super::{err_value, mark_failed, snapshot_reference_price};
    use crate::commands::error::ErrorCategory;
    use crate::commands::error_code::stock_sim as sim_err;
    use serde_json::json;

    #[test]
    fn picks_price_from_plain_camel_case_content() {
        let snap = json!({ "t-scoring": { "result": { "content": { "currentPrice": 19.01 } } } });
        assert_eq!(snapshot_reference_price(&snap), Some(19.01));
    }

    #[test]
    fn picks_price_when_content_is_json_string() {
        // 历史快照：content 是「二次编码的 JSON 字符串」
        let snap = json!({
            "t-scoring": { "result": { "content": "{\"currentPrice\": 12.34, \"totalScore\": 60}" } }
        });
        assert_eq!(snapshot_reference_price(&snap), Some(12.34));
    }

    #[test]
    fn falls_back_to_result_and_raw_key() {
        // content 缺失 ⇒ 退回 result 同级字段
        let snap = json!({ "t-scoring": { "result": { "currentPrice": 8.5 } } });
        assert_eq!(snapshot_reference_price(&snap), Some(8.5));
        // 只有 _raw. 前缀 key 的旧版快照
        let raw =
            json!({ "_raw.t-scoring": { "result": { "content": { "current_price": 7.0 } } } });
        assert_eq!(snapshot_reference_price(&raw), Some(7.0));
    }

    #[test]
    fn rejects_non_positive_and_missing_price() {
        // 0 / 负数 / 非有限值一律视为「取不到价」：仿真以 0 为参考价会算出无意义的
        // 涨跌幅（价格恒为 0 ⇒ 涨跌幅无定义），必须走 NO_REFERENCE_PRICE 降级。
        let zero = json!({ "t-scoring": { "result": { "content": { "currentPrice": 0 } } } });
        assert_eq!(snapshot_reference_price(&zero), None);
        let neg = json!({ "t-scoring": { "result": { "content": { "currentPrice": -3.0 } } } });
        assert_eq!(snapshot_reference_price(&neg), None);
        let missing = json!({ "t-scoring": { "result": { "content": { "totalScore": 60 } } } });
        assert_eq!(snapshot_reference_price(&missing), None);
        let no_node = json!({ "t-valuation": { "result": {} } });
        assert_eq!(snapshot_reference_price(&no_node), None);
    }

    #[test]
    fn failure_carries_error_code_and_no_free_text_field() {
        // 契约：失败体 = ErrorResponse 形状 + simOk:false。
        // 反向断言（`reason` / `error` 必须不存在）是刻意的 —— 这两个字段正是
        // 上一版自造的第二套错误码体系，回归时本用例会红。
        let v = mark_failed(err_value(
            sim_err::NO_REFERENCE_PRICE,
            ErrorCategory::Validation,
            "no realtime price",
        ));
        assert_eq!(v.get("simOk").and_then(|x| x.as_bool()), Some(false));
        assert_eq!(v.get("code").and_then(|x| x.as_str()), Some("STOCK_SIM_NO_REFERENCE_PRICE"));
        // category 是 snake_case（与 ErrorCategory 的 serde 配置、前端 KNOWN_CATEGORIES 一致）
        assert_eq!(v.get("category").and_then(|x| x.as_str()), Some("validation"));
        assert!(v.get("detail").is_some());
        assert!(v.get("reason").is_none(), "自造小写 reason 必须已移除");
        assert!(v.get("error").is_none(), "自由文本 error 字段必须已移除");
    }

    #[test]
    fn error_code_passes_the_ci_code_pattern() {
        // check-errorcode-alignment.mjs 用 /^[A-Z][A-Z0-9]+(?:_[A-Z0-9]+)+$/ 取码，
        // 且要求「后端码 ⊆ 11 语言 error 段」。码值不符合该形态 ⇒ 后端定义了、
        // 脚本却取不到 ⇒ 契约静默失守。此处把形态钉死在测试里。
        let re = regex_code_pattern();
        for code in [
            sim_err::INPUT_MISSING,
            sim_err::NO_REFERENCE_PRICE,
            sim_err::RESULT_PARSE_FAILED,
            sim_err::EXEC_FAILED,
            sim_err::CANCELLED,
        ] {
            assert!(re(code), "错误码 {code} 不符合 CI 取码形态");
        }
    }

    /// 与 `scripts/check-errorcode-alignment.mjs` 的 `CODE_RE` 保持逐字一致。
    fn regex_code_pattern() -> impl Fn(&str) -> bool {
        |s: &str| {
            let bytes = s.as_bytes();
            if bytes.is_empty() || !bytes[0].is_ascii_uppercase() {
                return false;
            }
            let mut prev_us = false;
            let mut has_us = false;
            for (i, b) in bytes.iter().enumerate() {
                match b {
                    b'A'..=b'Z' | b'0'..=b'9' => prev_us = false,
                    b'_' => {
                        // 首字符不能是下划线，且不允许连续下划线或尾随下划线
                        if i == 0 || prev_us {
                            return false;
                        }
                        prev_us = true;
                        has_us = true;
                    },
                    _ => return false,
                }
            }
            has_us && !prev_us
        }
    }

    #[test]
    fn mark_failed_passes_through_host_error_shape_verbatim() {
        // 宿主（run_mc_preset）已返回 ErrorResponse 形状 ⇒ 只补 simOk，
        // **不得**改写 code/category/detail。上一版正是在这里做了一次"归一化"映射，
        // 才产生出第二套小写 reason。
        let host = json!({
            "code": "STOCK_SIM_EXEC_FAILED",
            "category": "unrecoverable",
            "detail": "sim kernel exploded",
        });
        let v = mark_failed(host);
        assert_eq!(v.get("code").and_then(|x| x.as_str()), Some("STOCK_SIM_EXEC_FAILED"));
        assert_eq!(v.get("category").and_then(|x| x.as_str()), Some("unrecoverable"));
        assert_eq!(v.get("detail").and_then(|x| x.as_str()), Some("sim kernel exploded"));
        assert_eq!(v.get("simOk").and_then(|x| x.as_bool()), Some(false));
    }

    #[test]
    fn mark_failed_on_non_object_yields_identifiable_failure() {
        // 非对象载荷不可返回半成品：必须仍是一个含 code 的可识别失败体。
        let v = mark_failed(json!("kernel returned a bare string"));
        assert_eq!(v.get("simOk").and_then(|x| x.as_bool()), Some(false));
        assert_eq!(v.get("code").and_then(|x| x.as_str()), Some("STOCK_SIM_RESULT_PARSE_FAILED"));
    }
}
