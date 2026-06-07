import json, os, re

locales_dir = "D:/OneManager/AxInvest/src/i18n/locales"
locales = ["en-US","zh-CN","zh-TW","ja","ko","fr","de","es","ru","hi","ar"]

with open("D:/OneManager/AxInvest/scripts/used_keys_full.txt", encoding="utf-8") as f:
    used_keys = [l.strip() for l in f if l.strip()]

print(f"Total used keys: {len(used_keys)}")

def get_nested(obj, parts):
    for p in parts:
        if isinstance(obj, dict) and p in obj:
            obj = obj[p]
        else:
            return None
    return obj

def set_nested(obj, parts, value):
    for p in parts[:-1]:
        if p not in obj or not isinstance(obj[p], dict):
            obj[p] = {}
        obj = obj[p]
    last = parts[-1]
    if obj.get(last) is None or obj[last] == "":
        obj[last] = value

def humanize(k):
    return re.sub(r"([A-Z])", r" \1", k).title()

ZH = {
    "actionBuy":"买入","actionSell":"卖出","actionIncrease":"加仓","actionReduce":"减仓",
    "addFailed":"添加失败","addToWatchlist":"加入自选","addedToWatchlist":"已加入自选",
    "askAI":"问AI","askAi.decision":"AI决策","askAi.prompt":"请输入问题",
    "analyzing":"分析中",
    "alert.title":"价格提醒","alert.name":"名称","alert.code":"代码",
    "alert.condition":"条件","alert.price":"价格","alert.date":"日期",
    "alert.triggered":"已触发","alert.empty":"暂无提醒",
    "analystCount":"分析师数","announcements":"公告","announcementsEmpty":"暂无公告",
    "autoRefresh":"自动刷新","charCount":"字符数",
    "chart.restore":"恢复默认","chart.zoom":"缩放",
    "expandView":"展开查看","emptyHint":"暂无数据",
    "error":"错误","failure":"失败","completed":"已完成",
    "compare":"对比","exportReport":"导出报告","contextCopied":"已复制",
    "dataFetched":"数据已获取","dataFetching":"获取数据...",
    "dataSource":"数据源","openDataSource":"打开数据源设置",
    "indexQuotes":"指数行情","indexQuoteEmpty":"暂无指数数据",
    "industry":"行业","concepts":"概念","conceptBlocks":"概念板块","conceptBlocksEmpty":"暂无概念板块",
    "clsFlash":"快讯","clsFlashEmpty":"暂无快讯",
    "peers":"同行对比","peersEmpty":"暂无同行数据",
    "recommendation.title":"推荐结果","recommendation.empty":"暂无推荐数据",
    "recommendation.periodShort":"短期","recommendation.periodMid":"中期","recommendation.periodLong":"长期",
    "recommendation.generatedAt":"生成于 {time}",
    "recommendation.openSettings":"打开设置",
    "recommendation.row.entry":"入场区间","recommendation.row.stopLoss":"止损位",
    "recommendation.row.target":"目标价","recommendation.row.position":"建议仓位",
    "recommendation.row.holding":"持仓天数","recommendation.row.confidence":"置信度",
    "recommendation.row.secondaryStyle":"次要风格",
    "recommendation.row.reasons":"理由","recommendation.row.risks":"风险",
    "debate":"辩论","finalDecision":"最终决策","decision":"决策",
    "risk":"风险","risks":"风险",
    "value":"估值","pe":"市盈率","pb":"市净率","roe":"ROE",
    "revenue":"营收","netProfit":"净利润","grossMargin":"毛利率",
    "expectedUpside":"预期涨幅",
    "watchlist":"自选股","watchlistEmpty":"自选股为空",
    "dailyReview.title":"每日复盘","history":"历史","historyEmpty":"暂无历史",
    "export.title":"导出","exported":"已导出",
    "settings.title":"设置","settings.panels.refresh":"刷新",
    "holdings":"持仓","holding":"持仓",
    "position":"仓位","style":"风格","styles":"风格",
    "refresh":"刷新","refreshing":"刷新中",
    "col.name":"名称","col.code":"代码","col.price":"价格",
    "col.change":"涨跌","col.changePct":"涨跌幅",
    "today":"今日","yesterday":"昨日",
    "northBound":"北向资金","northBoundEmpty":"暂无北向资金数据",
    "mainInflow":"主力净流入","mainInflowEmpty":"暂无主力资金数据",
    "financials":"财务数据","financialsEmpty":"暂无财务数据",
    "upgrade":"上调","downgrade":"下调","rating":"评级",
    "ratingBuy":"买入","ratingHold":"持有","ratingSell":"卖出",
    "confirm":"确认","cancel":"取消","save":"保存","delete":"删除",
    "edit":"编辑","view":"查看","close":"关闭",
    "loading":"加载中","noData":"暂无数据","fetchFailed":"获取失败",
    "retry":"重试","back":"返回","next":"下一步","prev":"上一步",
    "bull":"看涨","bear":"看跌","buyShort":"买跌",
    "change":"涨跌","close":"收盘","high":"最高","low":"最低",
    "volume":"成交量","amount":"成交额","turnover":"换手率",
    "backtest.title":"回测","backtest.summary":"回测摘要","backtest.total":"总数",
    "backtest.correct":"正确","backtest.wrong":"错误","backtest.accuracy":"准确率",
    "backtest.avgReturn":"平均收益","backtest.avgConfidence":"平均置信度",
    "backtest.maxDrawdown":"最大回撤","backtest.holdingDays":"持仓天数",
    "backtest.returnRate":"收益率","backtest.alpha":"Alpha",
    "backtest.run":"运行回测","backtest.runAll":"全部回测",
    "backtest.allFailed":"全部失败","backtest.failed":"失败",
}

