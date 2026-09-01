// SPDX-License-Identifier: AGPL-3.0-only

// ! OPC 需求发现（Demand Discovery）类型
// !
// ! 与后端 `crates/harness/src/types/opc_demand.rs` 的 serde camelCase 输出一一对应；
// ! 数据落地见 v131 migration（opc_demand_platforms / opc_demand_leads）。

/** 需求平台配置 */
export interface DemandPlatform {
  /** 平台标识（与内置扫描器 platform() 一致，如 "reddit"） */
  id: string;
  /** 展示名 */
  name: string;
  /** 连接器类型：api / scanner / mock / manual */
  platformType: string;
  /** 是否启用 */
  enabled: boolean;
  /** 平台基础 URL（api 类型可覆盖默认端点） */
  baseUrl: string | null;
  /** 连接器扩展配置 */
  config: Record<string, unknown> | null;
  /** 最近一次扫描成功时间戳（秒），null 表示从未扫描 */
  lastSyncAt: number | null;
  /** 连接器状态：idle / ok / error */
  status: string;
  createdAt: number;
  updatedAt: number;
}

/** 保存（新增或更新）需求平台配置的输入 */
export interface SaveDemandPlatformInput {
  /** 为空表示新增（后端自动生成 id），否则按 id 部分更新 */
  id?: string;
  name?: string;
  platformType?: string;
  enabled?: boolean;
  baseUrl?: string;
  config?: Record<string, unknown>;
}

/** 需求线索（带评估结果） */
export interface DemandLead {
  id: string;
  /** 来源平台标识 */
  platform: string;
  title: string;
  description: string;
  budgetMin: number | null;
  budgetMax: number | null;
  budgetCurrency: string;
  contactName: string | null;
  contactEmail: string | null;
  contactPhone: string | null;
  sourceUrl: string | null;
  /** 生命周期：new / evaluated / contacted / won / lost */
  status: string;
  /** 评估置信度 0-1 */
  confidence: number;
  /** 痛点强度 0-100 */
  painScore: number;
  /** 市场空白度 0-100 */
  marketGapScore: number;
  /** 商业价值综合分 0-100（veryHigh ≥ 80） */
  commercialValueScore: number;
  /** 等级：low / medium / high / very_high */
  opportunityLevel: string;
  /** 需求类型（snake_case 标识） */
  demandType: string;
  createdAt: number;
  updatedAt: number;
}

/** 一轮「扫描 → 评估 → 持久化」的执行摘要 */
export interface DiscoverLeadsSummary {
  /** 本轮扫到的原始线索总数 */
  totalScanned: number;
  /** 完成评估的线索数 */
  totalEvaluated: number;
  /** 新入库数（去重后跳过的不计） */
  totalSaved: number;
  /** 命中去重窗口外的同源线索、被刷新评分的条数 */
  totalRefreshed: number;
  /** 其中高价值（commercialValueScore ≥ 60）数量 */
  highValueCount: number;
  /** 高价值线索明细 */
  leads: DemandLead[];
}

/** 扫描策略：并发 / 限流 / 重试 / 去重窗口 */
export interface ScanPolicy {
  /** 并发扫描的平台数上限（1-32） */
  concurrency: number;
  /** 全局速率上限（次/分钟）；0 = 不限速 */
  rateLimitPerMin: number;
  /** 单平台失败后的最大重试次数（不含首次，上限 5） */
  retryMax: number;
  /** 重试退避基数（毫秒）；第 n 次重试等待 base * 2^(n-1) */
  retryBackoffMs: number;
  /** 单平台单次请求超时（秒，1-120） */
  timeoutSecs: number;
  /** 去重时间窗口（小时）；0 = 永久去重 */
  dedupWindowHours: number;
  /** 单次扫描保留的线索数上限（1-5000） */
  maxLeadsPerScan: number;
}
