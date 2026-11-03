import i18n from "@/i18n";
import { render, screen, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { poolBatchLineFor, RecommendationPanel } from "../RecommendationPanel";

/**
 * 候选池批次留痕的呈现契约（W4-3，2026-11-01）。
 *
 * 背景：`reco_pool_batches` 写了 307 行而荐股面板一行都没读过 ⇒「这一批到底扫了多少只」
 * 在界面上无处可查；同时批次时刻只显示 `03:49` 这种时分，上一周的批次看着像刚刷新的。
 *
 * 两层各测各的：
 *   - **渲染层**（A–F）：时间锚 store 用默认态（`mode:"live"`），本文件不 import 任何
 *     别的 feature store —— 判据的开关面在下面的纯函数层，不在这里绕。
 *   - **判据层**（G）：四种缺席、契约漂移、回放抑制全部打在 `poolBatchLineFor` 这同一个守卫上，
 *     每条负控只让它对应的那一句变。
 */
const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

const LEDGER_TESTID = "reco-pool-batch";
const TIME_TESTID = "reco-batch-time";

function batchResponse(generatedAt: number) {
  return {
    byHorizon: {
      short: {
        period: "short",
        picks: {
          trend: [
            {
              stockCode: "600001",
              stockName: "测试甲",
              style: "trend",
              period: "short",
              price: 10,
              entryLow: 9.8,
              entryHigh: 10.2,
              stopLoss: 9.5,
              targetPrice: 11,
              positionPct: 5,
              holdingDays: 5,
              confidence: 70,
              reasons: [],
              riskNotes: [],
              synthetic: false,
            },
          ],
        },
        disabledStyles: [],
        degradedStyles: [],
        generatedAt,
        rawSeedPoolSize: 90,
        seedPoolOrigin: {
          hot: 30,
          industry: 12,
          fallback: 0,
          user: 0,
          mover: 0,
          reflow: 0,
          moverMiss: 0,
        },
        mode: "live",
      },
    },
  };
}

/** 按命令名分发：留痕命令的返回值由每个用例自己给，其余命令给「什么都没有」。 */
function wire(ledger: unknown, generatedAt = Date.now()) {
  const resp = batchResponse(generatedAt);
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === "recommend_stocks_all_periods") { return resp; }
    // 挂载走的是「按档回读缓存」这条命令（单数），返回值是**单档**响应而不是整批
    if (cmd === "get_cached_recommendation") { return resp.byHorizon.short; }
    if (cmd === "get_reco_pool_batch") { return ledger; }
    return null;
  });
}

function row(overrides: Record<string, unknown> = {}) {
  return {
    generatedAt: "2026-11-01T03:49:11",
    period: "all",
    stockCount: 4120,
    degraded: false,
    purpose: "live",
    hasSnapshot: true,
    ...overrides,
  };
}

function renderPanel() {
  return render(
    <MemoryRouter>
      <I18nextProvider i18n={i18n}>
        <RecommendationPanel />
      </I18nextProvider>
    </MemoryRouter>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("RecommendationPanel — 候选池留痕（渲染层）", () => {
  it("A. 有快照且非空 ⇒ 报只数（W3 之后这是唯一能证明「池子不是那几个票」的读数）", async () => {
    wire(row());
    renderPanel();
    const line = await screen.findByTestId(LEDGER_TESTID);
    expect(line.textContent).toContain("4120");
    expect(line.textContent).not.toContain("为空");
    expect(line.textContent).not.toContain("未留");
  });

  it("B. 有快照但 0 只 ⇒ 说「池为空」，不得说成未留痕", async () => {
    wire(row({ stockCount: 0, degraded: true }));
    renderPanel();
    const line = await screen.findByTestId(LEDGER_TESTID);
    expect(line.textContent).toContain("为空");
    expect(line.textContent).not.toContain("未留");
    expect(line.textContent).not.toContain("还没有");
  });

  it("C. 快照原文缺失 ⇒ 说「未留快照」，并且不得同时说池为空（两者含义相反）", async () => {
    wire(row({ stockCount: 0, degraded: true, hasSnapshot: false }));
    renderPanel();
    const line = await screen.findByTestId(LEDGER_TESTID);
    expect(line.textContent).toContain("未留");
    expect(line.textContent).not.toContain("为空（");
    expect(line.textContent).not.toContain("还没有");
  });

  it("D. 命令返回 null ⇒ 说「还没有实盘批次」，不得伪装成池为空", async () => {
    wire(null);
    renderPanel();
    const line = await screen.findByTestId(LEDGER_TESTID);
    expect(line.textContent).toContain("还没有");
    expect(line.textContent).not.toContain("未留");
    expect(line.textContent).not.toContain("为空");
  });

  it("E. 契约漂移（返回的不是留痕行）⇒ 一条声明都不下，也不许读成「没有批次」", async () => {
    wire({ someOtherPayload: true });
    renderPanel();
    await waitFor(() => expect(screen.getAllByText("测试甲").length).toBeGreaterThan(0));
    expect(screen.queryByTestId(LEDGER_TESTID)).toBeNull();
  });

  it("F1. 批次时刻带日期，且满一天才给年龄（旧批次不再看着像刚刷新）", async () => {
    wire(row(), Date.now() - 3 * 86400000);
    renderPanel();
    const time = await screen.findByTestId(TIME_TESTID);
    expect(time.textContent).toContain("2026");
    expect(time.textContent).toContain("3 天前");
  });

  it("F2. 同一分钟内的批次不得显示「0 天前」（同屏噪声，也防负数）", async () => {
    wire(row(), Date.now());
    renderPanel();
    const time = await screen.findByTestId(TIME_TESTID);
    expect(time.textContent).not.toContain("天前");
  });
});

describe("poolBatchLineFor — 那一格说什么（判据层，同一个守卫）", () => {
  it("G1. 回放态抑制这一格：读的是实盘最近一批，不能顶替「那天的池」", () => {
    expect(poolBatchLineFor({ kind: "row", row: row() }, true)).toBeNull();
    // 同一条数据在实盘态要说只数 —— 撤掉 `isReplay` 那一条判据时这里必须红
    expect(poolBatchLineFor({ kind: "row", row: row() }, false)?.kind).toBe("count");
  });

  it("G2. 四种缺席各自成一句，不折叠", () => {
    expect(poolBatchLineFor({ kind: "none" }, false)?.kind).toBe("none");
    expect(poolBatchLineFor({ kind: "row", row: row({ hasSnapshot: false }) }, false)?.kind)
      .toBe("noSnapshot");
    expect(poolBatchLineFor({ kind: "row", row: row({ stockCount: 0 }) }, false)?.kind)
      .toBe("empty");
    const count = poolBatchLineFor({ kind: "row", row: row({ stockCount: 5570 }) }, false);
    expect(count?.kind).toBe("count");
    expect(count?.size).toBe(5570);
  });

  it("G3. 未判定态（读失败/形状不符）不下任何声明，回放与否都一样", () => {
    expect(poolBatchLineFor({ kind: "unknown" }, false)).toBeNull();
    expect(poolBatchLineFor({ kind: "unknown" }, true)).toBeNull();
  });
});
