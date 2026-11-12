mod view;
use super::*;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::time::Duration;
type Screen = Terminal<CrosstermBackend<io::Stdout>>;
struct TerminalGuard;
impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        if let Err(e) = execute!(io::stdout(), EnterAlternateScreen, crossterm::cursor::Hide) {
            let _ = disable_raw_mode();
            return Err(e.into());
        }
        Ok(Self)
    }
    fn restore() {
        let _ = execute!(io::stdout(), crossterm::cursor::Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        Self::restore();
    }
}
pub fn secret(title: &str) -> Result<Option<String>> {
    Ok(Some(rpassword::prompt_password(format!("{title}: "))?))
}
pub fn confirmed(title: &str) -> Result<bool> {
    let _guard = TerminalGuard::enter()?;
    let mut screen = Screen::new(CrosstermBackend::new(io::stdout()))?;
    loop {
        screen.draw(|f| view::confirmation(f, title))?;
        if let Event::Key(k) = event::read()? {
            if k.kind == KeyEventKind::Release {
                continue;
            }
            match k.code {
                KeyCode::Char('y') => return Ok(true),
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => return Ok(false),
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(false)
                }
                _ => {}
            }
        }
    }
}
#[derive(Clone, PartialEq)]
struct Input {
    label: &'static str,
    value: String,
    cursor: usize,
}
impl Input {
    fn new(label: &'static str, value: String) -> Self {
        let cursor = value.len();
        Self {
            label,
            value,
            cursor,
        }
    }
    fn key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.value.len(),
            KeyCode::Left => {
                self.cursor = self.value[..self.cursor]
                    .char_indices()
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            }
            KeyCode::Right => {
                if let Some(c) = self.value[self.cursor..].chars().next() {
                    self.cursor += c.len_utf8();
                }
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let at = self.value[..self.cursor]
                        .char_indices()
                        .next_back()
                        .unwrap()
                        .0;
                    self.value.drain(at..self.cursor);
                    self.cursor = at;
                }
            }
            KeyCode::Delete => {
                if let Some(c) = self.value[self.cursor..].chars().next() {
                    self.value.drain(self.cursor..self.cursor + c.len_utf8());
                }
            }
            KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.value.clear();
                self.cursor = 0;
            }
            KeyCode::Char(c)
                if !k
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.value.insert(self.cursor, c);
                self.cursor += c.len_utf8();
            }
            _ => {}
        }
    }
}
#[derive(Clone)]
enum FormKind {
    Custom(String),
    New(Option<String>),
    Rename(String, Mode),
    Proxy,
}
struct Form {
    kind: FormKind,
    mode: Mode,
    inputs: Vec<Input>,
    original: Vec<String>,
    focus: usize,
    error: String,
}
impl Form {
    fn dirty(&self) -> bool {
        self.inputs
            .iter()
            .map(|i| &i.value)
            .ne(self.original.iter())
    }
    fn count(&self) -> usize {
        if matches!(self.kind, FormKind::New(_)) && self.mode == Mode::Official {
            2
        } else {
            self.inputs.len()
        }
    }
    fn get(&self, i: usize) -> String {
        self.inputs[i].value.clone()
    }
}
#[derive(Clone)]
enum Confirm {
    Delete(String, Mode),
    Discard,
    Repair,
    Launch,
}
#[derive(Debug, PartialEq)]
enum Effect {
    None,
    Quit,
    Login(String),
    Launch(bool),
    Repair,
}
struct App {
    session: Session,
    agent: Agent,
    index: usize,
    filter: String,
    filtering: bool,
    form: Option<Form>,
    confirm: Option<Confirm>,
    help: bool,
    detail_scroll: u16,
    message: String,
    error: bool,
}
impl App {
    fn new(store: Store, agent: Agent) -> Result<Self> {
        Ok(Self {
            session: Session::open(store)?,
            agent,
            index: 0,
            filter: String::new(),
            filtering: false,
            form: None,
            confirm: None,
            help: false,
            detail_scroll: 0,
            message: "就绪 · 高亮仅查看，Enter 设为当前；s 显式启动".into(),
            error: false,
        })
    }
    fn rows(&self) -> Vec<(Mode, acs_core::config::Profile)> {
        let d = self.session.original.agent(self.agent);
        [Mode::Official, Mode::Custom]
            .into_iter()
            .flat_map(|m| {
                d.library
                    .profiles(m)
                    .iter()
                    .filter(|p| {
                        format!(
                            "{} {} {}",
                            p.name.as_deref().unwrap_or(""),
                            p.email.as_deref().unwrap_or(""),
                            p.id
                        )
                        .to_lowercase()
                        .contains(&self.filter.to_lowercase())
                    })
                    .cloned()
                    .map(move |p| (m, p))
            })
            .collect()
    }
    fn selected(&self) -> Option<(Mode, acs_core::config::Profile)> {
        self.rows().get(self.index).cloned()
    }
    fn result(&mut self, r: Result<()>, success: &str) {
        match r {
            Ok(()) => {
                self.message = success.into();
                self.error = false;
            }
            Err(e) => {
                self.message = format!("{e:#}");
                self.error = true;
            }
        }
    }
    fn save(&mut self, d: &Draft) -> Result<()> {
        self.session
            .save_for(&self.session.view().revision, d, self.agent)?;
        Ok(())
    }
    fn reload(&mut self) -> Result<()> {
        self.session = Session::open(self.session.store.clone())?;
        self.index = self.index.min(self.rows().len().saturating_sub(1));
        Ok(())
    }
    fn begin(&mut self, kind: FormKind) -> Result<()> {
        let d = self.session.original.agent(self.agent);
        let mut mode = Mode::Custom;
        let mut fields = CustomFields::blank(self.agent);
        let mut name = String::new();
        if let Some((m, p)) = self.selected() {
            mode = m;
            name = p.name.unwrap_or_default();
            if let Some(f) = d.custom_fields.get(&p.id) {
                fields = f.clone();
            }
        }
        let inputs = match &kind {
            FormKind::Proxy => vec![Input::new(
                "官方 HTTP 代理",
                d.library.official_proxy_url.clone().unwrap_or_default(),
            )],
            FormKind::Rename(_, _) => vec![Input::new("名称", name)],
            FormKind::Custom(_) => custom_inputs(name, fields),
            FormKind::New(copy) => {
                if copy.is_none() {
                    mode = Mode::Custom;
                    name = String::new();
                    fields = CustomFields::blank(self.agent);
                } else {
                    name.push_str(" 副本");
                }
                let mut all = vec![Input::new("类型（←→切换）", mode.label().into())];
                all.extend(custom_inputs(name, fields));
                all
            }
        };
        self.form = Some(Form {
            kind,
            mode,
            original: inputs.iter().map(|i| i.value.clone()).collect(),
            inputs,
            focus: 0,
            error: String::new(),
        });
        self.detail_scroll = 0;
        Ok(())
    }
    fn save_form(&mut self) -> Result<()> {
        let f = self.form.as_ref().context("表单不存在")?;
        let mut draft = self.session.original.clone();
        let d = draft.agent_mut(self.agent);
        match &f.kind {
            FormKind::Proxy => d.library.official_proxy_url = Some(f.get(0)),
            FormKind::Rename(id, m) => d.rename(*m, id, &f.get(0))?,
            FormKind::Custom(id) => {
                d.rename(Mode::Custom, id, &f.get(0))?;
                d.custom_fields.insert(id.clone(), fields_from(f, 0));
            }
            FormKind::New(copy) => {
                let id = d.add(
                    f.mode,
                    &f.get(1),
                    if f.mode == Mode::Custom {
                        copy.as_deref()
                    } else {
                        None
                    },
                )?;
                if f.mode == Mode::Custom {
                    d.custom_fields.insert(id, fields_from(f, 1));
                }
                d.mode = f.mode;
            }
        }
        self.save(&draft)?;
        self.form = None;
        self.index = self
            .rows()
            .iter()
            .position(|(m, p)| {
                self.session.original.agent(self.agent).mode == *m
                    && self
                        .session
                        .original
                        .agent(self.agent)
                        .library
                        .selection(*m)
                        == p.id
            })
            .unwrap_or(0);
        Ok(())
    }
    fn key(&mut self, k: KeyEvent) -> Effect {
        if k.kind == KeyEventKind::Release {
            return Effect::None;
        }
        if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
            return Effect::Quit;
        }
        if self.help {
            if matches!(
                k.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
            ) {
                self.help = false;
            }
            return Effect::None;
        }
        if let Some(c) = self.confirm.clone() {
            if matches!(
                k.code,
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q')
            ) {
                self.confirm = None;
                return Effect::None;
            }
            if matches!(c, Confirm::Launch) {
                let desktop = k.code == KeyCode::Char('2') && !cfg!(target_os = "linux");
                if desktop || k.code == KeyCode::Char('1') || k.code == KeyCode::Enter {
                    self.confirm = None;
                    return Effect::Launch(desktop);
                }
            } else if k.code == KeyCode::Char('y') {
                self.confirm = None;
                match c {
                    Confirm::Discard => {
                        self.form = None;
                        self.message = "已放弃修改".into();
                    }
                    Confirm::Repair => return Effect::Repair,
                    Confirm::Delete(id, m) => {
                        let mut d = self.session.original.clone();
                        let r = d
                            .agent_mut(self.agent)
                            .delete(m, &id)
                            .and_then(|_| self.save(&d));
                        self.result(r, "已删除并保存");
                        self.index = self.index.min(self.rows().len().saturating_sub(1));
                    }
                    Confirm::Launch => {}
                }
            }
            return Effect::None;
        }
        if self.form.is_some() {
            if k.code == KeyCode::Esc {
                if self.form.as_ref().unwrap().dirty() {
                    self.confirm = Some(Confirm::Discard);
                } else {
                    self.form = None;
                    self.message = "未修改，已返回".into();
                }
            } else if k.code == KeyCode::F(2)
                || (k.code == KeyCode::Char('s') && k.modifiers.contains(KeyModifiers::CONTROL))
            {
                let r = self.save_form();
                if let Err(e) = &r {
                    self.form.as_mut().unwrap().error = format!("{e:#}");
                }
                self.result(r, "已保存配置；未启动客户端");
            } else {
                let f = self.form.as_mut().unwrap();
                match k.code {
                    KeyCode::Up | KeyCode::BackTab => f.focus = f.focus.saturating_sub(1),
                    KeyCode::Down | KeyCode::Tab | KeyCode::Enter => {
                        f.focus = (f.focus + 1) % f.count()
                    }
                    KeyCode::Left | KeyCode::Right
                        if f.focus == 0 && matches!(f.kind, FormKind::New(_)) =>
                    {
                        f.mode = if f.mode == Mode::Official {
                            Mode::Custom
                        } else {
                            Mode::Official
                        };
                        f.inputs[0].value = f.mode.label().into();
                    }
                    _ => {
                        if f.focus != 0 || !matches!(f.kind, FormKind::New(_)) {
                            f.inputs[f.focus].key(k);
                        }
                    }
                }
            }
            return Effect::None;
        }
        if self.filtering {
            match k.code {
                KeyCode::Esc | KeyCode::Enter => self.filtering = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) => self.filter.push(c),
                _ => {}
            }
            self.index = 0;
            return Effect::None;
        }
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => return Effect::Quit,
            KeyCode::Up | KeyCode::Char('k') => {
                self.index = self.index.saturating_sub(1);
                self.detail_scroll = 0;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.index = (self.index + 1).min(self.rows().len().saturating_sub(1));
                self.detail_scroll = 0;
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                self.agent = if self.agent == Agent::Codex {
                    Agent::Claude
                } else {
                    Agent::Codex
                };
                self.index = 0;
                self.filter.clear();
                self.detail_scroll = 0;
            }
            KeyCode::PageDown => self.detail_scroll = self.detail_scroll.saturating_add(5),
            KeyCode::PageUp => self.detail_scroll = self.detail_scroll.saturating_sub(5),
            KeyCode::Char('/') => {
                self.filtering = true;
                self.filter.clear();
            }
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('n') => {
                let r = self.begin(FormKind::New(None));
                self.result(r, "新增配置 · F2 保存，Esc 取消");
            }
            KeyCode::Char('p') => {
                let r = self.begin(FormKind::Proxy);
                self.result(r, "编辑官方代理 · F2 保存");
            }
            KeyCode::Char('s') => self.confirm = Some(Confirm::Launch),
            KeyCode::Char('h') => self.confirm = Some(Confirm::Repair),
            KeyCode::Char('R') => {
                let r = self.reload();
                self.result(r, "已重新读取配置");
            }
            _ => {
                if let Some((m, p)) = self.selected() {
                    match k.code {
                        KeyCode::Enter => {
                            let mut draft = self.session.original.clone();
                            let d = draft.agent_mut(self.agent);
                            d.mode = m;
                            d.library.select(m, p.id);
                            let r = self.save(&draft);
                            self.result(r, "已设为当前；未启动客户端");
                        }
                        KeyCode::Char('e') => {
                            let kind = if m == Mode::Custom {
                                FormKind::Custom(p.id)
                            } else {
                                FormKind::Rename(p.id, m)
                            };
                            let r = self.begin(kind);
                            self.result(r, "编辑配置 · F2 保存");
                        }
                        KeyCode::Char('c') => {
                            let r = self.begin(FormKind::New(Some(p.id)));
                            self.result(r, "复制为新配置 · F2 保存");
                        }
                        KeyCode::Char('r') => {
                            let r = self.begin(FormKind::Rename(p.id, m));
                            self.result(r, "改名 · F2 保存");
                        }
                        KeyCode::Char('d') => self.confirm = Some(Confirm::Delete(p.id, m)),
                        KeyCode::Char('l') => {
                            if self.agent == Agent::Codex && m == Mode::Official {
                                return Effect::Login(p.id);
                            }
                            self.result(Err(anyhow::anyhow!("登录仅支持 Codex 官方账号")), "");
                        }
                        _ => {}
                    }
                }
            }
        }
        Effect::None
    }
}
fn custom_inputs(name: String, f: CustomFields) -> Vec<Input> {
    [
        ("名称", name),
        ("提供者名称", f.provider_name),
        ("API 地址", f.base_url),
        ("API Key", f.api_key),
        ("模型", f.model),
        ("思考层级", f.effort),
        ("HTTP 代理", f.proxy_url),
    ]
    .into_iter()
    .map(|(l, v)| Input::new(l, v))
    .collect()
}
fn fields_from(f: &Form, o: usize) -> CustomFields {
    CustomFields {
        provider_name: f.get(o + 1),
        base_url: f.get(o + 2),
        api_key: f.get(o + 3),
        model: f.get(o + 4),
        effort: f.get(o + 5),
        proxy_url: f.get(o + 6),
    }
}
pub fn menu(store: Store, selected: Option<AgentArg>) -> Result<()> {
    run_tui(store, selected, None)
}
pub fn dispatch(store: Store, action: Action, selected: Option<AgentArg>) -> Result<()> {
    let tui = match &action {
        Action::Add {
            agent,
            kind,
            name,
            fields,
            ..
        } => agent.is_none() || kind.is_none() || name.is_none() || fields.is_empty(),
        Action::Edit { target, fields } => target.is_none() || fields.is_empty(),
        Action::Use { target } | Action::Rm { target, .. } => target.is_none(),
        _ => false,
    };
    if tui {
        run_tui(store, selected, Some(action))
    } else {
        execute(store, action, selected)
    }
}
fn run_tui(store: Store, selected: Option<AgentArg>, initial: Option<Action>) -> Result<()> {
    let old = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        TerminalGuard::restore();
        old(info);
    }));
    let mut app = App::new(
        store.clone(),
        selected.map(Into::into).unwrap_or(Agent::Codex),
    )?;
    match initial {
        Some(Action::Add {
            agent,
            kind,
            name,
            copy,
            fields,
        }) => {
            if let Some(a) = agent {
                app.agent = a.into();
            }
            app.begin(FormKind::New(copy))?;
            let form = app.form.as_mut().unwrap();
            if let Some(k) = kind {
                form.mode = k.into();
                form.inputs[0].value = form.mode.label().into();
            }
            let mut f = fields_from(form, 1);
            fields.apply(&mut f);
            let mut inputs = vec![form.inputs[0].clone()];
            inputs.extend(custom_inputs(name.unwrap_or_else(|| form.get(1)), f));
            form.inputs = inputs;
            form.original = form.inputs.iter().map(|i| i.value.clone()).collect();
        }
        Some(Action::Edit {
            target: Some(q), ..
        }) => {
            let (a, m, id) = locate(&app.session.original, &q)?;
            app.agent = a;
            app.index = app.rows().iter().position(|(_, p)| p.id == id).unwrap_or(0);
            app.begin(if m == Mode::Custom {
                FormKind::Custom(id)
            } else {
                FormKind::Rename(id, m)
            })?;
        }
        Some(Action::Edit { target: None, .. }) => app.message = "选择配置后按 e 编辑".into(),
        Some(Action::Rm { .. }) => app.message = "选择配置后按 d 删除；y 确认".into(),
        _ => {}
    }
    let mut guard = Some(TerminalGuard::enter()?);
    let mut screen = Screen::new(CrosstermBackend::new(io::stdout()))?;
    loop {
        screen.draw(|f| view::draw(f, &app))?;
        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        let effect = match event::read()? {
            Event::Key(k) => app.key(k),
            Event::Resize(_, _) => {
                screen.clear()?;
                Effect::None
            }
            _ => Effect::None,
        };
        match effect {
            Effect::None => {}
            Effect::Quit => break,
            Effect::Login(id) => {
                drop(guard.take());
                let r = execute(store.clone(), Action::Login { target: id }, None);
                app.result(r, "登录完成，已保存并设为当前");
                if let Err(e) = app.reload() {
                    app.result(Err(e), "");
                }
                guard = Some(TerminalGuard::enter()?);
                screen.clear()?;
            }
            Effect::Launch(desktop) => {
                let r = (|| -> Result<_> {
                    let settings = app.session.launch_settings_for(
                        &app.session.view().revision,
                        &app.session.original,
                        app.agent,
                    )?;
                    let exe = if desktop {
                        acs_core::launch::desktop_executable(app.agent)?
                    } else {
                        acs_core::launch::cli_executable(app.agent)
                    };
                    settings.plan(exe, desktop, Vec::new())
                })();
                match r {
                    Ok(plan) => {
                        drop(guard.take());
                        let r = if desktop {
                            plan.spawn_desktop().map(|_| 0)
                        } else {
                            plan.spawn()
                                .and_then(|mut p| Ok(p.wait()?.code().unwrap_or(1)))
                        };
                        match r {
                            Ok(code) => std::process::exit(code),
                            Err(e) => {
                                app.result(Err(e), "");
                                guard = Some(TerminalGuard::enter()?);
                                screen.clear()?;
                            }
                        }
                    }
                    Err(e) => app.result(Err(e), ""),
                }
            }
            Effect::Repair => {
                let progress = std::cell::RefCell::new((&mut app, &mut screen));
                let report = acs_core::history::repair(&store, |p| {
                    let mut pair = progress.borrow_mut();
                    pair.0.message = format!("修复 {} {}/{}", p.phase, p.completed, p.total);
                    let (app, screen) = &mut *pair;
                    let _ = screen.draw(|f| view::draw(f, app));
                });
                app.result(report.map(|_| ()), "历史修复完成；备份已创建");
            }
        }
    }
    drop(guard);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    fn fixture() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(Roots::under(dir.path()));
        let mut s = Session::open(store.clone()).unwrap();
        let mut d = s.original.clone();
        for a in Agent::ALL {
            let p = d.agent_mut(a);
            p.add(Mode::Official, "example 官方账号", None).unwrap();
            let id = p.add(Mode::Custom, "example 中文 API", None).unwrap();
            p.mode = Mode::Custom;
            p.custom_fields.insert(
                id,
                CustomFields {
                    provider_name: "example".into(),
                    base_url: "https://api.example.com/v1".into(),
                    api_key: "example-key".into(),
                    model: "example-model".into(),
                    effort: "medium".into(),
                    proxy_url: String::new(),
                },
            );
        }
        s.save(&s.view().revision, &d).unwrap();
        (dir, App::new(store, Agent::Codex).unwrap())
    }
    fn key(app: &mut App, c: KeyCode) -> Effect {
        app.key(KeyEvent::new(c, KeyModifiers::NONE))
    }
    fn grid(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| view::draw(f, app)).unwrap();
        let b = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                use unicode_width::UnicodeWidthStr;
                let mut s = String::new();
                let mut x = 0;
                while x < width {
                    let symbol = b[(x, y)].symbol();
                    s.push_str(symbol);
                    x += symbol.width().max(1) as u16;
                }
                s
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn navigation_never_saves_and_untouched_cancel_needs_no_confirmation() {
        let (_dir, mut app) = fixture();
        let before = app.session.store.snapshot().unwrap();
        key(&mut app, KeyCode::Down);
        assert_eq!(app.index, 1);
        assert_eq!(app.session.store.snapshot().unwrap(), before);
        key(&mut app, KeyCode::Char('e'));
        assert!(app.form.is_some());
        key(&mut app, KeyCode::Esc);
        assert!(app.form.is_none() && app.confirm.is_none());
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.agent, Agent::Claude);
        assert_eq!(app.session.store.snapshot().unwrap(), before);
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Effect::Quit
        );
    }
    #[test]
    fn dirty_cancel_and_validation_failure_retain_form_and_explicit_discard_closes_it() {
        let (_dir, mut app) = fixture();
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Char('e'));
        app.form.as_mut().unwrap().inputs[2] = Input::new("API 地址", "bad-url".into());
        let before = app.session.store.snapshot().unwrap();
        key(&mut app, KeyCode::F(2));
        assert!(app.form.as_ref().unwrap().error.contains("地址"));
        assert_eq!(app.form.as_ref().unwrap().get(2), "bad-url");
        assert_eq!(app.session.store.snapshot().unwrap(), before);
        key(&mut app, KeyCode::Esc);
        assert!(app.confirm.is_some());
        key(&mut app, KeyCode::Esc);
        assert!(app.form.is_some() && app.confirm.is_none());
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Char('y'));
        assert!(app.form.is_none());
    }
    #[test]
    fn layouts_and_overlays_render_without_wrapping_status_rows_or_losing_context() {
        let (_dir, mut app) = fixture();
        for (w, h) in [(120, 32), (80, 24), (60, 20), (20, 8)] {
            let g = grid(&app, w, h);
            assert!(g.contains("acs 3.0.0"));
            if w >= 60 {
                assert!(g.contains("Codex") && g.contains("Claude"));
            }
        }
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Char('e'));
        assert!(grid(&app, 80, 24).contains("编辑表单"));
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Char('d'));
        assert!(grid(&app, 80, 24).contains("确认操作"));
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Char('?'));
        assert!(grid(&app, 120, 32).contains("按键帮助"));
        assert_eq!(view::truncate("中文ab", 5), "中文…");
    }
    #[test]
    fn unicode_cursor_and_delete_keep_utf8_valid() {
        let mut i = Input::new("值", "中🙂文".into());
        i.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        i.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(i.value, "中文");
        i.key(KeyEvent::new(KeyCode::Char('日'), KeyModifiers::NONE));
        assert_eq!(i.value, "中日文");
    }
}
