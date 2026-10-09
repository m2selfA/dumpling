use std::fmt::{Debug, Display};
use std::net::SocketAddr;
use std::sync::mpsc::Sender;

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, SecretKey};
use iroh_tickets::endpoint::EndpointTicket;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

pub const ALPN: &[u8] = b"DUMBPIPEV0";
const HANDSHAKE: [u8; 5] = *b"hello";

fn format_error(stage: &str, err: impl Display + Debug) -> String {
    format!("stage={stage} error={err} error_debug={err:?}")
}

fn format_connection_error(
    stage: &str,
    connection: &iroh::endpoint::Connection,
    remote: &str,
    err: impl Display + Debug,
) -> String {
    format!(
        "stage={stage} peer={:?} remote={remote} close_reason={:?} error={err} error_debug={err:?}",
        connection.remote_id(),
        connection.close_reason(),
    )
}

pub fn parse_ticket(text: &str) -> Result<EndpointTicket, String> {
    text.trim()
        .parse()
        .map_err(|err| format!("ticket 无效：{err}"))
}

pub async fn serve_client(
    ticket: EndpointTicket,
    local: SocketAddr,
    cancel: CancellationToken,
    ready: Sender<Result<SocketAddr, String>>,
) {
    let listener = match TcpListener::bind(local).await {
        Ok(listener) => listener,
        Err(err) => {
            let message = if local.port() == 0 {
                format!("无法找到可用的本机端口：{err}")
            } else {
                format!("无法监听 {local}：{err}")
            };
            let _ = ready.send(Err(message));
            return;
        }
    };
    let bound = match listener.local_addr() {
        Ok(addr) => addr,
        Err(err) => {
            let _ = ready.send(Err(format!("无法读取本机监听端口：{err}")));
            return;
        }
    };
    let endpoint = match bind_endpoint().await {
        Ok(endpoint) => endpoint,
        Err(err) => {
            let _ = ready.send(Err(err));
            return;
        }
    };
    let _ = ready.send(Ok(bound));
    let addr = ticket.endpoint_addr().clone();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { break };
                let endpoint = endpoint.clone();
                let addr = addr.clone();
                let cancel = cancel.clone();
                tokio::spawn(async move {
                    if let Err(err) = forward_out(stream, endpoint, addr, cancel).await {
                        eprintln!("forward out: {err}");
                    }
                });
            }
        }
    }
    endpoint.close().await;
}

pub async fn serve_host(
    target: SocketAddr,
    cancel: CancellationToken,
    ready: Sender<Result<String, String>>,
) {
    let endpoint = match bind_endpoint().await {
        Ok(endpoint) => endpoint,
        Err(err) => {
            let _ = ready.send(Err(err));
            return;
        }
    };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), endpoint.online()).await;
    let ticket = EndpointTicket::new(endpoint.addr());
    let _ = ready.send(Ok(ticket.to_string()));
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                let Ok(accepting) = incoming.accept() else { break };
                let cancel = cancel.clone();
                tokio::spawn(async move {
                    if let Err(err) = forward_in(accepting, target, cancel).await {
                        eprintln!("forward in: {err}");
                    }
                });
            }
        }
    }
    endpoint.close().await;
}

async fn bind_endpoint() -> Result<Endpoint, String> {
    Endpoint::builder(presets::N0)
        .secret_key(SecretKey::generate())
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .map_err(|err| format!("无法建立连接端点：{err}"))
}

async fn forward_out(
    stream: TcpStream,
    endpoint: Endpoint,
    addr: EndpointAddr,
    cancel: CancellationToken,
) -> Result<(), String> {
    let remote = format!("{addr:?}");
    let connection = endpoint
        .connect(addr, ALPN)
        .await
        .map_err(|err| format_error("connect", err))?;
    let (mut send, recv) = connection
        .open_bi()
        .await
        .map_err(|err| format_connection_error("open_stream", &connection, &remote, err))?;
    send.write_all(&HANDSHAKE)
        .await
        .map_err(|err| format_connection_error("handshake_write", &connection, &remote, err))?;
    let (read, write) = stream.into_split();
    forward(read, write, recv, send, cancel)
        .await
        .map_err(|err| format_connection_error("forward", &connection, &remote, err))
}

async fn forward_in(
    accepting: iroh::endpoint::Accepting,
    target: SocketAddr,
    cancel: CancellationToken,
) -> Result<(), String> {
    let remote = format!("{:?}", accepting.remote_addr());
    let connection = accepting
        .await
        .map_err(|err| format!("remote={remote} {}", format_error("accept_connection", err)))?;
    let (send, mut recv) = connection
        .accept_bi()
        .await
        .map_err(|err| format_connection_error("accept_stream", &connection, &remote, err))?;
    let mut buf = [0u8; HANDSHAKE.len()];
    recv.read_exact(&mut buf)
        .await
        .map_err(|err| format_connection_error("handshake_read", &connection, &remote, err))?;
    if buf != HANDSHAKE {
        return Err(format_connection_error(
            "handshake_validate",
            &connection,
            &remote,
            "handshake mismatch",
        ));
    }
    let stream = TcpStream::connect(target)
        .await
        .map_err(|err| format_connection_error("target_connect", &connection, &remote, err))?;
    let (read, write) = stream.into_split();
    forward(read, write, recv, send, cancel)
        .await
        .map_err(|err| format_connection_error("forward", &connection, &remote, err))
}

async fn forward(
    read: impl AsyncRead + Unpin + Send + 'static,
    write: impl AsyncWrite + Unpin + Send + 'static,
    recv: impl AsyncRead + Unpin + Send + 'static,
    send: impl AsyncWrite + Unpin + Send + 'static,
    cancel: CancellationToken,
) -> Result<(), String> {
    let cancel_out = cancel.clone();
    let to_remote = tokio::spawn(async move {
        tokio::select! {
            result = copy_to_send(read, send) => result,
            _ = cancel_out.cancelled() => Ok(()),
        }
    });
    let from_remote = tokio::spawn(async move {
        let mut recv = recv;
        let mut write = write;
        tokio::select! {
            result = tokio::io::copy(&mut recv, &mut write) => {
                result
                    .map(|_| ())
                    .map_err(|err| format_error("remote_to_local_copy", err))
            }
            _ = cancel.cancelled() => Ok(()),
        }
    });
    to_remote
        .await
        .map_err(|err| format_error("local_to_remote_task", err))??;
    from_remote
        .await
        .map_err(|err| format_error("remote_to_local_task", err))??;
    Ok(())
}

async fn copy_to_send(
    mut read: impl AsyncRead + Unpin,
    mut send: impl AsyncWrite + Unpin,
) -> Result<(), String> {
    tokio::io::copy(&mut read, &mut send)
        .await
        .map_err(|err| format_error("local_to_remote_copy", err))?;
    send.shutdown()
        .await
        .map_err(|err| format_error("local_to_remote_shutdown", err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_includes_display_and_debug_error() {
        let source = std::io::Error::new(std::io::ErrorKind::TimedOut, "probe");
        let rendered = format_error("probe_stage", source);

        assert!(rendered.contains("stage=probe_stage"));
        assert!(rendered.contains("error=probe"));
        assert!(rendered.contains("error_debug="));
    }
}
