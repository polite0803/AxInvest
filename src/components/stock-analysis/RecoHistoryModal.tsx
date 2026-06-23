import { invoke } from "@/lib/invoke";
import { Button, Checkbox, message, Modal, Table, Tag } from "antd";
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";

export function RecoHistoryModal() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [data, setData] = useState<
    Array<{
      generatedAt: string;
      period: string;
      stockCount: number;
      styles: string;
      createdAt: string;
    }>
  >([]);
  const [loading, setLoading] = useState(false);
  const [selected, setSelected] = useState<string[]>([]);
  const [deleting, setDeleting] = useState(false);

  const loadData = useCallback(async () => {
    setLoading(true);
    try {
      const list = await invoke<typeof data>("list_reco_history", { limit: 50 });
      setData(list ?? []);
    } catch { /* */ }
    setLoading(false);
  }, []);

  return (
    <>
      <Button
        size="small"
        onClick={() => {
          setOpen(true);
          loadData();
        }}
      >
        {t("stockAnalysis.recommendation.recoHistory.viewHistory")}
      </Button>
      <Modal
        title={t("stockAnalysis.recommendation.recoHistory.title")}
        open={open}
        onCancel={() => {
          setOpen(false);
          setSelected([]);
        }}
        footer={selected.length > 0
          ? (
            <div className="flex items-center gap-2">
              <span className="text-xs text-gray-400">
                {t("stockAnalysis.recommendation.recoHistory.selectedCount", { count: selected.length })}
              </span>
              <Button size="small" onClick={() => setSelected([])}>
                {t("stockAnalysis.recommendation.recoHistory.exitSelect")}
              </Button>
              <Button
                size="small"
                danger
                loading={deleting}
                onClick={async () => {
                  setDeleting(true);
                  try {
                    await invoke("batch_delete_reco_history", { generatedAts: selected });
                    message.success(
                      t("stockAnalysis.recommendation.recoHistory.deleteSuccess", { count: selected.length }),
                    );
                    setData((prev) => prev.filter((r) => !selected.includes(r.generatedAt)));
                    setSelected([]);
                  } catch (e) {
                    message.error(String(e));
                  }
                  setDeleting(false);
                }}
              >
                {t("stockAnalysis.recommendation.recoHistory.batchDelete", { count: selected.length })}
              </Button>
            </div>
          )
          : null}
        width={640}
      >
        <Table
          size="small"
          loading={loading}
          dataSource={data}
          rowKey="generatedAt"
          pagination={false}
          columns={[
            {
              title: (
                <Checkbox
                  checked={data.length > 0 && selected.length === data.length}
                  indeterminate={selected.length > 0 && selected.length < data.length}
                  onChange={(e) => {
                    setSelected(e.target.checked ? data.map((r) => r.generatedAt) : []);
                  }}
                />
              ),
              key: "select",
              width: 40,
              render: (_, r) => (
                <Checkbox
                  checked={selected.includes(r.generatedAt)}
                  onChange={(e) => {
                    setSelected(
                      e.target.checked
                        ? [...selected, r.generatedAt]
                        : selected.filter((g) => g !== r.generatedAt),
                    );
                  }}
                />
              ),
            },
            {
              title: t("stockAnalysis.recommendation.recoHistory.generatedAt"),
              dataIndex: "generatedAt",
              key: "generatedAt",
              render: (v: string) => (
                <span className="text-xs font-mono">
                  {new Date(v).toLocaleString()}
                </span>
              ),
            },
            {
              title: t("stockAnalysis.recommendation.recoHistory.period"),
              dataIndex: "period",
              key: "period",
              render: (v: string) => {
                const periodKey = `stockAnalysis.period${v.charAt(0).toUpperCase() + v.slice(1)}`;
                return (
                  <Tag className="text-[10px] m-0">
                    {t(periodKey)}
                  </Tag>
                );
              },
            },
            {
              title: t("stockAnalysis.recommendation.recoHistory.stockCount"),
              dataIndex: "stockCount",
              key: "stockCount",
              render: (v: number) => <span className="text-xs">{v}</span>,
            },
            {
              title: t("stockAnalysis.recommendation.recoHistory.styles"),
              dataIndex: "styles",
              key: "styles",
              render: (v: string) => (
                <div className="flex gap-1 flex-wrap">
                  {v.split(",").map((s) => <Tag key={s} className="text-[10px] m-0">{s}</Tag>)}
                </div>
              ),
            },
          ]}
        />
      </Modal>
    </>
  );
}
