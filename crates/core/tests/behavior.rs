use acs_core::{
    config::{normalize_proxy, proxy_env},
    history, Agent, CustomFields, Mode, Roots, Session, Store, WindowState,
};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{cell::RefCell, fs, path::Path};

fn fixture() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(Roots::under(dir.path()));
    for path in [&store.roots.codex, &store.roots.claude] {
        fs::create_dir_all(path).unwrap();
    }
    (dir, store)
}
fn custom(session: &mut Session) -> String {
    let mut draft = session.original.clone();
    draft.codex.mode = Mode::Custom;
    let id = draft.codex.add(Mode::Custom, "example API", None).unwrap();
    draft.codex.custom_fields.insert(
        id.clone(),
        CustomFields {
            provider_name: "example provider".into(),
            base_url: "https://api.example.com/v1".into(),
            api_key: "example-key".into(),
            model: "example-model".into(),
            effort: "medium".into(),
            ..Default::default()
        },
    );
    session.save(&session.view().revision, &draft).unwrap();
    id
}
fn official(session: &mut Session) -> String {
    let mut draft = session.original.clone();
    draft.codex.mode = Mode::Official;
    let id = draft
        .codex
        .add(Mode::Official, "example account", None)
        .unwrap();
    session.save(&session.view().revision, &draft).unwrap();
    id
}
fn credentials() -> Value {
    json!({"auth_mode":"chatgpt","tokens":{"access_token":"example-access","refresh_token":"example-refresh","account_id":"example-account","id_token":"example-id"}})
}
fn read(store: &Store, path: &str) -> String {
    fs::read_to_string(store.roots.codex.join(path)).unwrap()
}
fn seed_rollout(store: &Store) -> String {
    fs::create_dir_all(store.roots.codex.join("sessions")).unwrap();
    let input = "{\"type\":\"session_meta\",\"payload\":{\"model_provider\":\"old\",\"name\":\"example\"}}\r\n{\"type\":\"event_msg\",\"payload\":\"example\"}\n";
    fs::write(store.roots.codex.join("sessions/example.jsonl"), input).unwrap();
    input.into()
}
fn database(store: &Store, filename: &str, fail: bool) -> Connection {
    let path = store.roots.codex.join(filename);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let db = Connection::open(path).unwrap();
    db.execute_batch("CREATE TABLE threads (model_provider TEXT); INSERT INTO threads VALUES ('old'),(NULL); CREATE TABLE local_thread_catalog (model_provider TEXT); INSERT INTO local_thread_catalog VALUES ('old');").unwrap();
    if fail {
        db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON threads BEGIN SELECT RAISE(ABORT, 'example failure'); END;").unwrap();
    }
    db
}

