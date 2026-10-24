use super::Plan;
use crate::Agent;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use winreg::{enums::HKEY_CURRENT_USER, RegKey};

const BOOTSTRAP: &str = include_str!("windows.ps1");
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub(super) struct Package {
    pub executable: PathBuf,
    family: String,
    version: Vec<u32>,
}

/// 已安装的 MSIX 包名与包内可执行文件。
fn package_layout(agent: Agent) -> (&'static [&'static str], &'static str) {
    match agent {
        Agent::Codex => (&["OpenAI.Codex"], "ChatGPT.exe"),
        Agent::Claude => (&["Claude", "AnthropicPBC.Claude"], "Claude.exe"),
    }
}

pub(super) fn installed_packages(agent: Agent) -> Result<Vec<Package>> {
    let (names, program) = package_layout(agent);
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\CurrentVersion\\AppModel\\Repository\\Packages")
        .context("未找到客户端安装信息，请打开应用商店安装")?;
    let mut packages = Vec::new();
    for name in key.enum_keys() {
        let name = name?;
        let parts: Vec<_> = name.split('_').collect();
        if parts.len() != 5 || !names.contains(&parts[0]) {
            continue;
        }
        let root: String = key.open_subkey(&name)?.get_value("PackageRootFolder")?;
        let executable = PathBuf::from(root).join("app").join(program);
        if executable.is_file() {
            packages.push(Package {
                executable,
                family: format!("{}_{}", parts[0], parts[4]),
                version: parts[1]
                    .split('.')
                    .map(|n| n.parse().unwrap_or(0))
                    .collect(),
            });
        }
    }
    packages.sort_by(|a, b| b.version.cmp(&a.version));
    Ok(packages)
}

pub(super) fn package_for(executable: &Path) -> Result<Option<Package>> {
    let Ok(executable) = executable.canonicalize() else {
        return Ok(None);
    };
    let mut packages = Vec::new();
    for agent in Agent::ALL {
        packages.extend(installed_packages(agent)?);
    }
    Ok(packages.into_iter().find(|package| {
        package.executable.canonicalize().is_ok_and(|path| {
            path.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&executable.as_os_str().to_string_lossy())
        })
    }))
}

// CommandLineToArgvW / CRT quoting: quotes and trailing backslashes must survive
// both the PowerShell bootstrap and the final ProcessStartInfo.Arguments string.
fn quote_argument(value: &str) -> String {
    let mut quoted = String::from("\"");
    let mut slashes = 0;
    for character in value.chars() {
        match character {
            '\\' => slashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(slashes * 2 + 1));
                quoted.push('"');
                slashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(slashes));
                quoted.push(character);
                slashes = 0;
            }
        }
    }
    quoted.push_str(&"\\".repeat(slashes * 2));
    quoted.push('"');
    quoted
}

fn environment(plan: &Plan) -> Result<BTreeMap<String, String>> {
    // Windows environment names are case insensitive. Preserve the complete
    // caller environment, because package activation uses the broker's environment.
    let mut values: BTreeMap<String, (String, String)> = BTreeMap::new();
    let command = plan.command();
    let inherited = std::env::vars_os().map(|(key, value)| (key, Some(value)));
    let overrides = command
        .get_envs()
        .map(|(key, value)| (key.to_owned(), value.map(OsString::from)));
    for (key, value) in inherited.chain(overrides) {
        let key = key
            .into_string()
            .map_err(|_| anyhow::anyhow!("环境变量名不是有效 Unicode"))?;
        if let Some(value) = value {
            let value = value
                .into_string()
                .map_err(|_| anyhow::anyhow!("环境变量 {key} 不是有效 Unicode"))?;
            values.insert(key.to_uppercase(), (key, value));
        } else {
            values.remove(&key.to_uppercase());
        }
    }
    Ok(values.into_values().collect())
}

#[derive(Serialize)]
struct Request {
    executable: PathBuf,
    arguments: String,
    directory: Option<PathBuf>,
    environment: BTreeMap<String, String>,
    family: String,
    bootstrap_arguments: String,
}

#[derive(Deserialize)]
struct Outcome {
    pid: Option<u32>,
    error: Option<String>,
}

