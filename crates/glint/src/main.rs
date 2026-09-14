#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod engine;

use anyhow::{Context, Result, bail};
use glint_core::ConfigRuntime;
use glint_ipc::{Command, config_dir, request};
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        log::error!("Glint: {error:#}");
        eprintln!("Glint: {error:#}");
        // A GUI-subsystem executable may not have a console. Preserve startup failures.
        let dir = args().map(|a| a.dir).unwrap_or_else(|_| config_dir());
        if std::fs::create_dir_all(&dir).is_ok() {
            let _ = std::fs::write(dir.join("startup-error.log"), format!("{error:#}\n"));
        }
        std::process::exit(1);
    }
}

struct Args {
    dir: PathBuf,
    command: String,
    no_hooks: bool,
    replace_process: Option<u32>,
}
fn args() -> Result<Args> {
    parse_args(std::env::args().skip(1))
}

fn parse_args(arguments: impl Iterator<Item = String>) -> Result<Args> {
    let mut result = Args {
        dir: config_dir(),
        command: "run".into(),
        no_hooks: false,
        replace_process: None,
    };
    let mut args = arguments;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config-dir" => result.dir = args.next().context("--config-dir 需要路径")?.into(),
            "--no-hooks" => result.no_hooks = true,
            "--replace-process" => {
                result.replace_process = Some(
                    args.next()
                        .context("--replace-process 需要进程 ID")?
                        .parse()?,
                );
            }
            "--check" | "check" => result.command = "check".into(),
            "--init" | "init" => result.command = "init".into(),
            "--status" | "status" => result.command = "status".into(),
            "--pause" | "pause" => result.command = "pause".into(),
            "--resume" | "resume" => result.command = "resume".into(),
            "--reload" | "reload" => result.command = "reload".into(),
            "--quit" | "quit" => result.command = "quit".into(),
            "--help" | "-h" => result.command = "help".into(),
            "--version" | "-V" => result.command = "version".into(),
            "run" => result.command = "run".into(),
            _ if arg.starts_with("--config-dir=") => result.dir = arg[13..].into(),
            _ => bail!("未知参数：{arg}。使用 --help 查看帮助。"),
        }
    }
    if result.dir.is_relative() {
        result.dir = std::env::current_dir()?.join(result.dir);
    }
    if result.replace_process.is_some() && (result.command != "run" || result.no_hooks) {
        bail!("--replace-process 仅用于以管理员权限重启手势后台");
    }
    Ok(result)
}

fn run() -> Result<()> {
    let args = args()?;
    if args.command == "help" {
        println!(
            "Glint — 划过，即行动。\n\nglint [run|init|check|status|pause|resume|reload|quit] [--config-dir PATH]\n\n无参数：启动后台。glint-settings.exe：打开设置。\n--no-hooks：诊断模式，只运行配置和 IPC，不安装输入钩子。\n默认配置：{}",
            config_dir().display()
        );
        return Ok(());
    }
    if args.command == "version" {
        println!("Glint {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    glint_core::logging::init(&args.dir, "glint.log")?;
    match args.command.as_str() {
        "init" => {
            engine::initialize(&args.dir)?;
            println!("配置已就绪：{}", args.dir.display());
        }
        "check" => {
            let runtime = ConfigRuntime::load(&args.dir.join("config.json"))?;
            println!(
                "配置有效：{} 个手势，{} 个动作",
                runtime.config().gestures.len(),
                runtime
                    .config()
                    .packages
                    .iter()
                    .map(|p| p.actions.len())
                    .sum::<usize>()
            );
        }
        "run" => {
            if let Some(pid) = args.replace_process {
                replace_engine(&args.dir, pid)?;
            }
            engine::run(args.dir, args.no_hooks)?;
        }
        command => {
            let command = match command {
                "status" => Command::Status,
                "pause" => Command::Pause { paused: true },
                "resume" => Command::Pause { paused: false },
                "reload" => Command::Reload,
                "quit" => Command::Quit,
                _ => unreachable!(),
            };
            let response = request(&args.dir, command)?;
            println!("{}", serde_json::to_string_pretty(&response)?);
            if !response.ok {
                bail!("{}", response.message);
            }
        }
    }
    Ok(())
}

fn replace_engine(dir: &std::path::Path, pid: u32) -> Result<()> {
    anyhow::ensure!(
        glint_platform::elevation::is_elevated()?,
        "重启后的后台未获得管理员权限"
    );
    // Validate before stopping the working engine. Never replace an unrelated instance.
    ConfigRuntime::load(&dir.join("config.json"))?;
    let response = request(dir, Command::Status)?;
    anyhow::ensure!(
        response.ok && response.status.as_ref().and_then(|s| s.engine_pid) == Some(pid),
        "后台进程已改变，请重新从托盘操作"
    );
    glint_platform::elevation::wait_for_exit(pid, || {
        let response = request(dir, Command::QuitIfProcess { pid })?;
        anyhow::ensure!(response.ok, "{}", response.message);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_only_accepts_live_engine_start() {
        let parse = |args: &[&str]| parse_args(args.iter().map(|arg| (*arg).to_owned()));
        assert_eq!(
            parse(&["--replace-process", "123", "run"])
                .unwrap()
                .replace_process,
            Some(123)
        );
        for args in [
            vec!["--replace-process"],
            vec!["--replace-process", "invalid"],
            vec!["--replace-process", "123", "--no-hooks"],
            vec!["--replace-process", "123", "quit"],
        ] {
            assert!(parse(&args).is_err());
        }
        assert!(parse(&["--no-hooks"]).unwrap().replace_process.is_none());
        assert!(parse(&["status"]).unwrap().replace_process.is_none());
    }
}
