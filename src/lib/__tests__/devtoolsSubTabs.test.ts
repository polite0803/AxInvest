// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import {
  buildDevToolsSubSearch,
  DEFAULT_DEVTOOLS_SUB,
  DEVTOOLS_SUB_PATHS,
  DEVTOOLS_SUBS,
  isDevToolsSub,
  parseDevToolsSub,
} from "../devtoolsSubTabs";
import { BUILTIN_PAGE_PATH } from "../pageRegistry";

describe("devtoolsSubTabs — 子页契约", () => {
  it("默认子页必须存在于子页表内（否则 DevToolsPage 会渲染出无 activeKey 的空面板）", () => {
    expect(DEVTOOLS_SUBS).toContain(DEFAULT_DEVTOOLS_SUB);
  });

  it("每个子页都有唯一的历史路由路径", () => {
    const paths = DEVTOOLS_SUBS.map((sub) => DEVTOOLS_SUB_PATHS[sub]);
    for (const path of paths) {
      expect(path).toMatch(/^\/devtools\//);
    }
    expect(new Set(paths).size).toBe(paths.length);
  });

  it("子页路径必须等于 pageRegistry 的常量（防两处各自漂移）", () => {
    expect(DEVTOOLS_SUB_PATHS["trace-explorer"]).toBe(BUILTIN_PAGE_PATH.devtoolsTraceExplorer);
    expect(DEVTOOLS_SUB_PATHS.benchmark).toBe(BUILTIN_PAGE_PATH.devtoolsBenchmark);
    expect(DEVTOOLS_SUB_PATHS["tool-recommender"]).toBe(BUILTIN_PAGE_PATH.devtoolsToolRecommender);
    expect(DEVTOOLS_SUB_PATHS["fine-tune"]).toBe(BUILTIN_PAGE_PATH.devtoolsFineTune);
    expect(DEVTOOLS_SUB_PATHS["rl-training"]).toBe(BUILTIN_PAGE_PATH.devtoolsRlTraining);
  });

  it("历史路由路径互不相同（否则后面那条会被前一条永久遮蔽）", () => {
    const all = [BUILTIN_PAGE_PATH.devtools, ...DEVTOOLS_SUBS.map((s) => DEVTOOLS_SUB_PATHS[s])];
    expect(new Set(all).size).toBe(all.length);
  });
});

describe("devtoolsSubTabs — isDevToolsSub 脏值防线", () => {
  it("接受全部合法子页", () => {
    for (const sub of DEVTOOLS_SUBS) {
      expect(isDevToolsSub(sub)).toBe(true);
    }
  });

  it("拒绝非法值（URL 手改 / 旧版已删子页 / 非字符串）", () => {
    for (const bogus of ["__bogus__", "", "benchmark ", "BENCHMARK", null, undefined, 5, {}]) {
      expect(isDevToolsSub(bogus)).toBe(false);
    }
  });
});

describe("devtoolsSubTabs — parseDevToolsSub", () => {
  it("解析合法 ?sub=", () => {
    expect(parseDevToolsSub("?ws=devtools&sub=benchmark")).toBe("benchmark");
    expect(parseDevToolsSub("?sub=rl-training&ws=devtools")).toBe("rl-training");
  });

  it("无 sub 或脏 sub 时返回 null（**不得**回落默认值，由调用方决定）", () => {
    expect(parseDevToolsSub("")).toBeNull();
    expect(parseDevToolsSub("?ws=devtools")).toBeNull();
    expect(parseDevToolsSub("?sub=__bogus__")).toBeNull();
  });
});

describe("devtoolsSubTabs — buildDevToolsSubSearch", () => {
  it("只改 sub，保留 ws（丢掉 ws 会让工作台落到别的 Tab）", () => {
    const params = new URLSearchParams(buildDevToolsSubSearch("?ws=devtools&sub=benchmark", "fine-tune"));
    expect(params.get("ws")).toBe("devtools");
    expect(params.get("sub")).toBe("fine-tune");
  });

  it("空查询串也能生成 sub", () => {
    expect(buildDevToolsSubSearch("", "benchmark")).toBe("sub=benchmark");
  });
});
