const d = require(process.env.TEMP + "\\eslint-out.json");
d.forEach(f =>
  f.messages.filter(m => m.ruleId === "no-duplicate-case").forEach(m => {
    const start = Math.max(1, m.line - 3);
    const lines = f.source.split("\n").slice(start - 1, m.endLine + 2);
    lines.forEach((l, i) => {
      if (start + i >= m.line && start + i <= m.endLine) {
        console.log(">>" + (start + i) + ":" + l);
      } else {
        console.log("  " + (start + i) + ":" + l);
      }
    });
  })
);