fn prepare_request(plan: &Plan, package: &Package, directory: &Path) -> Result<Vec<String>> {
    let request_path = directory.join("request.json");
    let script_path = directory.join("launch.ps1");
    let arguments = vec![
        "-NoProfile".to_owned(),
        "-NonInteractive".into(),
        "-WindowStyle".into(),
        "Hidden".into(),
        "-ExecutionPolicy".into(),
        "Bypass".into(),
        "-File".into(),
        script_path.to_string_lossy().into_owned(),
        "-RequestPath".into(),
        request_path.to_string_lossy().into_owned(),
    ];
    let request = Request {
        executable: plan.executable.clone(),
        arguments: plan
            .args
            .iter()
            .map(|arg| quote_argument(arg))
            .collect::<Vec<_>>()
            .join(" "),
        directory: plan.command().get_current_dir().map(Path::to_path_buf),
        environment: environment(plan)?,
        family: package.family.clone(),
        bootstrap_arguments: arguments
            .iter()
            .map(|arg| quote_argument(arg))
            .chain(["-InPackage".into()])
            .collect::<Vec<_>>()
            .join(" "),
    };
    fs::write(&request_path, serde_json::to_vec(&request)?)?;
    // A BOM is needed for Windows PowerShell 5.1 to read UTF-8 source correctly.
    fs::write(&script_path, format!("\u{feff}{BOOTSTRAP}"))?;
    Ok(arguments)
}

fn powershell() -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("SystemRoot").context("缺少 SystemRoot")?)
            .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
    )
}

