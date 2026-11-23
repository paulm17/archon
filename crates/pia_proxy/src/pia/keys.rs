use base64::{engine::general_purpose::STANDARD, Engine as _};
use boringtun::x25519::{PublicKey, StaticSecret};
use rand::rngs::OsRng;

/// A WireGuard keypair. `StaticSecret` clamps per RFC 7748 on use, so the
/// derived public key always matches the scalar that goes on the wire.
#[derive(Clone)]
pub struct Keypair {
    pub private: StaticSecret,
    pub public: PublicKey,
}

impl Keypair {
    pub fn generate() -> Self {
        let private = StaticSecret::random_from_rng(OsRng);
        let public = PublicKey::from(&private);
        Self { private, public }
    }

    /// Base64 public key, the form PIA's `/addKey` endpoint expects.
    pub fn public_base64(&self) -> String {
        STANDARD.encode(self.public.as_bytes())
    }

    /// Base64 private key, for logging a fingerprint or writing a config out.
    pub fn private_base64(&self) -> String {
        STANDARD.encode(self.private.to_bytes())
    }
}
