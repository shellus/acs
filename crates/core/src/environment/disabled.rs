use super::*;
/// 拆出完整受管块，不解释块内 TOML；调用方编辑其余配置后可原样接回。
pub fn split_block<'a>(
    config: &'a str,
    begin: &str,
    end: &str,
) -> Result<(String, Option<&'a str>)> {
    let mut first = None;
    let mut last = None;
    let mut offset = 0;
    for line in config.split_inclusive('\n') {
        let plain = line.trim();
        if plain == begin {
            if first.is_some() {
                bail!("禁用 Skills 受管块重复");
            }
            first = Some(offset);
        }
        if plain == end {
            if first.is_none() || last.is_some() {
                bail!("禁用 Skills 结束标记不匹配");
            }
            last = Some(offset + line.len());
        }
        offset += line.len();
    }
    match (first, last) {
        (None, None) => Ok((config.into(), None)),
        (Some(a), Some(b)) if b > a => {
            let mut base = config[..a].to_string();
            base.push_str(&config[b..]);
            Ok((base, Some(&config[a..b])))
        }
        _ => bail!("禁用 Skills 受管块不完整"),
    }
}
pub fn update_disabled(config: &str, names: &[String]) -> Result<String> {
    let (mut base, old) = split_block(config, DISABLED_BEGIN, DISABLED_END)?;
    // 删除之前生成的分隔空行，避免重复 apply 积累空行。
    if let Some(block) = old {
        let start = config.find(block).unwrap();
        let prefix = &config[..start];
        if prefix.ends_with("\r\n\r\n") {
            base.replace_range(start - 2..start, "");
        } else if prefix.ends_with("\n\n") {
            base.replace_range(start - 1..start, "");
        }
    }
    if names.is_empty() {
        return Ok(base);
    }
    let newline = if config.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut sorted = names.to_vec();
    sorted.sort();
    sorted.dedup();
    if !base.is_empty() {
        if !base.ends_with('\n') {
            base.push_str(newline);
        }
        base.push_str(newline);
    }
    base.push_str(DISABLED_BEGIN);
    base.push_str(newline);
    for name in sorted {
        if !safe_name(&name) {
            bail!("不安全 disabled Skill {name:?}");
        }
        base.push_str(&format!(
            "[[skills.config]]{newline}name = {}{newline}enabled = false{newline}",
            serde_json::to_string(&name)?
        ));
    }
    base.push_str(DISABLED_END);
    base.push_str(newline);
    // 账号保存可能调整块前的空行；块内容相同时保留整份配置原字节。
    if old.is_some_and(|block| base.ends_with(block)) {
        return Ok(config.into());
    }
    Ok(base)
}
