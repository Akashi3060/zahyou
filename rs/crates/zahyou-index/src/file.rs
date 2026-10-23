//! 索引ファイル 1 本(Python 版 `SvIndexFile`)。ヘッダーは開くときに読み、表は使うときに mmap する。

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use memmap2::Mmap;
use rayon::prelude::*;
use zahyou_wcs::{angle_between, normalize, Vec3, ARCSEC_PER_RAD};

use crate::healpix::{healpix_xy_center, healpix_xy_radius};
use crate::kdtree::{ball_query, cone_visit, star_unit, Ints, KdInt, Tree, TreeMeta};
use crate::layout::{layout_from_source, name_for_check, read_layout, Layout, LayoutError};

/// 索引のエラー。`Display` は画面に出す文(形式の問題は Python 版と同じ文面)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexError {
    /// OS のエラー(開けない・読めない)。表示は「読めません(…)」
    Io(String),
    /// 索引の形式の問題
    Format(String),
    /// この機械では読めない(ビッグエンディアンの CPU・Windows 以外の OS)
    Unsupported(String),
    /// 呼び出しの引数がおかしい(問い合わせの形など)
    Argument(String),
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IndexError::Io(e) => write!(f, "読めません({e})"),
            IndexError::Format(s) | IndexError::Unsupported(s) | IndexError::Argument(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for IndexError {}

impl From<LayoutError> for IndexError {
    fn from(e: LayoutError) -> IndexError {
        match e {
            LayoutError::Io(e) => IndexError::Io(e.to_string()),
            LayoutError::Bad(s) => IndexError::Format(s),
        }
    }
}

/// 一度にまとめて並列にする問い合わせの最小の数(これより少なければ 1 本のスレッドで探す)。
const PAR_MIN_QUERIES: usize = 32;

/// 索引ファイル 1 本。ヘッダーは `open` で読み、表は使うときに mmap する(`close` で放す)。
///
/// 公開している値は Python 版の属性と同じ意味(design-native-solver.md 2 節)。書き換えないこと。
pub struct IndexFile {
    pub path: PathBuf,
    /// 拡張子なしのファイル名(例 "index-5206-02")
    pub name: String,
    pub indexid: u32,
    /// タイルの番号(xy 方式の HEALPix)。全天は −1
    pub healpix: i32,
    pub hpnside: i64,
    /// quad の星の数(3 か 4)
    pub dimquads: usize,
    pub nquads: usize,
    pub nstars: usize,
    /// quad の AB の角距離の範囲(SCALE_L / SCALE_U、ラジアン)
    pub scale_lo_rad: f64,
    pub scale_hi_rad: f64,
    /// 星の木のヘッダーの JITTER(無ければ 1.0)
    pub jitter_arcsec: f64,
    /// 一様化の HEALPix の nside
    pub cutnside: Option<u32>,
    /// タイルの中心と外接半径(全天の索引は None)
    pub tile_center: Option<Vec3>,
    pub tile_radius_rad: Option<f64>,
    layout: Layout,
    mapped: Mutex<Option<Arc<IndexData>>>,
}

impl fmt::Debug for IndexFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "IndexFile({:?}, dimquads={}, nquads={}, nstars={}, {})",
            self.name,
            self.dimquads,
            self.nquads,
            self.nstars,
            if self.is_open() { "open" } else { "closed" }
        )
    }
}

impl IndexFile {
    /// ヘッダーだけ読んで検査する(表は使うときに mmap)。
    pub fn open(path: impl AsRef<Path>) -> Result<IndexFile, IndexError> {
        let path = path.as_ref();
        let lay = read_layout(path, None)?;
        Ok(IndexFile::from_layout(path, lay))
    }

    pub(crate) fn from_layout(path: &Path, lay: Layout) -> IndexFile {
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let (tile_center, tile_radius_rad) = match (u32::try_from(lay.healpix), u32::try_from(lay.hpnside)) {
            (Ok(hp), Ok(ns)) if ns > 0 => (Some(healpix_xy_center(hp, ns)), Some(healpix_xy_radius(hp, ns))),
            _ => (None, None),
        };
        IndexFile {
            path: path.to_path_buf(),
            name,
            indexid: lay.indexid,
            healpix: lay.healpix,
            hpnside: lay.hpnside,
            dimquads: lay.dimquads,
            nquads: lay.nquads,
            nstars: lay.nstars,
            scale_lo_rad: lay.scale_l,
            scale_hi_rad: lay.scale_u,
            jitter_arcsec: lay.jitter.filter(|j| *j > 0.0).unwrap_or(1.0),
            cutnside: lay.cutnside,
            tile_center,
            tile_radius_rad,
            layout: lay,
            mapped: Mutex::new(None),
        }
    }

    /// 全天の索引か。
    pub fn allsky(&self) -> bool {
        self.healpix < 0
    }

    /// コードの次元(= 2·(dimquads − 2))。
    pub fn code_dim(&self) -> usize {
        2 * (self.dimquads - 2)
    }

