use acs_core::{Agent, CustomFields, Draft, Mode, Roots, Session, Store};
use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::{
    io::{self, Write},
    path::PathBuf,
};

#[derive(Parser)]
#[command(
    version,
    about = "管理 Codex 与 Claude 的官方账号、自定义 API 与独立代理；只有 run 会启动客户端"
)]
struct Cli {
    /// 把全部配置根目录放到该目录下，用于隔离测试
    #[arg(long, global = true)]
    root: Option<PathBuf>,
    /// 客户端；run 与 proxy 使用，ls 用它过滤
    #[arg(long, value_enum, global = true)]
    agent: Option<AgentArg>,
    #[command(subcommand)]
    command: Option<Action>,
}
#[derive(Clone, Copy, ValueEnum)]
enum AgentArg {
    Codex,
    Claude,
}
impl From<AgentArg> for Agent {
    fn from(value: AgentArg) -> Self {
        match value {
            AgentArg::Codex => Agent::Codex,
            AgentArg::Claude => Agent::Claude,
        }
    }
}
#[derive(Clone, Copy, ValueEnum)]
enum Kind {
    Official,
    Custom,
}
impl From<Kind> for Mode {
    fn from(k: Kind) -> Self {
        match k {
            Kind::Official => Mode::Official,
            Kind::Custom => Mode::Custom,
        }
    }
}
/// 自定义 API 的可编辑字段；未给出的字段保持原值。
#[derive(Args, Default)]
struct ApiFields {
    /// 提供者显示名称
    #[arg(short, long)]
    provider: Option<String>,
    #[arg(short, long)]
    url: Option<String>,
    /// API Key；写在命令行会进入 shell 历史与进程列表
    #[arg(short, long)]
    key: Option<String>,
    #[arg(short, long)]
    model: Option<String>,
    #[arg(short, long)]
    effort: Option<String>,
}
impl ApiFields {
    fn apply(&self, f: &mut CustomFields) {
        let pairs = [
            (&self.provider, &mut f.provider_name),
            (&self.url, &mut f.base_url),
            (&self.key, &mut f.api_key),
            (&self.model, &mut f.model),
            (&self.effort, &mut f.effort),
        ];
        for (value, slot) in pairs {
            if let Some(value) = value {
                *slot = value.clone();
            }
        }
    }
    fn is_empty(&self) -> bool {
        self.provider.is_none()
            && self.url.is_none()
            && self.key.is_none()
            && self.model.is_none()
            && self.effort.is_none()
    }
}

