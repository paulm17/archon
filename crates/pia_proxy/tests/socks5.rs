use pia_proxy::socks5;
use pia_proxy::upstream::{Stream, Upstream};
use pia_proxy::{Pool, Target};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A SOCKS5 server that demands username/password, exactly like PIA's proxy.
async fn mock_socks5(expected_user: &'static str, expected_pass: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind mock");
    let address = listener.local_addr().expect("addr").to_string();

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut greeting = [0u8; 2];
                if socket.read_exact(&mut greeting).await.is_err() {
                    return;
                }
                let mut methods = vec![0u8; greeting[1] as usize];
                if socket.read_exact(&mut methods).await.is_err() {
                    return;
                }
                // Always demand user/pass, like PIA.
                if socket.write_all(&[5, 2]).await.is_err() {
                    return;
                }

                let mut auth_header = [0u8; 2];
                if socket.read_exact(&mut auth_header).await.is_err() {
                    return;
                }
                let mut user = vec![0u8; auth_header[1] as usize];
                if socket.read_exact(&mut user).await.is_err() {
                    return;
                }
                let pass_len = match socket.read_u8().await {
                    Ok(len) => len as usize,
                    Err(_) => return,
                };
                let mut pass = vec![0u8; pass_len];
                if socket.read_exact(&mut pass).await.is_err() {
                    return;
                }

                let accepted = user == expected_user.as_bytes() && pass == expected_pass.as_bytes();
                let status = if accepted { 0u8 } else { 1u8 };
                if socket.write_all(&[1, status]).await.is_err() || !accepted {
                    return;
                }

                let mut request = [0u8; 4];
                if socket.read_exact(&mut request).await.is_err() {
                    return;
                }
                match request[3] {
                    1 => {
                        let mut octets = [0u8; 4];
                        socket.read_exact(&mut octets).await.ok();
                    }
                    3 => {
                        let len = socket.read_u8().await.unwrap_or(0) as usize;
                        let mut name = vec![0u8; len];
                        socket.read_exact(&mut name).await.ok();
                    }
                    4 => {
                        let mut octets = [0u8; 16];
                        socket.read_exact(&mut octets).await.ok();
                    }
                    _ => return,
                }
                let mut port = [0u8; 2];
                socket.read_exact(&mut port).await.ok();

                if socket.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await.is_ok() {
                    // Hold the connection open so the client sees success.
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            });
        }
    });

    address
}

#[tokio::test]
async fn authenticates_with_username_and_password() {
    let endpoint = mock_socks5("x1234567", "proxy-secret").await;
    let target = Target::new("example.com", 443);

    let stream = socks5::connect(&endpoint, Some(("x1234567", "proxy-secret")), &target)
        .await
        .expect("handshake should succeed");
    drop(stream);
}

#[tokio::test]
async fn reports_rejected_credentials() {
    let endpoint = mock_socks5("x1234567", "proxy-secret").await;
    let target = Target::new("example.com", 443);

    let error = socks5::connect(&endpoint, Some(("x1234567", "wrong")), &target)
        .await
        .expect_err("bad credentials must fail");
    assert!(error.to_string().contains("rejected credentials"), "{error}");
}

#[tokio::test]
async fn sends_domain_names_not_just_ips() {
    let endpoint = mock_socks5("x1", "p1").await;
    let target = Target::new("ifconfig.me", 80);
    let stream = socks5::connect(&endpoint, Some(("x1", "p1")), &target)
        .await
        .expect("domain target should be accepted");
    drop(stream);
}

#[tokio::test]
async fn pool_rotates_round_robin() {
    let first = mock_socks5("u", "p").await;
    let second = mock_socks5("u", "p").await;

    let pool = Pool::new(
        vec![
            Arc::new(Upstream::socks5("region-a", first, "u", "p")),
            Arc::new(Upstream::socks5("region-b", second, "u", "p")),
        ],
        2,
    )
    .expect("pool");

    let target = Target::new("example.com", 80);
    let (first_name, _) = pool.connect(&target).await.expect("first connect");
    let (second_name, _) = pool.connect(&target).await.expect("second connect");
    let (third_name, _) = pool.connect(&target).await.expect("third connect");

    assert_eq!(first_name, "region-a");
    assert_eq!(second_name, "region-b");
    assert_eq!(third_name, "region-a");
}

