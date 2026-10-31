//! `HostServices::http_get` for the sample pack (docs/SAMPLES.md): one short-lived
//! thread per request (blocking reqwest, rustls), body read in chunks with progress and a
//! hard cap, answers handed to the controller through a channel it drains every tick.
//! Feature `net`; without it downloads are unsupported (the built-in kit only).

use std::io::Read as _;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};
use ether_controller::HttpEvent;

const CHUNK: usize = 64 * 1024;
/// Longest wait for the server (answer head, each body read).
const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// Longest download (a 64 MiB pack file at ~40 KB/s).
const MAX_DOWNLOAD: Duration = Duration::from_secs(30 * 60);

#[derive(Clone)]
pub(crate) struct Http {
    tx: Sender<HttpEvent>,
    rx: Receiver<HttpEvent>,
}

impl std::fmt::Debug for Http {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Http")
    }
}

impl Default for Http {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        Self { tx, rx }
    }
}

fn client() -> Result<Arc<reqwest::blocking::Client>, String> {
    static CLIENT: OnceLock<Result<Arc<reqwest::blocking::Client>, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::blocking::Client::builder()
                .user_agent(concat!("ethereal/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(Duration::from_secs(15))
                // Per wait (the answer's head, each body read): a server that stalls is
                // dropped; `fetch` also bounds the whole download.
                .timeout(READ_TIMEOUT)
                .redirect(reqwest::redirect::Policy::custom(|a| {
                    if a.previous().len() >= MAX_REDIRECTS {
                        return a.error("too many redirects");
                    }
                    let allowed = a.previous().first().is_some_and(|f| follow(a.url(), f));
                    if allowed { a.follow() } else { a.stop() }
                }))
                .build()
                .map(Arc::new)
                .map_err(|e| e.to_string())
        })
        .clone()
}

/// Redirects followed per request.
const MAX_REDIRECTS: usize = 5;

/// May a download that started at `first` follow a redirect to `to`? To https on a host that
/// isn't a local or private address (a pack's server must not make the engine's machine
/// reach into its own network); plain http only from a loopback dev server to loopback. A
/// redirect not followed ends the download with its 3xx status.
fn follow(to: &reqwest::Url, first: &reqwest::Url) -> bool {
    use std::net::IpAddr;
    // `None`: no host; `Err(name)`: a name; `Ok(ip)`: an address.
    let host = |u: &reqwest::Url| -> Option<Result<IpAddr, String>> {
        let h = u.host_str()?;
        let bare = h.trim_start_matches('[').trim_end_matches(']');
        Some(bare.parse::<IpAddr>().map_err(|_| h.to_ascii_lowercase()))
    };
    let loopback = |u: &reqwest::Url| match host(u) {
        Some(Ok(ip)) => ip.is_loopback(),
        Some(Err(name)) => name == "localhost",
        None => false,
    };
    let local = |u: &reqwest::Url| match host(u) {
        Some(Ok(ip)) => ip.is_loopback() || ip.is_unspecified() || private(ip),
        Some(Err(name)) => name == "localhost",
        None => true,
    };
    match to.scheme() {
        "https" => !local(to) || loopback(first),
        "http" => first.scheme() == "http" && loopback(first) && loopback(to),
        _ => false,
    }
}

/// Private, link-local or shared address space (RFC 1918, 6598, 3927, 4193, 4291).
fn private(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_private() || v4.is_link_local() || (a == 100 && (64..128).contains(&b))
        }
        std::net::IpAddr::V6(v6) => {
            let seg = v6.segments()[0];
            (seg & 0xfe00) == 0xfc00
                || (seg & 0xffc0) == 0xfe80
                || v6.to_ipv4_mapped().is_some_and(|v4| {
                    let ip = std::net::IpAddr::V4(v4);
                    ip.is_loopback() || private(ip)
                })
        }
    }
}

fn fetch(id: u64, url: &str, max: u64, tx: &Sender<HttpEvent>) -> Result<(u16, Vec<u8>), String> {
    let started = std::time::Instant::now();
    let mut resp = client()?.get(url).send().map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let total = resp.content_length();
    if total.is_some_and(|t| t > max) {
        return Err("the download is bigger than expected".into());
    }
    let mut body = Vec::with_capacity(total.unwrap_or(0).min(max) as usize);
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = resp.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if (body.len() + n) as u64 > max {
            return Err("the download is bigger than expected".into());
        }
        if started.elapsed() > MAX_DOWNLOAD {
            return Err("the download took too long".into());
        }
        body.extend_from_slice(&buf[..n]);
        let _ = tx.send(HttpEvent::Progress {
            id,
            received: body.len() as u64,
            total,
        });
    }
    Ok((status, body))
}

impl Http {
    pub(crate) fn get(&self, id: u64, url: &str, max: u64) -> Result<(), String> {
        let tx = self.tx.clone();
        let url = url.to_string();
        std::thread::Builder::new()
            .name("ether-samples-http".into())
            .spawn(move || {
                let result = fetch(id, &url, max, &tx);
                let _ = tx.send(HttpEvent::Done { id, result });
            })
            .map(drop)
            .map_err(|e| e.to_string())
    }

    pub(crate) fn poll(&self, out: &mut Vec<HttpEvent>) {
        out.extend(self.rx.try_iter());
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::net::TcpListener;

    use super::*;

    /// One local HTTP/1.1 server answering `body` once.
    fn serve(body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut req = [0u8; 1024];
            let _ = s.read(&mut req);
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            s.write_all(head.as_bytes()).unwrap();
            s.write_all(&body).unwrap();
        });
        format!("http://{addr}/manifest.json")
    }

    fn wait(http: &Http, id: u64) -> Result<(u16, Vec<u8>), String> {
        for _ in 0..500 {
            let mut out = Vec::new();
            http.poll(&mut out);
            for e in out {
                if let HttpEvent::Done { id: got, result } = e {
                    assert_eq!(got, id);
                    return result;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no answer");
    }

    #[test]
    fn redirects_stay_on_public_https() {
        let u = |s: &str| reqwest::Url::parse(s).unwrap();
        let cdn = u("https://packs.example.com/manifest.json");
        assert!(follow(&u("https://cdn.example.net/a.wav"), &cdn));
        for to in [
            "http://cdn.example.net/a.wav",
            "https://10.0.0.1/x",
            "https://192.168.1.1/x",
            "https://169.254.169.254/latest",
            "https://127.0.0.1/x",
            "https://localhost/x",
            "https://[::1]/x",
            "https://[fd00::1]/x",
            "https://[::ffff:10.0.0.1]/x",
            "https://100.64.0.1/x",
            "file:///etc/passwd",
        ] {
            assert!(!follow(&u(to), &cdn), "{to}");
        }
        // A local dev server may redirect locally.
        let dev = u("http://127.0.0.1:8080/manifest.json");
        assert!(follow(&u("http://localhost:8080/b"), &dev));
        assert!(follow(&u("https://127.0.0.1:8443/b"), &dev));
        assert!(!follow(&u("http://example.com/b"), &dev));
    }

    #[test]
    fn downloads_and_caps() {
        let http = Http::default();
        let body = vec![7u8; 200_000];
        http.get(1, &serve(body.clone()), 1 << 20).unwrap();
        assert_eq!(wait(&http, 1).unwrap(), (200, body));
        http.get(2, &serve(vec![1u8; 5000]), 100).unwrap();
        assert!(wait(&http, 2).is_err());
    }
}
