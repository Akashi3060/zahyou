//! 赤経赤緯の格子と目盛り。astropy の WCSAxes(7.1)の手順を移したもの:
//!
//! 1. 座標の範囲(`find_coordinate_range`): 表示範囲を 51×51 点で天へ写し、赤経の 360° の折り返しをならして
//!    最小・最大を取り、1 割広げる。極が入ると赤経は 0〜360° 全部になる。
//! 2. 目盛りの間隔(`AngleFormatterLocator.locator`): 範囲 / 5 に近い「きりのよい値」。
//!    赤経は時分秒(1・2・3・4・5・6・10・12・15・20・30 と 1〜24 時)、赤緯は度分秒(1・2・3・5・10・15・20・30 と 1〜360°)。
//! 3. 格子の線(`_update_grid_lines`): 目盛りの値ごとに、もう一方の座標を範囲いっぱいに 1000 点とって画素へ写す。
//!    往復して戻らない点・飛び(前の歩みの 10 倍)で線を切る(`get_lon_lat_path`)。最後に枠で切り取る。
//! 4. 目盛り(`_update_ticks`): 枠の 4 辺を 1000 点ずつ天へ写し、目盛りの値をまたぐ所を線形補間で求める。
//!    向きは座標の等値線の向き(外向きに 3.5 pt)。
//! 5. 文字(`formatter`・`simplify_labels`): 間隔に合わせて項目数を決め(1° 以上なら度だけ、1′ 以上なら度分…)、
//!    同じ辺で前の文字と同じ頭の部分を落とす(`4ʰ07ᵐ30ˢ` の次は `45ˢ`)。
//! 6. どの辺に文字を出すか(`auto_assign_coord_positions`): 辺の順 b・l・t・r から 2 つを選ぶ組み合わせのうち、
//!    文字の数がいちばん多いもの(空の辺が前にあると減点)。
//!
//! 画面の向きは「x が右、y が上」。flip_y(上下反転の表示)なら画像の y が画面の下向きになる。

use zahyou_wcs::Wcs;

use crate::sexa::{ARCMIN_TO_DEG, ARCSEC_TO_DEG, DEG_TO_HOURANGLE, HOURANGLE_TO_DEG, sexagesimal_string};

/// 範囲を調べる格子の分割数(astropy の conf.coordinate_range_samples)
const RANGE_SAMPLES: usize = 50;
/// 枠の 1 辺の標本数(conf.frame_boundary_samples)
const FRAME_SAMPLES: usize = 1000;
/// 格子の線 1 本の標本数(conf.grid_samples)
const GRID_SAMPLES: usize = 1000;
/// 目盛りの数の目安(AngleFormatterLocator の number)
const TICK_NUMBER: f64 = 5.0;
/// 往復のずれの許し(標本の間隔に対する比。grid_paths.ROUND_TRIP_RTOL)
const ROUND_TRIP_RTOL: f64 = 1.0;
/// 飛びとみなす歩みの比(grid_paths.DISCONT_FACTOR)
const DISCONT_FACTOR: f64 = 10.0;
/// 目盛りが多すぎるとき(壊れた WCS)の打ち切り
const MAX_TICKS: i64 = 1000;

/// 座標軸の名前(WCSAxes の軸名に当たる)。0 = 赤経、1 = 赤緯。
pub const AXIS_LABELS: [&str; 2] = ["赤経 (RA)", "赤緯 (Dec)"];

/// 図の枠の辺(画面で見た向き)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Bottom,
    Right,
    Top,
    Left,
}

impl Side {
    /// WCSAxes の辺の名前(b・r・t・l)。
    pub fn letter(self) -> char {
        match self {
            Side::Bottom => 'b',
            Side::Right => 'r',
            Side::Top => 't',
            Side::Left => 'l',
        }
    }

    /// 枠の辺の並び(RectangularFrame.spine_names = "brtl")。
    pub const ALL: [Side; 4] = [Side::Bottom, Side::Right, Side::Top, Side::Left];
}

/// 目盛り 1 本(枠の上の印)。
#[derive(Clone, Debug, PartialEq)]
pub struct Tick {
    /// 0 = 赤経、1 = 赤緯
    pub coord: usize,
    pub side: Side,
    /// 枠の上の位置(画像の座標、画素)
    pub pos: [f64; 2],
    /// 等値線が枠の内側へ向かう向き(画面の座標で x 右・y 上、度)。目盛りはこの逆向き(外向き)に描く
    pub angle_deg: f64,
    /// 座標の値(度。赤経は [0, 360))
    pub world: f64,
}

/// 目盛りの文字。
#[derive(Clone, Debug, PartialEq)]
pub struct TickLabel {
    pub coord: usize,
    pub side: Side,
    /// 目盛りの位置(画像の座標)。文字はここから辺の外側へ離して置く
    pub pos: [f64; 2],
    /// 文字。赤経の時分秒の印は上付きの文字 `ʰ` `ᵐ` `ˢ` で持つ(描くときは [`TickLabel::spans`] で小さく上に)
    pub text: String,
    /// 座標の値(度)
    pub world: f64,
}

/// 文字の一部(上付きにするか)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSpan {
    pub text: String,
    pub superscript: bool,
}

impl TickLabel {
    /// 上付きの印 `ʰ` `ᵐ` `ˢ` を、上付きの `h` `m` `s` に分けた並び(WCSAxes の `$\mathregular{^h}$` に当たる)。
    pub fn spans(&self) -> Vec<TextSpan> {
        split_superscripts(&self.text)
    }

