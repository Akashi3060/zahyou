//! 試験の道具: localhost の小さな HTTP サーバー(Range と故障の注入つき)、python -m http.server、
//! 照合用の JSON、一時フォルダー、偽の索引ファイル。
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 受けた要求。
#[derive(Clone, Debug)]
pub struct Req {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Req {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// 返す応答。`cut_after` を入れると本文をそのバイト数だけ送って接続を切る。
/// `claim_len` を入れると Content-Length をその値にする(HEAD で大きなファイルのふりをする)。
#[derive(Clone, Debug)]
pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub cut_after: Option<usize>,
    pub claim_len: Option<u64>,
}

impl Resp {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Resp {
        Resp { status, headers: Vec::new(), body: body.into(), cut_after: None, claim_len: None }
    }
    pub fn json(v: &str) -> Resp {
        let mut r = Resp::new(200, v.as_bytes().to_vec());
        r.headers.push(("Content-Type".into(), "application/json".into()));
        r
    }
    pub fn cut(mut self, n: usize) -> Resp {
        self.cut_after = Some(n);
        self
    }
}

/// ファイルの中身を Range つきで返す(`range` が偽なら Range を無視して 200)。
pub fn serve_bytes(req: &Req, data: &[u8], range: bool) -> Resp {
    if range && let Some(r) = req.header("range").and_then(|v| v.strip_prefix("bytes=")) {
        let (a, b) = r.split_once('-').unwrap_or((r, ""));
        let a: usize = a.parse().unwrap_or(0);
        let last = data.len().saturating_sub(1);
        let b: usize = if b.is_empty() { last } else { b.parse().unwrap_or(last).min(last) };
        if data.is_empty() || a >= data.len() || a > b {
            let mut r = Resp::new(416, Vec::new());
            r.headers.push(("Content-Range".into(), format!("bytes */{}", data.len())));
            return r;
        }
        let mut r = Resp::new(206, data[a..=b].to_vec());
        r.headers.push(("Content-Range".into(), format!("bytes {a}-{b}/{}", data.len())));
        r.headers.push(("Accept-Ranges".into(), "bytes".into()));
        return r;
    }
    Resp::new(200, data.to_vec())
}

type Handler = dyn Fn(&Req) -> Resp + Send + Sync;

/// localhost の HTTP/1.1 サーバー(1 要求ごとに接続を閉じる)。
pub struct TestServer {
    pub port: u16,
    pub log: Arc<Mutex<Vec<Req>>>,
    stop: Arc<AtomicBool>,
}

impl TestServer {
    pub fn start(handler: impl Fn(&Req) -> Resp + Send + Sync + 'static) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let handler: Arc<Handler> = Arc::new(handler);
        {
            let (log, stop) = (log.clone(), stop.clone());
            std::thread::spawn(move || {
                for conn in listener.incoming() {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(conn) = conn else { continue };
                    let (h, log) = (handler.clone(), log.clone());
                    std::thread::spawn(move || {
                        let _ = serve_conn(conn, &*h, &log);
                    });
                }
            });
        }
        TestServer { port, log, stop }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn requests(&self) -> Vec<Req> {
        self.log.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.log.lock().unwrap().clear();
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn serve_conn(conn: TcpStream, h: &Handler, log: &Mutex<Vec<Req>>) -> std::io::Result<()> {
    conn.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(conn.try_clone()?);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(());
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    loop {
        let mut l = String::new();
        if reader.read_line(&mut l)? == 0 {
            break;
        }
        let l = l.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let len: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    let req = Req { method, path, headers, body };
    log.lock().unwrap().push(req.clone());
    let resp = h(&req);
    let mut out = conn;
    let reason = match resp.status {
        200 => "OK",
        206 => "Partial Content",
        404 => "Not Found",
        416 => "Range Not Satisfiable",
        _ => "X",
    };
    let len = resp.claim_len.unwrap_or(resp.body.len() as u64);
    let mut head = format!("HTTP/1.1 {} {reason}\r\nContent-Length: {len}\r\nConnection: close\r\n", resp.status);
    for (k, v) in &resp.headers {
        head += &format!("{k}: {v}\r\n");
    }
    head += "\r\n";
    out.write_all(head.as_bytes())?;
    if req.method != "HEAD" {
        let n = resp.cut_after.unwrap_or(resp.body.len()).min(resp.body.len());
        out.write_all(&resp.body[..n])?;
    }
    out.flush()?;
    let _ = out.shutdown(std::net::Shutdown::Both);
    Ok(())
}

/// 回数を数える小道具。
#[derive(Clone, Default)]
pub struct Counter(pub Arc<AtomicUsize>);

impl Counter {
    pub fn bump(&self) -> usize {
        self.0.fetch_add(1, Ordering::SeqCst) + 1
    }
    pub fn get(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

// ------------------------------------------------------------- 一時フォルダー ---

/// 試験ごとの空のフォルダー(%TMP%\zahyou-net-tests\<名前>-<pid>)。
pub fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("zahyou-net-tests").join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

// ------------------------------------------------------------- 照合用の値 ---

pub fn testdata_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("testdata").join(name)
}

/// testdata/net_python.json。無ければ None(試験は SKIP)。
pub fn python_values() -> Option<serde_json::Value> {
    let p = testdata_path("net_python.json");
    match std::fs::read(&p) {
        Ok(b) => Some(serde_json::from_slice(&b).expect("net_python.json を読めない")),
        Err(_) => {
            eprintln!("SKIP: {} がありません(testdata/gen_net.py で作る)", p.display());
            None
        }
    }
}

// ------------------------------------------------------------- 偽の索引ファイル ---

fn card(key: &str, value: &str) -> String {
    let s = if value.is_empty() { format!("{key:<8}") } else { format!("{key:<8}= {value:>20}") };
    format!("{s:<80}")
}

fn block(cards: &[String]) -> Vec<u8> {
    let mut s: String = cards.concat();
    s += &format!("{:<80}", "END");
    while !s.len().is_multiple_of(2880) {
        s.push(' ');
    }
    s.into_bytes()
}

/// 形だけ正しい索引ファイル(ヘッダー 2 つと、`data` バイトの表)。
pub fn fake_index(indexid: u32, healpix: i32, data: usize) -> Vec<u8> {
    let prim = block(&[
        card("SIMPLE", "T"),
        card("BITPIX", "8"),
        card("NAXIS", "0"),
        card("EXTEND", "T"),
        card("ENDIAN", "'04:03:02:01'"),
        card("INDEXID", &indexid.to_string()),
        card("HEALPIX", &healpix.to_string()),
        card("HPNSIDE", "2"),
    ]);
    let ext = block(&[
        card("XTENSION", "'BINTABLE'"),
        card("BITPIX", "8"),
        card("NAXIS", "2"),
        card("NAXIS1", "1"),
        card("NAXIS2", &data.to_string()),
        card("PCOUNT", "0"),
        card("GCOUNT", "1"),
        card("TFIELDS", "1"),
    ]);
    let mut out = prim;
    out.extend(ext);
    // 中身はでたらめでよい(HEALPIX = -1 もあるので桁あふれしない足し方で)
    let mut body: Vec<u8> = (0..data).map(|i| (i as u32).wrapping_mul(7).wrapping_add(indexid).wrapping_add(healpix as u32) as u8).collect();
    while !body.len().is_multiple_of(2880) {
        body.push(0);
    }
    out.extend(body);
    out
}

// ------------------------------------------------------------- python -m http.server ---

pub fn python_exe() -> PathBuf {
    if let Some(p) = std::env::var_os("ZAHYOU_PYTHON") {
        return PathBuf::from(p);
    }
    let p = PathBuf::from(r"C:\Users\yoshi\AppData\Local\Programs\Python\Python313\python.exe");
    if p.exists() { p } else { PathBuf::from("python") }
}

/// `python -m http.server` を localhost に立てる。Python が無ければ None(試験は SKIP)。
pub struct PyServer {
    child: Child,
    pub port: u16,
}

impl PyServer {
    pub fn start(dir: &Path) -> Option<PyServer> {
        let mut child = Command::new(python_exe())
            .args(["-u", "-m", "http.server", "0", "--bind", "127.0.0.1", "--directory"])
            .arg(dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut out = BufReader::new(child.stdout.take()?);
        let mut line = String::new();
        out.read_line(&mut line).ok()?;
        // "Serving HTTP on 127.0.0.1 port 54321 (http://127.0.0.1:54321/) ..."
        let port = line.split("port ").nth(1)?.split_whitespace().next()?.parse().ok()?;
        // 残りの出力は読み捨てる(パイプが詰まらないように)
        std::thread::spawn(move || {
            let mut sink = String::new();
            while out.read_line(&mut sink).map(|n| n > 0).unwrap_or(false) {
                sink.clear();
            }
        });
        Some(PyServer { child, port })
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for PyServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
