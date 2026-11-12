use acs_core::{environment as env, Agent};
use anyhow::{bail, Result};
use clap::{CommandFactory, Subcommand};
use std::{
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};
#[derive(Subcommand)]
pub enum Command {
    Source {
        #[command(subcommand)]
        command: Source,
    },
    #[command(arg_required_else_help = true)]
    Configure {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long, value_delimiter = ',')]
        agents: Option<Vec<Target>>,
    },
    Validate {
        #[arg(long)]
        repo: PathBuf,
    },
    /// 更新 Agent 与上游来源锁定版本
    Lock {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
    },
    /// 按锁定来源重新生成 vendor 快照
    Vendor {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
    },
    /// 计划变更并在确认后应用
    Apply {
        #[arg(long, short)]
        yes: bool,
    },
    /// 查看环境漂移；有漂移时仅报告，不写入目标文件
    Status,
    /// 列出指令、配置、Skills 与安装计划
    Diff,
    /// 从本机 AGS 显式接管，不删除其配置目录和二进制
    ImportAgs {
        #[arg(long)]
        ags_config: Option<PathBuf>,
        #[arg(long)]
        ags_state: Option<PathBuf>,
    },
}
#[derive(Subcommand)]
pub enum Source {
    Set {
        source: String,
        #[arg(long)]
        branch: Option<String>,
    },
    Show,
}
#[derive(Subcommand)]
pub enum AgentCommand {
    Install {
        #[arg(value_delimiter = ',', required = true)]
        agents: Vec<Target>,
    },
    Uninstall {
        #[arg(value_delimiter = ',', required = true)]
        agents: Vec<Target>,
        #[arg(long)]
        purge: bool,
        #[arg(long, short)]
        yes: bool,
    },
    Status {
        #[arg(value_delimiter = ',')]
        agents: Vec<Target>,
    },
}
#[derive(Clone)]
pub enum Target {
    One(Agent),
    All,
}
impl std::str::FromStr for Target {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        if s == "all" {
            Ok(Self::All)
        } else {
            Ok(Self::One(s.parse()?))
        }
    }
}
fn expand(targets: Vec<Target>) -> Vec<Agent> {
    let mut selected = std::collections::BTreeSet::new();
    for target in targets {
        match target {
            Target::All => selected.extend(Agent::ALL),
            Target::One(a) => {
                selected.insert(a);
            }
        }
    }
    selected.into_iter().collect()
}
fn paths(root: Option<&Path>) -> Result<env::Paths> {
    match root {
        Some(p) => Ok(env::Paths::under(p)),
        None => env::Paths::discover(),
    }
}
fn confirm(yes: bool) -> Result<()> {
    if yes {
        return Ok(());
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        super::Cli::command()
            .error(
                clap::error::ErrorKind::MissingRequiredArgument,
                "非终端应用或卸载必须显式传入 --yes",
            )
            .exit();
    }
    eprint!("应用以上变更？[y/N] ");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if !answer.trim().eq_ignore_ascii_case("y") {
        bail!("已取消，未修改目标文件");
    }
    Ok(())
}
fn config_text(config: &env::LocalConfig) {
    let e = &config.environment;
    println!(
        "来源：{}\n分支：{}\nProfile：{}\n启用 Agent：{}",
        if e.source.is_empty() {
            "未配置"
        } else {
            &e.source
        },
        e.branch,
        if e.profile.is_empty() {
            "未配置"
        } else {
            &e.profile
        },
        if e.agents.is_empty() {
            "无".into()
        } else {
            e.agents
                .iter()
                .map(|a| a.label())
                .collect::<Vec<_>>()
                .join("、")
        }
    );
}
fn print_plan(plan: &env::Plan) {
    println!("环境 Profile：{}", plan.profile);
    for (p, v) in &plan.packages {
        println!(
            "{}：已安装 {}，要求 {}{}",
            p.agent.label(),
            if v.is_empty() { "无" } else { v },
            p.requirement(),
            if p.satisfied(v) {
                ""
            } else {
                "；需要安装"
            }
        );
    }
    if !plan.has_changes() {
        println!("环境无漂移，无需写入");
    } else {
        for c in &plan.changes {
            println!("  {c}");
        }
    }
}
pub fn run(command: Command, root: Option<&Path>) -> Result<()> {
    if let Command::Validate { repo } = &command {
        env::validate(repo)?;
        println!("环境仓库校验通过：{}", repo.display());
        return Ok(());
    }
    // 参数检查必须早于配置读取、git 和 npm。
    if matches!(&command, Command::Apply { yes: false })
        && (!io::stdin().is_terminal() || !io::stdout().is_terminal())
    {
        confirm(false)?;
    }
    let paths = paths(root)?;
    let service = env::Service {
        paths: &paths,
        runner: &env::command::SystemRunner,
    };
    match command {
        Command::Source {
            command: Source::Show,
        } => config_text(&env::LocalConfig::load(&paths.config())?),
        Command::Source {
            command: Source::Set { source, branch },
        } => {
            let c = env::set_source(&paths, &source, branch.as_deref())?;
            println!("环境来源已保存");
            config_text(&c);
        }
        Command::Configure { profile, agents } => {
            let c = env::configure(&paths, profile.as_deref(), agents.map(expand))?;
            println!("本机环境选择已保存");
            config_text(&c);
        }
        Command::Apply { yes } => {
            let plan = service.prepare()?;
            print_plan(&plan);
            if plan.has_changes() {
                confirm(yes)?;
                service.apply(&plan)?;
                println!("环境应用完成");
            }
        }
        Command::Status | Command::Diff => print_plan(&service.prepare()?),
        Command::ImportAgs {
            ags_config,
            ags_state,
        } => {
            let home = match root {
                Some(p) => p.to_path_buf(),
                None => paths
                    .codex_skills
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .to_path_buf(),
            };
            let platform = if cfg!(windows) {
                env::LegacyPlatform::Windows
            } else {
                env::LegacyPlatform::Unix
            };
            let mut legacy = env::LegacyPaths::resolve(&home, platform, |k| {
                if root.is_some() {
                    None
                } else {
                    std::env::var(k).ok()
                }
            });
            if let Some(p) = ags_config {
                legacy.config = p;
            }
            if let Some(p) = ags_state {
                legacy.state = p;
            }
            for message in env::import_ags(&paths, &legacy)? {
                println!("{message}");
            }
        }
        Command::Lock { repo } => {
            let _lock = paths.store().lock()?;
            let result = env::update_lock(&repo, &env::command::SystemRunner)?;
            println!(
                "environment.lock 已更新：{} 个 Agent、{} 个来源",
                result.agents.len(),
                result.sources.len()
            );
        }
        Command::Vendor { repo } => {
            let snapshot = env::vendor(&paths, &repo, &env::command::SystemRunner, |p| {
                eprintln!("{} {}/{}", p.phase, p.completed, p.total)
            })?;
            println!("已发布 {} 个 vendor Skills", snapshot.skills.len());
        }
        Command::Validate { .. } => unreachable!(),
    }
    Ok(())
}
pub fn agents(command: AgentCommand, root: Option<&Path>) -> Result<()> {
    if matches!(&command, AgentCommand::Uninstall { yes: false, .. })
        && (!io::stdin().is_terminal() || !io::stdout().is_terminal())
    {
        confirm(false)?;
    }
    let paths = paths(root)?;
    let service = env::Service {
        paths: &paths,
        runner: &env::command::SystemRunner,
    };
    match command {
        AgentCommand::Install { agents } => {
            for text in service.install(&expand(agents))? {
                println!("{text}");
            }
        }
        AgentCommand::Status { agents } => {
            let agents = if agents.is_empty() {
                Agent::ALL.to_vec()
            } else {
                expand(agents)
            };
            for text in service.agent_status(&agents)? {
                println!("{text}");
            }
        }
        AgentCommand::Uninstall { agents, purge, yes } => {
            let agents = expand(agents);
            println!(
                "卸载：{}；{}",
                agents
                    .iter()
                    .map(|a| a.label())
                    .collect::<Vec<_>>()
                    .join("、"),
                if purge {
                    "同时删除客户端配置目录"
                } else {
                    "保留非受管内容和用户配置"
                }
            );
            confirm(yes)?;
            service.uninstall(&agents, purge)?;
            println!("卸载完成");
        }
    }
    Ok(())
}
