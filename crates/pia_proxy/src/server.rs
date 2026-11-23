//! Front end: one listener that accepts SOCKS5 or HTTP CONNECT.

use crate::config::Config;
use crate::error::{ProxyError, Result};
use crate::pool::Pool;
use crate::proxy::Proxy;
use crate::socks5;
use crate::Target;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{copy_bidirectional, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, warn};

const MAX_HEAD: usize = 32 * 1024;
const SNIFF_TIMEOUT: Duration = Duration::from_secs(15);

/// Start every configured listener and run until the process stops.
pub async fn serve(config: Config) -> Result<()> {
    let proxy = Arc::new(Proxy::new(&config).await?);
    let listen = config.listen.clone();

    info!(
        upstreams = ?proxy.names(),
        "starting proxy listeners"
    );

    // Gluetun style: a fixed local port per region.
    let mut pinned_listeners = tokio::task::JoinSet::new();
    for (port, region_id) in config.pinned_ports() {
        match proxy.pinned_pool(&region_id) {
            Some(pool) => {
                let host = listen.host.clone();
                let id = region_id.clone();
                pinned_listeners.spawn(async move {
                    info!(%id, port, "region pinned listener starting");
                    if let Err(error) = run_listener(host, port, pool, false).await {
                        error!(%id, port, error = %error, "pinned listener stopped");
                    }
                });
            }
            None => warn!(
                region = %region_id,
                "region is pinned to a port but could not be provisioned; port not opened"
            ),
        }
    }

    // A region is retired after `failures_before_retire` consecutive failures.
    // Once every region is retired there is nothing left to serve with, so the
    // listener stops and the run ends instead of failing page after page.
    run_listener(listen.host, listen.port, proxy.pool(), true).await
}

/// Why the listener must stop, if there is no region left in rotation.
fn stop_reason(pool: &Pool) -> Option<ProxyError> {
    pool.is_exhausted().then(|| {
        ProxyError::NoUpstream(format!(
            "every region was retired ({} of {} in rotation)",
            pool.live(),
            pool.len()
        ))
    })
}

async fn run_listener(
    host: String,
    port: u16,
    pool: Arc<Pool>,
    stop_when_exhausted: bool,
) -> Result<()> {
    let listener = TcpListener::bind((host.as_str(), port)).await.map_err(|e| {
        ProxyError::Config(format!("cannot bind {host}:{port}: {e}"))
    })?;

    info!(%host, port, upstreams = ?pool.names(), "listening");

    loop {
        let (socket, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error) => {
                warn!(error = %error, "accept failed");
                continue;
            }
        };
        if stop_when_exhausted {
            if let Some(reason) = stop_reason(&pool) {
                error!(
                    upstreams = pool.len(),
                    "every region was retired; stopping the listener"
                );
                return Err(reason);
            }
        }

        let pool = pool.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_client(socket, pool).await {
                debug!(%peer, error = %error, "client connection ended");
            }
        });
    }
}

async fn handle_client(socket: TcpStream, pool: Arc<Pool>) -> Result<()> {
    // Sniff without consuming: SOCKS5 starts with 0x05, HTTP with an ASCII
    // method. peek() leaves the bytes in place for the real parser.
    let mut first = [0u8; 1];
    match tokio::time::timeout(SNIFF_TIMEOUT, socket.peek(&mut first)).await {
        Ok(Ok(0)) => return Ok(()),
        Ok(Ok(_)) => {}
        Ok(Err(error)) => return Err(error.into()),
        Err(_) => return Ok(()),
    }

    if first[0] == 0x05 {
        handle_socks5(socket, pool).await
    } else {
        handle_http(socket, pool).await
    }
}