    /// 上付きを普通の文字にした文(例 `4h07m30s`)。
    pub fn plain(&self) -> String {
        self.text.chars().map(|c| sup_to_plain(c).unwrap_or(c)).collect()
    }
}

fn sup_to_plain(c: char) -> Option<char> {
    match c {
        'ʰ' => Some('h'),
        'ᵐ' => Some('m'),
        'ˢ' => Some('s'),
        _ => None,
    }
}

/// 上付きの印を分ける。
pub fn split_superscripts(text: &str) -> Vec<TextSpan> {
    let mut out: Vec<TextSpan> = Vec::new();
    for c in text.chars() {
        let (ch, sup) = match sup_to_plain(c) {
            Some(p) => (p, true),
            None => (c, false),
        };
        match out.last_mut() {
            Some(last) if last.superscript == sup => last.text.push(ch),
            _ => out.push(TextSpan { text: ch.to_string(), superscript: sup }),
        }
    }
    out
}

/// 軸名(「赤経 (RA)」など)と、出す辺。
#[derive(Clone, Debug, PartialEq)]
pub struct AxisLabel {
    pub coord: usize,
    pub side: Side,
    pub text: String,
}

/// 格子の線の 1 区間(枠で切り取った折れ線)。
#[derive(Clone, Debug, PartialEq)]
pub struct GridLine {
    pub coord: usize,
    /// この線の座標の値(度。赤経は折り返す前の値で、負や 360 以上もありうる)
    pub world: f64,
    /// 画像の座標(画素)
    pub points: Vec<[f64; 2]>,
}

/// 格子と目盛りの全部。
#[derive(Clone, Debug, Default)]
pub struct Grid {
    pub lines: Vec<GridLine>,
    pub ticks: Vec<Tick>,
    pub labels: Vec<TickLabel>,
    pub axis_labels: Vec<AxisLabel>,
    /// 座標の範囲(度)。[赤経, 赤緯]
    pub ranges: [(f64, f64); 2],
    /// 目盛りの値(度、折り返す前)
    pub tick_values: [Vec<f64>; 2],
    /// 目盛りの間隔(度)
    pub spacing: [f64; 2],
}

/// 表示の範囲と画面の向き。`x` は (左端, 右端)、`y` は (下端, 上端)(画像の座標)。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub x: (f64, f64),
    pub y: (f64, f64),
    /// 画面 1 単位あたりの画素(目盛りの向きを差分で求める幅に使う)
    k: f64,
}

impl View {
    /// 画像全体を表示する範囲(matplotlib の imshow と同じく −0.5〜n−0.5)。flip_y なら上下を反転。
    pub fn image(width: usize, height: usize, flip_y: bool) -> View {
        let (w, h) = (width as f64, height as f64);
        let y = if flip_y { (h - 0.5, -0.5) } else { (-0.5, h - 0.5) };
        // 画面 1 単位 = matplotlib の既定の図(幅 12 インチ・100 dpi、高さ 12·ny/nx + 1.2 インチ)の 1 px とみなす。
        // WCSAxes は「画面で 2 px」進めて目盛りの向きを測るので、差分の幅を Python と同じくらいにする
        View { x: (-0.5, w - 0.5), y, k: screen_scale(w.max(1.0), h.max(1.0)) }
    }

    /// 拡大・移動した表示の範囲(画像の座標)。`x` は (左端, 右端)、`y` は (画面の下端, 上端)
    /// (上下反転の表示なら y.0 > y.1)。目盛りの向きを測る幅は、この範囲を既定の図に表示したときの 2 px。
    pub fn new(x: (f64, f64), y: (f64, f64)) -> View {
        View { x, y, k: screen_scale((x.1 - x.0).abs(), (y.1 - y.0).abs()) }
    }

    fn sx(&self) -> f64 {
        if self.x.1 >= self.x.0 { self.k } else { -self.k }
    }

    fn sy(&self) -> f64 {
        if self.y.1 >= self.y.0 { self.k } else { -self.k }
    }

    /// 画像の座標 → 画面の座標(x 右・y 上)。
    pub fn to_disp(&self, x: f64, y: f64) -> (f64, f64) {
        ((x - self.x.0) * self.sx(), (y - self.y.0) * self.sy())
    }

    /// 画面の座標 → 画像の座標。
    pub fn from_disp(&self, dx: f64, dy: f64) -> (f64, f64) {
        (self.x.0 + dx / self.sx(), self.y.0 + dy / self.sy())
    }

    /// 枠の矩形 (xmin, xmax, ymin, ymax)。
    pub fn rect(&self) -> (f64, f64, f64, f64) {
        (self.x.0.min(self.x.1), self.x.0.max(self.x.1), self.y.0.min(self.y.1), self.y.0.max(self.y.1))
    }
}

/// 幅 w・高さ h(画像の画素)の範囲を、matplotlib の既定の図(幅 12 インチ・100 dpi、高さ 12·h/w + 1.2 インチ)に
/// 表示したときの、画像 1 画素あたりの画面の px(おおよそ。目盛りの向きを差分で求める幅に使う)。
fn screen_scale(w: f64, h: f64) -> f64 {
    let (w1, h1) = (if w.is_finite() { w.max(1e-9) } else { 1.0 }, if h.is_finite() { h.max(1e-9) } else { 1.0 });
    let k = (1100.0 / w1).min((1200.0 * h1 / w1 - 30.0).max(1.0) / h1);
    if k.is_finite() && k > 0.0 { k } else { 1.0 }
}

