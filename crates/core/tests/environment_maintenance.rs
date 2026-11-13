use acs_core::{
    environment::{command::Runner, *},
    Agent,
};
use anyhow::{bail, Result};
use std::{
    cell::RefCell,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};
struct Fake {
    calls: RefCell<Vec<Vec<String>>>,
    fail: bool,
}
impl Runner for Fake {
    fn run(&self, program: &str, args: &[OsString], _: Option<&Path>) -> Result<Vec<u8>> {
        let a: Vec<_> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        self.calls
            .borrow_mut()
            .push(std::iter::once(program.into()).chain(a.clone()).collect());
        if self.fail {
            bail!("example interrupted command");
        }
        match program {
            "npm" => Ok(if a[1].contains("openai") {
                b"3.0.0\n".to_vec()
            } else {
                b"4.0.0\n".to_vec()
            }),
            "git" => Ok(b"0123456789abcdef0123456789abcdef01234567\tHEAD\n".to_vec()),
            _ => Ok(vec![]),
        }
    }
}
fn put(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let repo = d.path().join("repo");
    put(
        &repo.join("environment.yaml"),
        include_str!("fixtures/environment/environment.yaml"),
    );
    put(
        &repo.join("environment.lock"),
        include_str!("fixtures/environment/environment.lock"),
    );
    put(&repo.join("instructions/global.md"), "example rules\n");
    put(
        &repo.join("skills/local/example-demo/SKILL.md"),
        "example local",
    );
    fs::create_dir_all(repo.join("skills/vendor")).unwrap();
    (d, repo)
}
#[test]
fn lock_resolves_shared_source_once_and_failure_preserves_original() {
    let (_d, repo) = fixture();
    let m=fs::read_to_string(repo.join("environment.yaml")).unwrap().replace("vendor: skills/vendor","vendor: skills/vendor, vendor_include: [one]")+"\nsources:\n  one: {type: git, url: 'https://example.com/shared.git', lock: shared, discover: {mode: single}}\n  two: {type: git, url: 'https://example.com/shared.git', lock: shared, discover: {mode: collection}}\n";
    put(&repo.join("environment.yaml"), &m);
    put(
        &repo.join("environment.lock"),
        &include_str!("fixtures/environment/environment.lock").replace(
            "sources: {}",
            "sources: {shared: {commit: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa}}",
        ),
    );
    let old = fs::read(repo.join("environment.lock")).unwrap();
    assert!(update_lock(
        &repo,
        &Fake {
            calls: RefCell::new(vec![]),
            fail: true
        }
    )
    .is_err());
    assert_eq!(old, fs::read(repo.join("environment.lock")).unwrap());
    let f = Fake {
        calls: RefCell::new(vec![]),
        fail: false,
    };
    let lock = update_lock(&repo, &f).unwrap();
    assert_eq!(lock.agents[&Agent::Codex].version, "3.0.0");
    assert_eq!(lock.agents[&Agent::Claude].version, "4.0.0");
    assert_eq!(f.calls.borrow().iter().filter(|a| a[0] == "git").count(), 1);
    assert_eq!(
        load_yaml::<Lock>(&repo.join("environment.lock")).unwrap(),
        lock
    );
}
#[test]
fn vendor_builds_patches_discovers_and_repeats_byte_identically() {
    let (d, repo) = fixture();
    let m=fs::read_to_string(repo.join("environment.yaml")).unwrap().replace("vendor: skills/vendor","vendor: skills/vendor, vendor_include: ['one:*']")+"\nsources:\n  one:\n    type: local\n    path: upstream\n    discover: {mode: collection, path: skills}\n    build:\n      template: skills/demo/SKILL.src.md\n      output: skills/demo/SKILL.md\n      replacements: {TOKEN: example-vendored}\n      commands: {all: ['echo example-common'], windows: ['echo example-windows'], linux: ['echo example-linux']}\n    patches: [patches/example.yaml]\n";
    put(&repo.join("environment.yaml"), &m);
    put(
        &repo.join("upstream/skills/demo/SKILL.src.md"),
        "---\nname: '{{TOKEN}}'\n---\ntext old\nremove this\nbegin\ninside\nend\nkeep\n",
    );
    put(&repo.join("patches/example.yaml"),"source: one\npatches:\n- {file: skills/demo/SKILL.md, op: replace_line, match: '^text', replace: 'text new'}\n- {file: skills/demo/SKILL.md, op: delete_line, match: '^remove'}\n- {file: skills/demo/SKILL.md, op: delete_block, start: '^begin', end: '^end'}\n- {file: missing.md, op: delete_line, match: example}\n");
    let f = Fake {
        calls: RefCell::new(vec![]),
        fail: false,
    };
    let p = Paths::under(d.path());
    let mut progress = vec![];
    let snapshot = vendor(&p, &repo, &f, |p| progress.push((p.completed, p.total))).unwrap();
    assert!(snapshot.skills.contains_key("example-vendored"));
    let skill = repo.join("skills/vendor/example-vendored/SKILL.md");
    assert_eq!(
        fs::read_to_string(&skill).unwrap(),
        "---\nname: 'example-vendored'\n---\ntext new\nkeep\n"
    );
    let first = fs::read(repo.join("skills/vendor/.vendor-snapshot.yaml")).unwrap();
    vendor(&p, &repo, &f, |_| {}).unwrap();
    assert_eq!(
        first,
        fs::read(repo.join("skills/vendor/.vendor-snapshot.yaml")).unwrap()
    );
    validate(&repo).unwrap();
    assert!(progress.iter().any(|(n, total)| n == total && *n > 0));
    assert!(f
        .calls
        .borrow()
        .iter()
        .any(|a| a.iter().any(|s| s.contains("example-common"))));
}
#[test]
fn vendor_failure_does_not_replace_previous_snapshot() {
    let (d, repo) = fixture();
    let m = fs::read_to_string(repo.join("environment.yaml"))
        .unwrap()
        .replace(
            "vendor: skills/vendor",
            "vendor: skills/vendor, vendor_include: ['one:*']",
        )
        + "\nsources:\n  one: {type: local, path: missing-source, discover: {mode: flat}}\n";
    put(&repo.join("environment.yaml"), &m);
    put(&repo.join("skills/vendor/example-old/SKILL.md"), "old");
    assert!(vendor(
        &Paths::under(d.path()),
        &repo,
        &Fake {
            calls: RefCell::new(vec![]),
            fail: false
        },
        |_| {}
    )
    .is_err());
    assert_eq!(
        fs::read(repo.join("skills/vendor/example-old/SKILL.md")).unwrap(),
        b"old"
    );
}
#[test]
fn vendor_fetches_one_locked_git_commit_for_shared_sources() {
    struct Git {
        home: PathBuf,
        calls: RefCell<Vec<Vec<String>>>,
    }
    impl Runner for Git {
        fn run(&self, p: &str, a: &[OsString], dir: Option<&Path>) -> Result<Vec<u8>> {
            assert_eq!(p, "git");
            self.calls
                .borrow_mut()
                .push(a.iter().map(|s| s.to_string_lossy().into_owned()).collect());
            let mut c = std::process::Command::new(p);
            c.args(a)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.home.join("example-gitconfig"))
                .env("HOME", &self.home);
            if let Some(d) = dir {
                c.current_dir(d);
            }
            let o = c.output()?;
            if !o.status.success() {
                bail!("{}", String::from_utf8_lossy(&o.stderr));
            }
            Ok(o.stdout)
        }
    }
    if std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let (d, repo) = fixture();
    let upstream = d.path().join("upstream");
    put(
        &upstream.join("first/SKILL.md"),
        "---\nname: example-first\n---\nold locked\n",
    );
    put(
        &upstream.join("second/SKILL.md"),
        "---\nname: example-second\n---\n",
    );
    let git = Git {
        home: d.path().into(),
        calls: RefCell::new(vec![]),
    };
    put(
        &d.path().join("example-gitconfig"),
        "[core]\nsymlinks = false\nautocrlf = true\n",
    );
    let command = |args: &[&str]| {
        git.run(
            "git",
            &args.iter().map(OsString::from).collect::<Vec<_>>(),
            Some(&upstream),
        )
        .unwrap()
    };
    command(&["init", "--initial-branch=main"]);
    command(&["add", "."]);
    put(&upstream.join("example-link-target"), "SKILL.md");
    let blob = String::from_utf8(command(&["hash-object", "-w", "example-link-target"])).unwrap();
    command(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("120000,{},first/CLAUDE.md", blob.trim()),
    ]);
    command(&[
        "-c",
        "user.name=Example",
        "-c",
        "user.email=example@example.com",
        "commit",
        "-m",
        "example old",
    ]);
    let commit = String::from_utf8(command(&["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned();
    put(
        &upstream.join("first/SKILL.md"),
        "---\nname: example-first\n---\nnew unlocked\n",
    );
    command(&["add", "."]);
    command(&[
        "-c",
        "user.name=Example",
        "-c",
        "user.email=example@example.com",
        "commit",
        "-m",
        "example new",
    ]);
    let url = url::Url::from_directory_path(&upstream)
        .unwrap()
        .to_string();
    let manifest=fs::read_to_string(repo.join("environment.yaml")).unwrap().replace("vendor: skills/vendor","vendor: skills/vendor, vendor_include: [one, two]")+&format!("\nsources:\n  one: {{type: git, url: '{url}', lock: shared, discover: {{mode: single, path: first}}}}\n  two: {{type: git, url: '{url}', lock: shared, discover: {{mode: single, path: second}}}}\n");
    put(&repo.join("environment.yaml"), &manifest);
    put(
        &repo.join("environment.lock"),
        &include_str!("fixtures/environment/environment.lock").replace(
            "sources: {}",
            &format!("sources: {{shared: {{commit: '{commit}'}}}}"),
        ),
    );
    git.calls.borrow_mut().clear();
    vendor(&Paths::under(d.path()), &repo, &git, |_| {}).unwrap();
    assert!(
        fs::read_to_string(repo.join("skills/vendor/example-first/SKILL.md"))
            .unwrap()
            .contains("old locked")
    );
    assert_eq!(
        git.calls
            .borrow()
            .iter()
            .filter(|a| a.first().is_some_and(|s| s == "fetch"))
            .count(),
        1
    );
    validate(&repo).unwrap();
    let link = repo.join("skills/vendor/example-first/CLAUDE.md");
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), PathBuf::from("SKILL.md"));
    let before = fs::read(repo.join("skills/vendor").join(VENDOR_SNAPSHOT)).unwrap();
    vendor(&Paths::under(d.path()), &repo, &git, |_| {}).unwrap();
    assert_eq!(
        fs::read(repo.join("skills/vendor").join(VENDOR_SNAPSHOT)).unwrap(),
        before
    );
    assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
}
