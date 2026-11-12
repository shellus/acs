use anyhow::{bail, Context, Result};
use std::{ffi::OsString, path::Path};
/// 注入系统命令，测试只能替换执行器，不向正式程序添加测试入口。
pub trait Runner {
    fn run(&self, program: &str, args: &[OsString], dir: Option<&Path>) -> Result<Vec<u8>>;
}
pub struct SystemRunner;
impl Runner for SystemRunner {
    fn run(&self, program: &str, args: &[OsString], dir: Option<&Path>) -> Result<Vec<u8>> {
        #[cfg(windows)]
        let program = if program == "npm" { "npm.cmd" } else { program };
        let mut command = std::process::Command::new(program);
        command.args(args).env("GIT_TERMINAL_PROMPT", "0");
        if program.starts_with("npm") && args.first().is_some_and(|a| a == "ci") {
            command.env("PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD", "1");
        }
        if let Some(dir) = dir {
            command.current_dir(dir);
        }
        let output = command
            .output()
            .with_context(|| format!("运行 {program}"))?;
        if !output.status.success() {
            bail!(
                "{program} 执行失败：{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(output.stdout)
    }
}
pub fn run(
    runner: &dyn Runner,
    program: &str,
    args: &[&str],
    dir: Option<&Path>,
) -> Result<Vec<u8>> {
    runner.run(
        program,
        &args.iter().map(OsString::from).collect::<Vec<_>>(),
        dir,
    )
}