/// 画素 ↔ 天の写し(SIP は常にニュートン法で解く。astropy の all_world2pix と同じく近似を使わない)。
struct Trans<'a> {
    wcs: &'a Wcs,
    /// 画像の大きさ(遠すぎる点の打ち切りに使う)
    lim: f64,
    cx: f64,
    cy: f64,
}

impl<'a> Trans<'a> {
    fn new(wcs: &'a Wcs, width: usize, height: usize) -> Self {
        let (w, h) = (width as f64, height as f64);
        Trans { wcs, lim: 3.0 * w.max(h).max(1.0), cx: (w - 1.0) / 2.0, cy: (h - 1.0) / 2.0 }
    }

    fn p2w(&self, x: f64, y: f64) -> (f64, f64) {
        if !(x.is_finite() && y.is_finite()) {
            return (f64::NAN, f64::NAN);
        }
        self.wcs.pix_to_radec(x, y)
    }

    fn w2p(&self, lon: f64, lat: f64) -> (f64, f64) {
        if !(lon.is_finite() && lat.is_finite()) {
            return (f64::NAN, f64::NAN);
        }
        // 画像からずっと遠い点は SIP の多項式が意味を持たず、解くのも重いので打ち切る(どのみち枠の外)
        if self.wcs.has_sip() {
            match self.wcs.tan_only().radec_to_pix(lon, lat) {
                Some((x, y)) if (x - self.cx).abs() < self.lim && (y - self.cy).abs() < self.lim => {}
                _ => return (f64::NAN, f64::NAN),
            }
        }
        match self.wcs.radec_to_pix_exact(lon, lat) {
            Some((x, y)) => (x, y),
            None => (f64::NAN, f64::NAN),
        }
    }
}

/// numpy の `np.mod`(npy_divmod の余り。結果は割る数と同じ符号)。
fn np_mod(a: f64, b: f64) -> f64 {
    let m = a % b;
    if b == 0.0 {
        return m;
    }
    if m != 0.0 { if (b < 0.0) != (m < 0.0) { m + b } else { m } } else { 0.0f64.copysign(b) }
}

/// `wrap_angle_at(values, coord_wrap)` = mod(values − wrap, 360) − (360 − wrap)。
fn wrap_angle_at(v: f64, wrap: f64) -> f64 {
    np_mod(v - wrap, 360.0) - (360.0 - wrap)
}

fn wrap_180(v: f64) -> f64 {
    let m = np_mod(v, 360.0);
    if m > 180.0 { m - 360.0 } else { m }
}

/// Python の max(a, b)(b が大きいときだけ b)。
fn py_max(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}

fn py_min(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

/// `np.linspace(a, b, n)`(endpoint=True)。
fn linspace(a: f64, b: f64, n: usize) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![a];
    }
    let div = (n - 1) as f64;
    let step = (b - a) / div;
    let mut v: Vec<f64> = (0..n).map(|i| i as f64 * step + a).collect();
    v[n - 1] = b;
    v
}

fn nan_min_max(v: impl Iterator<Item = f64>) -> (f64, f64) {
    let (mut lo, mut hi) = (f64::NAN, f64::NAN);
    for x in v {
        if x.is_nan() {
            continue;
        }
        if lo.is_nan() || x < lo {
            lo = x;
        }
        if hi.is_nan() || x > hi {
            hi = x;
        }
    }
    (lo, hi)
}

/// `find_coordinate_range`(座標は経度・緯度の 2 つ。赤経の折り返しは 360°)。
fn coordinate_range(tr: &Trans, view: &View) -> [(f64, f64); 2] {
    let n = RANGE_SAMPLES + 1;
    let xs = linspace(view.x.0, view.x.1, n);
    let ys = linspace(view.y.0, view.y.1, n);
    // world[c][j][i]: j が y(行)、i が x(列)
    let mut world = [vec![vec![0.0; n]; n], vec![vec![0.0; n]; n]];
    for (j, &y) in ys.iter().enumerate() {
        for (i, &x) in xs.iter().enumerate() {
            let (lon, lat) = tr.p2w(x, y);
            world[0][j][i] = lon;
            world[1][j][i] = lat;
        }
    }
    let mut out = [(f64::NAN, f64::NAN); 2];
    for (c, xw) in world.iter_mut().enumerate() {
        // 先頭の行の飛びをならす(隣との差は元の値で取る。numpy の一括の引き算と同じ)
        let orig: Vec<f64> = xw[0].clone();
        for i in 1..n {
            let wj = orig[i] - orig[i - 1];
            if wj.abs() > 180.0 {
                let wj2 = wj + wj.signum() * 180.0;
                let wj3 = 360.0 * (wj2 / 360.0).trunc();
                xw[0][i] -= wj3;
            }
        }
        // ほかの行を先頭の行に合わせる
        let row0 = xw[0].clone();
        for row in xw.iter_mut().skip(1) {
            for i in 0..n {
                let wj = row[i] - row0[i];
                if wj.abs() > 180.0 {
                    let wj2 = wj + wj.signum() * 180.0;
                    let wj3 = 360.0 * (wj2 / 360.0).trunc();
                    row[i] -= wj3;
                }
            }
        }
        let all = || xw.iter().flat_map(|r| r.iter().copied());
        let (mut lo, mut hi) = nan_min_max(all());
        // 0〜360 にそろえた方が狭いか
        let (lo2, hi2) = nan_min_max(all().map(|v| np_mod(v, 360.0)));
        if hi2 - lo2 <= hi - lo && hi - lo < 360.0 {
            lo = lo2;
            hi = hi2;
        }
        // −180〜180 にそろえた方が狭いか
        let (lo3, hi3) = nan_min_max(all().map(wrap_180));
        if hi3 - lo3 < 360.0 && hi - lo >= hi3 - lo3 {
            lo = lo3;
            hi = hi3;
        }
        let r = hi - lo;
        if c == 0 {
            if r > 300.0 {
                // 極が入ると全周(折り返しの 360° から 1 ulp 手前まで)
                lo = 0.0;
                hi = 360.0 - 5.684341886080802e-14;
            } else if lo < 0.0 {
                lo = py_max(-180.0, lo - 0.1 * r);
                hi = py_min(180.0, hi + 0.1 * r);
            } else {
                lo = py_max(0.0, lo - 0.1 * r);
                hi = py_min(360.0, hi + 0.1 * r);
            }
        } else {
            lo = py_max(-90.0, lo - 0.1 * r);
            hi = py_min(90.0, hi + 0.1 * r);
        }
        out[c] = (lo, hi);
    }
    out
}

