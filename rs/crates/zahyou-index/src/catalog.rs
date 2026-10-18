//! 星図データのフォルダー全体(Python 版 `SvIndexCatalog`)。

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use zahyou_wcs::{angle_between, normalize, Vec3, ARCSEC_PER_RAD};

use crate::file::IndexFile;
use crate::layout::{match_index_name, read_layout};
use crate::pyfmt::float_repr;

/// quad の AB の最小 = 0.1 × 画像の短辺(select の下限)。
pub const QUAD_MIN_FRAC: f64 = 0.1;

/// 星図データのフォルダー。index-*.fits を走査してヘッダーを検査する(データは使うときに mmap)。
#[derive(Debug)]
pub struct Catalog {
    pub index_dir: PathBuf,
    /// 使える索引(INDEXID・HEALPIX の順)
    pub files: Vec<IndexFile>,
    /// 使えないファイル [(パス, 理由)]
    pub problems: Vec<(PathBuf, String)>,
    /// INDEXID → files の番号(全天の系列は 1 本、5200 系はタイルの数だけ。HEALPIX の順)
    pub series: BTreeMap<u32, Vec<usize>>,
}

impl Catalog {
    /// フォルダーを走査する(ヘッダーの検査は並列。結果の順は名前の順と同じ)。
    pub fn scan(dir: impl AsRef<Path>) -> Catalog {
        let index_dir = dir.as_ref().to_path_buf();
        let mut cat = Catalog { index_dir: index_dir.clone(), files: Vec::new(), problems: Vec::new(), series: BTreeMap::new() };
        if !index_dir.is_dir() {
            cat.problems.push((index_dir, "フォルダーがありません".to_string()));
            return cat;
        }
        let rd = match fs::read_dir(&index_dir) {
            Ok(rd) => rd,
            Err(e) => {
                cat.problems.push((index_dir, format!("読めません({e})")));
                return cat;
            }
        };
        let mut names: Vec<String> = rd
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| match_index_name(n).is_some())
            .collect();
        names.sort();
        let checked: Vec<(PathBuf, Option<Result<IndexFile, String>>)> = names
            .par_iter()
            .map(|nm| {
                let p = index_dir.join(nm);
                if !p.is_file() {
                    return (p, None);
                }
                let r = read_layout(&p, None).map(|lay| IndexFile::from_layout(&p, lay)).map_err(|e| e.message());
                (p, Some(r))
            })
            .collect();

        let mut seen: HashMap<(u32, i32), PathBuf> = HashMap::new();
        let mut files: Vec<IndexFile> = Vec::new();
        for (p, r) in checked {
            match r {
                None => {}
                Some(Err(msg)) => cat.problems.push((p, msg)),
                Some(Ok(f)) => {
                    let key = (f.indexid, f.healpix);
                    if let Some(first) = seen.get(&key) {
                        let base = first.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                        cat.problems.push((p, format!("同じ索引が 2 つあります({base} と同じ中身)")));
                        continue;
                    }
                    seen.insert(key, p);
                    files.push(f);
                }
            }
        }
        files.sort_by_key(|f| (f.indexid, f.healpix));