#[test]
fn saving_claude_preserves_invalid_unchanged_codex() {
    let (_dir, store) = fixture();
    let config = "model_provider = \"custom\"\n[model_providers.custom]\nbase_url = \"https://codex.example.com\"\n";
    let auth = "{\"auth_mode\":\"apikey\",\"OPENAI_API_KEY\":\"example-codex-key\"}";
    fs::write(store.roots.codex.join("config.toml"), config).unwrap();
    fs::write(store.roots.codex.join("auth.json"), auth).unwrap();
    let mut session = Session::open(store.clone()).unwrap();
    let mut draft = session.original.clone();
    draft.claude.mode = Mode::Custom;
    let id = draft
        .claude
        .add(Mode::Custom, "example Claude", None)
        .unwrap();
    draft.claude.custom_fields.insert(
        id,
        CustomFields {
            base_url: "https://claude.example.com".into(),
            api_key: "example-claude-key".into(),
            model: "example-model".into(),
            ..Default::default()
        },
    );
    session
        .save_for(&session.view().revision, &draft, Agent::Claude)
        .unwrap();
    session
        .save_for(
            &session.view().revision,
            &session.original.clone(),
            Agent::Claude,
        )
        .unwrap();
    assert_eq!(read(&store, "config.toml"), config);
    assert_eq!(read(&store, "auth.json"), auth);
    assert!(!store.roots.codex.join(".env").exists());
    let settings: Value = serde_json::from_str(
        &fs::read_to_string(store.roots.claude.join("settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        settings["env"]["ANTHROPIC_BASE_URL"],
        "https://claude.example.com"
    );
    session
        .launch_settings_for(&session.view().revision, &session.original, Agent::Claude)
        .unwrap();
}

#[test]
fn opening_and_drafts_do_not_write() {
    let (_dir, store) = fixture();
    let session = Session::open(store.clone()).unwrap();
    let before = store.snapshot().unwrap();
    let mut draft = session.original.clone();
    let id = draft.codex.add(Mode::Custom, "example", None).unwrap();
    draft.codex.delete(Mode::Custom, &id).unwrap();
    assert_eq!(before, store.snapshot().unwrap());
    assert!(!store.roots.acs.exists());
}
#[test]
fn saves_never_repair_and_launch_never_saves() {
    let (_dir, store) = fixture();
    let input = seed_rollout(&store);
    let mut s = Session::open(store.clone()).unwrap();
    custom(&mut s);
    assert_eq!(read(&store, "sessions/example.jsonl"), input);
    let before = store.snapshot().unwrap();
    let settings = s.launch_settings(&s.view().revision, &s.original).unwrap();
    assert_eq!(settings.mode, Mode::Custom);
    assert_eq!(before, store.snapshot().unwrap());
    let mut dirty = s.original.clone();
    dirty.codex.library.custom_providers[0].name = Some("changed".into());
    assert!(s.launch_settings(&s.view().revision, &dirty).is_err());
    assert_eq!(before, store.snapshot().unwrap());
}
#[test]
fn each_external_file_change_blocks_save_and_launch() {
    let cases = [
        ("codex", "config.toml"),
        ("codex", "auth.json"),
        ("acs", "profiles.json"),
        ("codex", ".env"),
    ];
    for (root, path) in cases {
        let (_dir, store) = fixture();
        let mut s = Session::open(store.clone()).unwrap();
        custom(&mut s);
        let base = if root == "codex" {
            &store.roots.codex
        } else {
            &store.roots.acs
        };
        fs::write(base.join(path), "example external change").unwrap();
        let before = store.snapshot().unwrap();
        assert!(s.save(&s.view().revision, &s.original.clone()).is_err());
        assert!(s.launch_settings(&s.view().revision, &s.original).is_err());
        assert_eq!(before, store.snapshot().unwrap());
    }
}
#[test]
fn launch_does_not_recover_missing_active_key_from_profile() {
    let (_dir, store) = fixture();
    let mut s = Session::open(store.clone()).unwrap();
    custom(&mut s);
    fs::write(
        store.roots.codex.join("auth.json"),
        r#"{"auth_mode":"apikey"}"#,
    )
    .unwrap();
    let s = Session::open(store).unwrap();
    assert!(s.launch_settings(&s.view().revision, &s.original).is_err());
}
#[test]
fn provider_snapshot_roundtrip_preserves_nested_unknown_values() {
    let (_dir, store) = fixture();
    let mut s = Session::open(store.clone()).unwrap();
    let id = custom(&mut s);
    let mut draft = s.original.clone();
    draft.codex.library.custom_providers[0]
        .model_providers_toml
        .as_mut()
        .unwrap()
        .push_str("\n[model_providers.custom.http_headers]\n\"X-Example\" = \"test\"\n");
    s.save(&s.view().revision, &draft).unwrap();
    let original = read(&store, "config.toml");
    official(&mut s);
    let clean = read(&store, "config.toml");
    assert!(clean.contains("[model_providers.custom]"));
    assert!(!clean.contains("base_url"));
    assert!(!clean.contains("X-Example"));
    s.save(&s.view().revision, &s.original.clone()).unwrap();
    let mut draft = s.original.clone();
    draft.codex.mode = Mode::Custom;
    draft.codex.library.selected_custom = id;
    s.save(&s.view().revision, &draft).unwrap();
    let restored = read(&store, "config.toml");
    assert!(restored.contains("X-Example"));
    assert_eq!(
        original
            .parse::<toml_edit::DocumentMut>()
            .unwrap()
            .to_string(),
        restored
            .parse::<toml_edit::DocumentMut>()
            .unwrap()
            .to_string()
    );
}
#[test]
fn mode_switches_preserve_user_providers_without_snapshotting_them() {
    let (_dir, store) = fixture();
    let user_config = "# User-owned provider\n[model_providers.other]\nname = 'example other'\nbase_url = 'https://other.example.com/v1'\nenv_key = 'EXAMPLE_API_KEY'\n[model_providers.other.http_headers]\n'X-Example' = 'user value'\n[model_providers.another]\nname = 'example another'\n[profiles.example]\nmodel_provider = 'other'\nmodel = 'example-model'\n";
    fs::write(store.roots.codex.join("config.toml"), user_config).unwrap();
    let mut s = Session::open(store.clone()).unwrap();
    let id = custom(&mut s);
    let assert_user_owned = |s: &Session| {
        let config = read(&store, "config.toml");
        let actual = config.parse::<toml_edit::DocumentMut>().unwrap();
        let expected = user_config.parse::<toml_edit::DocumentMut>().unwrap();
        assert!(config.contains("# User-owned provider"));
        for provider in ["other", "another"] {
            assert_eq!(
                actual["model_providers"][provider].to_string(),
                expected["model_providers"][provider].to_string()
            );
        }
        assert_eq!(
            actual["profiles"].to_string(),
            expected["profiles"].to_string()
        );
        let library = fs::read_to_string(store.roots.acs.join("profiles.json")).unwrap();
        assert!(!library.contains("model_providers.other"));
        assert!(!library.contains("model_providers.another"));
        assert!(!library.contains("profiles.example"));
        for profile in &s.original.codex.library.custom_providers {
            let snapshot = profile.model_providers_toml.as_ref().unwrap();
            let doc = snapshot.parse::<toml_edit::DocumentMut>().unwrap();
            let providers = doc["model_providers"].as_table().unwrap();
            assert_eq!(providers.len(), 1);
            assert!(providers.contains_key("custom"));
        }
    };
    assert_user_owned(&s);
    official(&mut s);
    assert_user_owned(&s);
    s.save(&s.view().revision, &s.original.clone()).unwrap();
    assert_user_owned(&s);
    // Reopen to verify persisted snapshots and fresh target baselines.
    let mut s = Session::open(store.clone()).unwrap();
    let mut draft = s.original.clone();
    draft.codex.mode = Mode::Custom;
    draft.codex.library.selected_custom = id;
    s.save(&s.view().revision, &draft).unwrap();
    assert_user_owned(&s);
    let before = store.snapshot().unwrap();
    s.save(&s.view().revision, &s.original.clone()).unwrap();
    assert_eq!(before, store.snapshot().unwrap());
    assert_user_owned(&s);
}
#[test]
fn auth_refresh_capture_and_logout_do_not_restore_old_tokens() {
    let (_dir, store) = fixture();
    let mut s = Session::open(store.clone()).unwrap();
    official(&mut s);
    fs::write(
        store.roots.codex.join("auth.json"),
        serde_json::to_vec(&credentials()).unwrap(),
    )
    .unwrap();
    let mut s = Session::open(store.clone()).unwrap();
    let id = s.original.codex.library.selected_official.clone();
    custom(&mut s);
    let mut draft = s.original.clone();
    draft.codex.mode = Mode::Official;
    draft.codex.library.selected_official = id.clone();
    s.save(&s.view().revision, &draft).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&read(&store, "auth.json")).unwrap()["tokens"]
            ["access_token"],
        "example-access"
    );
    fs::remove_file(store.roots.codex.join("auth.json")).unwrap();
    let mut s = Session::open(store.clone()).unwrap();
    assert!(s
        .original
        .codex
        .library
        .selected(Mode::Official)
        .unwrap()
        .official_auth
        .is_none());
    s.save(&s.view().revision, &s.original.clone()).unwrap();
    assert!(!read(&store, "auth.json").contains("example-access"));
}
#[test]
fn partial_tokens_block_without_clearing_library() {
    let (_dir, store) = fixture();
    let mut s = Session::open(store.clone()).unwrap();
    official(&mut s);
    fs::write(
        store.roots.codex.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"access_token":"example"}}"#,
    )
    .unwrap();
    let before = store.snapshot().unwrap();
    assert!(Session::open(store.clone()).is_err());
    assert_eq!(before, store.snapshot().unwrap());
}

