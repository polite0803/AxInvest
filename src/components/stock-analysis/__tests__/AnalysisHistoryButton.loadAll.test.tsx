import i18n from "@/i18n";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { App } from "antd";
import { readFileSync } from "node:fs";
import path from "node:path";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { AnalysisHistoryButton } from "../AnalysisHistoryButton";

/**
 * 历史分析弹窗「全量加载 + 筛选」的接线层。
 *
 * 背景（2026-10-22，用户报「对之前的记录存在截断」）：弹窗原先固定
 * `invoke("list_stock_analyses", { limit: 30 })`，而库里实测 360 条 / 103 只 ⇒ 只看得到 8%。
 * 本轮改成「打开即按 offset 连续翻页取全 + 关键词/状态/档位三维筛选」。
 *
 * 每条断言各守一格（破坏哪格就只红哪条）：
 *   取全（offset 序列 + 条数读数）、筛选（分子/分母）、筛空声明、取数失败声明、撞上限声明。
 * 呈现层用真 i18n（zh-CN）⇒ 断的是**译文**，不只是 key 接线。
 */
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: () => Promise.resolve(() => {}),
  isTauri: () => false,
  TimeoutError: class MockTimeoutError extends Error {},
}));

const ROOT = path.resolve(import.meta.dirname, "..", "..", "..", "..");
const COMPONENT = "src/components/stock-analysis/AnalysisHistoryButton.tsx";

/** 单页 500 条要与组件里的 PAGE_SIZE 对齐，测试里同样写死一份只为造夹具，不参与判定。 */
const PAGE = 500;

type Row = Record<string, unknown>;

function row(id: string, over: Partial<Row> = {}): Row {
  return {
    id,
    stockCode: "600519",
    stockName: "贵州茅台",
    analysisDate: "2026-10-01",
    status: "completed",
    decisionAction: "BUY",
    decisionPositionPct: 10,
    decisionPositionState: null,
    decisionJson: null,
    createdAt: 1_760_000_000_000,
    parentAnalysisId: null,
    templateId: "stock-analysis",
    decisionTimeHorizon: "mid",
    decisionHorizonSource: "formula",
    templateVersion: 160,
    // 来历两列给**生产形态**（列 NOT NULL；live 行也带 as_of_date，现网 283 行全部非空）
    analysisKind: "live",
    asOfDate: "2026-10-01",
    ...over,
  };
}

function rows(count: number, from = 0): Row[] {
  return Array.from({ length: count }, (_, i) => row(`r${from + i}`));
}

/**
 * 用一批「页」编排 `list_stock_analyses` 的返回：每次调用消费一页，耗尽后返回空页。
 * 同时记录调用收到的 offset 序列（截断缺陷的直接读数）。
 */
function scriptPages(pages: Row[][]) {
  const offsets: number[] = [];
  let cursor = 0;
  invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
    if (cmd === "get_stock_template_version_status") {
      return Promise.resolve({ codeVersion: 160, dbVersion: 160 });
    }
    if (cmd === "list_stock_analyses") {
      offsets.push(Number(args?.offset));
      const page = pages[cursor++] ?? [];
      return Promise.resolve(page);
    }
    return Promise.resolve(null);
  });
  return offsets;
}

async function openPopup() {
  render(
    <MemoryRouter>
      <I18nextProvider i18n={i18n}>
        <App>
          <AnalysisHistoryButton />
        </App>
      </I18nextProvider>
    </MemoryRouter>,
  );
  await act(async () => {});
  fireEvent.click(screen.getByRole("button", { name: /历史分析/ }));
  // 翻页循环是「await 后继续 await」，需要多轮 microtask 才跑得完（撞上限那例要 20 轮）
  for (let i = 0; i < 40; i++) {
    await act(async () => {
      await Promise.resolve();
    });
  }
}

/** 完整性那一行的读数（缺料声明只住在这里，故单独取）。 */
function completenessText(): string {
  return screen.getByTestId("history-completeness").textContent ?? "";
}

