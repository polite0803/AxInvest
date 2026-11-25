// SPDX-License-Identifier: AGPL-3.0-only
/**
 * bot-ipc-audit P5 段的合成源（供 selftest 正控 / 单点破坏用）。
 * 与真实仓库无关：这里只保证「完整 P5 契约 ⇒ 0 错」，破坏任一项 ⇒ 对应红。
 */
const NL = String.fromCharCode(10);

const P5_COST_COMMANDS = [
  "get_bot_cost_quota",
  "set_bot_cost_quota",
  "reset_bot_cost_quota",
  "list_bot_usage",
  "get_bot_usage_summary",
];

const P5_SKILL_COMMANDS = [
  "list_bot_skills",
  "set_bot_skill_enabled",
  "import_bot_skill_preset",
];

const COST_QUOTA_FIELDS = [
  "scope", "scope_id", "max_total_tokens", "max_cost_usd", "window_hours",
  "is_default", "updated_at",
];
const COST_QUOTA_INPUT_FIELDS = [
  "scope", "scope_id", "max_total_tokens", "max_cost_usd", "window_hours",
];
const USAGE_ENTRY_FIELDS = [
  "id", "bot_id", "department_id", "conversation_id", "execution_id",
  "workflow_run_id", "assignment_id", "model_id", "origin",
  "input_tokens", "output_tokens", "cache_creation_tokens", "cache_read_tokens",
  "total_tokens", "estimated_cost_usd", "pricing_source", "created_at",
];
const USAGE_FILTER_FIELDS = [
  "bot_id", "department_id", "origin", "workflow_run_id", "assignment_id",
  "since", "until",
];
const USAGE_SUMMARY_FIELDS = [
  "scope", "scope_id", "window_hours", "input_tokens", "output_tokens",
  "cache_creation_tokens", "cache_read_tokens", "total_tokens",
  "estimated_cost_usd", "entry_count", "breached", "pricing_note",
];
const SKILL_FIELDS = [
  "name", "description", "source", "enabled", "shadowed_global", "missing",
  "preset_id", "updated_at",
];
const SKILL_PRESET_FIELDS = ["version", "name", "description", "skills"];
const SKILL_PRESET_ITEM_FIELDS = [
  "name", "description", "content", "category", "tags", "requires_toolsets",
];
const SKILL_PRESET_SKIPPED_FIELDS = ["name", "reason", "detail"];
const SKILL_PRESET_RESULT_FIELDS = ["preset_id", "imported", "skipped"];

const camel = (f) => f.replace(/_([a-z])/g, (_, c) => c.toUpperCase());

const rustStruct = (name, fields) => "#[derive(Serialize, Deserialize)]" + NL
  + '#[serde(rename_all = "camelCase")]' + NL
  + "pub struct " + name + " {" + NL
  + fields.map((f) => "    pub " + f + ": String,").join(NL) + NL + "}" + NL;

const tsStruct = (name, fields) => "export interface " + name + " {" + NL
  + fields.map((f) => "  " + camel(f) + ": string;").join(NL) + NL + "}" + NL;

export function decorateWithP5(sources) {
  // register_commands registrations
  const regs = [
    ...P5_COST_COMMANDS.map((name) => "        commands::bot_cost::" + name + ","),
    ...P5_SKILL_COMMANDS.map((name) => "        commands::bot_skill::" + name + ","),
  ].join(NL);
  sources.registerRs = sources.registerRs.replace("    ]", regs + NL + "    ]");

  // DTOs
  sources.typesRs += NL
    + rustStruct("BotCostQuota", COST_QUOTA_FIELDS)
    + rustStruct("BotCostQuotaInput", COST_QUOTA_INPUT_FIELDS)
    + rustStruct("BotUsageEntry", USAGE_ENTRY_FIELDS)
    + rustStruct("BotUsageFilter", USAGE_FILTER_FIELDS)
    + rustStruct("BotUsageSummary", USAGE_SUMMARY_FIELDS)
    + rustStruct("BotSkill", SKILL_FIELDS)
    + rustStruct("BotSkillPreset", SKILL_PRESET_FIELDS)
    + rustStruct("BotSkillPresetItem", SKILL_PRESET_ITEM_FIELDS)
    + rustStruct("BotSkillPresetSkipped", SKILL_PRESET_SKIPPED_FIELDS)
    + rustStruct("BotSkillPresetResult", SKILL_PRESET_RESULT_FIELDS);

  sources.typesTs += NL
    + tsStruct("BotCostQuotaDto", COST_QUOTA_FIELDS)
    + tsStruct("BotCostQuotaInputDto", COST_QUOTA_INPUT_FIELDS)
    + tsStruct("BotUsageEntryDto", USAGE_ENTRY_FIELDS)
    + tsStruct("BotUsageFilterDto", USAGE_FILTER_FIELDS)
    + tsStruct("BotUsageSummaryDto", USAGE_SUMMARY_FIELDS)
    + tsStruct("BotSkillDto", SKILL_FIELDS)
    + tsStruct("BotSkillPresetDto", SKILL_PRESET_FIELDS)
    + tsStruct("BotSkillPresetItemDto", SKILL_PRESET_ITEM_FIELDS)
    + tsStruct("BotSkillPresetSkippedDto", SKILL_PRESET_SKIPPED_FIELDS)
    + tsStruct("BotSkillPresetResultDto", SKILL_PRESET_RESULT_FIELDS);

  // frontend call stubs
  sources.frontendFiles[0].src += NL
    + 'await invoke<unknown>("get_bot_cost_quota", { scope: "bot", scopeId: id });' + NL
    + 'await invoke<unknown>("set_bot_cost_quota", { input: {} });' + NL
    + 'await invoke<unknown>("reset_bot_cost_quota", { scope: "bot", scopeId: id });' + NL
    + 'await invoke<unknown>("list_bot_usage", { filter: {} });' + NL
    + 'await invoke<unknown>("get_bot_usage_summary", { scope: "bot", scopeId: id, windowHours: 168 });' + NL
    + 'await invoke<unknown>("list_bot_skills", { botId: id });' + NL
    + 'await invoke<unknown>("set_bot_skill_enabled", { botId: id, skillName: "x", enabled: true });' + NL
    + 'await invoke<unknown>("import_bot_skill_preset", { botId: id, preset: {} });';

  sources.p5 = {
    costRs: P5_COST_COMMANDS.map((name) =>
      "pub async fn " + name + "(state: State<AppState>) -> Result<(), String> { " +
      "let _ = BOT_COST_QUOTA_INVALID; let _ = BOT_COST_QUOTA_EXCEEDED; Ok(()) }"
    ).join(NL),
    skillRs: P5_SKILL_COMMANDS.map((name) =>
      "pub async fn " + name + "(state: State<AppState>) -> Result<(), String> { " +
      "let _ = BOT_SKILL_NOT_FOUND; let _ = BOT_SKILL_PRESET_INVALID; Ok(()) }"
    ).join(NL),
    quotaEnforceRs: 'let _ = BOT_COST_QUOTA_EXCEEDED; // P5 cost breach check',
  };
  return sources;
}
