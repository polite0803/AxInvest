// SPDX-License-Identifier: AGPL-3.0-only

//! 工具 → 插件命名命令的桥调用（`ToolContext::plugin_command_bridge` 的唯一入口）。
//!
//! 工作流工具与宿主直连时代码是「import 域 crate → 取 DB → 调函数」。迁出到插件后
//! （#36 OPC / #240 股票），工具侧既不该 import 域包（consumer 不得依赖 implementor），
//! 也不该自己取 DB —— 数据面一律经插件命令 → 宿主接缝。
//!
//! **fail-closed**：桥未注入时不回落任何直连路径。回落会把「插件未启用 / 桥没接」
//! 伪装成「数据为空」，那是最难查的一类静默降级（`opc.rs` 的先例同判据）。
//!
//! 错误形态：`Err(String)` 由调用方按自己的语义包装（工具面要区分「入参不合规」与
//! 「执行失败」，所以本层不预先归类）。

use crate::ToolContext;
use serde_json::Value;

/// 调 `plugin_id` 的命名命令 `command`，入参原样透传。
///
/// **本层不改名**（形状转换点必须唯一：插件侧或宿主 provider）。`plugin_id` 必须与插件
/// 清单的 `name` 逐字一致（`"stock"` / `"opc"`），因为 `PluginManager` 按它 find。
///
/// ⚠ 例外归调用方而非本层：插件命令闸（`plugins/src/manager.rs` 的
/// `check_command_input_shape`）逐键校验清单 `inputSchema.required`、**不认 serde alias**
/// ⇒ 若某工具的 schema 承诺收 camelCase 别名，它必须在**送桥之前**把别名收敛成清单主名
/// （先例见 `tools/market_mainline.rs` 的 `normalize_outer_aliases`）。那是「让载荷满足
/// 插件的 required」而不是给域 DTO 造第二套形状，故不冲突上面这条。
pub(crate) async fn call_plugin_command(
    ctx: &ToolContext,
    plugin_id: &str,
    command: &str,
    input: Value,
) -> Result<Value, String> {
    let bridge = ctx
        .plugin_command_bridge
        .as_ref()
        .ok_or_else(|| format!("{command}: 插件命令桥未注入（fail-closed，不回落直连）"))?;
    bridge.execute(plugin_id, command, input).await
}