#[test]
fn unknown_official_account_is_not_added_to_the_library() {
    let (_dir, store) = fixture();
    fs::write(
        store.roots.codex.join("auth.json"),
        serde_json::to_vec(&credentials()).unwrap(),
    )
    .unwrap();
    let before = store.snapshot().unwrap();
    let first = Session::open(store.clone()).unwrap();
    let second = Session::open(store.clone()).unwrap();
    assert!(first.original.codex.library.official_accounts.is_empty());
    assert_eq!(first.original, second.original);
    assert_eq!(before, store.snapshot().unwrap());
}
#[test]
fn unknown_external_api_does_not_create_a_profile() {
    let (_dir, store) = fixture();
    let mut s = Session::open(store.clone()).unwrap();
    let old_id = custom(&mut s);
    let doc = read(&store, "config.toml").replace("api.example.com", "other.example.com");
    fs::write(store.roots.codex.join("config.toml"), doc).unwrap();
    let s = Session::open(store).unwrap();
    assert_eq!(s.original.codex.library.custom_providers.len(), 1);
    assert_eq!(s.original.codex.library.selected_custom, old_id);
    assert_eq!(
        s.original.codex.custom_fields[&old_id].base_url,
        "https://api.example.com/v1"
    );
}

#[test]
fn short_ids_are_unique_across_clients_and_modes_and_duplicates_never_save() {
    let (_dir, store) = fixture();
    let mut session = Session::open(store.clone()).unwrap();
    let mut draft = session.original.clone();
    let mut ids = std::collections::HashSet::new();
    for agent in Agent::ALL {
        let target = draft.agent_mut(agent);
        let official_id = target.add(Mode::Official, "example account", None).unwrap();
        let custom_id = target.add(Mode::Custom, "example API", None).unwrap();
        for id in [&official_id, &custom_id] {
            assert_eq!(id.len(), 8);
            assert!(id.bytes().all(|c| c.is_ascii_hexdigit()));
            assert!(ids.insert(id.clone()));
        }
        target.custom_fields.insert(
            custom_id,
            CustomFields {
                provider_name: "example".into(),
                base_url: "https://api.example.com/v1".into(),
                api_key: "example-key".into(),
                model: "example-model".into(),
                effort: "medium".into(),
                ..Default::default()
            },
        );
    }
    session.save(&session.view().revision, &draft).unwrap();
    let baseline = store.snapshot().unwrap();
    for cross_client in [false, true] {
        let mut invalid = session.original.clone();
        let repeated = invalid.codex.library.official_accounts[0].id.clone();
        let library = if cross_client {
            &mut invalid.claude.library
        } else {
            &mut invalid.codex.library
        };
        let mode = if cross_client {
            Mode::Official
        } else {
            Mode::Custom
        };
        library.profiles_mut(mode)[0].id = repeated.clone();
        library.select(mode, repeated);
        let error = session
            .save(&session.view().revision, &invalid)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("全局唯一"), "{error}");
        assert_eq!(baseline, store.snapshot().unwrap());
        let libraries = acs_core::Libraries {
            codex: invalid.codex.library,
            claude: invalid.claude.library,
        };
        fs::write(
            store.roots.acs.join("profiles.json"),
            serde_json::to_vec(&libraries).unwrap(),
        )
        .unwrap();
        let corrupted = store.snapshot().unwrap();
        assert!(Session::open(store.clone())
            .err()
            .unwrap()
            .to_string()
            .contains("全局唯一"));
        assert_eq!(corrupted, store.snapshot().unwrap());
        fs::write(
            store.roots.acs.join("profiles.json"),
            baseline[0].as_ref().unwrap(),
        )
        .unwrap();
    }
    let mut invalid = session.original.clone();
    invalid.codex.library.official_accounts[0].id = "a".repeat(32);
    invalid.codex.library.selected_official = "a".repeat(32);
    assert!(session
        .save(&session.view().revision, &invalid)
        .err()
        .unwrap()
        .to_string()
        .contains("8 位"));
    assert_eq!(baseline, store.snapshot().unwrap());
}