beforeAll(async () => {
  await i18n.changeLanguage("zh-CN");
});

beforeEach(() => {
  invokeMock.mockReset();
});

describe("AnalysisHistoryButton · 打开即取全（不再固定 30 条）", () => {
  it("满页后继续按 offset 翻下一页，条数读数等于两页之和", async () => {
    const offsets = scriptPages([rows(PAGE), rows(2, PAGE)]);
    await openPopup();

    expect(offsets).toEqual([0, PAGE]);
    expect(completenessText()).toContain(`共 ${PAGE + 2} 条`);
    // 「已达上限」不能出现在真正取完的场合
    expect(completenessText()).not.toContain("上限");
    // 夹具必须给满 500 行才能触发「满页续翻」，而这正是组件的 PAGE_SIZE ⇒ 默认 5s 在满载机器上会抖
  }, 20_000);

  it("源码里那条固定 30 条已消失，翻页上限走常量", async () => {
    const src = readFileSync(path.join(ROOT, COMPONENT), "utf8");
    expect(src).not.toContain("limit: 30, offset: 0");
    expect(src).toContain("limit: PAGE_SIZE");
    expect(src).toContain("MAX_PAGES_PER_ROUND");
    expect(src).toContain("mergeAnalysisHistoryPages");
  });

  /**
   * 全量渲染把这条崩暴露成整页错误（用户 2026-10-22 报）：库里有一行 `decision_json`
   * 存的是字面量 `"null"`（600036 / 2026-07-25），来历解析器在它上面抛
   * `Cannot read properties of null (reading 'actionSource')`。旧代码只渲染最新 30 行 ⇒ 碰不到。
   * ⇒ 崩点修在解析器（单一权威），这条锁的是「弹窗这一层不再因为一行脏数据整页塌」。
   */
  it("某行 decision_json 是字面量 null ⇒ 该行照常渲染，不造整页崩溃", async () => {
    scriptPages([[
      row("bad", { decisionJson: "null", stockName: "招商银行", stockCode: "600036" }),
      row("good", { decisionJson: JSON.stringify({ action: "持有", confidence: 62 }) }),
    ]]);
    await openPopup();

    expect(screen.getByText("招商银行")).toBeTruthy();
    expect(screen.getByText("贵州茅台")).toBeTruthy();
    expect(completenessText()).toContain("共 2 条");
  });
});

describe("AnalysisHistoryButton · 关键词筛选", () => {
  it("输入代码 ⇒ 只剩该股，条数改成「显示 分子 / 共 分母」", async () => {
    scriptPages([[
      row("a", { stockCode: "600519", stockName: "贵州茅台" }),
      row("b", { stockCode: "000001", stockName: "平安银行" }),
    ]]);
    await openPopup();
    expect(completenessText()).toContain("共 2 条");

    fireEvent.change(screen.getByPlaceholderText("搜索股票名称或代码"), { target: { value: "000001" } });
    for (let i = 0; i < 5; i++) {
      await act(async () => {});
    }

    expect(completenessText()).toContain("显示 1 / 共 2 条");
    expect(screen.queryByText("贵州茅台")).toBeNull();
    expect(screen.getByText("平安银行")).toBeTruthy();
  });
});

