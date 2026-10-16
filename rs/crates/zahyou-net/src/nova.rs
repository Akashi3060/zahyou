//! nova.astrometry.net のオンライン解析(Python は astroquery の `AstrometryNet` を使っていた)。
//!
//! 手順(astroquery 0.4.10 の astrometry_net/core.py と同じ要求を送る):
//! 1. `POST api/login`  本文 `request-json={"apikey": "..."}` → `session`
//! 2. 星のリストがあれば `POST api/url_upload`(x・y・image_width・image_height・session と手がかり)→ `subid`
//!    画像を送るときは `POST api/upload`(multipart: request-json と file)→ `subid`
//! 3. 1 秒ごとに `GET api/submissions/<subid>` → `jobs[0]`、`GET api/jobs/<job>/info` → `status`
//! 4. `success` なら `GET wcs_file/<job>`(FITS のヘッダー)
//!
//! API キーは呼ぶ側が引数で渡す。画面の「API キー」の欄の初めの値は [`DEFAULT_API_KEY`](作者のキー。v6 と同じ)で、
//! 環境変数 ASTROMETRY_API_KEY があればそちら([`initial_api_key`])。利用者は欄で自分のキーに替えられる。
//! キーの値はログ・エラーの文・自己診断に出さない: [`ApiKey`] の Debug は伏せ字にし、nova の応答やエラーの文に
//! キーが混じっていれば [`redact_key`] で伏せてから返す。
//! 星のリストは Python 版と同じく 1 始まり(FITS 規約)に直して送る。明るい順に並べて渡すこと。
//!
//! 中止: 要求はどれも別のスレッドで投げ、50 ms ごとに中止の合図を見ながら応答を待つ(中止なら待たずに戻る。
//! 投げた要求は裏で時間切れまで続くことがある = Python の _with_deadline と同じ)。画像を送っている途中なら、
//! 本文の読み出しを止めて送るのをやめる。中止のあとで新しい要求(特に画像の送信)は始めない。

use crate::http;
use crate::pyfmt::{py_json, py_urlencode};
use crate::NetError;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fmt;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

/// nova.astrometry.net(astroquery の conf.server と同じ)
pub const NOVA_URL: &str = "https://nova.astrometry.net";
/// API キーを入れる環境変数
pub const API_KEY_ENV: &str = "ASTROMETRY_API_KEY";

/// 既定の API キー(作者のもの)。v6 の gui/zahyou_gui.py の `_params` と dev/nb_settings.py の既定値と同じ。
///
/// 画面の「API キー」の欄の初めの値で、欄の「元に戻す」もこれに戻す(2026-10-16 にユーザーが決めた)。
/// 利用者は nova.astrometry.net に登録して得た自分のキーに替えられる。v6 のソースで公開済みの値だが、
/// ログ・画面の文・自己診断・試験の出力には値を出さない(出すのは長さと一致だけ。[`ApiKey::describe`])。
pub const DEFAULT_API_KEY: &str = "axvhxwfkvnzreobn";