async fn handle_socks5(mut client: TcpStream, pool: Arc<Pool>) -> Result<()> {
    let target = socks5::accept(&mut client).await?;

    match pool.connect(&target).await {
        Ok((upstream, mut stream)) => {
            socks5::reply_success(&mut client).await?;
            debug!(%target, %upstream, "SOCKS5 connection open");
            let (client_to_upstream, upstream_to_client) =
                copy_bidirectional(&mut client, &mut stream).await?;
            debug!(
                %target,
                sent = client_to_upstream,
                received = upstream_to_client,
                "SOCKS5 connection closed"
            );
            Ok(())
        }
        Err(error) => {
            let _ = socks5::reply_error(&mut client, 0x01).await;
            Err(error)
        }
    }
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    host: String,
    port: u16,
    /// Request head rewritten for origin-form, when the client sent an
    /// absolute URI (plain HTTP proxying).
    rewritten_head: Option<Vec<u8>>,
}

async fn handle_http(mut client: TcpStream, pool: Arc<Pool>) -> Result<()> {
    let (head, leftover) = read_head(&mut client).await?;
    let request = parse_request(&head)?;
    let target = Target::new(request.host.clone(), request.port);
    let is_connect = request.method.eq_ignore_ascii_case("CONNECT");

    let mut stream = match pool.connect(&target).await {
        Ok((_upstream, stream)) => stream,
        Err(error) => {
            reply_bad_gateway(&mut client).await;
            return Err(error);
        }
    };

    if is_connect {
        client
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await?;
        debug!(%target, "HTTP CONNECT open");
    } else {
        match request.rewritten_head {
            Some(rewritten) => stream.write_all(&rewritten).await?,
            None => stream.write_all(&head).await?,
        }
        debug!(%target, "HTTP request forwarded");
    }

    if !leftover.is_empty() {
        stream.write_all(&leftover).await?;
    }

    let (up, down) = copy_bidirectional(&mut client, &mut stream).await?;
    debug!(%target, sent = up, received = down, "HTTP connection closed");
    Ok(())
}

async fn read_head(client: &mut TcpStream) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut buffer = Vec::with_capacity(2048);
    let mut chunk = [0u8; 1024];

    loop {
        let read = client.read(&mut chunk).await?;
        if read == 0 {
            return Err(ProxyError::Protocol(
                "client closed before sending a complete request head".into(),
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);

        if let Some(end) = find_head_end(&buffer) {
            let head = buffer[..end + 4].to_vec();
            let leftover = buffer[end + 4..].to_vec();
            return Ok((head, leftover));
        }
        if buffer.len() > MAX_HEAD {
            return Err(ProxyError::Protocol("HTTP request head too large".into()));
        }
    }
}

fn find_head_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|w| w == b"\r\n\r\n")
}

fn parse_request(head: &[u8]) -> Result<HttpRequest> {
    let text = std::str::from_utf8(head)
        .map_err(|_| ProxyError::Protocol("request head is not valid UTF-8".into()))?;

    let first_line_end = text
        .find("\r\n")
        .ok_or_else(|| ProxyError::Protocol("request has no request line".into()))?;
    let first_line = &text[..first_line_end];

    let mut parts = first_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| ProxyError::Protocol("missing method".into()))?
        .to_string();
    let uri = parts
        .next()
        .ok_or_else(|| ProxyError::Protocol("missing request target".into()))?;
    let version = parts.next().unwrap_or("HTTP/1.1");

    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = split_authority(uri, 443)?;
        return Ok(HttpRequest {
            method,
            host,
            port,
            rewritten_head: None,
        });
    }

    let rest = uri.strip_prefix("http://").ok_or_else(|| {
        ProxyError::Protocol(format!(
            "unsupported request target '{uri}' (need CONNECT or an absolute http:// URI)"
        ))
    })?;

    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    let (host, port) = split_authority(authority, 80)?;

    // Origin-form for the upstream server, headers untouched.
    let new_first_line = format!("{method} {path} {version}\r\n");
    let mut rewritten = Vec::with_capacity(new_first_line.len() + head.len());
    rewritten.extend_from_slice(new_first_line.as_bytes());
    rewritten.extend_from_slice(&head[first_line_end + 2..]);

    Ok(HttpRequest {
        method,
        host,
        port,
        rewritten_head: Some(rewritten),
    })
}

