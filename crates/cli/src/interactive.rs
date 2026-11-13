mod environment;
#[cfg(test)]
mod environment_tests;
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
        static HOOK: std::sync::Once = std::sync::Once::new();
        HOOK.call_once(|| {
            let old = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                TerminalGuard::restore();
                old(info);
            }));
        });
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
        self.cursor = self.position();
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
    fn position(&self) -> usize {
        let mut pos = self.cursor.min(self.value.len());
        while !self.value.is_char_boundary(pos) {
            pos -= 1;
        }
        pos
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
            1
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
    EnvRefresh,
    EnvApply,
    EnvDoctor,
    EnvBack,
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
    type_picker: bool,
    login: Option<LoginPrompt>,
    environment: environment::Page,
    environment_open: bool,
    quit_after_environment: bool,
}
struct LoginPrompt {
    id: String,
    proxy: Input,
    editing: bool,
    error: String,
    route: std::result::Result<acs_core::oauth::LoginRoute, String>,
}
impl App {
    fn new(store: Store, agent: Agent, paths: acs_core::environment::Paths) -> Result<Self> {
        let mut app = Self {
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
            type_picker: false,
            login: None,
            environment: environment::Page::new(paths),
            environment_open: false,
            quit_after_environment: false,
        };
        app.focus_current();
        Ok(app)
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
    fn focus_current(&mut self) {
        let d = self.session.original.agent(self.agent);
        self.index = self
            .rows()
            .iter()
            .position(|(m, p)| d.mode == *m && d.library.selection(*m) == p.id)
            .unwrap_or(0);
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
                    mode = Mode::Official;
                    name = String::new();
                    fields = CustomFields::blank(self.agent);
                } else {
                    name.push_str(" 副本");
                }
                custom_inputs(name, fields)
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
    fn ask_login(&mut self, id: String) {
        let proxy = self
            .session
            .original
            .codex
            .library
            .official_proxy_url
            .clone()
            .unwrap_or_default();
        let route = acs_core::oauth::LoginRoute::resolve(&proxy).map_err(|e| format!("{e:#}"));
        self.login = Some(LoginPrompt {
            id,
            proxy: Input::new("官方 HTTP 代理", proxy),
            editing: false,
            error: String::new(),
            route,
        });
        self.message = "登录前请确认换令牌代理；p 编辑代理，y / Enter 登录，Esc 稍后".into();
        self.error = false;
    }
    fn save_login_proxy(&mut self) -> Result<()> {
        let value = self
            .login
            .as_ref()
            .context("登录确认不存在")?
            .proxy
            .value
            .clone();
        let mut draft = self.session.original.clone();
        draft.codex.library.official_proxy_url = Some(value);
        self.session
            .save_for(&self.session.view().revision, &draft, Agent::Codex)?;
        let value = self
            .session
            .original
            .codex
            .library
            .official_proxy_url
            .clone()
            .unwrap_or_default();
        let prompt = self.login.as_mut().unwrap();
        prompt.route = acs_core::oauth::LoginRoute::resolve(&value).map_err(|e| format!("{e:#}"));
        prompt.proxy = Input::new("官方 HTTP 代理", value);
        prompt.editing = false;
        prompt.error.clear();
        Ok(())
    }
    fn login_key(&mut self, k: KeyEvent) -> Effect {
        let editing = self.login.as_ref().unwrap().editing;
        if editing {
            if k.code == KeyCode::Esc {
                let value = self
                    .session
                    .original
                    .codex
                    .library
                    .official_proxy_url
                    .clone()
                    .unwrap_or_default();
                let prompt = self.login.as_mut().unwrap();
                prompt.proxy = Input::new("官方 HTTP 代理", value);
                prompt.editing = false;
                prompt.error.clear();
            } else if matches!(k.code, KeyCode::Enter | KeyCode::F(2))
                || (k.code == KeyCode::Char('s') && k.modifiers.contains(KeyModifiers::CONTROL))
            {
                match self.save_login_proxy() {
                    Ok(()) => {
                        self.message = "官方代理已保存；y / Enter 继续登录".into();
                        self.error = false;
                        if k.code == KeyCode::Enter {
                            return Effect::Login(self.login.take().unwrap().id);
                        }
                    }
                    Err(e) => {
                        let message = format!("{e:#}");
                        self.login.as_mut().unwrap().error = message.clone();
                        self.result(Err(e), "");
                    }
                }
            } else {
                self.login.as_mut().unwrap().proxy.key(k);
            }
        } else {
            match k.code {
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => {
                    self.login = None;
                    self.message = "已保存账号；稍后高亮该账号按 l 登录".into();
                    self.error = false;
                }
                KeyCode::Char('p') => self.login.as_mut().unwrap().editing = true,
                KeyCode::Char('y') | KeyCode::Enter => {
                    return Effect::Login(self.login.take().unwrap().id)
                }
                _ => {}
            }
        }
        Effect::None
    }
    fn save_form(&mut self) -> Result<()> {
        let f = self.form.as_ref().context("表单不存在")?;
        let mut draft = self.session.original.clone();
        let d = draft.agent_mut(self.agent);
        let mut created_official = None;
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
                    &f.get(0),
                    if f.mode == Mode::Custom {
                        copy.as_deref()
                    } else {
                        None
                    },
                )?;
                if f.mode == Mode::Custom {
                    d.custom_fields.insert(id.clone(), fields_from(f, 0));
                } else {
                    created_official = Some(id);
                }
                d.mode = f.mode;
            }
        }
        self.save(&draft)?;
        self.form = None;
        self.message = "已保存配置；未启动客户端".into();
        if let Some(id) = created_official {
            self.filter.clear();
            if self.agent == Agent::Codex {
                self.ask_login(id);
            } else {
                self.message = "已保存 Claude 官方账号；请启动 Claude 客户端完成登录".into();
            }
        }
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
            if self.environment.busy() {
                self.quit_after_environment = true;
                self.environment.notice =
                    "已请求退出；当前任务完成后恢复终端并退出，避免中断事务。".into();
                return Effect::None;
            }
            return Effect::Quit;
        }
        if self.environment_open && !self.help {
            if k.code == KeyCode::Char('?') {
                self.help = true;
                return Effect::None;
            }
            if !self.environment.busy() && self.environment.state != environment::State::Confirm {
                match k.code {
                    KeyCode::Tab | KeyCode::Right => {
                        self.environment_open = false;
                        self.agent = Agent::Codex;
                        self.focus_current();
                        return Effect::None;
                    }
                    KeyCode::BackTab | KeyCode::Left => {
                        self.environment_open = false;
                        self.agent = Agent::Claude;
                        self.focus_current();
                        return Effect::None;
                    }
                    _ => {}
                }
            }
            return self.environment.key(k);
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
        if self.login.is_some() {
            return self.login_key(k);
        }
        if self.type_picker {
            match k.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.type_picker = false;
                    self.form = None;
                    self.message = "已取消新增".into();
                }
                KeyCode::Enter => {
                    self.type_picker = false;
                    self.form.as_mut().unwrap().focus = 0;
                    self.message = "填写名称；F2 保存，Esc 取消".into();
                }
                KeyCode::Left
                | KeyCode::Right
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Tab
                | KeyCode::BackTab => {
                    let form = self.form.as_mut().unwrap();
                    form.mode = if form.mode == Mode::Official {
                        Mode::Custom
                    } else {
                        Mode::Official
                    };
                }
                KeyCode::Home | KeyCode::Char('1') => {
                    self.form.as_mut().unwrap().mode = Mode::Official
                }
                KeyCode::End | KeyCode::Char('2') => {
                    self.form.as_mut().unwrap().mode = Mode::Custom
                }
                _ => {}
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
                if let Err(e) = r {
                    self.result(Err(e), "");
                } else {
                    self.error = false;
                }
            } else {
                let f = self.form.as_mut().unwrap();
                match k.code {
                    KeyCode::Up | KeyCode::BackTab => f.focus = f.focus.saturating_sub(1),
                    KeyCode::Down | KeyCode::Tab | KeyCode::Enter => {
                        f.focus = (f.focus + 1) % f.count()
                    }
                    _ => f.inputs[f.focus].key(k),
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
            KeyCode::Char('v') | KeyCode::F(3) => {
                self.environment_open = true;
                return Effect::EnvRefresh;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.index = self.index.saturating_sub(1);
                self.detail_scroll = 0;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.index = (self.index + 1).min(self.rows().len().saturating_sub(1));
                self.detail_scroll = 0;
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                let forward = matches!(k.code, KeyCode::Right | KeyCode::Tab);
                if (forward && self.agent == Agent::Claude)
                    || (!forward && self.agent == Agent::Codex)
                {
                    self.environment_open = true;
                    return Effect::EnvRefresh;
                }
                self.agent = if self.agent == Agent::Codex {
                    Agent::Claude
                } else {
                    Agent::Codex
                };
                self.index = 0;
                self.filter.clear();
                self.detail_scroll = 0;
                self.focus_current();
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
                self.type_picker = r.is_ok();
                self.result(r, "新增配置：先选择官方账号或自定义 API");
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
                                self.ask_login(p.id);
                            } else {
                                self.result(
                                    Err(anyhow::anyhow!(
                                        "Claude 官方账号请由 Claude 客户端登录；API 配置无需 OAuth"
                                    )),
                                    "",
                                );
                            }
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
pub fn menu(
    store: Store,
    selected: Option<AgentArg>,
    root: Option<&std::path::Path>,
) -> Result<()> {
    run_tui(store, selected, None, root)
}
pub fn dispatch(
    store: Store,
    action: Action,
    selected: Option<AgentArg>,
    root: Option<&std::path::Path>,
) -> Result<()> {
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
        run_tui(store, selected, Some(action), root)
    } else {
        execute(store, action, selected)
    }
}
fn run_tui(
    store: Store,
    selected: Option<AgentArg>,
    initial: Option<Action>,
    root: Option<&std::path::Path>,
) -> Result<()> {
    let paths = match root {
        Some(root) => acs_core::environment::Paths::under(root),
        None => acs_core::environment::Paths::discover()?,
    };
    let mut app = App::new(
        store.clone(),
        selected.map(Into::into).unwrap_or(Agent::Codex),
        paths,
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
            } else {
                app.type_picker = true;
            }
            let mut f = fields_from(form, 0);
            fields.apply(&mut f);
            form.inputs = custom_inputs(name.unwrap_or_else(|| form.get(0)), f);
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
        if app.environment.poll() && !app.environment.busy() {
            // Environment apply may update Codex config.toml; refresh account baselines too.
            if let Err(e) = app.reload() {
                app.environment
                    .notice
                    .push_str(&format!("；账号状态读取失败：{e:#}"));
                app.result(Err(e), "");
            }
        }
        if app.quit_after_environment && !app.environment.busy() {
            break;
        }
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
            Effect::EnvBack => {
                app.environment_open = false;
            }
            Effect::EnvRefresh => app.environment.start(
                environment::Task::Refresh,
                acs_core::environment::command::SystemRunner,
            ),
            Effect::EnvDoctor => app.environment.start(
                environment::Task::Doctor,
                acs_core::environment::command::SystemRunner,
            ),
            Effect::EnvApply => {
                if let Some(plan) = app.environment.plan.take() {
                    app.environment.start(
                        environment::Task::Apply(plan),
                        acs_core::environment::command::SystemRunner,
                    );
                }
            }
            Effect::Quit => break,
            Effect::Login(id) => {
                drop(guard.take());
                match login_command(store.clone(), &id) {
                    Ok(true) => app.result(Ok(()), "登录完成，已保存并设为当前"),
                    Ok(false) => {
                        app.result(Ok(()), "已取消登录；已保存配置和代理保留，稍后按 l 登录")
                    }
                    Err(e) => app.result(Err(e), ""),
                }
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
    pub(super) fn fixture() -> (tempfile::TempDir, App) {
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
        let mut app = App::new(
            store,
            Agent::Codex,
            acs_core::environment::Paths::under(dir.path()),
        )
        .unwrap();
        app.index = 0;
        (dir, app)
    }
    fn key(app: &mut App, c: KeyCode) -> Effect {
        app.key(KeyEvent::new(c, KeyModifiers::NONE))
    }
    pub(super) fn grid(app: &App, width: u16, height: u16) -> String {
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
            assert!(g.contains(concat!("acs ", env!("CARGO_PKG_VERSION"))));
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
    #[test]
    fn new_official_account_offers_proxy_and_login_while_claude_explains_client_login() {
        for agent in Agent::ALL {
            let (_dir, mut app) = fixture();
            app.agent = agent;
            key(&mut app, KeyCode::Char('n'));
            assert!(app.type_picker);
            for code in [
                KeyCode::Left,
                KeyCode::Right,
                KeyCode::Home,
                KeyCode::End,
                KeyCode::Backspace,
                KeyCode::Delete,
            ] {
                key(&mut app, code);
                assert!(app
                    .form
                    .as_ref()
                    .unwrap()
                    .inputs
                    .iter()
                    .all(|i| i.cursor <= i.value.len() && i.value.is_char_boundary(i.cursor)));
                grid(&app, 80, 24);
            }
            key(&mut app, KeyCode::Home);
            key(&mut app, KeyCode::Enter);
            assert_eq!(app.form.as_ref().unwrap().count(), 1);
            for c in "example 新官方账号".chars() {
                key(&mut app, KeyCode::Char(c));
            }
            key(&mut app, KeyCode::F(2));
            assert!(app.form.is_none());
            if agent == Agent::Codex {
                assert!(app.login.is_some());
                assert!(grid(&app, 120, 32).contains("地区限制"));
                key(&mut app, KeyCode::Char('p'));
                for c in "http://proxy.example.test:8080".chars() {
                    key(&mut app, KeyCode::Char(c));
                }
                key(&mut app, KeyCode::F(2));
                assert!(app.login.as_ref().unwrap().error.is_empty());
                assert_eq!(
                    app.session
                        .original
                        .codex
                        .library
                        .official_proxy_url
                        .as_deref(),
                    Some("http://proxy.example.test:8080")
                );
                let id = app.login.as_ref().unwrap().id.clone();
                assert_eq!(key(&mut app, KeyCode::Enter), Effect::Login(id));
            } else {
                assert!(app.login.is_none());
                assert!(app.message.contains("Claude 客户端"));
                let g = grid(&app, 120, 32);
                assert!(!g.contains("l 登录") && !g.contains("l登"));
            }
        }
    }
    fn assert_inputs(app: &App) {
        if let Some(form) = &app.form {
            assert!(form.focus < form.count());
            for input in &form.inputs {
                assert!(input.cursor <= input.value.len());
                assert!(input.value.is_char_boundary(input.cursor));
            }
        }
        if let Some(prompt) = &app.login {
            assert!(prompt.proxy.cursor <= prompt.proxy.value.len());
            assert!(prompt.proxy.value.is_char_boundary(prompt.proxy.cursor));
        }
    }
    fn keys() -> Vec<KeyEvent> {
        let mut keys: Vec<_> = (32u8..=126)
            .map(|b| KeyEvent::new(KeyCode::Char(b as char), KeyModifiers::NONE))
            .collect();
        keys.extend(
            [
                KeyCode::Left,
                KeyCode::Right,
                KeyCode::Up,
                KeyCode::Down,
                KeyCode::Home,
                KeyCode::End,
                KeyCode::Backspace,
                KeyCode::Delete,
                KeyCode::Tab,
                KeyCode::BackTab,
                KeyCode::Enter,
                KeyCode::Esc,
                KeyCode::PageUp,
                KeyCode::PageDown,
                KeyCode::F(1),
                KeyCode::F(2),
                KeyCode::F(3),
                KeyCode::Char('中'),
                KeyCode::Char('日'),
                KeyCode::Char('🙂'),
                KeyCode::Char('\u{301}'),
            ]
            .into_iter()
            .map(|c| KeyEvent::new(c, KeyModifiers::NONE)),
        );
        keys.extend(
            ['u', 's', 'c'].map(|c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)),
        );
        let mut release = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        keys.push(release);
        keys
    }
    fn state(base: &App, kind: usize, agent: Agent) -> App {
        let mut app = App::new(
            base.session.store.clone(),
            agent,
            base.environment.paths.clone(),
        )
        .unwrap();
        app.session.original = base.session.original.clone();
        app.session.baseline = base.session.baseline.clone();
        app.index = 0;
        let rows = app.rows();
        let official = rows
            .iter()
            .find(|(m, _)| *m == Mode::Official)
            .unwrap()
            .1
            .id
            .clone();
        match kind {
            0 => {}
            1 => {
                key(&mut app, KeyCode::Char('n'));
            }
            2 | 3 => {
                key(&mut app, KeyCode::Char('n'));
                key(
                    &mut app,
                    if kind == 2 {
                        KeyCode::Home
                    } else {
                        KeyCode::End
                    },
                );
                key(&mut app, KeyCode::Enter);
            }
            4 => {
                app.index = 1;
                key(&mut app, KeyCode::Char('e'));
            }
            5 => {
                key(&mut app, KeyCode::Char('r'));
            }
            6 => {
                key(&mut app, KeyCode::Char('p'));
            }
            7 => {
                key(&mut app, KeyCode::Char('/'));
            }
            8 => app.confirm = Some(Confirm::Delete(official, Mode::Official)),
            9 => {
                key(&mut app, KeyCode::Char('r'));
                key(&mut app, KeyCode::Char('中'));
                key(&mut app, KeyCode::Esc);
            }
            10 => app.confirm = Some(Confirm::Repair),
            11 => app.confirm = Some(Confirm::Launch),
            12 | 13 => {
                app.agent = Agent::Codex;
                app.ask_login(
                    base.session.original.codex.library.official_accounts[0]
                        .id
                        .clone(),
                );
                if kind == 13 {
                    key(&mut app, KeyCode::Char('p'));
                }
            }
            14 => app.help = true,
            15..=24 => environment_tests::set_state(&mut app, base, kind),
            _ => unreachable!(),
        }
        app
    }
    fn render_checked(app: &App, w: u16, h: u16) {
        assert_inputs(app);
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| view::draw(f, app)).unwrap();
        assert_inputs(app);
    }
    #[test]
    fn every_supported_key_and_seeded_sequences_in_every_ui_state_keep_safe_cursor_and_render() {
        let (dir, mut base) = fixture();
        environment_tests::configure_fixture(&mut base, dir.path());
        // Synthetic route keeps broad state/render tests independent of platform proxy APIs.
        base.session.original.codex.library.official_proxy_url =
            Some("http://proxy.example.test:8080".into());
        let keys = keys();
        let sizes = [(120, 32), (80, 24), (60, 20), (20, 8), (1, 1), (0, 0)];
        for agent in Agent::ALL {
            for kind in 0..25 {
                for (i, key) in keys.iter().enumerate() {
                    let mut app = state(&base, kind, agent);
                    let _ = app.key(*key);
                    let (w, h) = sizes[i % sizes.len()];
                    render_checked(&app, w, h);
                }
                for seed in 1..=4u64 {
                    let mut app = state(&base, kind, agent);
                    let mut rng = seed * 701 + kind as u64;
                    for step in 0..96 {
                        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                        let _ = app.key(keys[(rng >> 32) as usize % keys.len()]);
                        let (w, h) = sizes[step % sizes.len()];
                        render_checked(&app, w, h);
                    }
                }
            }
        }
    }
    #[test]
    fn renderer_and_editing_defend_against_stale_or_nonboundary_cursors() {
        let (_dir, mut app) = fixture();
        app.begin(FormKind::New(None)).unwrap();
        app.form.as_mut().unwrap().inputs[0] = Input {
            label: "名称",
            value: "官方账号".into(),
            cursor: 13,
        };
        grid(&app, 80, 24);
        key(&mut app, KeyCode::Left);
        assert_inputs(&app);
        app.form.as_mut().unwrap().inputs[0].cursor = 1;
        grid(&app, 60, 20);
        key(&mut app, KeyCode::Delete);
        assert_inputs(&app);
        app.form.as_mut().unwrap().inputs[0].cursor = usize::MAX;
        grid(&app, 20, 8);
        key(&mut app, KeyCode::Backspace);
        assert_inputs(&app);
    }
    #[test]
    #[ignore = "subprocess-only panic probe; invoked by the Unix PTY recovery test"]
    fn terminal_panic_probe() {
        let _guard = TerminalGuard::enter().unwrap();
        panic!("example terminal panic recovery probe");
    }
    #[test]
    #[cfg(unix)]
    fn panic_restores_real_pty_before_printing_the_panic() {
        let script = r#"
import os,pty,termios,fcntl,subprocess,select,time,sys,signal
signal.alarm(25)
master,slave=pty.openpty()
os.set_blocking(master,False)
def setup():
 os.setsid()
 fcntl.ioctl(slave,termios.TIOCSCTTY,0)
# Keep the session leader alive while inspecting attributes: macOS revokes its
# PTY when the leader exits, even if the outer process still holds the slave.
runner=r'''
import subprocess,sys,termios
before=termios.tcgetattr(0)
probe=subprocess.run([sys.argv[1],'--exact','interactive::tests::terminal_panic_probe','--ignored','--nocapture'],timeout=10)
assert probe.returncode!=0
after=termios.tcgetattr(0)
if sys.platform=='darwin':
 # XNU sets PENDIN when ICANON is restored; it is transient driver state.
 before[3]&=~termios.PENDIN
 after[3]&=~termios.PENDIN
assert after==before,(before,after)
print('ACS_PANIC_RECOVERY_OK',flush=True)
'''
child=subprocess.Popen([sys.executable,'-c',runner,sys.argv[1]],stdin=slave,stdout=slave,stderr=slave,preexec_fn=setup)
out=b''
deadline=time.monotonic()+15
while time.monotonic()<deadline:
 if select.select([master],[],[],.1)[0]:
  try: chunk=os.read(master,65536)
  except BlockingIOError: continue
  except OSError: break
  if not chunk: break
  out+=chunk
 if child.poll() is not None:
  drain_until=time.monotonic()+1
  while time.monotonic()<drain_until and select.select([master],[],[],.1)[0]:
   try: chunk=os.read(master,65536)
   except OSError: break
   if not chunk: break
   out+=chunk
  break
child.wait(timeout=5)
assert child.returncode==0,out.decode(errors='replace')
assert b'ACS_PANIC_RECOVERY_OK' in out
marker=out.index(b'example terminal panic recovery probe')
assert out.index(b'\x1b[?1049l')<marker
assert out.index(b'\x1b[?25h')<marker
os.close(master);os.close(slave)
"#;
        let result = std::process::Command::new("python3")
            .arg("-c")
            .arg(script)
            .arg(std::env::current_exe().unwrap())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
