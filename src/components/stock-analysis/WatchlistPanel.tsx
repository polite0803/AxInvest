import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { DeleteOutlined, PlusOutlined } from "@ant-design/icons";
import { Button, List, Popconfirm, Tag } from "antd";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface WatchlistItem {
  id: string;
  stockCode: string;
  stockName: string;
  notes?: string;
  createdAt: number;
}

export function WatchlistPanel() {
  const { t } = useTranslation();
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const stockName = useStockAnalysisStore((s) => s.stockName);
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const [items, setItems] = useState<WatchlistItem[]>([]);

  const loadWatchlist = async () => {
    try {
      const list = await invoke<WatchlistItem[]>("list_watchlist");
      setItems(list);
    } catch {
      // 后端未运行或无数据时静默
    }
  };

  useEffect(() => {
    loadWatchlist();
  }, []);

  const addCurrent = async () => {
    if (!stockCode || !stockName) { return; }
    try {
      await invoke("add_to_watchlist", { stockCode, stockName });
      await loadWatchlist();
    } catch {
      // 静默处理
    }
  };

  const remove = async (id: string) => {
    try {
      await invoke("remove_from_watchlist", { id });
      await loadWatchlist();
    } catch {
      // 静默处理
    }
  };

  return (
    <div>
      <div className="flex justify-between items-center mb-2">
        <span className="text-xs font-semibold">{t("stockAnalysis.watchlist")}</span>
        <Button
          size="small"
          icon={<PlusOutlined />}
          disabled={!stockCode}
          onClick={addCurrent}
        />
      </div>
      <List
        size="small"
        dataSource={items}
        renderItem={(item) => (
          <List.Item
            style={{ cursor: "pointer" }}
            onClick={() => getStockQuote(item.stockCode)}
            actions={[
              <Popconfirm
                key="del"
                title={t("common.confirm")}
                onConfirm={() => remove(item.id)}
              >
                <Button size="small" type="text" danger icon={<DeleteOutlined />} />
              </Popconfirm>,
            ]}
          >
            <Tag>{item.stockCode}</Tag>
            <span className="text-xs">{item.stockName}</span>
          </List.Item>
        )}
      />
    </div>
  );
}
