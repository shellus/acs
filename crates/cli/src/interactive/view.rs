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
            "acs {}  ·  配置与状态     * 当前生效   > 高亮查看",
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
    } else if app.form.is_some() {
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
    let hint = if app.confirm.is_some() {
        "y 确认 · n/Esc 取消"
    } else if app.help {
        "Esc/? 关闭帮助"
    } else if app.form.is_some() {
        "↑↓/Tab 字段 · ←→ 光标 · F2 保存 · Esc 取消 · Ctrl+U 清空"
    } else if app.filtering {
        "/ 过滤输入 · Enter 应用 · Esc 返回"
    } else if area.width >= 110 {
        "↑↓/jk选择 Tab客户端 Enter当前 e编辑 n新增 c复制 r改名 d删 l登 s启 p代理 /过滤 ?帮助 q退出"
    } else {
        "↑↓jk Tab切换 Enter当前 · e n c r d l s p / · ?帮助 q退出"
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
        items.push(ListItem::new(truncate(
            &s,
            area.width.saturating_sub(4) as usize,
        )));
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
    if let Some(form) = &app.form {
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
                } else {
                    "尚未登录；Codex 可按 l 登录".into()
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
    let block = panel("编辑表单 · F2 保存 / Esc 取消");
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
        let prefix = format!(
            "{} {}：",
            if i == form.focus { ">" } else { " " },
            input.label
        );
        let width = inner.width.saturating_sub(prefix.width() as u16) as usize;
        let before = &input.value[..input.cursor];
        let mut from = 0;
        if i == form.focus {
            while input.value[from..input.cursor].width() >= width.saturating_sub(1).max(1) {
                let Some(c) = input.value[from..input.cursor].chars().next() else {
                    break;
                };
                from += c.len_utf8();
            }
        }
        let value = truncate(&input.value[from..], width);
        f.render_widget(
            Paragraph::new(format!("{prefix}{value}")).style(if i == form.focus {
                selected()
            } else {
                Style::default()
            }),
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            },
        );
        if i == form.focus && width > 0 {
            let x = inner.x + prefix.width() as u16 + before[from..].width() as u16;
            f.set_cursor_position((x.min(inner.right().saturating_sub(1)), y));
        }
    }
    if !form.error.is_empty() {
        f.render_widget(
            Paragraph::new(format!("保存失败：{}", form.error))
                .style(danger())
                .wrap(Wrap { trim: false }),
            Rect {
                x: inner.x,
                y: inner.bottom().saturating_sub(4),
                width: inner.width,
                height: 4.min(inner.height),
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
            "直接输入 · 空值允许保存前校验 · API Key 与 GUI 一样明文显示".into(),
            Style::default(),
        );
    }
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
    let help="查看与导航\n↑↓ / j k 选择配置；Tab / ←→ 切换客户端\nPgUp / PgDn 滚动完整详情；/ 过滤；R 重新读取\n\n配置操作（成功即保存）\nEnter 设为当前；e 编辑；n 新增；c 复制\nr 改名；d 删除（y 确认）；p 官方代理\nl Codex 官方登录（临时离开全屏，复制完整链接）\ns 显式启动；h 修复 Codex 历史（备份并确认）\n\n表单\n↑↓ / Tab 切换字段；←→ 光标；Home / End\nCtrl+U 清空；F2 / Ctrl+S 保存；Esc 取消\n仅修改过的表单需要确认放弃；确认框 Esc 继续编辑\n\nq / Esc 退出；Ctrl+C 从任意界面退出并恢复终端\n? / Esc 关闭帮助";
    f.render_widget(
        Paragraph::new(help)
            .wrap(Wrap { trim: false })
            .block(panel("按键帮助 · Esc 返回").border_style(accent())),
        r,
    );
}
