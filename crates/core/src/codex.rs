use crate::{
    config::{
        json, nonempty, proxy_env, text, Agent, AgentDraft, CustomFields, Library, Mode, Profile,
    },
    storage::{Snapshot, CODEX_AUTH, CODEX_CONFIG, CODEX_ENV},
};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::{json as jsonv, Value};
use toml_edit::{value, DocumentMut, Item, Table};

pub fn parse_doc(text: &str) -> Result<DocumentMut> {
    text.parse().context("config.toml 格式无效")
}
fn string(doc: &DocumentMut, key: &str) -> String {
    doc.get(key)
        .and_then(Item::as_str)
        .unwrap_or_default()
        .into()
}
pub fn mode(doc: &DocumentMut) -> Mode {
    if doc
        .get("model_provider")
        .and_then(Item::as_str)
        .unwrap_or("openai")
        == "openai"
    {
        Mode::Official
    } else {
        Mode::Custom
    }
}
pub fn valid_tokens(v: &Value) -> bool {
    v["auth_mode"] == "chatgpt"
        && nonempty(&v["tokens"]["access_token"])
        && nonempty(&v["tokens"]["refresh_token"])
        && nonempty(&v["tokens"]["account_id"])
}

pub fn fields(p: &Profile) -> Result<CustomFields> {
    let d = parse_doc(p.model_providers_toml.as_deref().unwrap_or_default())?;
    let get = |key| {
        d.get("model_providers")
            .and_then(|v| v.get("custom"))
            .and_then(|v| v.get(key))
            .and_then(Item::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Ok(CustomFields {
        provider_name: get("name"),
        base_url: get("base_url"),
        api_key: p.custom_key.clone().unwrap_or_default(),
        model: p.custom_model.clone().unwrap_or_default(),
        effort: p.custom_effort.clone().unwrap_or_else(|| "medium".into()),
    })
}
pub fn validate_fields(f: &CustomFields) -> Result<()> {
    let url = url::Url::parse(&f.base_url).context("API 地址无效")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.path().ends_with("/v1")
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        bail!("Codex 的 API 地址须为 https://主机/v1，可包含 /v1 前的路径")
    }
    if [&f.provider_name, &f.api_key, &f.model, &f.effort]
        .iter()
        .any(|s| s.trim().is_empty())
    {
        bail!("请填写提供者名称、API Key、模型和思考层级")
    }
    Ok(())
}
pub fn apply_fields(f: &CustomFields, p: &mut Profile) -> Result<()> {
    let mut doc = parse_doc(p.model_providers_toml.as_deref().unwrap_or_default())?;
    ensure_custom(&mut doc)?;
    doc["model_providers"]["custom"]["name"] = value(&f.provider_name);
    doc["model_providers"]["custom"]["base_url"] = value(&f.base_url);
    doc["model_providers"]["custom"]["wire_api"] = value("responses");
    doc["model_providers"]["custom"]["requires_openai_auth"] = value(true);
    p.model_providers_toml = Some(provider_snapshot(&doc));
    p.custom_key = Some(f.api_key.clone());
    p.custom_model = Some(f.model.clone());
    p.custom_effort = Some(f.effort.clone());
    Ok(())
}
fn ensure_custom(doc: &mut DocumentMut) -> Result<()> {
    if doc.get("model_providers").is_none() {
        let mut table = Table::new();
        table.set_implicit(true);
        doc["model_providers"] = Item::Table(table);
    }
    if !doc["model_providers"].is_table() {
        bail!("model_providers 必须使用 TOML 表")
    }
    if doc["model_providers"].get("custom").is_none() {
        doc["model_providers"]["custom"] = Item::Table(Table::new());
    }
    if !doc["model_providers"]["custom"].is_table() {
        bail!("model_providers.custom 必须使用 TOML 表")
    }
    Ok(())
}
fn provider_snapshot(doc: &DocumentMut) -> String {
    let mut out = DocumentMut::new();
    if let Some(p) = doc.get("model_providers") {
        out["model_providers"] = p.clone();
    }
    out.to_string()
}

pub fn capture(mut library: Library, baseline: &Snapshot) -> Result<AgentDraft> {
    library.validate()?;
    let doc = parse_doc(text(&baseline[CODEX_CONFIG])?)?;
    let credentials = json(&baseline[CODEX_AUTH], "auth.json")?;
    let active = mode(&doc);
    capture_official(&mut library, &doc, &credentials)?;
    if active == Mode::Custom {
        let mut p = Profile {
            model_providers_toml: Some(provider_snapshot(&doc)),
            custom_key: credentials["OPENAI_API_KEY"].as_str().map(str::to_owned),
            custom_model: Some(string(&doc, "model")),
            custom_effort: Some(string(&doc, "model_reasoning_effort")),
            ..Default::default()
        };
        let f = fields(&p)?;
        let existing = library.custom_providers.iter().position(|old| {
            fields(old).is_ok_and(|old| old.base_url == f.base_url && old.api_key == f.api_key)
        });
        if let Some(i) = existing {
            p.id = library.custom_providers[i].id.clone();
            p.name = library.custom_providers[i].name.clone();
            p.extra = library.custom_providers[i].extra.clone();
            library.custom_providers[i] = p.clone();
            library.selected_custom = p.id;
        }
    }
    AgentDraft::new(Agent::Codex, active, library)
}
fn capture_official(library: &mut Library, doc: &DocumentMut, credentials: &Value) -> Result<()> {
    if mode(doc) == Mode::Official {
        library.official_model = doc.get("model").and_then(Item::as_str).map(str::to_owned);
        library.official_effort = doc
            .get("model_reasoning_effort")
            .and_then(Item::as_str)
            .map(str::to_owned);
    }
    if !valid_tokens(credentials) {
        if credentials["tokens"].is_object()
            && credentials["tokens"]
                .as_object()
                .is_some_and(|t| !t.is_empty())
        {
            bail!("官方凭据不完整；未视为退出登录，也未覆盖配置库")
        }
        if mode(doc) == Mode::Official {
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
    let account_id = credentials["tokens"]["account_id"].as_str().unwrap();
    let claims = credentials["tokens"]["id_token"]
        .as_str()
        .and_then(|s| s.split('.').nth(1))
        .and_then(|s| URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .unwrap_or(Value::Null);
    let index = library
        .official_accounts
        .iter()
        .position(|p| p.account_id.as_deref() == Some(account_id))
        .or_else(|| {
            library
                .official_accounts
                .iter()
                .position(|p| p.id == library.selected_official && p.account_id.is_none())
        });
    let Some(i) = index else { return Ok(()) };
    let p = &mut library.official_accounts[i];
    p.account_id = Some(account_id.into());
    p.email = claims["email"].as_str().map(str::to_owned);
    p.account_name = claims["name"].as_str().map(str::to_owned);
    p.official_auth = Some(serde_json::to_string(credentials)?);
    library.selected_official = p.id.clone();
    Ok(())
}

pub fn apply(after: &mut Snapshot, baseline: &Snapshot, draft: &mut AgentDraft) -> Result<()> {
    let mut doc = parse_doc(text(&baseline[CODEX_CONFIG])?)?;
    validate_conflicts(&doc, draft.mode)?;
    let target = draft.resolve()?;
    let library = &draft.library;
    let mut next_auth;
    doc.remove("model_providers");
    if draft.mode == Mode::Official {
        next_auth = json(
            &Some(
                target
                    .official_auth
                    .clone()
                    .unwrap_or("{\"auth_mode\":\"chatgpt\",\"OPENAI_API_KEY\":null}".into())
                    .into_bytes(),
            ),
            "auth.json",
        )?;
        if next_auth["auth_mode"] != "chatgpt" {
            bail!("官方凭据模式无效")
        }
        if !next_auth["tokens"].is_null() && !valid_tokens(&next_auth) {
            bail!("官方凭据不完整，请核对 auth.json")
        }
        next_auth["OPENAI_API_KEY"] = Value::Null;
        doc["model_provider"] = value("openai");
        for (k, v) in [
            ("model", &library.official_model),
            ("model_reasoning_effort", &library.official_effort),
        ] {
            doc.remove(k);
            if let Some(v) = v.as_deref().filter(|v| !v.trim().is_empty()) {
                doc[k] = value(v);
            }
        }
        ensure_custom(&mut doc)?;
        let name = library
            .selected(Mode::Custom)
            .map(fields)
            .transpose()?
            .map(|f| f.provider_name)
            .filter(|s| !s.is_empty())
            .unwrap_or("custom".into());
        doc["model_providers"]["custom"]["name"] = value(name);
        doc["model_providers"]["custom"]["wire_api"] = value("responses");
        doc["model_providers"]["custom"]["requires_openai_auth"] = value(true);
    } else {
        let f = fields(&target)?;
        f.validate(Agent::Codex)?;
        let providers = parse_doc(target.model_providers_toml.as_deref().unwrap_or_default())?;
        doc["model_providers"] = providers["model_providers"].clone();
        validate_custom_overrides(&doc)?;
        doc["model_provider"] = value("custom");
        doc["model"] = value(&f.model);
        doc["model_reasoning_effort"] = value(&f.effort);
        next_auth = jsonv!({"auth_mode":"apikey","OPENAI_API_KEY":f.api_key});
    }
    let env = proxy_env(
        text(&baseline[CODEX_ENV])?,
        if draft.mode == Mode::Official {
            library.official_proxy_url.as_deref().unwrap_or_default()
        } else {
            ""
        },
    )?;
    after[CODEX_CONFIG] = Some(doc.to_string().into_bytes());
    after[CODEX_AUTH] = Some(serde_json::to_vec_pretty(&next_auth)?);
    after[CODEX_ENV] = if baseline[CODEX_ENV].is_none() && env.is_empty() {
        None
    } else {
        Some(env.into_bytes())
    };
    Ok(())
}
pub fn launch_check(baseline: &Snapshot) -> Result<Mode> {
    let doc = parse_doc(text(&baseline[CODEX_CONFIG])?)?;
    let active = mode(&doc);
    validate_conflicts(&doc, active)?;
    let credentials = json(&baseline[CODEX_AUTH], "auth.json")?;
    if active == Mode::Custom {
        if string(&doc, "model_provider") != "custom" {
            bail!("活动 provider 不是 custom，请先保存配置")
        }
        let p = Profile {
            model_providers_toml: Some(provider_snapshot(&doc)),
            custom_key: credentials["OPENAI_API_KEY"].as_str().map(str::to_owned),
            custom_model: Some(string(&doc, "model")),
            custom_effort: Some(string(&doc, "model_reasoning_effort")),
            ..Default::default()
        };
        fields(&p)?.validate(Agent::Codex)?;
        validate_custom_overrides(&doc)?;
        let provider = doc.get("model_providers").and_then(|p| p.get("custom"));
        if credentials["auth_mode"] != "apikey"
            || provider
                .and_then(|p| p.get("wire_api"))
                .and_then(Item::as_str)
                != Some("responses")
            || provider
                .and_then(|p| p.get("requires_openai_auth"))
                .and_then(Item::as_bool)
                != Some(true)
        {
            bail!("活动 API 认证或协议不完整，请先保存配置")
        }
    } else {
        if nonempty(&credentials["OPENAI_API_KEY"])
            || credentials.get("auth_mode").is_some_and(|v| v != "chatgpt")
        {
            bail!("活动认证不是官方模式，请先保存配置")
        }
        if !credentials["tokens"].is_null() && !valid_tokens(&credentials) {
            bail!("官方凭据不完整，请核对 auth.json")
        }
    }
    Ok(active)
}
pub fn validate_conflicts(doc: &DocumentMut, mode: Mode) -> Result<()> {
    for key in ["forced_login_method", "profile"] {
        if doc.contains_key(key) {
            bail!("配置冲突：{key}，请先核对 config.toml")
        }
    }
    if doc
        .get("cli_auth_credentials_store")
        .is_some_and(|i| i.as_str() != Some("file"))
    {
        bail!("cli_auth_credentials_store 必须为 file 或不设置")
    }
    if mode == Mode::Official {
        for key in ["chatgpt_base_url", "openai_base_url"] {
            if doc.contains_key(key) {
                bail!("配置冲突：{key} 覆盖官方路由")
            }
        }
    }
    if doc.get("model_providers").is_some_and(|p| !p.is_table()) {
        bail!("model_providers 必须使用 TOML 表")
    }
    Ok(())
}
fn validate_custom_overrides(doc: &DocumentMut) -> Result<()> {
    if let Some(p) = doc.get("model_providers").and_then(|p| p.get("custom")) {
        for key in [
            "env_key",
            "experimental_bearer_token",
            "env_key_instructions",
        ] {
            if p.get(key).is_some() {
                bail!("custom.{key} 与 auth.json 认证冲突，请先核对")
            }
        }
        for key in ["http_headers", "env_http_headers"] {
            if let Some(headers) = p.get(key).and_then(Item::as_table_like) {
                if headers.iter().any(|(k, _)| {
                    k.eq_ignore_ascii_case("authorization") || k.eq_ignore_ascii_case("api-key")
                }) {
                    bail!("custom.{key} 存在认证覆盖")
                }
            }
        }
    }
    Ok(())
}