#[test]
fn claude_login_refresh_keeps_the_existing_id_and_unknown_accounts_are_not_added() {
    let (_dir, store) = fixture();
    let mut session = Session::open(store.clone()).unwrap();
    let mut draft = session.original.clone();
    let id = draft
        .claude
        .add(Mode::Official, "example account", None)
        .unwrap();
    session
        .save_for(&session.view().revision, &draft, Agent::Claude)
        .unwrap();
    let credentials_path = store.roots.claude.join(".credentials.json");
    fs::write(
        &credentials_path,
        r#"{"claudeAiOauth":{"accessToken":"example-access","refreshToken":"example-refresh"}}"#,
    )
    .unwrap();
    fs::write(&store.roots.claude_json, r#"{"oauthAccount":{"accountUuid":"example-account","emailAddress":"account@example.com"}}"#).unwrap();
    let mut session = Session::open(store.clone()).unwrap();
    assert_eq!(session.original.claude.library.selected_official, id);
    assert_eq!(session.original.claude.library.official_accounts.len(), 1);
    assert_eq!(
        session.original.claude.library.official_accounts[0]
            .account_id
            .as_deref(),
        Some("example-account")
    );
    session
        .save_for(
            &session.view().revision,
            &session.original.clone(),
            Agent::Claude,
        )
        .unwrap();
    fs::write(
        &credentials_path,
        r#"{"claudeAiOauth":{"accessToken":"example-refreshed","refreshToken":"example-refresh"}}"#,
    )
    .unwrap();
    let refreshed = Session::open(store.clone()).unwrap();
    assert_eq!(
        refreshed.original.claude.library.official_accounts[0].id,
        id
    );
    assert!(refreshed.original.claude.library.official_accounts[0]
        .official_auth
        .as_ref()
        .unwrap()
        .contains("example-refreshed"));
    fs::write(
        &store.roots.claude_json,
        r#"{"oauthAccount":{"accountUuid":"example-other"}}"#,
    )
    .unwrap();
    assert_eq!(
        Session::open(store.clone())
            .unwrap()
            .original
            .claude
            .library
            .official_accounts
            .len(),
        1
    );
    fs::remove_file(store.roots.acs.join("profiles.json")).unwrap();
    let before = store.snapshot().unwrap();
    assert!(Session::open(store.clone())
        .unwrap()
        .original
        .claude
        .library
        .official_accounts
        .is_empty());
    assert_eq!(before, store.snapshot().unwrap());
}
#[test]
fn proxy_validation_roundtrip_and_parent_environment_unchanged() {
    for invalid in [
        "https://example.com:80",
        "http://user:pass@example.com:80",
        "http://example.com/path",
        "http://example.com:0",
        "http://example.com:80?q=1",
    ] {
        assert!(normalize_proxy(invalid).is_err(), "{invalid}");
    }
    for original in ["", "EXAMPLE=1", "EXAMPLE=1\n", "EXAMPLE=1\r\n"] {
        let updated = proxy_env(original, "http://127.0.0.1:7890").unwrap();
        assert_eq!(
            proxy_env(&updated, "http://127.0.0.1:7890").unwrap(),
            updated
        );
        assert_eq!(proxy_env(&updated, "").unwrap(), original);
    }
    // 2.x 的旧标记必须继续被清理，否则遗留代理变量会继续生效。
    assert!(proxy_env("# END CHATGPT API ONLY PROXY\n", "").is_err());
    let env_before: std::collections::BTreeMap<_, _> = std::env::vars_os().collect();
    let settings = acs_core::launch::Settings {
        agent: Agent::Codex,
        mode: Mode::Official,
        proxy: "http://127.0.0.1:7890".into(),
        roots: Roots::under(Path::new("example")),
    };
    let plan = settings.plan("example".into(), true, vec![]).unwrap();
    let _command = plan.command();
    assert!(plan.args[0].starts_with("--proxy-server="));
    assert!(plan.remove_env.contains(&"OPENAI_API_KEY".into()));
    assert_eq!(env_before, std::env::vars_os().collect());
    let cli = settings.plan("example".into(), false, vec![]).unwrap();
    assert!(cli.args.is_empty());
}
#[test]
fn custom_api_proxy_is_per_profile_and_replaces_domain_blocking() {
    const PROXY: &str = "http://127.0.0.1:7890";
    let (_dir, store) = fixture();
    let mut s = Session::open(store.clone()).unwrap();
    let id = custom(&mut s);
    let direct = s
        .launch_settings(&s.view().revision, &s.original.clone())
        .unwrap();
    let plan = direct.plan("example".into(), true, vec![]).unwrap();
    assert!(plan.args[0].starts_with("--host-resolver-rules="));
    assert!(plan.env.iter().all(|(k, _)| k != "HTTPS_PROXY"));

    let mut draft = s.original.clone();
    draft.codex.custom_fields.get_mut(&id).unwrap().proxy_url = "ftp://example".into();
    let before = store.snapshot().unwrap();
    assert!(s.save(&s.view().revision, &draft).is_err());
    assert_eq!(before, store.snapshot().unwrap());

    draft.codex.custom_fields.get_mut(&id).unwrap().proxy_url = format!("{PROXY}/");
    s.save(&s.view().revision, &draft).unwrap();
    assert!(read(&store, ".env").contains(&format!("HTTPS_PROXY={PROXY}")));
    let mut s = Session::open(store.clone()).unwrap();
    assert_eq!(s.original.codex.custom_fields[&id].proxy_url, PROXY);
    // 无变更再次保存保持幂等。
    let before = store.snapshot().unwrap();
    s.save(&s.view().revision, &s.original.clone()).unwrap();
    assert_eq!(before, store.snapshot().unwrap());
    let proxied = s
        .launch_settings(&s.view().revision, &s.original.clone())
        .unwrap();
    let plan = proxied.plan("example".into(), true, vec![]).unwrap();
    assert_eq!(plan.args, vec![format!("--proxy-server={PROXY}")]);
    assert!(plan
        .env
        .contains(&("HTTPS_PROXY".to_string(), PROXY.to_string())));

    // 官方账号不继承自定义 API 的代理。
    let mut draft = s.original.clone();
    draft.codex.mode = Mode::Official;
    draft
        .codex
        .add(Mode::Official, "example account", None)
        .unwrap();
    s.save(&s.view().revision, &draft).unwrap();
    assert!(!read(&store, ".env").contains("HTTPS_PROXY"));

    let mut draft = s.original.clone();
    draft.claude.mode = Mode::Custom;
    let claude = draft
        .claude
        .add(Mode::Custom, "example Claude", None)
        .unwrap();
    draft.claude.custom_fields.insert(
        claude.clone(),
        CustomFields {
            base_url: "https://claude.example.com".into(),
            api_key: "example-claude-key".into(),
            proxy_url: PROXY.into(),
            ..Default::default()
        },
    );
    s.save_for(&s.view().revision, &draft, Agent::Claude)
        .unwrap();
    let settings: Value =
        serde_json::from_slice(&fs::read(store.roots.claude.join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(settings["env"]["HTTPS_PROXY"], PROXY);
    assert_eq!(
        settings["env"]["ANTHROPIC_BASE_URL"],
        "https://claude.example.com"
    );
    let mut s = Session::open(store.clone()).unwrap();
    assert_eq!(s.original.claude.custom_fields[&claude].proxy_url, PROXY);
    let launch = s
        .launch_settings_for(&s.view().revision, &s.original.clone(), Agent::Claude)
        .unwrap();
    assert_eq!(launch.proxy, PROXY);

    let mut draft = s.original.clone();
    draft
        .claude
        .custom_fields
        .get_mut(&claude)
        .unwrap()
        .proxy_url = String::new();
    s.save_for(&s.view().revision, &draft, Agent::Claude)
        .unwrap();
    let settings: Value =
        serde_json::from_slice(&fs::read(store.roots.claude.join("settings.json")).unwrap())
            .unwrap();
    assert!(settings["env"]["HTTPS_PROXY"].is_null());
    assert!(settings["env"]["NODE_USE_ENV_PROXY"].is_null());
    assert_eq!(
        settings["env"]["ANTHROPIC_AUTH_TOKEN"],
        "example-claude-key"
    );
}
#[test]
fn invalid_proxy_marker_blocks_all_four_file_changes() {
    let (_dir, store) = fixture();
    let mut s = Session::open(store.clone()).unwrap();
    official(&mut s);
    // 2.x 旧标记若残留且不完整，同样必须阻止写入。
    fs::write(
        store.roots.codex.join(".env"),
        "# BEGIN CHATGPT API ONLY PROXY\n",
    )
    .unwrap();
    let mut s = Session::open(store.clone()).unwrap();
    let before = store.snapshot().unwrap();
    assert!(s.save(&s.view().revision, &s.original.clone()).is_err());
    assert_eq!(before, store.snapshot().unwrap());
}
#[test]
fn conflicting_routing_is_not_silently_removed() {
    for setting in [
        "profile='example'",
        "forced_login_method='chatgpt'",
        "cli_auth_credentials_store='keyring'",
        "openai_base_url='https://example.com/v1'",
    ] {
        let (_dir, store) = fixture();
        fs::write(store.roots.codex.join("config.toml"), setting).unwrap();
        let mut s = Session::open(store.clone()).unwrap();
        let mut draft = s.original.clone();
        draft.codex.add(Mode::Official, "example", None).unwrap();
        let before = store.snapshot().unwrap();
        assert!(s.save(&s.view().revision, &draft).is_err());
        assert_eq!(before, store.snapshot().unwrap());
    }
}
#[test]
fn repair_success_backup_metadata_and_idempotence() {
    let (_dir, store) = fixture();
    let original = seed_rollout(&store);
    let db = database(&store, "state_5.sqlite", false);
    db.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    let events = RefCell::new(Vec::new());
    let result = history::repair(&store, |p| events.borrow_mut().push(p)).unwrap();
    assert_eq!(result.changed, 4);
    let backup = result.backup.unwrap();
    assert_eq!(
        fs::read_to_string(backup.join("sessions/example.jsonl")).unwrap(),
        original
    );
    let backup_db = Connection::open(backup.join("state_5.sqlite")).unwrap();
    assert_eq!(
        backup_db
            .query_row(
                "SELECT COUNT(*) FROM threads WHERE model_provider IS NOT 'custom'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    let result = read(&store, "sessions/example.jsonl");
    assert!(result.contains("\"name\":\"example\""));
    assert!(result.contains("\r\n"));
    assert_eq!(history::repair(&store, |_| {}).unwrap().changed, 0);
    assert!(events.borrow().iter().all(|p| p.completed <= p.total));
}
#[test]
fn second_database_failure_rolls_back_all_databases_and_rollouts() {
    let (_dir, store) = fixture();
    let original = seed_rollout(&store);
    let a = database(&store, "sqlite/a.db", false);
    let b = database(&store, "sqlite/b.db", true);
    assert!(history::repair(&store, |_| {}).is_err());
    assert_eq!(read(&store, "sessions/example.jsonl"), original);
    for db in [a, b] {
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM threads WHERE model_provider IS NOT 'custom'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
    }
    assert!(store
        .roots
        .codex
        .join("backups_state/provider-sync")
        .exists());
}

#[test]
fn unrelated_databases_are_not_attached_or_backed_up() {
    let (_dir, store) = fixture();
    let _history = database(&store, "state_5.sqlite", false);
    fs::create_dir_all(store.roots.codex.join("sqlite")).unwrap();
    let mut unrelated = Vec::new();
    for i in 0..12 {
        let db =
            Connection::open(store.roots.codex.join(format!("sqlite/example-{i}.db"))).unwrap();
        db.execute_batch("CREATE TABLE settings (value TEXT); INSERT INTO settings VALUES ('example'); BEGIN IMMEDIATE;").unwrap();
        unrelated.push(db);
    }
    let result = history::repair(&store, |_| {}).unwrap();
    assert_eq!(result.changed, 3);
    assert!(!result.backup.unwrap().join("sqlite").exists());
    for db in unrelated {
        db.execute_batch("ROLLBACK;").unwrap();
    }
}
#[test]
fn changing_rollout_aborts_sql_transaction() {
    let (_dir, store) = fixture();
    seed_rollout(&store);
    let db = database(&store, "state_5.sqlite", false);
    let result = history::repair(&store, |p| {
        if p.phase == "修复对话" && p.completed == 0 {
            fs::write(
                store.roots.codex.join("sessions/example.jsonl"),
                "example external write",
            )
            .unwrap();
        }
    });
    assert!(result.is_err());
    assert_eq!(
        read(&store, "sessions/example.jsonl"),
        "example external write"
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM threads WHERE model_provider IS NOT 'custom'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}
#[test]
fn failed_rollout_after_first_write_restores_first_and_sql() {
    let (_dir, store) = fixture();
    let original = seed_rollout(&store);
    fs::write(store.roots.codex.join("sessions/second.jsonl"), &original).unwrap();
    let db = database(&store, "state_5.sqlite", false);
    let result = history::repair(&store, |p| {
        if p.phase == "修复对话" && p.completed == 4 {
            fs::write(
                store.roots.codex.join("sessions/second.jsonl"),
                "example external write",
            )
            .unwrap();
        }
    });
    assert!(result.is_err());
    assert_eq!(read(&store, "sessions/example.jsonl"), original);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM threads WHERE model_provider IS NOT 'custom'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}
#[test]
fn window_geometry_persists_without_disturbing_the_saved_file_baseline() {
    let (_dir, store) = fixture();
    let mut session = Session::open(store.clone()).unwrap();
    custom(&mut session);
    let revision = session.view().revision;
    let state = WindowState {
        width: 900.0,
        height: 660.0,
        x: 120.0,
        y: 60.0,
        maximized: false,
    };
    store.set_window_state(&state);
    assert_eq!(store.window_state(), Some(state));
    // 窗口几何不属于目标文件基线，记住尺寸不得让已打开的草稿失效。
    session.save(&revision, &session.original.clone()).unwrap();
}