#[tokio::test]
async fn pool_fails_over_to_a_healthy_upstream() {
    // Port 1 on loopback refuses connections, standing in for a dead region.
    let pool = Pool::new(
        vec![
            Arc::new(Upstream::socks5("dead", "127.0.0.1:1", "u", "p")),
            Arc::new(Upstream::socks5(
                "alive",
                mock_socks5("u", "p").await,
                "u",
                "p",
            )),
        ],
        2,
    )
    .expect("pool");

    let (name, stream) = pool
        .connect(&Target::new("example.com", 80))
        .await
        .expect("failover should succeed");
    assert_eq!(name, "alive");
    assert!(matches!(stream, Stream::Tcp(_)));
}

#[tokio::test]
async fn pool_reports_failure_when_every_upstream_dies() {
    let pool = Pool::new(
        vec![
            Arc::new(Upstream::socks5("dead-1", "127.0.0.1:1", "u", "p")),
            Arc::new(Upstream::socks5("dead-2", "127.0.0.1:2", "u", "p")),
        ],
        2,
    )
    .expect("pool");

    let error = pool
        .connect(&Target::new("example.com", 80))
        .await
        .expect_err("all upstreams are dead");
    assert!(error.to_string().contains("no usable upstream") || error.to_string().contains("failed"));
}

/// A SOCKS5 server that drops the first `failures` connections, then behaves.
async fn flaky_socks5(failures: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind flaky");
    let address = listener.local_addr().expect("addr").to_string();

    tokio::spawn(async move {
        let mut seen = 0usize;
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            seen += 1;
            if seen <= failures {
                drop(socket);
                continue;
            }
            tokio::spawn(async move {
                let mut greeting = [0u8; 2];
                if socket.read_exact(&mut greeting).await.is_err() {
                    return;
                }
                let mut methods = vec![0u8; greeting[1] as usize];
                if socket.read_exact(&mut methods).await.is_err() {
                    return;
                }
                if socket.write_all(&[5, 2]).await.is_err() {
                    return;
                }
                let mut auth_header = [0u8; 2];
                if socket.read_exact(&mut auth_header).await.is_err() {
                    return;
                }
                let mut user = vec![0u8; auth_header[1] as usize];
                if socket.read_exact(&mut user).await.is_err() {
                    return;
                }
                let pass_len = socket.read_u8().await.unwrap_or(0) as usize;
                let mut pass = vec![0u8; pass_len];
                if socket.read_exact(&mut pass).await.is_err() {
                    return;
                }
                if socket.write_all(&[1, 0]).await.is_err() {
                    return;
                }
                let mut request = [0u8; 4];
                if socket.read_exact(&mut request).await.is_err() {
                    return;
                }
                match request[3] {
                    1 => {
                        let mut octets = [0u8; 4];
                        socket.read_exact(&mut octets).await.ok();
                    }
                    3 => {
                        let len = socket.read_u8().await.unwrap_or(0) as usize;
                        let mut name = vec![0u8; len];
                        socket.read_exact(&mut name).await.ok();
                    }
                    4 => {
                        let mut octets = [0u8; 16];
                        socket.read_exact(&mut octets).await.ok();
                    }
                    _ => return,
                }
                let mut port = [0u8; 2];
                socket.read_exact(&mut port).await.ok();
                let _ = socket.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await;
                tokio::time::sleep(Duration::from_millis(50)).await;
            });
        }
    });

    address
}

/// Accepts connections and then says nothing, like a blackholed tunnel: no
/// error, no data, just silence until the caller's own deadline.
async fn silent_tcp() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind silent");
    let address = listener.local_addr().expect("addr").to_string();
    tokio::spawn(async move {
        let mut held = Vec::new();
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            held.push(socket);
        }
    });
    address
}

#[tokio::test]
async fn failover_stays_inside_its_budget() {
    // One upstream that never answers, and two that refuse at once. The whole
    // failover has to finish inside the budget: it runs while a browser
    // navigation waits on this connection, and that navigation has its own
    // deadline.
    let silent = silent_tcp().await;
    let pool = Pool::new(
        vec![
            Arc::new(Upstream::socks5("silent", silent, "u", "p")),
            Arc::new(Upstream::socks5("refused-1", "127.0.0.1:1", "u", "p")),
            Arc::new(Upstream::socks5("refused-2", "127.0.0.1:2", "u", "p")),
        ],
        8,
    )
    .expect("pool")
    .with_connect_budget(Duration::from_millis(1200));

    let started = std::time::Instant::now();
    let error = pool
        .connect(&Target::new("example.com", 80))
        .await
        .expect_err("nothing can answer");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(2400),
        "failover took {elapsed:?} with a 1200ms budget: {error}"
    );
    // All three were tried: the silent upstream did not swallow the budget.
    assert!(
        error.to_string().contains("after 3 attempt(s)"),
        "{error}"
    );
}

