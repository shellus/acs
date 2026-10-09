use crate::{
    config::{
        json, nonempty, normalize_proxy, proxy_variables, Agent, AgentDraft, CustomFields, Library,
        Mode, Profile,
    },
    storage::{Snapshot, CLAUDE_ACCOUNT, CLAUDE_CREDENTIALS, CLAUDE_SETTINGS},
};
use anyhow::{bail, Context, Result};
use serde_json::{json as jsonv, Value};

const TOKEN: &str = "ANTHROPIC_AUTH_TOKEN";
const BASE_URL: &str = "ANTHROPIC_BASE_URL";

fn env<'a>(settings: &'a Value, key: &str) -> &'a Value {
    &settings["env"][key]
}
fn set_env(settings: &mut Value, key: &str, value: Option<&str>) {
    if value.is_none() && !settings["env"].is_object() {
        return;
    }
    if !settings["env"].is_object() {
        settings["env"] = Value::Object(Default::default());
    }
    let map = settings["env"].as_object_mut().unwrap();
    match value {
        Some(v) => {
            map.insert(key.into(), Value::String(v.into()));
        }
        None => {
            map.remove(key);
        }
    }
    if settings["env"].as_object().is_some_and(|m| m.is_empty()) {
        settings.as_object_mut().unwrap().remove("env");
    }
}
fn set_model(settings: &mut Value, model: Option<&str>) {
    let map = settings.as_object_mut().unwrap();
    match model.map(str::trim).filter(|m| !m.is_empty()) {
        Some(m) => {
            map.insert("model".into(), Value::String(m.into()));
        }
        None => {
            map.remove("model");
        }
    }
}
/// 代理由本程序接管：写入当前模式生效的代理，留空时移除受管键。
fn set_proxy(settings: &mut Value, proxy: &str) {
    for (key, value) in proxy_variables(proxy) {
        set_env(
            settings,
            key,
            if proxy.is_empty() { None } else { Some(&value) },
        );
    }
}
pub fn valid_tokens(credentials: &Value) -> bool {
    let oauth = &credentials["claudeAiOauth"];
    nonempty(&oauth["accessToken"]) && nonempty(&oauth["refreshToken"])
}
fn active_mode(settings: &Value) -> Mode {
    if nonempty(env(settings, TOKEN)) {
        Mode::Custom
    } else {
        Mode::Official
    }
}

pub fn fields(p: &Profile) -> CustomFields {
    CustomFields {
        provider_name: String::new(),
        base_url: p.custom_base_url.clone().unwrap_or_default(),
        api_key: p.custom_key.clone().unwrap_or_default(),
        model: p.custom_model.clone().unwrap_or_default(),
        effort: String::new(),
        proxy_url: p.custom_proxy_url.clone().unwrap_or_default(),
    }
}
pub fn validate_fields(f: &CustomFields) -> Result<()> {
    let url = url::Url::parse(&f.base_url).context("API 地址无效")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        bail!("Claude 的 API 地址须为 https://主机，可包含路径，不含查询参数或账号密码")
    }
    if f.api_key.trim().is_empty() {
        bail!("请填写 API Key")
    }
    Ok(())
}
pub fn apply_fields(f: &CustomFields, p: &mut Profile) {
    p.custom_base_url = Some(f.base_url.trim().trim_end_matches('/').into());
    p.custom_key = Some(f.api_key.trim().into());
    p.custom_model = Some(f.model.trim().into());
}

