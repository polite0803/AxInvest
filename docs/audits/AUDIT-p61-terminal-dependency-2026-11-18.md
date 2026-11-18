# AUDIT — P6.1 终局依赖断边时点快照（2026-11-18）

> 状态：active ｜ 日期：2026-11-18 ｜ 对应版本：v3.0.1
> 关联：`PLAN-shell-plus-plugins.md`，`AUDIT-p61-completion-2026-11-18.md`
> 性质：时点快照。记录两次已提交、未推送的机械实测；历史快照正文不回写。

## 一、结论

P6.1 终局四项尚未全部达标。当前进度是：**宿主插件内 `stock-core` 依赖已真实断开，宿主股票 crate 直接依赖由 6 降至 5，构建边由 10 降至 9**；插件 E2E 的 staging 不再允许 `SKIP` 冒充通过。

## 二、本轮落地的真实变化

1. `7c823d63e`：把 `nextest` 前置 staging 收敛成单一规格，新增 20 项自检；补齐 `AXAGENT_STOCK_WORKER_BIN`；8 个受影响 E2E 全部真执行，0 条 `SKIP`。同时修掉三处被 SKIP 长期掩盖的陈旧断言（security 域包页、stock 数据切片页、stock 两个任务源）。
2. `d39171790`：意图解析、因子、成本、命中判定四个纯函数模块上收 `axagent-harness`，`stock-core` 仅保留 re-export；`src-tauri/Cargo.toml` 摘除 `axagent-stock-core`，离线 bin 经 harness 门面取用。

## 三、取证读数

- `node scripts/audit-l1-blockers.mjs`：`Cargo.toml 构建边` STOCK = **9**，OPC = 1；Rust 反向阻塞仍为 0/0；前端反向阻塞仍为 12/10。
- `rg -n '^axagent-(analysis-engine|astock-data|company-runtime|market-sim|quant|stock-core)' src-tauri/Cargo.toml`：5 条，分别对应 `analysis-engine` / `astock-data` / `company-runtime` / `market-sim` / `quant`。
- `cargo check --manifest-path src-tauri/Cargo.toml -p axagent --bins`：EXIT=0，证明宿主与两支离线 bin 在摘掉 `stock-core` 后仍可编译。
- `cargo test --manifest-path axagent-core/Cargo.toml -p axagent-harness --lib`：1016 passed / 0 failed / 1 ignored。
- `cargo clippy --manifest-path axagent-plugins/Cargo.toml -p axagent-stock-core --all-targets -- -D warnings`：EXIT=0。
- `node scripts/check-capability-register.mjs`：EXIT=0，K1 文件数重锚至 2064。

## 四、仍未完成的终局判据

1. 宿主仍直接依赖 5 条股票相关 crate；完整“卸载股票后其余 1099 条命令可用”尚未实测。
2. 真机 GUI 点击与生产历史库逐表对比尚未完成。
3. `company-runtime` 仍有 11 个宿主文件、56 处引用；`analysis-engine` 37 文件、337 处引用；不能靠门面改名断边。
4. `market_sim` / `quant` 的宿主用途是 DES / Walk-Forward 编排，现有 `market_sim_service` 仍在宿主图内，单靠上收命令层不能断 Cargo 边。

## 五、推进次序建议

下一批应先处理 `company-runtime` 的宿主持有面（opc_store provider、LocalCapabilityPackRegistry 装配位），或把 `market_sim` 核心编排整体移出主壳；两者都需要搬真实实现，不以降 grep 读数为目标。
