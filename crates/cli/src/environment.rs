use acs_core::{environment as env, Agent};
use anyhow::Result;
use clap::Subcommand;
use std::path::{Path, PathBuf};
#[derive(Subcommand)]
pub enum Command {
    /// 设置或查看环境来源
    Source {
        #[command(subcommand)]
        command: Source,
    },
    /// 保存 Profile 与启用的 Agent；未给出的字段保留
    #[command(arg_required_else_help = true)]
    Configure {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long,value_delimiter=',',value_parser=parse_agent)]
        agents: Option<Vec<Agent>>,
    },
    /// 只读验证环境仓库，不下载、不执行 build/patch
    Validate {
        #[arg(long)]
        repo: PathBuf,
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
fn parse_agent(value: &str) -> Result<Agent, String> {
    match value {
        "codex" => Ok(Agent::Codex),
        "claude" => Ok(Agent::Claude),
        _ => Err("Agent 必须为 codex 或 claude".into()),
    }
}
pub fn run(command: Command, root: Option<&Path>) -> Result<()> {
    if let Command::Validate { repo } = &command {
        env::validate(repo)?;
        println!("环境仓库校验通过：{}", repo.display());
        return Ok(());
    }
    let paths = match root {
        Some(p) => env::Paths::under(p),
        None => env::Paths::discover()?,
    };
    let config = match command {
        Command::Source {
            command: Source::Show,
        } => env::LocalConfig::load(&paths.config())?,
        Command::Source {
            command: Source::Set { source, branch },
        } => env::set_source(&paths, &source, branch.as_deref())?,
        Command::Configure { profile, agents } => {
            env::configure(&paths, profile.as_deref(), agents)?
        }
        Command::Validate { .. } => unreachable!(),
    };
    println!("{}", serde_json::to_string_pretty(&config)?);
    Ok(())
}
