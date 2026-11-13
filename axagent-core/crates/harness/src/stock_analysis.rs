// SPDX-License-Identifier: AGPL-3.0-only

//! 股票分析数据面接缝（`stock.analysis`）— Trait（DTO 留在域包）。
//!
//! 宿主把二十条**确定性取数/落库/计算/任务管理**能力经本接缝暴露给插件：
//! `industry_constituents`（按行业取全 A 股成分股）、`batch_upsert_mainlines`（按日批量
//! upsert 市场主线）、`list_recent_mainlines`（列出最近 N 天主线，#304 C3）、
//! `stock_factor`（逐票 1 月涨跌幅，反转因子原料）——E 批三条 + C3 补一条读；
//! `pipeline_history`（管道执行历史）、`pipeline_run_detail`（单次执行详情）、
//! `pending_evolution_plans`（待审批进化计划）、`des_integration`（Walk-Forward + DES
//! 对比仿真）——T2 试点批（#248）四条，插件命名命令 `stock_get_pipeline_history` /
//! `stock_get_pipeline_run_detail` / `stock_list_pending_evolution_plans` /
//! `stock_wf_des_integration` 的取数面；三族定时任务（荐股 / 趋势智选 / 候选池扫描）
//! 的 CRUD 十二条 —— 批 1a（#35），插件命名命令 `stock_create_*_cron` 等，
//! 数据面是宿主 `CronJobStore`（不落 sea-orm 表），见下方 trait 内的段落注释。
//!
//! ## 为什么需要它
//!
//! 这三个能力的实现体在 `analysis-engine`（**implementor**，持 `entities` 与
//! `DatabaseConnection`），而 `tools`（hybrid）的薄包装工具原先直接依赖它 ⇒ 构成
//! **跨包口**：一个功能包的工具体调另一个功能包的实现层。改造方向与 `opc.store`
//! （#36）同族 —— 工具只发插件命令，取数经宿主接缝往返。
//!
//! ## 为什么 payload / 出参都是 `serde_json::Value`
//!
//! 三个 op 的 DTO（`IndustryConstituentsInput` / `IndustryConstituentsReport` /
//! `BatchUpsertInput` / `BatchUpsertResult` / `StockFactorReport` / `StockFactorRow`）的
//! **权威定义在 `analysis-engine`**（implementor，随股票域最终迁进插件仓）。harness 是
//! **foundation**，按依赖铁律不得依赖任何 implementor ⇒ 这里**不能**上收这些结构体。
//! 故本接缝只承诺「JSON 形状与工具出参信封逐字一致」，形状的权威仍在域包；
//! 宿主 provider（wiring）负责 `Value` ↔ 域 DTO 的转换，转换点**唯一**。
//!
//! 这与 `opc.store` 的做法（DTO 上收 harness）不同，判据是**归属**：OPC 的 DTO 已经
//! 是接缝自有契约（没有别的权威），而股票三件各自的 DTO 都另有 implementor 权威 ——
//! 上收会制造第二份定义（禁区 12）。
//!
//! ## 权限闸
//!
//! **十条写 op**：`batch_upsert_mainlines`（改 `market_mainlines` 表：插入 / 更新 /
//! 可选归档当日未提及的主线）+ 批 1a 的九个 cron 管理写 op（3 create + 3 toggle +
//! 3 delete，增删改宿主 `CronJobStore`）。权限登记在 `worker.rs` 的
//! `required_permission_for`，本 trait 不重复校验。其余十个 op（三条 E 批读 / 三条
//! 试点读 / 三个 cron list / `list_recent_mainlines`）是纯读/纯计算面 ⇒ **一条都不许加闸**（给读缝加闸
//! 会让只读插件被无端拒掉，判据同 `opc.store` 模块头）。`des_integration` 虽是重 CPU
//! 仿真，但它不落库、不外发，读/写权限面同样不适用。

use async_trait::async_trait;
use serde_json::Value;

use crate::core_error::{AxAgentError, Result};

