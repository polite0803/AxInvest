// SPDX-License-Identifier: AGPL-3.0-only
import type { SubWorkflowOrigin } from "@/types";
import * as fs from "node:fs";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import { attributeWorkflowEvent, mergeSubFlowStatus, replaceSubFlowStatuses } from "../workEngineStore";

const ROOT = path.resolve(__dirname, "../../../..");

const ORIGIN: SubWorkflowOrigin = {
  parentExecutionId: "PARENT",
  parentNodeId: "sw-tier-mid",
};

describe("attributeWorkflowEvent —— 子执行事件归属判定", () => {
  it("本执行自己的事件走原路径（父节点状态表）", () => {
    expect(attributeWorkflowEvent({ execution_id: "PARENT" }, "PARENT")).toEqual({ scope: "self" });
  });

  it("缺任一 id 时按原语义放行到 self（不因新逻辑收紧而丢旧事件）", () => {
    expect(attributeWorkflowEvent({}, "PARENT")).toEqual({ scope: "self" });
    expect(attributeWorkflowEvent({ execution_id: "CHILD" }, null)).toEqual({ scope: "self" });
  });

  it("本执行的子执行 ⇒ 归到 child 桶，带出父节点 id 与子执行 id", () => {
    expect(
      attributeWorkflowEvent({ execution_id: "CHILD-1", sub_workflow_origin: ORIGIN }, "PARENT"),
    ).toEqual({ scope: "child", origin: ORIGIN, childExecutionId: "CHILD-1" });
  });

  // 反向锁：守卫不能改成「全收」，也不能把无归属的事件猜进某个桶。
  it("别人的执行（无归属）必须仍被丢弃", () => {
    expect(attributeWorkflowEvent({ execution_id: "OTHER" }, "PARENT")).toBeNull();
  });

  it("带归属但父不是本执行 ⇒ 丢弃，不得回退成「归第一个子执行」", () => {
    expect(
      attributeWorkflowEvent(
        {
          execution_id: "CHILD-1",
          sub_workflow_origin: { parentExecutionId: "NOT-ME", parentNodeId: "sw-x" },
        },
        "PARENT",
      ),
    ).toBeNull();
  });

  it("null 归属（顶层事件的字段缺席形态）⇒ 走 self，不建桶", () => {
    expect(
      attributeWorkflowEvent({ execution_id: "PARENT", sub_workflow_origin: null }, "PARENT"),
    ).toEqual({ scope: "self" });
  });
});

describe("subFlow 桶 —— 父子同 id 不互相覆盖", () => {
  it("两个父节点各跑同一张子模板 ⇒ 同名子节点分落两桶，互不覆盖", () => {
    // 这正是「四档子工作流」的形态：同一张模板跑 4 次 ⇒ 子图里的 trigger/end 全同名。
    // 若按 node_id 平铺进一张表，四档就只剩最后一档。
    let buckets = mergeSubFlowStatus({}, ORIGIN, "CHILD-MID", "trigger", "completed");
    buckets = mergeSubFlowStatus(
      buckets,
      { parentExecutionId: "PARENT", parentNodeId: "sw-tier-long" },
      "CHILD-LONG",
      "trigger",
      "running",
    );
    expect(buckets["CHILD-MID"].nodeStatuses.trigger).toBe("completed");
    expect(buckets["CHILD-LONG"].nodeStatuses.trigger).toBe("running");
    expect(buckets["CHILD-MID"].parentNodeId).toBe("sw-tier-mid");
    expect(buckets["CHILD-LONG"].parentNodeId).toBe("sw-tier-long");
  });

  it("同一父节点重试出多个子执行 ⇒ 两个桶并存，后一次不覆盖前一次", () => {
    let buckets = mergeSubFlowStatus({}, ORIGIN, "CHILD-1", "n1", "failed");
    buckets = mergeSubFlowStatus(buckets, ORIGIN, "CHILD-2", "n1", "completed");
    expect(Object.keys(buckets).sort()).toEqual(["CHILD-1", "CHILD-2"]);
    expect(buckets["CHILD-1"].nodeStatuses.n1).toBe("failed");
    expect(buckets["CHILD-2"].nodeStatuses.n1).toBe("completed");
  });

  it("同一子执行的后续事件是增量合并，不是整表替换", () => {
    let buckets = mergeSubFlowStatus({}, ORIGIN, "CHILD-1", "n1", "running");
    buckets = mergeSubFlowStatus(buckets, ORIGIN, "CHILD-1", "n2", "running");
    expect(buckets["CHILD-1"].nodeStatuses).toEqual({ n1: "running", n2: "running" });
  });

  it("state-changed 的全量快照覆盖桶内状态，但不动别的桶", () => {
    let buckets = mergeSubFlowStatus({}, ORIGIN, "CHILD-1", "n1", "running");
    buckets = mergeSubFlowStatus(buckets, { ...ORIGIN, parentNodeId: "sw-a" }, "CHILD-2", "n9", "running");
    buckets = replaceSubFlowStatuses(buckets, ORIGIN, "CHILD-1", [
      { node_id: "n1", status: "completed" },
      { node_id: "n2", status: "failed" },
    ]);
    expect(buckets["CHILD-1"].nodeStatuses).toEqual({ n1: "completed", n2: "failed" });
    expect(buckets["CHILD-2"].nodeStatuses).toEqual({ n9: "running" });
    expect(buckets["CHILD-1"].parentNodeId).toBe("sw-tier-mid");
  });

  it("replaceSubFlowStatuses 传空数组 ⇒ 桶存在且状态表为空（不是把桶删掉）", () => {
    let buckets = mergeSubFlowStatus({}, ORIGIN, "CHILD-1", "n1", "running");
    buckets = replaceSubFlowStatuses(buckets, ORIGIN, "CHILD-1", []);
    expect(buckets["CHILD-1"]).toEqual({
      parentNodeId: "sw-tier-mid",
      childExecutionId: "CHILD-1",
      nodeStatuses: {},
    });
  });
});

