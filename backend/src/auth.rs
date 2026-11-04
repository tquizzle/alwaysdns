//! Local users (argon2 password hashes from the environment) and in-memory
//! sessions identified by random tokens in an HttpOnly cookie.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use serde::{Deserialize, Serialize};

pub const SESSION_COOKIE: &str = "dnsui_session";
const SESSION_TTL: Duration = Duration::from_secs(12 * 60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Viewer,
}

impl Role {
    pub fn is_admin(self) -> bool {
        self == Role::Admin
    }
}

struct User {
    username: String,
    password_hash: String,
    role: Role,
}

pub struct Users {
    users: Vec<User>,
}

/// The admin login resolved at startup.
pub struct AdminCredentials {
    pub username: String,
    pub password_hash: String,
    /// Set only on the first start, when the password was generated here.
    /// It is shown once and never stored in plain text.
    pub generated_password: Option<String>,
}

const STORED_ADMIN_FILE: &str = "admin.json";
const GENERATED_PASSWORD_LEN: usize = 24;
/// 56 symbols, without look-alikes such as 0/O and 1/l/I.
const PASSWORD_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";

#[derive(Serialize, Deserialize)]
struct StoredAdmin {
    username: String,
    password_hash: String,
}

/// Resolves the admin login, in this order:
/// 1. `DNSUI_ADMIN_PASSWORD_HASH` from the environment (username from `DNSUI_ADMIN_USER`, default `admin`).
/// 2. `admin.json` in the data directory, written by an earlier start.
/// 3. A fresh data directory: a new random password is generated, its hash is saved
///    to `admin.json`, and the password is returned once for display.
pub fn resolve_admin(get: impl Fn(&str) -> Option<String>, data_dir: &Path) -> Result<AdminCredentials> {
    let non_empty = |key: &str| get(key).filter(|v| !v.trim().is_empty());
    let configured_user = non_empty("DNSUI_ADMIN_USER").unwrap_or_else(|| "admin".to_string());

    if let Some(hash) = non_empty("DNSUI_ADMIN_PASSWORD_HASH") {
        check_phc("DNSUI_ADMIN_PASSWORD_HASH", &hash)?;
        return Ok(AdminCredentials {
            username: configured_user,
            password_hash: hash,
            generated_password: None,
        });
    }

    let path = data_dir.join(STORED_ADMIN_FILE);
    if path.exists() {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let stored: StoredAdmin = serde_json::from_str(&raw)
            .with_context(|| format!("parsing {}", path.display()))?;
        check_phc("admin.json password_hash", &stored.password_hash)?;
        return Ok(AdminCredentials {
            username: stored.username,
            password_hash: stored.password_hash,
            generated_password: None,
        });
    }

    // Fresh data directory: create the first admin with a random password.
    let password = random_password()?;
    let password_hash = hash_password(&password)?;
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("creating data directory {}", data_dir.display()))?;
    let json = serde_json::to_string_pretty(&StoredAdmin {
        username: configured_user.clone(),
        password_hash: password_hash.clone(),
    })?;
    write_private(&path, json.as_bytes())?;
    Ok(AdminCredentials {
        username: configured_user,
        password_hash,
        generated_password: Some(password),
    })
}

/// Writes a new file readable only by its owner. Fails if the file already exists.
fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts
        .open(path)
        .with_context(|| format!("writing {}", path.display()))?;
    file.write_all(data)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Random password from the OS source. Bytes outside the unbiased range are rejected.
fn random_password() -> Result<String> {
    let limit = (256 / PASSWORD_ALPHABET.len()) * PASSWORD_ALPHABET.len();
    let mut out = String::with_capacity(GENERATED_PASSWORD_LEN);
    let mut buf = [0u8; 64];
    while out.len() < GENERATED_PASSWORD_LEN {
        getrandom::fill(&mut buf).map_err(|e| anyhow!("OS random source unavailable: {e}"))?;
        for &b in buf.iter() {
            if (b as usize) < limit && out.len() < GENERATED_PASSWORD_LEN {
                out.push(PASSWORD_ALPHABET[b as usize % PASSWORD_ALPHABET.len()] as char);
            }
        }
    }
    Ok(out)
}

impl Users {
    /// Builds the user list: the resolved admin plus an optional viewer from
    /// `DNSUI_VIEWER_USER` and `DNSUI_VIEWER_PASSWORD_HASH`.
    pub fn from_env(admin: AdminCredentials) -> Result<Self> {
        Self::from_lookup(|key| std::env::var(key).ok(), admin)
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>, admin: AdminCredentials) -> Result<Self> {
        let non_empty = |key: &str| get(key).filter(|v| !v.trim().is_empty());
        let admin_user = admin.username.clone();

        let mut users = vec![User {
            username: admin.username,
            password_hash: admin.password_hash,
            role: Role::Admin,
        }];

        if let Some(viewer_user) = non_empty("DNSUI_VIEWER_USER") {
            let viewer_hash = non_empty("DNSUI_VIEWER_PASSWORD_HASH")
                .context("DNSUI_VIEWER_PASSWORD_HASH is required when DNSUI_VIEWER_USER is set")?;
            check_phc("DNSUI_VIEWER_PASSWORD_HASH", &viewer_hash)?;
            if viewer_user == admin_user {
                bail!("DNSUI_VIEWER_USER must differ from DNSUI_ADMIN_USER");
            }
            users.push(User {
                username: viewer_user,
                password_hash: viewer_hash,
                role: Role::Viewer,
            });
        }

        Ok(Self { users })
    }