#[tokio::test]
async fn a_single_blip_does_not_retire_an_upstream() {
    let pool = Pool::new(
        vec![Arc::new(Upstream::socks5("blip", "127.0.0.1:1", "u", "p"))],
        1,
    )
    .expect("pool");

    let target = Target::new("example.com", 80);
    assert!(pool.connect(&target).await.is_err());

    // One failure is a blip: the region stays in rotation.
    assert_eq!(pool.live(), 1);
    assert!(pool.retired_names().is_empty());
    assert_eq!(pool.names(), vec!["blip"]);
}

#[tokio::test]
async fn a_retired_upstream_is_probed_again_and_revives() {
    let endpoint = flaky_socks5(2).await;
    let pool = Pool::new(
        vec![Arc::new(Upstream::socks5("flaky", endpoint, "u", "p"))],
        1,
    )
    .expect("pool")
    .with_retirement(2, Duration::from_millis(50));

    let target = Target::new("example.com", 80);
    assert!(pool.connect(&target).await.is_err());
    assert_eq!(pool.live(), 1, "one failure is only a blip");

    assert!(pool.connect(&target).await.is_err());
    assert_eq!(pool.live(), 0, "two failures retire it");
    assert_eq!(pool.retired_names(), vec!["flaky"]);

    // After the cooldown it is probed again and comes back.
    tokio::time::sleep(Duration::from_millis(80)).await;
    let (name, _) = pool.connect(&target).await.expect("probe should revive it");
    assert_eq!(name, "flaky");
    assert_eq!(pool.live(), 1);
    assert!(pool.retired_names().is_empty());
}

#[tokio::test]
async fn pool_retires_dead_upstreams_and_keeps_serving() {
    let pool = Pool::new(
        vec![
            Arc::new(Upstream::socks5("dead-1", "127.0.0.1:1", "u", "p")),
            Arc::new(Upstream::socks5("dead-2", "127.0.0.1:2", "u", "p")),
        ],
        8,
    )
    .expect("pool");

    let target = Target::new("example.com", 80);
    assert_eq!(pool.live(), 2);

    // Three consecutive failures each, not one blip, before they are retired.
    for _ in 1..=3 {
        let error = pool.connect(&target).await.expect_err("both upstreams are down");
        assert!(error.to_string().contains("no usable upstream"), "{error}");
    }

    // Both are out of rotation, but the pool itself stays alive: the listener
    // must never stop accepting because of it.
    assert_eq!(pool.live(), 0);
    assert!(pool.is_exhausted());
    assert!(pool.names().is_empty());
    assert_eq!(pool.retired_names(), vec!["dead-1", "dead-2"]);

    // A later request still tries the retired upstreams rather than refusing
    // outright, and every one of them is probed — not just the first.
    let error = pool.connect(&target).await.expect_err("still down");
    assert!(
        error.to_string().contains("after 2 attempt(s)"),
        "a request after exhaustion must probe every retired upstream: {error}"
    );
}

#[tokio::test]
async fn a_pool_with_nothing_in_rotation_probes_a_retired_upstream() {
    // Drops its first connection, then behaves. One failure retires it, and the
    // cooldown is far longer than the test, so only a forced probe can revive it.
    let endpoint = flaky_socks5(1).await;
    let pool = Pool::new(
        vec![Arc::new(Upstream::socks5("flaky", endpoint, "u", "p"))],
        1,
    )
    .expect("pool")
    .with_retirement(1, Duration::from_secs(3600));

    let target = Target::new("example.com", 80);
    assert!(pool.connect(&target).await.is_err());
    assert!(pool.is_exhausted(), "a single failure retired it");

    let (name, _) = pool
        .connect(&target)
        .await
        .expect("a request after exhaustion must still probe");
    assert_eq!(name, "flaky");
    assert_eq!(pool.live(), 1, "the probe put it back in rotation");
}

