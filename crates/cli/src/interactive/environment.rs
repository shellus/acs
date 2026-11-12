//! Environment page state and background work; selection and validation stay in core.
use super::*;
use acs_core::environment::{
    self as env, command::Runner, DoctorReport, LocalConfig, Paths, Plan, Progress, Service,
};
use std::{
    sync::{mpsc, Arc},
    time::Instant,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum State {
    Unconfigured,
    Loading,
    Ready,
    Confirm,
    Applying,
    Failed,
    Diagnosis,
}
pub(super) struct Page {
    pub paths: Paths,
    pub config: Option<LocalConfig>,
    pub plan: Option<Arc<Plan>>,
    pub report: Option<DoctorReport>,
    pub state: State,
    pub notice: String,
    pub progress: Option<Progress>,
    pub scroll: usize,
    pub started: Instant,
    pub scroll_limit: std::cell::Cell<usize>,
    busy_preview: Vec<String>,
    worker: Option<mpsc::Receiver<Message>>,
    worker_thread: Option<std::thread::JoinHandle<()>>,
}
pub(super) enum Task {
    Refresh,
    Apply(Arc<Plan>),
    Doctor,
}
enum Outcome {
    Prepared(LocalConfig, Option<Arc<Plan>>, String),
    Diagnosed(DoctorReport),
}
enum Message {
    Configured(LocalConfig),
    Progress(Progress),
    Finished(Result<Outcome>),
}
impl Page {
    pub fn new(paths: Paths) -> Self {
        Self {
            paths,
            config: None,
            plan: None,
            report: None,
            state: State::Unconfigured,
            notice: String::new(),
            progress: None,
            scroll: 0,
            started: Instant::now(),
            worker: None,
            worker_thread: None,
            scroll_limit: std::cell::Cell::new(0),
            busy_preview: vec![],
        }
    }
    pub fn busy(&self) -> bool {
        matches!(self.state, State::Loading | State::Applying)
    }
    pub fn start(&mut self, task: Task, runner: impl Runner + Send + 'static) {
        if self.worker.is_some() {
            return;
        }
        self.state = if matches!(task, Task::Apply(_)) {
            State::Applying
        } else {
            State::Loading
        };
        self.notice = match &task {
            Task::Refresh => "正在读取环境；来源同步可能需要联网",
            Task::Apply(_) => "正在应用已确认的计划；请等待事务完成",
            Task::Doctor => "正在诊断依赖、来源与本机状态",
        }
        .into();
        self.started = Instant::now();
        self.progress = None;
        self.scroll = 0;
        self.busy_preview = match &task {
            Task::Apply(plan) => plan_lines(plan),
            _ => vec![],
        };
        let previous_plan = self.plan.take();
        self.report = None;
        let (tx, rx) = mpsc::channel();
        self.worker = Some(rx);
        let paths = self.paths.clone();
        self.worker_thread = match std::thread::Builder::new()
            .name("acs-environment".into())
            .spawn(move || {
                // Plan owns a temporary Skills tree; filesystem cleanup belongs off the UI thread too.
                drop(previous_plan);
                if let Ok(config) = LocalConfig::load(&paths.config()) {
                    let _ = tx.send(Message::Configured(config));
                }
                let result = work(&paths, task, &runner, |p| {
                    let _ = tx.send(Message::Progress(p));
                });
                let _ = tx.send(Message::Finished(result));
            }) {
            Ok(handle) => Some(handle),
            Err(e) => {
                self.worker = None;
                self.state = State::Failed;
                self.plan = None;
                self.notice = format!("无法启动环境后台任务：{e}");
                None
            }
        };
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Some(rx) = &self.worker {
            let message = match rx.try_recv() {
                Ok(m) => m,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.worker = None;
                    if let Some(handle) = self.worker_thread.take() {
                        let _ = handle.join();
                    }
                    self.plan = None;
                    self.progress = None;
                    self.state = State::Failed;
                    self.notice = "后台任务意外结束；按 R 重新读取".into();
                    return true;
                }
            };
            changed = true;
            match message {
                Message::Configured(config) => self.config = Some(config),
                Message::Progress(p) => self.progress = Some(p),
                Message::Finished(result) => {
                    self.worker = None;
                    if let Some(handle) = self.worker_thread.take() {
                        let _ = handle.join();
                    }
                    self.progress = None;
                    match result {
                        Ok(Outcome::Prepared(config, plan, notice)) => {
                            self.config = Some(config);
                            self.state = if plan.is_some() {
                                State::Ready
                            } else {
                                State::Unconfigured
                            };
                            self.plan = plan;
                            self.notice = notice;
                        }
                        Ok(Outcome::Diagnosed(report)) => {
                            self.notice = if report.healthy() {
                                "诊断通过"
                            } else {
                                "诊断发现需处理的项目；修复提示见下方"
                            }
                            .into();
                            self.report = Some(report);
                            self.state = State::Diagnosis;
                        }
                        Err(e) => {
                            self.plan = None;
                            self.state = State::Failed;
                            self.notice = format!("{e:#}");
                        }
                    }
                    break;
                }
            }
        }
        changed
    }
    pub fn title(&self) -> &'static str {
        match self.state {
            State::Unconfigured => "尚未配置环境",
            State::Loading => "加载中",
            State::Ready if self.plan.as_ref().is_some_and(|p| p.has_changes()) => {
                "有漂移 · 请查看变更"
            }
            State::Ready => "无漂移",
            State::Confirm => "确认应用当前计划",
            State::Applying => "应用中",
            State::Failed => "操作失败 · R 重试",
            State::Diagnosis => "诊断结果",
        }
    }
    pub fn key(&mut self, k: KeyEvent) -> Effect {
        self.scroll = self.scroll.min(self.scroll_limit.get());
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll = self.scroll.saturating_add(1),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(10),
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = usize::MAX,
            KeyCode::Char('?') => return Effect::None,
            _ if self.busy() => {
                self.notice =
                    "任务仍在进行；可滚动查看，完成后可返回或退出。应用事务不能中断。".into();
            }
            KeyCode::Char('y') if self.state == State::Confirm => return Effect::EnvApply,
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q')
                if self.state == State::Confirm =>
            {
                self.state = State::Ready;
                self.notice = "已取消应用；文件未写入".into();
            }
            _ if self.state == State::Confirm => {}
            KeyCode::Char('a') if self.plan.as_ref().is_some_and(|p| p.has_changes()) => {
                self.state = State::Confirm;
                self.notice =
                    "将应用下方这份计划（可能运行 npm 全局安装）；y 确认，n/Esc 取消".into();
            }
            KeyCode::Char('R') | KeyCode::Char('r') => return Effect::EnvRefresh,
            KeyCode::Char('d') => return Effect::EnvDoctor,
            KeyCode::Esc | KeyCode::Char('q') => return Effect::EnvBack,
            _ => {}
        }
        Effect::None
    }
    /// Full logical lines, wrapped by the view; no shortened paths or change details.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![];
        if let Some(c) = &self.config {
            let e = &c.environment;
            lines.extend([
                format!("来源：{}", e.source),
                format!("分支：{}", e.branch),
                format!("Profile：{}", e.profile),
                format!(
                    "启用 Agent：{}",
                    e.agents
                        .iter()
                        .map(|a| a.label())
                        .collect::<Vec<_>>()
                        .join("、")
                ),
            ]);
        }
        lines.push(self.notice.clone());
        if self.state == State::Unconfigured {
            lines.extend([
                "设置来源与 Profile 后按 R 刷新：".into(),
                "acs env source set <仓库路径或 URL> --branch main".into(),
                "acs env configure --profile <名称> --agents all".into(),
                "隔离运行时请在 acs 后继续加上本次的 --root 参数。".into(),
            ]);
        }
        if let Some(plan) = &self.plan {
            lines.extend(plan_lines(plan));
        } else if self.busy() {
            lines.extend(self.busy_preview.clone());
        }
        if let Some(report) = &self.report {
            for c in &report.checks {
                lines.push(format!(
                    "{} {}：{}",
                    if c.ok { "通过" } else { "失败" },
                    c.name,
                    c.detail
                ));
                if !c.fix.is_empty() {
                    lines.push(format!("修复：{}", c.fix));
                }
            }
        }
        lines
    }
}
fn plan_lines(plan: &Plan) -> Vec<String> {
    let mut lines = vec![String::new(), "Agent 安装版本与要求".into()];
    for (p, installed) in &plan.packages {
        lines.push(format!(
            "{} · 当前版本 {} · 要求 {} · {}",
            p.agent.label(),
            if installed.is_empty() {
                "未安装"
            } else {
                installed
            },
            p.requirement(),
            if p.satisfied(installed) {
                "满足"
            } else {
                "需要安装或调整版本"
            }
        ));
    }
    lines.push(String::new());
    lines.push(format!("变更项（{}）", plan.changes.len()));
    if !plan.has_changes() {
        lines.push("环境已同步，无需应用。".into());
    }
    for change in &plan.changes {
        lines.push(format!("· {change}"));
    }
    lines
}
impl Drop for Page {
    fn drop(&mut self) {
        // A terminal I/O failure must not let process exit interrupt an active transaction.
        if let Some(handle) = self.worker_thread.take() {
            let _ = handle.join();
        }
    }
}
fn work(
    paths: &Paths,
    task: Task,
    runner: &dyn Runner,
    mut progress: impl FnMut(Progress),
) -> Result<Outcome> {
    let service = Service { paths, runner };
    let mut notice = "已重新读取环境状态".to_string();
    match task {
        Task::Doctor => return Ok(Outcome::Diagnosed(env::doctor(paths, runner))),
        Task::Apply(plan) => match service.apply_with_progress(&plan, &mut progress) {
            Ok(()) => notice = "应用成功；已重新读取最新状态".into(),
            Err(e) if e.is::<env::StalePlan>() => {
                notice = format!("{e:#}。未应用旧计划；已刷新，请查看后再次确认。");
            }
            Err(e) => return Err(e),
        },
        Task::Refresh => {}
    }
    let config = LocalConfig::load(&paths.config())?;
    let e = &config.environment;
    if e.source.is_empty() || e.profile.is_empty() || e.agents.is_empty() {
        return Ok(Outcome::Prepared(
            config,
            None,
            "来源、Profile 或启用 Agent 尚未配置完整".into(),
        ));
    }
    let plan = service
        .prepare_with_progress(progress)
        .with_context(|| format!("{notice}；读取最新状态"))?;
    Ok(Outcome::Prepared(
        plan.config.clone(),
        Some(Arc::new(plan)),
        notice,
    ))
}
