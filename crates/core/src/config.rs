use crate::{
    claude, codex,
    storage::{revision, Roots, Snapshot, Store, LIBRARY},
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

/// 被管理的客户端。两个客户端各自独立生效，不是二选一：Codex 可以停在官方账号，
/// 同时 Claude 走自定义 API。
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    #[default]
    Codex,
    Claude,
}
impl Agent {
    pub const ALL: [Agent; 2] = [Agent::Codex, Agent::Claude];
    pub fn label(self) -> &'static str {
        match self {
            Agent::Codex => "Codex",
            Agent::Claude => "Claude",
        }
    }
}
impl std::str::FromStr for Agent {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "codex" => Ok(Agent::Codex),
            "claude" => Ok(Agent::Claude),
            other => bail!("未知客户端 {other}，可选 codex 或 claude"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Official,
    Custom,
}
impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Official => "官方账号",
            Mode::Custom => "自定义 API",
        }
    }
}
impl std::str::FromStr for Mode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "official" | "login" => Ok(Mode::Official),
            "custom" | "api" => Ok(Mode::Custom),
            other => bail!("未知模式 {other}，可选 login 或 api"),
        }
    }
}

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub id: String,
    pub name: Option<String>,
    /// Codex 保存 auth.json 全文；Claude 保存 `{credentials, account}`。
    pub official_auth: Option<String>,
    pub account_id: Option<String>,
    pub email: Option<String>,
    pub account_name: Option<String>,
    pub custom_key: Option<String>,
    pub custom_model: Option<String>,
    pub custom_effort: Option<String>,
    /// Claude 的 API 地址；Codex 的地址在 `model_providers_toml` 里。
    pub custom_base_url: Option<String>,
    pub model_providers_toml: Option<String>,
    /// 自定义 API 自己的 HTTP 代理；海外中转站等需要代理，留空直连。
    pub custom_proxy_url: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub official_accounts: Vec<Profile>,
    pub custom_providers: Vec<Profile>,
    pub selected_official: String,
    pub selected_custom: String,
    pub official_model: Option<String>,
    pub official_effort: Option<String>,
    pub official_proxy_url: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Libraries {
    pub codex: Library,
    pub claude: Library,
}
#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomFields {
    pub provider_name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub effort: String,
    #[serde(default)]
    pub proxy_url: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentDraft {
    pub agent: Agent,
    pub mode: Mode,
    pub library: Library,
    pub custom_fields: BTreeMap<String, CustomFields>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    #[serde(default)]
    pub codex: AgentDraft,
    #[serde(default)]
    pub claude: AgentDraft,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct View {
    pub revision: String,
    pub draft: Draft,
    pub roots: Roots,
}
pub struct Session {
    pub store: Store,
    pub baseline: Snapshot,
    pub original: Draft,
}

pub(crate) fn text(bytes: &Option<Vec<u8>>) -> Result<&str> {
    Ok(std::str::from_utf8(bytes.as_deref().unwrap_or_default())?.trim_start_matches('\u{feff}'))
}
pub(crate) fn json(bytes: &Option<Vec<u8>>, label: &str) -> Result<Value> {
    let text = text(bytes)?;
    if text.trim().is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    let v: Value = serde_json::from_str(text).with_context(|| format!("{label} 格式无效"))?;
    if !v.is_object() {
        bail!("{label} 必须是对象")
    }
    Ok(v)
}
pub(crate) fn nonempty(v: &Value) -> bool {
    v.as_str().is_some_and(|s| !s.trim().is_empty())
}
fn validate_libraries<'a>(libraries: impl IntoIterator<Item = &'a Library>) -> Result<()> {
    let mut ids = HashSet::new();
    for library in libraries {
        for mode in [Mode::Official, Mode::Custom] {
            let profiles = library.profiles(mode);
            for profile in profiles {
                if profile.id.len() != 8
                    || !profile
                        .id
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                {
                    bail!("配置 ID 必须是 8 位小写十六进制字符")
                }
                if !ids.insert(&profile.id) {
                    bail!(
                        "配置 ID {} 重复；所有客户端和模式的 ID 必须全局唯一",
                        profile.id
                    )
                }
            }
            if (profiles.is_empty() && !library.selection(mode).is_empty())
                || (!profiles.is_empty() && library.selected(mode).is_none())
            {
                bail!("选中的配置不存在")
            }
        }
    }
    Ok(())
}

