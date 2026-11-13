use super::{
    command::{run, Runner},
    *,
};
#[derive(Clone, Debug)]
pub struct AgentPackage {
    pub agent: Agent,
    pub name: String,
    pub version: String,
    pub pin: bool,
}
impl AgentPackage {
    pub fn satisfied(&self, installed: &str) -> bool {
        let installed = installed.trim();
        if installed.is_empty() {
            return false;
        }
        if self.pin {
            return installed == self.version;
        }
        compare_versions(installed, &self.version).is_none_or(|o| !o.is_lt())
    }
    pub fn requirement(&self) -> String {
        format!("{}{}", if self.pin { "=" } else { ">=" }, self.version)
    }
    fn validate_name(&self) -> Result<()> {
        if self.name.starts_with('-')
            || self.name.contains(['\\', ':'])
            || self
                .name
                .split('/')
                .any(|p| p.is_empty() || p == ".." || p == ".")
        {
            bail!("无效 npm 包名 {}", self.name);
        }
        Ok(())
    }
    pub fn installed(&self, runner: &dyn Runner) -> Result<String> {
        let root = String::from_utf8(run(runner, "npm", &["root", "-g"], None)?)?;
        let root = root.trim();
        if root.is_empty() {
            bail!("npm root -g 返回空目录");
        }
        self.validate_name()?;
        let path = Path::new(root).join(&self.name).join("package.json");
        let Some(bytes) = storage::read(&path)? else {
            return Ok(String::new());
        };
        let metadata: serde_json::Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("解析已安装包 {}", self.name))?;
        Ok(metadata["version"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .into())
    }
    pub fn install(&self, runner: &dyn Runner) -> Result<()> {
        self.validate_name()?;
        if self.name.trim().is_empty() || self.version.trim().is_empty() {
            bail!("npm 包名与版本不能为空");
        }
        run(
            runner,
            "npm",
            &["install", "-g", &format!("{}@{}", self.name, self.version)],
            None,
        )?;
        Ok(())
    }
    pub fn uninstall(&self, runner: &dyn Runner) -> Result<()> {
        self.validate_name()?;
        run(runner, "npm", &["uninstall", "-g", &self.name], None)?;
        Ok(())
    }
}
impl Repository {
    pub fn package(&self, agent: Agent) -> Result<AgentPackage> {
        let p = self
            .manifest
            .agents
            .get(&agent)
            .context("环境缺少 Agent 包")?;
        let l = self
            .lock
            .agents
            .get(&agent)
            .context("环境缺少 Agent 锁定版本")?;
        Ok(AgentPackage {
            agent,
            name: p.package.clone(),
            version: l.version.clone(),
            pin: p.pin,
        })
    }
}

/// 版本比较支持：可带 v、构建元数据，无法解析时由调用方保留安装。
pub fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    fn parse(v: &str) -> Option<([i64; 3], Vec<&str>)> {
        let v = v
            .trim()
            .strip_prefix('v')
            .unwrap_or(v.trim())
            .split('+')
            .next()?;
        let (core, pre) = v
            .split_once('-')
            .map(|(a, b)| (a, b.split('.').collect::<Vec<_>>()))
            .unwrap_or((v, vec![]));
        let numbers = core
            .split('.')
            .map(|s| s.parse::<i64>().ok().filter(|n| *n >= 0))
            .collect::<Option<Vec<_>>>()?;
        if numbers.len() != 3 || pre.iter().any(|p| p.is_empty()) {
            return None;
        }
        Some((numbers.try_into().ok()?, pre))
    }
    let (ac, ap) = parse(a)?;
    let (bc, bp) = parse(b)?;
    let core = ac.cmp(&bc);
    if !core.is_eq() {
        return Some(core);
    }
    if ap.is_empty() || bp.is_empty() {
        return Some(ap.is_empty().cmp(&bp.is_empty()));
    }
    for (a, b) in ap.iter().zip(&bp) {
        let cmp = match (a.parse::<i64>(), b.parse::<i64>()) {
            (Ok(a), Ok(b)) => a.cmp(&b),
            (Ok(_), Err(_)) => std::cmp::Ordering::Less,
            (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
            _ => a.cmp(b),
        };
        if !cmp.is_eq() {
            return Some(cmp);
        }
    }
    Some(ap.len().cmp(&bp.len()))
}