#[tokio::test]
async fn a_pinned_port_shares_health_with_the_shared_pool() {
    // Pinned ports used to keep their own strikes, so a pin could retire a
    // region the shared listener was still serving with — and vice versa.
    let pool = Arc::new(
        Pool::new(
            vec![
                Arc::new(Upstream::socks5("healthy", "127.0.0.1:1", "u", "p")),
                Arc::new(Upstream::socks5("pinned", "127.0.0.1:2", "u", "p")),
            ],
            1,
        )
        .expect("pool")
        .with_retirement(1, Duration::from_secs(3600)),
    );
    let pin = pool.pinned(1).expect("pin");

    let target = Target::new("example.com", 80);
    assert!(pin.connect(&target).await.is_err());

    // The pin's failure retires that region everywhere, not just on the pin.
    assert!(pin.is_exhausted(), "the pinned region retired");
    assert_eq!(pool.live(), 1, "the shared pool sees the same retirement");
    assert_eq!(pool.retired_names(), vec!["pinned"]);
    assert_eq!(pool.names(), vec!["healthy"]);
}

#[tokio::test]
async fn telemetry_hosts_are_refused_without_spending_a_region() {
    // The blackholed upstream would hang for the whole budget if it were used,
    // so an instant refusal proves no tunnel was opened and no strike recorded.
    let silent = silent_tcp().await;
    let pool = Pool::new(
        vec![Arc::new(Upstream::socks5("silent", silent, "u", "p"))],
        1,
    )
    .expect("pool")
    .with_connect_budget(Duration::from_millis(1200));

    for host in [
        "static.cloudflareinsights.com",
        "www.google.com",
        "ogads-pa.clients6.google.com",
    ] {
        let started = std::time::Instant::now();
        let error = pool
            .connect(&Target::new(host, 443))
            .await
            .expect_err("{host} is telemetry");
        assert!(
            error.to_string().contains("telemetry"),
            "{host}: {error}"
        );
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "{host} should be refused without a tunnel attempt"
        );
    }

    assert_eq!(pool.live(), 1, "telemetry must never retire a region");
    assert!(pool.retired_names().is_empty());

    // Real content still goes through the upstream.
    let error = pool
        .connect(&Target::new("freely-given.org", 443))
        .await
        .expect_err("the upstream is silent");
    assert!(!error.to_string().contains("telemetry"), "{error}");
}

#[tokio::test]
async fn an_exhausted_pool_probes_every_retired_upstream() {
    // All three are down. Once retired, one page used to get a single probe and
    // fail immediately (ERR_TUNNEL_CONNECTION_FAILED) while two regions sat
    // untried; they must all be attempted inside the same budget.
    let pool = Pool::new(
        vec![
            Arc::new(Upstream::socks5("dead-1", "127.0.0.1:1", "u", "p")),
            Arc::new(Upstream::socks5("dead-2", "127.0.0.1:2", "u", "p")),
            Arc::new(Upstream::socks5("dead-3", "127.0.0.1:3", "u", "p")),
        ],
        8,
    )
    .expect("pool")
    .with_retirement(1, Duration::from_secs(3600));

    let target = Target::new("example.com", 80);
    assert!(pool.connect(&target).await.is_err());
    assert!(pool.is_exhausted(), "one failure each retires all three");

    let error = pool.connect(&target).await.expect_err("all down");
    assert!(
        error.to_string().contains("after 3 attempt(s)"),
        "every retired upstream must be probed: {error}"
    );
}

#[tokio::test]
async fn pool_keeps_survivors_in_rotation() {
    let alive = mock_socks5("u", "p").await;
    let pool = Pool::new(
        vec![
            Arc::new(Upstream::socks5("dead", "127.0.0.1:1", "u", "p")),
            Arc::new(Upstream::socks5("alive", alive, "u", "p")),
        ],
        8,
    )
    .expect("pool");

    let target = Target::new("example.com", 80);
    // The dead one is only probed on the requests that start on it, so give it
    // enough rounds to reach three failures.
    for _ in 0..6 {
        let (name, _) = pool.connect(&target).await.expect("failover");
        assert_eq!(name, "alive");
    }
    // Only the upstream that kept failing is out; the rest stay in rotation.
    assert_eq!(pool.live(), 1);
    assert_eq!(pool.retired_names(), vec!["dead"]);
}

