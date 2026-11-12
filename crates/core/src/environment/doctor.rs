use super::{
    command::{run, Runner},
    *,
};
#[derive(Clone, Debug)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    pub fix: String,
}
#[derive(Clone, Debug, Default)]
pub struct DoctorReport {
    pub checks: Vec<Check>,
}
impl DoctorReport {
    pub fn healthy(&self) -> bool {
        self.checks.iter().all(|c| c.ok)
    }
    fn add(&mut self, name: impl Into<String>, result: Result<String>, fix: &str) {
        let (ok, detail) = match result {
            Ok(s) => (true, s),
            Err(e) => (false, format!("{e:#}")),
        };
        self.checks.push(Check {
            name: name.into(),
            ok,
            detail,
            fix: if ok { String::new() } else { fix.into() },
        });
    }
}
pub fn doctor(paths: &Paths, runner: &dyn Runner) -> DoctorReport {
    let mut report = DoctorReport::default();
    for program in ["git", "node", "npm"] {
        report.add(
            format!("依赖 {program}"),
            run(runner, program, &["--version"], None)
                .and_then(|b| Ok(String::from_utf8(b)?.trim().into())),
            "安装 git 与 Node.js（含 npm），确认 PATH 后重试 acs doctor",
        );
    }
    let config = LocalConfig::load(&paths.config());
    report.add("本机环境配置",config.as_ref().map(|c|format!("配置版本 {}",c.version)).map_err(|e|anyhow::anyhow!("{e:#}")),"修正 environment.yaml，或 acs env source set <仓库> 后 acs env configure --profile <名称> --agents all");
    let Ok(config) = config else { return report };
    let repo = source_repository(
        paths,
        &config.environment.source,
        &config.environment.branch,
    );
    report.add(
        "来源与仓库声明",
        repo.as_ref()
            .map(|r| r.root.display().to_string())
            .map_err(|e| anyhow::anyhow!("{e:#}")),
        "检查网络、仓库权限和分支；acs env source set <可达仓库> --branch <分支>",
    );
    if let Ok(repo) = repo {
        for agent in Agent::ALL {
            let result = repo.package(agent).and_then(|p| {
                let installed = p.installed(runner)?;
                if !p.satisfied(&installed) {
                    bail!("已安装 {:?}，要求 {}", installed, p.requirement());
                }
                Ok(installed)
            });
            report.add(
                format!("{} 安装版本", agent.label()),
                result,
                &format!("acs agent install {}", agent.label().to_lowercase()),
            );
        }
    }
    let state = State::load(&paths.state());
    report.add(
        "应用状态格式",
        state
            .as_ref()
            .map(|s| format!("状态版本 {}", s.version))
            .map_err(|e| anyhow::anyhow!("{e:#}")),
        "恢复状态备份后重试；不要直接删除受管状态",
    );
    if let Ok(state) = state {
        for agent in &config.environment.agents {
            let root = paths.destinations(*agent).1;
            let result = ManagedMarker::load(&root).and_then(|m| {
                let s = state
                    .agents
                    .get(agent)
                    .context("尚未记录该 Agent 的应用状态")?;
                if !root.join(MANAGED_MARKER).exists() {
                    bail!("缺少受管标记");
                }
                let instruction =
                    storage::read(&paths.destinations(*agent).0)?.context("缺少全局指令")?;
                use sha2::{Digest, Sha256};
                if format!("{:x}", Sha256::digest(&instruction)) != s.instruction_hash {
                    bail!("指令哈希与应用状态不一致");
                }
                if s.skills_root != root || s.skills != m.skills {
                    bail!("Skills 标记与应用状态不一致");
                }
                Ok(format!("{} 个受管 Skills", m.skills.len()))
            });
            report.add(
                format!("{} 标记/状态", agent.label()),
                result,
                "acs env diff；确认变更后运行 acs env apply",
            );
        }
    }
    let service = Service { paths, runner };
    report.add(
        "实际文件漂移",
        service.prepare().and_then(|p| {
            if p.has_changes() {
                bail!("检测到变更：{}", p.changes.join("；"));
            }
            Ok("环境无漂移".into())
        }),
        "acs env diff；确认变更后运行 acs env apply",
    );
    report
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Missing;
    impl Runner for Missing {
        fn run(&self, _: &str, _: &[std::ffi::OsString], _: Option<&Path>) -> Result<Vec<u8>> {
            bail!("example missing executable")
        }
    }
    #[test]
    fn missing_dependencies_and_source_are_reported_individually_without_global_state() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::under(d.path());
        let report = doctor(&p, &Missing);
        assert!(!report.healthy());
        assert!(report.checks.iter().filter(|c| !c.ok).count() >= 4);
        assert!(report
            .checks
            .iter()
            .filter(|c| !c.ok)
            .all(|c| !c.fix.is_empty()));
        assert!(!p.config().exists());
    }
}