describe("AnalysisHistoryButton · 状态筛选", () => {
  it("点「已中断」只放 interrupted 行进来，且该 chip 自己是按下态", async () => {
    scriptPages([[
      row("a", { status: "completed" }),
      row("b", { status: "interrupted" }),
      row("c", { status: "failed" }),
    ]]);
    await openPopup();

    fireEvent.click(screen.getByRole("button", { name: "已中断" }));
    for (let i = 0; i < 5; i++) {
      await act(async () => {});
    }

    expect(completenessText()).toContain("显示 1 / 共 3 条");

    // 行内那枚状态标记走 <span title="原始态">译文</span> ⇒ 按 title 定位才不与同名 chip 撞车
    expect(screen.getByTitle("interrupted")).toBeTruthy();
    expect(screen.getByRole("button", { name: "已中断" }).getAttribute("aria-pressed")).toBe("true");
    // 已完成行的 ✓ 不得还在（那条被状态门筛掉了）
    expect(screen.queryByText("✓")).toBeNull();
  });

  it("点「全部」撤掉状态门（三条都回来）", async () => {
    scriptPages([[
      row("a", { status: "completed" }),
      row("b", { status: "interrupted" }),
      row("c", { status: "failed" }),
    ]]);
    await openPopup();
    fireEvent.click(screen.getByRole("button", { name: "已中断" }));
    await act(async () => {});

    const allChips = screen.getAllByRole("button", { name: "全部" });
    // 第 0 枚属状态行，第 1 枚属档位行 —— 点状态行那枚才该撤状态门
    fireEvent.click(allChips[0]);
    for (let i = 0; i < 5; i++) {
      await act(async () => {});
    }
    expect(completenessText()).toContain("共 3 条");
    expect(completenessText()).not.toContain("显示");
  });
});

describe("AnalysisHistoryButton · 档位筛选与呈现同判据", () => {
  it("中文脏值「短期」那行显示为未知周期，故必须被「未知周期」命中、不被「短线」命中", async () => {
    scriptPages([[
      row("a", { decisionTimeHorizon: "short" }),
      row("b", { decisionTimeHorizon: "短期" }),
      row("c", { decisionTimeHorizon: null }),
    ]]);
    await openPopup();

    fireEvent.click(screen.getByRole("button", { name: "未知周期" }));
    for (let i = 0; i < 5; i++) {
      await act(async () => {});
    }
    expect(completenessText()).toContain("显示 2 / 共 3 条");

    fireEvent.click(screen.getByRole("button", { name: "短线" }));
    for (let i = 0; i < 5; i++) {
      await act(async () => {});
    }
    expect(completenessText()).toContain("显示 1 / 共 3 条");
  });
});

describe("AnalysisHistoryButton · 结论筛选", () => {
  it("点「买入」只剩买入那一行；空结论行按后端哨兵归「数据缺失」，不再有自造的「无结论」格", async () => {
    scriptPages([[
      row("a", { stockCode: "600519", stockName: "贵州茅台", decisionAction: "买入" }),
      row("b", { stockCode: "000001", stockName: "平安银行", decisionAction: "观望" }),
      // 列表 DTO 对空 decision_action 下发权威哨兵 UNAVAILABLE（2026-10-22 裁定后补齐）
      row("c", { stockCode: "300750", stockName: "宁德时代", decisionAction: "UNAVAILABLE", decisionJson: null }),
    ]]);
    await openPopup();

    fireEvent.click(screen.getByRole("button", { name: "买入" }));
    for (let i = 0; i < 5; i++) {
      await act(async () => {});
    }
    expect(completenessText()).toContain("显示 1 / 共 3 条");
    expect(screen.getByText("贵州茅台")).toBeTruthy();
    expect(screen.queryByText("平安银行")).toBeNull();
    expect(screen.queryByText("宁德时代")).toBeNull();

    expect(screen.queryByRole("button", { name: "无结论" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "数据缺失" }));
    for (let i = 0; i < 5; i++) {
      await act(async () => {});
    }
    // 哨兵行必须落在「数据缺失」这一档（筛出来的 = Tag 上那枚）
    expect(completenessText()).toContain("显示 1 / 共 3 条");
    expect(screen.getByText("宁德时代")).toBeTruthy();
    expect(screen.queryByText("贵州茅台")).toBeNull();
  });

  it("结论与档位两维同时生效（取交集，不是各筛各的并集）", async () => {
    scriptPages([[
      row("a", { stockCode: "600519", stockName: "贵州茅台", decisionAction: "买入", decisionTimeHorizon: "mid" }),
      row("b", { stockCode: "600519", stockName: "贵州茅台", decisionAction: "买入", decisionTimeHorizon: "long" }),
    ]]);
    await openPopup();

    fireEvent.click(screen.getByRole("button", { name: "买入" }));
    await act(async () => {});
    fireEvent.click(screen.getByRole("button", { name: "长线" }));
    for (let i = 0; i < 5; i++) {
      await act(async () => {});
    }
    expect(completenessText()).toContain("显示 1 / 共 2 条");
  });
});