EN = {
    "actionBuy":"Buy","actionSell":"Sell","actionIncrease":"Increase","actionReduce":"Reduce",
    "addToWatchlist":"Add to Watchlist","addedToWatchlist":"Added to Watchlist",
    "askAI":"Ask AI","analyzing":"Analyzing",
    "alert.title":"Price Alert","alert.name":"Name","alert.code":"Code",
    "alert.condition":"Condition","alert.price":"Price","alert.date":"Date",
    "alert.triggered":"Triggered","alert.empty":"No Alerts",
    "analystCount":"Analyst Count","announcements":"Announcements","announcementsEmpty":"No Announcements",
    "backtest.title":"Backtest","backtest.summary":"Backtest Summary",
    "backtest.accuracy":"Accuracy","backtest.avgReturn":"Avg Return",
    "backtest.avgConfidence":"Avg Confidence","backtest.maxDrawdown":"Max Drawdown",
    "backtest.holdingDays":"Holding Days","backtest.returnRate":"Return Rate",
    "backtest.run":"Run Backtest","backtest.runAll":"Run All Backtests",
    "bull":"Bullish","bear":"Bearish","buyShort":"Buy Short",
    "change":"Change","close":"Close","high":"High","low":"Low",
    "volume":"Volume","amount":"Amount","turnover":"Turnover",
    "autoRefresh":"Auto Refresh","error":"Error","failure":"Failed","completed":"Completed",
    "compare":"Compare","exportReport":"Export Report","contextCopied":"Copied",
    "indexQuotes":"Index Quotes","industry":"Industry","concepts":"Concepts",
    "conceptBlocks":"Concept Blocks","clsFlash":"Flash News",
    "peers":"Peers","recommendation.title":"Recommendation",
    "debate":"Debate","finalDecision":"Final Decision","decision":"Decision",
    "risk":"Risk","risks":"Risks",
    "value":"Valuation","pe":"P/E","pb":"P/B","roe":"ROE",
    "revenue":"Revenue","netProfit":"Net Profit","grossMargin":"Gross Margin",
    "expectedUpside":"Expected Upside","watchlist":"Watchlist",
    "dailyReview.title":"Daily Review","history":"History",
    "export.title":"Export","exported":"Exported",
    "settings.title":"Settings","refresh":"Refresh",
    "holdings":"Holdings","holding":"Holding","position":"Position",
    "style":"Style","styles":"Styles","refreshing":"Refreshing",
    "col.name":"Name","col.code":"Code","col.price":"Price",
    "col.change":"Change","col.changePct":"Change %",
    "today":"Today","yesterday":"Yesterday",
    "askAi.prompt":"Enter your question",
    "northBound":"North Bound","northBoundEmpty":"No North Bound Data",
    "mainInflow":"Main Inflow","mainInflowEmpty":"No Main Inflow Data",
    "financials":"Financials","financialsEmpty":"No Financial Data",
    "upgrade":"Upgrade","downgrade":"Downgrade","rating":"Rating",
    "ratingBuy":"Buy","ratingHold":"Hold","ratingSell":"Sell",
    "confirm":"Confirm","cancel":"Cancel","save":"Save","delete":"Delete",
    "edit":"Edit","view":"View","close":"Close",
    "loading":"Loading","noData":"No Data","fetchFailed":"Fetch Failed",
    "retry":"Retry","back":"Back","next":"Next","prev":"Previous",
}

