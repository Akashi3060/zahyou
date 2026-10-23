//! 索引ファイル 1 本(Python 版 `SvIndexFile`)。ヘッダーは開くときに読み、表は使うときに mmap する。

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use memmap2::Mmap;
use rayon::prelude::*;
use zahyou_wcs::{angle_between, normalize, Vec3, ARCSEC_PER_RAD};

use crate::healpix::{healpix_xy_center, healpix_xy_radius};
use crate::kdtree::{ball_query, cone_query, star_unit, Ints, KdInt, Tree, TreeMeta};
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
        let c = normalize(center);
        let r = radius_rad;
        // 半径が負・NaN、中心が NaN(長さ 0 のベクトルを含む)なら空(Python 版と同じ答え。木をたどらずに返す)
        if r.is_nan() || r < 0.0 || c.iter().any(|v| !v.is_finite()) {
            return Vec::new();
        }
        if r >= std::f64::consts::PI {
            return (0..self.nstars as u32).collect();
        }
        if let (Some(tc), Some(tr)) = (self.tile_center, self.tile_radius_rad)
            && angle_between(c, tc) > tr + r
        {
            return Vec::new(); // タイルに届かない
        }
        let chord = 2.0 * (r / 2.0).sin();
        let Some(t) = self.tree(&self.stars) else { return Vec::new() };
        let mut out = Vec::new();
        match (t.split, t.data) {
            (Ints::U16(s), Ints::U16(d)) => cone_query(t.meta, t.lr, s, d, &c, chord, &mut out),
            (Ints::U16(s), Ints::U32(d)) => cone_query(t.meta, t.lr, s, d, &c, chord, &mut out),
            (Ints::U32(s), Ints::U16(d)) => cone_query(t.meta, t.lr, s, d, &c, chord, &mut out),
            (Ints::U32(s), Ints::U32(d)) => cone_query(t.meta, t.lr, s, d, &c, chord, &mut out),
        }
        out.sort_unstable();
        out
    }
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
