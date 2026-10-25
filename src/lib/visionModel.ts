// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 图片理解使用哪个 provider/model —— 唯一解析点。
 *
 * 优先级：设置页「图片模型」> 当前会话模型。
 * 此前 ImageAnalysisPanel 只读会话模型，导致「图片模型」设置存了没用。
 *
 * 入参取结构化宽松类型：`NullableModelRef` 的 id 是品牌类型，而本函数只做
 * 「有值则取、无值则回退」，不参与 id 构造。
 */
export function resolveVisionModel(
  imageModel: { a?: string | null; b?: string | null } | null | undefined,
  conversation: { providerId?: string | null; modelId?: string | null } | null | undefined,
): { providerId: string; modelId: string } {
  return {
    providerId: imageModel?.a ?? conversation?.providerId ?? "",
    modelId: imageModel?.b ?? conversation?.modelId ?? "",
  };
}
