use super::*;
use inquire::{Confirm, Password, Select, Text};

fn render() -> inquire::ui::RenderConfig<'static> {
    if std::env::var_os("NO_COLOR").is_some() {
        inquire::ui::RenderConfig::empty()
    } else {
        inquire::ui::RenderConfig::default()
    }
}
fn choose(title: &str, options: Vec<String>) -> Result<Option<usize>> {
    if options.is_empty() {
        eprintln!("没有可选配置，请先新增");
        return Ok(None);
    }
    Ok(Select::new(title, options)
        .with_render_config(render())
        .with_page_size(14)
        .with_help_message("↑↓ 选择 · 输入筛选 · Enter 确认 · Esc 返回")
        .raw_prompt_skippable()?
        .map(|a| a.index))
}
fn text(title: &str, current: &str) -> Result<Option<String>> {
    Ok(Text::new(title)
        .with_initial_value(current)
        .with_render_config(render())
        .prompt_skippable()?)
}
pub fn secret(title: &str) -> Result<Option<String>> {
    Ok(Password::new(title)
        .without_confirmation()
        .with_render_config(render())
        .prompt_skippable()?)
}
pub fn confirmed(title: &str) -> Result<bool> {
    Ok(Confirm::new(title)
        .with_default(false)
        .with_render_config(render())
        .prompt_skippable()?
        .unwrap_or(false))
}
fn agent_arg(a: Agent) -> AgentArg {
    match a {
        Agent::Codex => AgentArg::Codex,
        Agent::Claude => AgentArg::Claude,
    }
}
fn kind(m: Mode) -> Kind {
    match m {
        Mode::Official => Kind::Official,
        Mode::Custom => Kind::Custom,
    }
}
fn pick(store: &Store, selected: Option<AgentArg>, custom: bool) -> Result<Option<String>> {
    let session = Session::open(store.clone())?;
    let rows: Vec<_> = display::rows(&session.original, selected.map(Into::into))
        .into_iter()
        .filter(|r| !custom || r.mode == Mode::Custom)
        .collect();
    let (_, lines) = display::aligned(&rows);
    Ok(choose("选择配置", lines)?.map(|i| rows[i].id.clone()))
}

// Missing arguments are prompted only with both stdin/stdout attached to a terminal.
pub fn dispatch(store: Store, action: Action, selected: Option<AgentArg>) -> Result<()> {
    match action {
        Action::Add {
            agent,
            kind,
            name,
            copy,
            fields,
        } if agent.is_none() || kind.is_none() || name.is_none() || fields.is_empty() => {
            add(store, agent, kind, name, copy, fields)
        }
        Action::Edit { target, fields } => {
            let target = match target {
                Some(t) => t,
                None => match pick(&store, selected, true)? {
                    Some(t) => t,
                    None => return Ok(()),
                },
            };
            if fields.is_empty() {
                edit(store, &target)
            } else {
                execute(
                    store,
                    Action::Edit {
                        target: Some(target),
                        fields,
                    },
                    selected,
                )
            }
        }
        Action::Use { target: None } => {
            if let Some(target) = pick(&store, selected, false)? {
                execute(
                    store,
                    Action::Use {
                        target: Some(target),
                    },
                    selected,
                )?;
            }
            Ok(())
        }
        Action::Rm { target: None, yes } => {
            if let Some(target) = pick(&store, selected, false)? {
                execute(
                    store,
                    Action::Rm {
                        target: Some(target),
                        yes,
                    },
                    selected,
                )?;
            }
            Ok(())
        }
        other => execute(store, other, selected),
    }
}

