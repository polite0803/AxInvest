#!/usr/bin/env node
// Stock Analysis CLI
// Usage: node scripts/stock-cli.mjs search 茅台
//        node scripts/stock-cli.mjs quote 600519
//        node scripts/stock-cli.mjs analyze 600519
//        node scripts/stock-cli.mjs backtest
//        node scripts/stock-cli.mjs watchlist

const API_BASE = process.env.AXINVEST_API || "http://localhost:5173/api";

async function api(path, options = {}) {
  const url = `${API_BASE}${path}`;
  const res = await fetch(url, {
    headers: { "Content-Type": "application/json", ...options.headers },
    ...options,
  });
  if (!res.ok) {
    console.error(`Error ${res.status}: ${await res.text()}`);
    process.exit(1);
  }
  return res.json();
}

async function main() {
  const [cmd, ...args] = process.argv.slice(2);

  switch (cmd) {
    case "search": {
      const keyword = args.join(" ") || args[0];
      if (!keyword) {
        console.error("Usage: stock-cli search <keyword>");
        process.exit(1);
      }
      const results = await api(
        `/stock/search?keyword=${encodeURIComponent(keyword)}`,
      );
      console.log(JSON.stringify(results, null, 2));
      break;
    }
    case "quote": {
      const code = args[0];
      if (!code) {
        console.error("Usage: stock-cli quote <code>");
        process.exit(1);
      }
      const quote = await api(`/stock/quote?code=${code}`);
      console.log(`${quote.name} (${quote.code})`);
      const sign = quote.changePct >= 0 ? "+" : "";
      console.log(
        `  价格: ${quote.price}  (${sign}${quote.changePct.toFixed(2)}%)`,
      );
      console.log(
        `  开: ${quote.open}  高: ${quote.high}  低: ${quote.low}`,
      );
      console.log(`  量: ${(quote.volume / 10000).toFixed(1)}万手`);
      if (quote.pe) console.log(`  PE: ${quote.pe}`);
      if (quote.pb) console.log(`  PB: ${quote.pb}`);
      break;
    }
    case "analyze": {
      const code = args[0];
      if (!code) {
        console.error("Usage: stock-cli analyze <code> [date]");
        process.exit(1);
      }
      const date = args[1] || new Date().toISOString().split("T")[0];
      console.log(`启动分析: ${code} (${date})...`);
      const result = await api("/stock/analysis", {
        method: "POST",
        body: JSON.stringify({ stock_code: code, date, provider_id: "" }),
      });
      console.log(`分析ID: ${result.analysis_id}`);
      console.log(`状态: ${result.status}`);
      console.log(`查看结果: stock-cli analysis ${result.analysis_id}`);
      break;
    }
    case "analysis": {
      const id = args[0];
      if (!id) {
        console.error("Usage: stock-cli analysis <id>");
        process.exit(1);
      }
      const analysis = await api(`/stock/analysis/${id}`);
      console.log(JSON.stringify(analysis, null, 2));
      break;
    }
    case "backtest": {
      console.log("回测历史分析...");
      const stats = await api("/stock/backtest", {
        method: "POST",
        body: JSON.stringify({ holding_days: 20 }),
      });
      console.log(`总分析数: ${stats.total_analyses}`);
      console.log(`准确率: ${stats.accuracy_pct.toFixed(1)}%`);
      console.log(`平均收益: ${stats.avg_return_pct.toFixed(2)}%`);
      const conf = (stats.avg_confidence * 100).toFixed(1);
      console.log(`平均置信度: ${conf}%`);
      break;
    }
    case "watchlist": {
      const sub = args[0];
      if (sub === "add") {
        const code = args[1];
        const name = args[2];
        await api("/stock/watchlist", {
          method: "POST",
          body: JSON.stringify({ stock_code: code, stock_name: name }),
        });
        console.log(`已添加: ${name} (${code})`);
      } else if (sub === "rm") {
        await api(`/stock/watchlist/${args[1]}`, { method: "DELETE" });
        console.log("已删除");
      } else {
        const items = await api("/stock/watchlist");
        items.forEach((i) =>
          console.log(`  ${i.stockCode}  ${i.stockName}`)
        );
      }
      break;
    }
    case "portfolio": {
      const holdings = await api("/stock/portfolio");
      console.log("持仓汇总:");
      console.log(
        "代码      名称      持仓     成本    现价     市值      盈亏     盈亏%",
      );
      console.log("-".repeat(80));
      for (const h of holdings) {
        const sign = h.pnl >= 0 ? "+" : "";
        const pnlStr = `${sign}${h.pnl.toFixed(0)}`;
        const cols = [
          h.stockCode.padEnd(10),
          h.stockName.padEnd(8),
          String(h.shares).padEnd(8),
          h.avgCost.toFixed(2).padEnd(7),
          h.currentPrice.toFixed(2).padEnd(7),
          h.marketValue.toFixed(0).padEnd(8),
          pnlStr.padEnd(8),
          h.pnlPct.toFixed(2) + "%",
        ];
        console.log(cols.join(" "));
      }
      break;
    }
    default:
      console.log("AxInvest Stock CLI");
      console.log("  search <keyword>       搜索股票");
      console.log("  quote <code>           实时行情");
      console.log("  analyze <code> [date]  启动分析");
      console.log("  analysis <id>          查看分析结果");
      console.log("  backtest               回测历史分析");
      console.log("  watchlist              查看自选股");
      console.log("  watchlist add <code> <name>  添加自选");
      console.log("  watchlist rm <id>            删除自选");
      console.log("  portfolio              查看持仓盈亏");
      break;
  }
}

main().catch((e) => {
  console.error("Error:", e.message);
  process.exit(1);
});
