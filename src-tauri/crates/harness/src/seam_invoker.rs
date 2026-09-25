// SPDX-License-Identifier: AGPL-3.0-only

//! 跨边界接缝调用面（B 层动态插件）—— **只约定「JSON 进、JSON 出」，不约定传输方式**。
//!
//! ## 为什么需要这一层
//!
//! 进程内实现（A 层）可以直接把 `Arc<dyn Xxx>` 存进能力注册表；但**跨进程**的插件
//! 无法把 Rust trait object 送过边界（Rust 无稳定 ABI）。故 B 层插件统一退化为
//! 「一个 op 名 + 一段 JSON 参数 → 一段 JSON 返回值」的调用面，由本 trait 表达。
//!
//! ## 传输无关
//!
//! 本 trait **不规定** JSON 怎么过界：
//! - 主路径：长驻 worker 子进程 + stdin/stdout 上的长度前缀 JSON 帧（`axagent-plugin-proto`）
//! - 备选路径：cdylib 的 C-ABI 函数指针
//!
//! 二者只在构造 [`SeamInvoker`] 的实现时不同 —— 于是「远程门面」与「接缝注册表」
//! 无需感知传输形态，内置实现与远程实现在注册表里**是同一个类型**（`Arc<dyn Xxx>`），
//! 消费方零改动（详见 `PLAN-everything-is-plugin.md` §4.0 / §5.0 / §15.4）。

use serde_json::Value;

/// 跨边界插件调用的统一调用面。
///
/// 实现方负责把一次调用编成帧 / 字节缓冲送出去，并把对端结果解回 JSON。
///
/// **失败语义**：对端崩溃、通道 EOF、协议错误一律返回 `Err`（不 panic 宿主）——
/// 进程边界因此天然获得崩溃隔离。
pub trait SeamInvoker: Send + Sync {
    /// 调用插件的一个 op；`args` 与返回值均为 JSON。
    ///
    /// `op` 的取值由各接缝的映射表约定（如 `workflow.sandbox` → `"execute"`），
    /// 另有一组协议级 op（`describe` / `event` / `emit` / `call_seam` / `shutdown`）。
    fn invoke(&self, op: &str, args: Value) -> Result<Value, String>;

    /// 带**调用链**的调用面上（L2 跨插件调用，`PLAN-everything-is-plugin.md` §15.4）。
    ///
    /// ## 为什么链走参数而不是 thread-local
    ///
    /// 环检测必须在**目标端**做（宿主经注册表只能拿到 `Arc<dyn Xxx>` 门面，认不出背后是哪个
    /// 插件），于是链必须真的穿过那层类型擦除的门面 —— 而 thread-local 无法穿过
    /// `Arc<dyn AgentTurnRunner> → Arc<dyn SeamInvoker>` 这条链路（异步门面还要跨
    /// `spawn_blocking` 换线程，thread-local 当场丢失）。故链作为入参显式传递。
    ///
    /// **默认实现**委托 `invoke`（链被忽略）—— 进程内实现无需感知调用链；
    /// 只有跨进程的 `WorkerInvoker` 覆盖它做环/深度自检。
    fn invoke_with_chain(
        &self,
        op: &str,
        args: Value,
        _call_chain: Vec<String>,
    ) -> Result<Value, String> {
        self.invoke(op, args)
    }
}
