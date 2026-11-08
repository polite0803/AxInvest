/**
 * 从变量描述文本中解析枚举候选值 —— 三个 ConfigPanel 的共享实现。
 *
 * 此前 `DemandDiscoveryConfigPanel` / `LiteraryCreationConfigPanel` /
 * `StockAnalysisConfigPanel` 各自复制了一份**逐字相同**的 `parseEnumOptions`
 * （归一化后 sha1 指纹一致），按 AGENTS.md 禁区 12 收敛为单一权威源，避免三份漂移。
 *
 * 约定：描述里以 `: ` 起头、用 `/` 分隔的片段即候选选项。
 * 例：`log_level` 的描述结尾 `... 可选值: info / warn / error`
 * → `["info", "warn", "error"]`。描述不含 `: ` 时返回空数组，调用方渲染为空下拉。
 */
export function parseEnumOptions(desc?: string): string[] {
  if (!desc) { return []; }
  const match = desc.match(/: (.+)/);
  if (match) { return match[1].split(/\s*\/\s*/).map((s) => s.trim()); }
  return [];
}
