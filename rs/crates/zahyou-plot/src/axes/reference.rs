//! 旧版の格子の作り方(点ごとに写す。SIP の打ち切りの判定で毎回 `tan_only()` を複製し、目盛りの向きの点を
//! 座標ごとに写し直す)。新版(まとめて写す)と Grid がビット単位で同じことを確かめる試験の参照で、試験のときだけ
//! 組み込む。中身は変更前の axes.rs から、名前に `Old`・`_old` を付けただけで写してある(書き換えない)。

use super::*;

/// 画素 ↔ 天の写し(SIP は常にニュートン法で解く。astropy の all_world2pix と同じく近似を使わない)。
struct OldTrans<'a> {
    wcs: &'a Wcs,
    /// 画像の大きさ(遠すぎる点の打ち切りに使う)
    lim: f64,
    cx: f64,
    cy: f64,
}

impl<'a> OldTrans<'a> {
    fn new(wcs: &'a Wcs, width: usize, height: usize) -> Self {
        let (w, h) = (width as f64, height as f64);
        OldTrans { wcs, lim: 3.0 * w.max(h).max(1.0), cx: (w - 1.0) / 2.0, cy: (h - 1.0) / 2.0 }
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

/// `find_coordinate_range`(座標は経度・緯度の 2 つ。赤経の折り返しは 360°)。
fn coordinate_range_old(tr: &OldTrans, view: &View) -> [(f64, f64); 2] {
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

/// 枠の 1 辺の標本。
struct OldSpine {
    side: Side,
    /// 画像の座標
    data: Vec<[f64; 2]>,
    /// (赤経, 赤緯)
    world: Vec<[f64; 2]>,
    /// 各区間の内向きの法線の角度(画面、度)
    normal: Vec<f64>,
}

fn spines_old(tr: &OldTrans, view: &View) -> Vec<OldSpine> {
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
            OldSpine { side, data, world, normal }
        })
        .collect()
}

/// 1 つの座標の目盛り(`_update_ticks` と `_compute_ticks`)。
fn ticks_for_old(
    coord: usize,
    tick_values: &[f64],
    sp: &OldSpine,
    tr: &OldTrans,
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

/// 表示の範囲を指定して格子を作る(拡大した表示に描き直すとき)。
pub(super) fn grid_for_view_old(wcs: &Wcs, width: usize, height: usize, view: &View) -> Grid {
    let tr = OldTrans::new(wcs, width, height);
    let ranges = coordinate_range_old(&tr, view);
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
    let sps = spines_old(&tr, view);
    let mut raw: Vec<RawLabel> = Vec::new();
    for c in 0..2 {
        for sp in &sps {
            ticks_for_old(c, &grid.tick_values[c], sp, &tr, view, &mut grid.ticks, &mut raw);
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

/// Grid の全部の値を 1 行ずつの文字にしたもの(浮動小数は to_bits の 16 進、文字・辺・座標の番号はそのまま)。
fn flat(g: &Grid) -> Vec<String> {
    let f = |x: f64| format!("{:016x}", x.to_bits());
    let pts = |p: &[[f64; 2]]| p.iter().map(|q| format!("{},{}", f(q[0]), f(q[1]))).collect::<Vec<_>>().join(" ");
    let mut v = Vec::new();
    for (c, r) in g.ranges.iter().enumerate() {
        v.push(format!("range {c} {} {}", f(r.0), f(r.1)));
    }
    for c in 0..2 {
        v.push(format!("values {c} {}", g.tick_values[c].iter().map(|&x| f(x)).collect::<Vec<_>>().join(" ")));
        v.push(format!("spacing {c} {}", f(g.spacing[c])));
    }
    for l in &g.lines {
        v.push(format!("line {} {} {}", l.coord, f(l.world), pts(&l.points)));
    }
    for t in &g.ticks {
        v.push(format!("tick {} {:?} {} {} {}", t.coord, t.side, pts(&[t.pos]), f(t.angle_deg), f(t.world)));
    }
    for l in &g.labels {
        v.push(format!("label {} {:?} {} {} {}", l.coord, l.side, pts(&[l.pos]), l.text, f(l.world)));
    }
    for a in &g.axis_labels {
        v.push(format!("axis {} {:?} {}", a.coord, a.side, a.text));
    }
    v
}

/// 新版と旧版の Grid が同じ。違えば最初に食い違った行を出す。戻り値は (線の数, 目盛りの数)。
fn same_grid(wcs: &Wcs, w: usize, h: usize, view: &View, what: &str) -> (usize, usize) {
    let new = grid_for_view(wcs, w, h, view);
    let old = grid_for_view_old(wcs, w, h, view);
    let (a, b) = (flat(&new), flat(&old));
    if let Some(i) = (0..a.len().max(b.len())).find(|&i| a.get(i) != b.get(i)) {
        panic!("{what}: {i} 行目が違う\n新: {:?}\n旧: {:?}", a.get(i), b.get(i));
    }
    (new.lines.len(), new.ticks.len())
}

fn wcs_of(ra: f64, dec: f64, scale_arcsec: f64, rot_deg: f64, parity: f64, crpix: [f64; 2], sip: Option<Sip>) -> Wcs {
    let s = scale_arcsec / 3600.0;
    let t = rot_deg.to_radians();
    let cd = [[-s * t.cos(), s * parity * t.sin()], [s * t.sin(), s * parity * t.cos()]];
    Wcs::new(radec_to_xyz(ra, dec), crpix, cd, sip)
}

/// 3 次の SIP(画像の半径 half で amp 画素ほどゆがむ)。
fn sip3(amp: f64, half: f64) -> Sip {
    let m = 4;
    let mut a = vec![0.0; m * m];
    let mut b = vec![0.0; m * m];
    a[2 * m] = amp / half.powi(2);
    a[m + 1] = -0.6 * amp / half.powi(2);
    b[2] = 0.8 * amp / half.powi(2);
    a[3 * m] = 0.3 * amp / half.powi(3);
    b[3] = -0.4 * amp / half.powi(3);
    Sip::new(3, a, b).unwrap()
}

use zahyou_wcs::{Sip, fit_sip_inverse};

#[test]
fn batch_grid_matches_old() {
    // TAN・SIP(逆変換あり・なし・強いゆがみ)・極・赤経 0° のまたぎ・広い画角と、画像全体・拡大・縮小・移動・
    // 上下反転の表示で、格子の線・目盛り・文字・範囲が旧版と to_bits まで同じ
    let mut cases: Vec<(String, Wcs, usize, usize)> = Vec::new();
    let (w, h) = (600usize, 400usize);
    let half = (w as f64).hypot(h as f64) / 2.0;
    cases.push(("TAN".into(), wcs_of(83.8, -5.4, 2.0, 20.0, 1.0, [300.0, 200.0], None), w, h));
    let sip = wcs_of(200.0, 30.0, 3.0, 70.0, -1.0, [280.0, 215.0], Some(sip3(5.0, half)));
    cases.push(("SIP 逆変換なし".into(), sip.clone(), w, h));
    cases.push(("SIP 逆変換あり".into(), fit_sip_inverse(&sip, w as f64, h as f64, None).unwrap(), w, h));
    cases.push((
        "強い SIP".into(),
        wcs_of(200.0, 30.0, 3.0, 70.0, -1.0, [20.0, 15.0], Some(sip3(500.0, 25.0))),
        40,
        30,
    ));
    cases.push(("北極".into(), wcs_of(30.0, 89.99, 6.0, 25.0, 1.0, [250.0, 200.0], Some(sip3(3.0, 320.0))), 500, 400));
    cases.push(("南極".into(), wcs_of(30.0, -89.995, 6.0, 25.0, -1.0, [250.0, 200.0], None), 500, 400));
    cases.push((
        "極ちょうど".into(),
        wcs_of(0.0, 90.0, 20.0, 0.0, 1.0, [250.0, 200.0], Some(sip3(2.0, 320.0))),
        500,
        400,
    ));
    cases.push((
        "赤経 0°".into(),
        wcs_of(0.01, 20.0, 9.0, 10.0, 1.0, [250.0, 200.0], Some(sip3(4.0, 320.0))),
        500,
        400,
    ));
    cases.push((
        "広い画角".into(),
        wcs_of(120.0, 45.0, 60.0, 33.0, 1.0, [2000.0, 1500.0], Some(sip3(40.0, 2500.0))),
        4000,
        3000,
    ));
    // 壊れた WCS(NaN・特異な CD)
    cases.push((
        "NaN の WCS".into(),
        Wcs::tan([f64::NAN, 0.0, 1.0], [f64::NAN, 1.0], [[f64::NAN, 0.0], [0.0, 1e-4]]),
        40,
        30,
    ));
    cases.push(("特異な CD".into(), Wcs::tan(radec_to_xyz(10.0, 20.0), [5.0, 5.0], [[0.0, 0.0], [0.0, 0.0]]), 40, 30));
    let mut lines = 0;
    let mut ticks = 0;
    for (name, wcs, w, h) in &cases {
        let (wf, hf) = (*w as f64, *h as f64);
        let mut views = vec![View::image(*w, *h, false), View::image(*w, *h, true)];
        for (x, y) in [
            ((0.3 * wf, 0.45 * wf), (0.4 * hf, 0.5 * hf)),      // 拡大
            ((-3.0 * wf, 4.0 * wf), (-3.0 * hf, 4.0 * hf)),     // 縮小
            ((-20.0 * wf, 21.0 * wf), (-20.0 * hf, 21.0 * hf)), // 大きく縮小(SIP の打ち切りが効く)
            ((0.6 * wf, 1.6 * wf), (0.9 * hf, -0.1 * hf)),      // 移動して上下反転
            ((f64::NAN, wf), (0.0, hf)),                        // 壊れた表示の範囲
        ] {
            views.push(View::new(x, y));
        }
        for (k, v) in views.iter().enumerate() {
            let (l, t) = same_grid(wcs, *w, *h, v, &format!("{name} の表示 {k}"));
            lines += l;
            ticks += t;
        }
    }
    // 比べた中身が空ではない
    assert!(lines > 200 && ticks > 200, "線 {lines} 本・目盛り {ticks} 本");
}

#[test]
fn batch_trans_matches_old_points() {
    // 打ち切りの判定を通る点・通らない点・反対の半球・有限でない点を混ぜた並びで、まとめて写した値が
    // 1 点ずつ写した旧版と to_bits まで同じ(判定を通った点だけを詰めて解き、元の位置へ戻す所の確かめ)
    let (w, h) = (600usize, 400usize);
    let sip = wcs_of(200.0, 30.0, 3.0, 70.0, -1.0, [280.0, 215.0], Some(sip3(5.0, 360.0)));
    let inv = fit_sip_inverse(&sip, w as f64, h as f64, None).unwrap();
    let tan = sip.tan_only();
    let bits = |v: &[[f64; 2]]| v.iter().flat_map(|p| [p[0].to_bits(), p[1].to_bits()]).collect::<Vec<u64>>();
    // 接点から 0.001° 〜 約 170° まで、いろいろな向きに。間に有限でない値を挟む
    let mut world: Vec<[f64; 2]> = Vec::new();
    for i in 0..900 {
        let r = 0.001 * 1.0135f64.powi(i);
        let t = i as f64 * 2.399;
        let dec = (30.0 + r * t.sin()).clamp(-90.0, 90.0);
        world.push([200.0 + r * t.cos() / 30f64.to_radians().cos(), dec]);
        match i % 97 {
            0 => world.push([f64::NAN, 30.0]),
            1 => world.push([200.0, f64::INFINITY]),
            2 => world.push([f64::NEG_INFINITY, f64::NAN]),
            _ => {}
        }
    }
    let mut pix: Vec<[f64; 2]> = Vec::new();
    for i in 0..900 {
        let r = 1.0135f64.powi(i);
        let t = i as f64 * 2.399;
        pix.push([300.0 + r * t.cos(), 200.0 + r * t.sin()]);
        if i % 89 == 0 {
            pix.push([f64::NAN, 1.0]);
            pix.push([2.0, f64::NEG_INFINITY]);
        }
    }
    for wcs in [&sip, &inv, &tan] {
        let new = Trans::new(wcs, w, h);
        let old = OldTrans::new(wcs, w, h);
        let a = new.w2p_many(&world);
        let b: Vec<[f64; 2]> = world.iter().map(|p| old.w2p(p[0], p[1])).map(|(x, y)| [x, y]).collect();
        assert_eq!(bits(&a), bits(&b), "w2p");
        let a = new.p2w_many(&pix);
        let b: Vec<[f64; 2]> = pix.iter().map(|p| old.p2w(p[0], p[1])).map(|(x, y)| [x, y]).collect();
        assert_eq!(bits(&a), bits(&b), "p2w");
        // 空の並び
        assert!(new.w2p_many(&[]).is_empty() && new.p2w_many(&[]).is_empty());
    }
    // SIP のとき、打ち切りの判定で落ちる点(TAN では写せるが遠い点)と、通って解ける点の両方がある
    let tr = Trans::new(&sip, w, h);
    let (mut cut, mut kept) = (0, 0);
    for p in world.iter().filter(|p| finite2(p)) {
        match tan.radec_to_pix(p[0], p[1]) {
            Some((x, y)) if (x - tr.cx).abs() < tr.lim && (y - tr.cy).abs() < tr.lim => kept += 1,
            Some(_) => cut += 1,
            None => {}
        }
    }
    assert!(cut > 50 && kept > 50, "落ちる点 {cut}・通る点 {kept}");
}