describe("AnalysisHistoryButton · 三种缺席各占一句", () => {
  it("筛空 ⇒ 说「没有符合筛选条件的记录」并带上已加载条数，不套用空库文案", async () => {
    scriptPages([[row("a"), row("b")]]);
    await openPopup();

    fireEvent.change(screen.getByPlaceholderText("搜索股票名称或代码"), { target: { value: "不存在的票" } });
    for (let i = 0; i < 5; i++) {
      await act(async () => {});
    }

    expect(screen.getByText("没有符合筛选条件的记录（已加载 2 条）")).toBeTruthy();
    expect(screen.queryByText("暂无历史分析")).toBeNull();
  });

  it("库里真的没记录 ⇒ 说「暂无历史分析」，且不显示「共 0 条」这种齐备态读数", async () => {
    scriptPages([[]]);
    await openPopup();

    expect(screen.getByText("暂无历史分析")).toBeTruthy();
    expect(screen.queryByTestId("history-completeness")).toBeNull();
  });

  it("取数失败 ⇒ 说「记录加载失败」并给重试，不得显示成空库或齐备态", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_stock_template_version_status") {
        return Promise.resolve({ codeVersion: 160, dbVersion: 160 });
      }
      return Promise.reject(new Error("boom"));
    });
    await openPopup();

    expect(completenessText()).toContain("记录加载失败");
    expect(screen.getByRole("button", { name: "重试" })).toBeTruthy();
    expect(completenessText()).not.toMatch(/共 \d+ 条/);
    expect(screen.queryByText("暂无历史分析")).toBeNull();
  });

  it("撞单次上限 ⇒ 显式说「可能还有更早的记录」并给继续加载，重复页不得把条数顶虚", async () => {
    // 每页都返回同一批 500 行（重叠页）⇒ 循环必然跑满 20 页才停，条数靠 id 去重守住
    const offsets = scriptPages(Array.from({ length: 25 }, () => rows(PAGE)));
    await openPopup();

    expect(offsets).toHaveLength(20);
    expect(completenessText()).toContain(`本次已加载 ${PAGE} 条`);
    expect(completenessText()).toContain("可能还有更早的记录");
    expect(screen.getByRole("button", { name: "继续加载" })).toBeTruthy();
    // 这条最重：20 页 × 500 行的去重与渲染，默认 5s 会被同机并发挤爆（实测单跑 1.3s、全量跑超 5s）
  }, 30_000);

  it("还在翻页时先说「已加载 N 条…」，不把中间态报成最终条数", async () => {
    let resolveFirst: (v: Row[]) => void = () => {};
    invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "get_stock_template_version_status") {
        return Promise.resolve({ codeVersion: 160, dbVersion: 160 });
      }
      if (Number(args?.offset) === 0) {
        return new Promise<Row[]>((res) => {
          resolveFirst = res;
        });
      }
      return Promise.resolve(rows(3, PAGE));
    });

    render(
      <MemoryRouter>
        <I18nextProvider i18n={i18n}>
          <App>
            <AnalysisHistoryButton />
          </App>
        </I18nextProvider>
      </MemoryRouter>,
    );
    await act(async () => {});
    fireEvent.click(screen.getByRole("button", { name: /历史分析/ }));
    await act(async () => {});

    expect(completenessText()).toContain("已加载 0 条…");

    await act(async () => {
      resolveFirst(rows(PAGE));
    });
    for (let i = 0; i < 10; i++) {
      await act(async () => {});
    }
    expect(completenessText()).toContain(`共 ${PAGE + 3} 条`);
  }, 20_000);
});