        // 同じ系列の中で段の値がそろっていないファイルは外す(いちばん多い形に合わせる。
        // 同数のときは先に出てくる形。Python 版は set の順で決まる)
        type Sig = (usize, u64, u64, i64);
        let sig = |f: &IndexFile| -> Sig { (f.dimquads, f.scale_lo_rad.to_bits(), f.scale_hi_rad.to_bits(), f.hpnside) };
        let mut by_id: BTreeMap<u32, Vec<IndexFile>> = BTreeMap::new();
        for f in files {
            by_id.entry(f.indexid).or_default().push(f);
        }
        for (iid, fs) in by_id {
            let sigs: Vec<Sig> = fs.iter().map(sig).collect();
            let mut best = sigs[0];
            let mut best_n = 0;
            for s in &sigs {
                let n = sigs.iter().filter(|t| *t == s).count();
                if n > best_n {
                    best = *s;
                    best_n = n;
                }
            }
            let mut idx = Vec::new();
            for (f, s) in fs.into_iter().zip(sigs) {
                if s == best {
                    idx.push(cat.files.len());
                    cat.files.push(f);
                } else {
                    cat.problems.push((
                        f.path.clone(),
                        "同じ系列のほかのファイルと段の値(DIMQUADS・SCALE・HPNSIDE)が違います".to_string(),
                    ));
                }
            }
            cat.series.insert(iid, idx);
        }
        cat
    }

    /// 開いている索引の mmap を全部放す。
    pub fn close(&self) {
        for f in &self.files {
            f.close();
        }
    }

    /// 手持ちの段の quad の大きさの範囲 (lo, hi)(分角)。索引が無ければ None。
    pub fn coverage_arcmin(&self) -> Option<(f64, f64)> {
        let lo = self.files.iter().map(|f| f.scale_lo_rad).reduce(f64::min)?;
        let hi = self.files.iter().map(|f| f.scale_hi_rad).reduce(f64::max)?;
        Some((lo.to_degrees() * 60.0, hi.to_degrees() * 60.0))
    }

    /// 画像に使う索引を選んで、試す順に並べる(files の番号)。入力がおかしければ空。
    ///
    /// 理由が要るときは [`Catalog::try_select`]。
    pub fn select(&self, scale_lo: f64, scale_hi: f64, width: f64, height: f64, hint: Option<(Vec3, f64)>) -> Vec<usize> {
        self.try_select(scale_lo, scale_hi, width, height, hint).unwrap_or_default()
    }

    /// 画像に使う索引を選んで、試す順に並べる(files の番号)。
    ///
    /// scale_lo, scale_hi: 画素スケールの範囲(″/px)。width, height: 画像の大きさ(px)。
    /// quad の大きさ(px)が [0.1 × 短辺, 対角] に入りうる段 = [0.1·min(W,H)·scale_lo, hypot(W,H)·scale_hi](″)と
    /// [SCALE_L, SCALE_U] が重なる段を選ぶ。重なる段が無ければ最も近い段 1 つ。
    /// 順は「quad の大きさが画像の短辺の半分くらい(0.5·min(W,H)·√(scale_lo·scale_hi))になる段」から。
    /// ヒント(中心の単位ベクトルと半径 rad)があれば、タイルの系列はヒントの円(半径 + 画像の対角の半分)と
    /// 重なるタイルだけを、ヒントに近い順に。無ければ番号の順。
    pub fn try_select(
        &self,
        scale_lo: f64,
        scale_hi: f64,
        width: f64,
        height: f64,
        hint: Option<(Vec3, f64)>,
    ) -> Result<Vec<usize>, String> {
        let (s_lo, s_hi, w, h) = (scale_lo, scale_hi, width, height);
        if !(s_lo > 0.0 && s_hi >= s_lo && w > 0.0 && h > 0.0) {
            // 数の書き方は Python 版に合わせる(整数ならそのまま、そうでなければ repr)
            let num = |v: f64| if v.is_finite() && v.fract() == 0.0 && v.abs() < 1e15 { format!("{}", v as i64) } else { float_repr(v) };
            return Err(format!(
                "スケール {}〜{}″/px・大きさ {}×{} がおかしい",
                float_repr(scale_lo),
                float_repr(scale_hi),
                num(width),
                num(height)
            ));
        }
        let short = w.min(h);
        let diag = w.hypot(h);
        let qlo = QUAD_MIN_FRAC * short * s_lo / ARCSEC_PER_RAD;
        let qhi = diag * s_hi / ARCSEC_PER_RAD;
        let target = 0.5 * short * (s_lo * s_hi).sqrt() / ARCSEC_PER_RAD;
        let rng: Vec<(u32, f64, f64)> = self
            .series
            .iter()
            .filter_map(|(iid, fs)| {
                let f = &self.files[*fs.first()?];
                Some((*iid, f.scale_lo_rad, f.scale_hi_rad))
            })
            .collect();
        let mut ids: Vec<(u32, f64, f64)> = rng.iter().copied().filter(|(_, l, u)| *l <= qhi && *u >= qlo).collect();
        if ids.is_empty() && !rng.is_empty() {
            let gap = |l: f64, u: f64| if l > qhi { (l / qhi).ln() } else { (qlo / u).ln() };
            let best = rng
                .iter()
                .copied()
                .min_by(|a, b| (gap(a.1, a.2), a.0).partial_cmp(&(gap(b.1, b.2), b.0)).unwrap_or(std::cmp::Ordering::Equal));
            ids.extend(best);
        }
        let key = |(iid, l, u): &(u32, f64, f64)| ((((l * u).sqrt()) / target).ln().abs(), *iid);
        ids.sort_by(|a, b| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal));

        let hint = hint.map(|(c, r)| (normalize(c), r + 0.5 * diag * s_hi / ARCSEC_PER_RAD));
        let mut out = Vec::new();
        for (iid, _, _) in ids {
            let fs = &self.series[&iid];
            match hint {
                Some((c, reach)) if self.files[fs[0]].tile_center.is_some() => out.extend(self.near(fs, c, reach)),
                _ => out.extend(fs.iter().copied()),
            }
        }
        Ok(out)
    }

    /// fs のうち、中心 c・半径 reach の円と重なりうるもの(近い順。同じ距離は番号の順)。
    fn near(&self, fs: &[usize], c: Vec3, reach: f64) -> Vec<usize> {
        let mut hit: Vec<(f64, usize)> = fs
            .iter()
            .filter_map(|&i| {
                let f = &self.files[i];
                match (f.tile_center, f.tile_radius_rad) {
                    (Some(tc), Some(tr)) => {
                        let ang = angle_between(tc, c);
                        (ang <= tr + reach).then_some((ang, i))
                    }
                    _ => Some((0.0, i)), // 全天のファイルはどこにでも届く
                }
            })
            .collect();
        hit.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        hit.into_iter().map(|(_, i)| i).collect()
    }

    /// 系列 indexid のうち、円(中心 center、半径 radius_rad)と重なりうるファイル(近い順)。
    ///
    /// 全天の系列はそのファイル。検証でタイルの境界をまたぐ視野の星を集めるのに使う。
    pub fn tiles_near(&self, indexid: u32, center: Vec3, radius_rad: f64) -> Vec<usize> {
        let Some(fs) = self.series.get(&indexid) else { return Vec::new() };
        if fs.is_empty() || self.files[fs[0]].tile_center.is_none() {
            return fs.clone();
        }
        self.near(fs, normalize(center), radius_rad)
    }
}
