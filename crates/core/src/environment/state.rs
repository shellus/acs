use super::*;
use std::collections::BTreeMap;
/// apply 的本机状态 v1；不含账号或 Provider 数据。迁移器应显式构造此类型。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub version: u32,
    pub source: String,
    pub branch: String,
    pub commit: String,
    pub profile: String,
    pub agents: BTreeMap<Agent, AgentState>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentState {
    pub version: String,
    pub instruction_hash: String,
    pub skills: BTreeMap<String, String>,
    pub skills_root: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedMarker {
    pub version: u32,
    pub skills: BTreeMap<String, String>,
}
impl State {
    pub fn empty() -> Self {
        Self {
            version: 1,
            ..Self::default()
        }
    }
    pub fn load(path: &Path) -> Result<Self> {
        let Some(bytes) = storage::read(path)? else {
            return Ok(Self::empty());
        };
        let s: Self = serde_json::from_slice(&bytes)?;
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!("不支持环境状态版本 {}", self.version);
        }
        for a in self.agents.values() {
            validate_skills(&a.skills)?;
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut b = serde_json::to_vec_pretty(self)?;
        b.push(b'\n');
        Ok(b)
    }
}
fn validate_skills(skills: &BTreeMap<String, String>) -> Result<()> {
    for name in skills.keys() {
        if !safe_name(name) {
            bail!("状态包含不安全 Skill 名 {name:?}");
        }
    }
    Ok(())
}
impl ManagedMarker {
    pub fn empty() -> Self {
        Self {
            version: 1,
            skills: BTreeMap::new(),
        }
    }
    pub fn load(root: &Path) -> Result<Self> {
        let Some(b) = storage::read(&root.join(MANAGED_MARKER))? else {
            return Ok(Self::empty());
        };
        let m: Self = serde_json::from_slice(&b)?;
        m.validate()?;
        Ok(m)
    }
    fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!("不支持受管标记版本 {}", self.version);
        }
        validate_skills(&self.skills)
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut b = serde_json::to_vec_pretty(self)?;
        b.push(b'\n');
        Ok(b)
    }
}