    pub fn scale_lo_arcsec(&self) -> f64 {
        self.scale_lo_rad * ARCSEC_PER_RAD
    }

    pub fn scale_hi_arcsec(&self) -> f64 {
        self.scale_hi_rad * ARCSEC_PER_RAD
    }

    /// mmap 済みか。
    pub fn is_open(&self) -> bool {
        self.mapped.lock().map(|g| g.is_some()).unwrap_or_else(|e| e.into_inner().is_some())
    }

    /// 表を mmap して、そのハンドルを返す(何度呼んでもよい。2 回目からは同じものを返す)。
    ///
    /// 探索を何度も繰り返すときは、これで得たハンドルを持って [`IndexData`] のメソッドを呼ぶと
    /// 毎回の錠を省ける。ハンドルが 1 つでも残っている間はファイルが mmap されたまま。
    pub fn data(&self) -> Result<Arc<IndexData>, IndexError> {
        if let Some(d) = self.mapped.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            return Ok(Arc::clone(d));
        }
        // 錠を持たずに開く。開くときの木の検査は rayon で並列に走り、rayon の作業スレッドは待つ間に別の仕事を
        // 横取りする。錠を持ったままだと、横取りした仕事が同じ索引を開こうとして自分の持つ錠を待ち、止まる
        // (同じ目録を 2 つの探索が同時に使ったときに実際に止まった)。同時に開いたら先に入れたほうを使う
        let d = Arc::new(IndexData::map(self)?);
        let mut g = self.mapped.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(first) = g.as_ref() {
            return Ok(Arc::clone(first));
        }
        *g = Some(Arc::clone(&d));
        Ok(d)
    }

    /// mmap を放す(Windows でファイルのロックが残らないように)。あとで使えばまた開く。
    /// [`IndexFile::data`] で受け取ったハンドルが外に残っていれば、それが消えたときに放される。
    pub fn close(&self) {
        let mut g = self.mapped.lock().unwrap_or_else(|e| e.into_inner());
        *g = None;
    }

    /// コードが ‖q − c‖ ≤ radius の quad を、問い合わせのそれぞれについて全部探す。
    ///
    /// queries は Q × code_dim の実数(行優先)。戻り値は (問い合わせの番号, quad の番号) の組で、
    /// 問い合わせの番号 → quad の番号の順に並ぶ。開けない・形が違うときは空(理由は [`IndexFile::try_code_ball`])。
    pub fn code_ball(&self, queries: &[f64], radius: f64) -> Vec<(u32, u32)> {
        self.try_code_ball(queries, radius).unwrap_or_default()
    }

    /// [`IndexFile::code_ball`] の、エラーを返す版。
    pub fn try_code_ball(&self, queries: &[f64], radius: f64) -> Result<Vec<(u32, u32)>, IndexError> {
        self.data()?.code_ball(queries, radius)
    }

    /// quad の星番号 [A, B, C, D](DIMQUADS = 3 なら D は u32::MAX)。
    /// quad の番号が範囲の外・星番号が壊れている・開けないときは全部 u32::MAX。
    pub fn quad_stars(&self, quad: u32) -> [u32; 4] {
        self.data().ok().and_then(|d| d.quad_stars(quad)).unwrap_or([u32::MAX; 4])
    }

    /// 星の単位ベクトル(長さ 1 に正規化済み)。番号が範囲の外・開けないときは NaN。
    pub fn star_xyz(&self, id: u32) -> Vec3 {
        self.data().ok().and_then(|d| d.star_xyz(id)).unwrap_or([f64::NAN; 3])
    }

    /// 星の sweep(小さいほど早く選ばれた = そのあたりで明るい星)。番号が範囲の外・開けないときは 255。
    pub fn sweep(&self, id: u32) -> u8 {
        self.data().ok().and_then(|d| d.sweep(id)).unwrap_or(u8::MAX)
    }

    /// center から角距離 radius_rad 以内の星の番号(昇順)。開けないときは空。
    pub fn stars_in_cone(&self, center: Vec3, radius_rad: f64) -> Vec<u32> {
        self.data().map(|d| d.stars_in_cone(center, radius_rad)).unwrap_or_default()
    }

    pub(crate) fn layout(&self) -> &Layout {
        &self.layout
    }
}

/// mmap した索引の表。[`IndexFile::data`] で得る。
pub struct IndexData {
    mmap: Mmap,
    // 共有モードで書き込み・削除を拒否して開いたハンドル。mmap が生きている間は持ち続ける(SAFETY の前提)。
    // フィールドは宣言の順に捨てられるので mmap より後ろに置く(mmap を放してから閉じる)
    _file: File,
    name: String,
    dimquads: usize,
    nquads: usize,
    nstars: usize,
    quads: (usize, usize),
    sweep: (usize, usize),
    mag: Option<(usize, usize, usize)>,
    codes: TreeMeta,
    stars: TreeMeta,
    tile_center: Option<Vec3>,
    tile_radius_rad: Option<f64>,
}