pub fn menu(store: Store, selected: Option<AgentArg>) -> Result<()> {
    loop {
        let session = Session::open(store.clone())?;
        let rows = display::rows(&session.original, selected.map(Into::into));
        let (header, mut lines) = display::aligned(&rows);
        lines.extend(
            [
                "＋ 新增配置",
                "启动 Codex",
                "启动 Claude",
                "官方代理",
                "修复 Codex 历史对话",
                "退出",
            ]
            .map(str::to_owned),
        );
        let Some(i) = choose(&format!("acs · 配置与状态\n  {header}"), lines)? else {
            return Ok(());
        };
        let result = if i < rows.len() {
            profile_menu(store.clone(), &rows[i].id)
        } else {
            match i - rows.len() {
                0 => add(store.clone(), None, None, None, None, ApiFields::default()),
                1 => launch(store.clone(), Agent::Codex),
                2 => launch(store.clone(), Agent::Claude),
                3 => proxy(store.clone()),
                4 => execute(store.clone(), Action::Repair { yes: false }, None),
                _ => return Ok(()),
            }
        };
        if let Err(e) = result {
            if e.downcast_ref::<inquire::InquireError>()
                .is_some_and(|e| matches!(e, inquire::InquireError::OperationInterrupted))
            {
                return Ok(());
            }
            eprintln!("操作失败：{e:#}");
        }
    }
}

fn profile_menu(store: Store, id: &str) -> Result<()> {
    loop {
        let session = Session::open(store.clone())?;
        let (agent, mode, _) = locate(&session.original, id)?;
        let row = display::rows(&session.original, Some(agent))
            .into_iter()
            .find(|r| r.id == id)
            .context("配置已不存在")?;
        let mut options = vec![
            "设为当前",
            "设为当前并启动",
            "编辑",
            "改名",
            "复制为新配置",
            "删除",
        ];
        if agent == Agent::Codex && mode == Mode::Official {
            options.push("登录");
        }
        options.push("返回");
        let Some(i) = choose(
            &row.cells.join(" · "),
            options.iter().map(|s| (*s).into()).collect(),
        )?
        else {
            return Ok(());
        };
        let result = match options[i] {
            "设为当前" => execute(
                store.clone(),
                Action::Use {
                    target: Some(id.into()),
                },
                None,
            ),
            "设为当前并启动" => {
                execute(
                    store.clone(),
                    Action::Use {
                        target: Some(id.into()),
                    },
                    None,
                )?;
                launch(store.clone(), agent)
            }
            "编辑" => edit(store.clone(), id),
            "改名" => {
                if let Some(name) = text("配置名称", &row.cells[3])? {
                    execute(
                        store.clone(),
                        Action::Mv {
                            target: id.into(),
                            name,
                        },
                        None,
                    )
                } else {
                    Ok(())
                }
            }
            "复制为新配置" => add(
                store.clone(),
                Some(agent_arg(agent)),
                Some(kind(mode)),
                None,
                if mode == Mode::Custom {
                    Some(id.into())
                } else {
                    None
                },
                ApiFields::default(),
            ),
            "删除" => {
                if confirmed("删除该配置并立即保存？")? {
                    execute(
                        store.clone(),
                        Action::Rm {
                            target: Some(id.into()),
                            yes: true,
                        },
                        None,
                    )?;
                    return Ok(());
                }
                Ok(())
            }
            "登录" => execute(store.clone(), Action::Login { target: id.into() }, None),
            _ => return Ok(()),
        };
        if let Err(e) = result {
            eprintln!("操作失败：{e:#}");
        }
    }
}

fn add(
    store: Store,
    agent: Option<AgentArg>,
    mode: Option<Kind>,
    name: Option<String>,
    copy: Option<String>,
    fields: ApiFields,
) -> Result<()> {
    let agent = match agent {
        Some(a) => a,
        None => match choose("客户端", vec!["Codex".into(), "Claude".into()])? {
            Some(0) => AgentArg::Codex,
            Some(_) => AgentArg::Claude,
            None => return Ok(()),
        },
    };
    let mode = match mode {
        Some(m) => m,
        None => match choose("配置类型", vec!["官方账号".into(), "自定义 API".into()])? {
            Some(0) => Kind::Official,
            Some(_) => Kind::Custom,
            None => return Ok(()),
        },
    };
    let name = match name {
        Some(n) => n,
        None => match text("新配置名称", "")? {
            Some(n) => n,
            None => return Ok(()),
        },
    };
    if matches!(mode, Kind::Official) {
        execute(
            store.clone(),
            Action::Add {
                agent: Some(agent),
                kind: Some(mode),
                name: Some(name),
                copy,
                fields,
            },
            None,
        )?;
        if matches!(agent, AgentArg::Codex) && confirmed("立即登录这个 Codex 官方账号？")?
        {
            let s = Session::open(store.clone())?;
            execute(
                store,
                Action::Login {
                    target: s.original.codex.library.selected_official.clone(),
                },
                None,
            )?;
        }
        return Ok(());
    }
    let mut session = Session::open(store)?;
    let mut draft = session.original.clone();
    let a = agent.into();
    let id = prepare_add(&mut draft, a, Mode::Custom, &name, copy.as_deref(), &fields)?;
    form(&mut session, draft, a, &id)
}

