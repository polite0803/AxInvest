// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from "vitest";
import { invalidate, onInvalidate } from "../cacheInvalidation";

/**
 * 中性失效广播的契约（B8-4a，`PLAN-shell-plus-plugins.md` §109）。
 *
 * 它替掉的是「宿主直接 import 股票域 `clearVendorCheckCache` 并调它」——
 * 因此这里必须钉住三件事：订阅者被调用、取消订阅后不再被调用、
 * 单个订阅者抛错不影响其余（否则一个坏订阅者会让整条失效链断掉）。
 */

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
});

describe("cacheInvalidation", () => {
  it("广播命中该主题的全部订阅者", () => {
    const a = vi.fn();
    const b = vi.fn();
    const stopA = onInvalidate("vendor-config", a);
    const stopB = onInvalidate("vendor-config", b);

    invalidate("vendor-config");

    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(1);
    stopA();
    stopB();
  });

  it("取消订阅后不再被调用", () => {
    const fn = vi.fn();
    const stop = onInvalidate("vendor-config", fn);
    stop();

    invalidate("vendor-config");

    expect(fn).not.toHaveBeenCalled();
  });

  it("单个订阅者抛错不阻断其余订阅者", () => {
    const bad = vi.fn(() => {
      throw new Error("boom");
    });
    const good = vi.fn();
    const stopBad = onInvalidate("vendor-config", bad);
    const stopGood = onInvalidate("vendor-config", good);

    expect(() => invalidate("vendor-config")).not.toThrow();
    expect(good).toHaveBeenCalledTimes(1);

    stopBad();
    stopGood();
  });
});
