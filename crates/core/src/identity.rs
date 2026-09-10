//! Platform identity: roles, the authenticated-caller payload, and token
//! hashing shared by the bearer middleware (web/control) and the users
//! store. Tokens are never stored or logged in plaintext — only their
//! sha256 hex digests.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Platform role. Wire format is snake_case (`admin` / `root` / `user`).
/// The OS-level identity of node processes is unchanged; roles gate only
/// the platform HTTP surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    Root,
    User,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Root => "root",
            Self::User => "user",
        }
    }
}

/// Parse a role from its wire name, rejecting unknown values.
pub fn parse_role(value: &str) -> Option<Role> {
    match value {
        "admin" => Some(Role::Admin),
        "root" => Some(Role::Root),
        "user" => Some(Role::User),
        _ => None,
    }
}

/// The authenticated caller attached to a request by the bearer middleware.
/// Serialized as-is by `GET /api/me`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub name: String,
    pub role: Role,
}

impl Identity {
    pub fn admin(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            role: Role::Admin,
        }
    }

    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }
}

/// sha256 hex digest of a bearer token; the only stored representation.
pub fn token_hash(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_serialize_snake_case() {
        assert_eq!(serde_json::to_string(&Role::Admin).unwrap(), "\"admin\"");
        assert_eq!(serde_json::to_string(&Role::Root).unwrap(), "\"root\"");
        assert_eq!(serde_json::to_string(&Role::User).unwrap(), "\"user\"");
        assert_eq!(serde_json::from_str::<Role>("\"root\"").unwrap(), Role::Root);
        assert!(serde_json::from_str::<Role>("\"superuser\"").is_err());
        for role in [Role::Admin, Role::Root, Role::User] {
            assert_eq!(parse_role(role.as_str()), Some(role));
        }
        assert_eq!(parse_role("nope"), None);
    }

    #[test]
    fn identity_serializes_name_and_role() {
        let identity = Identity::admin("ops");
        assert_eq!(
            serde_json::to_value(&identity).unwrap(),
            serde_json::json!({"name": "ops", "role": "admin"})
        );
        assert!(identity.is_admin());
    }

    #[test]
    fn token_hash_is_stable_hex_sha256() {
        assert_eq!(token_hash("secret").len(), 64);
        assert_eq!(token_hash("secret"), token_hash("secret"));
        assert_ne!(token_hash("secret"), token_hash("Secret"));
        assert_eq!(
            token_hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