fn edit(store: Store, query: &str) -> Result<()> {
    let mut session = Session::open(store)?;
    let (agent, mode, id) = locate(&session.original, query)?;
    if mode != Mode::Custom {
        bail!("官方账号没有 API 字段；请使用改名、登录或官方代理")
    }
    let draft = session.original.clone();
    form(&mut session, draft, agent, &id)
}

fn form(session: &mut Session, mut draft: Draft, agent: Agent, id: &str) -> Result<()> {
    let revision = session.view().revision;
    loop {
        let f = draft
            .agent_mut(agent)
            .custom_fields
            .get_mut(id)
            .context("API 表单缺失")?;
        // GUI currently displays the literal key in its text field; keep that summary behavior.
        let labels = [
            "提供者名称",
            "API 地址",
            "API Key",
            "模型",
            "思考层级",
            "HTTP 代理",
        ];
        let values = [
            &f.provider_name,
            &f.base_url,
            &f.api_key,
            &f.model,
            &f.effort,
            &f.proxy_url,
        ];
        let mut options: Vec<String> = labels
            .iter()
            .zip(values)
            .map(|(l, v)| format!("{l}：{}", if v.is_empty() { "（空）" } else { v }))
            .collect();
        options.extend(["保存".into(), "放弃修改".into()]);
        let Some(i) = choose("编辑自定义 API · 保存前修改仅在本表单内", options)?
        else {
            if confirmed("放弃表单修改？")? {
                return Ok(());
            }
            continue;
        };
        if i == 7 {
            return Ok(());
        }
        if i == 6 {
            match save(session, &revision, &draft, agent) {
                Ok(()) => return Ok(()),
                Err(e) => eprintln!("保存失败：{e:#}\n已保留填写内容，可继续修改或放弃。"),
            }
            continue;
        }
        let slots = [
            &mut f.provider_name,
            &mut f.base_url,
            &mut f.api_key,
            &mut f.model,
            &mut f.effort,
            &mut f.proxy_url,
        ];
        let slot = slots.into_iter().nth(i).unwrap();
        let value = if i == 2 {
            secret("API Key（隐藏输入，Esc 保留原值）")?
        } else {
            text(labels[i], slot)?
        };
        if let Some(v) = value {
            *slot = v;
        }
    }
}
fn launch(store: Store, agent: Agent) -> Result<()> {
    let options = if cfg!(target_os = "linux") {
        vec!["CLI".into()]
    } else {
        vec!["CLI".into(), "桌面版".into()]
    };
    if let Some(i) = choose(&format!("启动 {}", agent.label()), options)? {
        execute(
            store,
            Action::Run {
                desktop: i == 1,
                executable: None,
                dry_run: false,
                args: Vec::new(),
            },
            Some(agent_arg(agent)),
        )?;
    }
    Ok(())
}
fn proxy(store: Store) -> Result<()> {
    let Some(i) = choose("官方代理所属客户端", vec!["Codex".into(), "Claude".into()])?
    else {
        return Ok(());
    };
    let agent = if i == 0 { Agent::Codex } else { Agent::Claude };
    loop {
        let session = Session::open(store.clone())?;
        let current = session
            .original
            .agent(agent)
            .library
            .official_proxy_url
            .as_deref()
            .unwrap_or("");
        let Some(url) = text("官方 HTTP 代理（留空直连）", current)? else {
            return Ok(());
        };
        match execute(
            store.clone(),
            Action::Proxy { url: Some(url) },
            Some(agent_arg(agent)),
        ) {
            Ok(()) => return Ok(()),
            Err(e) => eprintln!("保存失败：{e:#}"),
        }
    }
}