impl fmt::Debug for IndexData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IndexData({:?}, {} バイト)", self.name, self.mmap.len())
    }
}

/// 書き込みと削除を拒否する共有モードで開く(Windows)。開いている間、ほかの open(このプロセスの中の open も含む)は
/// 書き換えも削除も名前の変更もできず、すでに書き込み用に開いているものがあればこちらが失敗する。
fn open_deny_write(path: &Path) -> io::Result<File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(path)
    }
    #[cfg(not(windows))]
    {
        OpenOptions::new().read(true).open(path)
    }
}

/// 開けなかった理由の文。共有違反(Windows の ERROR_SHARING_VIOLATION = 32)は、ほかのプロセスが書き込み用に
/// 開いているためなので、そう書き足す(中身が壊れているのではない)。
fn open_error_text(e: &io::Error) -> String {
    if cfg!(windows) && e.raw_os_error() == Some(32) {
        format!("ほかのプロセスが書き込み用に開いています: {e}")
    } else {
        e.to_string()
    }
}

/// ファイル全体を読み取り専用で mmap する(Windows だけ)。この crate で unsafe を使うのはここだけ。
#[cfg(windows)]
#[allow(unsafe_code)]
fn map_readonly(file: &File) -> io::Result<Mmap> {
    // SAFETY: memmap2 の Mmap::map が unsafe なのは、mmap の下のファイルがほかから書き換えられたり
    // 切り詰められたりすると、Rust の &[u8] の不変の約束が破れる(読む値が変わる・範囲の外になる)ためである。
    // - file は open_deny_write で FILE_SHARE_READ だけを許して開いてある。Windows は、この open のハンドル(複製も含む)が
    //   残っている間、ほかの open が書き込み用に開くこと・削除・名前の変更を共有違反で拒否する。共有モードの照合はプロセスを
    //   区別しないので、このプロセスの中の別の open も拒否される(開く時点で書き込み用に開いているものがあれば、こちらの open が
    //   失敗する)。mmap 中のファイルの切り詰めも拒否する(ERROR_USER_MAPPED_FILE)。
    // - その open のハンドルは mmap より長く持つ。IndexData の _file は mmap より後ろのフィールドなので、mmap を放してから
    //   閉じる(フィールドは宣言の順に捨てられる)。memmap2 も DuplicateHandle で複製したハンドル(同じ open を指す)を
    //   Mmap の中に持ち、UnmapViewOfFile の後で閉じる。
    // - このプロセスの中で索引のファイルを書いたり消したりするのは zahyou-net のダウンロード(落とし直しを含む)で、.part に
    //   書いてから名前を変える。mmap 中の索引を消したり .part へ移したりしようとしても、上の共有モードで失敗する(削除と
    //   書き込み用の open は試験 close_releases_file で確かめている。名前の変更も削除と同じアクセスで開くので拒否される)。
    //   この crate は読み取り専用の mmap しか作らない。
    // - 長さが変わっていないことは呼ぶ側が先に確かめ、表の位置はすべて mmap の長さの内か確かめてから読む。
    // 前提は Windows のローカルのディスク(内蔵・外付け)に置いたとき。ネットワーク上の置き場所では、サーバーの種類によっては
    // サーバーの側やほかの経路からの書き換えまでは止められない(コードでは確かめず、README で NAS を勧めないと書くだけ)。
    // ほかの OS は書き込みを止めないので mmap しない(IndexData::map が先に断る)。
    unsafe { Mmap::map(file) }
}

/// Windows 以外では mmap しない(SAFETY の前提を OS が守らせないため)。IndexData::map が先に断るので、ここへは来ない。
#[cfg(not(windows))]
fn map_readonly(_file: &File) -> io::Result<Mmap> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

