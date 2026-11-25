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

const P4_INPUT_COMMANDS = [
  "list_bot_input_requests",
  "answer_bot_input_request",
  "cancel_bot_input_request",
];

// 合成源里的缩进占位（保持 rustStruct 的字段风格一致，不参与断言）。
const QUOTA_NOOP = '    pub const PENDING: &str = ';

export function decorateWithP4(sources) {
  const quotaRegs = P4_QUOTA_COMMANDS
    .map(([name, mod]) => "        commands::" + mod + "::" + name + ",").join(NL);
  const inputRegs = P4_INPUT_COMMANDS
    .map((name) => "        commands::bot_input::" + name + ",").join(NL);
  sources.registerRs = sources.registerRs.replace(
    "    ]",
    quotaRegs + NL + inputRegs + NL + "    ]",
  );

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

  const INPUT_FIELDS = [
    "id", "ask_id", "conversation_id", "bot_id", "workflow_run_id", "assignment_id",
    "origin", "question", "options", "status", "answer", "answered_at", "expires_at",
    "created_at", "updated_at",
  ];
  sources.typesRs += NL + rustStruct("BotInputRequest", INPUT_FIELDS)
    + "pub mod input_status {" + NL
    + QUOTA_NOOP + '"pending";' + NL + '    pub const ANSWERED: &str = "answered";' + NL
    + '    pub const EXPIRED: &str = "expired";' + NL
    + '    pub const CANCELLED: &str = "cancelled";' + NL + "}" + NL
    + "pub mod input_origin {" + NL
    + '    pub const PERSON: &str = "person";' + NL
    + '    pub const ROUTINE: &str = "routine";' + NL
    + '    pub const WORKFLOW: &str = "workflow";' + NL + "}" + NL;
  sources.typesTs += NL + tsStruct("BotInputRequestDto", INPUT_FIELDS);
  sources.eventsEnum = sources.eventsEnum.replace("}", "    BotInputRequestUpdated," + NL + "}");
  sources.eventsTs = sources.eventsTs.replace("]", '  "bot-input-request-updated",' + NL + "]");

  sources.p4 = {
    quotaRs: P4_QUOTA_COMMANDS
      .map(([name]) => "pub async fn " + name + "() { let _ = BOT_QUOTA_INVALID; }").join(NL),
    quotaEnforceRs: "let _ = BOT_QUOTA_EXCEEDED;",
    agentRs: 'pub async fn agent_respond_ask(app_state: State<AppState>, '
      + 'request: AgentRespondAskRequest) -> Result<(), String> { todo!() }',
    inputRs: P4_INPUT_COMMANDS
      .map((name) => "pub async fn " + name + "(state: State<AppState>) -> Result<(), String> { todo!() }")
      .join(NL),
    inputHelperRs: "pub use axagent_harness::types::bot::input_status::{PENDING, ANSWERED};" + NL      + "pub fn needs(status: &str) -> bool { status == input_status::PENDING }" + NL
      + "pub fn emit(app: &AppHandle, r: &BotInputRequest) { let _ = "
      + "axagent_harness::IpcEventName::BotInputRequestUpdated.as_str(); }",
  };
  return sources;
}
