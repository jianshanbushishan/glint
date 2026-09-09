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
}
fn args() -> Result<Args> {
    let mut result = Args {
        dir: config_dir(),
        command: "run".into(),
        no_hooks: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config-dir" => result.dir = args.next().context("--config-dir 需要路径")?.into(),
            "--no-hooks" => result.no_hooks = true,
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
        "run" => engine::run(args.dir, args.no_hooks)?,
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
