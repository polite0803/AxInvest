// SPDX-License-Identifier: AGPL-3.0-only
//! Plugin system: discovery, lifecycle, registry, and management.

pub mod core;
pub mod manager;
pub mod sandbox;
pub mod source_audit;
pub mod source_build;
pub mod types;
pub mod ui_schema;

pub mod agent_provider;
mod hooks;
pub mod mcp_launcher;
pub mod skill_installer;
pub mod test_isolation;
pub mod worker;

pub use core::*;
pub use hooks::{HookEvent, HookRunResult, HookRunner};
pub use manager::*;
pub use mcp_launcher::{McpLaunchError, McpLauncher};
pub use sandbox::{
    SandboxConfig, apply_env_to_command, build_sandbox_from_manifest,
    build_sandbox_from_permissions, check_path_permission, check_subprocess_permission,
    default_denied_paths, filter_env_vars, is_env_allowed, note_network_access,
};
pub use skill_installer::SkillInstaller;
pub use source_audit::*;
pub use source_build::*;
pub use types::*;
pub use worker::{
    LoadedPlugin, OP_UI_ACTION, PluginWorkerConfig, SUPPORTED_REMOTE_SEAMS, WorkerError,
    is_supported_remote_seam,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::test_support::isolated_config;
    use serde_json::Value;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn env_guard() -> parking_lot::MutexGuard<'static, ()> {
        crate::manager::env_lock().lock()
    }

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("plugins-{label}-{nanos}"))
    }

    /// 子进程门控：`aggregates_and_executes_plugin_tools` 会**启动真实 external plugin 子进程**
    /// 来执行工具，在 CI 里曾因启动超时被整体 `#[ignore]`（= 静默永不执行）。
    /// 现改为显式门控：显式设 `AXAGENT_TEST_PLUGIN_SUBPROCESS=1` 才真跑；
    /// 缺环境时**打印说明后跳过**，不伪造通过、不留红。
    fn require_plugin_subprocess() -> bool {
        if std::env::var("AXAGENT_TEST_PLUGIN_SUBPROCESS").as_deref() == Ok("1") {
            return true;
        }
        eprintln!(
            "SKIP: 未设置 AXAGENT_TEST_PLUGIN_SUBPROCESS=1，跳过启动真实 external plugin 子进程的测试。\
             \n      本测试**未通过，也未被验证** —— 跳过仅表示环境不允许跑子进程。"
        );
        false
    }

    #[test]
    fn env_guard_recovers_after_poisoning() {
        let poisoned = std::thread::spawn(|| {
            let _guard = env_guard();
            panic!("poison env lock");
        })
        .join();
        assert!(poisoned.is_err(), "poisoning thread should panic");

        let _guard = env_guard();
    }

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .unwrap_or_else(|e| panic!("Failed to create parent dir {parent:?}: {e}"));
        }
        fs::write(path, contents).unwrap_or_else(|e| panic!("Failed to write file {path:?}: {e}"));
    }

    fn write_loader_plugin(root: &Path) {
        write_file(root.join("hooks").join("pre.sh").as_path(), "#!/bin/sh\nprintf 'pre'\n");
        write_file(root.join("tools").join("echo-tool.sh").as_path(), "#!/bin/sh\ncat\n");
        write_file(root.join("commands").join("sync.sh").as_path(), "#!/bin/sh\nprintf 'sync'\n");
        write_file(
            root.join(MANIFEST_FILE_NAME).as_path(),
            r#"{
  "name": "loader-demo",
  "version": "1.2.3",
  "description": "Manifest loader test plugin",
  "permissions": ["read", "write"],
  "hooks": {
    "PreToolUse": ["./hooks/pre.sh"]
  },
  "tools": [
    {
      "name": "echo_tool",
      "description": "Echoes JSON input",
      "inputSchema": {
        "type": "object"
      },
      "command": "./tools/echo-tool.sh",
      "requiredPermission": "workspace-write"
    }
  ],
  "commands": [
    {
      "name": "sync",
      "description": "Sync command",
      "command": "./commands/sync.sh"
    }
  ]
}"#,
        );
    }

    fn write_external_plugin(root: &Path, name: &str, version: &str) {
        write_file(root.join("hooks").join("pre.sh").as_path(), "#!/bin/sh\nprintf 'pre'\n");
        write_file(root.join("hooks").join("post.sh").as_path(), "#!/bin/sh\nprintf 'post'\n");
        write_file(
            root.join(MANIFEST_RELATIVE_PATH).as_path(),
            format!(
                "{{\n  \"name\": \"{name}\",\n  \"version\": \"{version}\",\n  \"description\": \"test plugin\",\n  \"permissions\": [\"subprocess_execution\"],\n  \"hooks\": {{\n    \"PreToolUse\": [\"./hooks/pre.sh\"],\n    \"PostToolUse\": [\"./hooks/post.sh\"]\n  }}\n}}"
            )
            .as_str(),
        );
    }

    fn write_broken_plugin(root: &Path, name: &str) {
        write_file(
            root.join(MANIFEST_RELATIVE_PATH).as_path(),
            format!(
                "{{\n  \"name\": \"{name}\",\n  \"version\": \"1.0.0\",\n  \"description\": \"broken plugin\",\n  \"hooks\": {{\n    \"PreToolUse\": [\"./hooks/missing.sh\"]\n  }}\n}}"
            )
            .as_str(),
        );
    }

    fn write_directory_path_plugin(root: &Path, name: &str) {
        fs::create_dir_all(root.join("hooks").join("pre-dir")).expect("hook dir");
        fs::create_dir_all(root.join("tools").join("tool-dir")).expect("tool dir");
        fs::create_dir_all(root.join("commands").join("sync-dir")).expect("command dir");
        fs::create_dir_all(root.join("lifecycle").join("init-dir")).expect("lifecycle dir");
        write_file(
            root.join(MANIFEST_FILE_NAME).as_path(),
            format!(
                "{{\n  \"name\": \"{name}\",\n  \"version\": \"1.0.0\",\n  \"description\": \"directory path plugin\",\n  \"hooks\": {{\n    \"PreToolUse\": [\"./hooks/pre-dir\"]\n  }},\n  \"lifecycle\": {{\n    \"Init\": [\"./lifecycle/init-dir\"]\n  }},\n  \"tools\": [\n    {{\n      \"name\": \"dir_tool\",\n      \"description\": \"Directory tool\",\n      \"inputSchema\": {{\"type\": \"object\"}},\n      \"command\": \"./tools/tool-dir\"\n    }}\n  ],\n  \"commands\": [\n    {{\n      \"name\": \"sync\",\n      \"description\": \"Directory command\",\n      \"command\": \"./commands/sync-dir\"\n    }}\n  ]\n}}"
            )
            .as_str(),
        );
    }

    fn write_broken_failure_hook_plugin(root: &Path, name: &str) {
        write_file(
            root.join(MANIFEST_RELATIVE_PATH).as_path(),
            format!(
                "{{\n  \"name\": \"{name}\",\n  \"version\": \"1.0.0\",\n  \"description\": \"broken plugin\",\n  \"hooks\": {{\n    \"PostToolUseFailure\": [\"./hooks/missing-failure.sh\"]\n  }}\n}}"
            )
            .as_str(),
        );
    }

    fn write_lifecycle_plugin(root: &Path, name: &str, version: &str) -> PathBuf {
        let log_path = root.join("lifecycle.log");

        #[cfg(windows)]
        let (init_name, init_body, shutdown_name, shutdown_body) = (
            "init.cmd",
            "@echo off\r\n@echo init>> lifecycle.log\r\n",
            "shutdown.cmd",
            "@echo off\r\n@echo shutdown>> lifecycle.log\r\n",
        );
        #[cfg(not(windows))]
        let (init_name, init_body, shutdown_name, shutdown_body) = (
            "init.sh",
            "#!/bin/sh\nprintf 'init\\n' >> lifecycle.log\n",
            "shutdown.sh",
            "#!/bin/sh\nprintf 'shutdown\\n' >> lifecycle.log\n",
        );

        let init_path = root.join("lifecycle").join(init_name);
        write_file(init_path.as_path(), init_body);
        let shutdown_path = root.join("lifecycle").join(shutdown_name);
        write_file(shutdown_path.as_path(), shutdown_body);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            for script in [&init_path, &shutdown_path] {
                let mut permissions = fs::metadata(script).expect("metadata").permissions();
                permissions.set_mode(0o755);
                fs::set_permissions(script, permissions).expect("chmod");
            }
        }

        write_file(
            root.join(MANIFEST_RELATIVE_PATH).as_path(),
            format!(
                "{{\n  \"name\": \"{name}\",\n  \"version\": \"{version}\",\n  \"description\": \"lifecycle plugin\",\n  \"lifecycle\": {{\n    \"Init\": [\"./lifecycle/{init_name}\"],\n    \"Shutdown\": [\"./lifecycle/{shutdown_name}\"]\n  }}\n}}"
            )
            .as_str(),
        );
        log_path
    }

    fn write_tool_plugin(root: &Path, name: &str, version: &str) {
        write_tool_plugin_with_name(root, name, version, "plugin_echo");
    }

    fn write_tool_plugin_with_name(root: &Path, name: &str, version: &str, tool_name: &str) {
        #[cfg(windows)]
        let (script_name, script_content) = (
            "echo-json.cmd",
            "@echo off\r\nset /p INPUT=\r\necho {\"plugin\":\"%CLAWD_PLUGIN_ID%\",\"tool\":\"%CLAWD_TOOL_NAME%\",\"input\":%INPUT%}\r\n",
        );
        #[cfg(not(windows))]
        let (script_name, script_content) = (
            "echo-json.sh",
            "#!/bin/sh\nINPUT=$(cat)\nprintf '{\"plugin\":\"%s\",\"tool\":\"%s\",\"input\":%s}\\n' \"$CLAWD_PLUGIN_ID\" \"$CLAWD_TOOL_NAME\" \"$INPUT\"\n",
        );

        let script_path = root.join("tools").join(script_name);
        write_file(&script_path, script_content);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(&script_path).expect("metadata").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&script_path, permissions).expect("chmod");
        }
        write_file(
            root.join(MANIFEST_RELATIVE_PATH).as_path(),
            format!(
                "{{\n  \"name\": \"{name}\",\n  \"version\": \"{version}\",\n  \"description\": \"tool plugin\",\n  \"tools\": [\n    {{\n      \"name\": \"{tool_name}\",\n      \"description\": \"Echo JSON input\",\n      \"inputSchema\": {{\"type\": \"object\", \"properties\": {{\"message\": {{\"type\": \"string\"}}}}, \"required\": [\"message\"], \"additionalProperties\": false}},\n      \"command\": \"./tools/{script_name}\",\n      \"requiredPermission\": \"workspace-write\"\n    }}\n  ]\n}}"
            )
            .as_str(),
        );
    }

    fn write_bundled_plugin(root: &Path, name: &str, version: &str, default_enabled: bool) {
        write_file(
            root.join(MANIFEST_RELATIVE_PATH).as_path(),
            format!(
                "{{\n  \"name\": \"{name}\",\n  \"version\": \"{version}\",\n  \"description\": \"bundled plugin\",\n  \"defaultEnabled\": {}\n}}",
                if default_enabled { "true" } else { "false" }
            )
            .as_str(),
        );
    }

    /// 同版本 bundled 插件的页面导航元数据修正夹具：只改 manifest，不改 version。
    fn write_bundled_plugin_page_with_icon(root: &Path, name: &str, icon: &str) {
        write_file(
            root.join(MANIFEST_RELATIVE_PATH).as_path(),
            format!(
                "{{\n  \"name\": \"{name}\",\n  \"version\": \"1.0.0\",\n  \"description\": \"bundled plugin\",\n  \"defaultEnabled\": true,\n  \"pages\": [{{\n    \"id\": \"{name}-home\",\n    \"title\": \"{name}\",\n    \"entry\": \"index.html\",\n    \"loading\": \"bundle\",\n    \"navGroup\": \"packs\",\n    \"labelKey\": \"nav.packs\",\n    \"icon\": \"{icon}\",\n    \"navOrder\": 1\n  }}]\n}}"
            )
            .as_str(),
        );
    }

    /// 阶段3-①（commands[] 执行分发）夹具：声明一个命名命令的插件。
    /// `with_subprocess=false` 时故意不声明 `subprocess_execution`，用于验证权限拦截。
    fn write_command_plugin(root: &Path, name: &str, version: &str, with_subprocess: bool) {
        #[cfg(windows)]
        let (script_name, script_content) = (
            "greet.cmd",
            "@echo off\r\nset /p INPUT=\r\necho command=%CLAWD_TOOL_NAME% input=%CLAWD_TOOL_INPUT%\r\n",
        );
        #[cfg(not(windows))]
        let (script_name, script_content) = (
            "greet.sh",
            "#!/bin/sh\ncat >/dev/null\nprintf 'command=%s input=%s\\n' \"$CLAWD_TOOL_NAME\" \"$CLAWD_TOOL_INPUT\"\n",
        );

        let script_path = root.join("commands").join(script_name);
        write_file(&script_path, script_content);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(&script_path).expect("metadata").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&script_path, permissions).expect("chmod");
        }
        let permissions = if with_subprocess {
            r#"["read", "subprocess_execution"]"#
        } else {
            r#"["read"]"#
        };
        write_file(
            root.join(MANIFEST_RELATIVE_PATH).as_path(),
            format!(
                "{{\n  \"name\": \"{name}\",\n  \"version\": \"{version}\",\n  \"description\": \"command plugin\",\n  \"defaultEnabled\": true,\n  \"permissions\": {permissions},\n  \"commands\": [\n    {{\n      \"name\": \"greet\",\n      \"description\": \"Greet the input\",\n      \"command\": \"./commands/{script_name}\"\n    }}\n  ]\n}}"
            )
            .as_str(),
        );
    }

    fn load_enabled_plugins(path: &Path) -> BTreeMap<String, bool> {
        let contents = fs::read_to_string(path).expect("settings should exist");
        let root: Value = serde_json::from_str(&contents).expect("settings json");
        root.get("enabledPlugins")
            .and_then(Value::as_object)
            .map(|enabled_plugins| {
                enabled_plugins
                    .iter()
                    .map(|(plugin_id, value)| {
                        (plugin_id.clone(), value.as_bool().expect("plugin state should be a bool"))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn load_plugin_from_directory_validates_required_fields() {
        let _guard = env_guard();
        let root = temp_dir("manifest-required");
        write_file(
            root.join(MANIFEST_FILE_NAME).as_path(),
            r#"{"name":"","version":"1.0.0","description":"desc"}"#,
        );

        let error = load_plugin_from_directory(&root).expect_err("empty name should fail");
        assert!(error.to_string().contains("name cannot be empty"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_plugin_from_directory_reads_root_manifest_and_validates_entries() {
        let _guard = env_guard();
        let root = temp_dir("manifest-root");
        write_loader_plugin(&root);

        let manifest = load_plugin_from_directory(&root).expect("manifest should load");
        assert_eq!(manifest.name, "loader-demo");
        assert_eq!(manifest.version, "1.2.3");
        assert_eq!(
            manifest.permissions.iter().map(|permission| permission.as_str()).collect::<Vec<_>>(),
            vec!["read", "write"]
        );
        assert_eq!(manifest.hooks.pre_tool_use, vec!["./hooks/pre.sh"]);
        assert_eq!(manifest.tools.len(), 1);
        assert_eq!(manifest.tools[0].name, "echo_tool");
        assert_eq!(manifest.tools[0].required_permission, PluginToolPermission::WorkspaceWrite);
        assert_eq!(manifest.commands.len(), 1);
        assert_eq!(manifest.commands[0].name, "sync");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_plugin_from_directory_supports_packaged_manifest_path() {
        let _guard = env_guard();
        let root = temp_dir("manifest-packaged");
        write_external_plugin(&root, "packaged-demo", "1.0.0");

        let manifest = load_plugin_from_directory(&root).expect("packaged manifest should load");
        assert_eq!(manifest.name, "packaged-demo");
        assert!(manifest.tools.is_empty());
        assert!(manifest.commands.is_empty());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_plugin_from_directory_defaults_optional_fields() {
        let _guard = env_guard();
        let root = temp_dir("manifest-defaults");
        write_file(
            root.join(MANIFEST_FILE_NAME).as_path(),
            r#"{
  "name": "minimal",
  "version": "0.1.0",
  "description": "Minimal manifest"
}"#,
        );

        let manifest = load_plugin_from_directory(&root).expect("minimal manifest should load");
        assert!(manifest.permissions.is_empty());
        assert!(manifest.hooks.is_empty());
        assert!(manifest.tools.is_empty());
        assert!(manifest.commands.is_empty());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_plugin_from_directory_rejects_duplicate_permissions_and_commands() {
        let _guard = env_guard();
        let root = temp_dir("manifest-duplicates");
        write_file(root.join("commands").join("sync.sh").as_path(), "#!/bin/sh\nprintf 'sync'\n");
        write_file(
            root.join(MANIFEST_FILE_NAME).as_path(),
            r#"{
  "name": "duplicate-manifest",
  "version": "1.0.0",
  "description": "Duplicate validation",
  "permissions": ["read", "read"],
  "commands": [
    {"name": "sync", "description": "Sync one", "command": "./commands/sync.sh"},
    {"name": "sync", "description": "Sync two", "command": "./commands/sync.sh"}
  ]
}"#,
        );

        let error = load_plugin_from_directory(&root).expect_err("duplicates should fail");
        match error {
            PluginError::ManifestValidation(errors) => {
                assert!(errors.iter().any(|error| matches!(
                    error,
                    PluginManifestValidationError::DuplicatePermission { permission }
                    if permission == "read"
                )));
                assert!(errors.iter().any(|error| matches!(
                    error,
                    PluginManifestValidationError::DuplicateEntry { kind, name }
                    if *kind == "command" && name == "sync"
                )));
            },
            other => panic!("expected manifest validation errors, got {other}"),
        }

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_plugin_from_directory_accepts_mcpservers_skills_agents_still_rejects_unknown_hooks() {
        let root = temp_dir("manifest-cc-compat");
        write_file(
            root.join(MANIFEST_FILE_NAME).as_path(),
            r#"{
  "name": "oh-my-claudecode",
  "version": "4.10.2",
  "description": "Claude Code style manifest",
  "hooks": {
    "SessionStart": ["scripts/session-start.mjs"]
  },
  "agents": [{"agentType": "bot", "description": "test", "tools": [], "disallowedTools": [], "background": false}],
  "commands": ["commands/**/*.md"],
  "skills": [{"name": "skill", "path": "s.md"}],
  "mcpServers": [{"name": "mcp", "command": "echo", "args": [], "env": {}}]
}"#,
        );

        let error = load_plugin_from_directory(&root)
            .expect_err("should reject SessionStart hook and string commands");
        let rendered = error.to_string();
        // OpenClaw compat: skills/mcpServers/agents are now ACCEPTED
        assert!(
            !rendered.contains("field `skills`"),
            "skills should be accepted but got: {rendered}"
        );
        assert!(
            !rendered.contains("field `mcpServers`"),
            "mcpServers should be accepted but got: {rendered}"
        );
        assert!(
            !rendered.contains("field `agents`"),
            "agents should be accepted but got: {rendered}"
        );
        // Still rejected: SessionStart hook and string commands
        assert!(rendered.contains("hook `SessionStart`"));
        assert!(rendered.contains("field `commands`"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_plugin_from_directory_rejects_missing_tool_or_command_paths() {
        let root = temp_dir("manifest-paths");
        write_file(
            root.join(MANIFEST_FILE_NAME).as_path(),
            r#"{
  "name": "missing-paths",
  "version": "1.0.0",
  "description": "Missing path validation",
  "tools": [
    {
      "name": "tool_one",
      "description": "Missing tool script",
      "inputSchema": {"type": "object"},
      "command": "./tools/missing.sh"
    }
  ]
}"#,
        );

        let error = load_plugin_from_directory(&root).expect_err("missing paths should fail");
        assert!(error.to_string().contains("does not exist"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_plugin_from_directory_rejects_missing_lifecycle_paths() {
        // given
        let root = temp_dir("manifest-lifecycle-paths");
        write_file(
            root.join(MANIFEST_FILE_NAME).as_path(),
            r#"{
  "name": "missing-lifecycle-paths",
  "version": "1.0.0",
  "description": "Missing lifecycle path validation",
  "lifecycle": {
    "Init": ["./lifecycle/init.sh"],
    "Shutdown": ["./lifecycle/shutdown.sh"]
  }
}"#,
        );

        // when
        let error =
            load_plugin_from_directory(&root).expect_err("missing lifecycle paths should fail");

        // then
        match error {
            PluginError::ManifestValidation(errors) => {
                assert!(errors.iter().any(|error| matches!(
                    error,
                    PluginManifestValidationError::MissingPath { kind, path }
                    if *kind == "lifecycle command"
                        && path.ends_with(Path::new("lifecycle/init.sh"))
                )));
                assert!(errors.iter().any(|error| matches!(
                    error,
                    PluginManifestValidationError::MissingPath { kind, path }
                    if *kind == "lifecycle command"
                        && path.ends_with(Path::new("lifecycle/shutdown.sh"))
                )));
            },
            other => panic!("expected manifest validation errors, got {other}"),
        }

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_plugin_from_directory_rejects_directory_command_paths() {
        // given
        let root = temp_dir("manifest-directory-paths");
        write_directory_path_plugin(&root, "directory-paths");

        // when
        let error =
            load_plugin_from_directory(&root).expect_err("directory command paths should fail");

        // then
        match error {
            PluginError::ManifestValidation(errors) => {
                assert!(errors.iter().any(|error| matches!(
                    error,
                    PluginManifestValidationError::PathIsDirectory { kind, path }
                    if *kind == "hook" && path.ends_with(Path::new("hooks/pre-dir"))
                )));
                assert!(errors.iter().any(|error| matches!(
                    error,
                    PluginManifestValidationError::PathIsDirectory { kind, path }
                    if *kind == "lifecycle command"
                        && path.ends_with(Path::new("lifecycle/init-dir"))
                )));
                assert!(errors.iter().any(|error| matches!(
                    error,
                    PluginManifestValidationError::PathIsDirectory { kind, path }
                    if *kind == "tool" && path.ends_with(Path::new("tools/tool-dir"))
                )));
                assert!(errors.iter().any(|error| matches!(
                    error,
                    PluginManifestValidationError::PathIsDirectory { kind, path }
                    if *kind == "command" && path.ends_with(Path::new("commands/sync-dir"))
                )));
            },
            other => panic!("expected manifest validation errors, got {other}"),
        }

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_plugin_from_directory_rejects_invalid_permissions() {
        let root = temp_dir("manifest-invalid-permissions");
        write_file(
            root.join(MANIFEST_FILE_NAME).as_path(),
            r#"{
  "name": "invalid-permissions",
  "version": "1.0.0",
  "description": "Invalid permission validation",
  "permissions": ["admin"]
}"#,
        );

        let error = load_plugin_from_directory(&root).expect_err("invalid permissions should fail");
        match error {
            PluginError::ManifestValidation(errors) => {
                assert!(errors.iter().any(|error| matches!(
                    error,
                    PluginManifestValidationError::InvalidPermission { permission }
                    if permission == "admin"
                )));
            },
            other => panic!("expected manifest validation errors, got {other}"),
        }

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_plugin_from_directory_rejects_invalid_tool_required_permission() {
        let root = temp_dir("manifest-invalid-tool-permission");
        write_file(root.join("tools").join("echo.sh").as_path(), "#!/bin/sh\ncat\n");
        write_file(
            root.join(MANIFEST_FILE_NAME).as_path(),
            r#"{
  "name": "invalid-tool-permission",
  "version": "1.0.0",
  "description": "Invalid tool permission validation",
  "tools": [
    {
      "name": "echo_tool",
      "description": "Echo tool",
      "inputSchema": {"type": "object"},
      "command": "./tools/echo.sh",
      "requiredPermission": "admin"
    }
  ]
}"#,
        );

        let error =
            load_plugin_from_directory(&root).expect_err("invalid tool permission should fail");
        match error {
            PluginError::ManifestValidation(errors) => {
                assert!(errors.iter().any(|error| matches!(
                    error,
                    PluginManifestValidationError::InvalidToolRequiredPermission {
                        tool_name,
                        permission
                    } if tool_name == "echo_tool" && permission == "admin"
                )));
            },
            other => panic!("expected manifest validation errors, got {other}"),
        }

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_plugin_from_directory_accumulates_multiple_validation_errors() {
        let root = temp_dir("manifest-multi-error");
        write_file(
            root.join(MANIFEST_FILE_NAME).as_path(),
            r#"{
  "name": "",
  "version": "1.0.0",
  "description": "",
  "permissions": ["admin"],
  "commands": [
    {"name": "", "description": "", "command": "./commands/missing.sh"}
  ]
}"#,
        );

        let error =
            load_plugin_from_directory(&root).expect_err("multiple manifest errors should fail");
        match error {
            PluginError::ManifestValidation(errors) => {
                assert!(errors.len() >= 4);
                assert!(errors.iter().any(|error| matches!(
                    error,
                    PluginManifestValidationError::EmptyField { field } if *field == "name"
                )));
                assert!(errors.iter().any(|error| matches!(
                    error,
                    PluginManifestValidationError::EmptyField { field }
                    if *field == "description"
                )));
                assert!(errors.iter().any(|error| matches!(
                    error,
                    PluginManifestValidationError::InvalidPermission { permission }
                    if permission == "admin"
                )));
            },
            other => panic!("expected manifest validation errors, got {other}"),
        }

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn discovers_builtin_and_bundled_plugins() {
        let _guard = env_guard();
        let config_home = temp_dir("discover");
        // #189 例外：本用例验证**缺省 bundled 根**在远程-only 模型下也不自动安装，
        // 必须保留一次真实默认配置构造（钉到空目录就证明不了默认根的行为）。
        let manager = PluginManager::new(PluginManagerConfig::new(&config_home));
        assert!(
            manager.list_plugins().expect("plugins should list").is_empty(),
            "未安装任何插件时，builtin / bundled / external 目录都不得泄漏进列表"
        );
        assert!(
            manager.load_registry().expect("registry should load").plugins.is_empty(),
            "默认 bundled 根不得自动写入 installed registry"
        );
        let _ = fs::remove_dir_all(config_home);
    }

    #[test]
    fn installs_enables_updates_and_uninstalls_external_plugins() {
        let _guard = env_guard();
        let config_home = temp_dir("home");
        let source_root = temp_dir("source");
        write_external_plugin(&source_root, "demo", "1.0.0");

        let mut manager = PluginManager::new(isolated_config(&config_home));
        let install = manager
            .install_test_fixture(source_root.to_str().expect("utf8 path"))
            .expect("install should succeed");
        assert_eq!(install.plugin_id, "demo@external");
        // 安装后默认 disabled：registry 里有记录，但没有任何能力投影。
        let summary = manager
            .list_plugins()
            .expect("list plugins")
            .into_iter()
            .find(|plugin| plugin.metadata.id == "demo@external")
            .expect("installed plugin should be listed");
        assert!(!summary.enabled);
        assert!(manager.aggregated_hooks().expect("hooks should aggregate").is_empty());
        let anchored = manager
            .load_registry()
            .expect("registry should load")
            .plugins
            .get("demo@external")
            .and_then(|record| record.integrity.clone())
            .expect("install should pin integrity anchor");
        crate::manager::verify_declared_integrity(&install.install_path, &anchored)
            .expect("installed directory should match pinned hash");

        // 显式启用（含写盘 + 同步内存）后能力才出现。
        manager.enable("demo@external").expect("显式启用");
        let hooks = manager.aggregated_hooks().expect("hooks should aggregate");
        assert_eq!(hooks.pre_tool_use.len(), 1);
        assert!(hooks.pre_tool_use[0].contains("pre.sh"));

        manager.disable("demo@external").expect("disable should work");
        assert!(manager.aggregated_hooks().expect("hooks after disable").is_empty());
        manager.enable("demo@external").expect("enable should work");
        assert_eq!(manager.aggregated_hooks().expect("hooks after enable").pre_tool_use.len(), 1);

        manager.uninstall("demo@external").expect("uninstall should work");
        assert!(
            !manager
                .load_registry()
                .expect("registry after uninstall")
                .plugins
                .contains_key("demo@external"),
            "卸载必须从 registry 移除记录"
        );
        assert!(!install.install_path.exists(), "卸载必须删除安装目录");
        assert!(
            !manager
                .list_plugins()
                .expect("list plugins")
                .iter()
                .any(|plugin| plugin.metadata.id == "demo@external")
        );

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(source_root);
    }

    #[test]
    fn auto_installs_bundled_plugins_into_the_registry() {
        let _guard = env_guard();
        let config_home = temp_dir("bundled-home");
        let bundled_root = temp_dir("bundled-root");
        write_bundled_plugin(&bundled_root.join("starter"), "starter", "0.1.0", false);

        let mut config = isolated_config(&config_home);
        config.bundled_root = Some(bundled_root.clone());
        let manager = PluginManager::new(config);

        assert!(
            manager.list_installed_plugins().expect("installed list").is_empty(),
            "bundled 根里有插件也不等于已安装"
        );
        let registry = manager.load_registry().expect("registry should exist");
        assert!(!registry.plugins.contains_key("starter@bundled"));
        assert!(
            !manager.install_root().join("starter-bundled").exists(),
            "bundled 根不得被自动复制到安装根"
        );

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(bundled_root);
    }

    #[test]
    fn default_bundled_root_loads_repo_bundles_as_installed_plugins() {
        let _guard = env_guard();
        let config_home = temp_dir("default-bundled-home");
        // #189 例外：本用例专门验证**缺省** bundled 根（仓内 `bundled/`）也不再自动安装，
        // 钉到空目录就证明不了默认根的非安装行为。它不计入隔离规则。
        let manager = PluginManager::new(PluginManagerConfig::new(&config_home));

        assert!(
            manager.list_installed_plugins().expect("installed list").is_empty(),
            "远程-only 模型下，仓内 bundled 插件不得自动进入 installed 列表"
        );
        assert!(
            manager.load_registry().expect("registry should load").plugins.is_empty(),
            "仓内 bundled 插件不得自动写入 installed registry"
        );

        let _ = fs::remove_dir_all(config_home);
    }

    #[test]
    fn bundled_sync_prunes_removed_bundled_registry_entries() {
        let _guard = env_guard();
        let config_home = temp_dir("bundled-prune-home");
        let bundled_root = temp_dir("bundled-prune-root");
        let stale_install_path =
            config_home.join("plugins").join("installed").join("stale-bundled-external");
        write_bundled_plugin(&bundled_root.join("active"), "active", "0.1.0", false);
        write_file(
            stale_install_path.join(MANIFEST_RELATIVE_PATH).as_path(),
            r#"{
  "name": "stale",
  "version": "0.1.0",
  "description": "stale bundled plugin"
}"#,
        );

        let mut config = isolated_config(&config_home);
        config.bundled_root = Some(bundled_root.clone());
        config.install_root = Some(config_home.join("plugins").join("installed"));
        let manager = PluginManager::new(config);

        let mut registry = InstalledPluginRegistry::default();
        registry.plugins.insert(
            "stale@bundled".to_string(),
            InstalledPluginRecord {
                kind: PluginKind::Bundled,
                id: "stale@bundled".to_string(),
                name: "stale".to_string(),
                version: "0.1.0".to_string(),
                description: "stale bundled plugin".to_string(),
                install_path: stale_install_path.clone(),
                source: PluginInstallSource::LocalPath { path: bundled_root.join("stale") },
                installed_at_unix_ms: 1,
                updated_at_unix_ms: 1,
                integrity: None,
            },
        );
        manager.store_registry(&registry).expect("store registry");
        manager
            .write_enabled_state("stale@bundled", Some(true))
            .expect("seed bundled enabled state");

        let installed = manager.list_installed_plugins().expect("installed list");
        assert!(
            installed.iter().any(|plugin| plugin.metadata.id == "stale@bundled"),
            "registry 中有记录就仍投影；远程-only 模型不做 bundled 同步/裁剪"
        );
        assert!(
            installed.iter().all(|plugin| plugin.metadata.id != "active@bundled"),
            "bundled 根里的插件未经安装不得进入 installed 列表"
        );

        let registry = manager.load_registry().expect("load registry");
        assert!(registry.plugins.contains_key("stale@bundled"));
        assert!(
            stale_install_path.exists(),
            "registry-only 模型不会扫描/清理 bundled 根外的安装目录"
        );

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(bundled_root);
    }

    /// 回归：同版本 bundled 插件只改 `pages[].icon` 也必须刷新已安装目录。
    /// 旧逻辑只比版本/名称/描述，用户升级应用后仍读到旧 manifest ⇒ 侧栏全是 Puzzle。
    #[test]
    fn bundled_sync_refreshes_same_version_manifest_page_metadata() {
        let _guard = env_guard();
        let config_home = temp_dir("bundled-metadata-home");
        let bundled_root = temp_dir("bundled-metadata-root");
        let source = bundled_root.join("navfix");
        write_bundled_plugin(&source, "navfix", "1.0.0", true);

        let mut config = isolated_config(&config_home);
        config.bundled_root = Some(bundled_root.clone());
        let manager = PluginManager::new(config.clone());
        let pages_before = manager.list_plugin_pages().expect("首次页面投影");
        assert!(pages_before.is_empty(), "未安装的 bundled 插件不应产生页面投影");

        // 同版本只改页面导航元数据：远程-only 模型下，bundled 目录变化不触发任何同步。
        write_bundled_plugin_page_with_icon(&source, "navfix", "Calculator");

        let reloaded = PluginManager::new(config);
        let pages_after = reloaded.list_plugin_pages().expect("刷新后的页面投影");
        assert!(pages_after.is_empty(), "bundled 根 manifest 变化不得使未安装插件进入页面投影");
        assert!(
            reloaded.load_registry().expect("registry should load").plugins.is_empty(),
            "bundled 根 manifest 变化不得写入 installed registry"
        );

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(bundled_root);
    }

    /// 回归：一个 bundled 插件同步失败（安装位置被占用）不能让其余插件和 registry 一起丢失。
    /// 真机上对应 Windows 的 `os error 5`：旧 worker 锁住 `stock-bundled/bin/*.exe`，
    /// 同步在 stock 处 `?` 退出，后面的插件不发、registry 不写、插件列表直接空。
    #[test]
    fn bundled_sync_failure_does_not_abort_other_plugins() {
        let _guard = env_guard();
        let config_home = temp_dir("bundled-partial-failure-home");
        let bundled_root = temp_dir("bundled-partial-failure-root");
        write_bundled_plugin(&bundled_root.join("aaa-blocked"), "aaa-blocked", "1.0.0", true);
        write_bundled_plugin(&bundled_root.join("bbb-good"), "bbb-good", "1.0.0", true);

        let install_root = config_home.join("plugins").join("installed");
        fs::create_dir_all(&install_root).expect("建安装根目录");
        fs::write(install_root.join("aaa-blocked-bundled"), b"occupied").expect("占位成普通文件");
        let mut config = isolated_config(&config_home);
        config.bundled_root = Some(bundled_root.clone());

        let manager = PluginManager::new(config);
        assert!(manager.list_installed_plugins().expect("列表不应失败").is_empty());
        let registry = manager.load_registry().expect("registry 应可读");
        assert!(!registry.plugins.contains_key("aaa-blocked@bundled"));
        assert!(!registry.plugins.contains_key("bbb-good@bundled"));
        assert!(
            fs::metadata(install_root.join("aaa-blocked-bundled"))
                .expect("占位文件应保留")
                .is_file(),
            "远程-only 模型不会修改 bundled 路径下的其它目录"
        );

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(bundled_root);
    }

    #[test]
    fn installed_plugin_discovery_keeps_registry_entries_outside_install_root() {
        let _guard = env_guard();
        let config_home = temp_dir("registry-fallback-home");
        let bundled_root = temp_dir("registry-fallback-bundled");
        let install_root = config_home.join("plugins").join("installed");
        let external_install_path = temp_dir("registry-fallback-external");
        write_file(
            external_install_path.join(MANIFEST_FILE_NAME).as_path(),
            r#"{
  "name": "registry-fallback",
  "version": "1.0.0",
  "description": "Registry fallback plugin"
}"#,
        );

        let mut config = isolated_config(&config_home);
        config.bundled_root = Some(bundled_root.clone());
        config.install_root = Some(install_root.clone());
        let manager = PluginManager::new(config);

        let mut registry = InstalledPluginRegistry::default();
        registry.plugins.insert(
            "registry-fallback@external".to_string(),
            InstalledPluginRecord {
                kind: PluginKind::External,
                id: "registry-fallback@external".to_string(),
                name: "registry-fallback".to_string(),
                version: "1.0.0".to_string(),
                description: "Registry fallback plugin".to_string(),
                install_path: external_install_path.clone(),
                source: PluginInstallSource::LocalPath { path: external_install_path.clone() },
                installed_at_unix_ms: 1,
                updated_at_unix_ms: 1,
                integrity: None,
            },
        );
        manager.store_registry(&registry).expect("store registry");
        manager
            .write_enabled_state("stale-external@external", Some(true))
            .expect("seed stale external enabled state");

        let installed =
            manager.list_installed_plugins().expect("registry fallback plugin should load");
        assert!(installed.iter().any(|plugin| plugin.metadata.id == "registry-fallback@external"));

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(bundled_root);
        let _ = fs::remove_dir_all(external_install_path);
    }

    #[test]
    fn installed_plugin_discovery_prunes_stale_registry_entries() {
        let _guard = env_guard();
        let config_home = temp_dir("registry-prune-home");
        let bundled_root = temp_dir("registry-prune-bundled");
        let install_root = config_home.join("plugins").join("installed");
        let missing_install_path = temp_dir("registry-prune-missing");

        let mut config = isolated_config(&config_home);
        config.bundled_root = Some(bundled_root.clone());
        config.install_root = Some(install_root);
        let manager = PluginManager::new(config);

        let mut registry = InstalledPluginRegistry::default();
        registry.plugins.insert(
            "stale-external@external".to_string(),
            InstalledPluginRecord {
                kind: PluginKind::External,
                id: "stale-external@external".to_string(),
                name: "stale-external".to_string(),
                version: "1.0.0".to_string(),
                description: "stale external plugin".to_string(),
                install_path: missing_install_path.clone(),
                source: PluginInstallSource::LocalPath { path: missing_install_path.clone() },
                installed_at_unix_ms: 1,
                updated_at_unix_ms: 1,
                integrity: None,
            },
        );
        manager.store_registry(&registry).expect("store registry");

        let installed =
            manager.list_installed_plugins().expect("stale registry entries should be pruned");
        assert!(!installed.iter().any(|plugin| plugin.metadata.id == "stale-external@external"));

        let registry = manager.load_registry().expect("load registry");
        assert!(!registry.plugins.contains_key("stale-external@external"));

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(bundled_root);
    }

    #[test]
    fn persists_bundled_plugin_enable_state_across_reloads() {
        let _guard = env_guard();
        let config_home = temp_dir("bundled-state-home");
        let source_root = temp_dir("bundled-state-source");
        write_external_plugin(&source_root, "starter", "0.1.0");

        let mut manager = PluginManager::new(isolated_config(&config_home));
        manager
            .install_test_fixture(source_root.to_str().expect("utf8 path"))
            .expect("fixture install should succeed");
        manager.enable("starter@external").expect("显式启用");
        assert_eq!(
            load_enabled_plugins(&manager.settings_path()).get("starter@external"),
            Some(&true)
        );

        let mut reloaded_config = isolated_config(&config_home);
        reloaded_config.enabled_plugins = PluginManagerConfig::load_enabled_plugins(&config_home);
        let reloaded_manager = PluginManager::new(reloaded_config);
        let reloaded = reloaded_manager.list_plugins().expect("installed plugin should be listed");
        assert!(
            reloaded
                .iter()
                .any(|plugin| { plugin.metadata.id == "starter@external" && plugin.enabled }),
            "启用态应跨重载保留"
        );

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(source_root);
    }

    /// 启动期只拉起 installed registry 中显式 enabled 的插件：
    /// `defaultEnabled` 不再兜底，bundled 根里的未安装插件也绝不启动。
    #[test]
    fn start_enabled_plugins_starts_default_enabled_bundled_without_settings_entry() {
        let _guard = env_guard();
        let config_home = temp_dir("default-start-home");
        let bundled_root = temp_dir("default-start-root");
        let source_root = temp_dir("default-start-source");
        write_bundled_plugin(&bundled_root.join("starter"), "starter", "0.1.0", true);
        write_external_plugin(&source_root, "starter", "1.0.0");

        let mut config = isolated_config(&config_home);
        config.bundled_root = Some(bundled_root.clone());
        let mut manager = PluginManager::new(config);

        assert!(manager.start_enabled_plugins().expect("startup should succeed").is_empty());

        let install = manager
            .install_test_fixture(source_root.to_str().expect("utf8 path"))
            .expect("fixture install should succeed");
        assert!(manager.start_enabled_plugins().expect("startup after install").is_empty());

        manager.enable(&install.plugin_id).expect("显式启用");
        assert_eq!(
            manager.start_enabled_plugins().expect("startup after explicit enable"),
            vec![install.plugin_id.clone()]
        );

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(bundled_root);
        let _ = fs::remove_dir_all(source_root);
    }

    #[test]
    fn persists_bundled_plugin_disable_state_across_reloads() {
        let _guard = env_guard();
        let config_home = temp_dir("bundled-disabled-home");
        let source_root = temp_dir("bundled-disabled-source");
        write_external_plugin(&source_root, "starter", "1.0.0");

        let mut manager = PluginManager::new(isolated_config(&config_home));
        let install = manager
            .install_test_fixture(source_root.to_str().expect("utf8 path"))
            .expect("fixture install should succeed");
        assert!(!install.plugin_id.is_empty());

        manager.enable(&install.plugin_id).expect("显式启用");
        manager.disable(&install.plugin_id).expect("disable installed plugin should succeed");
        assert_eq!(
            load_enabled_plugins(&manager.settings_path()).get(&install.plugin_id),
            Some(&false)
        );

        let mut reloaded_config = isolated_config(&config_home);
        reloaded_config.enabled_plugins = PluginManagerConfig::load_enabled_plugins(&config_home);
        let reloaded_manager = PluginManager::new(reloaded_config);
        let reloaded = reloaded_manager.list_plugins().expect("installed plugin should be listed");
        assert!(
            reloaded
                .iter()
                .any(|plugin| { plugin.metadata.id == install.plugin_id && !plugin.enabled }),
            "禁用态应跨重载保留"
        );

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(source_root);
    }

    #[test]
    fn local_fixture_source_is_rejected_by_release_only_gate() {
        let _guard = env_guard();
        let config_home = temp_dir("validate-home");
        let source_root = temp_dir("validate-source");
        write_external_plugin(&source_root, "validator", "1.0.0");
        let manager = PluginManager::new(isolated_config(&config_home));
        let error = manager
            .validate_release_source(source_root.to_str().expect("utf8 path"))
            .expect_err("本地目录不能作为发布源");
        assert!(matches!(error, PluginError::InvalidManifest(_)));
        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(source_root);
    }

    #[test]
    fn plugin_registry_tracks_enabled_state_and_lookup() {
        let _guard = env_guard();
        let config_home = temp_dir("registry-home");
        let source_root = temp_dir("registry-source");
        write_external_plugin(&source_root, "registry-demo", "1.0.0");

        let mut manager = PluginManager::new(isolated_config(&config_home));
        manager
            .install_test_fixture(source_root.to_str().expect("utf8 path"))
            .expect("install should succeed");
        manager.disable("registry-demo@external").expect("disable should succeed");

        let registry = manager.plugin_registry().expect("registry should build");
        let plugin = registry
            .get("registry-demo@external")
            .expect("installed plugin should be discoverable");
        assert_eq!(plugin.metadata().name, "registry-demo");
        assert!(!plugin.is_enabled());
        assert!(registry.contains("registry-demo@external"));
        assert!(!registry.contains("missing@external"));

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(source_root);
    }

    #[test]
    fn plugin_registry_report_collects_load_failures_without_dropping_valid_plugins() {
        let _guard = env_guard();
        // given
        let config_home = temp_dir("report-home");
        let valid_root = temp_dir("report-valid");
        let broken_root = temp_dir("report-broken");
        write_external_plugin(&valid_root, "valid-report", "1.0.0");
        write_broken_plugin(&broken_root, "broken-report");

        let mut manager = PluginManager::new(isolated_config(&config_home));
        let install = manager
            .install_test_fixture(valid_root.to_str().expect("utf8 path"))
            .expect("fixture install should succeed");
        let mut registry = manager.load_registry().expect("registry should load");
        let record = registry.plugins.get(&install.plugin_id).expect("valid record").clone();
        registry.plugins.insert(
            "broken-report@external".to_string(),
            InstalledPluginRecord {
                kind: PluginKind::External,
                id: "broken-report@external".to_string(),
                name: "broken-report".to_string(),
                version: "1.0.0".to_string(),
                description: "broken report plugin".to_string(),
                install_path: broken_root.clone(),
                source: PluginInstallSource::LocalPath { path: broken_root.clone() },
                installed_at_unix_ms: 1,
                updated_at_unix_ms: 1,
                integrity: None,
            },
        );
        manager.store_registry(&registry).expect("store registry");

        // 外部目录中的同结构插件没有 registry 记录，必须保持不可见。
        let orphan_root = temp_dir("report-orphan");
        write_external_plugin(&orphan_root, "orphan-report", "1.0.0");

        let report = manager.plugin_registry_report().expect("report should tolerate a bad record");
        assert!(report.registry().contains(&install.plugin_id));
        assert!(!report.registry().contains("orphan-report@external"));
        assert_eq!(report.failures().len(), 1);
        assert_eq!(report.failures()[0].kind, PluginKind::External);
        assert_eq!(report.failures()[0].plugin_root, broken_root);
        assert!(report.failures()[0].error().to_string().contains("does not exist"));

        let summaries =
            manager.list_plugin_marketplace_summaries().expect("UI 列表应容忍单个加载失败的插件");
        assert!(summaries.iter().any(|p| p.summary.metadata.id == install.plugin_id));
        assert!(summaries.iter().all(|p| p.summary.metadata.id != "orphan-report@external"));

        let error =
            manager.plugin_registry().expect_err("strict registry should surface load failures");
        match error {
            PluginError::LoadFailures(failures) => {
                assert_eq!(failures.len(), 1);
                assert_eq!(failures[0].plugin_root, broken_root);
            },
            other => panic!("expected load failures, got {other}"),
        }
        assert_eq!(record.id, install.plugin_id);

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(valid_root);
        let _ = fs::remove_dir_all(broken_root);
        let _ = fs::remove_dir_all(orphan_root);
    }

    #[test]
    fn installed_plugin_registry_report_collects_load_failures_from_install_root() {
        let _guard = env_guard();
        // given
        let config_home = temp_dir("installed-report-home");
        let valid_root = temp_dir("installed-report-valid");
        let broken_root = temp_dir("installed-report-broken");
        let install_root = config_home.join("plugins").join("installed");
        fs::create_dir_all(&install_root).expect("install root");
        write_external_plugin(&install_root.join("unregistered"), "unregistered", "1.0.0");
        write_external_plugin(&valid_root, "installed-valid", "1.0.0");
        write_broken_plugin(&broken_root, "installed-broken");

        let mut manager = PluginManager::new(isolated_config(&config_home));
        let install = manager
            .install_test_fixture(valid_root.to_str().expect("utf8 path"))
            .expect("fixture install should succeed");
        let mut registry = manager.load_registry().expect("registry should load");
        registry.plugins.insert(
            "installed-broken@external".to_string(),
            InstalledPluginRecord {
                kind: PluginKind::External,
                id: "installed-broken@external".to_string(),
                name: "installed-broken".to_string(),
                version: "1.0.0".to_string(),
                description: "broken installed plugin".to_string(),
                install_path: broken_root.clone(),
                source: PluginInstallSource::LocalPath { path: broken_root.clone() },
                installed_at_unix_ms: 1,
                updated_at_unix_ms: 1,
                integrity: None,
            },
        );
        manager.store_registry(&registry).expect("store registry");

        let report = manager
            .installed_plugin_registry_report()
            .expect("installed report should tolerate invalid installed plugins");
        assert!(report.registry().contains(&install.plugin_id));
        assert!(!report.registry().contains("unregistered@external"));
        assert_eq!(report.failures().len(), 1);
        assert_eq!(report.failures()[0].plugin_root, broken_root);

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(valid_root);
        let _ = fs::remove_dir_all(broken_root);
    }

    #[test]
    fn rejects_plugin_sources_with_missing_hook_paths() {
        let _guard = env_guard();
        // given
        let config_home = temp_dir("broken-home");
        let source_root = temp_dir("broken-source");
        write_broken_plugin(&source_root, "broken");

        let manager = PluginManager::new(isolated_config(&config_home));

        // 本地目录首先被 release-only 来源门禁拒绝，不会去读 manifest。
        let error = manager
            .validate_release_source(source_root.to_str().expect("utf8 path"))
            .expect_err("本地目录不能作为发布源");
        assert!(matches!(error, PluginError::InvalidManifest(_)));

        // 测试夹具入口仍要拒绝缺失的 hook 文件（缺路径校验不可被来源门禁掩盖）。
        let mut manager = PluginManager::new(isolated_config(&config_home));
        let install_error = manager
            .install_test_fixture(source_root.to_str().expect("utf8 path"))
            .expect_err("install should reject invalid hook paths");
        assert!(install_error.to_string().contains("does not exist"));

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(source_root);
    }

    #[test]
    fn rejects_plugin_sources_with_missing_failure_hook_paths() {
        let _guard = env_guard();
        // given
        let config_home = temp_dir("broken-failure-home");
        let source_root = temp_dir("broken-failure-source");
        write_broken_failure_hook_plugin(&source_root, "broken-failure");

        let manager = PluginManager::new(isolated_config(&config_home));

        // 本地目录首先被 release-only 来源门禁拒绝，不会去读 manifest。
        let error = manager
            .validate_release_source(source_root.to_str().expect("utf8 path"))
            .expect_err("本地目录不能作为发布源");
        assert!(matches!(error, PluginError::InvalidManifest(_)));

        // 测试夹具入口仍要拒绝缺失的 failure hook 文件。
        let mut manager = PluginManager::new(isolated_config(&config_home));
        let install_error = manager
            .install_test_fixture(source_root.to_str().expect("utf8 path"))
            .expect_err("install should reject invalid failure hook paths");
        assert!(install_error.to_string().contains("does not exist"));

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(source_root);
    }

    #[test]
    fn plugin_registry_runs_initialize_and_shutdown_for_enabled_plugins() {
        let _guard = env_guard();
        let config_home = temp_dir("lifecycle-home");
        let source_root = temp_dir("lifecycle-source");
        let _ = write_lifecycle_plugin(&source_root, "lifecycle-demo", "1.0.0");

        let mut manager = PluginManager::new(isolated_config(&config_home));
        let install = manager
            .install_test_fixture(source_root.to_str().expect("utf8 path"))
            .expect("install should succeed");
        let log_path = install.install_path.join("lifecycle.log");

        let registry = manager.plugin_registry().expect("registry should build");
        let registered = registry.get(&install.plugin_id).expect("installed plugin is registered");
        assert!(!registered.is_enabled(), "安装后默认 disabled");
        registry.initialize().expect("init on disabled registry should succeed");
        registry.shutdown().expect("shutdown on disabled registry should succeed");
        assert!(!log_path.exists(), "未启用的插件不得执行 lifecycle hook");

        manager.enable(&install.plugin_id).expect("显式启用");
        let registry = manager.plugin_registry().expect("enabled registry should build");
        registry.initialize().expect("init should succeed");
        registry.shutdown().expect("shutdown should succeed");

        let log = fs::read_to_string(&log_path).expect("lifecycle log should exist");
        // Windows cmd echo 输出含 \r\n，normalize 后精确匹配
        let normalized = log.replace("\r\n", "\n").replace(" \n", "\n");
        assert_eq!(normalized, "init\nshutdown\n");

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(source_root);
    }

    #[test]
    fn aggregates_and_executes_plugin_tools() {
        if !require_plugin_subprocess() {
            return;
        }
        let _guard = env_guard();
        let config_home = temp_dir("tool-home");
        let source_root = temp_dir("tool-source");
        write_tool_plugin(&source_root, "tool-demo", "1.0.0");

        let mut manager = PluginManager::new(isolated_config(&config_home));
        manager
            .install_test_fixture(source_root.to_str().expect("utf8 path"))
            .expect("install should succeed");

        let tools = manager.aggregated_tools().expect("tools should aggregate");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].definition().name, "plugin_echo");
        assert_eq!(tools[0].required_permission(), "workspace-write");

        let output = tools[0]
            .execute(&serde_json::json!({ "message": "hello" }))
            .expect("plugin tool should execute");
        let payload: Value = serde_json::from_str(&output).expect("valid json");
        assert_eq!(payload["plugin"], "tool-demo@external");
        assert_eq!(payload["tool"], "plugin_echo");
        assert_eq!(payload["input"]["message"], "hello");

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(source_root);
    }

    /// 阶段3-①：manifest.commands[] 必须流经 summary 可见面并在分发入口被把关 ——
    /// 未声明 `subprocess_execution` 的插件在 **spawn 之前**即 PermissionDenied，
    /// 未知命令名 NotFound（此用例不起子进程，无门控）。
    #[test]
    fn command_dispatch_gate_blocks_without_subprocess_permission() {
        let _guard = env_guard();
        let config_home = temp_dir("cmdgate-home");
        let source_root = temp_dir("cmdgate-source");
        write_command_plugin(&source_root, "cmd-gate", "1.0.0", false);

        let mut manager = PluginManager::new(isolated_config(&config_home));
        let install = manager
            .install_test_fixture(source_root.to_str().expect("utf8 path"))
            .expect("install should succeed");

        manager.enable(&install.plugin_id).expect("显式启用");

        // 可见面：plugin_list 的 summary 携带 commands 声明。
        let summaries = manager.list_plugins().expect("list should succeed");
        let summary = summaries
            .iter()
            .find(|s| s.metadata.id == install.plugin_id)
            .expect("installed plugin should be listed");
        assert_eq!(summary.commands.len(), 1);
        assert_eq!(summary.commands[0].name, "greet");
        assert_eq!(summary.commands[0].description, "Greet the input");

        // 权限门槛：不声明 subprocess_execution ⇒ spawn 前拒绝。
        let err = manager
            .execute_plugin_command(&install.plugin_id, "greet", &Value::Null)
            .expect_err("缺 subprocess_execution 必须拒绝");
        assert!(
            matches!(err, PluginError::PermissionDenied(_)),
            "期望 PermissionDenied，实际 {err:?}"
        );

        // 未知命令名：NotFound，不猜不兜底。
        let err = manager
            .execute_plugin_command(&install.plugin_id, "nope", &Value::Null)
            .expect_err("未声明的命令必须报错");
        assert!(matches!(err, PluginError::NotFound(_)), "期望 NotFound，实际 {err:?}");

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(source_root);
    }

    /// 阶段3-① 全链：声明齐权限的插件经 `execute_plugin_command` 真实执行命名命令，
    /// 输入经 stdin / `CLAWD_COMMAND_INPUT` 注入，stdout 原样回传。
    #[test]
    fn executes_plugin_command_via_dispatch() {
        if !require_plugin_subprocess() {
            return;
        }
        let _guard = env_guard();
        let config_home = temp_dir("cmdexec-home");
        let source_root = temp_dir("cmdexec-source");
        write_command_plugin(&source_root, "cmd-exec", "1.0.0", true);

        let mut manager = PluginManager::new(isolated_config(&config_home));
        let install = manager
            .install_test_fixture(source_root.to_str().expect("utf8 path"))
            .expect("install should succeed");

        let output = manager
            .execute_plugin_command(
                &install.plugin_id,
                "greet",
                &serde_json::json!({ "who": "world" }),
            )
            .expect("命令执行应成功");
        assert!(output.contains("command=greet"), "命令名应注入执行环境：{output}");
        assert!(output.contains("who"), "输入应到达脚本：{output}");

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(source_root);
    }

    #[test]
    fn list_installed_plugins_scans_install_root_without_registry_entries() {
        let _guard = env_guard();
        let config_home = temp_dir("installed-scan-home");
        let install_root = config_home.join("plugins").join("installed");
        let source_root = temp_dir("installed-scan-source");
        write_file(
            install_root.join("scan-demo").join(MANIFEST_FILE_NAME).as_path(),
            r#"{
  "name": "scan-demo",
  "version": "1.0.0",
  "description": "Unregistered directory in install root"
}"#,
        );
        write_external_plugin(&source_root, "registered-demo", "1.0.0");

        let mut manager = PluginManager::new(isolated_config(&config_home));
        let install = manager
            .install_test_fixture(source_root.to_str().expect("utf8 path"))
            .expect("fixture install should succeed");

        let installed = manager.list_installed_plugins().expect("installed list");
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].metadata.id, install.plugin_id);
        assert!(
            installed.iter().all(|plugin| plugin.metadata.id != "scan-demo@external"),
            "install root 里的裸目录不得凭 manifest 变成已安装插件"
        );

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(source_root);
    }

    #[test]
    fn list_installed_plugins_scans_packaged_manifests_in_install_root() {
        let _guard = env_guard();
        let config_home = temp_dir("installed-packaged-scan-home");
        let install_root = config_home.join("plugins").join("installed");
        let source_root = temp_dir("installed-packaged-source");
        write_file(
            install_root.join("scan-packaged").join(MANIFEST_RELATIVE_PATH).as_path(),
            r#"{
  "name": "scan-packaged",
  "version": "1.0.0",
  "description": "Unregistered packaged manifest in install root"
}"#,
        );
        write_external_plugin(&source_root, "registered-packaged", "1.0.0");

        let mut manager = PluginManager::new(isolated_config(&config_home));
        let install = manager
            .install_test_fixture(source_root.to_str().expect("utf8 path"))
            .expect("fixture install should succeed");

        let installed = manager.list_installed_plugins().expect("installed list");
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].metadata.id, install.plugin_id);
        assert!(
            installed.iter().all(|plugin| plugin.metadata.id != "scan-packaged@external"),
            "install root 里的打包 manifest 目录也不得绕过 registry"
        );

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(source_root);
    }

    /// Regression test for ROADMAP #41: verify that `CLAW_CONFIG_HOME` isolation prevents
    /// host `~/.claw/plugins/` from bleeding into test runs.
    #[test]
    fn claw_config_home_isolation_prevents_host_plugin_leakage() {
        let _guard = env_guard();

        // Create a temp directory to act as our isolated CLAW_CONFIG_HOME
        let config_home = temp_dir("isolated-home");
        let bundled_root = temp_dir("isolated-bundled");

        // Set CLAW_CONFIG_HOME to our temp directory
        // SAFETY: Test code (inside #[test] fn); set_var is unsafe because it's not
        // thread-safe in multi-threaded contexts; the test uses env_lock() to ensure
        // exclusive access; the env var is restored/removed in cleanup.
        unsafe { std::env::set_var("CLAW_CONFIG_HOME", &config_home) };

        // 在隔离的 config home 里放一个只有目录、没有 registry 记录的插件。
        let install_root = config_home.join("plugins").join("installed");
        write_file(
            install_root.join("isolated-test-plugin").join(MANIFEST_RELATIVE_PATH).as_path(),
            r#"{
  "name": "isolated-test-plugin",
  "version": "1.0.0",
  "description": "Unregistered plugin in isolated config home"
}"#,
        );

        let manager = PluginManager::new(isolated_config(&config_home));
        let installed = manager.list_installed_plugins().expect("installed plugins should list");
        assert!(
            installed.is_empty(),
            "host ~/.claw/plugins/ 与 install root 裸目录都不得泄漏进隔离实例"
        );
        assert!(manager.load_registry().expect("registry should load").plugins.is_empty());

        // Cleanup
        // SAFETY: Same as above — test code with env_lock() guard ensuring exclusive access.
        unsafe { std::env::remove_var("CLAW_CONFIG_HOME") };
        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(bundled_root);
    }

    #[test]
    fn plugin_lifecycle_handles_parallel_execution() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
        use std::thread;

        let _guard = env_guard();

        // Shared base directory for all threads
        let base_dir = temp_dir("parallel-base");

        // Track successful installations and any errors
        let success_count = Arc::new(AtomicUsize::new(0));
        let error_count = Arc::new(AtomicUsize::new(0));

        // Spawn multiple threads to install plugins simultaneously
        let mut handles = Vec::new();
        for thread_id in 0..5 {
            let base_dir = base_dir.clone();
            let success_count = Arc::clone(&success_count);
            let error_count = Arc::clone(&error_count);

            let handle = thread::spawn(move || {
                // Create unique directories for this thread
                let config_home = base_dir.join(format!("config-{thread_id}"));
                let source_root = base_dir.join(format!("source-{thread_id}"));

                // Write lifecycle plugin for this thread
                let _log_path =
                    write_lifecycle_plugin(&source_root, &format!("parallel-{thread_id}"), "1.0.0");

                // Create PluginManager and install
                let mut manager = PluginManager::new(isolated_config(&config_home));
                let install_result =
                    manager.install_test_fixture(source_root.to_str().expect("utf8 path"));

                match install_result {
                    Ok(install) => {
                        let log_path = install.install_path.join("lifecycle.log");

                        // Initialize and shutdown the registry to trigger lifecycle hooks
                        manager
                            .write_enabled_state(&install.plugin_id, Some(true))
                            .expect("写启用态");
                        manager.config.enabled_plugins.insert(install.plugin_id.clone(), true);
                        let registry = manager.plugin_registry();
                        match registry {
                            Ok(registry) => {
                                if registry.initialize().is_ok() && registry.shutdown().is_ok() {
                                    // Verify lifecycle.log exists and has expected content
                                    if let Ok(log) = fs::read_to_string(&log_path) {
                                        // Windows cmd echo 输出含尾随空格 + CRLF，normalize 后精确匹配
                                        let normalized =
                                            log.replace("\r\n", "\n").replace(" \n", "\n");
                                        if normalized == "init\nshutdown\n" {
                                            success_count.fetch_add(1, AtomicOrdering::Relaxed);
                                        }
                                    }
                                }
                            },
                            Err(_) => {
                                error_count.fetch_add(1, AtomicOrdering::Relaxed);
                            },
                        }
                    },
                    Err(_) => {
                        error_count.fetch_add(1, AtomicOrdering::Relaxed);
                    },
                }
            });
            handles.push(handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            handle.join().expect("thread should complete");
        }

        // Verify all threads succeeded without collisions
        let successes = success_count.load(AtomicOrdering::Relaxed);
        let errors = error_count.load(AtomicOrdering::Relaxed);

        assert_eq!(successes, 5, "all 5 parallel plugin installations should succeed");
        assert_eq!(errors, 0, "no errors should occur during parallel execution");

        // Cleanup
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn parse_install_source_recognizes_npm_scoped() {
        let result = parse_install_source("@clawd/ths").expect("should parse");
        assert!(matches!(
            result,
            PluginInstallSource::NpmPackage { ref name, ref version }
            if name == "@clawd/ths" && version.is_none()
        ));
    }

    #[test]
    fn parse_install_source_recognizes_npm_with_version() {
        let result = parse_install_source("@clawd/stock@1.2.0").expect("should parse");
        assert!(matches!(
            result,
            PluginInstallSource::NpmPackage { ref name, ref version }
            if name == "@clawd/stock" && version == &Some("1.2.0".to_string())
        ));
    }

    #[test]
    fn parse_install_source_recognizes_git_url() {
        let result =
            parse_install_source("https://github.com/user/repo.git").expect("should parse");
        assert!(matches!(result, PluginInstallSource::GitUrl { .. }));
    }

    #[test]
    fn manifest_parses_mcp_servers() {
        let json = r#"{
            "name": "test-plugin",
            "version": "1.0.0",
            "description": "test",
            "mcpServers": [
                {
                    "name": "test-mcp",
                    "command": "python",
                    "args": ["-m", "test"],
                    "env": {}
                }
            ]
        }"#;
        let raw: RawPluginManifest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(raw.mcp_servers.len(), 1);
        assert_eq!(raw.mcp_servers[0].name, "test-mcp");
    }

    #[test]
    fn manifest_parses_skills() {
        let json = r#"{
            "name": "test-plugin",
            "version": "1.0.0",
            "description": "test",
            "skills": [
                {"name": "analyzer", "path": "skills/analyzer/SKILL.md"}
            ]
        }"#;
        let raw: RawPluginManifest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(raw.skills.len(), 1);
        assert_eq!(raw.skills[0].name, "analyzer");
    }

    #[test]
    fn manifest_parses_agents() {
        let json = r#"{
            "name": "test-plugin",
            "version": "1.0.0",
            "description": "test",
            "agents": [
                {
                    "agentType": "stock-bot",
                    "description": "Stock analysis agent",
                    "tools": ["get_price"],
                    "disallowedTools": [],
                    "background": false
                }
            ]
        }"#;
        let raw: RawPluginManifest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(raw.agents.len(), 1);
        assert_eq!(raw.agents[0].agent_type, "stock-bot");
    }

    #[test]
    fn manifest_accepts_mcp_servers_without_error() {
        let json = serde_json::json!({
            "name": "test-plugin",
            "version": "1.0.0",
            "description": "test",
            "mcpServers": [{"name": "mcp", "command": "echo", "args": [], "env": {}}],
            "skills": [{"name": "skill", "path": "s.md"}],
            "agents": [{"agentType": "bot", "description": "bot", "tools": [], "disallowedTools": [], "background": false}]
        });
        let errors = detect_claude_code_manifest_contract_gaps(&json);
        assert!(errors.is_empty(), "mcpServers/skills/agents should not be rejected");
    }
}
