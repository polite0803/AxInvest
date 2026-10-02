// SPDX-License-Identifier: AGPL-3.0-only

import type { ModelIconProps } from "@lobehub/icons";
import { lazy, Suspense } from "react";

/**
 * `@lobehub/icons` 桶在 dev 预打包后是 332 个品牌图标组件（实测 `@lobehub/icons.js`
 * 23.3 MB + 其共享分片 10.3 MB + `Tavily` 1.7 MB）。任何一处**静态** import 都会把它
 * 整包挂进 DOMContentLoaded 之前的模块图；而主窗口以 `visible: false` 启动、要等
 * `AppInitializer` 首帧后才 `show()`，于是这 35 MB 全部计入「首屏弹出」的等待时间。
 *
 * 这里改成动态 import：图标晚一个 microtask 出现，加载中用等尺寸空块占位
 * （与 `DynamicLobeIcon` 的加载态同一约定，不引起布局跳动）。
 */
const LazyLobeModelIcon = lazy(() => import("@lobehub/icons").then((m) => ({ default: m.ModelIcon })));

export function ModelIcon(props: ModelIconProps) {
  const size = props.size ?? 12;
  return (
    <Suspense fallback={<div style={{ width: size, height: size }} />}>
      <LazyLobeModelIcon {...props} />
    </Suspense>
  );
}
