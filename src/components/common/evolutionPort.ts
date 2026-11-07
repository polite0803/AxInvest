// SPDX-License-Identifier: AGPL-3.0-only

// 进化能力注入口（页级 slot，任务 #220 组件中立性消键）。
//
// 背景：`src/components/workflow/**` 此前直接 import `@/stores/feature/evolutionStore`，
//   owner=workflow ≠ evolution ⇒ 命中中立性门禁的跨 feature store 键。
//   本契约把「进化能力」抽象为一个由**页级**（WorkflowPage，pages 不受中立性约束）
//   绑定 store 后逐层透传（WorkflowEditor → AIPanel → EvolutionTab）的端口对象，
//   使 workflow 组件不再直接依赖 evolution feature store。
//
// 类型来源：DTO 类型经 `@/stores` 聚合 barrel 的**纯类型**导出取得（stores/index.ts 已
//   `export type { ABTestResult, EngineStatus, SkillVersion, ... }`）。`import type` /
//   `export type` 在编译期被完全擦除 ⇒ 本文件零运行时依赖，不引入任何 store 运行时耦合；
//   中立性扫描器只判 `src/stores/feature/<B>Store` 直引，聚合 barrel 不在其判据面内。

import type { ABTestResult, EngineStatus, SkillVersion } from "@/stores";

/**
 * 进化能力端口：由 WorkflowPage 从 evolutionStore 绑定后注入。
 * 所有方法均为 store 同名能力的透传；组件侧只消费本契约，不感知 store。
 */
export interface EvolutionPort {
  /** 进化引擎状态快照（AIPanel 推荐上下文富化用）。 */
  engines: Record<string, EngineStatus>;
  /** 取指定技能的进化版本历史（任务 #218 批一异步化；批二接持久化轨迹，当前恒返回 []）。 */
  getSkillEvolutionHistory: (skillId: string) => Promise<SkillVersion[]>;
  /** 取指定技能的 A/B 测试结果（任务 #218 批一接真 evaluator_get_ab_results；空 skillId ⇒ 空态）。 */
  getABTestResults: (skillId: string) => Promise<ABTestResult[]>;
  /** 触发指定技能进化（fire-and-forget，内部异步执行）。 */
  triggerSkillEvolution: (skillId: string) => void;
}

export type { ABTestResult, EngineStatus, SkillVersion };
