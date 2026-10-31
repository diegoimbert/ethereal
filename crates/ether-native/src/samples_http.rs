//! `HostServices::http_get` for the sample pack (docs/SAMPLES.md): one short-lived
//! thread per request (blocking reqwest, rustls), body read in chunks with progress and a
//! hard cap, answers handed to the controller through a channel it drains every tick.
//! Feature `net`; without it downloads are unsupported (the built-in kit only).

use std::io::Read as _;
use std::net::{SocketAddr, ToSocketAddrs as _};
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

/// A client for one hop. Redirects are followed by `fetch`, one hop at a time, so that each
/// target's name is resolved and checked first and then reached only at the addresses checked
/// (`pinned`: the name and its addresses; a later lookup can't answer differently).
fn client(pinned: Option<(&str, &[SocketAddr])>) -> Result<reqwest::blocking::Client, String> {
    let mut b = reqwest::blocking::Client::builder()
        .user_agent(concat!("ethereal/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        // Per wait (the answer's head, each body read): a server that stalls is
        // dropped; `fetch` also bounds the whole download.
        .timeout(READ_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none());
    if let Some((name, addrs)) = pinned {
        b = b.resolve_to_addrs(name, addrs);
    }
    b.build().map_err(|e| e.to_string())
}

/// Redirects followed per request.
const MAX_REDIRECTS: usize = 5;

/// May a download that started at `first` follow a redirect to `to`? To https on a host that
/// isn't a local or private address (a pack's server must not make the engine's machine
/// reach into its own network); plain http only from a loopback dev server to loopback. A
/// redirect not followed ends the download with its 3xx status.
fn follow(to: &reqwest::Url, first: &reqwest::Url) -> bool {
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

/// `None`: no host; `Err(name)`: a name (lowercase); `Ok(ip)`: an address.
fn host(u: &reqwest::Url) -> Option<Result<std::net::IpAddr, String>> {
    let h = u.host_str()?;
    let bare = h.trim_start_matches('[').trim_end_matches(']');
    Some(bare.parse().map_err(|_| h.to_ascii_lowercase()))
}

fn loopback(u: &reqwest::Url) -> bool {
    match host(u) {
        Some(Ok(ip)) => ip.is_loopback(),
        Some(Err(name)) => name == "localhost",
        None => false,
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

/// The addresses a followed redirect to `to` may connect to: `None` when the redirect isn't
/// followed (`follow`), or when its host is a name and any address it resolves to (`lookup`)
/// is local or private (unless the download started on a loopback dev server). `Some(None)`:
/// followed, nothing to pin (an address, or a dev download).
fn hop(
    to: &reqwest::Url,
    first: &reqwest::Url,
    lookup: impl Fn(&str, u16) -> std::io::Result<Vec<SocketAddr>>,
) -> Option<Option<(String, Vec<SocketAddr>)>> {
    if !follow(to, first) {
        return None;
    }
    let name = match host(to)? {
        Ok(_) => return Some(None),
        Err(name) => name,
    };
    if loopback(first) {
        return Some(None);
    }
    let addrs = lookup(&name, to.port_or_known_default()?).ok()?;
    let public = |a: &SocketAddr| {
        let ip = a.ip();
        !(ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() || private(ip))
    };
    (!addrs.is_empty() && addrs.iter().all(public)).then_some(Some((name, addrs)))
}

fn system_lookup(name: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
    Ok((name, port).to_socket_addrs()?.collect())
}

/// Sends the request for `url` and follows redirects (`hop`), at most `MAX_REDIRECTS`. A
/// redirect not followed ends the download with its 3xx answer.
fn send(url: &str) -> Result<reqwest::blocking::Response, String> {
    let first = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
    let mut resp = client(None)?
        .get(first.clone())
        .send()
        .map_err(|e| e.to_string())?;
    for _ in 0..MAX_REDIRECTS {
        if !resp.status().is_redirection() {
            return Ok(resp);
        }
        let Some(to) = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|l| l.to_str().ok())
            .and_then(|l| resp.url().join(l).ok())
        else {
            return Ok(resp);
        };
        let Some(pinned) = hop(&to, &first, system_lookup) else {
            return Ok(resp);
        };
        let pinned = pinned.as_ref().map(|(n, a)| (n.as_str(), a.as_slice()));
        resp = client(pinned)?.get(to).send().map_err(|e| e.to_string())?;
    }
    if resp.status().is_redirection() {
        return Err("too many redirects".into());
    }
    Ok(resp)
}

fn fetch(id: u64, url: &str, max: u64, tx: &Sender<HttpEvent>) -> Result<(u16, Vec<u8>), String> {
    let started = std::time::Instant::now();
    let mut resp = send(url)?;
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
        serve_head(String::new(), "200 OK", body)
    }

    /// One local HTTP/1.1 server answering once with `status`, extra `headers` and `body`.
    fn serve_head(headers: String, status: &'static str, body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut req = [0u8; 1024];
            let _ = s.read(&mut req);
            let head = format!(
                "HTTP/1.1 {status}\r\n{headers}content-length: {}\r\nconnection: close\r\n\r\n",
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

    #[test]
    fn redirect_names_must_resolve_public() {
        use std::net::{IpAddr, Ipv4Addr};
        let u = |s: &str| reqwest::Url::parse(s).unwrap();
        let cdn = u("https://packs.example.com/manifest.json");
        let to = u("https://cdn.example.net/a.wav");
        let at = |ips: &'static [[u8; 4]]| {
            move |name: &str, port: u16| {
                assert_eq!((name, port), ("cdn.example.net", 443));
                Ok(ips
                    .iter()
                    .map(|&o| SocketAddr::new(IpAddr::V4(Ipv4Addr::from(o)), port))
                    .collect())
            }
        };
        let public = hop(&to, &cdn, at(&[[93, 184, 216, 34]])).unwrap().unwrap();
        assert_eq!(public.0, "cdn.example.net");
        assert_eq!(
            public.1[0].ip(),
            IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))
        );
        for bad in [
            &[[10, 0, 0, 1]][..],
            &[[93, 184, 216, 34], [127, 0, 0, 1]],
            &[[169, 254, 169, 254]],
            &[],
        ] {
            let bad: &'static [[u8; 4]] = Box::leak(bad.to_vec().into_boxed_slice());
            assert!(hop(&to, &cdn, at(bad)).is_none(), "{bad:?}");
        }
        let fails = |_: &str, _: u16| Err(std::io::Error::other("no such name"));
        assert!(hop(&to, &cdn, fails).is_none());
        // Addresses are checked by `follow`; a dev download isn't resolved.
        let unused = |_: &str, _: u16| -> std::io::Result<Vec<SocketAddr>> { panic!("resolved") };
        assert_eq!(hop(&u("https://93.184.216.34/a"), &cdn, unused), Some(None));
        let dev = u("http://127.0.0.1:8080/manifest.json");
        assert_eq!(hop(&u("http://localhost:8080/b"), &dev, unused), Some(None));
        assert_eq!(hop(&u("https://10.0.0.1/x"), &cdn, unused), None);
    }

    #[test]
    fn follows_and_stops_redirects() {
        let http = Http::default();
        let body = vec![3u8; 1000];
        let target = serve(body.clone()).replace("127.0.0.1", "localhost");
        let first = serve_head(format!("location: {target}\r\n"), "302 Found", Vec::new());
        http.get(1, &first, 1 << 20).unwrap();
        assert_eq!(wait(&http, 1).unwrap(), (200, body));
        // Not followed (plain http off loopback): the 3xx answer itself.
        let first = serve_head(
            "location: http://example.com/x\r\n".into(),
            "302 Found",
            b"moved".to_vec(),
        );
        http.get(2, &first, 1 << 20).unwrap();
        assert_eq!(wait(&http, 2).unwrap(), (302, b"moved".to_vec()));
    }
}