fn split_authority(authority: &str, default_port: u16) -> Result<(String, u16)> {
    if let Some(stripped) = authority.strip_prefix('[') {
        // [ipv6]:port
        if let Some(close) = stripped.find(']') {
            let host = stripped[..close].to_string();
            let port = stripped[close + 1..]
                .strip_prefix(':')
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(default_port);
            return Ok((host, port));
        }
    }

    // An unbracketed IPv6 literal contains two or more colons and has no port.
    if authority.matches(':').count() > 1 {
        return Ok((authority.to_string(), default_port));
    }

    match authority.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty() => {
            let port = port
                .parse::<u16>()
                .map_err(|_| ProxyError::Protocol(format!("bad port in '{authority}'")))?;
            Ok((host.to_string(), port))
        }
        _ => Ok((authority.to_string(), default_port)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_connect_request() {
        let head = b"CONNECT example.com:443 HTTP/1.1\r\nHost: example.com:443\r\n\r\n";
        let request = parse_request(head).expect("parse");
        assert_eq!(request.method, "CONNECT");
        assert_eq!(request.host, "example.com");
        assert_eq!(request.port, 443);
        assert!(request.rewritten_head.is_none());
    }

    #[test]
    fn rewrites_absolute_uri_to_origin_form() {
        let head = b"GET http://example.com/a/b?q=1 HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let request = parse_request(head).expect("parse");
        assert_eq!(request.host, "example.com");
        assert_eq!(request.port, 80);
        let rewritten = String::from_utf8(request.rewritten_head.expect("rewritten"));
        assert!(rewritten.unwrap().starts_with("GET /a/b?q=1 HTTP/1.1\r\n"));
    }

    #[test]
    fn honours_an_explicit_port_in_an_absolute_uri() {
        let head = b"GET http://example.com:8080/p HTTP/1.1\r\nHost: example.com:8080\r\n\r\n";
        let request = parse_request(head).expect("parse");
        assert_eq!(request.host, "example.com");
        assert_eq!(request.port, 8080);
    }

    #[test]
    fn rejects_https_absolute_uris() {
        let head = b"GET https://example.com/ HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let error = parse_request(head).expect_err("https needs CONNECT");
        assert!(error.to_string().contains("CONNECT"), "{error}");
    }

    #[test]
    fn splits_ipv6_authorities() {
        let (host, port) = split_authority("[2001:db8::1]:8443", 443).expect("split");
        assert_eq!(host, "2001:db8::1");
        assert_eq!(port, 8443);

        let (host, port) = split_authority("2001:db8::1", 443).expect("split");
        assert_eq!(host, "2001:db8::1");
        assert_eq!(port, 443);
    }

    // --- end to end, with a local relaying SOCKS5 upstream instead of PIA ---

    use crate::upstream::Upstream;
    use std::time::Duration;
    use tokio::net::TcpListener;

    /// Trivial HTTP server that answers every request with "hello".
    async fn hello_http_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind http");
        let port = listener.local_addr().expect("addr").port();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut buffer = [0u8; 2048];
                    let _ = socket.read(&mut buffer).await;
                    let _ = socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello")
                        .await;
                });
            }
        });
        port
    }

    /// A SOCKS5 proxy that really relays bytes to the requested target.
    async fn relaying_socks5() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind socks");
        let address = listener.local_addr().expect("addr").to_string();
        tokio::spawn(async move {
            loop {
                let Ok((mut client, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut greeting = [0u8; 2];
                    if client.read_exact(&mut greeting).await.is_err() {
                        return;
                    }
                    let mut methods = vec![0u8; greeting[1] as usize];
                    if client.read_exact(&mut methods).await.is_err() {
                        return;
                    }
                    if client.write_all(&[5, 0]).await.is_err() {
                        return;
                    }

                    let mut head = [0u8; 4];
                    if client.read_exact(&mut head).await.is_err() {
                        return;
                    }
                    let host = match head[3] {
                        1 => {
                            let mut octets = [0u8; 4];
                            if client.read_exact(&mut octets).await.is_err() {
                                return;
                            }
                            std::net::Ipv4Addr::from(octets).to_string()
                        }
                        3 => {
                            let len = match client.read_u8().await {
                                Ok(len) => len as usize,
                                Err(_) => return,
                            };
                            let mut name = vec![0u8; len];
                            if client.read_exact(&mut name).await.is_err() {
                                return;
                            }
                            String::from_utf8_lossy(&name).to_string()
                        }
                        _ => return,
                    };
                    let mut port_bytes = [0u8; 2];
                    if client.read_exact(&mut port_bytes).await.is_err() {
                        return;
                    }
                    let port = u16::from_be_bytes(port_bytes);

                    match tokio::net::TcpStream::connect((host.as_str(), port)).await {
                        Ok(mut upstream) => {
                            if client.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await.is_err() {
                                return;
                            }
                            let _ = copy_bidirectional(&mut client, &mut upstream).await;
                        }
                        Err(_) => {
                            let _ = client.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await;
                        }
                    }
                });
            }
        });
        address
    }

    async fn start_server(upstream: &str) -> u16 {
        let pool = Arc::new(
            Pool::new(vec![Arc::new(Upstream::socks5("local", upstream, "u", "p"))], 1)
                .expect("pool"),
        );
        // Reserve a port, release it, then let the listener take it.
        let probe = TcpListener::bind("127.0.0.1:0").await.expect("probe");
        let port = probe.local_addr().expect("addr").port();
        drop(probe);

        tokio::spawn(async move {
            let _ = run_listener("127.0.0.1".to_string(), port, pool, false).await;
        });
        tokio::time::sleep(Duration::from_millis(120)).await;
        port
    }

    #[tokio::test]
    async fn a_pool_with_no_regions_left_stops_the_listener() {
        let pool = Pool::new(
            vec![Arc::new(Upstream::socks5("dead", "127.0.0.1:1", "u", "p"))],
            1,
        )
        .expect("pool")
        .with_retirement(1, Duration::from_secs(3600));

        assert!(stop_reason(&pool).is_none(), "one live region is fine");

        let _ = pool.connect(&Target::new("example.com", 80)).await;
        let reason = stop_reason(&pool).expect("an empty rotation must stop the listener");
        assert!(reason.to_string().contains("retired"), "{reason}");
    }

    #[tokio::test]
    async fn proxies_plain_http_with_an_absolute_uri() {
        let http_port = hello_http_server().await;
        let server_port = start_server(&relaying_socks5().await).await;

        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", server_port))
            .await
            .expect("connect to our proxy");
        let request = format!(
            "GET http://127.0.0.1:{http_port}/x HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        );
        client
            .write_all(request.as_bytes())
            .await
            .expect("write request");

        let mut response = Vec::new();
        client
            .read_to_end(&mut response)
            .await
            .expect("read response");
        let text = String::from_utf8_lossy(&response);
        assert!(text.contains("200 OK"), "{text}");
        assert!(text.ends_with("hello"), "{text}");
    }

    #[tokio::test]
    async fn accepts_socks5_clients_and_relays() {
        let http_port = hello_http_server().await;
        let server_port = start_server(&relaying_socks5().await).await;

        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", server_port))
            .await
            .expect("connect to our proxy");

        // Greeting: SOCKS5, no auth only.
        client.write_all(&[5, 1, 0]).await.expect("greeting");
        let mut choice = [0u8; 2];
        client.read_exact(&mut choice).await.expect("choice");
        assert_eq!(choice, [5, 0]);

        // CONNECT 127.0.0.1:http_port
        let mut request = vec![5, 1, 0, 1];
        request.extend_from_slice(&[127, 0, 0, 1]);
        request.extend_from_slice(&http_port.to_be_bytes());
        client.write_all(&request).await.expect("connect request");

        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).await.expect("reply");
        assert_eq!(reply[0], 5);
        assert_eq!(reply[1], 0, "CONNECT should be granted");

        client
            .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .await
            .expect("write http");

        let mut response = Vec::new();
        client.read_to_end(&mut response).await.expect("read");
        let text = String::from_utf8_lossy(&response);
        assert!(text.contains("200 OK"), "{text}");
        assert!(text.ends_with("hello"), "{text}");
    }
}

async fn reply_bad_gateway(client: &mut TcpStream) {
    let _ = client
        .write_all(
            b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
}