/// 環境変数 ASTROMETRY_API_KEY の API キー(空なら None)。
pub fn api_key_from_env() -> Option<String> {
    std::env::var(API_KEY_ENV).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// 設定に API キーが無いとき(初めて起動したとき・v6 の gui.json)の初めの値:
/// 環境変数 ASTROMETRY_API_KEY があればそれ、無ければ [`DEFAULT_API_KEY`](v6 の `os.environ.get(…, 既定)` と同じ)。
pub fn initial_api_key() -> String {
    api_key_from_env().unwrap_or_else(|| DEFAULT_API_KEY.to_string())
}

/// API キーの値。Debug では値を伏せる(試験の失敗の表示や、設定・解析の条件を `{:?}` で書いたときに値が出ないように)。
/// 値を文字列に直す Display は持たない。値は [`ApiKey::expose`](nova へ送るときと入力欄)でだけ取り出す。
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn new(s: impl Into<String>) -> ApiKey {
        ApiKey(s.into())
    }

    /// 既定のキー([`DEFAULT_API_KEY`])。画面の「元に戻す」
    pub fn default_key() -> ApiKey {
        ApiKey(DEFAULT_API_KEY.to_string())
    }

    /// 設定に無いときの初めの値([`initial_api_key`])
    pub fn initial() -> ApiKey {
        ApiKey(initial_api_key())
    }

    /// 値そのもの(nova へ送るとき・画面の入力欄だけで使う。ログや文に入れない)
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// 画面の入力欄で書き換えるための中身
    pub fn edit(&mut self) -> &mut String {
        &mut self.0
    }

    /// 前後の空白を除いた値。空なら None(問い合わせない)
    pub fn usable(&self) -> Option<&str> {
        Some(self.0.trim()).filter(|s| !s.is_empty())
    }

    /// 既定のキー(作者のもの)と同じか(前後の空白は見ない)
    pub fn is_default(&self) -> bool {
        self.0.trim() == DEFAULT_API_KEY
    }

    /// 値を出さない説明(自己診断・画面の注記用): 「空」「既定のキー (16 文字)」「既定と違うキー (N 文字)」
    pub fn describe(&self) -> String {
        match self.usable() {
            None => "空".to_string(),
            Some(k) if k == DEFAULT_API_KEY => format!("既定のキー ({} 文字)", k.chars().count()),
            Some(k) => format!("既定と違うキー ({} 文字)", k.chars().count()),
        }
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ApiKey(<伏せ字 {} 文字>)", self.0.chars().count())
    }
}

impl From<&str> for ApiKey {
    fn from(s: &str) -> ApiKey {
        ApiKey(s.to_string())
    }
}

impl From<String> for ApiKey {
    fn from(s: String) -> ApiKey {
        ApiKey(s)
    }
}

/// 伏せたキーの代わりに書く文字
pub const REDACTED: &str = "(API キー)";

/// 文の中の API キーを伏せる(nova の応答やエラーの文にキーが混じっていても、ログに値を出さないため)。
/// 4 文字より短い値は伏せない(ふつうの文字まで消えてしまう。本物のキーは 16 文字)。
pub fn redact_key(text: &str, key: &str) -> String {
    let key = key.trim();
    if key.chars().count() < 4 || !text.contains(key) {
        return text.to_string();
    }
    text.replace(key, REDACTED)
}

/// オンライン解析の依頼(Python の `solve_online` の引数と同じ意味)。Debug では API キーを伏せる。
#[derive(Clone)]
pub struct OnlineRequest<'a> {
    pub api_key: &'a str,
    /// 星のリストで解けなかったときに送る画像(無ければ送らない)
    pub image_path: Option<&'a Path>,
    /// 検出した星の位置(0 始まりの画素座標、明るい順)
    pub sources: Option<&'a [(f64, f64)]>,
    pub width: usize,
    pub height: usize,
    /// 待つ時間 [秒](Python の ONLINE_TIMEOUT。既定 120)
    pub timeout_s: f64,
    /// 目標の方向 (RA, Dec) [度]
    pub hint_radec: Option<(f64, f64)>,
    /// 目標の方向の半径 [度](既定 5)
    pub hint_radius_deg: f64,
    /// 画素スケールの手がかり [″/px](0.75〜1.25 倍の範囲で探させる)
    pub scale_hint: Option<f64>,
}

impl fmt::Debug for OnlineRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OnlineRequest")
            .field("api_key", &ApiKey::new(self.api_key))
            .field("image_path", &self.image_path)
            .field("sources", &self.sources.map(|s| s.len()))
            .field("width", &self.width)
            .field("height", &self.height)
            .field("timeout_s", &self.timeout_s)
            .field("hint_radec", &self.hint_radec)
            .field("hint_radius_deg", &self.hint_radius_deg)
            .field("scale_hint", &self.scale_hint)
            .finish()
    }
}

/// 解けた結果。
#[derive(Clone, Debug, PartialEq)]
pub struct NovaSolution {
    /// `wcs_file` が返した FITS のヘッダー(80 文字のカードの並び)
    pub header: String,
    /// 80 文字ずつに分けたカード(END まで。END は含まない)
    pub cards: Vec<String>,
    pub submission_id: u64,
    pub job_id: u64,
    /// 星のリストで解けたか(偽なら画像を送って解けた)
    pub from_source_list: bool,
}