/// numpy の searchsorted(side="left"): 初めて a[i] >= x になる i。
fn searchsorted(a: &[f64], x: f64) -> usize {
    a.iter().position(|&v| v >= x).unwrap_or(a.len())
}

const MINSEC_LIMITS_DEG: [f64; 8] = [1.5, 2.5, 3.5, 8.0, 11.0, 18.0, 25.0, 45.0];
const MINSEC_STEPS_DEG: [f64; 8] = [1.0, 2.0, 3.0, 5.0, 10.0, 15.0, 20.0, 30.0];
const DEGREE_LIMITS: [f64; 10] = [1.5, 3.0, 7.0, 13.0, 20.0, 40.0, 70.0, 120.0, 270.0, 520.0];
const DEGREE_STEPS: [f64; 10] = [1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 45.0, 90.0, 180.0, 360.0];
const MINSEC_LIMITS_HOUR: [f64; 11] = [1.5, 2.5, 3.5, 4.5, 5.5, 8.0, 11.0, 14.0, 18.0, 25.0, 45.0];
const MINSEC_STEPS_HOUR: [f64; 11] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 10.0, 12.0, 15.0, 20.0, 30.0];
const HOUR_LIMITS: [f64; 9] = [1.5, 2.5, 3.5, 5.0, 7.0, 10.0, 15.0, 21.0, 36.0];
const HOUR_STEPS: [f64; 9] = [1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 18.0, 24.0];

/// `select_step_scalar`: 10 の冪 × {1, 2, 5, 10} のうち対数で最も近いもの。
pub fn select_step_scalar(dv: f64) -> f64 {
    let l = dv.log10();
    let base = l.floor();
    let frac = l - base;
    let steps = [0.0, 2f64.log10(), 5f64.log10(), 1.0];
    let mut best = 0;
    for i in 1..4 {
        if (frac - steps[i]).abs() < (frac - steps[best]).abs() {
            best = i;
        }
    }
    10f64.powf(base + steps[best])
}

/// `select_step_degree(dv)`(dv は度)。戻り値は度。
pub fn select_step_degree(dv: f64) -> f64 {
    if dv > ARCSEC_TO_DEG {
        let mut limits = Vec::with_capacity(26);
        limits.extend(MINSEC_LIMITS_DEG.iter().map(|v| v / 3600.0));
        limits.extend(MINSEC_LIMITS_DEG.iter().map(|v| v / 60.0));
        limits.extend(DEGREE_LIMITS);
        let n = searchsorted(&limits, dv).min(limits.len() - 1);
        if n < 8 {
            MINSEC_STEPS_DEG[n] * ARCSEC_TO_DEG
        } else if n < 16 {
            MINSEC_STEPS_DEG[n - 8] * ARCMIN_TO_DEG
        } else {
            DEGREE_STEPS[n - 16]
        }
    } else {
        select_step_scalar(dv * 3600.0) * ARCSEC_TO_DEG
    }
}

/// `select_step_hour(dv)`(dv は度)。戻り値は度。
pub fn select_step_hour(dv: f64) -> f64 {
    let s15 = 15.0 * ARCSEC_TO_DEG; // 15″ = 1 秒(時)
    if dv > s15 {
        let mut limits = Vec::with_capacity(31);
        limits.extend(MINSEC_LIMITS_HOUR.iter().map(|v| v / 3600.0));
        limits.extend(MINSEC_LIMITS_HOUR.iter().map(|v| v / 60.0));
        limits.extend(HOUR_LIMITS);
        let n = searchsorted(&limits, dv * DEG_TO_HOURANGLE).min(limits.len() - 1);
        if n < 11 {
            (MINSEC_STEPS_HOUR[n] * 15.0) * ARCSEC_TO_DEG
        } else if n < 22 {
            (MINSEC_STEPS_HOUR[n - 11] * 15.0) * ARCMIN_TO_DEG
        } else {
            HOUR_STEPS[n - 22] * HOURANGLE_TO_DEG
        }
    } else {
        // 15″ を単位にした数で 1-2-5
        (select_step_scalar(dv / s15) * 15.0) * ARCSEC_TO_DEG
    }
}

