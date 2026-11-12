use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
fn acs(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_acs"))
        .arg("--root")
        .arg(dir)
        .args(args)
        .output()
        .unwrap()
}
fn ok(dir: &Path, args: &[&str]) -> Vec<u8> {
    let output = acs(dir, args);
    assert!(
        output.status.success(),
        "acs {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
fn ids(dir: &Path, agent: &str, mode: &str) -> Vec<String> {
    let output = ok(dir, &["export"]);
    let view: serde_json::Value = serde_json::from_slice(&output).unwrap();
    view["draft"][agent]["library"][mode]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn manages_both_clients_without_launching_and_requires_repair_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    ok(
        dir.path(),
        &[
            "add",
            "codex",
            "custom",
            "example API",
            "--url",
            "https://api.example.com/v1",
            "--key",
            "example-key",
            "--model",
            "example-model",
        ],
    );
    let id = ids(dir.path(), "codex", "custom_providers")[0].clone();
    assert!(String::from_utf8_lossy(&ok(dir.path(), &["ls"])).contains("example API"));
    // ID 前缀定位到该配置并改名，不需要再写客户端和模式。
    ok(dir.path(), &["mv", &id[..8], "renamed example"]);
    assert!(String::from_utf8_lossy(&ok(dir.path(), &["ls"])).contains("renamed example"));
    // 同一个配置可以按名称再次定位。
    ok(
        dir.path(),
        &["edit", "renamed example", "--model", "example-other-model"],
    );
    let view: serde_json::Value = serde_json::from_slice(&ok(dir.path(), &["export"])).unwrap();
    assert_eq!(
        view["draft"]["codex"]["custom_fields"][&id]["model"],
        "example-other-model"
    );
    ok(
        dir.path(),
        &[
            "edit",
            "renamed example",
            "--proxy",
            "http://127.0.0.1:7890",
        ],
    );
    let view: serde_json::Value = serde_json::from_slice(&ok(dir.path(), &["export"])).unwrap();
    assert_eq!(
        view["draft"]["codex"]["custom_fields"][&id]["proxy_url"],
        "http://127.0.0.1:7890"
    );
    // 自定义 API 的代理不影响官方账号共用的代理。
    assert_eq!(
        String::from_utf8_lossy(&ok(dir.path(), &["proxy"])).trim(),
        ""
    );
    ok(dir.path(), &["edit", "renamed example", "--proxy", ""]);
    ok(dir.path(), &["use", "renamed example"]);
    // 客户端选择只影响 --agent 指定的那一个。
    ok(
        dir.path(),
        &[
            "add",
            "claude",
            "custom",
            "example Claude",
            "--url",
            "https://claude.example.com",
            "--key",
            "example-claude-key",
        ],
    );
    let launch = ok(dir.path(), &["run", "--dry-run"]);
    let plan: serde_json::Value = serde_json::from_slice(&launch).unwrap();
    assert_eq!(plan["args"], serde_json::json!([]));
    let desktop = ok(
        dir.path(),
        &[
            "run",
            "--desktop",
            "--executable",
            "example-client",
            "--dry-run",
        ],
    );
    let plan: serde_json::Value = serde_json::from_slice(&desktop).unwrap();
    assert!(plan["args"][0]
        .as_str()
        .unwrap()
        .starts_with("--host-resolver-rules="));
    let claude = ok(dir.path(), &["--agent", "claude", "run", "--dry-run"]);
    let plan: serde_json::Value = serde_json::from_slice(&claude).unwrap();
    assert_eq!(plan["remove_env"][0], "ANTHROPIC_API_KEY");
    let exported = ok(dir.path(), &["export"]);
    let export = dir.path().join("example-draft.json");
    fs::write(&export, &exported).unwrap();
    fs::write(dir.path().join("codex/.env"), "EXAMPLE=changed").unwrap();
    assert!(!acs(dir.path(), &["apply", export.to_str().unwrap()])
        .status
        .success());
    assert!(!acs(dir.path(), &["repair"]).status.success());
    assert!(!dir.path().join("codex/backups_state").exists());
    assert!(acs(dir.path(), &["repair", "--yes"]).status.success());
}

#[test]
fn ambiguous_and_unknown_targets_report_a_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["example one", "example two"] {
        ok(
            dir.path(),
            &[
                "add",
                "codex",
                "custom",
                name,
                "--url",
                "https://api.example.com/v1",
                "--key",
                "example-key",
                "--model",
                "example-model",
            ],
        );
    }
    let ambiguous = acs(dir.path(), &["use", "example"]);
    assert!(!ambiguous.status.success());
    let message = String::from_utf8_lossy(&ambiguous.stderr);
    assert!(message.contains("匹配到多项配置"), "{message}");
    let missing = acs(dir.path(), &["use", "example missing"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("找不到配置"));
    // 官方账号的凭据由客户端登录维护，不接受直接编辑。
    ok(dir.path(), &["add", "codex", "official", "example account"]);
    let official = acs(dir.path(), &["edit", "example account", "--model", "x"]);
    assert!(!official.status.success());
    assert!(String::from_utf8_lossy(&official.stderr).contains("官方客户端登录维护"));
}

#[test]
fn identical_client_profiles_have_short_unique_ids_and_delete_only_the_requested_account() {
    let dir = tempfile::tempdir().unwrap();
    let mut all = std::collections::HashSet::new();
    for agent in ["codex", "claude"] {
        ok(dir.path(), &["add", agent, "official", "example account"]);
        ok(
            dir.path(),
            &[
                "add",
                agent,
                "custom",
                "example API",
                "--url",
                "https://api.example.com/v1",
                "--key",
                "example-key",
                "--model",
                "example-model",
            ],
        );
        for mode in ["official_accounts", "custom_providers"] {
            for id in ids(dir.path(), agent, mode) {
                assert_eq!(id.len(), 8);
                assert!(all.insert(id));
            }
        }
    }
    let codex_id = ids(dir.path(), "codex", "official_accounts")[0].clone();
    let claude_id = ids(dir.path(), "claude", "official_accounts")[0].clone();
    let ambiguous = acs(dir.path(), &["rm", "example account", "--yes"]);
    assert!(!ambiguous.status.success());
    let error = String::from_utf8_lossy(&ambiguous.stderr);
    assert!(
        error.contains(&codex_id) && error.contains(&claude_id),
        "{error}"
    );
    let codex_config = fs::read(dir.path().join("codex/config.toml")).unwrap();
    let codex_auth = fs::read(dir.path().join("codex/auth.json")).unwrap();
    ok(dir.path(), &["rm", &claude_id, "--yes"]);
    assert!(ids(dir.path(), "claude", "official_accounts").is_empty());
    assert_eq!(ids(dir.path(), "codex", "official_accounts"), [codex_id]);
    assert_eq!(
        fs::read(dir.path().join("codex/config.toml")).unwrap(),
        codex_config
    );
    assert_eq!(
        fs::read(dir.path().join("codex/auth.json")).unwrap(),
        codex_auth
    );
    assert_eq!(ids(dir.path(), "claude", "custom_providers").len(), 1);
}

#[test]
fn proxy_command_reads_and_writes_the_selected_client_only() {
    let dir = tempfile::tempdir().unwrap();
    ok(dir.path(), &["add", "codex", "official", "example account"]);
    assert_eq!(
        String::from_utf8_lossy(&ok(dir.path(), &["proxy"])).trim(),
        ""
    );
    ok(dir.path(), &["proxy", "http://127.0.0.1:7890"]);
    assert_eq!(
        String::from_utf8_lossy(&ok(dir.path(), &["proxy"])).trim(),
        "http://127.0.0.1:7890"
    );
    // 两个客户端各自保留独立代理偏好。
    assert_eq!(
        String::from_utf8_lossy(&ok(dir.path(), &["--agent", "claude", "proxy"])).trim(),
        ""
    );
    // 非法代理地址在保存前被拒绝，不落盘。
    assert!(!acs(dir.path(), &["proxy", "http://example.com/path"])
        .status
        .success());
}

#[test]
fn pipes_keep_listing_missing_arguments_are_usage_errors_and_invalid_login_never_writes() {
    use std::io::Write;
    use std::process::Stdio;
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(ok(dir.path(), &[]), ok(dir.path(), &["ls"]));
    for args in [
        &["add"][..],
        &["add", "codex"],
        &["use"],
        &["edit"],
        &["rm"],
    ] {
        assert_eq!(acs(dir.path(), args).status.code(), Some(2));
    }
    ok(dir.path(), &["add", "codex", "official", "example account"]);
    let store = acs_core::Store::new(acs_core::Roots::under(dir.path()));
    let before = store.snapshot().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_acs"))
        .arg("--root")
        .arg(dir.path())
        .args(["login", "example account"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"http://localhost:1455/auth/callback?state=wrong&code=example-private-code\n")
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(!result.status.success());
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.find("换令牌代理：").unwrap()
            < stdout
                .find("https://auth.openai.com/oauth/authorize?")
                .unwrap()
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("官方代理未设置"));
    assert!(String::from_utf8_lossy(&result.stderr).contains("state"));
    assert!(!String::from_utf8_lossy(&result.stderr).contains("example-private-code"));
    assert_eq!(store.snapshot().unwrap(), before);
    let logs = fs::read_to_string(dir.path().join("acs/logs/acs.log")).unwrap();
    assert!(!logs.contains("example-private-code"));
}

#[test]
fn invalid_edit_preserves_saved_files_and_copy_retains_source_fields() {
    let dir = tempfile::tempdir().unwrap();
    ok(
        dir.path(),
        &[
            "add",
            "codex",
            "custom",
            "example API",
            "--url",
            "https://api.example.com/v1",
            "--key",
            "example-key",
            "--model",
            "example-model",
        ],
    );
    let store = acs_core::Store::new(acs_core::Roots::under(dir.path()));
    let before = store.snapshot().unwrap();
    assert!(
        !acs(dir.path(), &["edit", "example API", "--url", "invalid"])
            .status
            .success()
    );
    assert_eq!(store.snapshot().unwrap(), before);
    ok(
        dir.path(),
        &[
            "add",
            "codex",
            "custom",
            "example copy",
            "--copy",
            "example API",
        ],
    );
    let s = acs_core::Session::open(store.clone()).unwrap();
    assert_eq!(s.original.codex.library.custom_providers.len(), 2);
    assert_eq!(
        s.original
            .codex
            .custom_fields
            .values()
            .map(|f| &f.base_url)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        1
    );
    let before = store.snapshot().unwrap();
    assert!(!acs(
        dir.path(),
        &["edit", "example API", "--url", "bad-unselected-url"]
    )
    .status
    .success());
    assert_eq!(store.snapshot().unwrap(), before);
}
#[test]
fn environment_commands_are_isolated_and_missing_arguments_are_usage_errors() {
    let dir = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let r = repo.path();
    fs::write(
        r.join("environment.yaml"),
        include_str!("../../core/tests/fixtures/environment/environment.yaml"),
    )
    .unwrap();
    fs::write(
        r.join("environment.lock"),
        include_str!("../../core/tests/fixtures/environment/environment.lock"),
    )
    .unwrap();
    fs::create_dir_all(r.join("instructions")).unwrap();
    fs::write(r.join("instructions/global.md"), "example").unwrap();
    fs::create_dir_all(r.join("skills/local/example-demo")).unwrap();
    fs::write(r.join("skills/local/example-demo/SKILL.md"), "example").unwrap();
    fs::create_dir_all(r.join("skills/vendor")).unwrap();
    fs::write(
        r.join("skills/vendor/.ags-snapshot.yaml"),
        "version: 1\nsources: {}\nskills: {}\n",
    )
    .unwrap();
    // 错误的账号文件证明 env 不打开账号 Session。
    fs::create_dir_all(dir.path().join("codex")).unwrap();
    fs::write(dir.path().join("codex/auth.json"), "example invalid json").unwrap();
    ok(dir.path(), &["env", "source", "show"]);
    assert!(!dir.path().join("acs/environment.yaml").exists());
    ok(
        dir.path(),
        &["env", "validate", "--repo", r.to_str().unwrap()],
    );
    ok(
        dir.path(),
        &[
            "env",
            "source",
            "set",
            r.to_str().unwrap(),
            "--branch",
            "main",
        ],
    );
    ok(
        dir.path(),
        &[
            "env",
            "configure",
            "--profile",
            "default",
            "--agents",
            "codex,claude",
        ],
    );
    let saved = fs::read(dir.path().join("acs/environment.yaml")).unwrap();
    for args in [
        vec!["env"],
        vec!["env", "source"],
        vec!["env", "source", "set"],
        vec!["env", "configure"],
        vec!["env", "configure", "--profile"],
        vec!["env", "validate"],
        vec!["env", "configure", "--agents", "example-agent"],
    ] {
        assert_eq!(acs(dir.path(), &args).status.code(), Some(2), "{args:?}");
    }
    assert!(
        !acs(dir.path(), &["env", "configure", "--profile", "unknown"])
            .status
            .success()
    );
    assert_eq!(
        saved,
        fs::read(dir.path().join("acs/environment.yaml")).unwrap()
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("codex/auth.json")).unwrap(),
        "example invalid json"
    );
    assert!(!dir.path().join("acs/profiles.json").exists());
    assert!(!dir.path().join(".agents").exists());
    // 没有 git/node/npm 时的既有 ls 和 env source show 仍正常。
    let clean = tempfile::tempdir().unwrap();
    for args in [vec!["ls"], vec!["env", "source", "show"]] {
        let out = Command::new(env!("CARGO_BIN_EXE_acs"))
            .arg("--root")
            .arg(clean.path())
            .args(args)
            .env("PATH", "")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
#[test]
fn environment_remote_source_fetch_preserves_saved_config_on_failure() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    fs::write(
        d.path().join("environment.yaml"),
        include_str!("../../core/tests/fixtures/environment/environment.yaml"),
    )
    .unwrap();
    fs::write(
        d.path().join("environment.lock"),
        include_str!("../../core/tests/fixtures/environment/environment.lock"),
    )
    .unwrap();
    let git = |args: &[&str]| {
        let o = Command::new("git")
            .current_dir(d.path())
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", local.path().join("example-gitconfig"))
            .env("HOME", local.path())
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    };
    git(&["init", "--initial-branch=main"]);
    git(&["add", "."]);
    git(&[
        "-c",
        "user.name=Example",
        "-c",
        "user.email=example@example.com",
        "commit",
        "-m",
        "example fixture",
    ]);
    let path = d.path().to_string_lossy().replace('\\', "/");
    let url = if cfg!(windows) {
        format!("file:///{path}")
    } else {
        format!("file://{path}")
    };
    let run = |branch: &str| {
        Command::new(env!("CARGO_BIN_EXE_acs"))
            .arg("--root")
            .arg(local.path())
            .args(["env", "source", "set", &url, "--branch", branch])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", local.path().join("example-gitconfig"))
            .env("HOME", local.path())
            .output()
            .unwrap()
    };
    assert!(run("main").status.success());
    let saved = fs::read(local.path().join("acs/environment.yaml")).unwrap();
    let checkout = fs::read_dir(local.path().join("acs/cache/environment"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::remove_dir_all(&checkout).unwrap();
    fs::create_dir(&checkout).unwrap();
    fs::write(checkout.join("example-partial-clone"), "example").unwrap();
    assert!(run("main").status.success());
    assert!(checkout.join(".git").exists());
    assert!(!checkout.join("example-partial-clone").exists());
    assert_eq!(
        fs::read(local.path().join("acs/environment.yaml")).unwrap(),
        saved
    );
    assert!(!run("example-missing").status.success());
    assert_eq!(
        fs::read(local.path().join("acs/environment.yaml")).unwrap(),
        saved
    );
}
#[test]
fn environment_cli_apply_status_and_agent_group_use_isolated_npm() {
    let dir = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let r = repo.path();
    for (file, text) in [
        (
            "environment.yaml",
            include_str!("../../core/tests/fixtures/environment/environment.yaml"),
        ),
        (
            "environment.lock",
            include_str!("../../core/tests/fixtures/environment/environment.lock"),
        ),
        ("instructions/global.md", "example rules\n"),
        (
            "skills/local/example-demo/SKILL.md",
            "---\nname: example-demo\n---\nexample\n",
        ),
        (
            "skills/vendor/.ags-snapshot.yaml",
            "version: 1\nsources: {}\nskills: {}\n",
        ),
    ] {
        let path = r.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let shim = dir.path().join("bin");
    fs::create_dir(&shim).unwrap();
    #[cfg(windows)] fs::write(shim.join("npm.cmd"),"@echo off\r\nif \"%1\"==\"root\" goto root\r\necho Unexpected global mutation 1>&2\r\nexit /b 91\r\n:root\r\necho %ACS_EXAMPLE_NPM_ROOT%\r\nexit /b 0\r\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let p = shim.join("npm");
        fs::write(&p,"#!/bin/sh\nif [ \"$1\" = root ]; then printf '%s\\n' \"$ACS_EXAMPLE_NPM_ROOT\"; else exit 91; fi\n").unwrap();
        fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let npm = dir.path().join("npm");
    for (name, version) in [
        ("@openai/codex", "1.0.0"),
        ("@anthropic-ai/claude-code", "2.0.0"),
    ] {
        let path = npm.join(name).join("package.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!(r#"{{"version":"{version}"}}"#)).unwrap();
    }
    let path = std::env::join_paths(std::iter::once(shim).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_acs"))
            .arg("--root")
            .arg(dir.path())
            .args(args)
            .env("PATH", &path)
            .env("ACS_EXAMPLE_NPM_ROOT", &npm)
            .output()
            .unwrap()
    };
    for args in [
        vec!["env", "source", "set", r.to_str().unwrap()],
        vec![
            "env",
            "configure",
            "--profile",
            "default",
            "--agents",
            "all",
        ],
        vec!["env", "diff"],
        vec!["agent", "status", "all"],
        vec!["agent", "install", "all"],
    ] {
        let out = run(&args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!out.stdout.starts_with(b"{"));
    }
    for args in [
        vec!["env", "apply"],
        vec!["agent", "uninstall", "codex"],
        vec!["--agent", "codex", "env", "source", "show"],
        vec!["--agent", "codex", "agent", "status"],
    ] {
        assert_eq!(run(&args).status.code(), Some(2), "{args:?}");
    }
    let out = run(&["env", "apply", "--yes"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let config = dir.path().join("codex/config.toml");
    let before = fs::read(&config).unwrap();
    let modified = fs::metadata(&config).unwrap().modified().unwrap();
    let status = run(&["env", "status"]);
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout).contains("无漂移"));
    assert!(run(&["env", "apply", "--yes"]).status.success());
    assert_eq!(fs::read(&config).unwrap(), before);
    assert_eq!(fs::metadata(config).unwrap().modified().unwrap(), modified);
}
#[test]
fn cli_version_and_update_usage_do_not_read_accounts_or_access_network() {
    let d = tempfile::tempdir().unwrap();
    let output = acs(d.path(), &["self", "version"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains(env!("CARGO_PKG_VERSION")));
    assert!(!d.path().join("acs").exists());
    assert_eq!(acs(d.path(), &["self", "update"]).status.code(), Some(2));
    assert!(!d.path().join("acs").exists());
    let report = Command::new(env!("CARGO_BIN_EXE_acs"))
        .arg("--root")
        .arg(d.path())
        .arg("doctor")
        .env("PATH", "")
        .output()
        .unwrap();
    assert_eq!(report.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&report.stdout).contains("修复"));
}