/// 1 回の投稿の結末。
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Solved { header: String, job_id: u64 },
    /// nova が「解けなかった」と答えた
    Failed,
    /// 時間内に終わらなかった
    TimedOut,
}

/// 手がかりの設定(Python の solve_online が astroquery に渡す settings。順も同じ)。
pub fn hint_settings(hint_radec: Option<(f64, f64)>, hint_radius_deg: f64, scale_hint: Option<f64>) -> Vec<(String, Value)> {
    let mut s = Vec::new();
    if let Some((ra, dec)) = hint_radec {
        s.push(("center_ra".into(), fnum(ra)));
        s.push(("center_dec".into(), fnum(dec)));
        s.push(("radius".into(), fnum(hint_radius_deg)));
    }
    // Python の `if scale_hint:`(0 は無しと同じ)
    if let Some(sc) = scale_hint.filter(|v| *v != 0.0) {
        s.push(("scale_units".into(), Value::String("arcsecperpix".into())));
        s.push(("scale_type".into(), Value::String("ul".into())));
        s.push(("scale_lower".into(), fnum(sc * 0.75)));
        s.push(("scale_upper".into(), fnum(sc * 1.25)));
    }
    s
}

/// astroquery の `_validate_settings` と同じ下限の検査(上限は astroquery の書き方では効いていないので見ない)。
// `!(x >= lo)` は NaN も弾くための書き方
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn validate_settings(settings: &[(String, Value)]) -> Result<(), String> {
    for (k, v) in settings {
        let lower = match k.as_str() {
            "center_ra" | "radius" | "scale_lower" | "scale_upper" | "scale_est" | "positional_error" => Some(0.0),
            "center_dec" => Some(-90.0),
            _ => None,
        };
        if let (Some(lo), Some(x)) = (lower, v.as_f64()) {
            // NaN もここで落ちる(Python の `value >= bounds[0]` が偽)
            if !(x >= lo) {
                return Err(format!("Value {} for {k} is invalid.", crate::pyfmt::py_float_repr(x)));
            }
        }
    }
    Ok(())
}

fn fnum(x: f64) -> Value {
    serde_json::Number::from_f64(x).map(Value::Number).unwrap_or(Value::Null)
}

fn to_map(settings: &[(String, Value)]) -> Map<String, Value> {
    let mut m = Map::new();
    for (k, v) in settings {
        m.insert(k.clone(), v.clone());
    }
    m
}

/// `request-json` の値(Python の `json.dumps(settings)`)。
pub fn request_json(settings: &[(String, Value)]) -> String {
    py_json(&Value::Object(to_map(settings)), None, true)
}

/// login の本文(`request-json={"apikey": ...}` を urlencode したもの)。
pub fn login_body(api_key: &str) -> String {
    let rj = request_json(&[("apikey".into(), Value::String(api_key.into()))]);
    py_urlencode(&[("request-json", &rj)])
}

/// url_upload(星のリスト)の本文。`xs`・`ys` は 1 始まりに直したもの。
pub fn source_list_body(settings: &[(String, Value)], xs: &[f64], ys: &[f64], width: usize, height: usize, session: &str) -> String {
    let mut s = settings.to_vec();
    s.push(("x".into(), Value::Array(xs.iter().map(|v| fnum(*v)).collect())));
    s.push(("y".into(), Value::Array(ys.iter().map(|v| fnum(*v)).collect())));
    s.push(("image_width".into(), Value::from(width as u64)));
    s.push(("image_height".into(), Value::from(height as u64)));
    s.push(("session".into(), Value::String(session.into())));
    py_urlencode(&[("request-json", &request_json(&s))])
}

