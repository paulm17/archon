use pia_proxy::pia::{Endpoint, PiaRegion, Servers};
use pia_proxy::{Config, PiaClient};

const MINIMAL: &str = r#"
[listen]
port = 9999

[pia]
username = "p1234567"
password = "secret"

[[regions]]
id = "us_chicago"
name = "US Chicago"
"#;

#[test]
fn parses_minimal_config() {
    let config = Config::from_str(MINIMAL).expect("config should parse");
    assert_eq!(config.listen.port, 9999);
    assert_eq!(config.listen.host, "127.0.0.1");
    assert_eq!(config.rotation.max_attempts, 3);
    assert_eq!(config.rotation.cooldown_secs, 60);
    assert_eq!(config.regions.len(), 1);
    assert_eq!(config.regions[0].id, "us_chicago");
    assert!(config.socks5.is_none());
}

#[tokio::test]
async fn loads_credentials_from_the_dotenv_next_to_the_config() {
    let dir = std::env::temp_dir().join(format!("pia-config-test-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, "[[regions]]\nid = 'us_chicago'\n").unwrap();
    std::fs::write(
        dir.join(".env"),
        "PIA_USER = 'fixture-user'\nPIA_PASS = 'fixture password #1'\n",
    )
    .unwrap();
    let result = Config::load(&path).await;
    std::fs::remove_dir_all(dir).unwrap();
    let config = result.unwrap();
    let expected_user = std::env::var("PIA_USER").ok();
    let expected_pass = std::env::var("PIA_PASS").ok();
    assert!(
        config.pia.username
            == expected_user
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("fixture-user")
    );
    assert!(
        config.pia.password
            == expected_pass
                .as_deref()
                .filter(|value| !value.is_empty())
                .unwrap_or("fixture password #1")
    );
}

#[test]
fn rejects_missing_credentials() {
    let raw = r#"
[pia]
username = ""
password = ""

[[regions]]
id = "us_chicago"
"#;
    let error = Config::from_str(raw).expect_err("empty credentials must fail");
    assert!(
        error.to_string().contains("PIA_USER"),
        "unexpected: {error}"
    );
}

#[test]
fn rejects_config_with_no_upstreams() {
    let raw = r#"
[pia]
username = "p1"
password = "p"
"#;
    let error = Config::from_str(raw).expect_err("no upstreams must fail");
    assert!(error.to_string().contains("at least one"));
}

#[test]
fn rejects_duplicate_regions() {
    let raw = r#"
[pia]
username = "p1"
password = "p"

[[regions]]
id = "us_chicago"

[[regions]]
id = "us_chicago"
"#;
    let error = Config::from_str(raw).expect_err("duplicate region must fail");
    assert!(error.to_string().contains("more than once"));
}

#[test]
fn rejects_zero_attempts() {
    let raw = r#"
[pia]
username = "p1"
password = "p"

[rotation]
max_attempts = 0

[[regions]]
id = "us_chicago"
"#;
    let error = Config::from_str(raw).expect_err("zero attempts must fail");
    assert!(error.to_string().contains("max_attempts"));
}

#[test]
fn collects_pinned_ports() {
    let raw = r#"
[pia]
username = "p1"
password = "p"

[[regions]]
id = "us_chicago"
port = 9005

[[regions]]
id = "ca_ontario"
"#;
    let config = Config::from_str(raw).expect("config should parse");
    let pinned = config.pinned_ports();
    assert_eq!(pinned, vec![(9005, "us_chicago".to_string())]);
}

fn region(id: &str) -> PiaRegion {
    PiaRegion {
        id: id.to_string(),
        name: id.to_string(),
        country: "US".to_string(),
        offline: false,
        port_forward: false,
        servers: Servers {
            wg: vec![Endpoint {
                ip: "203.0.113.1".to_string(),
                cn: "server401".to_string(),
            }],
            ..Default::default()
        },
    }
}

#[test]
fn finds_known_region() {
    let regions = vec![region("us_chicago"), region("us_new_york_city")];
    let found = PiaClient::find_region(&regions, "us_new_york_city").expect("should find region");
    assert_eq!(found.id, "us_new_york_city");
}

#[test]
fn unknown_region_suggests_near_matches() {
    let regions = vec![region("us_chicago"), region("us_new_york_city")];
    let error = PiaClient::find_region(&regions, "us_new_york").expect_err("must be unknown");
    let message = error.to_string();
    assert!(
        message.contains("unknown PIA region 'us_new_york'"),
        "{message}"
    );
    assert!(message.contains("us_new_york_city"), "{message}");
}
