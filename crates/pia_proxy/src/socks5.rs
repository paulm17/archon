//! SOCKS5 (RFC 1928) with username/password auth (RFC 1929), both directions.
//!
//! Outbound: connect to a SOCKS5 proxy and open a CONNECT to the target.
//! Inbound: accept a SOCKS5 client so `socks5h://` consumers can use us.

use crate::error::{ProxyError, Result};
use crate::Target;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const VERSION: u8 = 0x05;
const METHOD_NO_AUTH: u8 = 0x00;
const METHOD_USER_PASS: u8 = 0x02;
const METHOD_NONE_ACCEPTABLE: u8 = 0xFF;
const CMD_CONNECT: u8 = 0x01;
const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;

fn push_address(buf: &mut Vec<u8>, host: &str) -> Result<()> {
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        buf.push(ATYP_IPV4);
        buf.extend_from_slice(&ip.octets());
    } else if let Ok(ip) = host.parse::<std::net::Ipv6Addr>() {
        buf.push(ATYP_IPV6);
        buf.extend_from_slice(&ip.octets());
    } else {
        if host.len() > 255 {
            return Err(ProxyError::Socks5("target hostname is longer than 255 bytes".into()));
        }
        buf.push(ATYP_DOMAIN);
        buf.push(host.len() as u8);
        buf.extend_from_slice(host.as_bytes());
    }
    Ok(())
}

async fn read_address(stream: &mut TcpStream, atyp: u8) -> Result<String> {
    let host = match atyp {
        ATYP_IPV4 => {
            let mut octets = [0u8; 4];
            stream.read_exact(&mut octets).await?;
            std::net::Ipv4Addr::from(octets).to_string()
        }
        ATYP_IPV6 => {
            let mut octets = [0u8; 16];
            stream.read_exact(&mut octets).await?;
            std::net::Ipv6Addr::from(octets).to_string()
        }
        ATYP_DOMAIN => {
            let len = stream.read_u8().await? as usize;
            let mut name = vec![0u8; len];
            stream.read_exact(&mut name).await?;
            String::from_utf8(name)
                .map_err(|_| ProxyError::Protocol("SOCKS5 hostname is not valid UTF-8".into()))?
        }
        other => {
            return Err(ProxyError::Protocol(format!(
                "unsupported SOCKS5 address type {other}"
            )))
        }
    };
    Ok(host)
}