/// 目盛りの値と間隔(度)。`locator(value_min, value_max)`。
fn locate(coord: usize, range: (f64, f64)) -> (Vec<f64>, f64) {
    let (lo, hi) = range;
    if !(lo.is_finite() && hi.is_finite()) || lo == hi {
        return (Vec::new(), ARCSEC_TO_DEG);
    }
    let dv = (hi - lo).abs() / TICK_NUMBER;
    let sp = if coord == 0 { select_step_hour(dv) } else { select_step_degree(dv) };
    if !(sp.is_finite() && sp > 0.0) {
        return (Vec::new(), ARCSEC_TO_DEG);
    }
    let imin = (lo / sp).ceil();
    let imax = (hi / sp).floor();
    if !(imin.is_finite() && imax.is_finite()) || imax - imin > MAX_TICKS as f64 {
        return (Vec::new(), sp);
    }
    let (a, b) = (imin as i64, imax as i64);
    ((a..=b).map(|i| i as f64 * sp).collect(), sp)
}

/// 目盛りの文字(`AngleFormatterLocator.formatter`)。赤経は時分秒、赤緯は度分秒。
pub fn format_tick(coord: usize, world_deg: f64, spacing_deg: f64) -> String {
    // 間隔を「秒」(赤経は時の秒、赤緯は角度の秒)で表して、項目数と小数の桁を決める
    let sp = spacing_deg * if coord == 0 { 240.0 } else { 3600.0 };
    let (fields, precision) = if sp >= 3600.0 {
        (1, 0)
    } else if sp >= 60.0 {
        (2, 0)
    } else if sp >= 1.0 {
        (3, 0)
    } else {
        let p = -(sp.log10().floor());
        (3, if p.is_finite() && p > 0.0 { (p as usize).min(12) } else { 0 })
    };
    if coord == 0 {
        sexagesimal_string(world_deg * DEG_TO_HOURANGLE, Some(precision), false, ["ʰ", "ᵐ", "ˢ"], fields)
    } else {
        sexagesimal_string(world_deg, Some(precision), false, ["°", "'", "\""], fields)
    }
}

/// `simplify_labels`: 同じ辺で、前の文字と同じ頭の部分を落とす(数字・小数点・符号は区切りまで戻す)。
fn simplify(texts: &mut [String]) {
    if texts.is_empty() {
        return;
    }
    let skippable = |c: char| "0123456789.+-".contains(c);
    let mut t1: Vec<char> = texts[0].chars().collect();
    for t in texts.iter_mut().skip(1) {
        let t2: Vec<char> = t.chars().collect();
        if t1.len() != t2.len() {
            t1 = t2;
            continue;
        }
        let mut start = 0;
        for j in 0..t1.len().saturating_sub(1) {
            if t1[j] != t2[j] {
                break;
            }
            if !skippable(t1[j]) {
                start = j + 1;
            }
        }
        t1 = t2.clone();
        if start != 0 {
            *t = t2[start..].iter().collect();
        }
    }
}

/// 枠の 1 辺の標本。
struct Spine {
    side: Side,
    /// 画像の座標
    data: Vec<[f64; 2]>,
    /// (赤経, 赤緯)
    world: Vec<[f64; 2]>,
    /// 各区間の内向きの法線の角度(画面、度)
    normal: Vec<f64>,
}

fn spines(tr: &Trans, view: &View) -> Vec<Spine> {
    let (x0, x1) = view.x;
    let (y0, y1) = view.y;
    let ends = [
        (Side::Bottom, [x0, y0], [x1, y0]),
        (Side::Right, [x1, y0], [x1, y1]),
        (Side::Top, [x1, y1], [x0, y1]),
        (Side::Left, [x0, y1], [x0, y0]),
    ];
    ends.iter()
        .map(|&(side, a, b)| {
            let p = linspace(0.0, 1.0, FRAME_SAMPLES);
            let data: Vec<[f64; 2]> = p
                .iter()
                .map(|&t| {
                    // np.interp(t, [0, 1], [a, b])(端は端の値ちょうど)
                    if t >= 1.0 { b } else { [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t] }
                })
                .collect();
            let world: Vec<[f64; 2]> = data
                .iter()
                .map(|d| {
                    let (l, b) = tr.p2w(d[0], d[1]);
                    [l, b]
                })
                .collect();
            let disp: Vec<(f64, f64)> = data.iter().map(|d| view.to_disp(d[0], d[1])).collect();
            let normal = disp.windows(2).map(|w| (w[1].0 - w[0].0).atan2(-(w[1].1 - w[0].1)).to_degrees()).collect();
            Spine { side, data, world, normal }
        })
        .collect()
}

struct RawLabel {
    coord: usize,
    side: Side,
    pos: [f64; 2],
    world: f64,
    disp: f64,
}