/// upload(画像)の本文。戻り値 (Content-Type, 本文)。
pub fn upload_body(settings: &[(String, Value)], session: &str, file_name: &str, data: &[u8], boundary: &str) -> (String, Vec<u8>) {
    let mut s = settings.to_vec();
    s.push(("session".into(), Value::String(session.into())));
    let rj = request_json(&s);
    // ファイル名は urllib3 2.x と同じく HTML5 の書き方(" と改行だけを %XX にする)
    let fname: String = file_name
        .chars()
        .map(|c| match c {
            '"' => "%22".to_string(),
            '\r' => "%0D".to_string(),
            '\n' => "%0A".to_string(),
            c => c.to_string(),
        })
        .collect();
    let mut body = Vec::with_capacity(data.len() + rj.len() + 512);
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"request-json\"\r\n\r\n").as_bytes());
    body.extend_from_slice(rj.as_bytes());
    body.extend_from_slice(
        format!("\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{fname}\"\r\nContent-Type: application/octet-stream\r\n\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

/// FITS のヘッダーを 80 文字のカードに分ける(END の手前まで)。
pub fn split_cards(header: &str) -> Vec<String> {
    let chars: Vec<char> = header.chars().filter(|c| *c != '\n' && *c != '\r').collect();
    let mut out = Vec::new();
    for c in chars.chunks(80) {
        let card: String = c.iter().collect();
        if card.get(..8).map(|k| k.trim_end()) == Some("END") {
            break;
        }
        out.push(card);
    }
    out
}

/// nova.astrometry.net の API の窓口。
pub struct NovaClient {
    base: String,
    session: Option<String>,
    /// 状態を聞く間隔(astroquery と同じ 1 秒)
    pub poll: Duration,
    agent: ureq::Agent,
    upload_agent: ureq::Agent,
}

impl NovaClient {
    /// `base` は "https://nova.astrometry.net" の形(試験では localhost)。
    pub fn new(base: &str) -> NovaClient {
        NovaClient {
            base: base.trim_end_matches('/').to_string(),
            session: None,
            poll: Duration::from_secs(1),
            agent: http::agent(Duration::from_secs(60), false),
            upload_agent: http::agent(Duration::from_secs(600), false),
        }
    }

    fn post_form(&self, path: &str, body: &str, stop: &dyn Fn() -> bool) -> Result<Value, NetError> {
        let url = format!("{}/api/{path}", self.base);
        let agent = self.agent.clone();
        let body = body.to_string();
        let text = run_stoppable(stop, move |_| {
            let mut resp = agent
                .post(&url)
                .content_type("application/x-www-form-urlencoded")
                .send(body.as_bytes())
                .map_err(|e| http::describe(&e))?;
            let status = resp.status().as_u16();
            let text = http::read_text(&mut resp, 4 << 20)?;
            if status != 200 {
                return Err(format!("Post of job failed (HTTP {status})"));
            }
            Ok(text)
        })?;
        serde_json::from_str(&text).map_err(|e| NetError::Failed(format!("応答を読めません ({e})")))
    }

    fn get_json(&self, path: &str, stop: &dyn Fn() -> bool) -> Result<Value, NetError> {
        let url = format!("{}/api/{path}", self.base);
        let text = self.get_text(url, "", stop)?;
        serde_json::from_str(&text).map_err(|e| NetError::Failed(format!("応答を読めません ({e})")))
    }

    /// GET して本文を文字列で。HTTP 200 でなければ Failed(「{what}HTTP nnn」)。
    fn get_text(&self, url: String, what: &'static str, stop: &dyn Fn() -> bool) -> Result<String, NetError> {
        let agent = self.agent.clone();
        run_stoppable(stop, move |_| {
            let mut resp = agent.get(&url).call().map_err(|e| http::describe(&e))?;
            let status = resp.status().as_u16();
            let text = http::read_text(&mut resp, 4 << 20)?;
            if status != 200 {
                return Err(format!("{what}HTTP {status}"));
            }
            Ok(text)
        })
    }

    /// API キーでログインする(済んでいれば何もしない)。
    pub fn login(&mut self, api_key: &str, stop: &dyn Fn() -> bool) -> Result<(), NetError> {
        if self.session.is_some() {
            return Ok(());
        }
        if api_key.trim().is_empty() {
            return Err(NetError::Failed("Astrometry.net の API キーが空です(画面の「API キー」の欄に入れてください)".into()));
        }
        let v = self.post_form("login", &login_body(api_key.trim()), stop)?;
        if v.get("status").and_then(|s| s.as_str()) != Some("success") {
            return Err(NetError::Failed("Unable to log in to astrometry.net".into()));
        }
        let s = v
            .get("session")
            .and_then(|s| s.as_str())
            .ok_or_else(|| NetError::Failed("ログインの応答に session がありません".into()))?;
        self.session = Some(s.to_string());
        Ok(())
    }

    fn session(&self) -> Result<&str, NetError> {
        self.session.as_deref().ok_or_else(|| NetError::Failed("ログインしていません".into()))
    }

    /// 星のリストを送る。戻り値 subid。
    pub fn submit_source_list(
        &self,
        settings: &[(String, Value)],
        xs: &[f64],
        ys: &[f64],
        width: usize,
        height: usize,
        stop: &dyn Fn() -> bool,
    ) -> Result<u64, NetError> {
        validate_settings(settings).map_err(NetError::Failed)?;
        let body = source_list_body(settings, xs, ys, width, height, self.session()?);
        let v = self.post_form("url_upload", &body, stop)?;
        subid(&v).map_err(NetError::Failed)
    }

    /// 画像そのものを送る。戻り値 subid。中止の合図が立てば、送っている途中でも本文の読み出しを止めて Cancelled。
    pub fn submit_image(&self, settings: &[(String, Value)], path: &Path, stop: &dyn Fn() -> bool) -> Result<u64, NetError> {
        validate_settings(settings).map_err(NetError::Failed)?;
        if stop() {
            return Err(NetError::Cancelled);
        }
        let data = std::fs::read(path).map_err(|e| NetError::Failed(format!("{}: {e}", path.display())))?;
        let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
        let boundary = make_boundary(&data);
        let (ctype, body) = upload_body(settings, self.session()?, &name, &data, &boundary);
        drop(data);
        let url = format!("{}/api/upload", self.base);
        let agent = self.upload_agent.clone();
        let text = run_stoppable(stop, move |abandon| {
            // 長さを先に伝える(urllib3 と同じく Content-Length。reader の本文でも chunked にしない)
            let len = body.len();
            let mut reader = StopReader { data: &body, pos: 0, abandon: &abandon };
            let mut resp = agent
                .post(&url)
                .content_type(&ctype)
                .header("Content-Length", len.to_string())
                .send(ureq::SendBody::from_reader(&mut reader))
                .map_err(|e| http::describe(&e))?;
            let status = resp.status().as_u16();
            let text = http::read_text(&mut resp, 4 << 20)?;
            if status != 200 {
                return Err(format!("Post of job failed (HTTP {status})"));
            }
            Ok(text)
        })?;
        let v: Value = serde_json::from_str(&text).map_err(|e| NetError::Failed(format!("応答を読めません ({e})")))?;
        subid(&v).map_err(NetError::Failed)
    }

    /// 結末が出るまで待つ(astroquery の `monitor_submission`)。状態の問い合わせの失敗は締め切りまで続ける。
    pub fn monitor(&self, subid: u64, timeout: Duration, stop: &dyn Fn() -> bool, log: &dyn Fn(&str)) -> Result<Outcome, NetError> {
        let start = Instant::now();
        let mut job: Option<u64> = None;
        let mut warned = false;
        loop {
            if !http::sleep_with_stop(self.poll, stop) {
                return Err(NetError::Cancelled);
            }
            let mut status = String::new();
            if job.is_none() {
                match self.get_json(&format!("submissions/{subid}"), stop) {
                    Ok(v) => job = v.get("jobs").and_then(|j| j.get(0)).and_then(|j| j.as_u64()),
                    Err(NetError::Cancelled) => return Err(NetError::Cancelled),
                    Err(e) if !warned => {
                        log(&format!("  - nova の状態を聞けませんでした ({})。続けて待ちます。", net_msg(&e)));
                        warned = true;
                    }
                    Err(_) => {}
                }
                // 問い合わせの間に中止されていたら、次の問い合わせをしない
                if stop() {
                    return Err(NetError::Cancelled);
                }
            }
            if let Some(j) = job {
                match self.get_json(&format!("jobs/{j}/info"), stop) {
                    Ok(v) => status = v.get("status").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                    Err(NetError::Cancelled) => return Err(NetError::Cancelled),
                    Err(e) if !warned => {
                        log(&format!("  - nova の状態を聞けませんでした ({})。続けて待ちます。", net_msg(&e)));
                        warned = true;
                    }
                    Err(_) => {}
                }
            }
            // 答えを受け取る間に中止されていたら、その答えでは動かない(failure なら画像を送ることになる)
            if stop() {
                return Err(NetError::Cancelled);
            }
            match (status.as_str(), job) {
                ("success", Some(j)) => {
                    let header = self.wcs_file(j, stop)?;
                    return Ok(Outcome::Solved { header, job_id: j });
                }
                ("failure", _) => return Ok(Outcome::Failed),
                _ => {}
            }
            if start.elapsed() > timeout {
                return Ok(Outcome::TimedOut);
            }
        }
    }

    /// 解けた WCS(FITS のヘッダー)を受け取る。
    pub fn wcs_file(&self, job: u64, stop: &dyn Fn() -> bool) -> Result<String, NetError> {
        let url = format!("{}/wcs_file/{job}", self.base);
        self.get_text(url, "wcs_file: ", stop)
    }
}

/// NetError の理由の文(ログ用)。
fn net_msg(e: &NetError) -> String {
    match e {
        NetError::Failed(m) => m.clone(),
        NetError::Cancelled => "中止".into(),
    }
}

/// 要求 f を別のスレッドで走らせ、中止の合図を 50 ms ごとに見ながら終わるのを待つ。
///
/// 中止なら f の終わりを待たずに Err(Cancelled) を返し、f に渡した合図(abandon)を立てる(画像の本文の読み出しは
/// それを見て止まる)。f はそのまま裏で時間切れまで続くことがあるが、結果は捨てる。スレッドを作れなければ
/// その場で走らせる(そのときは中止を待てない)。
fn run_stoppable<T: Send + 'static>(
    stop: &dyn Fn() -> bool,
    f: impl FnOnce(Arc<AtomicBool>) -> Result<T, String> + Send + 'static,
) -> Result<T, NetError> {
    if stop() {
        return Err(NetError::Cancelled);
    }
    let abandon = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let slot = Arc::new(std::sync::Mutex::new(Some(f)));
    let (slot2, abandon2) = (slot.clone(), abandon.clone());
    let spawned = std::thread::Builder::new().name("nova-request".into()).spawn(move || {
        let f = slot2.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(f) = f {
            let _ = tx.send(f(abandon2));
        }
    });
    if spawned.is_err() {
        // スレッドを作れない(資源切れ)。中止は待てないが、その場で投げる
        let f = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        return match f {
            Some(f) => f(abandon).map_err(NetError::Failed),
            None => Err(NetError::Failed("要求を始められませんでした".into())),
        };
    }
    loop {
        if stop() {
            abandon.store(true, Ordering::Relaxed);
            return Err(NetError::Cancelled);
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(r) => return r.map_err(NetError::Failed),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            // f が panic した
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(NetError::Failed("要求の途中で問題が起きました".into())),
        }
    }
}

/// 送る本文の読み出し。abandon が立ったら(中止)誤りを返し、送るのをやめさせる。
struct StopReader<'a> {
    data: &'a [u8],
    pos: usize,
    abandon: &'a AtomicBool,
}