/// 股票分析数据面提供者（宿主独占实现：落库与行情表读面都不在插件侧）。
///
/// 实现方：宿主 wiring 层（`src-tauri/src/init/stock_analysis_service.rs`），内部调
/// `analysis-engine` 的权威实现 + 宿主 sea-orm 连接，本层不写 SQL。
/// 默认实现返回「该提供者不支持此数据面」，便于外部实现只覆盖关心的 op
/// （与 `OpcStoreProvider` / `MarketDataProvider` 同款缺省）。
///
/// ⚠ 三个 op 的入参/出参 JSON 形状**必须与工具侧原样一致**（改名即背离「工具名、
/// ToolDef、出参信封逐字不变」的改造红线）：
/// - `industry_constituents`：入参 = `IndustryConstituentsInput`（`{industries: [...]}`，
///   元素为字符串或 `{name, leaderCode?}`）；出参 = `IndustryConstituentsReport`
///   （`{groups, universeTotal, universeWithIndustry}`，camelCase）。
/// - `batch_upsert_mainlines`：入参 = `BatchUpsertInput`（camelCase 主名 +
///   snake_case alias 兼容）；出参 = `BatchUpsertResult`（`{inserted, updated, archived}`）。
/// - `stock_factor`：入参 = `{stock_codes: <宽容形态>, as_of_date?: "YYYY-MM-DD"}`
///   （provider 另收 camel 主名 `stockCodes` / `asOfDate`）—— **与工具入参逐字相同**：
///   字符串数组 / 代码对象数组 / 含 `stocks` 分组的信封。⚠ 归一（`parse_stock_refs`）
///   落在**宿主 provider**，不在插件侧：该函数属 `analysis-engine`（implementor），
///   插件 worker 只透传。出参 =
///   `StockFactorReport`（`{rows, requested, scored, missing, cutoffDate, windowRows}`）。
#[async_trait]
pub trait StockAnalysisProvider: Send + Sync {
    /// 按行业名取全 A 股成分股分组（`industry_constituents` op）—— 读 op，无权限闸。
    async fn industry_constituents(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::industry_constituents 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 按日批量 upsert 市场主线（`batch_upsert_mainlines` op）—— 写 op，要 `write` 权限。
    ///
    /// `archive_missing = true` 会把当日已有但本次未提及的主线置 `status=archived`
    /// ⇒ 不可逆的数据面动作（归档后要靠同主题再 upsert 才回到 active）。
    async fn batch_upsert_mainlines(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::batch_upsert_mainlines 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 列出最近 N 天市场主线（`list_recent_mainlines` op）—— 读 op，无权限闸。
    ///
    /// 入参 `{days?: number}`（camelCase 平铺，缺省 7；宿主 provider 拍上界），
    /// 出参 = `market_mainlines` 实体行数组（形状权威在 analysis-engine，与被退役的
    /// 宿主命令 `market_mainline_list_recent` 原样返回同一 JSON，#304 C3）。
    async fn list_recent_mainlines(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::list_recent_mainlines 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 逐票 1 月涨跌幅（`stock_factor` op）—— 读 op，无权限闸。
    async fn stock_factor(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::stock_factor 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    // ── T2 试点批（#248）：四条宿主命令的取数面 ─────────────────────────────
    // 与上面三条不同，这四条的 DTO 权威**不在 analysis-engine**，而在主壳 commands 层
    // （pipeline 行是 `core.rs` 手拼的 `json!`、进化计划是 dao 的 `EvolutionHistoryModel`、
    // DES 仿真是 `commands/wf_des.rs` 的 `WfDesRequest`/`WfDesReport`）。仍按本接缝的
    // 「Value 透传」铁律处理：形状权威在哪一侧，provider 就照哪一侧透传，本层不上收。
    //
    // 三个查询命令的入参形状与前端 `invoke` 传参逐字一致（Tauri 自动 camelCase 映射）：
    // `pipeline_history` 收 `{limit?}`、`pipeline_run_detail` 收 `{runId}`、
    // `pending_evolution_plans` 收 `{}`；`des_integration` 收 `WfDesRequest` 的 camelCase
    // JSON（provider 侧 `serde_json::from_value` 是唯一形状权威，缺料由它报 Validation）。

    /// 管道执行历史（`pipeline_history` op）—— 读 op，无权限闸。
    async fn pipeline_history(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::pipeline_history 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 单次管道执行详情（`pipeline_run_detail` op）—— 读 op，无权限闸。
    async fn pipeline_run_detail(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::pipeline_run_detail 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 待审批进化计划列表（`pending_evolution_plans` op）—— 读 op，无权限闸。
    async fn pending_evolution_plans(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::pending_evolution_plans 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// Walk-Forward + DES 对比仿真（`des_integration` op）—— 纯计算 op，无权限闸。
    ///
    /// ⚠ 重 CPU（秒级）：宿主 provider 必须 `spawn_blocking` 执行，不许在 async
    /// executor 上同步跑；实现体内部自建 tokio runtime（`commands/wf_des.rs` 的同步
    /// 命令形态），**只能**在 blocking 线程上调，否则构成嵌套 runtime。
    async fn des_integration(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::des_integration 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    // ── 批 1a（#35）：十二股票 cron 管理命令的管理面 ─────────────────────────
    // 荐股 / 趋势智选 / 候选池扫描三族定时任务的 CRUD：create / list / toggle /
    // delete 各 3 条（`stock-recommendation` / `trend-screening` / `pool-scan`）。
    // 数据面是宿主 `CronJobStore`（runtime-core），**不落 sea-orm 表**；配置归一
    // （`RecoCronConfig` / `PoolScanConfig` 默认值与校验）、desc 文案、uuid 生成、
    // task_type 标记与响应形状（`RecoCronJobResponse` / `CronJobResponse`）的权威
    // 都在主壳 commands 层（`commands/recommendation_cron.rs` / `pool_scan.rs` 的
    // 无 State 核心函数）⇒ 继续按本接缝的「Value 透传」铁律：形状权威在主壳，
    // 本层不上收（同上面四条试点 op 的判据）。
    //
    // 入参 = 命令参数的 camelCase JSON（Tauri 自动映射面，如
    // `{cronExpression, periods, minConfidence, topN}`）；`list` 收 `{}` 或缺省；
    // `toggle` 收 `{id, enabled}`；`delete` 收 `{id}`；两个 create 收各自 Config
    // 参数（缺省值补齐在核心函数）。出参 = 命令原样返回值（create/list 返回
    // 响应数组或对象，toggle/delete 返回 null）。
    //
    // ## 权限闸
    //
    // **9 写**（3 create + 3 toggle + 3 delete，增删改宿主 CronJobStore）在
    // `worker.rs` 的 `required_permission_for` 登记为 `Write`；**3 读**（list）
    // 无闸（给读缝加闸会让只读插件被无端拒掉）。

    /// 创建荐股定时任务（`create_recommendation_cron` op）—— 写 op，要 `write` 权限。
    async fn create_recommendation_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::create_recommendation_cron 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 列出荐股定时任务（`list_recommendation_crons` op）—— 读 op，无权限闸。
    async fn list_recommendation_crons(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::list_recommendation_crons 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 启停荐股定时任务（`toggle_recommendation_cron` op）—— 写 op，要 `write` 权限。
    async fn toggle_recommendation_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::toggle_recommendation_cron 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 删除荐股定时任务（`delete_recommendation_cron` op）—— 写 op，要 `write` 权限。
    async fn delete_recommendation_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::delete_recommendation_cron 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 创建趋势智选定时任务（`create_trend_screening_cron` op）—— 写 op，要 `write` 权限。
    async fn create_trend_screening_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::create_trend_screening_cron 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 列出趋势智选定时任务（`list_trend_screening_crons` op）—— 读 op，无权限闸。
    async fn list_trend_screening_crons(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::list_trend_screening_crons 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 启停趋势智选定时任务（`toggle_trend_screening_cron` op）—— 写 op，要 `write` 权限。
    async fn toggle_trend_screening_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::toggle_trend_screening_cron 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 删除趋势智选定时任务（`delete_trend_screening_cron` op）—— 写 op，要 `write` 权限。
    async fn delete_trend_screening_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::delete_trend_screening_cron 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 创建候选池扫描定时任务（`create_pool_scan_cron` op）—— 写 op，要 `write` 权限。
    async fn create_pool_scan_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::create_pool_scan_cron 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 列出候选池扫描定时任务（`list_pool_scan_crons` op）—— 读 op，无权限闸。
    async fn list_pool_scan_crons(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::list_pool_scan_crons 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 启停候选池扫描定时任务（`toggle_pool_scan_cron` op）—— 写 op，要 `write` 权限。
    async fn toggle_pool_scan_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::toggle_pool_scan_cron 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 删除候选池扫描定时任务（`delete_pool_scan_cron` op）—— 写 op，要 `write` 权限。
    async fn delete_pool_scan_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::delete_pool_scan_cron 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    // ── F1 余三条（#35）：estimation 快照读面 + 快照采集触发面 ────────────────
    // `list_financial_snapshots` 读宿主本地 `financial_snapshots` 表原始行——
    // 与 `market.quote::get_financials`（vendor 财报）**不等价**，形状权威是
    // entities 的 `financial_snapshots::Model` ⇒ 照 `list_recent_mainlines` 先例
    // 「实体行数组整块透传」，harness 不上收 DTO。`sweep_daily_snapshots` 是
    // **触发口**：执行体 `run_daily_snapshot_sweep` 留宿主（还被后台定时服务复用），
    // 长任务不搬进 worker 帧。入参 snake_case 接缝风格（同 `opc.store`）。

    /// 列估值快照原始行（`list_financial_snapshots` op）—— 读 op，无权限闸。
    ///
    /// 入参 `{"stock_code": str, "start": str?, "end": str?}`（`start`/`end` 为
    /// `YYYY-MM-DD` 快照日期闭区间，缺省不筛）；出参 = `financial_snapshots`
    /// 实体行数组（按 `snapshot_date` 升序，与被退役的宿主命令原样同形状）。
    async fn list_financial_snapshots(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::list_financial_snapshots 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 触发每日快照采集（`sweep_daily_snapshots` op）—— 写 op，要 `write` 权限。
    ///
    /// 入参 `{}`；出参 = 采集总结串（与被退役的宿主命令同形状）。归属日 =
    /// 最近一个**已完成**交易日的计算留宿主（盘中触发绝不把半日数据当收盘值落盘）。
    async fn sweep_daily_snapshots(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::sweep_daily_snapshots 未实现（该提供者不支持此数据面）".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 缺省实现必须显式报「不支持此数据面」，不得静默返回空值
    /// （空值会被工具侧读成「全部缺席但查询成功」，那是另一种谎）。
    struct Stub;

    #[async_trait]
    impl StockAnalysisProvider for Stub {}

    #[test]
    fn default_impls_reject_explicitly() {
        let stub = Stub;
        // harness 内既有异步断言一律用 `futures::executor::block_on`（见
        // `analysis_record.rs` / `opc_store.rs` 的测试），不为一条断言引入异步测试属性。
        let cases = [
            (
                "industry_constituents",
                futures::executor::block_on(stub.industry_constituents(Value::Null)),
            ),
            (
                "batch_upsert_mainlines",
                futures::executor::block_on(stub.batch_upsert_mainlines(Value::Null)),
            ),
            (
                "list_recent_mainlines",
                futures::executor::block_on(stub.list_recent_mainlines(Value::Null)),
            ),
            ("stock_factor", futures::executor::block_on(stub.stock_factor(Value::Null))),
            ("list_financial_snapshots", {
                futures::executor::block_on(stub.list_financial_snapshots(Value::Null))
            }),
            (
                "sweep_daily_snapshots",
                futures::executor::block_on(stub.sweep_daily_snapshots(Value::Null)),
            ),
            ("pipeline_history", futures::executor::block_on(stub.pipeline_history(Value::Null))),
            (
                "pipeline_run_detail",
                futures::executor::block_on(stub.pipeline_run_detail(Value::Null)),
            ),
            (
                "pending_evolution_plans",
                futures::executor::block_on(stub.pending_evolution_plans(Value::Null)),
            ),
            ("des_integration", futures::executor::block_on(stub.des_integration(Value::Null))),
            // 批 1a（#35）十二 cron 管理 op：默认实现同样必须显式报错，不得返回空值
            // （空数组会被前端读成「这类定时任务一个都没有」，那是另一种谎）。
            (
                "create_recommendation_cron",
                futures::executor::block_on(stub.create_recommendation_cron(Value::Null)),
            ),
            (
                "list_recommendation_crons",
                futures::executor::block_on(stub.list_recommendation_crons(Value::Null)),
            ),
            (
                "toggle_recommendation_cron",
                futures::executor::block_on(stub.toggle_recommendation_cron(Value::Null)),
            ),
            (
                "delete_recommendation_cron",
                futures::executor::block_on(stub.delete_recommendation_cron(Value::Null)),
            ),
            (
                "create_trend_screening_cron",
                futures::executor::block_on(stub.create_trend_screening_cron(Value::Null)),
            ),
            (
                "list_trend_screening_crons",
                futures::executor::block_on(stub.list_trend_screening_crons(Value::Null)),
            ),
            (
                "toggle_trend_screening_cron",
                futures::executor::block_on(stub.toggle_trend_screening_cron(Value::Null)),
            ),
            (
                "delete_trend_screening_cron",
                futures::executor::block_on(stub.delete_trend_screening_cron(Value::Null)),
            ),
            (
                "create_pool_scan_cron",
                futures::executor::block_on(stub.create_pool_scan_cron(Value::Null)),
            ),
            (
                "list_pool_scan_crons",
                futures::executor::block_on(stub.list_pool_scan_crons(Value::Null)),
            ),
            (
                "toggle_pool_scan_cron",
                futures::executor::block_on(stub.toggle_pool_scan_cron(Value::Null)),
            ),
            (
                "delete_pool_scan_cron",
                futures::executor::block_on(stub.delete_pool_scan_cron(Value::Null)),
            ),
        ];
        for (name, result) in cases {
            let err = result.expect_err(&format!("{name} 缺省实现应报错"));
            assert!(err.to_string().contains("未实现"), "{name} 实际: {err}");
        }
    }
}
