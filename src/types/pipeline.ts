// SPDX-License-Identifier: AGPL-3.0-only

/** 管道执行结果 */
export interface PipelineResult {
  runId: string;
  runDate: string;
  status: string;
  candidates: string[];
  newAnalyses: PipelineAnalysisSummary[];
  reassessed: PipelineAnalysisSummary[];
  summary: PipelineSummary;
  error: string | null;
}

/**
 * 单只股票分析摘要（管道上下文）。
 * 与 stock-analysis.ts 中的 AnalysisSummary 字段不同，故独立命名为 PipelineAnalysisSummary。
 */
export interface PipelineAnalysisSummary {
  stockCode: string;
  stockName: string;
  status: string;
  analysisId: string | null;
  action: string | null;
  confidence: number | null;
  error: string | null;
}

/** 管道汇总报告 */
export interface PipelineSummary {
  pipelineDate: string;
  discovery: { candidatesFound: number };
  analysis: {
    newAnalyzed: number;
    newFailed: number;
    reassessed: number;
    reassessFailed: number;
  };
  decisions: {
    buy: number;
    hold: number;
    watch: number;
    sell: number;
  };
  reflectionScheduled: number;
  note: string;
}

/** 管道历史记录（列表项） */
export interface PipelineRun {
  id: string;
  runDate: string;
  asOfDate: string | null;
  status: string;
  startedAt: number;
  completedAt: number | null;
  errorMessage: string | null;
  summary: PipelineSummary | null;
}

/**
 * 管道进度事件（`pipeline-step`）。
 *
 * **为什么拆成 `nodeId` + `status` 两个机器可读字段**：历史实现由后端 `format!` 拼出
 * `"<nodeId>: 执行中"` 塞进 `detail`，前端原样渲染 ⇒ 中文硬编码漏到界面，
 * 同一处还混着英文 `"failed"`/`"timeout"`（三语混杂），且非中文用户看到中文。
 * 现在**文案由前端按 i18n 组装**（`pipeline.stepRunning` 等），后端只发结构化值。
 */
export interface PipelineStepEvent {
  /** 步骤标识（当前恒为 `pipeline_step`，由后端 `stock_pipeline/core.rs` 传入） */
  step: string;
  /** 节点 ID（如 `p1-discovery`） */
  nodeId?: string;
  /** 节点状态：`running` / `completed` / `failed` / `timeout` */
  status?: string;
  /**
   * @deprecated 后端拼接好的中文文案。**不要用于展示**（中文硬编码漏到界面、跨语言失效）；
   * 仅作旧载荷的展示兜底（`nodeId`/`status` 缺失时）。
   */
  detail?: string;
  timestamp: number;
}