impl IndexData {
    fn map(f: &IndexFile) -> Result<IndexData, IndexError> {
        if cfg!(target_endian = "big") {
            return Err(IndexError::Unsupported(
                "ビッグエンディアンの CPU では索引を読めません(表はリトルエンディアン)".to_string(),
            ));
        }
        if cfg!(not(windows)) {
            return Err(IndexError::Unsupported(
                "Windows 以外では索引を読めません(使っている間にファイルが書き換えられないことを OS が守らないため)"
                    .to_string(),
            ));
        }
        let lay = f.layout();
        let file = open_deny_write(&f.path).map_err(|e| IndexError::Io(open_error_text(&e)))?;
        let len = file.metadata().map_err(|e| IndexError::Io(e.to_string()))?.len();
        if len != lay.size {
            return Err(IndexError::Format(format!("{}: 開いている間にファイルの大きさが変わりました", f.name)));
        }
        let mmap = map_readonly(&file).map_err(|e| IndexError::Io(e.to_string()))?;
        let bytes: &[u8] = &mmap;
        if bytes.len() as u64 != lay.size {
            return Err(IndexError::Format(format!("{}: 開いている間にファイルの大きさが変わりました", f.name)));
        }
        // mmap した中身のヘッダーを読み直し、開いたときと同じか確かめる(別のファイルに入れ替わっていないか)
        let fname = name_for_check(&f.path, None);
        let again = layout_from_source(&mut &bytes[..], lay.size, &fname)?;
        if again != *lay {
            return Err(IndexError::Format(format!("{}: 開いている間にファイルの中身が変わりました", f.name)));
        }
        let codes = TreeMeta::build("codes", &lay.codes, bytes).map_err(IndexError::Format)?;
        let stars = TreeMeta::build("stars", &lay.stars, bytes).map_err(IndexError::Format)?;
        let span = |off: u64, n: usize| -> Result<(usize, usize), IndexError> {
            let o = usize::try_from(off).map_err(|_| IndexError::Format("表の位置がファイルの外です".into()))?;
            match o.checked_add(n) {
                Some(e) if e <= bytes.len() => Ok((o, n)),
                _ => Err(IndexError::Format("表の位置がファイルの外です".into())),
            }
        };
        let quads = span(lay.quads, 4 * lay.dimquads * lay.nquads)?;
        // quads は u32 の並びとして読めること(位置が 4 バイトに揃っている)
        bytemuck::try_cast_slice::<u8, u32>(&bytes[quads.0..quads.0 + quads.1])
            .map_err(|e| IndexError::Format(format!("表 quads を整数の並びとして読めません({e:?})")))?;
        let sweep = span(lay.sweep, lay.nstars)?;
        let mag = match &lay.mag {
            Some(m) => {
                let s = span(m.offset, 16 * lay.nstars)?;
                Some((s.0, m.row_bytes as usize, m.col_offset as usize))
            }
            None => None,
        };
        let name = f.name.clone();
        let (dimquads, nquads, nstars) = (lay.dimquads, lay.nquads, lay.nstars);
        Ok(IndexData {
            mmap,
            _file: file,
            name,
            dimquads,
            nquads,
            nstars,
            quads,
            sweep,
            mag,
            codes,
            stars,
            tile_center: f.tile_center,
            tile_radius_rad: f.tile_radius_rad,
        })
    }

    fn bytes(&self, (o, n): (usize, usize)) -> &[u8] {
        self.mmap.get(o..o + n).unwrap_or(&[])
    }

    fn quads_table(&self) -> &[u32] {
        bytemuck::try_cast_slice(self.bytes(self.quads)).unwrap_or(&[])
    }

    fn tree<'a>(&'a self, meta: &'a TreeMeta) -> Option<Tree<'a>> {
        meta.view(&self.mmap).ok()
    }

    /// ファイル名(拡張子なし)。
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn dimquads(&self) -> usize {
        self.dimquads
    }

    pub fn nquads(&self) -> usize {
        self.nquads
    }

    pub fn nstars(&self) -> usize {
        self.nstars
    }

    /// コードの次元(= 2·(dimquads − 2))。
    pub fn code_dim(&self) -> usize {
        self.codes.ndim
    }

    /// コードが ‖q − c‖ ≤ radius の quad を、問い合わせ(Q × code_dim の実数、行優先)のそれぞれについて全部探す。
    ///
    /// 戻り値は (問い合わせの番号, quad の番号)。問い合わせの番号 → quad の番号の順に並ぶ。
    /// 保存されている木をたどり(作り直さない)、木の単位で量子化の分(D × 0.25 目盛り²)だけ広げて候補を取り、
    /// 最後に実数で判定する。問い合わせが多ければ rayon で並列に探す(結果の順は変わらない)。
    pub fn code_ball(&self, queries: &[f64], radius: f64) -> Result<Vec<(u32, u32)>, IndexError> {
        let d = self.codes.ndim;
        if !queries.len().is_multiple_of(d) {
            return Err(IndexError::Argument(format!(
                "問い合わせの長さ {} が {d} の倍数ではありません",
                queries.len()
            )));
        }
        let nq = queries.len() / d;
        if nq == 0 || radius.is_nan() || radius < 0.0 {
            return Ok(Vec::new());
        }
        if nq > u32::MAX as usize {
            return Err(IndexError::Argument("問い合わせが多すぎます".to_string()));
        }
        let Some(t) = self.tree(&self.codes) else { return Ok(Vec::new()) };
        let per_query = match d {
            2 => run_ball::<2>(&t, queries, radius),
            3 => run_ball::<3>(&t, queries, radius),
            4 => run_ball::<4>(&t, queries, radius),
            _ => Vec::new(),
        };
        let total: usize = per_query.iter().map(Vec::len).sum();
        let mut out = Vec::with_capacity(total);
        for (qi, hits) in per_query.into_iter().enumerate() {
            out.extend(hits.into_iter().map(|c| (qi as u32, c)));
        }
        Ok(out)
    }