impl Library {
    pub fn profiles(&self, mode: Mode) -> &Vec<Profile> {
        if mode == Mode::Official {
            &self.official_accounts
        } else {
            &self.custom_providers
        }
    }
    pub fn profiles_mut(&mut self, mode: Mode) -> &mut Vec<Profile> {
        if mode == Mode::Official {
            &mut self.official_accounts
        } else {
            &mut self.custom_providers
        }
    }
    pub fn selection(&self, mode: Mode) -> &str {
        if mode == Mode::Official {
            &self.selected_official
        } else {
            &self.selected_custom
        }
    }
    pub fn select(&mut self, mode: Mode, id: String) {
        if mode == Mode::Official {
            self.selected_official = id;
        } else {
            self.selected_custom = id;
        }
    }
    pub fn selected(&self, mode: Mode) -> Option<&Profile> {
        self.profiles(mode)
            .iter()
            .find(|p| p.id == self.selection(mode))
    }
    pub fn validate(&self) -> Result<()> {
        validate_libraries([self])
    }
    /// 该模式生效的代理：官方账号共用客户端代理，自定义 API 使用选中配置自己的代理。
    pub fn proxy(&self, mode: Mode) -> &str {
        match mode {
            Mode::Official => self.official_proxy_url.as_deref(),
            Mode::Custom => self
                .selected(Mode::Custom)
                .and_then(|p| p.custom_proxy_url.as_deref()),
        }
        .unwrap_or_default()
    }
}
impl AgentDraft {
    pub fn new(agent: Agent, mode: Mode, library: Library) -> Result<Self> {
        let custom_fields = library
            .custom_providers
            .iter()
            .map(|p| Ok((p.id.clone(), CustomFields::read(agent, p)?)))
            .collect::<Result<_>>()?;
        Ok(Self {
            agent,
            mode,
            library,
            custom_fields,
        })
    }
    pub fn add(&mut self, mode: Mode, name: &str, copy: Option<&str>) -> Result<String> {
        if name.trim().is_empty() {
            bail!("请输入配置名称")
        }
        let mut p = if let Some(id) = copy {
            if mode == Mode::Official {
                bail!("官方账号请通过登录添加")
            }
            self.library
                .profiles(mode)
                .iter()
                .find(|p| p.id == id)
                .context("待复制配置不存在")?
                .clone()
        } else {
            Profile::default()
        };
        let fields = copy
            .and_then(|id| self.custom_fields.get(id))
            .cloned()
            .unwrap_or_else(|| CustomFields::blank(self.agent));
        p.id = loop {
            let id = uuid::Uuid::new_v4().simple().to_string()[..8].to_owned();
            if !self
                .library
                .official_accounts
                .iter()
                .chain(&self.library.custom_providers)
                .any(|p| p.id == id)
            {
                break id;
            }
        };
        p.name = Some(name.trim().into());
        let id = p.id.clone();
        self.library.profiles_mut(mode).push(p);
        self.library.select(mode, id.clone());
        if mode == Mode::Custom {
            self.custom_fields.insert(id.clone(), fields);
        }
        Ok(id)
    }
    pub fn delete(&mut self, mode: Mode, id: &str) -> Result<()> {
        let list = self.library.profiles_mut(mode);
        let index = list
            .iter()
            .position(|p| p.id == id)
            .context("待删除配置不存在")?;
        list.remove(index);
        let first = list.first().map(|p| p.id.clone()).unwrap_or_default();
        if self.library.selection(mode) == id {
            self.library.select(mode, first);
        }
        if mode == Mode::Custom {
            self.custom_fields.remove(id);
        }
        Ok(())
    }
    /// 把表单写回配置并规范化代理，返回当前模式选中的配置。
    pub(crate) fn resolve(&mut self) -> Result<Profile> {
        self.library.validate()?;
        let agent = self.agent;
        let fields = std::mem::take(&mut self.custom_fields);
        for p in &mut self.library.custom_providers {
            fields.get(&p.id).context("API 表单缺失")?.apply(agent, p)?;
        }
        self.custom_fields = fields;
        let proxy = normalize_proxy(
            self.library
                .official_proxy_url
                .as_deref()
                .unwrap_or_default(),
        )?;
        self.library.official_proxy_url = Some(proxy);
        Ok(self
            .library
            .selected(self.mode)
            .context("请先添加并选择配置")?
            .clone())
    }
}
impl Draft {
    pub fn agent(&self, agent: Agent) -> &AgentDraft {
        match agent {
            Agent::Codex => &self.codex,
            Agent::Claude => &self.claude,
        }
    }
    pub fn agent_mut(&mut self, agent: Agent) -> &mut AgentDraft {
        match agent {
            Agent::Codex => &mut self.codex,
            Agent::Claude => &mut self.claude,
        }
    }
}
impl CustomFields {
    pub fn blank(agent: Agent) -> Self {
        match agent {
            Agent::Codex => Self {
                provider_name: "custom".into(),
                effort: "medium".into(),
                ..Default::default()
            },
            Agent::Claude => Self::default(),
        }
    }
    pub fn read(agent: Agent, p: &Profile) -> Result<Self> {
        match agent {
            Agent::Codex => codex::fields(p),
            Agent::Claude => Ok(claude::fields(p)),
        }
    }
    pub fn validate(&self, agent: Agent) -> Result<()> {
        match agent {
            Agent::Codex => codex::validate_fields(self),
            Agent::Claude => claude::validate_fields(self),
        }
    }
    pub fn apply(&self, agent: Agent, p: &mut Profile) -> Result<()> {
        match agent {
            Agent::Codex => codex::apply_fields(self, p)?,
            Agent::Claude => claude::apply_fields(self, p),
        }
        p.custom_proxy_url = Some(normalize_proxy(&self.proxy_url)?);
        Ok(())
    }
}

