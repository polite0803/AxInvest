const fs = require('fs');
const path = 'src/lib/agentOutput.ts';
let code = fs.readFileSync(path, 'utf-8');

const orphanLine = code.indexOf('(?:invoke|function|parameter)');
if (orphanLine < 0) { console.log('NOT FOUND'); process.exit(1); }

const lineStart = code.lastIndexOf('\n', orphanLine) + 1;
const originalLine = code.slice(lineStart, code.indexOf('\n', orphanLine));
console.log('Original:', originalLine.trim().slice(0, 60));

const blockRegex = '  // Orphan invoke/function/parameter with content\n' +
  '  cleaned = cleaned.replace(/<(?:[a-z][\w-]*:)?(?:invoke|function|parameter)\b[^>]*>[\s\S]*?<\/(?:[a-z][\w-]*:)?(?:invoke|function|parameter)\b[^>]*>/gi, "");\n';

code = code.slice(0, lineStart) + blockRegex + code.slice(lineStart);
fs.writeFileSync(path, code, 'utf-8');

const { execSync } = require('child_process');
const tsc = execSync('npx tsc --noEmit', {cwd: process.cwd()});
console.log('OK');
