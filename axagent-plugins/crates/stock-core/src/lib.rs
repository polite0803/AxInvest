// SPDX-License-Identifier: AGPL-3.0-only
//! 股票域**纯逻辑**（foundation）。
//!
//! 为什么单独成 crate 而不是留在 `analysis-engine`：插件侧 worker 只能依赖 foundation
//! （零 `axagent-*` 依赖），而 `analysis-engine` 是 implementor —— 插件链接它会把
//! `dao` / `entities` / `astock-data` 一起拖进插件进程。把纯函数放在这一层，
//! 宿主与插件就能链接**同一份实现**（不是复制粘贴的第二份词表）。
//!
//! ⚠ 本 crate 只收纯函数：不查库、不读文件、不取时钟、不碰 AppState；
//! 需要宿主状态或外部数据的能力走接缝，不进这里。

pub mod intent_parser;