impl Session {
    pub fn open(store: Store) -> Result<Self> {
        let baseline = store.snapshot()?;
        let libraries = libraries(&baseline)?;
        validate_libraries([&libraries.codex, &libraries.claude])?;
        let codex = codex::capture(libraries.codex, &baseline)?;
        let claude = claude::capture(libraries.claude, &baseline)?;
        let original = Draft { codex, claude };
        Ok(Self {
            store,
            baseline,
            original,
        })
    }
    pub fn view(&self) -> View {
        View {
            revision: revision(&self.baseline),
            draft: self.original.clone(),
            roots: self.store.roots.clone(),
        }
    }
    pub fn check(&self, rev: &str) -> Result<()> {
        if rev != revision(&self.baseline) {
            bail!("读取版本已过期，请重新读取配置")
        }
        self.store.check(&self.baseline)
    }
    pub fn save(&mut self, rev: &str, draft: &Draft) -> Result<View> {
        self.save_scope(rev, draft, None)
    }
    pub fn save_for(&mut self, rev: &str, draft: &Draft, agent: Agent) -> Result<View> {
        self.save_scope(rev, draft, Some(agent))
    }
    fn save_scope(&mut self, rev: &str, draft: &Draft, agent: Option<Agent>) -> Result<View> {
        let _lock = self.store.lock()?;
        self.check(rev)?;
        validate_libraries([&draft.codex.library, &draft.claude.library])?;
        let mut draft = draft.clone();
        let mut after = self.baseline.clone();
        let codex_changed = draft.codex != self.original.codex;
        let claude_changed = draft.claude != self.original.claude;
        // 当前客户端即使没有修改也执行校验；其他客户端只处理实际草稿变更。
        if codex_changed || agent.is_none_or(|a| a == Agent::Codex) {
            codex::apply(&mut after, &self.baseline, &mut draft.codex)?;
        }
        if claude_changed || agent.is_none_or(|a| a == Agent::Claude) {
            claude::apply(&mut after, &self.baseline, &mut draft.claude)?;
        }
        after[LIBRARY] = Some(serde_json::to_vec_pretty(&Libraries {
            codex: draft.codex.library.clone(),
            claude: draft.claude.library.clone(),
        })?);
        self.store.commit(&self.baseline, &after)?;
        self.baseline = after;
        self.original = draft;
        Ok(self.view())
    }
    pub fn launch_settings(&self, rev: &str, draft: &Draft) -> Result<crate::launch::Settings> {
        self.launch_settings_for(rev, draft, Agent::Codex)
    }
    pub fn launch_settings_for(
        &self,
        rev: &str,
        draft: &Draft,
        agent: Agent,
    ) -> Result<crate::launch::Settings> {
        self.check(rev)?;
        if draft != &self.original {
            bail!("有未保存修改，请先保存配置")
        }
        let mode = match agent {
            Agent::Codex => codex::launch_check(&self.baseline)?,
            Agent::Claude => claude::launch_check(&self.baseline)?,
        };
        Ok(crate::launch::Settings {
            agent,
            mode,
            proxy: normalize_proxy(self.original.agent(agent).library.proxy(mode))?,
            roots: self.store.roots.clone(),
        })
    }
}

