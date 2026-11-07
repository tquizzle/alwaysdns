//! Limits on where admin-added list sources may point. A URL source must not
//! resolve to a loopback, unspecified, link-local or multicast address, and a
//! file source must lie inside the lists directory.
//!
//! Known limit: a hostname that re-resolves to a blocked address after the
//! check (DNS rebinding) is not caught. Redirects are checked only when the
//! target is a literal IP; a redirect to a hostname is followed unchecked.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

use reqwest::redirect::{Action, Attempt, Policy};
use url::{Host, Url};

/// Redirects a download may follow before it is abandoned.
const MAX_REDIRECTS: usize = 5;

/// Why a URL source was refused. `Blocked` is a hard refusal. `Unresolved` means
/// the host has no address to judge, which is a DNS failure, not an unsafe source.
/// A malformed URL is `Blocked`: it can never download, so no cached copy exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    Blocked(String),
    Unresolved(String),
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Blocked(msg) | Self::Unresolved(msg) => f.write_str(msg),
        }
    }
}

/// True for addresses a list source must never reach. Private LAN ranges
/// (RFC 1918, fc00::/7) are allowed on purpose: homelab users host lists there.
pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            // ::ffff:127.0.0.1 reaches the same host as 127.0.0.1, so judge it as IPv4.
            Some(v4) => is_blocked_v4(v4),
            None => is_blocked_v6(v6),
        },
    }
}

fn is_blocked_v4(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_unspecified() || ip.is_link_local() || ip.is_multicast()
}

fn is_blocked_v6(ip: Ipv6Addr) -> bool {
    let link_local = ip.segments()[0] & 0xffc0 == 0xfe80;
    ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() || link_local
}

/// Refuses a URL source when its host resolves to any blocked address. Nothing
/// is requested when this fails. Literal IPs are checked without a lookup.
pub async fn check_url_source(source: &str) -> Result<(), SourceError> {
    let url = Url::parse(source)
        .map_err(|e| SourceError::Blocked(format!("url is not valid: {e}")))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| SourceError::Blocked("url has no port".into()))?;
    let addrs = match url.host() {
        Some(Host::Ipv4(ip)) => vec![IpAddr::V4(ip)],
        Some(Host::Ipv6(ip)) => vec![IpAddr::V6(ip)],
        Some(Host::Domain(domain)) => resolve(domain, port).await?,
        None => return Err(SourceError::Blocked("url has no host".into())),
    };
    refuse_if_blocked(&addrs)
}

async fn resolve(host: &str, port: u16) -> Result<Vec<IpAddr>, SourceError> {
    let addrs: Vec<IpAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| SourceError::Unresolved(format!("could not resolve {host}: {e}")))?
        .map(|addr| addr.ip())
        .collect();
    if addrs.is_empty() {
        return Err(SourceError::Unresolved(format!(
            "{host} resolved to no addresses"
        )));
    }
    Ok(addrs)
}

fn refuse_if_blocked(addrs: &[IpAddr]) -> Result<(), SourceError> {
    match addrs.iter().find(|ip| is_blocked_ip(**ip)) {
        Some(ip) => Err(SourceError::Blocked(format!(
            "source resolves to {ip}, which is a loopback, unspecified, link-local or multicast address"
        ))),
        None => Ok(()),
    }
}

/// Redirect policy for the shared HTTP client: at most `MAX_REDIRECTS` hops,
/// and a hop to a literal blocked IP ends the download with an error.
pub fn redirect_policy() -> Policy {
    Policy::custom(redirect_action)
}

fn redirect_action(attempt: Attempt<'_>) -> Action {
    // previous() holds the original URL plus each followed hop, so the redirect
    // being decided is hop number previous().len().
    if attempt.previous().len() > MAX_REDIRECTS {
        return attempt.error("too many redirects");
    }
    let blocked = match attempt.url().host() {
        Some(Host::Ipv4(ip)) => is_blocked_ip(IpAddr::V4(ip)),
        Some(Host::Ipv6(ip)) => is_blocked_ip(IpAddr::V6(ip)),
        _ => false,
    };
    if blocked {
        attempt.error("redirect to a loopback, unspecified, link-local or multicast address")
    } else {
        attempt.follow()
    }
}

