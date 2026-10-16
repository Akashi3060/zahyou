//! 1 回の探索で使い回す画像の側の情報(Python の `_SvField`)。

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::{Arc, OnceLock, RwLock};

use crate::Params;
use crate::parts::{box_order, field_order};

/// 有効面積を数える画像の区切り(10 × 10。Python の SV_AREA_GRID)。
pub(crate) const AREA_GRID: usize = 10;

/// 検証の並びを錠なしで引く表の大きさ(箱の数 nw・nh がこれ未満のもの)。
const SMALL_BOXES: usize = 32;

/// 検証の一様化の並び(箱 nw × nh)と、その逆引き。
pub(crate) struct VOrder {
    /// 並び(vxy の行)
    pub order: Vec<usize>,
    /// vxy の行 → 並びの中の位置
    pub pos: Vec<usize>,
}

/// 画像の側の情報。
pub(crate) struct Field {
    /// 入力の星(0 始まりの画素)
    pub xy: Vec<[f64; 2]>,
    pub w: f64,
    pub h: f64,
    pub n: usize,
    /// 明るい順 → 一様化した順(入力の添字)
    pub order: Vec<usize>,
    /// quad 作りに使う星の数
    pub nq: usize,
    /// quad 作りに使う星の位置(order の先頭 nq 個)
    pub p: Vec<[f64; 2]>,
    /// 検証の星(入力の添字、明るい順)と位置、入力の添字 → vxy の行(無ければ −1)
    pub vidx: Vec<usize>,
    pub vxy: Vec<[f64; 2]>,
    pub vpos: Vec<i64>,
    /// 有効面積を数える格子の点
    pub grid: Vec<[f64; 2]>,
    /// 画像の中心 ((W−1)/2, (H−1)/2) と、対角の半分
    pub center: [f64; 2],
    pub half_diag: f64,
    /// 検証の並び(箱 nw × nh)。小さい箱の数は錠なしの表(仮説ごとに引くので、並列で取り合わないように)、
    /// 大きいものは錠つきの表に覚える
    vorder_small: Vec<OnceLock<VOrder>>,
    vorder: RwLock<HashMap<(usize, usize), Arc<VOrder>>>,
}

/// [`Field::verify_order`] の戻り値(表から借りたものか、錠つきの表から分けてもらったもの)。
pub(crate) enum VOrderRef<'a> {
    Borrowed(&'a VOrder),
    Shared(Arc<VOrder>),
}

impl Deref for VOrderRef<'_> {
    type Target = VOrder;
    fn deref(&self) -> &VOrder {
        match self {
            VOrderRef::Borrowed(v) => v,
            VOrderRef::Shared(v) => v,
        }
    }
}

impl Field {
    pub fn new(xy: &[[f64; 2]], flux: Option<&[f64]>, width: f64, height: f64, params: &Params) -> Field {
        let n = xy.len();
        let fo = field_order(xy, flux, width, height, params);
        let p: Vec<[f64; 2]> = fo.order[..fo.nq].iter().map(|&i| xy[i]).collect();
        let vxy: Vec<[f64; 2]> = fo.verify.iter().map(|&i| xy[i]).collect();
        let mut vpos = vec![-1i64; n];
        for (k, &i) in fo.verify.iter().enumerate() {
            vpos[i] = k as i64;
        }
        let g = AREA_GRID as f64;
        let mut grid = Vec::with_capacity(AREA_GRID * AREA_GRID);
        for iy in 0..AREA_GRID {
            for ix in 0..AREA_GRID {
                grid.push([(ix as f64 + 0.5) / g * width - 0.5, (iy as f64 + 0.5) / g * height - 0.5]);
            }
        }
        Field {
            xy: xy.to_vec(),
            w: width,
            h: height,
            n,
            order: fo.order,
            nq: fo.nq,
            p,
            vidx: fo.verify,
            vxy,
            vpos,
            grid,
            center: [(width - 1.0) / 2.0, (height - 1.0) / 2.0],
            half_diag: 0.5 * width.hypot(height),
            vorder_small: (0..SMALL_BOXES * SMALL_BOXES).map(|_| OnceLock::new()).collect(),
            vorder: RwLock::new(HashMap::new()),
        }
    }

    /// 検証の一様化(箱 nw × nh から 1 個ずつ)をした、検証の星の並び(Python の `verify_order`)。
    pub fn verify_order(&self, nw: usize, nh: usize) -> VOrderRef<'_> {
        if nw < SMALL_BOXES && nh < SMALL_BOXES {
            return VOrderRef::Borrowed(self.vorder_small[nw * SMALL_BOXES + nh].get_or_init(|| self.make_vorder(nw, nh)));
        }
        let key = (nw, nh);
        if let Ok(m) = self.vorder.read()
            && let Some(v) = m.get(&key)
        {
            return VOrderRef::Shared(Arc::clone(v));
        }
        let v = Arc::new(self.make_vorder(nw, nh));
        VOrderRef::Shared(match self.vorder.write() {
            Ok(mut m) => Arc::clone(m.entry(key).or_insert(v)),
            Err(_) => v,
        })
    }

    fn make_vorder(&self, nw: usize, nh: usize) -> VOrder {
        let order = box_order(&self.vxy, self.w, self.h, nw, nh);
        let mut pos = vec![0usize; order.len()];
        for (k, &i) in order.iter().enumerate() {
            pos[i] = k;
        }
        VOrder { order, pos }
    }
}