#[derive(Subcommand)]
enum Action {
    /// 列出两个客户端的全部配置；不写子命令时执行同样的动作
    #[command(alias = "list")]
    Ls,
    /// 输出完整编辑草稿（包含凭据），可配合 apply 使用
    Export,
    /// 应用 export 的 JSON 文件，校验文件内 revision
    Apply { file: PathBuf },
    /// 新增配置并保存；不启动
    Add {
        #[arg(value_enum)]
        agent: AgentArg,
        #[arg(value_enum)]
        kind: Kind,
        name: String,
        /// 复制已有自定义 API 配置作为初值
        #[arg(short, long)]
        copy: Option<String>,
        #[command(flatten)]
        fields: ApiFields,
    },
    /// 按 ID、ID 前缀或名称定位自定义 API 配置并修改
    Edit {
        target: String,
        #[command(flatten)]
        fields: ApiFields,
    },
    /// 按 ID、ID 前缀或名称定位并切换配置；不启动
    Use { target: String },
    /// 按 ID、ID 前缀或名称定位并改名
    Mv { target: String, name: String },
    /// 按 ID、ID 前缀或名称定位并删除
    Rm {
        target: String,
        #[arg(short, long)]
        yes: bool,
    },
    /// 设置当前客户端的官方代理偏好；空字符串禁用；不带参数只显示
    Proxy { url: Option<String> },
    /// 修复 Codex 历史对话的 provider ID；先备份
    Repair {
        #[arg(short, long)]
        yes: bool,
    },
    /// 启动已保存配置；--agent 选择客户端，默认启动 CLI
    Run {
        /// 启动桌面客户端
        #[arg(short, long)]
        desktop: bool,
        /// 指定客户端可执行文件，跳过自动查找
        #[arg(short = 'x', long)]
        executable: Option<PathBuf>,
        /// 只输出启动计划，不启动
        #[arg(short = 'n', long)]
        dry_run: bool,
        /// 传给客户端自身的参数
        #[arg(last = true)]
        args: Vec<String>,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("操作失败：{error:#}");
        std::process::exit(1);
    }
}
fn confirm(yes: bool, message: &str) -> Result<()> {
    if yes {
        return Ok(());
    }
    eprint!("{message} [y/N] ");
    io::stderr().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    if !input.trim().eq_ignore_ascii_case("y") {
        bail!("已取消，未修改文件")
    }
    Ok(())
}
/// 配置 ID 全局唯一，因此 ID、ID 前缀和名称足以定位客户端与模式，不必再写 kind。
/// 按精确 ID、ID 前缀、精确名称、名称前缀逐级匹配；某一级命中多项时要求说得更具体。
fn locate(draft: &Draft, query: &str) -> Result<(Agent, Mode, String)> {
    let mut all = Vec::new();
    for agent in Agent::ALL {
        let d = draft.agent(agent);
        for mode in [Mode::Official, Mode::Custom] {
            for p in d.library.profiles(mode) {
                all.push((agent, mode, p.id.clone(), p.name.clone()));
            }
        }
    }
    let matched = |rank: usize| -> Vec<_> {
        all.iter()
            .filter(|(_, _, id, name)| match rank {
                0 => id == query,
                1 => id.starts_with(query),
                2 => name.as_deref().is_some_and(|n| n.trim() == query),
                _ => name.as_deref().is_some_and(|n| n.trim().starts_with(query)),
            })
            .collect()
    };
    for rank in 0..4 {
        match matched(rank).as_slice() {
            [] => continue,
            [only] => return Ok((only.0, only.1, only.2.clone())),
            many => bail!(
                "{query} 匹配到多项配置，请使用以下 ID 之一：{}",
                many.iter()
                    .map(|(a, m, id, _)| format!("{id}（{} {}）", a.label(), m.label()))
                    .collect::<Vec<_>>()
                    .join("、")
            ),
        }
    }
    bail!("找不到配置 {query}")
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let store = match cli.root {
        Some(path) => Store::new(Roots::under(&path)),
        None => Store::discover()?,
    };
    let result = execute(store.clone(), cli.command.unwrap_or(Action::Ls), cli.agent);
    if let Err(error) = &result {
        store.log("CLI", &format!("{error:#}"));
    }
    result
}
fn execute(store: Store, action: Action, selected: Option<AgentArg>) -> Result<()> {
    if let Action::Repair { yes } = action {
        confirm(yes, "将 Codex 的 sessions、archived_sessions 和 SQLite 历史的 provider ID 改为 custom；请先关闭正在写入历史的 Codex。继续？")?;
        let report = acs_core::history::repair(&store, |p| {
            eprintln!("{} {}/{}", p.phase, p.completed, p.total)
        })?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    let mut session = Session::open(store)?;
    let revision = session.view().revision;
    let mut draft: Draft = session.original.clone();
    match action {
        Action::Ls => {
            list(&draft, selected.map(Into::into));
            Ok(())
        }
        Action::Export => {
            println!("{}", serde_json::to_string_pretty(&session.view())?);
            Ok(())
        }
        Action::Apply { file } => {
            let v: acs_core::View = serde_json::from_slice(&std::fs::read(file)?)?;
            session.save(&v.revision, &v.draft)?;
            println!("已保存配置；未启动客户端");
            Ok(())
        }
        Action::Add {
            agent,
            kind,
            name,
            copy,
            fields,
        } => {
            let agent: Agent = agent.into();
            let mode: Mode = kind.into();
            if mode == Mode::Official && !fields.is_empty() {
                bail!("官方账号的凭据由官方客户端登录维护，请只给出名称")
            }
            let target = draft.agent_mut(agent);
            let id = target.add(mode, &name, copy.as_deref())?;
            if mode == Mode::Custom {
                fields.apply(target.custom_fields.get_mut(&id).unwrap());
            }
            target.mode = mode;
            save(&mut session, &revision, &draft, agent)?;
            eprintln!("配置 ID：{id}");
            Ok(())
        }
        Action::Edit { target, fields } => {
            let (agent, mode, id) = locate(&draft, &target)?;
            if mode == Mode::Official {
                bail!("官方账号的凭据由官方客户端登录维护；改名请用 mv")
            }
            fields.apply(
                draft
                    .agent_mut(agent)
                    .custom_fields
                    .get_mut(&id)
                    .context("API 表单缺失")?,
            );
            save(&mut session, &revision, &draft, agent)
        }
        Action::Use { target } => {
            let (agent, mode, id) = locate(&draft, &target)?;
            let target = draft.agent_mut(agent);
            target.mode = mode;
            target.library.select(mode, id);
            save(&mut session, &revision, &draft, agent)
        }
        Action::Mv { target, name } => {
            if name.trim().is_empty() {
                bail!("请输入名称")
            }
            let (agent, mode, id) = locate(&draft, &target)?;
            draft
                .agent_mut(agent)
                .library
                .profiles_mut(mode)
                .iter_mut()
                .find(|p| p.id == id)
                .context("配置不存在")?
                .name = Some(name.trim().into());
            save(&mut session, &revision, &draft, agent)
        }
        Action::Rm { target, yes } => {
            let (agent, mode, id) = locate(&draft, &target)?;
            confirm(
                yes,
                &format!("删除 {} 的配置 {} 并保存？", agent.label(), id),
            )?;
            draft.agent_mut(agent).delete(mode, &id)?;
            save(&mut session, &revision, &draft, agent)
        }
        Action::Proxy { url } => {
            let agent = selected.map(Into::into).unwrap_or(Agent::Codex);
            let target = draft.agent_mut(agent);
            let Some(url) = url else {
                println!(
                    "{}",
                    target.library.official_proxy_url.as_deref().unwrap_or("")
                );
                return Ok(());
            };
            target.library.official_proxy_url = Some(url);
            save(&mut session, &revision, &draft, agent)
        }
        Action::Run {
            desktop,
            executable,
            dry_run,
            args,
        } => {
            let agent = selected.map(Into::into).unwrap_or(Agent::Codex);
            let settings = session.launch_settings_for(&revision, &draft, agent)?;
            let executable = match executable {
                Some(p) => p,
                None if desktop => acs_core::launch::desktop_executable(agent)?,
                None => acs_core::launch::cli_executable(agent),
            };
            let plan = settings.plan(executable, desktop, args)?;
            if dry_run {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            } else if desktop {
                plan.spawn_desktop()?;
            } else {
                let status = plan.spawn()?.wait()?;
                std::process::exit(status.code().unwrap_or(1));
            }
            Ok(())
        }
        Action::Repair { .. } => unreachable!(),
    }
}
fn save(session: &mut Session, revision: &str, draft: &Draft, agent: Agent) -> Result<()> {
    session.save_for(revision, draft, agent)?;
    println!("已保存配置；未启动客户端");
    Ok(())
}
fn list(draft: &Draft, only: Option<Agent>) {
    for agent in Agent::ALL {
        if only.is_some_and(|a| a != agent) {
            continue;
        }
        let d = draft.agent(agent);
        println!("{} · 当前 {}", agent.label(), d.mode.label());
        let mut empty = true;
        for mode in [Mode::Official, Mode::Custom] {
            let profiles = d.library.profiles(mode);
            if profiles.is_empty() {
                continue;
            }
            empty = false;
            println!("  {}", mode.label());
            for p in profiles {
                let selected = d.mode == mode && d.library.selection(mode) == p.id;
                let name = p
                    .name
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("未命名配置");
                let detail = match mode {
                    Mode::Official => p
                        .email
                        .clone()
                        .or_else(|| p.account_name.clone())
                        .unwrap_or_else(|| {
                            if p.official_auth.is_some() {
                                "已保存本地凭据".into()
                            } else {
                                "尚未登录".into()
                            }
                        }),
                    Mode::Custom => d
                        .custom_fields
                        .get(&p.id)
                        .map(|f| f.base_url.trim().to_owned())
                        .filter(|u| !u.is_empty())
                        .unwrap_or_else(|| "未填写地址".into()),
                };
                println!(
                    "    {} {}  {}  {}",
                    if selected { '*' } else { ' ' },
                    p.id,
                    name,
                    detail
                );
            }
        }
        if empty {
            println!("  （没有配置）");
        }
    }
}