describe("AnalysisHistoryButton · 来历标识（live / as-of 回放）", () => {
  it("实况行不打来历标 —— 无标即 live（列 NOT NULL 且写侧只有两态，第三值另打「来源未知」）", async () => {
    scriptPages([[row("a", { analysisKind: "live" })]]);
    await openPopup();
    expect(screen.queryByTestId("provenance-tag")).toBeNull();
  });

  it("回放行打「历史回放」；决策日与列表日期相同时不把日期重复一遍", async () => {
    scriptPages([[
      row("a", { analysisKind: "replay", analysisDate: "2026-10-01", asOfDate: "2026-10-01" }),
    ]]);
    await openPopup();
    expect(screen.getByTestId("provenance-tag").textContent).toBe("历史回放");
  });

  it("决策日与列表日期不同 ⇒ 把决策日带上（现网 87 条回放里 37 条正是这一形态）", async () => {
    scriptPages([[
      row("a", { analysisKind: "replay", analysisDate: "2026-10-21", asOfDate: "2026-10-20" }),
    ]]);
    await openPopup();
    expect(screen.getByTestId("provenance-tag").textContent).toBe("历史回放 · 2026-10-20");
  });

  it("值域外的来历（空串 / 第三种值）⇒ 说「来源未知」，不得默认成实况或回放", async () => {
    scriptPages([[
      row("a", { stockCode: "600000", analysisKind: "" }),
      row("b", { stockCode: "600001", analysisKind: "backfill" }),
      row("c", { stockCode: "600002", analysisKind: "live" }),
    ]]);
    await openPopup();
    const tags = screen.getAllByTestId("provenance-tag");
    expect(tags).toHaveLength(2);
    for (const el of tags) {
      expect(el.textContent).toBe("来源未知");
    }
  });
});

describe("AnalysisHistoryButton · 决策结果配色（八档互异）", () => {
  const EIGHT: Array<[string, string]> = [
    ["600001", "买入"],
    ["600002", "增持"],
    ["600003", "持有"],
    ["600004", "观望"],
    ["600005", "减持"],
    ["600006", "卖出"],
    ["600007", "不确定"],
    ["600008", "数据缺失"],
  ];

  it("八档各出一枚 Tag 且颜色两两不同（旧表把 买入/增持、卖出/减持、不确定/数据缺失 各并成同色）", async () => {
    scriptPages([EIGHT.map(([code, action]) => row(code, { stockCode: code, decisionAction: action }))]);
    await openPopup();

    const tags = screen.getAllByTestId("action-tag");
    expect(tags).toHaveLength(8);
    const colorOf = new Map(tags.map((el) => [el.textContent ?? "", (el as HTMLElement).style.color]));
    // 断的是「互不相同」这件事本身：jsdom 会把 hex 归一成 rgb，所以不断具体字面量
    expect(new Set(tags.map((el) => (el as HTMLElement).style.color)).size).toBe(8);
    expect(colorOf.get("买入")).not.toBe(colorOf.get("增持"));
    expect(colorOf.get("卖出")).not.toBe(colorOf.get("减持"));
    expect(colorOf.get("不确定")).not.toBe(colorOf.get("数据缺失"));
  });

  it("库里存中文与存英文 token 渲染出同一个颜色（配色只认解析后的档，不认字形）", async () => {
    scriptPages([[
      row("a", { stockCode: "600001", decisionAction: "买入" }),
      row("b", { stockCode: "600002", decisionAction: "BUY" }),
      row("c", { stockCode: "600003", decisionAction: "strong_buy" }),
    ]]);
    await openPopup();
    const tags = screen.getAllByTestId("action-tag");
    expect(tags).toHaveLength(3);
    expect(new Set(tags.map((el) => (el as HTMLElement).style.color)).size).toBe(1);
  });
});
