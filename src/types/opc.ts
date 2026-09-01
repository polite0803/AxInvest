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
  /** 转化生成的实现工作流模板 ID（null = 未转化） */
  linkedWorkflowId: string | null;
  /** 首次启动实现工作流执行的时间戳（秒；null = 未执行） */
  implementedAt: number | null;
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
  /** 高价值线索明细（全局榜单，非本轮独有） */
  leads: DemandLead[];
  /** 本轮实际扫描评估到的线索明细（订阅推送按此过滤，避免串推） */
  roundLeads: DemandLead[];
}

/** 需求订阅（长期跟踪的关键词） */
export interface DemandSubscription {
  id: string;
  /** 订阅关键词（唯一） */
  keyword: string;
  enabled: boolean;
  /** 扫描间隔（小时） */
  intervalHours: number;
  /** 推送门槛：商业价值分低于此值不计入高价值命中 */
  minScore: number;
  /** 限定平台 ID 列表；空数组 = 跟随全局启用的平台 */
  platforms: string[];
  /** 最近一次扫描时间戳（秒）；null = 从未扫描 */
  lastScannedAt: number | null;
  /** 最近一次扫描的高价值命中数 */
  lastHitCount: number;
  createdAt: number;
  updatedAt: number;
}

/** 保存（新增或更新）需求订阅的输入 */
export interface SaveDemandSubscriptionInput {
  id?: string | null;
  keyword?: string | null;
  enabled?: boolean | null;
  intervalHours?: number | null;
  minScore?: number | null;
  platforms?: string[] | null;
}

/** 单个订阅词的扫描结果 */
export interface KeywordScanOutcome {
  subscriptionId: string;
  keyword: string;
  /** 扫描是否成功 */
  ok: boolean;
  /** 失败原因（ok=false 时） */
  error: string | null;
  /** 本词命中的高价值线索（已按 minScore 过滤） */
  hits: DemandLead[];
}

/** 一轮订阅扫描的汇总 */
export interface SubscriptionScanSummary {
  /** 本轮扫描的订阅词数 */
  scannedSubscriptions: number;
  /** 新入库线索总数 */
  totalSaved: number;
  /** 刷新评分的线索总数 */
  totalRefreshed: number;
  /** 命中推送门槛的高价值线索总数 */
  highValueHits: number;
  /** 逐词结果 */
  outcomes: KeywordScanOutcome[];
}

/** 单条能力的匹配命中项 */
export interface CapabilityMatchItem {
  capabilityId: string;
  name: string;
  /** 能力类型（tool / workflow / skill / agent / ...） */
  kind: string;
  /** 业务域（general / automation / devops / ...） */
  domain: string;
  /** 综合检索分 0.0-1.0 */
  retrievalScore: number;
  /** 一句话摘要（未声明时为 null） */
  summary: string | null;
}

/** 线索的能力匹配结论 */
export interface LeadCapabilityMatch {
  leadId: string;
  /** ready（可直接接）/ partial（部分覆盖）/ missing（能力缺失） */
  verdict: "ready" | "partial" | "missing";
  /** 最高检索分 */
  bestScore: number;
  /** 命中的能力（按检索分降序） */
  matches: CapabilityMatchItem[];
  /** 该需求类型要求的能力域 */
  requiredDomains: string[];
  /** 必需域中未被命中的部分 = 缺口 */
  missingDomains: string[];
  /** 缺口说明；无缺口时为 null */
  gapHint: string | null;
}

/** 交付发票：won 线索的账本行（draft → sent → paid 单向） */
export interface DeliveryInvoice {
  id: string;
  leadId: string;
  /** P2 转化出的交付工作流（人工交付时为 null） */
  linkedWorkflowId: string | null;
  title: string;
  /** 金额（多币种并存，汇总按币种分组） */
  amount: number;
  /** ISO 4217 币种 */
  currency: string;
  /** draft / sent / paid */
  status: "draft" | "sent" | "paid";
  issuedAt: number | null;
  paidAt: number | null;
  notes: string | null;
  createdAt: number;
  updatedAt: number;
}

/** 开票入参：缺省字段由后端从线索元数据自动填充 */
export interface CreateInvoiceFromLeadInput {
  title?: string;
  amount?: number;
  currency?: string;
  notes?: string;
}

/** 单币种的回款小计 */
export interface RevenueByCurrency {
  currency: string;
  /** 已回款（paid）总额 */
  paidTotal: number;
  /** 已开出（sent + paid）总额 */
  issuedTotal: number;
}

/** 交付环节汇总（转化率只统计，不自动回写评分权重） */
export interface DeliverySummary {
  wonLeads: number;
  /** 非 lost 线索总数（转化率分母） */
  activeLeads: number;
  invoiceCount: number;
  paidCount: number;
  revenues: RevenueByCurrency[];
  /** won / active，active 为 0 时为 0 */
  conversionRate: number;
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
