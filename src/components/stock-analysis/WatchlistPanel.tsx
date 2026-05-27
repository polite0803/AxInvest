import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { DeleteOutlined, PlusOutlined } from "@ant-design/icons";
import { Button, Dropdown, Empty, Input, List, message, Popconfirm, Select, Space, Spin, Tag } from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

interface WatchlistItem {
  id: string;
  stockCode: string;
  stockName: string;
  group: string;
  notes?: string;
  createdAt: number;
}

interface QuoteSnapshot {
  price: number;
  changePct: number;
  pe?: number;
  timestamp: string;
}

const DEFAULT_GROUP = "__default__";
const GROUP_STORAGE_KEY = "ax_watchlist_groups";

type SortKey = "name" | "change" | "code";

export function WatchlistPanel() {
  const { t } = useTranslation();
  const store = useStockAnalysisStore();
  const stockCode = store.stockCode;
  const stockName = store.stockName;
  const getStockQuote = store.getStockQuote;
  const getStockKline = store.getStockKline;
  const startAnalysis = store.startAnalysis;
  const watchlistVersion = useStockAnalysisStore((s) => s.watchlistVersion);

  const [items, setItems] = useState<WatchlistItem[]>([]);
  const [quotes, setQuotes] = useState<Record<string, QuoteSnapshot>>({});
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [sortKey, setSortKey] = useState<SortKey>("change");
  const [activeGroup, setActiveGroup] = useState(DEFAULT_GROUP);
  const [newGroup, setNewGroup] = useState("");
  const [editingGroup, setEditingGroup] = useState<string | null>(null);

  // 分组列表（从 localStorage）
  const groups = useMemo(() => {
    try {
      return JSON.parse(localStorage.getItem(GROUP_STORAGE_KEY) ?? "[]") as string[];
    } catch {
      return [];
    }
  }, []);

  const saveGroups = (g: string[]) => {
    localStorage.setItem(GROUP_STORAGE_KEY, JSON.stringify(g));
  };

  const loadWatchlist = useCallback(async () => {
    setLoading(true);
    try {
      const list = await invoke<WatchlistItem[]>("list_watchlist");
      if (Array.isArray(list)) {
        // 从 notes JSON 恢复 group 信息
        const parsed = list.map((w: any) => {
          let group = DEFAULT_GROUP;
          try {
            if (w.notes) {
              const n = JSON.parse(w.notes);
              if (n.group) { group = n.group; }
            }
          } catch { /* ignore */ }
          return { ...w, group };
        });
        setItems(parsed);
      }
    } catch { /* 后端未运行 */ }
    setLoading(false);
  }, []);

  useEffect(() => {
    loadWatchlist();
  }, [watchlistVersion, loadWatchlist]);

  // 当前分组下的自选股
  const groupItems = useMemo(
    () => items.filter((i) => i.group === activeGroup),
    [items, activeGroup],
  );

  // 实时行情刷新
  const refreshQuotes = useCallback(async () => {
    if (groupItems.length === 0) { return; }
    setRefreshing(true);
    const snap: Record<string, QuoteSnapshot> = {};
    for (const item of groupItems) {
      try {
        const q = await invoke<any>("get_stock_quote", { stockCode: item.stockCode });
        snap[item.stockCode] = {
          price: q?.price ?? 0,
          changePct: q?.changePct ?? 0,
          pe: q?.pe,
          timestamp: q?.timestamp ?? "",
        };
      } catch { /* skip */ }
    }
    setQuotes(snap);
    setRefreshing(false);
  }, [groupItems]);

  useEffect(() => {
    refreshQuotes();
    const timer = setInterval(refreshQuotes, 15000);
    return () => clearInterval(timer);
  }, [refreshQuotes]);

  // 排序
  const sorted = useMemo(() => {
    return [...groupItems].sort((a, b) => {
      const qa = quotes[a.stockCode];
      const qb = quotes[b.stockCode];
      switch (sortKey) {
        case "change":
          return (qb?.changePct ?? -999) - (qa?.changePct ?? -999);
        case "code":
          return a.stockCode.localeCompare(b.stockCode);
        case "name":
          return a.stockName.localeCompare(b.stockName);
        default:
          return 0;
      }
    });
  }, [groupItems, quotes, sortKey]);

  const addCurrent = async () => {
    if (!stockCode || !stockName) { return; }
    try {
      await invoke("add_to_watchlist", { stockCode, stockName });
      loadWatchlist();
    } catch { /* 静默 */ }
  };

  const remove = async (id: string) => {
    try {
      await invoke("remove_from_watchlist", { id });
      loadWatchlist();
    } catch { /* 静默 */ }
  };

  const moveToGroup = async (item: WatchlistItem, targetGroup: string) => {
    try {
      await invoke("remove_from_watchlist", { id: item.id });
      await invoke("add_to_watchlist", { stockCode: item.stockCode, stockName: item.stockName });
      // 用 notes 存 group
      // 简化：直接更新本地状态
      setItems((prev) => prev.map((i) => (i.id === item.id ? { ...i, group: targetGroup } : i)));
    } catch { /* 静默 */ }
  };

  const handleClick = async (code: string) => {
    await getStockQuote(code);
    await getStockKline(code, "daily", 120);
    startAnalysis(code);
  };

  const analyzeAll = async () => {
    for (const item of sorted) {
      try {
        await getStockQuote(item.stockCode);
        await getStockKline(item.stockCode, "daily", 120);
        startAnalysis(item.stockCode);
      } catch { /* 继续 */ }
    }
    message.info(`已启动 ${sorted.length} 只股票分析`);
  };

  const addGroup = () => {
    const name = newGroup.trim();
    if (!name || groups.includes(name)) { return; }
    const updated = [...groups, name];
    saveGroups(updated);
    setNewGroup("");
    setActiveGroup(name);
  };

  if (loading) { return <Spin size="small" />; }

  return (
    <div>
      {/* 分组 Tab */}
      <div className="flex items-center gap-1 mb-2 flex-wrap">
        <Tag
          color={activeGroup === DEFAULT_GROUP ? "blue" : "default"}
          className="cursor-pointer m-0 text-xs"
          onClick={() => setActiveGroup(DEFAULT_GROUP)}
        >
          全部 ({items.length})
        </Tag>
        {groups.map((g) => (
          <Tag
            key={g}
            color={activeGroup === g ? "blue" : "default"}
            className="cursor-pointer m-0 text-xs"
            closable={editingGroup === g}
            onClose={() => {
              saveGroups(groups.filter((x) => x !== g));
              setActiveGroup(DEFAULT_GROUP);
              setEditingGroup(null);
            }}
            onClick={() => setActiveGroup(g)}
            onDoubleClick={() => setEditingGroup(g)}
          >
            {g} ({items.filter((i) => i.group === g).length})
          </Tag>
        ))}
        <Input
          size="small"
          placeholder="新分组"
          style={{ width: 80 }}
          value={newGroup}
          onChange={(e) => setNewGroup(e.target.value)}
          onPressEnter={addGroup}
          onBlur={addGroup}
        />
      </div>

      {/* 工具栏 */}
      <div className="flex items-center justify-between mb-1">
        <Space size={4}>
          <Button size="small" icon={<PlusOutlined />} disabled={!stockCode} onClick={() => addCurrent()}>
            {t("stockAnalysis.addToWatchlist")}
          </Button>
          <Button size="small" loading={refreshing} onClick={refreshQuotes}>
            刷新行情
          </Button>
        </Space>
        <Space size={4}>
          <Select
            size="small"
            style={{ width: 90 }}
            value={sortKey}
            onChange={(v) => setSortKey(v)}
            options={[
              { value: "change", label: "按涨跌" },
              { value: "code", label: "按代码" },
              { value: "name", label: "按名称" },
            ]}
          />
          {sorted.length > 1 && (
            <Button size="small" type="primary" onClick={analyzeAll}>
              分析全部
            </Button>
          )}
        </Space>
      </div>

      {sorted.length === 0
        ? <Empty description={t("stockAnalysis.watchlistEmpty")} image={Empty.PRESENTED_IMAGE_SIMPLE} />
        : (
          <List
            size="small"
            dataSource={sorted}
            renderItem={(item) => {
              const q = quotes[item.stockCode];
              const up = (q?.changePct ?? 0) >= 0;
              const changeStr = q ? `${up ? "+" : ""}${q.changePct.toFixed(2)}%` : "—";
              return (
                <List.Item
                  style={{ cursor: "pointer", padding: "4px 8px" }}
                  onClick={() => handleClick(item.stockCode)}
                  actions={[
                    activeGroup !== DEFAULT_GROUP && (
                      <Dropdown
                        key="move"
                        menu={{
                          items: [
                            { key: DEFAULT_GROUP, label: "移回全部" },
                            ...groups.filter((g) => g !== activeGroup).map((g) => ({ key: g, label: g })),
                          ],
                          onClick: ({ key }) => moveToGroup(item, key),
                        }}
                      >
                        <Button size="small" type="text" className="text-xs">移动</Button>
                      </Dropdown>
                    ),
                    <Popconfirm
                      key="del"
                      title={t("common.confirm")}
                      onConfirm={() => remove(item.id)}
                    >
                      <Button size="small" type="text" danger icon={<DeleteOutlined />} />
                    </Popconfirm>,
                  ].filter(Boolean)}
                >
                  <div className="flex items-center gap-2 w-full">
                    <Tag className="m-0 text-xs shrink-0">{item.stockCode}</Tag>
                    <span className="text-xs flex-1 truncate">{item.stockName}</span>
                    {q && (
                      <Space size={4} className="shrink-0">
                        <span className="text-xs font-mono">{q.price.toFixed(2)}</span>
                        <span className={`text-xs ${up ? "text-red-500" : "text-green-500"}`}>
                          {changeStr}
                        </span>
                        {q.pe && <span className="text-xs text-gray-400">PE {q.pe.toFixed(1)}</span>}
                      </Space>
                    )}
                  </div>
                </List.Item>
              );
            }}
          />
        )}
    </div>
  );
}
