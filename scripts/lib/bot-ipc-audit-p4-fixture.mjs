// SPDX-License-Identifier: AGPL-3.0-only
/**
 * bot-ipc-audit P4 段的合成源（供 selftest 正控 / 单点破坏用）。
 * 与真实仓库无关：这里只保证「完整 P4 契约 ⇒ 0 错」，破坏任一项 ⇒ 对应红。
 * P4-b（待答请求）在 B3 卡接入：本 fixture 的 `inputRs` 缺省表示「未接入」，
 * 审计段的待答检查据此跳过，保证两条切片可独立收口（ADR-P4-7）。
 */
const NL = String.fromCharCode(10);

const P4_QUOTA_COMMANDS = [
  ["get_bot_department_quota", "bot_quota"],
  ["set_bot_department_quota", "bot_quota"],
  ["reset_bot_department_quota", "bot_quota"],
];

const QUOTA_FIELDS = [
  "department_id", "max_concurrency", "total_timeout_secs", "max_steps",
  "max_in_flight_per_depth", "step_timeout_secs", "max_concurrent_runs",
  "is_default", "updated_at",
];
const QUOTA_INPUT_FIELDS = QUOTA_FIELDS.filter((f) => f !== "is_default" && f !== "updated_at");

const camel = (f) => f.replace(/_([a-z])/g, (_, c) => c.toUpperCase());

export function decorateWithP4(sources) {
  const quotaRegs = P4_QUOTA_COMMANDS
    .map(([name, mod]) => "        commands::" + mod + "::" + name + ",").join(NL);
  sources.registerRs = sources.registerRs.replace("    ]", quotaRegs + NL + "    ]");

  const rustStruct = (name, fields) => "#[derive(Serialize, Deserialize)]" + NL
    + '#[serde(rename_all = "camelCase")]' + NL
    + "pub struct " + name + " {" + NL
    + fields.map((f) => "    pub " + f + ": String,").join(NL) + NL + "}" + NL;
  sources.typesRs += NL + rustStruct("BotDepartmentQuota", QUOTA_FIELDS)
    + rustStruct("BotDepartmentQuotaInput", QUOTA_INPUT_FIELDS);

  const tsStruct = (name, fields) => "export interface " + name + " {" + NL
    + fields.map((f) => "  " + camel(f) + ": string;").join(NL) + NL + "}" + NL;
  sources.typesTs += NL + tsStruct("BotDepartmentQuotaDto", QUOTA_FIELDS)
    + tsStruct("BotDepartmentQuotaInputDto", QUOTA_INPUT_FIELDS);

  sources.p4 = {
    quotaRs: P4_QUOTA_COMMANDS
      .map(([name]) => "pub async fn " + name + "() { let _ = BOT_QUOTA_INVALID; }").join(NL),
    quotaEnforceRs: "let _ = BOT_QUOTA_EXCEEDED;",
    agentRs: 'pub async fn agent_respond_ask(app_state: State<AppState>, '
      + 'request: AgentRespondAskRequest) -> Result<(), String> { todo!() }',
  };
  return sources;
}
