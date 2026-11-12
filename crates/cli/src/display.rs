use acs_core::{Agent, Draft, Mode};
use std::io::{self, IsTerminal};
use unicode_width::UnicodeWidthStr;

pub struct Row {
    pub cells: Vec<String>,
    pub current: bool,
}
// Prevent control characters in user-controlled names/URLs from escaping the status row.
fn cell(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
pub fn rows(draft: &Draft, only: Option<Agent>) -> Vec<Row> {
    let mut rows = Vec::new();
    for agent in Agent::ALL {
        if only.is_some_and(|a| a != agent) {
            continue;
        }
        let d = draft.agent(agent);
        for mode in [Mode::Official, Mode::Custom] {
            for p in d.library.profiles(mode) {
                let current = d.mode == mode && d.library.selection(mode) == p.id;
                let f = d.custom_fields.get(&p.id);
                let detail = match mode {
                    Mode::Official => p.email.as_deref().or(p.account_name.as_deref()).unwrap_or(
                        if p.official_auth.is_some() {
                            "已保存本地凭据"
                        } else {
                            "尚未登录"
                        },
                    ),
                    Mode::Custom => f.map(|f| f.base_url.as_str()).unwrap_or("未填写地址"),
                };
                let model = if mode == Mode::Official {
                    d.library.official_model.as_deref().unwrap_or("默认")
                } else {
                    f.map(|f| f.model.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or("默认")
                };
                let proxy = if mode == Mode::Official {
                    d.library.official_proxy_url.as_deref()
                } else {
                    f.map(|f| f.proxy_url.as_str())
                }
                .filter(|s| !s.is_empty())
                .unwrap_or("直连");
                rows.push(Row {
                    current,
                    cells: vec![
                        agent.label().into(),
                        if current { "*".into() } else { "".into() },
                        p.id.clone(),
                        p.name.as_deref().unwrap_or("未命名配置").into(),
                        mode.label().into(),
                        detail.into(),
                        model.into(),
                        proxy.into(),
                    ]
                    .into_iter()
                    .map(|s| cell(&s))
                    .collect(),
                });
            }
        }
    }
    rows
}
pub fn aligned(rows: &[Row]) -> (String, Vec<String>) {
    let headers = [
        "客户端",
        "当前",
        "ID",
        "名称",
        "类型",
        "地址 / 账号",
        "模型",
        "代理",
    ];
    let widths: Vec<_> = (0..headers.len())
        .map(|i| {
            rows.iter()
                .map(|r| r.cells[i].width())
                .chain([headers[i].width()])
                .max()
                .unwrap()
        })
        .collect();
    let format = |cells: Vec<&str>| {
        cells
            .iter()
            .enumerate()
            .map(|(i, s)| format!("{s}{}", " ".repeat(widths[i] - s.width())))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_owned()
    };
    (
        format(headers.to_vec()),
        rows.iter()
            .map(|r| format(r.cells.iter().map(String::as_str).collect()))
            .collect(),
    )
}
pub fn list(draft: &Draft, only: Option<Agent>) {
    let rows = rows(draft, only);
    let (header, lines) = aligned(&rows);
    println!("{header}");
    let color = io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    for (row, line) in rows.iter().zip(lines) {
        if color && row.current {
            println!("\x1b[32m{line}\x1b[0m");
        } else {
            println!("{line}");
        }
    }
    if rows.is_empty() {
        println!("（没有配置，使用 acs add 新增）");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aligns_wide_characters_and_sanitizes_controls() {
        assert_eq!(cell("a\x1b\n中"), "a  中");
        let row = |name: &str| Row {
            current: false,
            cells: vec![
                "Codex",
                "*",
                "12345678",
                name,
                "官方账号",
                "未登录",
                "默认",
                "直连",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        };
        let (_, lines) = aligned(&[row("中文名称"), row("English")]);
        let positions: Vec<_> = lines
            .iter()
            .map(|s| s[..s.find("官方账号").unwrap()].width())
            .collect();
        assert_eq!(positions[0], positions[1]);
    }
}