ZH_TW = {}
for k, v in ZH.items():
    s = v
    s = s.replace("买入","買入").replace("卖出","賣出").replace("加仓","加倉").replace("减仓","減倉")
    s = s.replace("自选","自選").replace("价格","價格").replace("条件","條件").replace("触发","觸發")
    s = s.replace("分析师","分析師").replace("公告","公告").replace("回测","回測").replace("准确率","準確率")
    s = s.replace("最大回撤","最大回撤").replace("持仓天数","持有天數")
    s = s.replace("看涨","看漲").replace("看跌","看跌")
    s = s.replace("涨跌","漲跌").replace("收盘","收盤").replace("最高","最高").replace("最低","最低")
    s = s.replace("成交量","成交量").replace("成交额","成交額").replace("换手率","換手率")
    s = s.replace("自动刷新","自動刷新").replace("错误","錯誤").replace("失败","失敗").replace("完成","完成")
    s = s.replace("指数行情","指數行情").replace("行业","行業").replace("概念","概念")
    s = s.replace("板块","板塊").replace("快讯","快訊")
    s = s.replace("同行","同行").replace("推荐结果","推薦結果").replace("暂无","暫無").replace("数据","數據")
    s = s.replace("辩论","辯論").replace("决策","決策").replace("风险","風險")
    s = s.replace("估值","估值").replace("市盈率","市盈率").replace("市净率","市淨率")
    s = s.replace("营收","營收").replace("净利润","淨利潤").replace("毛利率","毛利率")
    s = s.replace("预期涨幅","預期漲幅").replace("自选股","自選股").replace("每日复盘","每日復盤").replace("历史","歷史")
    s = s.replace("导出","導出").replace("设置","設定").replace("刷新","刷新").replace("持仓","持倉").replace("仓位","倉位")
    s = s.replace("代码","代碼").replace("市值","市值").replace("流通","流通")
    s = s.replace("今日","今日").replace("昨日","昨日").replace("本周","本週").replace("本月","本月")
    s = s.replace("北向资金","北向資金").replace("主力净流入","主力淨流入")
    s = s.replace("财务数据","財務數據").replace("总资产","總資產").replace("总负债","總負債").replace("资产负债率","資產負債率")
    s = s.replace("上调","上調").replace("下调","下調").replace("评级","評級")
    s = s.replace("确认","確認").replace("取消","取消").replace("保存","保存").replace("删除","刪除")
    s = s.replace("编辑","編輯").replace("查看","查看").replace("关闭","關閉")
    s = s.replace("加载中","載入中").replace("获取失败","獲取失敗").replace("重试","重試").replace("返回","返回").replace("下一步","下一步").replace("上一步","上一步")
    s = s.replace("入场区间","入場區間").replace("止损位","止損位").replace("目标价","目標價")
    s = s.replace("建议仓位","建議倉位").replace("持仓天数","持倉天數").replace("置信度","置信度")
    s = s.replace("次要风格","次要風格").replace("理由","理由").replace("风险","風險")
    s = s.replace("生成于 {time}","生成於 {time}").replace("打开设置","打開設定")
    s = s.replace("部分风格已禁用: {styles}","部分風格已禁用: {styles}").replace("风格 {style} 已在设置中禁用","風格 {style} 已在設定中禁用")
    s = s.replace("暂无推荐数据","暫無推薦數據").replace("短期","短期").replace("中期","中期").replace("长期","長期")
    ZH_TW[k] = s

for loc in locales:
    f = os.path.join(locales_dir, loc + ".json")
    if not os.path.exists(f):
        print(f"Missing: {f}")
        continue
    with open(f, encoding="utf-8") as fp:
        d = json.load(fp)
    if "stockAnalysis" not in d:
        d["stockAnalysis"] = {}
    sa = d["stockAnalysis"]
    changed = False

    for full_key in used_keys:
        parts = full_key.replace("stockAnalysis.", "").split(".")
        ex = get_nested(sa, parts)
        if ex is not None and ex != "":
            continue
        dk = ".".join(parts)
        if loc == "en-US":
            val = EN.get(dk, humanize(parts[-1]))
        elif loc == "zh-CN":
            val = ZH.get(dk, full_key)
        elif loc == "zh-TW":
            val = ZH_TW.get(dk, ZH.get(dk, full_key))
        else:
            val = EN.get(dk, humanize(parts[-1]))
        set_nested(sa, parts, val)
        changed = True

    if changed:
        with open(f, "w", encoding="utf-8") as fp:
            json.dump(d, fp, ensure_ascii=False, indent=2)
            fp.write("\n")
        print(f"Updated: {loc}")
    else:
        print(f"No changes: {loc}")

print("Done!")