fn libraries(baseline: &Snapshot) -> Result<Libraries> {
    if let Some(bytes) = &baseline[LIBRARY] {
        let v: Value = serde_json::from_slice(bytes).context("profiles.json 格式无效")?;
        if !v["codex"].is_object() || !v["claude"].is_object() {
            bail!("profiles.json 配置结构不正确")
        }
        return Ok(serde_json::from_value(v)?);
    }
    Ok(Libraries::default())
}

pub fn normalize_proxy(raw: &str) -> Result<String> {
    if raw.trim().is_empty() {
        return Ok(String::new());
    }
    let u = url::Url::parse(raw.trim()).context("代理地址无效")?;
    if u.scheme() != "http"
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.path() != "/"
        || u.query().is_some()
        || u.fragment().is_some()
        || u.port() == Some(0)
    {
        bail!("代理须为 http://主机:端口，不含账号密码、路径或查询参数")
    }
    Ok(u.as_str().trim_end_matches('/').into())
}
pub fn proxy_variables(proxy: &str) -> Vec<(&'static str, String)> {
    vec![
        ("HTTP_PROXY", proxy.into()),
        ("HTTPS_PROXY", proxy.into()),
        ("ALL_PROXY", proxy.into()),
        ("NO_PROXY", "localhost,127.0.0.1,::1".into()),
        ("NODE_USE_ENV_PROXY", "1".into()),
    ]
}
/// 旧标记来自 2.x，改名后仍要清理，避免遗留的代理变量继续生效。
const BLOCKS: [(&str, &str); 2] = [
    ("# BEGIN ACS PROXY", "# END ACS PROXY"),
    (
        "# BEGIN CHATGPT API ONLY PROXY",
        "# END CHATGPT API ONLY PROXY",
    ),
];
fn strip_block(original: &str, begin: &str, end: &str) -> Result<String> {
    let mut pos = 0;
    let mut markers = Vec::new();
    for line in original.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\r', '\n']);
        if bare == begin || bare == end {
            markers.push((pos, pos + line.len(), bare));
        }
        pos += line.len();
    }
    if markers.is_empty() {
        return Ok(original.to_owned());
    }
    if markers.len() != 2 || markers[0].2 != begin || markers[1].2 != end {
        bail!(".env 的代理标记不完整、顺序错误或重复")
    }
    let mut start = markers[0].0;
    if start > 0 && original.as_bytes()[start - 1] == b'\n' {
        start -= 1;
        if start > 0 && original.as_bytes()[start - 1] == b'\r' {
            start -= 1;
        }
    }
    let prefix = &original[..start];
    let suffix = &original[markers[1].1..];
    Ok(format!(
        "{prefix}{}{suffix}",
        if !prefix.is_empty()
            && !suffix.is_empty()
            && !prefix.ends_with('\n')
            && !suffix.starts_with('\n')
        {
            "\n"
        } else {
            ""
        }
    ))
}
pub fn proxy_env(original: &str, proxy: &str) -> Result<String> {
    let mut clean = original.to_owned();
    for (begin, end) in BLOCKS {
        clean = strip_block(&clean, begin, end)?;
    }
    if proxy.is_empty() {
        return Ok(clean);
    }
    let (begin, end) = BLOCKS[0];
    let nl = if clean.contains("\r\n") { "\r\n" } else { "\n" };
    clean.push_str(nl);
    clean.push_str(begin);
    clean.push_str(nl);
    for (k, v) in proxy_variables(proxy) {
        clean.push_str(&format!("{k}={v}{nl}"));
    }
    clean.push_str(end);
    clean.push_str(nl);
    Ok(clean)
}
