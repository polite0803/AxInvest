// SPDX-License-Identifier: AGPL-3.0-only

/**
 * As-Of（时间旅行）降级类型 —— 前端中立层。
 *
 * 权威定义在后端 `axagent-core/crates/harness/src/as_of.rs` 的
 * `DegradationKind` / `DegradationEntry`：它属 harness 基座契约，**不在股票域内**
 * —— `market_data.rs` / `trading_engine.rs` 两个跨域契约都以 `AsOfContext` 为末参，
 * 降级留痕则是「所有受 as_of 约束的取数」的通用副产品（各类数据源都会记）。
 *
 * 放在 `src/types/` 顶层的理由：消费方横跨 `stores/shared/timeAnchorStore`（跨组件
 * 共享层）、`stores/feature/serenityStore`（股票域）与 `components/time-travel`
 * —— 三方都不该为拿一个类型去 import `@/types/stock-analysis` 这个单域门面
 * （那正是 §132.3 ③ 判出的层次倒挂）。故按「跨域共享 DTO 归域本身、中性 DTO 归顶层」
 * 的同一判据上收到此文件，不进宿主 barrel（与 `./timeTravel` 不同：后者是单域 DTO）。
 */

/** 严重度分档；与后端 `DegradationKind` 的 serde 输出逐值对齐（改一边必须改另一边） */
export type AsOfDegradationKind = "failure" | "noData" | "structuralGap";

/** `get_asof_degradation_log` 返回的一条记录 */
export interface AsOfDegradationEntry {
  vendor: string;
  method: string;
  reason: string;
  /** 该 DTO 是 wire snake_case 的既成例外（后端字段就叫 as_of），非本仓 camelCase 标准 */
  as_of: string;
  kind: AsOfDegradationKind;
}