    /// quad の星番号 [A, B, C, D](DIMQUADS = 3 なら D は u32::MAX)。範囲の外・星番号が壊れていれば None。
    pub fn quad_stars(&self, quad: u32) -> Option<[u32; 4]> {
        let dq = self.dimquads;
        let q = quad as usize;
        let row = self.quads_table().get(q * dq..q * dq + dq)?;
        let mut out = [u32::MAX; 4];
        for (k, &s) in row.iter().enumerate() {
            if s as usize >= self.nstars {
                return None;
            }
            out[k] = s;
        }
        Some(out)
    }

    /// quad のコード(実数に戻したもの)。三角形は [cx, cy, 0, 0]。範囲の外なら None。
    pub fn code(&self, quad: u32) -> Option<[f64; 4]> {
        let t = self.tree(&self.codes)?;
        let d = self.codes.ndim;
        let mut out = [0.0; 4];
        for (k, o) in out.iter_mut().enumerate().take(d) {
            *o = self.codes.to_real(k, t.data.get_u64(quad as usize * d + k)? as f64);
        }
        Some(out)
    }

    /// quad のコードの保存されている整数(検算用)。三角形は [cx, cy, 0, 0]。
    pub fn code_ints(&self, quad: u32) -> Option<[u64; 4]> {
        let t = self.tree(&self.codes)?;
        let d = self.codes.ndim;
        let mut out = [0; 4];
        for (k, o) in out.iter_mut().enumerate().take(d) {
            *o = t.data.get_u64(quad as usize * d + k)?;
        }
        Some(out)
    }

    /// 実数のコード → 木の単位(丸めない。検算用。round_ties_even すると保存値になるはず)。
    pub fn code_to_tree(&self, code: &[f64]) -> [f64; 4] {
        let mut out = [0.0; 4];
        for (k, o) in out.iter_mut().enumerate().take(self.codes.ndim.min(code.len())) {
            *o = self.codes.to_tree(k, code[k]);
        }
        out
    }

    /// 星の単位ベクトル(長さ 1 に正規化済み)。範囲の外なら None。
    pub fn star_xyz(&self, id: u32) -> Option<Vec3> {
        let t = self.tree(&self.stars)?;
        let i = id as usize * 3;
        Some(match t.data {
            Ints::U16(s) => star_unit(&self.stars, s.get(i..i + 3)?),
            Ints::U32(s) => star_unit(&self.stars, s.get(i..i + 3)?),
        })
    }

    /// 星の sweep(uint8。小さいほど早く選ばれた = そのあたりで明るい星)。範囲の外なら None。
    pub fn sweep(&self, id: u32) -> Option<u8> {
        self.bytes(self.sweep).get(id as usize).copied()
    }

    /// 星の等級(4100 系の MAG 列。表はビッグエンディアンの float32)。表が無い・0(欠測)・範囲の外なら None。
    pub fn mag(&self, id: u32) -> Option<f32> {
        let (o, row, col) = self.mag?;
        let at = o.checked_add((id as usize).checked_mul(row)?)?.checked_add(col)?;
        let b = self.mmap.get(at..at + 4)?;
        let v = f32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        (v != 0.0).then_some(v)
    }

    /// center から角距離 radius_rad 以内の星の番号(昇順)。判定は弦の長さ(正規化した星と中心の差 ≤ 2 sin(r/2))。
    pub fn stars_in_cone(&self, center: Vec3, radius_rad: f64) -> Vec<u32> {
        self.stars_in_cone_with(center, radius_rad, self.stars.prefilter_ok)
    }

    /// [`IndexData::stars_in_cone`] の本体(prefilter = 葉の前ふるいを使うか。試験で両方の答えを比べる)。
    fn stars_in_cone_with(&self, center: Vec3, radius_rad: f64, prefilter: bool) -> Vec<u32> {
        match self.cone_start(center, radius_rad) {
            ConeStart::Empty => Vec::new(),
            ConeStart::All => (0..self.nstars as u32).collect(),
            ConeStart::Tree { c, chord } => {
                let mut out = Vec::new();
                let _ = self.cone_each(&c, chord, prefilter, |id, _| {
                    out.push(id);
                    ControlFlow::Continue(())
                });
                // 葉は左から順なので並んでいるはずだが、念のため(並んでいれば並べ替えは 1 回の走査で済む)
                out.sort_unstable();
                out
            }
        }
    }

