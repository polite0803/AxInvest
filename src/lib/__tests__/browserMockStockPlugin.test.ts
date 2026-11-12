import { handleCommand } from "@/lib/browserMock";
import { describe, expect, it } from "vitest";

// #248 前端半：4 条股票宿主命令切到插件命名命令（stock_* 前缀）后，
// dev mock 必须有显式分支——浏览器模式没有插件安装根，也没有命令桥，
// mock 镜像的是「归属闸门放行后的执行形态」。负控锁 default 兜底仍在，
// 证明显式 case 真的在起作用（先例：browserMockTrade.test.ts）。
describe("browserMock 覆盖股票插件命名命令（T2 试点批）", () => {
  it("stock_get_pipeline_history 返回数组（消费端按 .filter/.length 使用）", async () => {
    const value = await handleCommand("stock_get_pipeline_history", { limit: 20 });
    expect(Array.isArray(value)).toBe(true);
  });

  it("stock_get_pipeline_run_detail 返回 null 而不是 {}（无运行可查；{} 会被详情抽屉当成有数据）", async () => {
    await expect(
      handleCommand("stock_get_pipeline_run_detail", { runId: "nope" }),
    ).resolves.toBeNull();
  });

  it("stock_list_pending_evolution_plans 返回演示行集，与宿主命令 list_pending_evolution_plans 同源同形", async () => {
    const rows = await handleCommand<Record<string, unknown>[]>(
      "stock_list_pending_evolution_plans",
      {},
    );
    expect(Array.isArray(rows)).toBe(true);
    expect(rows!.length).toBeGreaterThan(0);
    const hostRows = await handleCommand<Record<string, unknown>[]>(
      "list_pending_evolution_plans",
      {},
    );
    expect(rows).toEqual(hostRows);
  });

  it("stock_wf_des_integration 返回 null（浏览器模式无 WalkForward/DES 仿真引擎，诚实空态）", async () => {
    await expect(handleCommand("stock_wf_des_integration", {})).resolves.toBeNull();
  });

  // 批 2b：行情两条。`stock_get_quote` 的消费端（CompareView / WatchlistPanel）逐字段
  // `?? 0` 兜底 ⇒ 空对象就是诚实空态，与宿主 `get_stock_quote` 的 default 兜底同形；
  // `stock_get_kline` 的消费端（WfDesTab.fetchKlines）拿到就 `.sort` ⇒ 必须回数组。
  it("stock_get_quote 返回对象 DTO 空态 {}", async () => {
    await expect(handleCommand("stock_get_quote", { stockCode: "600519" })).resolves.toEqual({});
  });

  it("stock_get_kline 返回数组而不是 undefined（.sort 会当场 TypeError）", async () => {
    await expect(
      handleCommand("stock_get_kline", { stockCode: "600519", period: "daily", limit: 100 }),
    ).resolves.toEqual([]);
  });

  // 批 2c：全市场榜单/盘面四条（#262）。数组消费端（HotStocks / IndustryRanking /
  // SectorHeatmap / DragonTiger 四个面板的 `.slice` / Array.isArray）回 []，
  // 单对象 DTO 消费端（NorthBoundPanel 逐字段 `?? 0`）回 {}。
  // 三条不以 s 结尾的若删显式 case，default 兜底给 undefined（负控已锁此规则）。
  it("stock_get_hot_stocks 返回数组（HotStocksPanel Array.isArray 消费；不以显式 case 依赖 endsWith(s) 兜底）", async () => {
    await expect(handleCommand("stock_get_hot_stocks", {})).resolves.toEqual([]);
  });

  it("stock_get_industry_ranking 返回数组（default 兜底只会给 undefined，消费端 .slice 当场 TypeError）", async () => {
    await expect(handleCommand("stock_get_industry_ranking", {})).resolves.toEqual([]);
  });

  it("stock_get_market_dragon_tiger 返回数组（同上，DragonTigerPanel .slice 消费）", async () => {
    await expect(handleCommand("stock_get_market_dragon_tiger", {})).resolves.toEqual([]);
  });

  it("stock_get_north_bound_flow 返回对象 DTO 空态 {}（NorthBoundPanel 单对象消费，全 0 自判无数据）", async () => {
    await expect(handleCommand("stock_get_north_bound_flow", {})).resolves.toEqual({});
  });

  // 候选 A 直映批（#300 前端半）：六条有前端调用方的命令。
  // stock_search_stock 两类消费端（StockSearchBar / stockAnalysisStore）都先
  // Array.isArray 再取 [0] ⇒ [] 即诚实空态。
  it("stock_search_stock 返回数组（Array.isArray 消费；default 兜底给 undefined 同样能过但显式登记不受兜底收紧牵连）", async () => {
    await expect(handleCommand("stock_search_stock", { keyword: "茅台" })).resolves.toEqual([]);
  });

  it("stock_get_limit_up_pool 返回 null 而不是 {}（可空 DTO，{} 会让 entries 读取 TypeError 落 connectionFailed）", async () => {
    await expect(handleCommand("stock_get_limit_up_pool", {})).resolves.toBeNull();
  });

  it("stock_get_international_stock_quote 返回对象 DTO 空态 {}（crossMarketStore 逐字段消费）", async () => {
    await expect(
      handleCommand("stock_get_international_stock_quote", { stockCode: "AAPL" }),
    ).resolves.toEqual({});
  });

  it("三条跨市场 kline 返回数组而不是 undefined（消费端按数组使用）", async () => {
    await expect(
      handleCommand("stock_get_international_stock_kline", { stockCode: "AAPL", period: "daily", limit: 30 }),
    ).resolves.toEqual([]);
    await expect(
      handleCommand("stock_get_benchmark_kline", { benchmarkCode: "SPX", period: "daily", limit: 30 }),
    ).resolves.toEqual([]);
    await expect(
      handleCommand("stock_get_forex_kline", { pair: "USD/CNY", period: "daily", limit: 30 }),
    ).resolves.toEqual([]);
  });

  // 负控：这批命令名均不以 get_/list_ 开头，default 兜底对它们给 undefined
  // （或对 endsWith("s") 给 []）。用未登记探针锁住兜底规则本身仍在，
  // 避免上面四条因兜底被改而失去「显式 case 生效」的证明力。
  it("负控：未登记的 stock_ 探针命令仍走 default 兜底（undefined）", async () => {
    await expect(handleCommand("stock_probe_unregistered", {})).resolves.toBeUndefined();
  });

  it("负控：兜底对 get_ 前缀命令返回 {}，与插件命名命令的显式分支形态可区分", async () => {
    // stock_get_* 不命中 get_ 前缀规则（兜底 undefined）；
    // 真 get_ 前缀命令命中规则（兜底 {}）。两者都证明显式 case 是唯一来源。
    await expect(handleCommand("get_probe_unregistered", {})).resolves.toEqual({});
    await expect(handleCommand("stock_get_probe_unregistered", {})).resolves.toBeUndefined();
  });
});
