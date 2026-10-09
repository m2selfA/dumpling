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
            eprintln!("the connect subcommand is not implemented yet");
            std::process::exit(2);
        }
        Ok(Command::Share(config)) => {
            if let Err(err) = run(config) {
                eprintln!("Error: {err}");
                std::process::exit(1);
            }
        }
        Err(err) => {
            eprintln!("Error: {err}\n\n{}", usage());
            std::process::exit(2);
        }
    }
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut args = args.into_iter();
    match args.next().as_deref() {
        None => Err("missing subcommand: share".into()),
        Some("-h" | "--help") => Ok(Command::Help),
        Some("-V" | "--version") => Ok(Command::Version),
        Some("share") => parse_share_args(args),
        Some("connect") => {
            if let Some(arg) = args.next() {
                return Err(format!(
                    "connect is not implemented; unrecognized argument: {arg}"
                ));
            }
            Ok(Command::Connect)
        }
        Some(command) => Err(format!("unknown subcommand: {command}")),
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
                let value = args
                    .next()
                    .ok_or_else(|| format!("{arg} requires a port"))?;
                target.set_port(parse_port(&value)?);
            }
            "-t" | "--target" => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("{arg} requires an address"))?;
                target = parse_target(&value)?;
            }
            "-o" | "--ticket-file" => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("{arg} requires a file path"))?;
                ticket_file = Some(PathBuf::from(value));
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option: {arg}")),
            _ if positional_port => return Err("only one positional port may be specified".into()),
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
        .map_err(|_| format!("invalid port: {value} (expected 1 to 65535)"))?;
    if port == 0 {
        return Err("port must be a number from 1 to 65535".into());
    }
    Ok(port)
}

fn parse_target(value: &str) -> Result<SocketAddr, String> {
    let target = value
        .parse::<SocketAddr>()
        .map_err(|err| format!("invalid target address: {value} ({err})"))?;
    if target.port() == 0 {
        return Err("target port must be a number from 1 to 65535".into());
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
    "Usage: dumpling-cli <COMMAND> [OPTIONS]\n\nCommands:\n  share                     Share a local TCP service\n  connect                   Reserved for connecting to a remote ticket\n\nGlobal options:\n  -h, --help                Show help\n  -V, --version             Show version"
}

fn share_usage() -> &'static str {
    "Usage: dumpling-cli share [OPTIONS] [PORT]\n\nOptions:\n  -p, --port <PORT>          Local service port to share (default: 8080)\n  -t, --target <ADDR>        Full target address (default: 127.0.0.1:8080)\n  -o, --ticket-file <PATH>   Write the ticket to a file\n  -h, --help                 Show help\n  -V, --version              Show version\n\nThe ticket is printed to stdout and the service keeps running until Ctrl+C."
}

fn run(config: Config) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("could not create the Tokio runtime: {err}"))?;
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
        .map_err(|err| format!("failed while waiting for the sharing service to start: {err}"))?
        .map_err(|err| format!("failed while waiting for the ticket: {err}"))?;

    let ticket = match ready {
        Ok(ticket) => ticket,
        Err(err) => {
            cancel.cancel();
            let _ = task.await;
            return Err(err);
        }
    };

    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{ticket}").map_err(|err| format!("failed to write the ticket: {err}"))?;
    stdout
        .flush()
        .map_err(|err| format!("failed to flush the ticket output: {err}"))?;

    if let Some(path) = config.ticket_file {
        fs::write(&path, format!("{ticket}\n"))
            .map_err(|err| format!("failed to write the ticket file {}: {err}", path.display()))?;
        eprintln!("ticket written to {}", path.display());
    }
    eprintln!("Sharing {}. Press Ctrl+C to stop.", config.target);

    if let Err(err) = tokio::signal::ctrl_c().await {
        cancel.cancel();
        let _ = task.await;
        return Err(format!("failed to listen for Ctrl+C: {err}"));
    }

    cancel.cancel();
    task.await
        .map_err(|err| format!("failed to stop the sharing service: {err}"))?;
    eprintln!("Sharing service stopped.");
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