/**
 * 反手抄：消费端读的是 camelCase，生产端（Rust DTO + commands 层载荷键）必须同批改。
 * 任一侧单独漂移 ⇒ 这里红，而不是界面上静默「子节点又看不见了」。
 */
describe("生产端拼写锁", () => {
  const CMD = path.join(ROOT, "src-tauri/src/commands/work_engine.rs");
  const DTO = path.join(ROOT, "src-tauri/crates/harness/src/workflow_types.rs");

  it("commands 层两条载荷都发 sub_workflow_origin", () => {
    const body = fs.readFileSync(CMD, "utf8");
    const occurrences = body.match(/"sub_workflow_origin"/g) ?? [];
    expect(occurrences.length).toBeGreaterThanOrEqual(2);
  });

  it("state-changed 载荷的归属取自 full_state（真实状态，不是 per-node ctx）", () => {
    const body = fs.readFileSync(CMD, "utf8");
    expect(body).toContain("full_state.sub_workflow_origin");
  });

  it("DTO 走 camelCase ⇒ 推导出消费端应读的键名，并要求 store 源码逐个用到", () => {
    const src = fs.readFileSync(DTO, "utf8");
    const block = src.slice(
      src.indexOf("pub struct SubWorkflowOrigin"),
      src.indexOf("}", src.indexOf("pub struct SubWorkflowOrigin")),
    );
    expect(block).not.toBe("");
    const snake = [...block.matchAll(/pub (\w+):/g)].map((m) => m[1]);
    expect(snake.length).toBeGreaterThanOrEqual(2);
    const camel = snake.map((s) => s.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase()));
    // rename_all 没写 ⇒ camel 键是凭空断言
    const head = src.slice(0, src.indexOf("pub struct SubWorkflowOrigin"));
    expect(head.slice(-400)).toContain('rename_all = "camelCase"');
    const store = fs.readFileSync(path.join(ROOT, "src/stores/feature/workEngineStore.ts"), "utf8");
    for (const key of camel) {
      // 按**取值形态**断言：裸名会在别处（node_records 的 parentExecutionId 映射）先命中，
      // 那样这条断言就恒真了 —— 门不能这样建。
      expect(store, `store 必须读 origin.${key}`).toContain(`origin.${key}`);
    }
  });
});

describe("11 语言真译", () => {
  const KEYS = ["subFlowLive", "subFlowPending"] as const;
  const files = fs
    .readdirSync(path.join(ROOT, "src/i18n/locales"))
    .filter((f) => f.endsWith(".json"));

  it.each(KEYS)("每个语言都有 %s 且不是英文原样/空串", (key) => {
    expect(files.length).toBe(11);
    const seen = new Set<string>();
    for (const f of files) {
      const json = JSON.parse(fs.readFileSync(path.join(ROOT, "src/i18n/locales", f), "utf8"));
      const value = json.debugPanel?.[key];
      expect(typeof value, `${f} 缺 debugPanel.${key}`).toBe("string");
      expect((value as string).trim().length).toBeGreaterThan(0);
      if (key === "subFlowLive") {
        expect(value, `${f} 的 {{count}} 占位符破损`).toContain("{{count}}");
      }
      if (f !== "en-US.json") {
        seen.add(value as string);
      }
    }
    // 非英语语言不得全部照抄英文（同一措辞在两种语言碰巧相同是允许的，故只查「全同」）
    const en = JSON.parse(
      fs.readFileSync(path.join(ROOT, "src/i18n/locales/en-US.json"), "utf8"),
    ).debugPanel[key] as string;
    if (seen.size === 1 && [...seen][0] === en) {
      throw new Error(`${key}: 所有非英语语言都是英文占位`);
    }
  });
});
