const fs = require('fs');
const s = '\w\s\n';
fs.writeFileSync('regex_test_out.txt', s, 'utf-8');
console.log('Written, length:', s.length);
for (let i = 0; i < s.length; i++) {
  console.log('byte', i, ':', s.charCodeAt(i).toString(16), JSON.stringify(s[i]));
}
