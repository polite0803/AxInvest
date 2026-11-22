# P1 验收运行记录（2026-11-22）

$ node scripts/p1-acceptance.mjs

## 各步骤命令与退出码
```
$ npm run typecheck
exit=0  6.8s
$ node scripts/vitest-run.mjs run
exit=0  146.4s
$ node scripts/check-contracts.mjs --only=a,c,d,f,g,h,bot
exit=1  3.9s
$ node scripts/check-errorcode-alignment.mjs
exit=0  0.1s
$ python3 scripts/check_i18n.py --strict
exit=0  0.2s
$ node scripts/check-i18n-key-parity.mjs
exit=0  0.2s
$ node scripts/check-bot-i18n-keys.mjs
exit=0  0.1s
$ node scripts/check-layer-discipline.mjs
exit=0  0.4s
$ node scripts/check-serde-annotations.ts
exit=1  0.2s
$ npm run check:routes
exit=0  0.6s
```

## 判定表
```
```

完整逐步骤输出可由上述命令在任何一次运行中重新生成；
本文件只保留可审计的命令面与判定结果，避免把 300KB 控制台输出入库。