/// 1 つの座標の目盛り(`_update_ticks` と `_compute_ticks`)。
fn ticks_for(
    coord: usize,
    tick_values: &[f64],
    sp: &Spine,
    tr: &Trans,
    view: &View,
    ticks: &mut Vec<Tick>,
    labels: &mut Vec<RawLabel>,
) {
    let n = sp.data.len();
    if n < 2 || sp.world.iter().all(|w| w[coord].is_nan()) {
        return;
    }
    // 目盛りの向き: 画面で 2 単位ずつ進めた点の座標の差から、等値線の向き(内向き)を出す。
    // WCSAxes と同じく、縦は画像の +y の向きへ進める(上下反転の表示では画面の下向き)。WCSAxes はその差を
    // そのまま画面の角度として使うので、上下反転の図では目盛りが左右の鏡像に傾く。ここでは画面の向きに直す
    let flipped = view.y.1 < view.y.0;
    let mut tick_angle: Vec<f64> = Vec::with_capacity(n);
    for (i, d) in sp.data.iter().enumerate() {
        let (dx0, dy0) = view.to_disp(d[0], d[1]);
        let (ax, ay) = view.from_disp(dx0 + 2.0, dy0);
        let (bx, by) = view.from_disp(dx0, dy0 + if flipped { -2.0 } else { 2.0 });
        let w0 = sp.world[i][coord];
        let w1 = tr.p2w(ax, ay);
        let w2 = tr.p2w(bx, by);
        let (w1, w2) = if coord == 0 { (w1.0, w2.0) } else { (w1.1, w2.1) };
        let mut dx = w1 - w0;
        let mut dy = w2 - w0;
        (dx, dy) = (-dy, dx);
        if coord == 0 {
            dx = wrap_angle_at(dx, 180.0);
            dy = wrap_angle_at(dy, 180.0);
        }
        let mut a = dy.atan2(dx).to_degrees();
        if flipped {
            a = -a;
        }
        let normal = if i < n - 1 { sp.normal[i] } else { sp.normal[n - 2] };
        let r1 = np_mod(normal - a, 360.0) > 90.0;
        let r2 = np_mod(a - normal, 360.0) > 90.0;
        if r1 && r2 {
            a -= 180.0;
        }
        tick_angle.push(a);
    }
    // 区間ごとの始点・終点の座標(赤経は折り返しをならす)
    let mut w1: Vec<f64> = sp.world[..n - 1].iter().map(|w| w[coord]).collect();
    let mut w2: Vec<f64> = sp.world[1..].iter().map(|w| w[coord]).collect();
    if coord == 0 {
        for v in w1.iter_mut().chain(w2.iter_mut()) {
            *v = wrap_angle_at(*v, 360.0);
        }
        for i in 0..w1.len() {
            if w2[i] - w1[i] > 180.0 {
                w1[i] += 360.0;
            }
        }
        for i in 0..w1.len() {
            if w1[i] - w2[i] > 180.0 {
                w2[i] += 360.0;
            }
        }
    }
    let mut values: Vec<f64> = tick_values.to_vec();
    if coord == 0 {
        values.extend(tick_values.iter().map(|t| t + 360.0));
    }
    let m = w1.len();
    for &t in &values {
        let mut hits: Vec<usize> = (0..m).filter(|&i| t - w1[i] == 0.0).collect();
        hits.extend((0..m).filter(|&i| (t - w1[i]) * (t - w2[i]) < 0.0));
        if t - w2[m - 1] == 0.0 {
            hits.push(m - 1);
        }
        for imin in hits {
            let imax = imin + 1;
            // np.allclose(w1, w2, rtol=1e-13, atol=1e-13): 枠と平行な等値線は飛ばす
            if (w1[imin] - w2[imin]).abs() <= 1e-13 + 1e-13 * w2[imin].abs() {
                continue;
            }
            let frac = (t - w1[imin]) / (w2[imin] - w1[imin]);
            let d0 = sp.data[imin];
            let d1 = sp.data[imax];
            let x = d0[0] + frac * (d1[0] - d0[0]);
            let y = d0[1] + frac * (d1[1] - d0[1]);
            let mut da = tick_angle[imax] - tick_angle[imin];
            if da > 180.0 {
                da -= 360.0;
            } else if da < -180.0 {
                da += 360.0;
            }
            let angle = tick_angle[imin] + frac * da;
            let world = if coord == 0 { wrap_angle_at(t, 360.0) } else { t };
            ticks.push(Tick { coord, side: sp.side, pos: [x, y], angle_deg: angle, world });
            labels.push(RawLabel { coord, side: sp.side, pos: [x, y], world, disp: imin as f64 + frac });
        }
    }
}

