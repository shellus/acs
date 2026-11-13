use acs_core::{
    environment::{command::Runner, *},
    storage, Agent, CustomFields, Mode, Session, Store,
};
use anyhow::{bail, Result};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};
struct Fake {
    root: PathBuf,
    calls: RefCell<Vec<Vec<String>>>,
    fail_install: bool,
}
impl Fake {
    fn new(root: &Path) -> Self {
        Self {
            root: root.into(),
            calls: RefCell::new(vec![]),
            fail_install: false,
        }
    }
}
impl Runner for Fake {
    fn run(&self, program: &str, args: &[OsString], dir: Option<&Path>) -> Result<Vec<u8>> {
        let a: Vec<_> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        self.calls
            .borrow_mut()
            .push(std::iter::once(program.into()).chain(a.clone()).collect());
        if a.first().is_some_and(|s| s == "--version") {
            return Ok(b"example version 1.0.0".to_vec());
        }
        if program == "git" {
            bail!("example local repository");
        }
        assert_eq!(program, "npm");
        match a[0].as_str() {
            "root" => Ok(self.root.to_string_lossy().as_bytes().to_vec()),
            "install" => {
                if self.fail_install {
                    bail!("example install failure");
                }
                let (name, version) = a[2].rsplit_once('@').unwrap();
                put(
                    &self.root.join(name).join("package.json"),
                    &format!(r#"{{"version":"{version}"}}"#),
                );
                Ok(vec![])
            }
            "uninstall" => {
                let p = self.root.join(&a[2]);
                if p.exists() {
                    fs::remove_dir_all(p)?;
                }
                Ok(vec![])
            }
            "ci" => {
                put(
                    &dir.unwrap().join("node_modules/example/index.js"),
                    "example module\n",
                );
                Ok(vec![])
            }
            _ => panic!("unexpected npm {a:?}"),
        }
    }
}
fn put(path: &Path, text: &str) {
    storage::write(path, Some(text.as_bytes())).unwrap();
}
fn setup() -> (tempfile::TempDir, Paths, PathBuf, Fake) {
    let d = tempfile::tempdir().unwrap();
    let paths = Paths::under(d.path());
    let repo = d.path().join("repo");
    put(
        &repo.join("environment.yaml"),
        include_str!("fixtures/environment/environment.yaml"),
    );
    put(
        &repo.join("environment.lock"),
        include_str!("fixtures/environment/environment.lock"),
    );
    put(
        &repo.join("instructions/global.md"),
        "example global rules\n",
    );
    put(
        &repo.join("skills/local/example-demo/SKILL.md"),
        "---\nname: example-demo\n---\nexample\n",
    );
    put(
        &repo.join("skills/vendor/.vendor-snapshot.yaml"),
        "version: 1\nsources: {}\nskills: {}\n",
    );
    set_source(&paths, repo.to_str().unwrap(), None).unwrap();
    configure(&paths, Some("default"), Some(Agent::ALL.to_vec())).unwrap();
    let fake = Fake::new(&d.path().join("npm"));
    (d, paths, repo, fake)
}
fn files(paths: &Paths) -> BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)> {
    let mut values = BTreeMap::new();
    for root in [&paths.roots.codex, &paths.roots.claude, &paths.codex_skills] {
        for e in walkdir::WalkDir::new(root) {
            let Ok(e) = e else { continue };
            if e.file_type().is_file() {
                values.insert(
                    e.path().into(),
                    (
                        fs::read(e.path()).unwrap(),
                        fs::metadata(e.path()).unwrap().modified().unwrap(),
                    ),
                );
            }
        }
    }
    for path in [paths.config(), paths.state()] {
        if path.exists() {
            values.insert(
                path.clone(),
                (
                    fs::read(&path).unwrap(),
                    fs::metadata(path).unwrap().modified().unwrap(),
                ),
            );
        }
    }
    values
}
#[test]
fn applies_takeover_updates_removals_retirement_and_preserves_unmanaged_content() {
    let (_d, p, repo, f) = setup();
    let s = Service {
        paths: &p,
        runner: &f,
    };
    put(
        &p.codex_skills.join("example-demo/SKILL.md"),
        "example manual",
    );
    put(&p.codex_skills.join("personal/SKILL.md"), "example keep");
    put(&p.roots.claude.join("skills"), "example legacy root file");
    let legacy = p.roots.codex.join("skills");
    put(&legacy.join("old-managed/SKILL.md"), "example old");
    put(&legacy.join("personal/SKILL.md"), "example keep");
    storage::write(
        &legacy.join(MANAGED_MARKER),
        Some(
            &ManagedMarker {
                version: 1,
                skills: BTreeMap::from([("old-managed".into(), "example-hash".into())]),
            }
            .encode()
            .unwrap(),
        ),
    )
    .unwrap();
    let plan = s.prepare().unwrap();
    assert!(plan.has_changes());
    assert!(plan.changes.iter().any(|c| c.contains("非受管")));
    s.apply(&plan).unwrap();
    assert_eq!(
        fs::read_to_string(p.codex_skills.join("personal/SKILL.md")).unwrap(),
        "example keep"
    );
    assert!(!legacy.join("old-managed").exists());
    assert!(legacy.join("personal").exists());
    assert!(!legacy.join(MANAGED_MARKER).exists());
    assert!(p.roots.claude.join("skills").is_dir());
    let before = files(&p);
    let plan = s.prepare().unwrap();
    assert!(!plan.has_changes(), "{:?}", plan.changes);
    s.apply(&plan).unwrap();
    assert_eq!(before, files(&p));
    put(
        &p.codex_skills.join("example-demo/SKILL.md"),
        "example user edit",
    );
    assert!(s
        .prepare()
        .unwrap()
        .changes
        .iter()
        .any(|c| c.contains("更新")));
    s.apply(&s.prepare().unwrap()).unwrap();
    put(
        &repo.join("skills/local/example-extra/SKILL.md"),
        "---\nname: example-extra\n---\n",
    );
    let manifest = fs::read_to_string(repo.join("environment.yaml"))
        .unwrap()
        .replace("include: ['*']", "include: ['example-extra']")
        .replace("include: ['local:example-*']", "include: []");
    put(&repo.join("environment.yaml"), &manifest);
    s.apply(&s.prepare().unwrap()).unwrap();
    assert!(!p.codex_skills.join("example-demo").exists());
    assert!(p.codex_skills.join("example-extra").exists());
    assert!(p.codex_skills.join("personal").exists());
}
#[test]
fn state_recording_legacy_codex_root_retires_it_once() {
    let (_d, p, _repo, f) = setup();
    let s = Service {
        paths: &p,
        runner: &f,
    };
    let legacy = p.roots.codex.join("skills");
    put(&legacy.join("old-managed/SKILL.md"), "example old");
    put(&legacy.join("personal/SKILL.md"), "example keep");
    let skills = BTreeMap::from([("old-managed".into(), "example-hash".into())]);
    storage::write(
        &legacy.join(MANAGED_MARKER),
        Some(
            &ManagedMarker {
                version: 1,
                skills: skills.clone(),
            }
            .encode()
            .unwrap(),
        ),
    )
    .unwrap();
    let mut state = State::empty();
    state.agents.insert(
        Agent::Codex,
        AgentState {
            version: "1.0.0".into(),
            instruction_hash: "example".into(),
            skills,
            skills_root: legacy.clone(),
        },
    );
    storage::write(&p.state(), Some(&state.encode().unwrap())).unwrap();
    s.apply(&s.prepare().unwrap()).unwrap();
    assert!(!legacy.join("old-managed").exists());
    assert!(legacy.join("personal").exists());
    assert!(!s.prepare().unwrap().has_changes());
}
#[test]
fn stale_plan_bad_markers_and_failed_install_never_write_targets() {
    let (_d, p, _repo, f) = setup();
    let s = Service {
        paths: &p,
        runner: &f,
    };
    let plan = s.prepare().unwrap();
    put(
        &p.roots.codex.join("config.toml"),
        "model = 'example-external'\n",
    );
    let before = files(&p);
    assert!(s.apply(&plan).is_err());
    assert_eq!(before, files(&p));
    put(&p.roots.codex.join("config.toml"), DISABLED_BEGIN);
    assert!(s.prepare().is_err());
}
#[test]
fn disabled_blocks_preserve_text_newlines_and_reject_damage() {
    for nl in ["\n", "\r\n"] {
        let original = format!(
            "model = 'example'{nl}{nl}[[skills.config]]{nl}name = 'manual'{nl}enabled = true{nl}"
        );
        let updated =
            update_disabled(&original, &["example-b".into(), "example-a".into()]).unwrap();
        assert!(updated.starts_with(&original));
        assert_eq!(
            updated,
            update_disabled(&updated, &["example-b".into(), "example-a".into()]).unwrap()
        );
        assert_eq!(original, update_disabled(&updated, &[]).unwrap());
        if nl == "\r\n" {
            assert!(!updated.replace("\r\n", "").contains('\n'));
        }
    }
    for text in [
        DISABLED_BEGIN.to_string(),
        DISABLED_END.into(),
        format!("{DISABLED_BEGIN}\n{DISABLED_BEGIN}\n{DISABLED_END}\n"),
    ] {
        assert!(update_disabled(&text, &["example".into()]).is_err());
    }
}
#[test]
fn account_save_keeps_disabled_block_and_environment_apply_keeps_account() {
    let (_d, p, _repo, f) = setup();
    let service = Service {
        paths: &p,
        runner: &f,
    };
    let plan = service.prepare().unwrap();
    for agent in Agent::ALL {
        assert!(plan
            .changes
            .contains(&format!("{} Skills 受管标记：更新", agent.label())));
    }
    assert!(!plan.changes.contains(&"Skills 受管标记：更新".into()));
    service.apply(&plan).unwrap();
    let mut session = Session::open(Store::new(p.roots.clone())).unwrap();
    let mut draft = session.original.clone();
    draft.codex.mode = Mode::Custom;
    let id = draft.codex.add(Mode::Custom, "example API", None).unwrap();
    draft.codex.custom_fields.insert(
        id,
        CustomFields {
            provider_name: "example".into(),
            base_url: "https://api.example.com/v1".into(),
            api_key: "example-key".into(),
            model: "example-model".into(),
            effort: "medium".into(),
            ..Default::default()
        },
    );
    session.save(&session.view().revision, &draft).unwrap();
    let config = fs::read_to_string(p.roots.codex.join("config.toml")).unwrap();
    assert_eq!(config.matches(DISABLED_BEGIN).count(), 1);
    assert_eq!(config.matches(DISABLED_END).count(), 1);
    assert!(config.contains("example-disabled"));
    acs_core::codex::parse_doc(&config).unwrap();
    let account = fs::read(p.roots.codex.join("auth.json")).unwrap();
    let before = files(&p);
    let plan = service.prepare().unwrap();
    assert!(!plan.has_changes(), "{:?}", plan.changes);
    service.apply(&plan).unwrap();
    assert_eq!(before, files(&p));
    assert_eq!(account, fs::read(p.roots.codex.join("auth.json")).unwrap());
}
#[test]
fn npm_versions_minimum_pin_prerelease_and_unparseable_versions() {
    let p = AgentPackage {
        agent: Agent::Codex,
        name: "example-package".into(),
        version: "1.2.3".into(),
        pin: false,
    };
    for (v, ok) in [
        ("", false),
        ("1.2.2", false),
        ("1.2.3", true),
        ("v1.2.3", true),
        ("1.10.0", true),
        ("1.2.3-alpha.1", false),
        ("1.2.3+build.5", true),
        ("dev-build", true),
    ] {
        assert_eq!(p.satisfied(v), ok, "{v}");
    }
    let mut p = p;
    p.pin = true;
    assert!(p.satisfied("1.2.3"));
    assert!(!p.satisfied("1.3.0"));
    assert_eq!(p.requirement(), "=1.2.3");
}
#[test]
fn npm_install_status_uninstall_and_state_use_actual_newer_version() {
    let (_d, p, repo, f) = setup();
    put(
        &f.root.join("@openai/codex/package.json"),
        r#"{"version":"1.3.0"}"#,
    );
    put(
        &f.root.join("@anthropic-ai/claude-code/package.json"),
        r#"{"version":"2.1.0"}"#,
    );
    let m = fs::read_to_string(repo.join("environment.yaml"))
        .unwrap()
        .replace(
            "package: '@anthropic-ai/claude-code'",
            "package: '@anthropic-ai/claude-code', pin: true",
        );
    put(&repo.join("environment.yaml"), &m);
    let s = Service {
        paths: &p,
        runner: &f,
    };
    s.apply(&s.prepare().unwrap()).unwrap();
    let state = State::load(&p.state()).unwrap();
    assert_eq!(state.agents[&Agent::Codex].version, "1.3.0");
    assert_eq!(state.agents[&Agent::Claude].version, "2.0.0");
    assert!(s.agent_status(&Agent::ALL).unwrap()[0].contains("1.3.0"));
    assert!(!f
        .calls
        .borrow()
        .iter()
        .any(|a| a.iter().any(|a| a == "@openai/codex@1.0.0")));
    put(&p.codex_skills.join("personal/SKILL.md"), "keep");
    s.uninstall(&[Agent::Codex], false).unwrap();
    assert!(!p.codex_skills.join("example-demo").exists());
    assert!(p.codex_skills.join("personal").exists());
    assert!(!p.roots.codex.join("AGENTS.md").exists());
    assert!(!fs::read_to_string(p.roots.codex.join("config.toml"))
        .unwrap()
        .contains(DISABLED_BEGIN));
    assert!(p.roots.claude.join("skills/example-demo").exists());
    assert!(!State::load(&p.state())
        .unwrap()
        .agents
        .contains_key(&Agent::Codex));
}
#[test]
fn migration_windows_and_linux_is_atomic_idempotent_and_followed_by_zero_writes() {
    for platform in [LegacyPlatform::Windows, LegacyPlatform::Unix] {
        let (d, p, _repo, f) = setup();
        let s = Service {
            paths: &p,
            runner: &f,
        };
        s.apply(&s.prepare().unwrap()).unwrap();
        let old = LegacyPaths::resolve(d.path(), platform, |_| None);
        let mut cfg: serde_yaml::Value = load_yaml(&p.config()).unwrap();
        cfg["environment"]["agents"]
            .as_sequence_mut()
            .unwrap()
            .push(serde_yaml::Value::String("happy".into()));
        put(&old.config, &serde_yaml::to_string(&cfg).unwrap());
        let mut state: serde_json::Value =
            serde_json::from_slice(&fs::read(p.state()).unwrap()).unwrap();
        state["agents"]["happy"] =
            serde_json::json!({"version":"example","skills":{},"instruction_hash":"example"});
        put(&old.state, &serde_json::to_string_pretty(&state).unwrap());
        fs::remove_file(p.state()).unwrap();
        fs::remove_file(p.config()).unwrap();
        for a in Agent::ALL {
            let root = p.destinations(a).1;
            fs::rename(root.join(MANAGED_MARKER), root.join(".ags-managed.json")).unwrap();
        }
        let path = p.roots.codex.join("config.toml");
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("ACS MANAGED CODEX", "AGS MANAGED CODEX");
        put(&path, &text);
        let messages = import_ags(&p, &old).unwrap();
        assert!(messages.iter().any(|m| m.contains("happy")));
        assert!(old.config.exists() && old.state.exists());
        for a in Agent::ALL {
            let root = p.destinations(a).1;
            assert!(!root.join(".ags-managed.json").exists());
            assert!(root.join(MANAGED_MARKER).exists());
        }
        assert!(!fs::read_to_string(path).unwrap().contains("AGS MANAGED"));
        assert!(!s.prepare().unwrap().has_changes());
        let before = files(&p);
        s.apply(&s.prepare().unwrap()).unwrap();
        assert_eq!(before, files(&p));
        import_ags(&p, &old).unwrap();
        assert_eq!(before, files(&p));
    }
}
#[test]
fn migration_refuses_existing_acs_configuration() {
    let (d, p, _repo, _f) = setup();
    let legacy = LegacyPaths::resolve(d.path(), LegacyPlatform::Unix, |_| None);
    put(&legacy.config, "version: 1\nenvironment: {branch: main}\n");
    put(
        &legacy.state,
        r#"{"version":1,"source":"example","branch":"main","commit":"example","profile":"default","agents":{}}"#,
    );
    let before = files(&p);
    assert!(import_ags(&p, &legacy).is_err());
    assert_eq!(before, files(&p));
}
#[test]
fn node_skill_reuses_unchanged_locked_dependencies_without_running_ci_again() {
    let (_d, p, repo, f) = setup();
    let skill = repo.join("skills/local/example-demo");
    put(
        &skill.join("package.json"),
        r#"{"name":"example","version":"1.0.0"}"#,
    );
    put(&skill.join("package-lock.json"), "{}");
    let s = Service {
        paths: &p,
        runner: &f,
    };
    s.apply(&s.prepare().unwrap()).unwrap();
    let count = f
        .calls
        .borrow()
        .iter()
        .filter(|a| a.get(1).is_some_and(|s| s == "ci"))
        .count();
    assert_eq!(count, 2);
    assert!(!s.prepare().unwrap().has_changes());
    assert_eq!(
        f.calls
            .borrow()
            .iter()
            .filter(|a| a.get(1).is_some_and(|s| s == "ci"))
            .count(),
        count
    );
}
#[test]
fn roots_and_environment_share_exact_override_semantics() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path();
    for value in ["", "~", "~/.example", "example-relative"] {
        let env = |k: &str| match k {
            "ACS_HOME" | "CODEX_HOME" | "CLAUDE_CONFIG_DIR" | "ACS_CLAUDE_DESKTOP_DIR" => {
                Some(PathBuf::from(value))
            }
            _ => None,
        };
        let env_paths = Paths::resolve(home, env).unwrap();
        let roots = acs_core::Roots::resolve(home, &home.join("desktop-parent"), env).unwrap();
        assert_eq!(env_paths.roots, roots);
        assert_eq!(roots.acs, PathBuf::from(value));
        assert_eq!(roots.claude_json, PathBuf::from(value).join(".claude.json"));
    }
}
#[test]
fn missing_node_lock_and_failed_agent_install_are_non_destructive() {
    let (_d, p, repo, mut f) = setup();
    f.fail_install = true;
    let s = Service {
        paths: &p,
        runner: &f,
    };
    let plan = s.prepare().unwrap();
    let before = files(&p);
    assert!(s.apply(&plan).is_err());
    assert_eq!(before, files(&p));
    put(&repo.join("skills/local/example-demo/package.json"), "{}");
    assert!(s.prepare().is_err());
    assert_eq!(before, files(&p));
}
#[test]
fn legacy_paths_cover_overrides_and_system_layouts() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    let windows = LegacyPaths::resolve(h, LegacyPlatform::Windows, |_| None);
    assert_eq!(
        windows.state,
        h.join("AppData/Roaming/ags/environment.json")
    );
    assert_eq!(windows.cache, h.join("AppData/Local/ags"));
    let linux = LegacyPaths::resolve(h, LegacyPlatform::Unix, |_| None);
    assert_eq!(linux.state, h.join(".local/state/ags/environment.json"));
    assert_eq!(linux.cache, h.join(".cache/ags"));
    let overridden = LegacyPaths::resolve(h, LegacyPlatform::Unix, |k| match k {
        "AGS_STATE_DIR" => Some("~/example-state".into()),
        "XDG_STATE_HOME" => Some("~/unused-state".into()),
        _ => None,
    });
    assert_eq!(
        overridden.state,
        h.join("example-state/ags/environment.json")
    );
}
#[test]
fn migration_with_node_runtime_keeps_rendered_hash_and_zero_writes() {
    let (d, p, repo, f) = setup();
    let skill = repo.join("skills/local/example-demo");
    put(&skill.join("package.json"), "{}");
    put(&skill.join("package-lock.json"), "{}");
    let s = Service {
        paths: &p,
        runner: &f,
    };
    s.apply(&s.prepare().unwrap()).unwrap();
    let old = LegacyPaths::resolve(d.path(), LegacyPlatform::Unix, |_| None);
    put(&old.config, &fs::read_to_string(p.config()).unwrap());
    put(&old.state, &fs::read_to_string(p.state()).unwrap());
    fs::remove_file(p.config()).unwrap();
    fs::remove_file(p.state()).unwrap();
    for a in Agent::ALL {
        let r = p.destinations(a).1;
        fs::rename(r.join(MANAGED_MARKER), r.join(".ags-managed.json")).unwrap();
    }
    let c = p.roots.codex.join("config.toml");
    put(
        &c,
        &fs::read_to_string(&c)
            .unwrap()
            .replace("ACS MANAGED CODEX", "AGS MANAGED CODEX"),
    );
    import_ags(&p, &old).unwrap();
    let before = files(&p);
    let count = f
        .calls
        .borrow()
        .iter()
        .filter(|a| a.get(1).is_some_and(|s| s == "ci"))
        .count();
    let plan = s.prepare().unwrap();
    assert!(!plan.has_changes(), "{:?}", plan.changes);
    s.apply(&plan).unwrap();
    assert_eq!(before, files(&p));
    assert_eq!(
        count,
        f.calls
            .borrow()
            .iter()
            .filter(|a| a.get(1).is_some_and(|s| s == "ci"))
            .count()
    );
}
#[test]
fn version_comparison_covers_stable_prerelease_and_metadata_cases() {
    use std::cmp::Ordering::*;
    for (a, b, expected) in [
        ("2.1.295", "2.1.291", Greater),
        ("2.1.291", "2.1.295", Less),
        ("0.160.1", "0.160.1", Equal),
        ("1.10.0", "1.9.9", Greater),
        ("v1.2.3", "1.2.3", Equal),
        ("1.2.3+build.5", "1.2.3", Equal),
        ("1.2.3-alpha.1", "1.2.3", Less),
        ("1.2.3-alpha.2", "1.2.3-alpha.10", Less),
        ("1.2.3-alpha.1", "1.2.3-beta", Less),
        ("1.2.3-1", "1.2.3-alpha", Less),
        ("1.2.3-alpha", "1.2.3-alpha.1", Less),
    ] {
        assert_eq!(compare_versions(a, b), Some(expected));
    }
    for invalid in ["", "1.2", "1.2.x", "1.2.3-", "1.2.3-a..b", "latest"] {
        assert!(compare_versions(invalid, "1.2.3").is_none());
    }
}
#[cfg(unix)]
#[test]
fn applying_guidance_replaces_link_without_modifying_original_target() {
    let (d, p, _repo, f) = setup();
    let target = d.path().join("example-shared-guidance");
    put(&target, "example original");
    fs::create_dir_all(&p.roots.codex).unwrap();
    std::os::unix::fs::symlink(&target, p.roots.codex.join("AGENTS.md")).unwrap();
    let s = Service {
        paths: &p,
        runner: &f,
    };
    s.apply(&s.prepare().unwrap()).unwrap();
    assert!(!p.roots.codex.join("AGENTS.md").is_symlink());
    assert_eq!(fs::read_to_string(target).unwrap(), "example original");
}
#[test]
fn unchanged_plan_is_invalidated_by_skills_edits_during_confirmation() {
    let (_d, p, _repo, f) = setup();
    let s = Service {
        paths: &p,
        runner: &f,
    };
    s.apply(&s.prepare().unwrap()).unwrap();
    let plan = s.prepare().unwrap();
    assert!(!plan.has_changes());
    put(
        &p.codex_skills.join("example-demo/SKILL.md"),
        "example external change",
    );
    let before = files(&p);
    assert!(s.apply(&plan).is_err());
    assert_eq!(before, files(&p));
}
#[test]
fn source_or_installed_version_changes_invalidate_the_displayed_plan_before_any_write() {
    for source in [
        "environment.yaml",
        "environment.lock",
        "instructions/global.md",
        "skills/local/example-demo/SKILL.md",
        "skills/vendor/.vendor-snapshot.yaml",
        "installed-version",
    ] {
        let (_d, p, repo, f) = setup();
        let service = Service {
            paths: &p,
            runner: &f,
        };
        service.apply(&service.prepare().unwrap()).unwrap();
        let plan = service.prepare().unwrap();
        assert_eq!(plan.config, LocalConfig::load(&p.config()).unwrap());
        if source == "installed-version" {
            put(
                &f.root.join("@openai/codex/package.json"),
                r#"{"version":"1.1.0"}"#,
            );
        } else {
            let path = repo.join(source);
            let mut text = fs::read_to_string(&path).unwrap();
            text.push_str("\n# example external change\n");
            put(&path, &text);
        }
        let before = files(&p);
        let error = service.apply(&plan).unwrap_err();
        assert!(error.is::<StalePlan>(), "{error:#}");
        assert_eq!(files(&p), before);
        assert!(!service.prepare().unwrap().has_changes() || source != "installed-version");
    }
}
#[test]
fn broken_uninstall_targets_do_not_remove_npm_package() {
    let (_d, p, _repo, f) = setup();
    let s = Service {
        paths: &p,
        runner: &f,
    };
    s.apply(&s.prepare().unwrap()).unwrap();
    let guidance = p.roots.codex.join("AGENTS.md");
    fs::remove_file(&guidance).unwrap();
    fs::create_dir(&guidance).unwrap();
    assert!(s.uninstall(&[Agent::Codex], false).is_err());
    assert!(!f
        .calls
        .borrow()
        .iter()
        .any(|a| a.get(1).is_some_and(|s| s == "uninstall")));
}
#[cfg(windows)]
#[test]
#[ignore = "subprocess-only working-directory occupancy probe"]
fn occupied_directory_probe() {
    use std::io::{self, Write};
    let path = std::env::var_os("ACS_EXAMPLE_OCCUPIED_PATH").unwrap();
    std::env::set_current_dir(path).unwrap();
    println!("example-cwd-ready");
    io::stdout().flush().unwrap();
    let mut text = String::new();
    io::stdin().read_line(&mut text).unwrap();
}
#[cfg(windows)]
struct Occupied(std::process::Child);
#[cfg(windows)]
impl Drop for Occupied {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[cfg(windows)]
fn occupy(path: &Path) -> Occupied {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
    };
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "occupied_directory_probe",
            "--ignored",
            "--nocapture",
        ])
        .env("ACS_EXAMPLE_OCCUPIED_PATH", path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        assert!(
            reader.read_line(&mut line).unwrap() > 0,
            "occupancy child exited"
        );
        if line.contains("example-cwd-ready") {
            break;
        }
        line.clear();
    }
    Occupied(child)
}
#[cfg(windows)]
#[test]
fn unrelated_occupied_skill_does_not_block_apply_and_occupied_changed_skill_rolls_back() {
    let (_d, p, repo, f) = setup();
    for name in ["example-before", "example-extra"] {
        put(
            &repo.join(format!("skills/local/{name}/SKILL.md")),
            &format!("---\nname: {name}\n---\nexample old\n"),
        );
    }
    let s = Service {
        paths: &p,
        runner: &f,
    };
    s.apply(&s.prepare().unwrap()).unwrap();
    let extra = p.codex_skills.join("example-extra");
    let original = fs::metadata(&extra).unwrap().created().unwrap();
    let busy = occupy(&extra);
    put(
        &repo.join("skills/local/example-demo/SKILL.md"),
        "---\nname: example-demo\n---\nexample update\n",
    );
    let plan = s.prepare().unwrap();
    s.apply(&plan).unwrap();
    assert_eq!(fs::metadata(&extra).unwrap().created().unwrap(), original);
    drop(busy);
    put(
        &repo.join("skills/local/example-before/SKILL.md"),
        "---\nname: example-before\n---\nexample update\n",
    );
    put(
        &repo.join("skills/local/example-demo/SKILL.md"),
        "---\nname: example-demo\n---\nexample second update\n",
    );
    let busy = occupy(&p.codex_skills.join("example-demo"));
    let plan = s.prepare().unwrap();
    let before = files(&p);
    let error = s.apply(&plan).unwrap_err();
    assert!(
        error
            .chain()
            .filter_map(|e| e.downcast_ref::<std::io::Error>())
            .any(|e| matches!(e.raw_os_error(), Some(5 | 32))),
        "{error:#}"
    );
    assert_eq!(before, files(&p));
    drop(busy);
}
#[test]
fn thousand_file_single_skill_benchmark() {
    let (_d, p, repo, f) = setup();
    configure(&p, None, Some(vec![Agent::Codex])).unwrap();
    for i in 0..99 {
        let skill = repo.join(format!("skills/local/example-{i:03}"));
        put(
            &skill.join("SKILL.md"),
            &format!("---\nname: example-{i:03}\n---\n"),
        );
        for n in 0..9 {
            put(&skill.join(format!("example-{n}.txt")), "example payload\n");
        }
    }
    let s = Service {
        paths: &p,
        runner: &f,
    };
    s.apply(&s.prepare().unwrap()).unwrap();
    put(
        &repo.join("skills/local/example-demo/SKILL.md"),
        "---\nname: example-demo\n---\nexample changed line\n",
    );
    let start = std::time::Instant::now();
    let plan = s.prepare().unwrap();
    let status = start.elapsed();
    let staged_files = walkdir::WalkDir::new(p.cache().join("builds"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();
    assert_eq!(
        staged_files, 1,
        "single changed Skill must be the only staged payload"
    );

    let start = std::time::Instant::now();
    drop(s.prepare().unwrap());
    let diff = start.elapsed();
    let start = std::time::Instant::now();
    s.apply(&plan).unwrap();
    let apply = start.elapsed();
    let count = walkdir::WalkDir::new(&p.codex_skills)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();
    println!(
        "BENCH files={count} status_ms={} diff_ms={} apply_ms={}",
        status.as_millis(),
        diff.as_millis(),
        apply.as_millis()
    );
    assert!(!s.prepare().unwrap().has_changes());
}
#[test]
fn doctor_reports_good_environment_and_marker_state_inconsistency() {
    let (_d, p, _repo, f) = setup();
    let s = Service {
        paths: &p,
        runner: &f,
    };
    s.apply(&s.prepare().unwrap()).unwrap();
    let report = doctor(&p, &f);
    assert!(report.healthy(), "{:?}", report.checks);
    fs::remove_file(p.codex_skills.join(MANAGED_MARKER)).unwrap();
    let report = doctor(&p, &f);
    assert!(!report.healthy());
    assert!(report
        .checks
        .iter()
        .any(|c| !c.ok && c.name.contains("标记")));
}