    /// Returns the role if the username and password match.
    pub fn verify(&self, username: &str, password: &str) -> Option<Role> {
        let user = self.users.iter().find(|u| u.username == username)?;
        let parsed = PasswordHash::new(&user.password_hash).ok()?;
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .ok()
            .map(|_| user.role)
    }
}

fn check_phc(var: &str, value: &str) -> Result<()> {
    PasswordHash::new(value)
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("{var} is not a valid argon2 PHC string: {e}"))
}

struct Session {
    username: String,
    role: Role,
    expires: Instant,
}

#[derive(Default)]
pub struct Sessions {
    inner: Mutex<HashMap<String, Session>>,
}

impl Sessions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(&self, username: &str, role: Role) -> String {
        let token = new_token();
        if let Ok(mut map) = self.inner.lock() {
            let now = Instant::now();
            map.retain(|_, s| s.expires > now);
            map.insert(
                token.clone(),
                Session {
                    username: username.to_string(),
                    role,
                    expires: now + SESSION_TTL,
                },
            );
        }
        token
    }

    /// Returns `(username, role)` for a live session. Expired sessions are removed.
    pub fn lookup(&self, token: &str) -> Option<(String, Role)> {
        let mut map = self.inner.lock().ok()?;
        let now = Instant::now();
        match map.get(token) {
            Some(s) if s.expires > now => Some((s.username.clone(), s.role)),
            Some(_) => {
                map.remove(token);
                None
            }
            None => None,
        }
    }

    pub fn remove(&self, token: &str) {
        if let Ok(mut map) = self.inner.lock() {
            map.remove(token);
        }
    }
}

/// 256 bits of OS randomness, hex encoded.
fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS random source is unavailable");
    hex::encode(bytes)
}

/// Extracts the session token from a `Cookie` header value.
pub fn token_from_cookie_header(header: &str) -> Option<&str> {
    header.split(';').map(str::trim).find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == SESSION_COOKIE && !value.is_empty()).then_some(value)
    })
}

/// Hashes a password with argon2id and a random salt, returning a PHC string.
pub fn hash_password(password: &str) -> Result<String> {
    use argon2::PasswordHasher;
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!("hashing failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn admin_from(hash: &str) -> AdminCredentials {
        AdminCredentials {
            username: "admin".into(),
            password_hash: hash.into(),
            generated_password: None,
        }
    }

    #[test]
    fn hash_then_verify_roundtrip() {
        let hash = hash_password("correct horse").unwrap();
        let users = Users::from_lookup(
            |k| match k {
                "DNSUI_VIEWER_USER" => Some("viewer".into()),
                "DNSUI_VIEWER_PASSWORD_HASH" => Some(hash.clone()),
                _ => None,
            },
            admin_from(&hash),
        )
        .unwrap();
        assert_eq!(users.verify("admin", "correct horse"), Some(Role::Admin));
        assert_eq!(users.verify("viewer", "correct horse"), Some(Role::Viewer));
        assert_eq!(users.verify("admin", "wrong"), None);
        assert_eq!(users.verify("nobody", "correct horse"), None);
    }

    #[test]
    fn fresh_data_dir_generates_and_then_reuses_admin() {
        let dir = std::env::temp_dir().join(format!("dnsui-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let first = resolve_admin(|_| None, &dir).unwrap();
        let generated = first.generated_password.clone().expect("password generated on first start");
        assert_eq!(first.username, "admin");
        assert_eq!(generated.len(), GENERATED_PASSWORD_LEN);
        let users = Users::from_lookup(|_| None, first).unwrap();
        assert_eq!(users.verify("admin", &generated), Some(Role::Admin));

        // A second start loads the stored hash and generates nothing.
        let second = resolve_admin(|_| None, &dir).unwrap();
        assert!(second.generated_password.is_none());
        let users = Users::from_lookup(|_| None, second).unwrap();
        assert_eq!(users.verify("admin", &generated), Some(Role::Admin));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_hash_overrides_stored_admin() {
        let dir = std::env::temp_dir().join(format!("dnsui-test-env-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let hash = hash_password("from-env").unwrap();
        let admin = resolve_admin(
            |k| match k {
                "DNSUI_ADMIN_PASSWORD_HASH" => Some(hash.clone()),
                _ => None,
            },
            &dir,
        )
        .unwrap();
        assert!(admin.generated_password.is_none());
        assert!(!dir.join(STORED_ADMIN_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn viewer_must_differ_from_admin() {
        let hash = hash_password("x").unwrap();
        let result = Users::from_lookup(
            |k| match k {
                "DNSUI_VIEWER_USER" => Some("admin".into()),
                "DNSUI_VIEWER_PASSWORD_HASH" => Some(hash.clone()),
                _ => None,
            },
            admin_from(&hash),
        );
        assert!(result.is_err());
    }

    #[test]
    fn cookie_parsing() {
        assert_eq!(
            token_from_cookie_header("a=1; dnsui_session=abc123; b=2"),
            Some("abc123")
        );
        assert_eq!(token_from_cookie_header("dnsui_session="), None);
        assert_eq!(token_from_cookie_header("other=1"), None);
    }

    #[test]
    fn sessions_expire_and_revoke() {
        let s = Sessions::new();
        let token = s.create("admin", Role::Admin);
        assert_eq!(s.lookup(&token), Some(("admin".to_string(), Role::Admin)));
        s.remove(&token);
        assert_eq!(s.lookup(&token), None);
    }
}
