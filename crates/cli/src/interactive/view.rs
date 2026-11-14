use super::*;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

fn accent() -> Style {
    if std::env::var_os("NO_COLOR").is_some() {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    }
}
fn danger() -> Style {
    if std::env::var_os("NO_COLOR").is_some() {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::LightRed)
    }
}
fn selected() -> Style {
    if std::env::var_os("NO_COLOR").is_some() {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    }
}
pub(super) fn truncate(s: &str, width: usize) -> String {
    let s: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if s.width() <= width {
        return s;
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}
fn line(f: &mut Frame, area: Rect, text: String, style: Style) {
    f.render_widget(
        Paragraph::new(truncate(&text, area.width as usize)).style(style),
        area,
    );
}
fn panel(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default())
}
pub(super) fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    if area.width < 20 || area.height < 8 {
        f.render_widget(Paragraph::new("窗口过小；请放大。q 退出"), area);
        return;
    }
    if app.environment_open {
        environment(f, app);
        if app.help {
            environment_help(f, area);
        }
        return;
    }
    let [head, body, keys, status] = Layout::vertical([
        Constraint::Length(if area.width >= 100 { 2 } else { 3 }),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    line(
        f,
        Rect { height: 1, ..head },
        format!(
            "acs {} · 配置与状态 · v 环境   * 当前   > 查看",
            env!("CARGO_PKG_VERSION")
        ),
        accent(),
    );
    for (i, agent) in Agent::ALL.iter().enumerate() {
        let d = app.session.original.agent(*agent);
        let name = d
            .library
            .selected(d.mode)
            .and_then(|p| p.name.as_deref())
            .unwrap_or("外部配置 / 尚未入库");
        let tab_area = if area.width >= 100 {
            Rect {
                x: head.x + i as u16 * (head.width / 2),
                y: head.y + 1,
                width: head.width / 2,
                height: 1,
            }
        } else {
            Rect {
                x: head.x,
                y: head.y + i as u16 + 1,
                width: head.width,
                height: 1,
            }
        };
        line(
            f,
            tab_area,
            format!(
                "{} [{}]  {} · {}",
                if app.agent == *agent { ">" } else { " " },
                agent.label(),
                d.mode.label(),
                name
            ),
            if app.agent == *agent {
                accent()
            } else {
                Style::default()
            },
        );
    }
    if body.width >= 80 {
        let [left, right] = Layout::horizontal([
            Constraint::Length((body.width / 3).clamp(26, 38)),
            Constraint::Min(1),
        ])
        .areas(body);
        list(f, left, app);
        detail(f, right, app);
    } else if app.form.is_some() && !app.type_picker {
        detail(f, body, app);
    } else {
        let [list_area, detail_area] = Layout::vertical([
            Constraint::Length((body.height / 3).clamp(4, 8)),
            Constraint::Min(1),
        ])
        .areas(body);
        list(f, list_area, app);
        detail(f, detail_area, app);
    }
    let hint = if let Some(login) = &app.login {
        if login.editing {
            "←→ 光标 · F2 保存代理 · Enter 保存并登录 · Esc 返回确认"
        } else {
            "p 修改官方代理 · y/Enter 登录 · n/Esc 稍后"
        }
    } else if app.type_picker {
        "←→/↑↓/Tab 或 1/2 选择类型 · Enter 继续 · Esc 取消"
    } else if app.confirm.is_some() {
        "y 确认 · n/Esc 取消"
    } else if app.help {
        "Esc/? 关闭帮助"
    } else if app.form.is_some() {
        "↑↓/Tab 字段 · ←→ 光标 · F2 保存 · Esc 取消 · Ctrl+U 清空"
    } else if app.filtering {
        "/ 过滤输入 · Enter 应用 · Esc 返回"
    } else if app.agent == Agent::Claude {
        "终端运行 claude /login 登录 · ↑↓ Tab Enter e n c r d s p / ? q"
    } else if app
        .selected()
        .is_some_and(|(m, p)| m == Mode::Official && p.official_auth.is_none())
    {
        "l 登录/设置代理 · ↑↓ Tab Enter e n c r d s p / ? q"
    } else if area.width >= 110 {
        "↑↓/jk选择 Tab客户端 Enter当前 e编辑 n新增 c复制 r改名 d删 l登 s启 p代理 /过滤 ?帮助 q退出"
    } else {
        "↑↓jk Tab切换 v环境 Enter当前 · e n c r d l s p / · ?帮助 q退出"
    };
    line(f, keys, hint.into(), accent());
    line(
        f,
        status,
        if app.filtering {
            format!("过滤：{}", app.filter)
        } else {
            app.message.clone()
        },
        if app.error {
            danger()
        } else {
            Style::default()
        },
    );
    if app.help {
        help(f, area);
    }
    if app.type_picker {
        type_picker(f, area, app);
    }
    if let Some(login) = &app.login {
        login_prompt(f, area, app, login);
    }
    if let Some(c) = &app.confirm {
        let text = match c {
            Confirm::Delete(id, _) => format!("删除配置 {id}？\n成功后立即保存，不能撤销。"),
            Confirm::Discard => {
                "放弃本次未保存修改？\ny 放弃并返回；Esc 取消确认，继续编辑。".into()
            }
            Confirm::Repair => {
                "修复 Codex 历史对话？\n请先关闭写入历史的 Codex；修改前自动备份。".into()
            }
            Confirm::Launch => {
                if cfg!(target_os = "linux") {
                    "启动当前已保存的 CLI？\n1 / Enter 启动 CLI；Esc 返回。".into()
                } else {
                    "启动当前已保存配置？\n1 / Enter CLI；2 桌面版；Esc 返回。".into()
                }
            }
        };
        modal(f, area, "确认操作", &text);
    }
}
/// Wrap before slicing, so scrolling can reach every character even on narrow terminals.
pub(super) fn environment_lines(lines: Vec<String>, width: u16) -> Vec<String> {
    let width = usize::from(width.max(1));
    let mut out = Vec::new();
    for line in lines {
        for part in line.split('\n') {
            let mut row = String::new();
            let mut used = 0;
            for c in part.chars().map(|c| if c.is_control() { ' ' } else { c }) {
                let cells = c.width().unwrap_or(0);
                if used + cells > width && !row.is_empty() {
                    out.push(std::mem::take(&mut row));
                    used = 0;
                }
                row.push(c);
                used += cells;
            }
            out.push(row);
        }
    }
    out
}
fn environment(f: &mut Frame, app: &App) {
    let page = &app.environment;
    let [head, body, activity, keys] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(2),
        Constraint::Length(2),
    ])
    .areas(f.area());
    f.render_widget(
        Paragraph::new(format!(
            "acs {} · 环境\nCodex / Claude / [环境] · Tab 切换",
            env!("CARGO_PKG_VERSION")
        ))
        .style(accent()),
        head,
    );
    let block = panel(page.title());
    let inner = block.inner(body);
    let lines = environment_lines(page.lines(), inner.width);
    let limit = lines.len().saturating_sub(inner.height as usize);
    page.scroll_limit.set(limit);
    let offset = page.scroll.min(limit);
    let block = block.title_bottom(format!(
        "行 {}–{} / {} · ↑↓ PgUp/PgDn Home/End",
        offset + 1,
        (offset + inner.height as usize).min(lines.len()),
        lines.len()
    ));
    f.render_widget(block, body);
    f.render_widget(
        Paragraph::new(
            lines
                .into_iter()
                .skip(offset)
                .take(inner.height as usize)
                .map(Line::raw)
                .collect::<Vec<_>>(),
        ),
        inner,
    );
    if page.busy() {
        let elapsed = page.started.elapsed();
        let pulse = ["·", "··", "···"][(elapsed.as_millis() / 400 % 3) as usize];
        let stage = page
            .progress
            .as_ref()
            .map(|p| format!("{} {}/{}", p.phase, p.completed, p.total))
            .unwrap_or_else(|| "等待外部命令 / 读取环境".into());
        f.render_widget(
            Paragraph::new(format!(
                "{stage} · 已等待 {} 秒 {pulse}\n任务进行中；Ctrl+C 请求完成后退出",
                elapsed.as_secs()
            ))
            .style(accent()),
            activity,
        );
    } else if page.state == environment::State::Confirm {
        f.render_widget(Paragraph::new("应用上方已展示的计划？y 确认 · n/Esc 取消\n包含 Agent 安装；基线变化将拒绝并重新读取。").style(accent()), activity);
    } else {
        f.render_widget(
            Paragraph::new(if page.state == environment::State::Failed {
                "失败原因可在正文完整滚动查看；R 重试，d 诊断"
            } else if page.plan.as_ref().is_some_and(|p| !p.has_changes()) {
                "环境已同步；R 刷新，d 诊断"
            } else {
                "来源、版本与变更均来自核心计划；a 查看应用确认"
            }),
            activity,
        );
    }
    let hint = if page.busy() {
        "↑↓/PgUp/PgDn 滚动 · ? 帮助\nCtrl+C 请求任务完成后退出"
    } else if page.state == environment::State::Confirm {
        "y 确认 · n/Esc 取消 · ? 帮助\n↑↓/PgUp/PgDn 滚动 · Home/End"
    } else {
        "R 刷新  a 应用  d 诊断  ? 帮助\n↑↓/PgUp/PgDn 滚动 · Esc/q 返回"
    };
    f.render_widget(Paragraph::new(hint).style(accent()), keys);
}
fn environment_help(f: &mut Frame, area: Rect) {
    let r = centered(area, 74, 17);
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new("环境页按键\nR / r 重新同步来源、读取安装版本与漂移\na 查看应用确认；y 应用展示的计划；n / Esc 取消\nd 诊断依赖、来源、安装与受管状态\n↑↓ / j k 逐行滚动；PgUp / PgDn 翻页；Home / End\nTab / ←→ 切换账号与环境页；Esc / q 返回账号\n\n加载或应用时仍可滚动与查看帮助。\n阶段计数来自核心，等待时间持续更新。\n事务不能中断；Ctrl+C 请求任务完成后退出。\n计划变化会拒绝旧计划，刷新后需要再次确认。\n\n? / Esc 关闭帮助").wrap(Wrap { trim: false }).block(panel("环境帮助")), r);
}
fn list(f: &mut Frame, area: Rect, app: &App) {
    let rows = app.rows();
    let d = app.session.original.agent(app.agent);
    let mut items = Vec::new();
    let mut previous = None;
    let mut at = None;
    for (index, (mode, p)) in rows.iter().enumerate() {
        if previous != Some(*mode) {
            items.push(ListItem::new(Line::styled(
                format!("── {} ──", mode.label()),
                accent(),
            )));
            previous = Some(*mode);
        }
        if index == app.index {
            at = Some(items.len());
        }
        let current = d.mode == *mode && d.library.selection(*mode) == p.id;
        let s = format!(
            "{} {}",
            if current { "*" } else { " " },
            p.name.as_deref().unwrap_or("未命名")
        );
        let width = area.width.saturating_sub(4) as usize;
        let secondary = secondary(app, *mode, p);
        let remaining = width.saturating_sub(s.width() + 3);
        let spans = if remaining >= 6 && !secondary.is_empty() {
            vec![
                Span::raw(s),
                Span::styled(
                    format!(" · {}", truncate(&secondary, remaining)),
                    Style::default().add_modifier(Modifier::DIM),
                ),
            ]
        } else {
            vec![Span::raw(truncate(&s, width))]
        };
        items.push(ListItem::new(Line::from(spans)));
    }
    if items.is_empty() {
        items.push(ListItem::new("没有配置；n 新增"));
    }
    let mut state = ListState::default().with_selected(at);
    f.render_stateful_widget(
        List::new(items)
            .block(panel("配置 · / 过滤"))
            .highlight_symbol(">")
            .highlight_style(selected()),
        area,
        &mut state,
    );
}
fn detail(f: &mut Frame, area: Rect, app: &App) {
    if let Some(form) = app.form.as_ref().filter(|_| !app.type_picker) {
        return form_view(f, area, form);
    }
    let Some((mode, p)) = app.selected() else {
        f.render_widget(
            Paragraph::new("n 新增配置\nTab 切换客户端\n? 查看全部按键").block(panel("详情")),
            area,
        );
        return;
    };
    let d = app.session.original.agent(app.agent);
    let current = d.mode == mode && d.library.selection(mode) == p.id;
    let mut lines = vec![Line::styled(
        format!(
            "{}  ·  {}",
            p.name.as_deref().unwrap_or("未命名"),
            if current { "当前生效" } else { "仅查看" }
        ),
        accent(),
    )];
    let mut values = vec![("ID", p.id.clone()), ("类型", mode.label().into())];
    if mode == Mode::Custom {
        if let Some(v) = d.custom_fields.get(&p.id) {
            values.extend([
                ("提供者", v.provider_name.clone()),
                ("API 地址", v.base_url.clone()),
                ("API Key", v.api_key.clone()),
                ("模型", v.model.clone()),
                ("思考层级", v.effort.clone()),
                (
                    "配置代理",
                    if v.proxy_url.is_empty() {
                        "直连 / 原有环境变量仍可能生效".into()
                    } else {
                        v.proxy_url.clone()
                    },
                ),
            ]);
        }
        values.push(("登录状态", "自定义 API Key；未验证联网".into()));
    } else {
        values.extend([
            (
                "账号",
                p.email
                    .or(p.account_name)
                    .unwrap_or_else(|| "尚未绑定账号".into()),
            ),
            (
                "登录状态",
                if p.official_auth.is_some() {
                    "已保存本地凭据；有效性由官方客户端验证".into()
                } else if app.agent == Agent::Codex {
                    "未登录；按 l 打开登录确认，可先设置官方代理".into()
                } else {
                    "未登录；在终端运行 claude /login（桌面版登录不共享给命令行）".into()
                },
            ),
            (
                "模型",
                d.library
                    .official_model
                    .clone()
                    .unwrap_or_else(|| "默认".into()),
            ),
            (
                "思考层级",
                d.library
                    .official_effort
                    .clone()
                    .unwrap_or_else(|| "默认".into()),
            ),
            (
                "官方代理",
                d.library
                    .official_proxy_url
                    .clone()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "直连 / 原有环境变量仍可能生效".into()),
            ),
        ]);
    }
    let active = d.library.proxy(d.mode);
    values.push((
        "当前生效代理",
        if active.is_empty() {
            "直连 / 继承现有环境".into()
        } else {
            active.into()
        },
    ));
    for (label, value) in values {
        lines.push(Line::from(vec![
            Span::styled(format!("{label}："), accent()),
            Span::raw(if value.is_empty() {
                "（空）".into()
            } else {
                value
            }),
        ]));
    }
    for hint in app.session.profile_hints(app.agent, mode, &p.id) {
        lines.push(Line::styled(format!("提示：{hint}"), danger()));
    }
    if let Some(snapshot) = p.model_providers_toml.filter(|s| !s.is_empty()) {
        lines.push(Line::styled("custom 完整快照（PgDn 查看）", accent()));
        lines.extend(snapshot.lines().map(|s| Line::raw(s.to_owned())));
    }
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((app.detail_scroll, 0))
            .block(panel("详情 · PgUp/PgDn 滚动完整值")),
        area,
    );
}
fn form_view(f: &mut Frame, area: Rect, form: &Form) {
    let title = if matches!(form.kind, FormKind::New(_)) && form.mode == Mode::Official {
        "新增官方账号 · 仅名称 · F2 保存 / Esc 取消"
    } else {
        "编辑表单 · F2 保存 / Esc 取消"
    };
    let block = panel(title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let available = inner
        .height
        .saturating_sub(if form.error.is_empty() { 1 } else { 4 }) as usize;
    let start = form.focus.saturating_sub(available.saturating_sub(1));
    for (i, input) in form
        .inputs
        .iter()
        .take(form.count())
        .enumerate()
        .skip(start)
        .take(available)
    {
        let y = inner.y + (i - start) as u16;
        input_row(
            f,
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            },
            input,
            i == form.focus,
        );
    }
    if !form.error.is_empty() {
        let error_height = 4.min(inner.height);
        f.render_widget(
            Paragraph::new(format!("保存失败：{}", form.error))
                .style(danger())
                .wrap(Wrap { trim: false }),
            Rect {
                x: inner.x,
                y: inner.bottom() - error_height,
                width: inner.width,
                height: error_height,
            },
        );
    } else if inner.height > 0 {
        line(
            f,
            Rect {
                x: inner.x,
                y: inner.bottom() - 1,
                width: inner.width,
                height: 1,
            },
            if matches!(form.kind, FormKind::New(_)) && form.mode == Mode::Official {
                "仅填写名称；保存后显示该客户端的登录方式"
            } else {
                "直接输入 · API Key 与 GUI 一样明文显示"
            }
            .into(),
            Style::default(),
        );
    }
}
fn input_row(f: &mut Frame, area: Rect, input: &Input, focused: bool) {
    let cursor = input.position();
    let prefix = format!("{} {}：", if focused { ">" } else { " " }, input.label);
    let width = area.width.saturating_sub(prefix.width() as u16) as usize;
    let before = &input.value[..cursor];
    let mut from = 0;
    if focused {
        while input.value[from..cursor].width() >= width.saturating_sub(1).max(1) {
            let Some(c) = input.value[from..cursor].chars().next() else {
                break;
            };
            from += c.len_utf8();
        }
    }
    let value = truncate(&input.value[from..], width);
    f.render_widget(
        Paragraph::new(format!("{prefix}{value}")).style(if focused {
            selected()
        } else {
            Style::default()
        }),
        area,
    );
    if focused && width > 0 && area.height > 0 {
        let x = area.x + prefix.width() as u16 + before[from..].width() as u16;
        f.set_cursor_position((x.min(area.right().saturating_sub(1)), area.y));
    }
}
fn secondary(app: &App, mode: Mode, p: &acs_core::config::Profile) -> String {
    if mode == Mode::Official {
        return if p.official_auth.is_none() {
            "未登录".into()
        } else {
            p.email
                .clone()
                .or(p.account_name.clone())
                .unwrap_or_else(|| "已保存凭据".into())
        };
    }
    app.session
        .original
        .agent(app.agent)
        .custom_fields
        .get(&p.id)
        .map(|f| {
            url::Url::parse(&f.base_url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_owned))
                .unwrap_or_else(|| f.model.clone())
        })
        .unwrap_or_default()
}
fn type_picker(f: &mut Frame, area: Rect, app: &App) {
    let Some(form) = &app.form else {
        return;
    };
    let r = centered(area, 68, 9);
    f.render_widget(Clear, r);
    let title = format!("新增 {} · 先选择配置类型", app.agent.label());
    let block = panel(&title).border_style(accent());
    let inner = block.inner(r);
    f.render_widget(block, r);
    let description = if app.agent == Agent::Codex {
        "保存后可立即用 OAuth 登录"
    } else {
        "终端运行 claude /login 登录"
    };
    for (i, mode) in [Mode::Official, Mode::Custom].into_iter().enumerate() {
        let active = form.mode == mode;
        line(
            f,
            Rect {
                x: inner.x,
                y: inner.y + i as u16 * 2,
                width: inner.width,
                height: 1,
            },
            format!(
                "{} [{}] {} {} · {}",
                if active { ">" } else { " " },
                i + 1,
                if active { "●" } else { "○" },
                mode.label(),
                if mode == Mode::Official {
                    description
                } else {
                    "填写地址、API Key 和模型"
                }
            ),
            if active { selected() } else { Style::default() },
        );
    }
    if inner.height >= 6 {
        line(
            f,
            Rect {
                x: inner.x,
                y: inner.bottom() - 2,
                width: inner.width,
                height: 1,
            },
            "←→ / ↑↓ / Tab 或 1 / 2 选择；Enter 继续".into(),
            accent(),
        );
        line(
            f,
            Rect {
                x: inner.x,
                y: inner.bottom() - 1,
                width: inner.width,
                height: 1,
            },
            "Esc 取消；此处是二选一控件，不是文本输入".into(),
            Style::default(),
        );
    }
}
fn login_prompt(f: &mut Frame, area: Rect, app: &App, prompt: &LoginPrompt) {
    let r = centered(area, 76, 17);
    f.render_widget(Clear, r);
    let block = panel("Codex 官方账号 · 是否立即登录？").border_style(accent());
    let inner = block.inner(r);
    f.render_widget(block, r);
    let [route, warning, proxy, error, keys] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .areas(inner);
    let account = app
        .session
        .original
        .codex
        .library
        .official_accounts
        .iter()
        .find(|p| p.id == prompt.id)
        .and_then(|p| p.name.as_deref())
        .unwrap_or("官方账号");
    match &prompt.route {
        Ok(resolved) => {
            f.render_widget(
                Paragraph::new(format!("账号：{account}\n{}", resolved.summary()))
                    .wrap(Wrap { trim: false }),
                route,
            );
            if let Some(text) = resolved.warning() {
                f.render_widget(
                    Paragraph::new(text)
                        .style(danger())
                        .wrap(Wrap { trim: false }),
                    warning,
                );
            }
        }
        Err(e) => {
            f.render_widget(
                Paragraph::new(format!("{e:#}"))
                    .style(danger())
                    .wrap(Wrap { trim: false }),
                route,
            );
        }
    }
    input_row(
        f,
        Rect { height: 1, ..proxy },
        &prompt.proxy,
        prompt.editing,
    );
    f.render_widget(
        Paragraph::new("留空：继承环境 / 系统代理；若无可用代理则直连").wrap(Wrap { trim: false }),
        Rect {
            y: proxy.y + 1,
            height: 1,
            ..proxy
        },
    );
    f.render_widget(
        Paragraph::new(&*prompt.error)
            .style(danger())
            .wrap(Wrap { trim: false }),
        error,
    );
    f.render_widget(
        Paragraph::new(if prompt.editing {
            "Enter 保存代理并登录 · F2 仅保存\nEsc 放弃代理编辑，返回确认"
        } else {
            "p 就地修改官方代理 · y / Enter 登录\nn / Esc 稍后；未登录账号可随时按 l"
        })
        .wrap(Wrap { trim: false })
        .style(accent()),
        keys,
    );
}
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}
fn modal(f: &mut Frame, area: Rect, title: &str, text: &str) {
    let r = centered(area, 66, 8);
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(format!("{text}\n\ny 确认 · n / Esc 取消"))
            .wrap(Wrap { trim: false })
            .block(panel(title).border_style(accent())),
        r,
    );
}
pub(super) fn confirmation(f: &mut Frame, title: &str) {
    modal(f, f.area(), "确认操作", title);
}
fn help(f: &mut Frame, area: Rect) {
    let r = centered(area, 76, 22);
    f.render_widget(Clear, r);
    let help="查看与导航\n↑↓ / j k 选择配置；Tab / ←→ 切换账号与环境\nv / F3 进入环境页\nPgUp / PgDn 滚动完整详情；/ 过滤；R 重新读取\n\n配置操作（成功即保存）\nEnter 设为当前；e 编辑；n 先选类型再新增；c 复制\nr 改名；d 删除（y 确认）；p 官方代理\nl Codex 登录确认；确认处 p 就地保存代理后登录\ns 显式启动；h 修复 Codex 历史（备份并确认）\n\n表单\n↑↓ / Tab 切换字段；←→ 光标；Home / End\nCtrl+U 清空；F2 / Ctrl+S 保存；Esc 取消\n仅修改过的表单需要确认放弃；确认框 Esc 继续编辑\n\nq / Esc 退出；Ctrl+C 从任意界面退出并恢复终端\n? / Esc 关闭帮助";
    f.render_widget(
        Paragraph::new(help)
            .wrap(Wrap { trim: false })
            .block(panel("按键帮助 · Esc 返回").border_style(accent())),
        r,
    );
}