impl Read for StopReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.abandon.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("中止しました"));
        }
        // 64 KiB ずつ渡す(中止をこまめに見るため)
        let rest = &self.data[self.pos.min(self.data.len())..];
        let n = rest.len().min(buf.len()).min(64 << 10);
        buf[..n].copy_from_slice(&rest[..n]);
        self.pos += n;
        Ok(n)
    }
}

fn subid(v: &Value) -> Result<u64, String> {
    v.get("subid")
        .and_then(|s| s.as_u64())
        .ok_or_else(|| format!("投稿の応答に subid がありません ({})", v))
}

fn make_boundary(data: &[u8]) -> String {
    let mut h = Sha256::new();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    h.update(now.to_le_bytes());
    h.update(std::process::id().to_le_bytes());
    h.update(&data[..data.len().min(4096)]);
    let d = h.finalize();
    d[..16].iter().map(|b| format!("{b:02x}")).collect()
}

/// nova.astrometry.net に投げる(Python の `solve_online`)。
///
/// 星のリストがあるならそれを送る(画像より桁違いに軽く、オフラインと同じ検出結果で解ける)。
/// nova が「解けない」と答えたら画像そのものを送る。戻り値 Ok(Some) = 解けた、Ok(None) = 解けなかった。
/// 時間切れ・通信の失敗は Err(Failed)、中止は Err(Cancelled)。
pub fn solve_online(req: &OnlineRequest, stop: &dyn Fn() -> bool, log: &dyn Fn(&str)) -> Result<Option<NovaSolution>, NetError> {
    let mut client = NovaClient::new(NOVA_URL);
    solve_online_with(&mut client, req, stop, log)
}

