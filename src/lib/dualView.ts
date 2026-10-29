import type { ReactNode } from "react";

/**
 * DualView — 一个 panel 内容的两种视图:完整 panel 视图 + 紧凑 chat bubble 视图。
 * 注册到 dualViewRegistry,id 与 panelKey 对齐。
 *
 * 典型使用:
 *   registerDualView({
 *     id: "value",
 *     title: "估值评估",
 *     icon: "Banknote",
 *     defaultTab: "analyze",
 *     compact: (data) => <CompactValueAssessment data={data} />,
 *     panel: (data) => <ValueAssessmentPanel data={data} />,
 *   });
 */
export interface DualView<T = unknown> {
  id: string;
  title: string;
  icon: string; // lucide-react name,渲染时由 DualViewRenderer 动态 import
  defaultTab: "market" | "analyze" | "execute";
  compact: (data: T) => ReactNode;
  panel: (data: T) => ReactNode;
  /** 不参与 dual view 的黑名单,例如高频 ticker / 配置面板,可选 */
  noDualView?: boolean;
}

const registry = new Map<string, DualView<unknown>>();

/**
 * 判定两次注册是「同一视图的重注册」还是「真冲突」。
 *
 * 为什么需要这个判据（2026-10-29 修复）
 *
 * 原实现只看 `registry.has(id)` 就 `console.warn("duplicate registration")`，
 * 于是把**两种性质完全不同**的情形混成一条告警：
 *
 *   ① **HMR 重求值**（开发期高频）：Vite 热更新某个 `xxxDualView.tsx` 或其依赖时，
 *      该模块被重新求值 ⇒ `registerDualView` 再跑一次。此时 `registry` 里当然已有该 id，
 *      但**一切声明属性都没变**，覆盖后的视图与旧视图语义等价 ⇒ 这是正常且期望的行为
 *      （`registry.set` 本就是覆盖语义，见下方用例与 `dualView.test.ts`「后注册的覆盖」）。
 *      实测（2026-10-29）：改 `RecommendationPanel.tsx` 后控制台刷出
 *      `[dualView] duplicate registration: screener` —— 全仓 `id: "screener"` 只有一处
 *      （`recommendationDualView.tsx`），不可能真冲突，纯属 HMR 误报。
 *
 *   ② **真冲突**（配置错误）：两个不同文件都注册了同一 id，用于同一面板的两种实现。
 *      这种必须报，否则后注册者静默顶掉先注册者，`panel` 渲染的会是意料之外的组件。
 *
 * 判据为什么取「声明属性」而不是「函数引用」：HMR 重求值必然产出**新的函数引用**
 * （`compact` / `panel` 闭包重建），拿引用比对会把 ① 全判成 ② —— 反过来，声明属性
 * （`title` / `icon` / `defaultTab` / `noDualView`）都是**静态字面量**，HMR 前后恒等，
 * 而真冲突的两份实现必然至少有一项不同。因此三者**全等 ⇒ 幂等覆盖（静默）**，
 * 任一不等 ⇒ 真冲突（告警）。
 *
 * ⚠️ 不要退回「只告警一次（用 Set 记已报 id）」的折中方案：那会让真正的双文件冲突
 * 在第二次改动之后再也看不见，而噪音只是从 N 条降到 1 条 —— 治了症状没治根因。
 */
function isSameRegistration(a: DualView<unknown>, b: DualView<unknown>): boolean {
  return a.title === b.title
    && a.icon === b.icon
    && a.defaultTab === b.defaultTab
    && a.noDualView === b.noDualView;
}

export function registerDualView<T>(view: DualView<T>): void {
  const existing = registry.get(view.id);
  // 已注册且声明属性不同 ⇒ 真冲突；同属性重注册（HMR / 幂等重复 import）⇒ 静默覆盖
  if (existing && !isSameRegistration(existing, view as DualView<unknown>)) {
    console.warn(`[dualView] duplicate registration: ${view.id}`);
  }
  registry.set(view.id, view as DualView<unknown>);
}

export function getDualView(id: string): DualView | undefined {
  return registry.get(id);
}

export function listDualViews(): DualView[] {
  return Array.from(registry.values());
}

export function isDualViewEnabled(id: string): boolean {
  const v = registry.get(id);
  return !!v && !v.noDualView;
}

/** 测试用:清空注册表。仅在 __test__ 环境调用。 */
export function _resetDualViewRegistry(): void {
  registry.clear();
}