/// `get_lon_lat_path` で線を切る位置を決め、点の並びを区間に分ける。
fn split_gridline(world: &[[f64; 2]], pixel: &[[f64; 2]], check: &[[f64; 2]]) -> Vec<Vec<[f64; 2]>> {
    let n = world.len();
    if n < 2 {
        return Vec::new();
    }
    let rad = |w: [f64; 2]| [w[0].to_radians(), w[1].to_radians()];
    let scale = angular_separation(rad(world[0]), rad(world[1]));
    let mut moveto = vec![false; n];
    moveto[0] = true;
    let mut mask = vec![false; n];
    for i in 0..n {
        let s = angular_separation(rad(world[i]), rad(check[i]));
        let s = if s > std::f64::consts::PI { s - 2.0 * std::f64::consts::PI } else { s };
        mask[i] = s > ROUND_TRIP_RTOL * scale || pixel[i][0].is_nan() || pixel[i][1].is_nan();
    }
    for i in 0..n {
        if mask[i] {
            moveto[i] = true;
            if i + 1 < n {
                moveto[i + 1] = true;
            }
        }
    }
    let step: Vec<f64> =
        (0..n - 1).map(|i| (pixel[i + 1][0] - pixel[i][0]).hypot(pixel[i + 1][1] - pixel[i][1])).collect();
    for i in 0..step.len().saturating_sub(1) {
        if step[i + 1] > DISCONT_FACTOR * step[i] {
            moveto[i + 2] = true;
        }
    }
    if step.len() >= 2 && step[0] > DISCONT_FACTOR * step[1] {
        moveto[1] = true;
    }
    let mut out: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut cur: Vec<[f64; 2]> = Vec::new();
    for i in 0..n {
        if moveto[i] {
            if cur.len() >= 2 {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
        }
        if pixel[i][0].is_finite() && pixel[i][1].is_finite() {
            cur.push(pixel[i]);
        } else {
            cur.clear();
        }
    }
    if cur.len() >= 2 {
        out.push(cur);
    }
    out
}

/// astropy の angular_separation(Vincenty の式、ラジアン)。
fn angular_separation(a: [f64; 2], b: [f64; 2]) -> f64 {
    let (sdlon, cdlon) = (b[0] - a[0]).sin_cos();
    let (slat1, clat1) = a[1].sin_cos();
    let (slat2, clat2) = b[1].sin_cos();
    let num1 = clat2 * sdlon;
    let num2 = clat1 * slat2 - slat1 * clat2 * cdlon;
    let den = slat1 * slat2 + clat1 * clat2 * cdlon;
    num1.hypot(num2).atan2(den)
}

/// 折れ線を矩形で切り取る(Liang–Barsky)。はみ出た所で区間を分ける。
pub(crate) fn clip_polyline(pts: &[[f64; 2]], rect: (f64, f64, f64, f64)) -> Vec<Vec<[f64; 2]>> {
    let (xmin, xmax, ymin, ymax) = rect;
    let mut out: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut cur: Vec<[f64; 2]> = Vec::new();
    for w in pts.windows(2) {
        let (p, q) = (w[0], w[1]);
        let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
        let mut t0 = 0.0f64;
        let mut t1 = 1.0f64;
        let mut ok = true;
        for (pp, qq) in [(-dx, p[0] - xmin), (dx, xmax - p[0]), (-dy, p[1] - ymin), (dy, ymax - p[1])] {
            if pp == 0.0 {
                if qq < 0.0 {
                    ok = false;
                    break;
                }
            } else {
                let r = qq / pp;
                if pp < 0.0 {
                    if r > t1 {
                        ok = false;
                        break;
                    }
                    t0 = t0.max(r);
                } else {
                    if r < t0 {
                        ok = false;
                        break;
                    }
                    t1 = t1.min(r);
                }
            }
        }
        // t0・t1 が NaN(比べられない)ときも切り捨てる
        if !ok || t0.is_nan() || t1.is_nan() || t0 > t1 {
            if cur.len() >= 2 {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
            continue;
        }
        let a = [p[0] + t0 * dx, p[1] + t0 * dy];
        let b = [p[0] + t1 * dx, p[1] + t1 * dy];
        let continues = t0 == 0.0 && cur.last().is_some_and(|l| *l == p);
        if !continues {
            if cur.len() >= 2 {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
            cur.push(a);
        }
        cur.push(b);
        if t1 < 1.0 {
            if cur.len() >= 2 {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
        }
    }
    if cur.len() >= 2 {
        out.push(cur);
    }
    out
}

/// 画像全体を表示したときの、赤経赤緯の格子・目盛り・文字(WCSAxes と同じ選び方)。
pub fn wcs_grid(wcs: &Wcs, width: usize, height: usize, flip_y: bool) -> Grid {
    let view = View::image(width, height, flip_y);
    grid_for_view(wcs, width, height, &view)
}

/// 表示の範囲を指定して格子を作る(拡大した表示に描き直すとき)。
pub fn grid_for_view(wcs: &Wcs, width: usize, height: usize, view: &View) -> Grid {
    let tr = Trans::new(wcs, width, height);
    let ranges = coordinate_range(&tr, view);
    let mut grid = Grid { ranges, ..Default::default() };
    for (c, &range) in ranges.iter().enumerate() {
        let (vals, sp) = locate(c, range);
        grid.tick_values[c] = vals;
        grid.spacing[c] = sp;
    }
    // 格子の線(赤経の線 → 赤緯の線の順。WCSAxes の描く順)
    let rect = view.rect();
    for c in 0..2 {
        let other = ranges[1 - c];
        let samples = linspace(other.0, other.1, GRID_SAMPLES);
        for &w in &grid.tick_values[c] {
            let world: Vec<[f64; 2]> = samples.iter().map(|&s| if c == 0 { [w, s] } else { [s, w] }).collect();
            let pixel: Vec<[f64; 2]> = world
                .iter()
                .map(|p| {
                    let (x, y) = tr.w2p(p[0], p[1]);
                    [x, y]
                })
                .collect();
            let check: Vec<[f64; 2]> = pixel
                .iter()
                .map(|p| {
                    let (l, b) = tr.p2w(p[0], p[1]);
                    [l, b]
                })
                .collect();
            for piece in split_gridline(&world, &pixel, &check) {
                // 極ちょうどの赤緯の線のように、全部の点が 1 つに重なる線は描かない(長さ 0。astropy も描かない)
                let (x0, x1, y0, y1) = piece
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY), |(a, b, c, d), p| {
                        (a.min(p[0]), b.max(p[0]), c.min(p[1]), d.max(p[1]))
                    });
                if x1 - x0 < 1e-9 && y1 - y0 < 1e-9 {
                    continue;
                }
                for clipped in clip_polyline(&piece, rect) {
                    grid.lines.push(GridLine { coord: c, world: w, points: clipped });
                }
            }
        }
    }
    // 目盛り
    let sps = spines(&tr, view);
    let mut raw: Vec<RawLabel> = Vec::new();
    for c in 0..2 {
        for sp in &sps {
            ticks_for(c, &grid.tick_values[c], sp, &tr, view, &mut grid.ticks, &mut raw);
        }
    }
    // どの辺に文字を出すか(b・l・t・r の順に 2 つ選ぶ組み合わせで、文字の数が最多のもの)
    let order = [Side::Bottom, Side::Left, Side::Top, Side::Right];
    let count = |c: usize, s: Side| grid.ticks.iter().filter(|t| t.coord == c && t.side == s).count() as i64;
    let mut best: Option<(Side, Side)> = None;
    let mut best_n = -1i64;
    for &s0 in &order {
        for &s1 in &order {
            if s0 == s1 {
                continue;
            }
            let mut n_tick = count(0, s0) + count(1, s1);
            let sorted: Vec<i64> = order
                .iter()
                .map(|&s| {
                    if s == s0 {
                        count(0, s0)
                    } else if s == s1 {
                        count(1, s1)
                    } else {
                        0
                    }
                })
                .collect();
            if let Some(last) = sorted.iter().rposition(|&v| v != 0) {
                n_tick -= sorted[..last].iter().filter(|&&v| v == 0).count() as i64;
            }
            if n_tick > best_n {
                best_n = n_tick;
                best = Some((s0, s1));
            }
        }
    }
    let (side0, side1) = best.unwrap_or((Side::Bottom, Side::Left));
    // 文字(並べてから頭の重なりを落とす)
    for (c, side) in [(0usize, side0), (1usize, side1)] {
        let mut mine: Vec<&RawLabel> = raw.iter().filter(|l| l.coord == c && l.side == side).collect();
        let mut texts: Vec<(f64, String, usize)> =
            mine.iter().enumerate().map(|(i, l)| (l.disp, format_tick(c, l.world, grid.spacing[c]), i)).collect();
        texts.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let order_idx: Vec<usize> = texts.iter().map(|t| t.2).collect();
        let mut strs: Vec<String> = texts.into_iter().map(|t| t.1).collect();
        simplify(&mut strs);
        let sorted: Vec<&RawLabel> = order_idx.iter().map(|&i| mine[i]).collect();
        mine.clear();
        let mut any = false;
        for (l, s) in sorted.into_iter().zip(strs) {
            if s.is_empty() {
                continue;
            }
            any = true;
            grid.labels.push(TickLabel { coord: c, side, pos: l.pos, text: s, world: l.world });
        }
        if any {
            grid.axis_labels.push(AxisLabel { coord: c, side, text: AXIS_LABELS[c].to_string() });
        }
    }
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps() {
        // 範囲 / 5 に近いきりのよい値
        // 0.3° = 18′ は限界値 18′ ちょうど(searchsorted の左)なので 15′
        assert_eq!(select_step_degree(0.3), 15.0 * ARCMIN_TO_DEG);
        assert_eq!(select_step_degree(0.31), 20.0 * ARCMIN_TO_DEG);
        assert_eq!(select_step_degree(4.0), 5.0);
        assert_eq!(select_step_hour(72.0), 4.0 * HOURANGLE_TO_DEG);
        assert!((select_step_hour(0.5) - 2.0 * 15.0 * ARCMIN_TO_DEG).abs() < 1e-15);
        assert!((select_step_scalar(0.3) - 0.2).abs() < 1e-15);
    }

    #[test]
    fn simplify_drops_common_heads() {
        let mut v =
            vec!["4ʰ07ᵐ30ˢ".to_string(), "4ʰ07ᵐ45ˢ".to_string(), "4ʰ08ᵐ00ˢ".to_string(), "10ʰ00ᵐ00ˢ".to_string()];
        simplify(&mut v);
        assert_eq!(v, ["4ʰ07ᵐ30ˢ", "45ˢ", "08ᵐ00ˢ", "10ʰ00ᵐ00ˢ"]);
        let mut d = vec!["41°30'".to_string(), "41°45'".to_string(), "42°00'".to_string()];
        simplify(&mut d);
        assert_eq!(d, ["41°30'", "45'", "42°00'"]);
    }

    #[test]
    fn spans() {
        let l = TickLabel { coord: 0, side: Side::Bottom, pos: [0.0, 0.0], text: "4ʰ07ᵐ".into(), world: 61.75 };
        assert_eq!(l.plain(), "4h07m");
        let s = l.spans();
        assert_eq!(s.len(), 4);
        assert!(s[1].superscript && s[1].text == "h");
    }

    #[test]
    fn clip() {
        let r = (0.0, 10.0, 0.0, 10.0);
        let pieces = clip_polyline(&[[-5.0, 5.0], [5.0, 5.0], [15.0, 5.0], [15.0, 6.0], [5.0, 6.0]], r);
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0], vec![[0.0, 5.0], [5.0, 5.0], [10.0, 5.0]]);
        assert_eq!(pieces[1], vec![[10.0, 6.0], [5.0, 6.0]]);
        assert!(clip_polyline(&[[20.0, 20.0], [30.0, 30.0]], r).is_empty());
    }
}