/// [`solve_online`] の窓口を指定する版(試験用)。
///
/// ログの行と失敗の文に API キーが混じっていれば伏せる([`redact_key`]。nova の応答をそのまま文に入れる箇所があるため)。
pub fn solve_online_with(client: &mut NovaClient, req: &OnlineRequest, stop: &dyn Fn() -> bool, log: &dyn Fn(&str)) -> Result<Option<NovaSolution>, NetError> {
    let key = req.api_key;
    let log_safe = |m: &str| log(&redact_key(m, key));
    solve_online_inner(client, req, stop, &log_safe).map_err(|e| match e {
        NetError::Failed(m) => NetError::Failed(redact_key(&m, key)),
        e => e,
    })
}

fn solve_online_inner(client: &mut NovaClient, req: &OnlineRequest, stop: &dyn Fn() -> bool, log: &dyn Fn(&str)) -> Result<Option<NovaSolution>, NetError> {
    let fail = |e: String| NetError::Failed(e);
    let settings = hint_settings(req.hint_radec, req.hint_radius_deg, req.scale_hint);
    validate_settings(&settings).map_err(fail)?;
    // NaN・無限大・大きすぎる値でも落ちないように(1 秒〜1 日)
    let timeout = Duration::try_from_secs_f64(req.timeout_s.clamp(1.0, 86_400.0)).unwrap_or(Duration::from_secs(120));
    if stop() {
        return Err(NetError::Cancelled);
    }

    if let Some(src) = req.sources.filter(|s| !s.is_empty()) {
        // xylist は FITS 規約で 1 始まり
        let xs: Vec<f64> = src.iter().map(|p| p.0 + 1.0).collect();
        let ys: Vec<f64> = src.iter().map(|p| p.1 + 1.0).collect();
        log(&format!("  - 検出した {} 個の星の位置だけを送ります。", xs.len()));
        client.login(req.api_key, stop)?;
        let sub = client.submit_source_list(&settings, &xs, &ys, req.width, req.height, stop)?;
        match client.monitor(sub, timeout, stop, log)? {
            Outcome::Solved { header, job_id } => {
                let cards = split_cards(&header);
                return Ok(Some(NovaSolution { header, cards, submission_id: sub, job_id, from_source_list: true }));
            }
            Outcome::TimedOut => {
                return Err(NetError::Failed(format!("オンライン解析が {:.0} 秒で終わりませんでした。", req.timeout_s)));
            }
            Outcome::Failed => {}
        }
        if req.image_path.is_none() {
            return Ok(None);
        }
        // 「解けない」の答えと中止が重なったときに、画像を送り始めない
        if stop() {
            return Err(NetError::Cancelled);
        }
        log("  - 星の位置だけでは解けなかったので、画像そのものを送ります。");
    }

    let Some(path) = req.image_path else {
        return Ok(None);
    };
    if stop() {
        return Err(NetError::Cancelled);
    }
    client.login(req.api_key, stop)?;
    let sub = client.submit_image(&settings, path, stop)?;
    match client.monitor(sub, timeout, stop, log)? {
        Outcome::Solved { header, job_id } => {
            let cards = split_cards(&header);
            Ok(Some(NovaSolution { header, cards, submission_id: sub, job_id, from_source_list: false }))
        }
        Outcome::Failed => Ok(None),
        Outcome::TimedOut => Err(NetError::Failed(format!("オンライン解析が {:.0} 秒で終わりませんでした。", req.timeout_s))),
    }
}