    /// center から角距離 radius_rad 以内に、accept(番号) が true になる星があるか。
    ///
    /// `stars_in_cone(center, radius_rad).into_iter().any(accept)` と同じ答え(同じ前置き・同じ判定。accept は番号の昇順に
    /// 呼ぶ)で、見つけた時点でやめ、番号の並びを作らない。accept は副作用の無い判定にすること(途中でやめるので
    /// 全部の星には呼ばれない)。半径が π 以上なら、木を見ずに 0 … nstars − 1 の全部に accept を回す(stars_in_cone と同じ)。
    pub fn any_star_in_cone(&self, center: Vec3, radius_rad: f64, mut accept: impl FnMut(u32) -> bool) -> bool {
        match self.cone_start(center, radius_rad) {
            ConeStart::Empty => false,
            ConeStart::All => (0..self.nstars as u32).any(accept),
            ConeStart::Tree { c, chord } => self
                .cone_each(&c, chord, self.stars.prefilter_ok, |id, _| {
                    if accept(id) { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
                })
                .is_break(),
        }
    }

    /// 円錐の問い合わせの前置き(stars_in_cone・any_star_in_cone で共通。式も順も Python 版と同じ)。
    fn cone_start(&self, center: Vec3, radius_rad: f64) -> ConeStart {
        let c = normalize(center);
        let r = radius_rad;
        // 半径が負・NaN、中心が NaN(長さ 0 のベクトルを含む)なら空(Python 版と同じ答え。木をたどらずに返す)
        if r.is_nan() || r < 0.0 || c.iter().any(|v| !v.is_finite()) {
            return ConeStart::Empty;
        }
        if r >= std::f64::consts::PI {
            return ConeStart::All;
        }
        if let (Some(tc), Some(tr)) = (self.tile_center, self.tile_radius_rad)
            && angle_between(c, tc) > tr + r
        {
            return ConeStart::Empty; // タイルに届かない
        }
        ConeStart::Tree { c, chord: 2.0 * (r / 2.0).sin() }
    }

    /// 星の木をたどって、弦 chord 以内の星を番号の昇順に f(番号, 単位ベクトル) で知らせる(表が読めなければ何もしない)。
    fn cone_each(
        &self,
        c: &Vec3,
        chord: f64,
        prefilter: bool,
        f: impl FnMut(u32, Vec3) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        let Some(t) = self.tree(&self.stars) else { return ControlFlow::Continue(()) };
        match (t.split, t.data) {
            (Ints::U16(s), Ints::U16(d)) => cone_visit(t.meta, t.lr, s, d, c, chord, prefilter, f),
            (Ints::U16(s), Ints::U32(d)) => cone_visit(t.meta, t.lr, s, d, c, chord, prefilter, f),
            (Ints::U32(s), Ints::U16(d)) => cone_visit(t.meta, t.lr, s, d, c, chord, prefilter, f),
            (Ints::U32(s), Ints::U32(d)) => cone_visit(t.meta, t.lr, s, d, c, chord, prefilter, f),
        }
    }
}

/// 円錐の問い合わせの前置きの結果。
enum ConeStart {
    /// 答えは空(半径が負・NaN、中心がおかしい、タイルに届かない)
    Empty,
    /// 半径が π 以上: 0 … nstars − 1 の全部(木を見ない。Python 版と同じ)
    All,
    /// 木をたどる(正規化した中心と弦の長さ)
    Tree { c: Vec3, chord: f64 },
}

/// 問い合わせごとの結果(quad の番号の昇順)を、問い合わせの順に返す。
fn run_ball<const D: usize>(t: &Tree<'_>, queries: &[f64], radius: f64) -> Vec<Vec<u32>> {
    match (t.split, t.data) {
        (Ints::U16(s), Ints::U16(d)) => run_ball_typed::<u16, u16, D>(t, s, d, queries, radius),
        (Ints::U16(s), Ints::U32(d)) => run_ball_typed::<u16, u32, D>(t, s, d, queries, radius),
        (Ints::U32(s), Ints::U16(d)) => run_ball_typed::<u32, u16, D>(t, s, d, queries, radius),
        (Ints::U32(s), Ints::U32(d)) => run_ball_typed::<u32, u32, D>(t, s, d, queries, radius),
    }
}

fn run_ball_typed<S: KdInt, T: KdInt, const D: usize>(
    t: &Tree<'_>,
    split: &[S],
    data: &[T],
    queries: &[f64],
    radius: f64,
) -> Vec<Vec<u32>> {
    let meta = t.meta;
    let lr = t.lr;
    // 量子化(D × 0.25 目盛り²)の分を広げる(Python 版と同じ。候補が増えるだけで答えは変わらない)
    let r_t = ((radius * meta.scale).powi(2) + 0.25 * D as f64).sqrt() * (1.0 + 1e-9) + 1e-6;
    let one = |stack: &mut Vec<_>, q: &[f64]| -> Vec<u32> {
        let mut qq = [0.0; D];
        qq.copy_from_slice(q);
        let mut out = Vec::new();
        ball_query::<S, T, D>(meta, lr, split, data, &qq, radius, r_t, stack, &mut out);
        out
    };
    let cap = 2 * meta.levels + 2;
    if queries.len() / D < PAR_MIN_QUERIES {
        let mut stack = Vec::with_capacity(cap);
        queries.chunks_exact(D).map(|q| one(&mut stack, q)).collect()
    } else {
        queries
            .par_chunks_exact(D)
            .map_init(|| Vec::with_capacity(cap), |stack, q| one(stack, q))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::KdType;

    /// 試験用の乱数(xorshift)。
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }

        fn below(&mut self, n: usize) -> usize {
            ((self.next() * n as f64) as usize).min(n.saturating_sub(1))
        }

        fn unit(&mut self) -> Vec3 {
            loop {
                let v = [self.next() * 2.0 - 1.0, self.next() * 2.0 - 1.0, self.next() * 2.0 - 1.0];
                let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                if n > 0.1 && n <= 1.0 {
                    return [v[0] / n, v[1] / n, v[2] / n];
                }
            }
        }
    }

