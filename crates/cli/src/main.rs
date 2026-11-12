mod display;
mod environment;
mod interactive;
use acs_core::{Agent, CustomFields, Draft, Mode, Roots, Session, Store};
use anyhow::{bail, Context, Result};
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
};

#[derive(Parser)]
#[command(
    version,
    about = "管理 Codex 与 Claude 的配置与登录；无参进入全屏 TUI，显式启动客户端"
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
    /// 此配置的 HTTP 代理，空字符串表示直连
    #[arg(long)]
    proxy: Option<String>,
}
impl ApiFields {
    fn apply(&self, f: &mut CustomFields) {
        let pairs = [
            (&self.provider, &mut f.provider_name),
            (&self.url, &mut f.base_url),
            (&self.key, &mut f.api_key),
            (&self.model, &mut f.model),
            (&self.effort, &mut f.effort),
            (&self.proxy, &mut f.proxy_url),
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
            && self.proxy.is_none()
    }
}

#[derive(Subcommand)]
enum Action {
    /// 管理 Agent 的 npm 全局安装；与 --agent 客户端选择参数独立
    Agent {
        #[command(subcommand)]
        command: environment::AgentCommand,
    },
    /// 管理 Agent 环境来源与本机选择
    Env {
        #[command(subcommand)]
        command: environment::Command,
    },
    /// 列出两个客户端的全部配置；无参且连接终端时进入菜单
    #[command(alias = "list")]
    Ls,
    /// 输出完整编辑草稿（包含凭据），可配合 apply 使用
    Export,
    /// 应用 export 的 JSON 文件，校验文件内 revision
    Apply { file: PathBuf },
    /// 导出仅含自定义 API 的可迁移配置包（含 Key，不含官方账号）
    ExportCustom { file: PathBuf },
    /// 导入自定义配置包，只写配置库，保留当前生效文件
    ImportCustom { file: PathBuf },
    /// 新增配置并保存；不启动
    Add {
        #[arg(value_enum)]
        agent: Option<AgentArg>,
        #[arg(value_enum)]
        kind: Option<Kind>,
        name: Option<String>,
        /// 复制已有自定义 API 配置作为初值
        #[arg(short, long)]
        copy: Option<String>,
        #[command(flatten)]
        fields: ApiFields,
    },
    /// 按 ID、ID 前缀或名称定位自定义 API 配置并修改
    Edit {
        target: Option<String>,
        #[command(flatten)]
        fields: ApiFields,
    },
    /// 按 ID、ID 前缀或名称定位并切换配置；不启动
    Use { target: Option<String> },
    /// 按 ID、ID 前缀或名称定位并改名
    Mv { target: String, name: String },
    /// 按 ID、ID 前缀或名称定位并删除
    Rm {
        target: Option<String>,
        #[arg(short, long)]
        yes: bool,
    },
    /// Codex 官方账号通过粘贴浏览器回调 URL 登录，成功后设为当前
    Login { target: String },
    /// 设置当前客户端官方账号共用的代理；空字符串禁用；不带参数只显示。自定义 API 用 edit --proxy
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
    if terminal() {
        if interactive::confirmed(message)? {
            return Ok(());
        }
        bail!("已取消，未修改文件");
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
    if !terminal() {
        let missing = match &cli.command {
            Some(Action::Add {
                agent, kind, name, ..
            }) => agent.is_none() || kind.is_none() || name.is_none(),
            Some(
                Action::Edit { target, .. } | Action::Use { target } | Action::Rm { target, .. },
            ) => target.is_none(),
            _ => false,
        };
        if missing {
            Cli::command()
                .error(
                    clap::error::ErrorKind::MissingRequiredArgument,
                    "非终端输入必须提供完整参数；使用 acs <命令> --help 查看用法",
                )
                .exit();
        }
    }
    if cli.agent.is_some()
        && matches!(
            &cli.command,
            Some(Action::Env { .. } | Action::Agent { .. })
        )
    {
        Cli::command()
            .error(
                clap::error::ErrorKind::ArgumentConflict,
                "env/agent 命令使用自己的 Agent 选择参数，不能同时使用全局 --agent",
            )
            .exit();
    }
    if let Some(Action::Env { command }) = cli.command {
        return environment::run(command, cli.root.as_deref());
    }
    if let Some(Action::Agent { command }) = cli.command {
        return environment::agents(command, cli.root.as_deref());
    }
    let store = match cli.root {
        Some(path) => Store::new(Roots::under(&path)),
        None => Store::discover()?,
    };
    let result = match cli.command {
        None if terminal() => interactive::menu(store.clone(), cli.agent),
        Some(action) if terminal() => interactive::dispatch(store.clone(), action, cli.agent),
        action => execute(store.clone(), action.unwrap_or(Action::Ls), cli.agent),
    };
    if let Err(error) = &result {
        store.log("CLI", &format!("{error:#}"));
    }
    result
}
fn execute(store: Store, action: Action, selected: Option<AgentArg>) -> Result<()> {
    if let Action::Repair { yes } = action {
        confirm(yes, "将 Codex 的 sessions、archived_sessions 和 SQLite 历史的 provider ID 改为 custom；请先关闭正在写入历史的 Codex。继续？")?;
        let report = acs_core::history::repair(&store, |p| {
            if io::stderr().is_terminal() {
                eprint!("\r{} {}/{}      ", p.phase, p.completed, p.total);
                let _ = io::stderr().flush();
            } else {
                eprintln!("{} {}/{}", p.phase, p.completed, p.total);
            }
        });
        if io::stderr().is_terminal() {
            eprintln!();
        }
        let report = report?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    let mut session = Session::open(store)?;
    let revision = session.view().revision;
    let mut draft: Draft = session.original.clone();
    match action {
        Action::Agent { .. } | Action::Env { .. } => unreachable!("env 在配置会话打开前分发"),
        Action::ExportCustom { file } => {
            let count = acs_core::transfer::export(session.store.clone(), &file)?;
            println!("已导出 {count} 条自定义配置");
            Ok(())
        }
        Action::ImportCustom { file } => {
            let count = acs_core::transfer::import(session.store.clone(), &file)?;
            println!("已导入 {count} 条配置；当前生效配置未改写");
            Ok(())
        }
        Action::Ls => {
            display::list(&draft, selected.map(Into::into));
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
            let agent: Agent = required(agent, "add <agent> <kind> <name>")?.into();
            let mode: Mode = required(kind, "add <agent> <kind> <name>")?.into();
            let name = required(name, "add <agent> <kind> <name>")?;
            if mode == Mode::Official && !fields.is_empty() {
                bail!("官方账号的凭据由官方客户端登录维护，请只给出名称")
            }
            let id = prepare_add(&mut draft, agent, mode, &name, copy.as_deref(), &fields)?;
            save(&mut session, &revision, &draft, agent)?;
            eprintln!("配置 ID：{id}");
            Ok(())
        }
        Action::Edit { target, fields } => {
            let (agent, mode, id) = locate(&draft, &required(target, "请提供配置 ID 或名称")?)?;
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
            let (agent, mode, id) = locate(&draft, &required(target, "请提供配置 ID 或名称")?)?;
            let target = draft.agent_mut(agent);
            target.mode = mode;
            target.library.select(mode, id);
            save(&mut session, &revision, &draft, agent)
        }
        Action::Mv { target, name } => {
            let (agent, mode, id) = locate(&draft, &target)?;
            draft.agent_mut(agent).rename(mode, &id, &name)?;
            save(&mut session, &revision, &draft, agent)
        }
        Action::Rm { target, yes } => {
            let (agent, mode, id) = locate(&draft, &required(target, "请提供配置 ID 或名称")?)?;
            confirm(
                yes,
                &format!("删除 {} 的配置 {} 并保存？", agent.label(), id),
            )?;
            draft.agent_mut(agent).delete(mode, &id)?;
            save(&mut session, &revision, &draft, agent)
        }
        Action::Login { target } => {
            let (agent, mode, id) = locate(&draft, &target)?;
            if agent != Agent::Codex || mode != Mode::Official {
                bail!("登录仅支持 Codex 官方账号配置")
            }
            login_command(session.store.clone(), &id).map(|_| ())
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
fn login_command(store: Store, id: &str) -> Result<bool> {
    let login = acs_core::oauth::Login::begin(store, id)?;
    println!("{}", login.route().summary());
    if let Some(warning) = login.route().warning() {
        eprintln!("{warning}");
    }
    println!(
        "在任意有浏览器的机器打开以下链接并登录：\n{}",
        login.authorization_url()
    );
    eprintln!("跳转 localhost 后页面打不开也可，粘贴地址栏完整回调 URL（留空取消）：");
    io::stdout().flush()?;
    let mut callback = String::new();
    io::stdin().read_line(&mut callback)?;
    if callback.trim().is_empty() {
        println!("已取消登录；已保存的配置和代理保留");
        return Ok(false);
    }
    login.complete(&callback)?;
    println!("已保存 Codex 官方登录凭据并设为当前；后续刷新由官方客户端负责");
    Ok(true)
}
fn save(session: &mut Session, revision: &str, draft: &Draft, agent: Agent) -> Result<()> {
    session.save_for(revision, draft, agent)?;
    println!("已保存配置；未启动客户端");
    Ok(())
}
fn prepare_add(
    draft: &mut Draft,
    agent: Agent,
    mode: Mode,
    name: &str,
    copy: Option<&str>,
    fields: &ApiFields,
) -> Result<String> {
    let copy_id = copy
        .map(|q| -> Result<String> {
            let (a, m, id) = locate(draft, q)?;
            if a != agent || m != Mode::Custom || mode != Mode::Custom {
                bail!("只能复制同一客户端的自定义 API 配置")
            }
            Ok(id)
        })
        .transpose()?;
    let target = draft.agent_mut(agent);
    let id = target.add(mode, name, copy_id.as_deref())?;
    if mode == Mode::Custom {
        fields.apply(target.custom_fields.get_mut(&id).context("API 表单缺失")?);
    }
    target.mode = mode;
    Ok(id)
}
fn terminal() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}
fn required<T>(value: Option<T>, usage: &str) -> Result<T> {
    value.with_context(|| format!("缺少参数：{usage}；使用 acs --help 查看用法"))
}
