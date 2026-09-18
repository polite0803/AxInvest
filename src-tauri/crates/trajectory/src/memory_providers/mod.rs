// SPDX-License-Identifier: AGPL-3.0-only

pub mod closed_loop;
pub mod entity;
pub mod service;
// G21: MemoryHookProvider — 会话生命周期记忆同步 Hook（PluginHook 实现）
pub mod memory_hook_provider;
pub mod skill_summary_provider;

// [2026-09-13] 已删除 `honcho_provider` / `mem0_provider`（去重审计 P1-1）。
// 判据：两者类型均为 `pub(crate)`、`lib.rs` 无 re-export、祖先 `mod memory_providers;` 为私有
// ⇒ 对外不可达；crate 内亦零构造点 / 零类型引用 / 无字符串工厂派发（配置驱动构造亦无）。
// 且两者互为 155 行克隆（jscpd 配对第 13 名）⇒ 属「无人使用的实现被复制了一份」。
// 备份：`output/backup-2026-09-13/dup-fix/{honcho,mem0}_provider.rs.before-delete`。
// 注：若要恢复远端记忆同步能力，应先在 `init/services.rs` 建立真实构造点（当前无
// `MemoryProviderRegistry` —— `skill_summary_provider.rs` 文档中提及的该类型并不存在）。