    /// 星図データの索引 1 本(ZAHYOU_INDEX_DIR か C:\AstrometryData。無ければ None で試験を飛ばす。表を開くのは Windows だけ)。
    fn index(tag: &str) -> Option<IndexFile> {
        if !cfg!(windows) {
            return None;
        }
        let dir = std::env::var_os("ZAHYOU_INDEX_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\AstrometryData"));
        let p = dir.join(format!("index-{tag}.fits"));
        if !p.is_file() {
            eprintln!("SKIP: {} が無い", p.display());
            return None;
        }
        IndexFile::open(p).ok()
    }

    /// 星 v から方向 dir へ角距離 r の点。
    fn point_at(v: Vec3, dir: Vec3, r: f64) -> Vec3 {
        let wv = dir[0] * v[0] + dir[1] * v[1] + dir[2] * v[2];
        let t = normalize([dir[0] - wv * v[0], dir[1] - wv * v[1], dir[2] - wv * v[2]]);
        let (s, c) = r.sin_cos();
        normalize([v[0] * c + t[0] * s, v[1] * c + t[1] * s, v[2] * c + t[2] * s])
    }

    /// 円錐 1 回について: 前ふるいあり・なしの stars_in_cone が同じで、any_star_in_cone が stars_in_cone の any と同じ。
    /// 戻り値は円錐の中の星の数。
    fn check_cone(d: &IndexData, c: Vec3, r: f64, label: &str) -> usize {
        let with = d.stars_in_cone_with(c, r, true);
        let without = d.stars_in_cone_with(c, r, false);
        assert_eq!(with, without, "{label}: 前ふるいで答えが変わった c={c:?} r={r}");
        assert_eq!(d.stars_in_cone(c, r), with, "{label}");
        for m in [1u32, 2, 3, 7] {
            let p = |id: u32| id % m == m - 1;
            assert_eq!(d.any_star_in_cone(c, r, p), with.iter().any(|&id| p(id)), "{label}: any m={m} c={c:?} r={r}");
        }
        // accept は番号の昇順に呼ばれ、true を返した所でやめる
        let mut seen = Vec::new();
        let hit = d.any_star_in_cone(c, r, |id| {
            seen.push(id);
            seen.len() == 3
        });
        assert_eq!(hit, with.len() >= 3, "{label}");
        assert_eq!(seen, with[..with.len().min(3)], "{label}: 呼ぶ順");
        with.len()
    }

    /// 本物の索引で、乱数の円錐(星のまわり・星がちょうど境目に来る中心・半径 0〜葉の大きさ・端の値)について、
    /// 前ふるいあり・なしの答えが同じで、any_star_in_cone が stars_in_cone の any と同じ。
    #[test]
    fn cone_prefilter_and_any_match_on_real_indexes() {
        let mut rows = Vec::new();
        for tag in ["4119", "4115", "4111", "4107", "5206-00", "5202-00"] {
            let Some(f) = index(tag) else { continue };
            let d = f.data().unwrap();
            // 本物の索引は前ふるいの範囲に入る(入らなければ速くならないだけだが、ここで気づけるように)
            assert!(d.stars.prefilter_ok && d.codes.prefilter_ok, "{tag}: minval={:?} scale={}", d.stars.minval, d.stars.scale);
            let ns = d.nstars();
            let n = 25_000;
            let mut rng = Rng(0xD1B5_4A32_D192_ED03 ^ ns as u64);
            let queries: Vec<(Vec3, f64)> = (0..n)
                .map(|k| {
                    let s = d.star_xyz(rng.below(ns) as u32).unwrap();
                    // 半径: 0〜2e-3(葉の大きさのあたり)・0〜2e-2・ごく小さい値・0〜5e-3
                    let r = match k % 4 {
                        0 => rng.next() * 2e-3,
                        1 => rng.next() * 2e-2,
                        2 => rng.next() * 1e-6,
                        _ => rng.next() * 5e-3,
                    };
                    let c = if k % 3 == 0 {
                        // 星がちょうど境目に来る中心(角距離 r × (1 ± 1e-12 程度))
                        point_at(s, rng.unit(), r * (1.0 + (rng.next() - 0.5) * 2e-12))
                    } else {
                        point_at(s, rng.unit(), rng.next() * r)
                    };
                    (c, r)
                })
                .collect();
            let counts: Vec<usize> = queries.into_par_iter().map(|(c, r)| check_cone(&d, c, r, tag)).collect();
            let found: usize = counts.iter().sum();
            let nonempty = counts.iter().filter(|&&m| m > 0).count();
            // 端: 半径 0・ごく小さい・負・NaN・π 以上、中心が長さ 0
            let s7 = d.star_xyz(7).unwrap();
            for r in [0.0, 1e-9, -1.0, f64::NAN, std::f64::consts::PI, 4.0] {
                check_cone(&d, s7, r, tag);
            }
            check_cone(&d, [0.0, 0.0, 0.0], 0.1, tag);
            assert!(!d.any_star_in_cone(s7, f64::NAN, |_| true));
            assert!(d.any_star_in_cone(s7, std::f64::consts::PI, |id| id == ns as u32 - 1));
            assert!(nonempty > n / 4, "{tag}: 星の入る円錐が少ない({nonempty}/{n})");
            rows.push(format!("{tag} {n} 回 {found} 星"));
        }
        eprintln!("円錐の前ふるいあり・なし・any が一致: {}", rows.join("、"));
    }

    /// 星の値を書き換えた(単位ベクトルでない)索引の写しでも、前ふるいあり・なしの答えが同じ。
    /// 半分の長さにした星は木の単位では遠いが、正規化すると元の向きに戻るので元の向きの円錐に入る
    /// (ノルムの条件があるので前ふるいが捨てないことを確かめる)。ノルムの許容の境目の前後に伸び縮みした星も混ぜる。
    #[test]
    fn cone_prefilter_on_non_unit_stars() {
        let Some(src) = index("4115") else { return };
        let d0 = src.data().unwrap();
        let (off, len) = d0.stars.data;
        let meta = d0.stars.clone();
        let ns = d0.nstars();
        let orig: Vec<Vec3> = (0..ns as u32).map(|i| d0.star_xyz(i).unwrap()).collect();
        let mut bytes = std::fs::read(&src.path).unwrap();
        drop(d0);
        src.close();
        assert!(off + len <= bytes.len());
        let full = meta.full;
        let to_int = |k: usize, x: f64| ((x - meta.minval[k]) * meta.scale).round();
        let mut changed = Vec::new();
        for i in (0..ns).step_by(5) {
            // 1/5 の星を、半分・1.6 倍・ノルムの許容(2 目盛り)の外(+2.5 目盛り)と内(−1.5 目盛り)に伸び縮み
            let fct = match (i / 5) % 4 {
                0 => 0.5,
                1 => 1.6,
                2 => 1.0 + 2.5 / meta.scale,
                _ => 1.0 - 1.5 / meta.scale,
            };
            let x = orig[i];
            let ints: Vec<f64> = (0..3).map(|k| to_int(k, x[k] * fct)).collect();
            if ints.iter().any(|v| !(*v > 0.0 && *v < full)) {
                continue; // 範囲の表の外に出るものは書き換えない
            }
            for (k, v) in ints.iter().enumerate() {
                match meta.data_type {
                    KdType::U32 => {
                        let at = off + (i * 3 + k) * 4;
                        bytes[at..at + 4].copy_from_slice(&(*v as u32).to_le_bytes());
                    }
                    KdType::U16 => {
                        let at = off + (i * 3 + k) * 2;
                        bytes[at..at + 2].copy_from_slice(&(*v as u16).to_le_bytes());
                    }
                }
            }
            changed.push((i, fct));
        }
        let nhalf = changed.iter().filter(|c| c.1 == 0.5).count();
        assert!(changed.len() > 1000 && nhalf > 200, "書き換えた星が少ない({})", changed.len());
        let dir = std::env::temp_dir().join(format!("zahyou-index-prefilter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("index-4115.fits");
        std::fs::write(&p, &bytes).unwrap();
        let f = IndexFile::open(&p).unwrap();
        let d = f.data().unwrap();
        assert!(d.stars.prefilter_ok);
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        let mut half_found = 0;
        for &(i, fct) in &changed {
            let x = orig[i];
            for r in [1e-6, 1e-4, rng.next() * 3e-3] {
                check_cone(&d, x, r, "書き換えた 4115");
            }
            // 正規化し直した向きは量子化(0.5 目盛り ÷ 半分の長さ)だけずれるので、その数倍の半径で探す
            if fct == 0.5 && d.stars_in_cone(x, (10.0 / meta.scale).max(1e-6)).contains(&(i as u32)) {
                half_found += 1;
            }
            let c = point_at(x, rng.unit(), rng.next() * 2e-3);
            check_cone(&d, c, rng.next() * 3e-3, "書き換えた 4115");
        }
        assert_eq!(half_found, nhalf, "半分の長さにした星が元の向きの円錐に入らない");
        drop(d);
        f.close();
        let _ = std::fs::remove_dir_all(&dir);
        eprintln!("星を書き換えた索引({} 星)で前ふるいあり・なしが一致(半分の長さの星 {nhalf} 個が元の向きで見つかった)", changed.len());
    }
}
