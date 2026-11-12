use super::{
    environment::{Page, State, Task},
    tests::{fixture, grid},
    *,
};
use acs_core::environment::{self as env, command::Runner, Service};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Barrier,
    },
    time::Instant,
};

#[derive(Clone)]
struct Fake {
    npm: PathBuf,
    fail: bool,
    gate: Option<(Arc<AtomicBool>, Arc<Barrier>)>,
}
impl Runner for Fake {
    fn run(&self, program: &str, args: &[OsString], _: Option<&Path>) -> Result<Vec<u8>> {
        if let Some((once, barrier)) = &self.gate {
            if once.swap(false, Ordering::SeqCst) {
                barrier.wait();
                barrier.wait();
            }
        }
        if self.fail {
            bail!("example dependency unavailable; retry with R");
        }
        if args.first().is_some_and(|a| a == "--version") {
            return Ok(b"example 1.0.0".to_vec());
        }
        if program == "git" {
            bail!("example local source has no git metadata");
        }
        assert_eq!(program, "npm");
        assert_eq!(args, [OsString::from("root"), OsString::from("-g")]);
        Ok(self.npm.to_string_lossy().as_bytes().to_vec())
    }
}
fn put(path: &Path, value: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, value).unwrap();
}
fn runner(root: &Path) -> Fake {
    Fake {
        npm: root.join("npm-example"),
        fail: false,
        gate: None,
    }
}
pub(super) fn configure_fixture(app: &mut App, root: &Path) {
    let repo = root.join("example-repo");
    put(
        &repo.join("environment.yaml"),
        include_str!("../../../core/tests/fixtures/environment/environment.yaml"),
    );
    put(
        &repo.join("environment.lock"),
        include_str!("../../../core/tests/fixtures/environment/environment.lock"),
    );
    put(
        &repo.join("instructions/global.md"),
        "example global instructions\n",
    );
    put(
        &repo.join("skills/local/example-demo/SKILL.md"),
        "---\nname: example-demo\n---\nexample skill\n",
    );
    put(
        &repo.join("skills/vendor/.ags-snapshot.yaml"),
        "version: 1\nsources: {}\nskills: {}\n",
    );
    env::set_source(&app.environment.paths, repo.to_str().unwrap(), None).unwrap();
    env::configure(
        &app.environment.paths,
        Some("default"),
        Some(Agent::ALL.to_vec()),
    )
    .unwrap();
    let fake = runner(root);
    for (package, version) in [
        ("@openai/codex", "1.0.0"),
        ("@anthropic-ai/claude-code", "2.0.0"),
    ] {
        put(
            &fake.npm.join(package).join("package.json"),
            &format!(r#"{{"version":"{version}"}}"#),
        );
    }
    let plan = Service {
        paths: &app.environment.paths,
        runner: &fake,
    }
    .prepare()
    .unwrap();
    app.environment.config = Some(plan.config.clone());
    app.environment.plan = Some(Arc::new(plan));
    app.environment.state = State::Ready;
}
pub(super) fn set_state(app: &mut App, base: &App, kind: usize) {
    app.environment_open = true;
    app.environment.config = base.environment.config.clone();
    app.environment.plan = base.environment.plan.clone();
    app.environment.notice = "example 环境状态".into();
    app.environment.state = match kind {
        15 => {
            app.environment.plan = None;
            State::Unconfigured
        }
        16 => State::Loading,
        17 => {
            app.environment.plan = None;
            State::Ready
        }
        18 => State::Ready,
        19 => State::Confirm,
        20 => State::Applying,
        21 => {
            app.environment.notice = "应用成功；已重新读取最新状态".into();
            State::Ready
        }
        22 => {
            app.environment.plan = None;
            app.environment.notice = "example 失败原因；R 重试".into();
            State::Failed
        }
        23 => {
            app.environment.plan = None;
            app.environment.report = Some(env::DoctorReport {
                checks: vec![env::Check {
                    name: "example 依赖".into(),
                    ok: false,
                    detail: "example 未安装".into(),
                    fix: "安装 example 后按 R".into(),
                }],
            });
            State::Diagnosis
        }
        24 => {
            app.help = true;
            State::Ready
        }
        _ => unreachable!(),
    };
}
fn finish(page: &mut Page) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while page.busy() {
        page.poll();
        assert!(Instant::now() < deadline, "worker did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn press(app: &mut App, c: KeyCode) -> Effect {
    app.key(KeyEvent::new(c, KeyModifiers::NONE))
}

#[test]
fn every_environment_state_renders_wide_80_and_narrow_with_full_scrollable_content() {
    let (dir, mut base) = fixture();
    configure_fixture(&mut base, dir.path());
    let expected = [
        "尚未配置环境",
        "加载中",
        "无漂移",
        "有漂移",
        "确认应用",
        "应用中",
        "应用成功",
        "操作失败",
        "诊断结果",
        "环境帮助",
    ];
    for (kind, title) in (15..=24).zip(expected) {
        for (width, height) in [(120, 32), (80, 24), (40, 20)] {
            let mut app = App::new(
                base.session.store.clone(),
                Agent::Codex,
                base.environment.paths.clone(),
            )
            .unwrap();
            set_state(&mut app, &base, kind);
            let g = grid(&app, width, height);
            assert!(g.contains(title), "{kind}/{width}:\n{g}");
            assert!(g.contains("环境"));
        }
    }
    base.environment_open = true;
    let plan = Arc::get_mut(base.environment.plan.as_mut().unwrap()).unwrap();
    plan.changes.extend((0..250).map(|i| {
        format!(
            "Codex Skill example-{i:03}：更新；完整长路径 /example/中文目录/important-tail-{i:03}"
        )
    }));
    for width in [120, 80, 40] {
        base.environment.scroll = 0;
        let mut seen = String::new();
        for _ in 0..2000 {
            seen.push_str(&grid(&base, width, 20));
            if base.environment.scroll >= base.environment.scroll_limit.get() {
                break;
            }
            press(&mut base, KeyCode::PageDown);
        }
        assert!(
            seen.chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect::<String>()
                .contains("important-tail-249"),
            "long tail unreachable at {width}"
        );
        press(&mut base, KeyCode::End);
        let last = grid(&base, width, 20);
        assert!(last.contains("249"));
        press(&mut base, KeyCode::Up);
        assert!(base.environment.scroll < base.environment.scroll_limit.get());
        press(&mut base, KeyCode::Home);
        assert_eq!(base.environment.scroll, 0);
    }
    let text = "example/中文/🙂/very-long-path-with-important-tail";
    assert_eq!(view::environment_lines(vec![text.into()], 9).concat(), text);
}

#[test]
fn unconfigured_and_failed_tasks_keep_the_tui_and_show_actionable_commands() {
    let (dir, mut app) = fixture();
    app.environment_open = true;
    app.environment.start(Task::Refresh, runner(dir.path()));
    finish(&mut app.environment);
    assert_eq!(app.environment.state, State::Unconfigured);
    let text = app.environment.lines().join("\n");
    assert!(text.contains("acs env source set") && text.contains("acs env configure"));
    configure_fixture(&mut app, dir.path());
    let mut fake = runner(dir.path());
    fake.fail = true;
    app.environment.start(Task::Refresh, fake);
    finish(&mut app.environment);
    assert_eq!(app.environment.state, State::Failed);
    assert!(app
        .environment
        .notice
        .contains("example dependency unavailable"));
    assert!(app.environment.config.is_some());
    assert!(app.environment_open && app.environment.plan.is_none());
    assert_eq!(press(&mut app, KeyCode::Char('R')), Effect::EnvRefresh);
    assert_eq!(press(&mut app, KeyCode::Esc), Effect::EnvBack);
}

#[test]
fn confirmation_uses_the_displayed_plan_and_stale_source_is_refreshed_without_writing() {
    let (dir, mut app) = fixture();
    configure_fixture(&mut app, dir.path());
    app.environment_open = true;
    let before = app.session.store.snapshot().unwrap();
    let displayed = app.environment.plan.as_ref().unwrap().clone();
    press(&mut app, KeyCode::Char('a'));
    assert_eq!(app.environment.state, State::Confirm);
    assert!(Arc::ptr_eq(
        app.environment.plan.as_ref().unwrap(),
        &displayed
    ));
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.environment.state, State::Ready);
    assert_eq!(before, app.session.store.snapshot().unwrap());
    put(
        &dir.path().join("example-repo/instructions/global.md"),
        "example changed after preview\n",
    );
    press(&mut app, KeyCode::Char('a'));
    assert_eq!(press(&mut app, KeyCode::Char('y')), Effect::EnvApply);
    app.environment
        .start(Task::Apply(displayed.clone()), runner(dir.path()));
    finish(&mut app.environment);
    assert!(app.environment.notice.contains("未应用旧计划"));
    assert!(!Arc::ptr_eq(
        app.environment.plan.as_ref().unwrap(),
        &displayed
    ));
    assert_eq!(before, app.session.store.snapshot().unwrap());
    assert!(!app.environment.paths.codex_skills.exists());
    assert_eq!(app.environment.state, State::Ready);
    press(&mut app, KeyCode::Char('a'));
    assert_eq!(press(&mut app, KeyCode::Char('y')), Effect::EnvApply);
    let plan = app.environment.plan.take().unwrap();
    app.environment.start(Task::Apply(plan), runner(dir.path()));
    finish(&mut app.environment);
    assert!(app.environment.notice.contains("应用成功"));
    assert!(!app.environment.plan.as_ref().unwrap().has_changes());
    assert!(grid(&app, 80, 24).contains("无漂移"));
    assert_eq!(
        fs::read_to_string(app.environment.paths.roots.codex.join("AGENTS.md")).unwrap(),
        "example changed after preview\n"
    );
    app.environment.start(Task::Doctor, runner(dir.path()));
    finish(&mut app.environment);
    assert_eq!(app.environment.state, State::Diagnosis);
    assert!(app.environment.report.as_ref().unwrap().healthy());
}

#[test]
fn blocked_dependency_does_not_block_keys_rendering_or_progress_delivery() {
    let (dir, mut app) = fixture();
    configure_fixture(&mut app, dir.path());
    app.environment_open = true;
    let barrier = Arc::new(Barrier::new(2));
    let mut fake = runner(dir.path());
    fake.gate = Some((Arc::new(AtomicBool::new(true)), barrier.clone()));
    let plan = app.environment.plan.take().unwrap();
    app.environment.start(Task::Apply(plan), fake);
    barrier.wait();
    app.environment.poll();
    assert!(app.environment.progress.is_some());
    assert!(app.environment.busy());
    assert!(app.environment.plan.is_none());
    assert!(app.environment.lines().join("\n").contains("example-demo"));
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char('?'));
    assert!(grid(&app, 80, 24).contains("环境帮助"));
    press(&mut app, KeyCode::Esc);
    assert!(grid(&app, 80, 24).contains("已等待"));
    press(&mut app, KeyCode::Tab);
    assert!(app.environment_open);
    let effect = app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert_eq!(effect, Effect::None);
    assert!(app.quit_after_environment);
    barrier.wait();
    finish(&mut app.environment);
    assert_eq!(app.environment.state, State::Ready);
    assert!(!app.environment.plan.as_ref().unwrap().has_changes());
}

#[test]
fn failed_apply_retains_environment_page_and_cannot_reuse_failed_plan() {
    let (dir, mut app) = fixture();
    configure_fixture(&mut app, dir.path());
    app.environment_open = true;
    let before = app.session.store.snapshot().unwrap();
    press(&mut app, KeyCode::Char('a'));
    assert_eq!(press(&mut app, KeyCode::Char('y')), Effect::EnvApply);
    let plan = app.environment.plan.take().unwrap();
    let mut fake = runner(dir.path());
    fake.fail = true;
    app.environment.start(Task::Apply(plan), fake);
    finish(&mut app.environment);
    assert_eq!(app.environment.state, State::Failed);
    assert!(app.environment_open && app.environment.plan.is_none());
    assert_eq!(before, app.session.store.snapshot().unwrap());
    assert!(!app.environment.paths.codex_skills.exists());
    assert_eq!(press(&mut app, KeyCode::Char('a')), Effect::None);
    assert_eq!(press(&mut app, KeyCode::Char('y')), Effect::None);
    assert_eq!(app.environment.state, State::Failed);
    assert!(grid(&app, 80, 24).contains("操作失败"));
}