/// Directory that file list sources must live in: `DNSUI_LISTS_DIR`, or `lists`
/// inside the data directory (`DNSUI_DATA_DIR`, default `data`).
pub fn lists_dir() -> PathBuf {
    if let Some(dir) = non_empty_env("DNSUI_LISTS_DIR") {
        return PathBuf::from(dir);
    }
    let data = non_empty_env("DNSUI_DATA_DIR").unwrap_or_else(|| "data".to_string());
    PathBuf::from(data).join("lists")
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

/// Resolves a file source to its canonical path, which must be absolute and
/// must lie inside `dir` once symlinks and `..` are resolved. Errors name the
/// directory and never include file contents.
pub fn check_file_source(source: &str, dir: &Path) -> Result<PathBuf, String> {
    let dir_display = dir.display();
    if !Path::new(source).is_absolute() {
        return Err(format!(
            "file path must be absolute and inside the lists directory {dir_display}"
        ));
    }
    let base = std::fs::canonicalize(dir)
        .map_err(|e| format!("lists directory {dir_display} is not usable: {e}"))?;
    let file = std::fs::canonicalize(source).map_err(|e| {
        format!("{source} could not be read ({e}); file sources must be inside the lists directory {dir_display}")
    })?;
    if file.starts_with(&base) {
        Ok(file)
    } else {
        Err(format!(
            "file sources must be inside the lists directory {dir_display}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn loopback_unspecified_link_local_and_multicast_are_blocked() {
        // Cloud metadata sits on link-local; a source there would leak credentials.
        assert!(is_blocked_ip(ip("169.254.169.254")));
        assert!(is_blocked_ip(ip("127.0.0.1")));
        assert!(is_blocked_ip(ip("0.0.0.0")));
        assert!(is_blocked_ip(ip("::1")));
        assert!(is_blocked_ip(ip("::")));
        assert!(is_blocked_ip(ip("fe80::1")));
        assert!(is_blocked_ip(ip("ff02::1")));
        assert!(is_blocked_ip(ip("224.0.0.1")));
    }

    #[test]
    fn ipv4_mapped_ipv6_is_judged_as_ipv4() {
        // The same host answers both forms, so the mapped form must not slip past.
        assert!(is_blocked_ip(ip("::ffff:127.0.0.1")));
        assert!(is_blocked_ip(ip("::ffff:169.254.169.254")));
        assert!(!is_blocked_ip(ip("::ffff:10.0.0.5")));
    }

    #[test]
    fn private_lan_addresses_stay_allowed() {
        // Homelab users host lists on a LAN server; blocking these would break them.
        assert!(!is_blocked_ip(ip("10.0.0.5")));
        assert!(!is_blocked_ip(ip("192.168.1.10")));
        assert!(!is_blocked_ip(ip("172.16.4.2")));
        assert!(!is_blocked_ip(ip("fd00::5")));
        assert!(!is_blocked_ip(ip("2606:4700:4700::1111")));
    }

    #[tokio::test]
    async fn literal_blocked_hosts_are_refused_before_any_request() {
        // Literal IPs need no lookup, so these run offline and never connect.
        for source in [
            "http://169.254.169.254/latest/meta-data/",
            "http://127.0.0.1:8080/list",
            "http://[::1]/list.txt",
            "http://[::ffff:127.0.0.1]/list.txt",
            "http://0.0.0.0/list.txt",
        ] {
            assert!(
                check_url_source(source).await.is_err(),
                "{source} must be refused"
            );
        }
    }

    #[tokio::test]
    async fn localhost_is_refused_by_name() {
        // A name is refused because it resolves to loopback, not because of its spelling.
        assert!(check_url_source("http://localhost/list.txt").await.is_err());
    }

    #[tokio::test]
    async fn literal_lan_hosts_are_accepted() {
        assert!(check_url_source("http://10.0.0.5:8080/list").await.is_ok());
        assert!(check_url_source("http://192.168.1.10/hosts.txt")
            .await
            .is_ok());
    }

    #[test]
    fn blocked_ip_error_names_the_address_and_not_the_request() {
        let err = refuse_if_blocked(&[ip("127.0.0.1")]).unwrap_err();
        assert!(matches!(err, SourceError::Blocked(_)));
        assert!(err.to_string().contains("127.0.0.1"));
    }

    #[tokio::test]
    async fn unresolvable_host_is_unresolved_not_blocked() {
        // .invalid is reserved (RFC 6761) and never resolves. The answer is "cannot
        // tell", which lets a cached copy stand in; it must not read as "unsafe".
        let err = check_url_source("http://cache-test.invalid/list.txt")
            .await
            .unwrap_err();
        assert!(matches!(err, SourceError::Unresolved(_)));
    }

    #[tokio::test]
    async fn literal_blocked_address_is_blocked_not_unresolved() {
        // A literal needs no lookup, so it is always a judged refusal, never a DNS failure.
        let err = check_url_source("http://127.0.0.1:1/x").await.unwrap_err();
        assert!(matches!(err, SourceError::Blocked(_)));
    }

    fn scratch_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("sourceguard-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(dir.join("lists")).unwrap();
        dir
    }

    #[test]
    fn real_file_inside_the_lists_dir_is_accepted_and_readable() {
        let root = scratch_dir("accept");
        let lists = root.join("lists");
        std::fs::write(lists.join("ads.txt"), "ads.example\n").unwrap();
        let path = check_file_source(&lists.join("ads.txt").display().to_string(), &lists).unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "ads.example\n");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn dotdot_escape_out_of_the_lists_dir_is_refused() {
        // The path looks inside the directory, but ".." leads out of it.
        let root = scratch_dir("dotdot");
        let lists = root.join("lists");
        std::fs::write(root.join("secret.txt"), "private").unwrap();
        let source = format!("{}/../secret.txt", lists.display());
        assert!(check_file_source(&source, &lists).is_err());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn prefix_sibling_directory_is_not_inside_the_lists_dir() {
        // "lists-evil" starts with the same characters as "lists" but is a different folder.
        let root = scratch_dir("sibling");
        let lists = root.join("lists");
        let sibling = root.join("lists-evil");
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::write(sibling.join("x.txt"), "x").unwrap();
        let source = sibling.join("x.txt").display().to_string();
        assert!(check_file_source(&source, &lists).is_err());
        std::fs::remove_dir_all(root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_inside_the_lists_dir_pointing_outside_is_refused() {
        // A symlink is a path inside the directory, but it reads the target's bytes.
        let root = scratch_dir("symlink");
        let lists = root.join("lists");
        std::fs::write(root.join("secret.txt"), "private").unwrap();
        std::os::unix::fs::symlink(root.join("secret.txt"), lists.join("link.txt")).unwrap();
        let source = lists.join("link.txt").display().to_string();
        assert!(check_file_source(&source, &lists).is_err());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn relative_path_is_refused() {
        let root = scratch_dir("relative");
        let lists = root.join("lists");
        let err = check_file_source("lists/ads.txt", &lists).unwrap_err();
        assert!(err.contains("absolute"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn nonexistent_file_inside_the_lists_dir_gives_a_clear_error() {
        // The error must name the lists directory so the admin knows where files belong.
        let root = scratch_dir("missing");
        let lists = root.join("lists");
        let source = lists.join("missing.txt").display().to_string();
        let err = check_file_source(&source, &lists).unwrap_err();
        assert!(err.contains(&lists.display().to_string()));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn missing_lists_dir_is_reported_by_name() {
        let root = scratch_dir("nodir");
        let lists = root.join("absent");
        let err = check_file_source("/etc/hosts", &lists).unwrap_err();
        assert!(err.contains("lists directory"));
        std::fs::remove_dir_all(root).ok();
    }

    #[tokio::test]
    async fn redirect_to_a_blocked_literal_ip_stops_the_download() {
        // A local server answers with a redirect to metadata. The first hop is local
        // and never checked; the redirect target must stop the request.
        let (base, _hits, server) = spawn_redirect_server("http://169.254.169.254/latest/").await;
        let client = redirect_client();
        let err = client.get(&base).send().await.unwrap_err();
        assert!(err.is_redirect(), "redirect to metadata must fail: {err}");
        server.abort();
    }

    #[tokio::test]
    async fn redirect_loops_stop_after_five_hops() {
        // Each request the server sees is one hop. The original request plus five
        // followed redirects is six. reqwest's default of ten would send eleven.
        let (base, hits, server) = spawn_redirect_server("LOOP").await;
        let client = redirect_client();
        let err = client.get(&base).send().await.unwrap_err();
        assert!(err.is_redirect(), "endless redirects must fail: {err}");
        assert_eq!(hits.load(Ordering::SeqCst), MAX_REDIRECTS + 1);
        server.abort();
    }

    fn redirect_client() -> reqwest::Client {
        // Same provider setup as main and the API tests; the client panics without one.
        let _ = rustls::crypto::ring::default_provider().install_default();
        reqwest::Client::builder()
            .no_proxy()
            .redirect(redirect_policy())
            .build()
            .unwrap()
    }

    /// Serves 302 responses on 127.0.0.1 and counts the requests it receives. A
    /// target of "LOOP" points back at this server by name: a literal loopback
    /// redirect is refused, but a hostname is not checked by the redirect policy.
    async fn spawn_redirect_server(
        target: &str,
    ) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let location = if target == "LOOP" {
            format!("http://localhost:{}/", addr.port())
        } else {
            target.to_string()
        };
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let handle = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    return;
                };
                counter.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(answer_redirect(socket, location.clone()));
            }
        });
        (format!("http://{addr}/"), hits, handle)
    }

    async fn answer_redirect(mut socket: tokio::net::TcpStream, location: String) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut buf = [0u8; 4096];
        let _ = socket.read(&mut buf).await;
        let reply = format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let _ = socket.write_all(reply.as_bytes()).await;
    }
}
