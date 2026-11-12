//! Portable custom configuration bundles. Import changes only the library file.
use crate::{config::Libraries, storage::LIBRARY, Agent, Mode, Profile, Session, Store};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomBundle {
    pub format: String,
    pub codex: Vec<Profile>,
    pub claude: Vec<Profile>,
}
fn custom_only(p: &Profile) -> Result<()> {
    if p.official_auth.is_some()
        || p.account_id.is_some()
        || p.email.is_some()
        || p.account_name.is_some()
    {
        bail!("自定义配置包不得包含官方账号或令牌")
    }
    Ok(())
}
fn same(agent: Agent, a: &Profile, b: &Profile) -> Result<bool> {
    if crate::CustomFields::read(agent, a)? != crate::CustomFields::read(agent, b)? {
        return Ok(false);
    }
    if agent == Agent::Codex {
        let parse = |p: &Profile| {
            toml_edit::de::from_str::<serde_json::Value>(
                p.model_providers_toml.as_deref().unwrap_or_default(),
            )
        };
        return Ok(parse(a)? == parse(b)?);
    }
    Ok(true)
}
pub fn export(store: Store, file: &Path) -> Result<usize> {
    let s = Session::open(store)?;
    let bundle = CustomBundle {
        format: "acs-custom-v1".into(),
        codex: s.original.codex.library.custom_providers,
        claude: s.original.claude.library.custom_providers,
    };
    for p in bundle.codex.iter().chain(&bundle.claude) {
        custom_only(p)?;
    }
    if file.exists() {
        bail!("导出文件已存在，请使用新的文件名")
    }
    crate::storage::write(file, Some(&serde_json::to_vec_pretty(&bundle)?))?;
    Ok(bundle.codex.len() + bundle.claude.len())
}
pub fn import(store: Store, file: &Path) -> Result<usize> {
    let bytes = std::fs::read(file)?;
    let bundle: CustomBundle = serde_json::from_slice(&bytes).context("自定义配置包格式无效")?;
    if bundle.format != "acs-custom-v1" {
        bail!("不支持的自定义配置包版本")
    }
    let mut s = Session::open(store)?;
    let _lock = s.store.lock()?;
    s.check(&s.view().revision)?;
    let mut ids = HashSet::new();
    for a in Agent::ALL {
        for m in [Mode::Official, Mode::Custom] {
            for p in s.original.agent(a).library.profiles(m) {
                ids.insert(p.id.clone());
            }
        }
    }
    let mut libraries = Libraries {
        codex: s.original.codex.library.clone(),
        claude: s.original.claude.library.clone(),
    };
    let mut added = 0;
    for (agent, incoming) in [(Agent::Codex, bundle.codex), (Agent::Claude, bundle.claude)] {
        let library = if agent == Agent::Codex {
            &mut libraries.codex
        } else {
            &mut libraries.claude
        };
        for mut p in incoming {
            custom_only(&p)?;
            crate::CustomFields::read(agent, &p)?.validate(agent)?;
            crate::config::normalize_proxy(p.custom_proxy_url.as_deref().unwrap_or_default())?;
            let duplicate = library.custom_providers.iter().any(|old| {
                let mut old = old.clone();
                old.id = p.id.clone();
                old == p
            });
            if duplicate {
                continue;
            }
            if p.id.len() != 8
                || !p
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
                || ids.contains(&p.id)
            {
                p.id = fresh_id(&mut ids);
            } else {
                ids.insert(p.id.clone());
            }
            library.custom_providers.push(p);
            added += 1;
        }
        let active = match agent {
            Agent::Codex => crate::codex::active_custom(&s.baseline)?,
            Agent::Claude => crate::claude::active_custom(&s.baseline)?,
        };
        if let Some(mut active) = active {
            let found = library
                .custom_providers
                .iter()
                .map(|p| same(agent, p, &active))
                .collect::<Result<Vec<_>>>()?
                .iter()
                .position(|same| *same);
            library.selected_custom = if let Some(i) = found {
                library.custom_providers[i].id.clone()
            } else {
                active.id = fresh_id(&mut ids);
                active.name = Some(format!("{} 原有生效配置", agent.label()));
                let id = active.id.clone();
                library.custom_providers.push(active);
                id
            };
        }
        if library.selected_custom.is_empty() {
            library.selected_custom = library
                .custom_providers
                .first()
                .map(|p| p.id.clone())
                .unwrap_or_default();
        }
        library.validate()?;
    }
    let mut after = s.baseline.clone();
    after[LIBRARY] = Some(serde_json::to_vec_pretty(&libraries)?);
    s.store.commit(&s.baseline, &after)?;
    s.baseline = after;
    Ok(added)
}
fn fresh_id(ids: &mut HashSet<String>) -> String {
    loop {
        let id = uuid::Uuid::new_v4().simple().to_string()[..8].to_owned();
        if ids.insert(id.clone()) {
            return id;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CustomFields, Roots};
    fn fixture(model: &str) -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(Roots::under(dir.path()));
        let mut s = Session::open(store.clone()).unwrap();
        let mut d = s.original.clone();
        for a in Agent::ALL {
            let target = d.agent_mut(a);
            let id = target.add(Mode::Custom, "example API", None).unwrap();
            target.mode = Mode::Custom;
            target.custom_fields.insert(
                id,
                CustomFields {
                    provider_name: "example provider".into(),
                    base_url: "https://api.example.com/v1".into(),
                    api_key: "example-key".into(),
                    model: model.into(),
                    effort: "medium".into(),
                    proxy_url: String::new(),
                },
            );
        }
        s.save(&s.view().revision, &d).unwrap();
        (dir, store)
    }
    #[test]
    fn portable_custom_import_preserves_all_active_bytes_unknown_fields_and_is_idempotent() {
        let (source_dir, source) = fixture("example-incoming-model");
        let file = source_dir.path().join("bundle.json");
        let mut s = Session::open(source.clone()).unwrap();
        let mut d = s.original.clone();
        let official = d
            .codex
            .add(Mode::Official, "example official", None)
            .unwrap();
        assert!(!official.is_empty());
        // Unselected official credentials must never enter the custom-only bundle.
        d.codex.library.official_accounts[0].official_auth =
            Some("example-official-private".into());
        s.save_for(&s.view().revision, &d, Agent::Codex).unwrap();
        assert_eq!(export(source, &file).unwrap(), 2);
        assert!(!std::fs::read_to_string(&file)
            .unwrap()
            .contains("example-official-private"));
        let mut bundle: CustomBundle =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        bundle.codex[0]
            .model_providers_toml
            .as_mut()
            .unwrap()
            .push_str("\n[model_providers.custom.retry]\nunknown_limit = 7\n");
        std::fs::write(&file, serde_json::to_vec(&bundle).unwrap()).unwrap();
        let (_target_dir, target) = fixture("example-existing-model");
        crate::storage::write(&target.roots.acs.join("profiles.json"), None).unwrap();
        let before = target.snapshot().unwrap();
        assert_eq!(import(target.clone(), &file).unwrap(), 2);
        let after = target.snapshot().unwrap();
        assert_eq!(before[1..], after[1..]);
        let reopened = Session::open(target.clone()).unwrap();
        for a in Agent::ALL {
            assert_eq!(reopened.original.agent(a).library.custom_providers.len(), 2);
            let selected = reopened
                .original
                .agent(a)
                .library
                .selected(Mode::Custom)
                .unwrap();
            assert_eq!(
                selected.custom_model.as_deref(),
                Some("example-existing-model")
            );
        }
        let library: Libraries = serde_json::from_slice(after[0].as_ref().unwrap()).unwrap();
        assert!(library
            .codex
            .custom_providers
            .iter()
            .any(|p| p.model_providers_toml.as_deref()
                == bundle.codex[0].model_providers_toml.as_deref()));
        assert_eq!(import(target.clone(), &file).unwrap(), 0);
        assert_eq!(target.snapshot().unwrap(), after);
        bundle.codex[0].official_auth = Some("example-secret".into());
        std::fs::write(&file, serde_json::to_vec(&bundle).unwrap()).unwrap();
        assert!(import(target.clone(), &file).is_err());
        assert_eq!(target.snapshot().unwrap(), after);
    }
    #[test]
    fn same_active_profile_is_selected_without_creating_duplicate_and_bad_input_never_writes() {
        let (dir, source) = fixture("example-model");
        let file = dir.path().join("bundle.json");
        export(source, &file).unwrap();
        let (_td, target) = fixture("example-model");
        crate::storage::write(&target.roots.acs.join("profiles.json"), None).unwrap();
        import(target.clone(), &file).unwrap();
        let s = Session::open(target.clone()).unwrap();
        for a in Agent::ALL {
            assert_eq!(s.original.agent(a).library.custom_providers.len(), 1);
            assert!(s.original.agent(a).library.selected(Mode::Custom).is_some());
        }
        let before = target.snapshot().unwrap();
        std::fs::write(&file, b"{bad json}").unwrap();
        assert!(import(target.clone(), &file).is_err());
        assert_eq!(target.snapshot().unwrap(), before);
    }
}
