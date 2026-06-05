import { useSettingsStore, useStockAnalysisStore } from "@/stores";
import { ExpandOutlined } from "@ant-design/icons";
import { Button, Card, Collapse, Empty, Modal, Tag } from "antd";
import NodeRenderer from "markstream-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { cleanToolCallTags } from "./utils";

/**
 * 价值投资评估面板
 * 显示 value-investor 节点（巴菲特框架）的输出，附带 3 个预留字段
 * （ruleCheckResults / dataQualitySummary / rawData）的空状态展示。
 *
 * 数据来源:
 * - valueAssessments["value-investor"]: 巴菲特框架评估（工作流产出，活跃）
 * - ruleCheckResults / dataQualitySummary / rawData: 预留字段，工作流尚未
 *   产出这些节点。当前显示空态，标注"待工作流启用"。
 */
export function ValueAssessmentPanel() {
  const { t } = useTranslation();
  const themeMode = useSettingsStore((s) => s.settings.theme_mode);
  const isDark = themeMode === "dark"
    || (themeMode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  const valueAssessments = useStockAnalysisStore((s) => s.valueAssessments);
  const ruleCheckResults = useStockAnalysisStore((s) => s.ruleCheckResults);
  const dataQualitySummary = useStockAnalysisStore((s) => s.dataQualitySummary);
  const rawData = useStockAnalysisStore((s) => s.rawData);
  const [expanded, setExpanded] = useState(false);

  const valueReport = valueAssessments["value-investor"] ?? "";
  const hasValue = valueReport.trim().length > 0;
  const hasRuleCheck = Object.keys(ruleCheckResults).length > 0;
  const hasDataQuality = dataQualitySummary.trim().length > 0;
  const hasRawData = Object.keys(rawData).length > 0;
  const hasAny = hasValue || hasRuleCheck || hasDataQuality || hasRawData;

  if (!hasAny) {
    return (
      <div className="p-6">
        <Empty
          description={t("stockAnalysis.valueAssessment.empty")}
          image={Empty.PRESENTED_IMAGE_SIMPLE}
        />
      </div>
    );
  }

  return (
    <div className="p-4 space-y-3">
      {hasValue && (
        <Card
          size="small"
          title={
            <div className="flex items-center gap-2">
              <Tag color="gold">巴菲特</Tag>
              <span className="text-sm">{t("stockAnalysis.valueAssessment.title")}</span>
            </div>
          }
          extra={
            <Button
              type="text"
              size="small"
              icon={<ExpandOutlined />}
              onClick={() => setExpanded(true)}
            >
              展开
            </Button>
          }
        >
          <div className={`prose max-w-none text-sm ${isDark ? "prose-invert" : ""}`}>
            <NodeRenderer
              content={cleanToolCallTags(valueReport)}
              isDark={isDark}
            />
          </div>
        </Card>
      )}

      {(hasRuleCheck || hasDataQuality || hasRawData) && (
        <Collapse
          ghost
          items={[{
            key: "future",
            label: t("stockAnalysis.valueAssessment.futureFields"),
            children: (
              <div className="space-y-2 text-sm">
                {hasRuleCheck && (
                  <FieldBlock
                    title={t("stockAnalysis.valueAssessment.ruleCheck")}
                    content={JSON.stringify(ruleCheckResults, null, 2)}
                  />
                )}
                {hasDataQuality && (
                  <FieldBlock title={t("stockAnalysis.valueAssessment.dataQuality")} content={dataQualitySummary} />
                )}
                {hasRawData && (
                  <FieldBlock
                    title={t("stockAnalysis.valueAssessment.rawData")}
                    content={JSON.stringify(rawData, null, 2)}
                  />
                )}
              </div>
            ),
          }]}
        />
      )}

      <Modal
        open={expanded}
        onCancel={() => setExpanded(false)}
        footer={null}
        width={800}
        title={t("stockAnalysis.valueAssessment.title")}
      >
        <div className={`prose max-w-none text-sm ${isDark ? "prose-invert" : ""}`}>
          <NodeRenderer
            content={cleanToolCallTags(valueReport)}
            isDark={isDark}
          />
        </div>
      </Modal>
    </div>
  );
}

function FieldBlock({ title, content }: { title: string; content: string }) {
  return (
    <div>
      <div className="text-xs text-gray-500 mb-1">{title}</div>
      <pre className="bg-gray-50 dark:bg-gray-900 p-2 rounded text-xs overflow-x-auto whitespace-pre-wrap">
        {content}
      </pre>
    </div>
  );
}
