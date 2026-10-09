use std::env;
use std::fs;
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use dumpling::pipe;
use tokio_util::sync::CancellationToken;

#[derive(Debug, PartialEq, Eq)]
struct Config {
    target: SocketAddr,
    ticket_file: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Share(Config),
    Connect,
    Help,
    ShareHelp,
    Version,
}

fn main() {
    match parse_args(env::args().skip(1)) {
        Ok(Command::Help) => print_help(),
        Ok(Command::ShareHelp) => print_share_help(),
        Ok(Command::Version) => println!("{}", env!("CARGO_PKG_VERSION")),
        Ok(Command::Connect) => {
            eprintln!("connect 子命令尚未实现");
            std::process::exit(2);
        }
        Ok(Command::Share(config)) => {
            if let Err(err) = run(config) {
                eprintln!("错误：{err}");
                std::process::exit(1);
            }
        }
        Err(err) => {
            eprintln!("错误：{err}\n\n{}", usage());
            std::process::exit(2);
        }
    }
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut args = args.into_iter();
    match args.next().as_deref() {
        None => Err("缺少子命令：share".into()),
        Some("-h" | "--help") => Ok(Command::Help),
        Some("-V" | "--version") => Ok(Command::Version),
        Some("share") => parse_share_args(args),
        Some("connect") => {
            if let Some(arg) = args.next() {
                return Err(format!("connect 尚未实现，无法识别参数：{arg}"));
            }
            Ok(Command::Connect)
        }
        Some(command) => Err(format!("未知子命令：{command}")),
    }
}

fn parse_share_args(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let mut target = SocketAddr::from(([127, 0, 0, 1], 8080));
    let mut ticket_file = None;
    let mut positional_port = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::ShareHelp),
            "-V" | "--version" => return Ok(Command::Version),
            "-p" | "--port" => {
                let value = args.next().ok_or_else(|| format!("{arg} 需要端口参数"))?;
                target.set_port(parse_port(&value)?);
            }
            "-t" | "--target" => {
                let value = args.next().ok_or_else(|| format!("{arg} 需要地址参数"))?;
                target = parse_target(&value)?;
            }
            "-o" | "--ticket-file" => {
                let value = args.next().ok_or_else(|| format!("{arg} 需要文件路径"))?;
                ticket_file = Some(PathBuf::from(value));
            }
            _ if arg.starts_with('-') => return Err(format!("未知参数：{arg}")),
            _ if positional_port => return Err("只能指定一个位置端口参数".into()),
            _ => {
                target.set_port(parse_port(&arg)?);
                positional_port = true;
            }
        }
    }

    Ok(Command::Share(Config {
        target,
        ticket_file,
    }))
}

fn parse_port(value: &str) -> Result<u16, String> {
    let port = value
        .parse::<u16>()
        .map_err(|_| format!("端口无效：{value}（应为 1 到 65535）"))?;
    if port == 0 {
        return Err("端口应该是 1 到 65535 的数字".into());
    }
    Ok(port)
}

fn parse_target(value: &str) -> Result<SocketAddr, String> {
    let target = value
        .parse::<SocketAddr>()
        .map_err(|err| format!("目标地址无效：{value}（{err}）"))?;
    if target.port() == 0 {
        return Err("目标端口应该是 1 到 65535 的数字".into());
    }
    Ok(target)
}

fn print_help() {
    println!("dumpling-cli {}\n\n{}", env!("CARGO_PKG_VERSION"), usage());
}

fn print_share_help() {
    println!(
        "dumpling-cli share {}\n\n{}",
        env!("CARGO_PKG_VERSION"),
        share_usage()
    );
}

fn usage() -> &'static str {
    "用法：dumpling-cli <子命令> [选项]\n\n子命令：\n  share                     共享本机 TCP 服务\n  connect                   预留：连接远端 ticket\n\n全局选项：\n  -h, --help                显示帮助\n  -V, --version             显示版本"
}

fn share_usage() -> &'static str {
    "用法：dumpling-cli share [选项] [端口]\n\n选项：\n  -p, --port <PORT>          要共享的本机服务端口（默认 8080）\n  -t, --target <ADDR>        完整目标地址（默认 127.0.0.1:8080）\n  -o, --ticket-file <PATH>   将 ticket 写入文件\n  -h, --help                 显示帮助\n  -V, --version              显示版本\n\n启动后会把 ticket 输出到 stdout，并持续共享；按 Ctrl+C 停止。"
}

fn run(config: Config) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("无法创建 tokio runtime：{err}"))?;
    runtime.block_on(run_async(config))
}

async fn run_async(config: Config) -> Result<(), String> {
    let cancel = CancellationToken::new();
    let (ready_tx, ready_rx) = mpsc::channel();
    let task_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        pipe::serve_host(config.target, task_cancel, ready_tx).await;
    });

    let ready = tokio::task::spawn_blocking(move || ready_rx.recv_timeout(Duration::from_secs(10)))
        .await
        .map_err(|err| format!("等待共享器启动失败：{err}"))?
        .map_err(|err| format!("等待 ticket 失败：{err}"))?;

    let ticket = match ready {
        Ok(ticket) => ticket,
        Err(err) => {
            cancel.cancel();
            let _ = task.await;
            return Err(err);
        }
    };

    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{ticket}").map_err(|err| format!("写出 ticket 失败：{err}"))?;
    stdout
        .flush()
        .map_err(|err| format!("刷新 ticket 输出失败：{err}"))?;

    if let Some(path) = config.ticket_file {
        fs::write(&path, format!("{ticket}\n"))
            .map_err(|err| format!("写入 ticket 文件 {} 失败：{err}", path.display()))?;
        eprintln!("ticket 已写入 {}", path.display());
    }
    eprintln!("正在共享 {}，按 Ctrl+C 停止。", config.target);

    if let Err(err) = tokio::signal::ctrl_c().await {
        cancel.cancel();
        let _ = task.await;
        return Err(format!("监听 Ctrl+C 失败：{err}"));
    }

    cancel.cancel();
    task.await.map_err(|err| format!("停止共享器失败：{err}"))?;
    eprintln!("共享器已停止。");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_defaults_to_localhost_8080() {
        assert_eq!(
            parse_args(vec!["share".into()]),
            Ok(Command::Share(Config {
                target: SocketAddr::from(([127, 0, 0, 1], 8080)),
                ticket_file: None,
            }))
        );
    }

    #[test]
    fn parses_share_options_and_ticket_file() {
        assert_eq!(
            parse_args(vec![
                "share".into(),
                "--target".into(),
                "127.0.0.1:9000".into(),
                "--ticket-file".into(),
                "ticket.txt".into(),
            ]),
            Ok(Command::Share(Config {
                target: SocketAddr::from(([127, 0, 0, 1], 9000)),
                ticket_file: Some(PathBuf::from("ticket.txt")),
            }))
        );
    }

    #[test]
    fn reserves_connect_subcommand() {
        assert_eq!(parse_args(vec!["connect".into()]), Ok(Command::Connect));
        assert_eq!(
            parse_args(vec!["share".into(), "--help".into()]),
            Ok(Command::ShareHelp)
        );
    }

    #[test]
    fn rejects_zero_port() {
        assert!(parse_args(vec!["share".into(), "--port".into(), "0".into()]).is_err());
        assert!(parse_args(vec![
            "share".into(),
            "--target".into(),
            "127.0.0.1:0".into()
        ])
        .is_err());
    }
}