pub(super) fn spawn_packaged(plan: &Plan, package: &Package) -> Result<()> {
    let directory = tempfile::Builder::new().prefix("acs-launch-").tempdir()?;
    let arguments = prepare_request(plan, package, directory.path())?;
    let result_path = directory.path().join("result.json");
    let error_path = directory.path().join("stderr.txt");
    let mut bootstrap = Command::new(powershell()?)
        .args(&arguments)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(fs::File::create(&error_path)?)
        .spawn()
        .context("无法启动 Windows 包启动辅助进程")?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let result = (|| -> Result<()> {
        loop {
            if result_path.is_file() {
                let outcome: Outcome = serde_json::from_slice(&fs::read(&result_path)?)?;
                break match (outcome.pid, outcome.error) {
                    (Some(pid), None) if pid != 0 => Ok(()),
                    (_, Some(error)) => Err(anyhow::anyhow!("无法启动商店包客户端：{error}")),
                    _ => Err(anyhow::anyhow!("Windows 包启动返回了无效结果")),
                };
            }
            if let Some(status) = bootstrap.try_wait()? {
                if !status.success() {
                    // The result may have been published between the first check
                    // and observing the bootstrap exit.
                    if result_path.is_file() {
                        continue;
                    }
                    break Err(anyhow::anyhow!(
                        "Windows 包启动失败：{}",
                        fs::read_to_string(&error_path).unwrap_or_default()
                    ));
                }
            }
            if Instant::now() >= deadline {
                break Err(anyhow::anyhow!(
                    "等待商店包客户端启动确认超时；请检查客户端是否已打开"
                ));
            }
            thread::sleep(Duration::from_millis(50));
        }
    })();
    // Only the temporary bootstrap is ours to stop, never the existing client.
    let _ = bootstrap.kill();
    let _ = bootstrap.wait();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{launch::Settings, Agent, Mode, Roots};
    use base64::{engine::general_purpose::STANDARD, Engine};
    use std::{ffi::c_void, os::windows::ffi::OsStringExt};

    #[link(name = "shell32")]
    extern "system" {
        fn CommandLineToArgvW(command: *const u16, count: *mut i32) -> *mut *mut u16;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }

    #[test]
    fn windows_argument_roundtrip() {
        let arguments = [
            "",
            "plain",
            "中文 路径",
            r"C:\example path\",
            "quote\"here",
            r#"slash\\\"quote"#,
            "'$()`; & example",
            super::super::RESOLVER_RULES,
        ];
        let line = std::iter::once("example.exe".to_owned())
            .chain(arguments.iter().map(|value| quote_argument(value)))
            .collect::<Vec<_>>()
            .join(" ");
        let wide: Vec<u16> = line.encode_utf16().chain([0]).collect();
        unsafe {
            let mut count = 0;
            let parsed = CommandLineToArgvW(wide.as_ptr(), &mut count);
            assert!(!parsed.is_null());
            let values: Vec<_> = std::slice::from_raw_parts(parsed, count as usize)
                .iter()
                .map(|pointer| {
                    let mut length = 0;
                    while *pointer.add(length) != 0 {
                        length += 1;
                    }
                    OsString::from_wide(std::slice::from_raw_parts(*pointer, length))
                })
                .collect();
            LocalFree(parsed.cast());
            assert_eq!(values[1..], arguments.map(OsString::from));
        }
    }

    fn probe(directory: &Path, mode: Mode) -> Plan {
        let path = directory
            .join("probe.json")
            .to_string_lossy()
            .replace('\'', "''");
        let script = format!(
            r#"
$value = @{{ home = $env:CODEX_HOME; proxy = $env:HTTPS_PROXY; key = $env:OPENAI_API_KEY; base = $env:OPENAI_BASE_URL; example = $env:ACS_LAUNCH_EXAMPLE; args = [Environment]::GetCommandLineArgs() }}
[IO.File]::WriteAllText('{path}', ($value | ConvertTo-Json -Compress), [Text.UTF8Encoding]::new($false))
"#
        );
        let encoded = STANDARD.encode(
            script
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        let settings = Settings {
            agent: Agent::Codex,
            mode,
            // 自定义模式的直连路径；带代理的自定义模式与官方模式共用同一条注入逻辑。
            proxy: if mode == Mode::Official {
                "http://proxy.example:8080".into()
            } else {
                String::new()
            },
            roots: Roots::under(directory),
        };
        // The probe is a console program, so desktop Chromium flags are verified
        // separately rather than passed to PowerShell's option parser.
        let mut plan = settings
            .plan(
                powershell().unwrap(),
                false,
                vec![
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-WindowStyle".into(),
                    "Hidden".into(),
                    "-EncodedCommand".into(),
                    encoded,
                ],
            )
            .unwrap();
        plan.env.extend([
            (
                "ACS_HOME".into(),
                directory.join("acs").to_string_lossy().into_owned(),
            ),
            (
                "CLAUDE_CONFIG_DIR".into(),
                directory.join("claude").to_string_lossy().into_owned(),
            ),
            (
                "ACS_LAUNCH_EXAMPLE".into(),
                "中文 example ' $(); & \" \\".into(),
            ),
        ]);
        if mode == Mode::Custom {
            plan.remove_env.push("HTTPS_PROXY".into());
        }
        plan
    }

    fn assert_probe(directory: &Path, mode: Mode) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let path = directory.join("probe.json");
        while !path.is_file() {
            assert!(Instant::now() < deadline, "probe did not run");
            thread::sleep(Duration::from_millis(50));
        }
        let value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(
            value["home"],
            directory.join("codex").to_string_lossy().as_ref()
        );
        assert!(value["key"].is_null());
        assert!(value["base"].is_null());
        assert_eq!(value["example"], "中文 example ' $(); & \" \\");
        if mode == Mode::Official {
            assert_eq!(value["proxy"], "http://proxy.example:8080");
        } else {
            assert!(value["proxy"].is_null());
        }
    }

    #[test]
    fn bootstrap_preserves_environment_and_reports_real_spawn_failure() {
        let before: BTreeMap<_, _> = std::env::vars_os().collect();
        for mode in [Mode::Official, Mode::Custom] {
            let directory = tempfile::Builder::new()
                .prefix("acs example ' 中文 ")
                .tempdir()
                .unwrap();
            let mut plan = probe(directory.path(), mode);
            let package = Package {
                executable: plan.executable.clone(),
                family: "example".into(),
                version: vec![],
            };
            let arguments = prepare_request(&plan, &package, directory.path()).unwrap();
            let status = Command::new(powershell().unwrap())
                .args(&arguments)
                .arg("-InPackage")
                .env("OPENAI_API_KEY", "example-parent-key")
                .env("OPENAI_BASE_URL", "https://parent.example")
                .creation_flags(CREATE_NO_WINDOW)
                .status()
                .unwrap();
            assert!(status.success());
            let outcome: Outcome =
                serde_json::from_slice(&fs::read(directory.path().join("result.json")).unwrap())
                    .unwrap();
            assert!(outcome.pid.is_some());
            assert_probe(directory.path(), mode);

            fs::remove_file(directory.path().join("result.json")).unwrap();
            plan.executable = directory.path().join("missing.exe");
            prepare_request(&plan, &package, directory.path()).unwrap();
            let status = Command::new(powershell().unwrap())
                .args(&arguments)
                .arg("-InPackage")
                .creation_flags(CREATE_NO_WINDOW)
                .status()
                .unwrap();
            assert!(!status.success());
            let outcome: Outcome =
                serde_json::from_slice(&fs::read(directory.path().join("result.json")).unwrap())
                    .unwrap();
            assert!(outcome.pid.is_none());
            assert!(outcome.error.is_some());
        }
        assert_eq!(before, std::env::vars_os().collect());
    }

    #[test]
    #[ignore = "requires an installed Codex MSIX; runs only isolated PowerShell probes"]
    fn installed_package_preserves_launch_environment() {
        let package = installed_packages(Agent::Codex)
            .unwrap()
            .into_iter()
            .next()
            .expect("Codex must be installed");
        for mode in [Mode::Official, Mode::Custom] {
            let directory = tempfile::tempdir().unwrap();
            spawn_packaged(&probe(directory.path(), mode), &package).unwrap();
            assert_probe(directory.path(), mode);
        }
        let directory = tempfile::tempdir().unwrap();
        let mut plan = probe(directory.path(), Mode::Official);
        plan.executable = directory.path().join("missing.exe");
        assert!(spawn_packaged(&plan, &package)
            .unwrap_err()
            .to_string()
            .contains("无法启动商店包客户端"));
    }
}
