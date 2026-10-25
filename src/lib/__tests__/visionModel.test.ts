// SPDX-License-Identifier: AGPL-3.0-only

import { resolveVisionModel } from "@/lib/visionModel";
import { describe, expect, it } from "vitest";

describe("resolveVisionModel", () => {
  it("显式图片模型优先于当前会话模型", () => {
    expect(
      resolveVisionModel({ a: "img-provider", b: "img-model" }, {
        providerId: "conv-provider",
        modelId: "conv-model",
      }),
    ).toEqual({ providerId: "img-provider", modelId: "img-model" });
  });

  it("未设置图片模型时回退当前会话模型", () => {
    expect(
      resolveVisionModel(null, { providerId: "conv-provider", modelId: "conv-model" }),
    ).toEqual({ providerId: "conv-provider", modelId: "conv-model" });
  });

  it("两处都空时返回空串，不编造默认模型", () => {
    expect(resolveVisionModel(undefined, null)).toEqual({ providerId: "", modelId: "" });
  });
});
