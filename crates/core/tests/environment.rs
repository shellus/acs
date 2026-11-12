use acs_core::{environment::*, storage, Agent};
use std::path::Path;
const MANIFEST: &str = include_str!("fixtures/environment/environment.yaml");
const LOCK: &str = include_str!("fixtures/environment/environment.lock");
fn put(root: &Path, file: &str, data: &str) {
    storage::write(&root.join(file), Some(data.as_bytes())).unwrap();
}
fn fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    put(r, "environment.yaml", MANIFEST);
    put(r, "environment.lock", LOCK);
    put(r, "instructions/global.md", "example rules");
    put(
        r,
        "skills/local/example-demo/SKILL.md",
        "---\nname: example-demo\ndescription: example\n---\n",
    );
    put(
        r,
        "skills/vendor/.ags-snapshot.yaml",
        "version: 1\nsources: {}\nskills: {}\n",
    );
    d
}
#[test]
fn repository_and_agent_specific_selection() {
    let d = fixture();
    let r = Repository::load(d.path()).unwrap();
    let s = r.selection("default", Agent::Codex).unwrap();
    assert_eq!(s.include, ["*", "local:example-*"]);
    assert_eq!(s.exclude, ["example-hidden", "example-other"]);
    assert_eq!(s.disabled_skills, ["example-disabled"]);
    assert!(r.selection("missing", Agent::Codex).is_err());
    validate(d.path()).unwrap();
}
#[test]
fn strict_fields_agents_and_model_validation() {
    let d = fixture();
    for bad in [
        MANIFEST.replace("version: 2", "version: 3"),
        MANIFEST.replace("global.md", "../../../../outside"),
        MANIFEST.replace("claude: {}", "claude: {disabled_skills: [example]}"),
        MANIFEST.replace("example-disabled]", "../escape]"),
        MANIFEST.replace("example-disabled]", "example, example]"),
        MANIFEST.replace("claude: {}", "example-agent: {}"),
        MANIFEST.replace("claude: {}", "claude: {typo: true}"),
        MANIFEST.replace("vendor: skills/vendor", "vendor: skills/vendor, typo: true"),
        MANIFEST.replace("version: 2", "unknown: true\nversion: 2"),
        MANIFEST.replace("package: '@openai/codex'", "package: ''"),
        MANIFEST.replace("include: ['*']", "include: []"),
    ] {
        put(d.path(), "environment.yaml", &bad);
        assert!(Repository::load(d.path()).is_err(), "accepted {bad}");
    }
    put(d.path(), "environment.yaml", MANIFEST);
    for lock in [
        LOCK.replace("version: 2", "version: 1"),
        LOCK.replace("'1.0.0'", "''"),
        LOCK.replace("sources: {}", "sources: {}\ntypo: true"),
        LOCK.replace("claude:", "example-agent:"),
    ] {
        put(d.path(), "environment.lock", &lock);
        assert!(Repository::load(d.path()).is_err());
    }
}
#[test]
fn sources_build_and_patch_declarations() {
    let d = fixture();
    put(d.path(),"patches/example.yaml","source: example\npatches:\n- {id: example, file: SKILL.md, op: replace_line, match: '^example', replace: replacement}\n");
    let source="sources:\n  example:\n    type: git\n    url: https://example.com/skills.git\n    lock: example-lock\n    discover: {mode: collection, path: skills}\n    build: {template: in.md, output: SKILL.md, replacements: {EXAMPLE: example}, commands: {all: ['echo example'], windows: ['echo windows']}}\n    patches: [patches/example.yaml]\n";
    let good = MANIFEST.replace(
        "vendor: skills/vendor",
        "vendor: skills/vendor, vendor_include: ['example:*']",
    ) + source;
    put(d.path(), "environment.yaml", &good);
    put(
        d.path(),
        "environment.lock",
        &LOCK.replace(
            "sources: {}",
            "sources: {example-lock: {commit: example-commit}}",
        ),
    );
    Repository::load(d.path()).unwrap();
    for bad in [
        good.replace("type: git", "type: other"),
        good.replace("url: https://example.com/skills.git", "url: ''"),
        good.replace("mode: collection", "mode: recursive"),
        good.replace("template: in.md, ", ""),
        good.replace("output: SKILL.md", "output: ../outside"),
        good.replace("windows:", "unsupported:"),
        good.replace("patches/example.yaml", "../outside.yaml"),
    ] {
        put(d.path(), "environment.yaml", &bad);
        assert!(Repository::load(d.path()).is_err(), "{bad}");
    }
    put(d.path(), "environment.yaml", &good);
    for p in [
        "source: other\npatches: []",
        "source: example\nunknown: true",
        "source: example\npatches: [{file: ../outside, op: delete_line}]",
        "source: example\npatches: [{file: example, op: example}]",
        "source: example\npatches: [{file: example, op: delete_line, match: '['}]",
    ] {
        put(d.path(), "patches/example.yaml", p);
        assert!(Repository::load(d.path()).is_err());
    }
}
#[test]
fn local_configuration_roundtrip_defaults_and_rejections() {
    let d = fixture();
    let p = Paths::under(d.path());
    let c = LocalConfig::load(&p.config()).unwrap();
    assert_eq!(c.environment.branch, "main");
    assert!(!p.config().exists());
    let c = set_source(&p, d.path().to_str().unwrap(), None).unwrap();
    assert_eq!(c.environment.profile, "default");
    let c = configure(&p, Some("default"), Some(vec![Agent::Codex, Agent::Claude])).unwrap();
    assert_eq!(
        LocalConfig::load(&p.config()).unwrap().environment,
        c.environment
    );
    let before = std::fs::read(p.config()).unwrap();
    assert!(configure(&p, Some("unknown"), None).is_err());
    assert_eq!(std::fs::read(p.config()).unwrap(), before);
    for s in [
        "version: 9",
        "version: 1\nenvironment: {branch: ''}",
        "version: 1\nenvironment: {agents: [codex, codex]}",
        "version: 1\nenvironment: {agents: [example-agent]}",
        "version: 1\nenvironment: {unknown: true}",
        "version: 1\nunknown: true",
    ] {
        put(d.path(), "acs/environment.yaml", s);
        assert!(LocalConfig::load(&p.config()).is_err(), "{s}");
    }
}
#[test]
fn paths_are_injected_and_root_is_fully_isolated() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path();
    let p = Paths::resolve(home, |_| None).unwrap();
    assert_eq!(p.config(), home.join(".acs/environment.yaml"));
    assert_eq!(p.state(), home.join(".acs/state/environment.json"));
    assert_eq!(p.cache(), home.join(".acs/cache"));
    assert_eq!(
        p.destinations(Agent::Codex),
        (home.join(".codex/AGENTS.md"), home.join(".agents/skills"))
    );
    let p = Paths::resolve(home, |k| match k {
        "ACS_HOME" => Some(home.join("example-acs")),
        "CODEX_HOME" => Some(home.join("example-codex")),
        "CLAUDE_CONFIG_DIR" => Some("~/.config/claude".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(
        p.destinations(Agent::Claude).0,
        home.join(".config/claude/CLAUDE.md")
    );
    assert_eq!(p.roots.codex, home.join("example-codex"));
    assert_eq!(p.codex_skills, home.join(".agents/skills"));
    let p = Paths::under(home);
    for path in [
        p.config(),
        p.state(),
        p.cache(),
        p.destinations(Agent::Codex).0,
        p.destinations(Agent::Codex).1,
        p.destinations(Agent::Claude).0,
        p.destinations(Agent::Claude).1,
    ] {
        assert!(path.starts_with(home));
    }
    assert!(Paths::resolve(Path::new(""), |_| None).is_err());
    assert!(!p.config().exists());
}
#[test]
fn include_globs_exclusions_dedup_conflicts_and_no_match() {
    let units = vec![
        SkillUnit {
            source: "local".into(),
            name: "example-a".into(),
            path: "example".into(),
        },
        SkillUnit {
            source: "vendor".into(),
            name: "example-b".into(),
            path: "example".into(),
        },
    ];
    let strings = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        select_units(
            &strings(&["*", "local:example-?"]),
            &strings(&["example-b"]),
            &units
        )
        .unwrap(),
        vec![units[0].clone()]
    );
    assert_eq!(
        select_units(&strings(&["vendor"]), &[], &units).unwrap(),
        vec![units[1].clone()]
    );
    assert!(select_units(&strings(&["missing"]), &[], &units).is_err());
    let mut conflict = units.clone();
    conflict[1].name = conflict[0].name.clone();
    assert!(select_units(&strings(&["*"]), &[], &conflict).is_err());
}
#[test]
fn published_hashes_and_tampering() {
    let d = fixture();
    put(
        d.path(),
        "skills/vendor/example-upstream/SKILL.md",
        "example\r\n",
    );
    let skill = d.path().join("skills/vendor/example-upstream");
    let h = tree_hash(&skill).unwrap();
    put(
        d.path(),
        "skills/vendor/example-upstream/SKILL.md",
        "example\n",
    );
    assert_eq!(tree_hash(&skill).unwrap(), h);
    put(
        d.path(),
        "skills/vendor/.ags-snapshot.yaml",
        &format!("version: 1\nskills:\n  example-upstream: {{source: example, hash: '{h}'}}\n"),
    );
    validate(d.path()).unwrap();
    put(
        d.path(),
        "skills/vendor/example-upstream/SKILL.md",
        "changed",
    );
    assert!(validate(d.path()).is_err());
}
#[test]
fn state_and_marker_roundtrip_versions_and_unsafe_names() {
    let d = tempfile::tempdir().unwrap();
    let p = Paths::under(d.path());
    let s = State::load(&p.state()).unwrap();
    storage::write(&p.state(), Some(&s.encode().unwrap())).unwrap();
    assert_eq!(State::load(&p.state()).unwrap().version, 1);
    let m = ManagedMarker::empty();
    storage::write(&d.path().join(MANAGED_MARKER), Some(&m.encode().unwrap())).unwrap();
    assert!(ManagedMarker::load(d.path()).unwrap().skills.is_empty());
    for b in [
        br#"{"version":9,"skills":{}}"#.as_slice(),
        br#"{"version":1,"skills":{"../outside":"example"}}"#,
        br#"{"version":1,"skills":{},"unknown":true}"#,
    ] {
        storage::write(&d.path().join(MANAGED_MARKER), Some(b)).unwrap();
        assert!(ManagedMarker::load(d.path()).is_err());
    }
}
#[test]
fn portable_relative_paths() {
    for p in [
        "../outside",
        "C:/outside",
        "C:\\outside",
        "//example/share",
        "/outside",
        "a/../../outside",
    ] {
        assert!(relative_path(p, false).is_err(), "{p}");
    }
    for p in ["example/skill", "./example", "a/../example"] {
        relative_path(p, false).unwrap();
    }
}
#[test]
fn yaml_duplicate_keys_and_multiple_documents_are_rejected() {
    for input in ["example: 1\nexample: 2", "example: 1\n---\nexample: 2"] {
        assert!(decode_yaml::<std::collections::BTreeMap<String, u32>>(input.as_bytes()).is_err());
    }
}
