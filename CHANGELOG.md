# Changelog

All notable changes to AxAgent will be documented in this file.
## [v3.1.1] - 2026-11-27

### 🎨 Styling
- rustfmt 收口 13 个 Bot 集成文件，修 CI fmt 门禁
- dprint 收口 OPC 导航元数据表对齐空格（随 3f189dcc3 全仓格式化）


### 🐛 Bug Fixes
- 消除 clippy redundant_closure，修 CI Infra Clippy 红
- 收口 CI 全红并提交全部在途改动（Bot 集成 P1-P5 / invest 路由下线 / 前端导航）
- 修 CI run 45721938652 两处红灯（L1 白名单腐烂 / selftest CI 布局）
- OPC_DOMAIN_PAGE_NAV 前移到 PLUGIN_SPECS 之前，修打包 TDZ 崩溃
- 修 run 45729002592 剩余两红（裸 map_err ×2 / horizon 判据 D 失效）
- 收口 run 45736845533 三红（rustfmt / dprint / SDK 快照路径）
- BotSkillsPanel 预设模板示例描述改英文，修 CI 硬编码中文红
- BotsPage 三处 ID 拼接改 safeJoinIds，修 ID validation 红
- ID validation 仅盯写入形态，消掉 38 条读取比较假阳性
- 收口 Rust Check 与 Unit Tests 两处红
- 消除编排链路在 tokio runtime 内的阻塞锁 panic
- 消除全量 Vitest 收尾后 antd 帧回调的未捕获异常
- 用全局 setTimeout 兜底消除跨 teardown 的 antd setState
- 修复打包脚本 TDZ 并同步钉版
- 更新股票插件页断言以适配 T3 外壳
- 修复插件页 asset URL 与 CSP 导致的加载超时
- 修复工具集注册冲突与导航图标缺失
- 修复 clippy map_clone 与隐藏页导航门禁
- 远程工具集断言改用扩展注册表
- E2E 断言改用插件工具集扩展列表
- 同版本 bundled 清单变更也触发整目录同步
- 修复 worker 占用导致的插件列表整体失败


### 📝 Documentation
- 登记 Obsidian 知识库引用索引


### 📦 Miscellaneous
- bump version to v3.1.1
- 退役文档类CI门禁，纯文档迁Obsidian权威源
- 清理2个未引用的专家prompt文件
- bump 17 个插件源钉版到修复提交
- 同步 OPC 插件源钉版到 ready 修复提交


### 🔨 Refactoring
- pickVisibleAskUsers 下沉到 lib，修组件中立性红


### 🧪 Testing
- 插件导航元数据消费面门禁接入 CI（check-plugin-nav 9 项）
- 删除断言已外迁宿主 UI 的反思面板 spec，CI 三红收口

