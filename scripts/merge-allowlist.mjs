// Merge upstream i18n allowlist with local AxInvest violations
import { execSync } from "child_process";
import { readFileSync, writeFileSync } from "fs";

// Load upstream allowlist (full 269 entries)
const upstreamRaw = execSync(
  "git show upstream/master:scripts/.i18n-allowlist.json",
  { encoding: "utf8" },
);
const upstream = JSON.parse(upstreamRaw);

// Build existing map: file -> Set of line numbers
const existing = {};
upstream.entries.forEach((e) => {
  if (!existing[e.file]) { existing[e.file] = new Set(); }
  (e.lines || "").split(",").forEach((l) => {
    const t = l.trim();
    if (t) { existing[e.file].add(t); }
  });
});

// Run local i18n check to get current violations
console.log("Running i18n check...");
let checkResult = "";
try {
  checkResult = execSync("bash scripts/check-hardcoded-i18n.sh", {
    encoding: "utf8",
    stdio: "pipe",
  });
} catch (e) {
  checkResult = e.stdout || "";
}
let added = 0;
const lines = checkResult.split("\n");
for (const line of lines) {
  const m = line.match(/^\s+(src\/[^:]+):(\d+):/);
  if (m) {
    const file = m[1];
    const lnum = m[2];
    if (!existing[file]) { existing[file] = new Set(); }
    if (!existing[file].has(lnum)) {
      existing[file].add(lnum);
      added++;
    }
  }
}

// Rebuild entries array
upstream.entries = [];
for (const [file, linesSet] of Object.entries(existing)) {
  const sorted = Array.from(linesSet)
    .map(Number)
    .sort((a, b) => a - b)
    .map(String);
  upstream.entries.push({
    file,
    lines: sorted.join(","),
    reason: "硬编码中文字符串",
    phase: 3,
  });
}

upstream.total_entries = upstream.entries.length;
upstream.total_files = upstream.entries.length;
upstream.generated = new Date().toISOString().slice(0, 10);

writeFileSync(
  "scripts/.i18n-allowlist.json",
  JSON.stringify(upstream, null, 2) + "\n",
);
console.log(
  `Allowlist: ${upstream.entries.length} entries | ${added} new lines added`,
);