/// Open a CONNECT through `proxy` to `target`, authenticating when configured.
pub async fn connect(
    proxy: &str,
    credentials: Option<(&str, &str)>,
    target: &Target,
) -> Result<TcpStream> {
    let mut stream = TcpStream::connect(proxy).await.map_err(|e| {
        ProxyError::Socks5(format!("cannot reach SOCKS5 proxy {proxy}: {e}"))
    })?;

    let methods: &[u8] = if credentials.is_some() {
        &[METHOD_NO_AUTH, METHOD_USER_PASS]
    } else {
        &[METHOD_NO_AUTH]
    };
    let mut greeting = Vec::with_capacity(2 + methods.len());
    greeting.push(VERSION);
    greeting.push(methods.len() as u8);
    greeting.extend_from_slice(methods);
    stream.write_all(&greeting).await?;

    let mut choice = [0u8; 2];
    stream.read_exact(&mut choice).await?;
    if choice[0] != VERSION {
        return Err(ProxyError::Protocol(format!(
            "proxy answered SOCKS version {}",
            choice[0]
        )));
    }

    match choice[1] {
        METHOD_NO_AUTH => {}
        METHOD_USER_PASS => {
            let (user, pass) = credentials.ok_or_else(|| {
                ProxyError::Socks5("proxy requires username/password but none is configured".into())
            })?;
            if user.len() > 255 || pass.len() > 255 {
                return Err(ProxyError::Socks5(
                    "SOCKS5 username/password must be at most 255 bytes".into(),
                ));
            }
            let mut auth = Vec::with_capacity(3 + user.len() + pass.len());
            auth.push(0x01);
            auth.push(user.len() as u8);
            auth.extend_from_slice(user.as_bytes());
            auth.push(pass.len() as u8);
            auth.extend_from_slice(pass.as_bytes());
            stream.write_all(&auth).await?;

            let mut status = [0u8; 2];
            stream.read_exact(&mut status).await?;
            if status[1] != 0x00 {
                return Err(ProxyError::Socks5(format!(
                    "SOCKS5 proxy rejected credentials (status {})",
                    status[1]
                )));
            }
        }
        METHOD_NONE_ACCEPTABLE => {
            return Err(ProxyError::Socks5(
                "SOCKS5 proxy accepted none of our auth methods".into(),
            ))
        }
        other => {
            return Err(ProxyError::Protocol(format!(
                "SOCKS5 proxy selected an unoffered auth method {other}"
            )))
        }
    }

    let mut request = Vec::with_capacity(32);
    request.push(VERSION);
    request.push(CMD_CONNECT);
    request.push(0x00);
    push_address(&mut request, &target.host)?;
    request.extend_from_slice(&target.port.to_be_bytes());
    stream.write_all(&request).await?;

    let mut head = [0u8; 4];
    stream.read_exact(&mut head).await?;
    if head[1] != 0x00 {
        return Err(ProxyError::Socks5(format!(
            "SOCKS5 CONNECT to {}:{} refused (reply code {})",
            target.host, target.port, head[1]
        )));
    }
    let _bound_host = read_address(&mut stream, head[3]).await?;
    let _bound_port = stream.read_u16().await?;

    Ok(stream)
}

/// Read a SOCKS5 client's greeting and CONNECT request, answering the greeting.
pub async fn accept(stream: &mut TcpStream) -> Result<Target> {
    let version = stream.read_u8().await?;
    if version != VERSION {
        return Err(ProxyError::Protocol(format!(
            "expected SOCKS5 greeting, got version {version}"
        )));
    }
    let count = stream.read_u8().await? as usize;
    let mut methods = vec![0u8; count];
    stream.read_exact(&mut methods).await?;

    if !methods.contains(&METHOD_NO_AUTH) {
        stream.write_all(&[VERSION, METHOD_NONE_ACCEPTABLE]).await?;
        return Err(ProxyError::Socks5(
            "client offered no acceptable auth method (we only accept no-auth)".into(),
        ));
    }
    stream.write_all(&[VERSION, METHOD_NO_AUTH]).await?;

    let mut head = [0u8; 4];
    stream.read_exact(&mut head).await?;
    if head[0] != VERSION {
        return Err(ProxyError::Protocol("bad SOCKS5 request version".into()));
    }
    if head[1] != CMD_CONNECT {
        stream
            .write_all(&[VERSION, 0x07, 0x00, ATYP_IPV4, 0, 0, 0, 0, 0, 0])
            .await?;
        return Err(ProxyError::Protocol(format!(
            "only SOCKS5 CONNECT is supported, got command {}",
            head[1]
        )));
    }

    let host = read_address(stream, head[3]).await?;
    let port = stream.read_u16().await?;
    Ok(Target::new(host, port))
}

/// Tell the client the connection is established.
pub async fn reply_success(stream: &mut TcpStream) -> Result<()> {
    stream
        .write_all(&[VERSION, 0x00, 0x00, ATYP_IPV4, 0, 0, 0, 0, 0, 0])
        .await?;
    Ok(())
}

/// Reject the client's CONNECT with a SOCKS5 reply code.
pub async fn reply_error(stream: &mut TcpStream, code: u8) -> Result<()> {
    stream
        .write_all(&[VERSION, code, 0x00, ATYP_IPV4, 0, 0, 0, 0, 0, 0])
        .await?;
    Ok(())
}