pub fn capture(mut library: Library, baseline: &Snapshot) -> Result<AgentDraft> {
    library.validate()?;
    let settings = json(&baseline[CLAUDE_SETTINGS], "settings.json")?;
    let credentials = json(&baseline[CLAUDE_CREDENTIALS], ".credentials.json")?;
    let account = match &baseline[CLAUDE_ACCOUNT] {
        Some(bytes) => {
            serde_json::from_slice(bytes).context(".claude.json 的 oauthAccount 无效")?
        }
        None => Value::Null,
    };
    let active = active_mode(&settings);
    if active == Mode::Official {
        library.official_model = settings["model"].as_str().map(str::to_owned);
        // 接管用户原先手写在 settings.json 里的代理。
        library.official_proxy_url = env(&settings, "HTTP_PROXY").as_str().map(str::to_owned);
    }
    capture_official(&mut library, &credentials, &account, active)?;
    if active == Mode::Custom {
        let mut p = Profile {
            custom_base_url: env(&settings, BASE_URL).as_str().map(str::to_owned),
            custom_key: env(&settings, TOKEN).as_str().map(str::to_owned),
            custom_model: settings["model"].as_str().map(str::to_owned),
            custom_proxy_url: env(&settings, "HTTP_PROXY").as_str().map(str::to_owned),
            ..Default::default()
        };
        let f = fields(&p);
        let existing = library.custom_providers.iter().position(|old| {
            let old = fields(old);
            old.base_url == f.base_url && old.api_key == f.api_key
        });
        if let Some(i) = existing {
            p.id = library.custom_providers[i].id.clone();
            p.name = library.custom_providers[i].name.clone();
            p.extra = library.custom_providers[i].extra.clone();
            library.custom_providers[i] = p.clone();
            library.selected_custom = p.id;
        }
    }
    AgentDraft::new(Agent::Claude, active, library)
}
fn capture_official(
    library: &mut Library,
    credentials: &Value,
    account: &Value,
    active: Mode,
) -> Result<()> {
    if !valid_tokens(credentials) {
        // 空字段或过期字段表示已退出登录；读取配置不能因此失败。
        if active == Mode::Official {
            let selected = library.selected_official.clone();
            if let Some(p) = library
                .official_accounts
                .iter_mut()
                .find(|p| p.id == selected)
            {
                p.official_auth = None;
            }
        }
        return Ok(());
    }
    let account_id = account["accountUuid"].as_str();
    let index = account_id
        .and_then(|id| {
            library
                .official_accounts
                .iter()
                .position(|p| p.account_id.as_deref() == Some(id))
        })
        .or_else(|| {
            library
                .official_accounts
                .iter()
                .position(|p| p.id == library.selected_official && p.account_id.is_none())
        });
    let Some(i) = index else { return Ok(()) };
    let p = &mut library.official_accounts[i];
    p.account_id = account_id.map(str::to_owned);
    p.email = account["emailAddress"].as_str().map(str::to_owned);
    p.account_name = account["displayName"].as_str().map(str::to_owned);
    p.official_auth = Some(serde_json::to_string(
        &jsonv!({"credentials": credentials, "account": account}),
    )?);
    library.selected_official = p.id.clone();
    Ok(())
}

pub fn apply(after: &mut Snapshot, baseline: &Snapshot, draft: &mut AgentDraft) -> Result<()> {
    if draft.library.official_accounts.is_empty() && draft.library.custom_providers.is_empty() {
        return Ok(());
    }
    let mut settings = json(&baseline[CLAUDE_SETTINGS], "settings.json")?;
    let target = draft.resolve()?;
    if draft.mode == Mode::Official {
        let saved = match &target.official_auth {
            Some(text) => serde_json::from_str::<Value>(text).context("官方凭据格式无效")?,
            None => Value::Null,
        };
        let credentials = &saved["credentials"];
        if !credentials.is_null() && !valid_tokens(credentials) {
            bail!("Claude 官方凭据不完整，请核对 .credentials.json")
        }
        after[CLAUDE_CREDENTIALS] = credentials
            .is_object()
            .then(|| serde_json::to_vec_pretty(credentials))
            .transpose()?;
        after[CLAUDE_ACCOUNT] = saved["account"]
            .is_object()
            .then(|| serde_json::to_vec(&saved["account"]))
            .transpose()?;
        set_env(&mut settings, TOKEN, None);
        set_env(&mut settings, BASE_URL, None);
        set_model(&mut settings, draft.library.official_model.as_deref());
    } else {
        let f = fields(&target);
        f.validate(Agent::Claude)?;
        set_env(&mut settings, TOKEN, Some(&f.api_key));
        set_env(&mut settings, BASE_URL, Some(&f.base_url));
        set_model(&mut settings, Some(&f.model));
        // 官方凭据原样保留，切回官方账号时不必重新登录。
    }
    set_proxy(&mut settings, draft.library.proxy(draft.mode));
    let mut bytes = serde_json::to_vec_pretty(&settings)?;
    bytes.push(b'\n');
    after[CLAUDE_SETTINGS] = Some(bytes);
    Ok(())
}
pub fn launch_check(baseline: &Snapshot) -> Result<Mode> {
    let settings = json(&baseline[CLAUDE_SETTINGS], "settings.json")?;
    let credentials = json(&baseline[CLAUDE_CREDENTIALS], ".credentials.json")?;
    let active = active_mode(&settings);
    if active == Mode::Custom {
        let p = Profile {
            custom_base_url: env(&settings, BASE_URL).as_str().map(str::to_owned),
            custom_key: env(&settings, TOKEN).as_str().map(str::to_owned),
            ..Default::default()
        };
        fields(&p).validate(Agent::Claude)?;
    } else {
        if nonempty(env(&settings, BASE_URL)) {
            bail!("settings.json 仍设置了 {BASE_URL}，请先保存配置")
        }
        if credentials["claudeAiOauth"].is_object() && !valid_tokens(&credentials) {
            bail!("Claude 官方凭据不完整，请核对 .credentials.json")
        }
    }
    if let Some(proxy) = env(&settings, "HTTP_PROXY").as_str() {
        normalize_proxy(proxy)?;
    }
    Ok(active)
}
