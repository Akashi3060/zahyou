//! 照合データを使わない試験: 壊れた入力でパニックしない・極と赤経 0h の折り返し・描いた PNG の向きと配色。

use std::path::PathBuf;

use zahyou_image::{Image2D, Source};
use zahyou_plot::*;
use zahyou_wcs::{Sip, Wcs, fit_sip_inverse, radec_to_xyz};

/// 整数の擬似乱数(試験を毎回同じにする)。
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64) / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.next()
    }
}

fn image(w: usize, h: usize, rng: &mut Rng) -> Image2D {
    let data = (0..w * h)
        .map(|_| (1000.0 + rng.range(-30.0, 30.0) + if rng.next() < 0.01 { 5000.0 } else { 0.0 }) as f32)
        .collect();
    Image2D::new(w, h, data).unwrap()
}

fn wcs(ra: f64, dec: f64, scale_arcsec: f64, rot_deg: f64, parity: f64, crpix: [f64; 2], sip: Option<Sip>) -> Wcs {
    let s = scale_arcsec / 3600.0;
    let t = rot_deg.to_radians();
    let cd = [[-s * t.cos(), s * parity * t.sin()], [s * t.sin(), s * parity * t.cos()]];
    Wcs::new(radec_to_xyz(ra, dec), crpix, cd, sip)
}

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

/// 表示リストと目盛りの座標が全部有限で、格子の線は枠の中に収まっている。
fn check_figure(fig: &Figure, what: &str) {
    assert_eq!(fig.rgba.len(), fig.width * fig.height * 4, "{what}: RGBA の大きさ");
    let (xmin, xmax, ymin, ymax) =
        (-0.5 - 1e-9, fig.width as f64 - 0.5 + 1e-9, -0.5 - 1e-9, fig.height as f64 - 0.5 + 1e-9);
    for p in &fig.prims {
        match p {
            Prim::Polyline { points, .. } => {
                assert!(points.len() >= 2, "{what}: 点が 1 つの線");
                for q in points {
                    assert!(q[0].is_finite() && q[1].is_finite(), "{what}: 線に NaN");
                    assert!(q[0] >= xmin && q[0] <= xmax && q[1] >= ymin && q[1] <= ymax, "{what}: 線が枠の外 {q:?}");
                }
            }
            Prim::Circle { center, .. } | Prim::Cross { center, .. } => {
                assert!(center[0].is_finite() && center[1].is_finite(), "{what}: 印に NaN")
            }
            Prim::Arrow { from, to, .. } => {
                assert!(from.iter().chain(to).all(|v| v.is_finite()), "{what}: 矢印に NaN");
                // 矢印の先は枠の中(画面外の目標は縁の 0.97 倍で止める)
                assert!(to[0] >= xmin && to[0] <= xmax && to[1] >= ymin && to[1] <= ymax, "{what}: 矢印の先が枠の外");
            }
            Prim::Text { anchor, .. } => assert!(anchor[0].is_finite() && anchor[1].is_finite()),
        }
    }
    for t in &fig.ticks {
        assert!(t.pos[0].is_finite() && t.pos[1].is_finite() && t.angle_deg.is_finite(), "{what}: 目盛りに NaN");
    }
    for l in &fig.tick_labels {
        assert!(!l.text.is_empty(), "{what}: 空の目盛りの文字");
    }
}

/// 配置: 軸・目盛りの文字・軸名・題が図の中に収まり、題は軸にも目盛りの文字にも重ならない。
fn check_layout(fig: &Figure, fonts: Option<&Fonts>, what: &str) {
    let opts = RenderOptions { width: 800, ..Default::default() };
    let l = layout_with(fig, &opts, fonts);
    let (w, h) = (l.size.0 as f64, l.size.1 as f64);
    let (bw, bh) = l.axes.size();
    let inside = |b: [f64; 4], name: &str| {
        assert!(b.iter().all(|v| v.is_finite()), "{what}: {name} に NaN {b:?}");
        assert!(
            b[0] >= -0.5 && b[1] >= -0.5 && b[2] <= w + 0.5 && b[3] <= h + 0.5,
            "{what}: {name} が図の外 {b:?} / {w}×{h}"
        );
    };
    inside([l.axes.x0, l.axes.y0, l.axes.x0 + bw, l.axes.y0 + bh], "軸");
    for b in &l.tick_labels {
        inside(*b, "目盛りの文字");
    }
    for a in &l.axis_labels {
        inside(a.2, "軸名");
    }
    let t = l.title.expect("題");
    inside(t, "題");
    assert!(t[3] <= l.axes.y0 + 1e-6, "{what}: 題 {t:?} が軸(上端 {})に重なる", l.axes.y0);
    for (b, tl) in l.tick_labels.iter().zip(&fig.tick_labels) {
        if tl.side == Side::Top {
            assert!(t[3] <= b[1] + 1e-6, "{what}: 題が上の目盛りの文字に重なる");
        }
    }
}

#[test]
fn broken_inputs_do_not_panic() {
    let mut rng = Rng(1);
    let w = wcs(83.8, -5.4, 2.0, 20.0, 1.0, [20.0, 15.0], None);
    // 空の画像・大きさの合わない画像
    let empty = Image2D { width: 0, height: 0, data: Vec::new() };
    assert_eq!(figure_original(&empty, &w, &[], None, false, true).unwrap_err(), PlotError::EmptyImage);
    assert_eq!(figure_north_up(&empty, &w, None).unwrap_err(), PlotError::EmptyImage);
    let bad = Image2D { width: 10, height: 10, data: vec![0.0; 7] };
    assert!(matches!(figure_original(&bad, &w, &[], None, false, true), Err(PlotError::BadImage(_))));
    assert!(matches!(figure_north_up(&bad, &w, None), Err(PlotError::BadImage(_))));
    let img = image(40, 30, &mut rng);
    // 特異な CD・NaN の WCS
    let singular = Wcs::tan(radec_to_xyz(10.0, 20.0), [5.0, 5.0], [[0.0, 0.0], [0.0, 0.0]]);
    let f = figure_original(&img, &singular, &[], Some((10.0, 20.0)), true, true).unwrap();
    check_figure(&f, "特異な CD");
    assert!(figure_north_up(&img, &singular, Some((10.0, 20.0))).is_err());
    let nan = Wcs::tan([f64::NAN, 0.0, 1.0], [f64::NAN, 1.0], [[f64::NAN, 0.0], [0.0, 1e-4]]);
    let f = figure_original(&img, &nan, &[], Some((10.0, 20.0)), false, true).unwrap();
    check_figure(&f, "NaN の WCS");
    assert!(figure_north_up(&img, &nan, None).is_err());
    // 巨大な SIP(画像の中で何十画素もゆがむ)
    let wild = wcs(200.0, 30.0, 3.0, 70.0, -1.0, [20.0, 15.0], Some(sip3(500.0, 25.0)));
    let f = figure_original(&img, &wild, &[], Some((200.01, 30.01)), true, true).unwrap();
    check_figure(&f, "巨大な SIP");
    match figure_north_up(&img, &wild, Some((200.01, 30.01))) {
        Ok(f) => check_figure(&f, "巨大な SIP 北が上"),
        Err(e) => eprintln!("巨大な SIP の北が上: {e}"),
    }
    // NaN・∞ の目標と星
    let srcs = vec![
        Source { x: f64::NAN, y: 1.0, flux: 1.0, peak: 1.0, area: 3, elong: 1.0, fwhm: 2.0, snr: 9.0 },
        Source { x: 5.0, y: 6.0, flux: 1.0, peak: 1.0, area: 3, elong: 1.0, fwhm: 2.0, snr: 9.0 },
    ];
    for t in [Some((f64::NAN, 0.0)), Some((f64::INFINITY, 1.0)), Some((0.0, f64::NAN)), None] {
        let f = figure_original(&img, &w, &srcs, t, false, true).unwrap();
        check_figure(&f, "NaN の目標");
        assert!(!f.prims.iter().any(|p| matches!(p, Prim::Arrow { .. })));
        let f = figure_north_up(&img, &w, t).unwrap();
        check_figure(&f, "NaN の目標 北が上");
    }
    // 全部 NaN の画像: 伸張なし・全部透明
    let allnan = Image2D::new(8, 6, vec![f32::NAN; 48]).unwrap();
    let f = figure_original(&allnan, &w, &[], None, false, false).unwrap();
    assert!(f.norm.is_none() && f.rgba.iter().all(|&b| b == 0));
    render_rgba_with(&f, &RenderOptions::default(), None).unwrap();
    // 1 画素・細長い画像
    for (iw, ih) in [(1usize, 1usize), (1, 50), (300, 1), (2, 2)] {
        let im = image(iw, ih, &mut rng);
        let ww = wcs(10.0, 10.0, 5.0, 33.0, 1.0, [iw as f64 / 2.0, ih as f64 / 2.0], None);
        let f = figure_original(&im, &ww, &[], Some((10.001, 10.001)), true, true).unwrap();
        check_figure(&f, "小さい画像");
        render_png_with(&f, &RenderOptions { width: 300, ..Default::default() }, None).unwrap();
        let f = figure_north_up(&im, &ww, Some((10.001, 10.001))).unwrap();
        check_figure(&f, "小さい画像 北が上");
    }
    // おかしな描き出しの設定
    let f = figure_original(&img, &w, &[], None, false, true).unwrap();
    for o in [
        RenderOptions { dpi: 0.0, ..Default::default() },
        RenderOptions { dpi: f32::NAN, ..Default::default() },
        RenderOptions { width: 1, height: 1, ..Default::default() },
        RenderOptions { width: 100_000, height: 30, ..Default::default() },
    ] {
        let r = render_rgba_with(&f, &o, None).unwrap();
        assert!(r.width >= 1 && r.width <= 16384 && r.data.len() == (r.width * r.height * 4) as usize);
    }
    // フォントの無いフォルダー
    let dir = std::env::temp_dir();
    assert!(Fonts::from_dir(&dir.join("zahyou-plot-no-such-dir")).is_err());
}

#[test]
fn random_wcs_fuzz() {
    // 乱数の WCS(TAN・SIP、鏡像、極・赤経 0° のそば)と目標で、図 2 枚と PNG がパニックせずに作れる
    let mut rng = Rng(42);
    let specials = [(0.0, 90.0), (0.0, -90.0), (137.0, 89.9999), (359.9995, 0.0), (0.0004, -45.0), (250.0, -89.99)];
    for i in 0..60 {
        let (ra, dec) = if i < specials.len() {
            specials[i]
        } else {
            (rng.range(0.0, 360.0), rng.range(-1.0, 1.0f64).asin().to_degrees())
        };
        let (iw, ih) = (rng.range(5.0, 90.0) as usize, rng.range(5.0, 70.0) as usize);
        let img = image(iw, ih, &mut rng);
        let scale = 10f64.powf(rng.range(-0.5, 2.5));
        let sip = if i % 3 == 1 { Some(sip3(rng.range(0.1, 5.0), (iw as f64).hypot(ih as f64) / 2.0)) } else { None };
        let mut ww = wcs(
            ra,
            dec,
            scale,
            rng.range(0.0, 360.0),
            if i % 2 == 0 { 1.0 } else { -1.0 },
            [rng.range(0.0, iw as f64), rng.range(0.0, ih as f64)],
            sip,
        );
        if i % 6 == 4 {
            ww = fit_sip_inverse(&ww, iw as f64, ih as f64, None).unwrap_or(ww);
        }
        let c = ww.pix_to_radec((iw as f64 - 1.0) / 2.0, (ih as f64 - 1.0) / 2.0);
        let fov = scale * iw.max(ih) as f64 / 3600.0;
        let targets = [
            None,
            Some((c.0 + 0.1 * fov / c.1.to_radians().cos().max(0.01), c.1)),
            Some(((c.0 + 180.0) % 360.0, -c.1 * 0.3)),
            Some((c.0, (c.1 + 3.0 * fov).min(90.0))),
        ];
        for t in targets {
            let flip = rng.next() < 0.5;
            let f1 = figure_original(&img, &ww, &[], t, flip, true).unwrap();
            check_figure(&f1, &format!("乱数 {i} 図 1"));
            match figure_north_up(&img, &ww, t) {
                Ok(f2) => {
                    check_figure(&f2, &format!("乱数 {i} 図 2"));
                    assert!(f2.prims.iter().any(|p| matches!(p, Prim::Cross { .. })));
                }
                Err(e) => panic!("乱数 {i}: 北が上を作れない: {e}"),
            }
            if i % 10 == 0 {
                render_png_with(&f1, &RenderOptions { width: 400, ..Default::default() }, Fonts::system()).unwrap();
            }
            check_layout(&f1, Fonts::system(), &format!("乱数 {i} 図 1"));
            check_layout(&f1, None, &format!("乱数 {i} 図 1(フォントなし)"));
        }
    }
}

#[test]
fn poles_and_ra_wrap_grid() {
    // 天の北極・南極を含む画像: 赤経の線が極で集まり、赤緯の線は輪になる。目盛りの文字が出る
    for (dec, flip) in [(89.99, false), (-89.995, true)] {
        let ww = wcs(30.0, dec, 6.0, 25.0, 1.0, [250.0, 200.0], None);
        let g = wcs_grid(&ww, 500, 400, flip);
        assert!(g.ranges[0].1 - g.ranges[0].0 > 359.0, "極を含むと赤経は全周");
        assert!(g.tick_values[0].len() >= 5 && g.tick_values[1].len() >= 3);
        assert!(g.lines.iter().any(|l| l.coord == 0) && g.lines.iter().any(|l| l.coord == 1));
        assert!(!g.labels.is_empty() && g.axis_labels.len() == 2);
        // 赤経の線はどれも極(画像の中)の近くを通る
        let pole = ww.radec_to_pix(0.0, if dec > 0.0 { 90.0 } else { -90.0 }).unwrap();
        for l in g.lines.iter().filter(|l| l.coord == 0) {
            let d = l.points.iter().map(|p| (p[0] - pole.0).hypot(p[1] - pole.1)).fold(f64::INFINITY, f64::min);
            assert!(d < 3.0, "赤経 {} の線が極を通らない({d:.1} px)", l.world);
        }
    }
    // 赤経 0h をまたぐ画像: 目盛りの文字に 23ʰ と 0ʰ の両方が出て、値は [0, 360)
    let ww = wcs(0.01, 20.0, 9.0, 10.0, 1.0, [250.0, 200.0], None);
    let g = wcs_grid(&ww, 500, 400, false);
    assert!(g.ranges[0].0 < 0.0 && g.ranges[0].1 > 0.0, "−180〜180 で表す");
    let ra_labels: Vec<&TickLabel> = g.labels.iter().filter(|l| l.coord == 0).collect();
    assert!(ra_labels.iter().all(|l| (0.0..360.0).contains(&l.world)));
    let texts: Vec<String> = ra_labels.iter().map(|l| l.plain()).collect();
    assert!(texts.iter().any(|t| t.starts_with("23h")) && texts.iter().any(|t| t.starts_with("0h")), "{texts:?}");
    // 北が上の図も、極のそばで作れて北が上(画素の +y が北)
    let ww = wcs(10.0, 89.98, 20.0, 75.0, -1.0, [17.0, 13.0], None);
    let mut rng = Rng(5);
    let img = image(34, 26, &mut rng);
    let nu = reproject_north_up(&img, &ww).unwrap();
    let (cx, cy) = ((nu.image.width as f64 - 1.0) / 2.0, (nu.image.height as f64 - 1.0) / 2.0);
    let a = nu.wcs.pix_to_radec(cx, cy);
    let b = nu.wcs.pix_to_radec(cx, cy + 1.0);
    assert!(b.1 > a.1, "上へ行くと赤緯が増える");
}

#[test]
fn rendered_png_orientation_and_theme() {
    // 行 0〜5 だけ明るく、ほかは雑音の画像(行 0 の側が白)。flip_y でなければ白が画面の下、flip_y なら上に描かれる
    let (w, h) = (40usize, 30usize);
    let data: Vec<f32> = (0..w * h).map(|i| if i / w < 6 { 10000.0 } else { 0.0 } + (i % 7) as f32 * 0.3).collect();
    let img = Image2D::new(w, h, data).unwrap();
    let ww = wcs(120.0, 10.0, 3.0, 0.0, 1.0, [20.0, 15.0], None);
    for (flip, theme) in [(false, Theme::light()), (true, Theme::dark())] {
        let f = figure_original(&img, &ww, &[], None, flip, false).unwrap();
        let opts = RenderOptions { theme, ..Default::default() };
        let r = render_rgba(&f, &opts).unwrap();
        let bx = axes_box(&f, &opts);
        let px = |x: f64, y: f64| {
            let (sx, sy) = bx.to_screen(x, y);
            let i = (sy as usize * r.width as usize + sx as usize) * 4;
            r.data[i]
        };
        // 画像の行 3(白)と行 26(黒)。格子・印の無い所(x = 5)
        // (明るい行が 2 割あるので σ クリップで何も捨てられず、伸張の幅が広い。明暗の差だけを見る)
        assert!(px(5.0, 3.0) as i32 > px(5.0, 26.0) as i32 + 40, "flip_y={flip}: {} / {}", px(5.0, 3.0), px(5.0, 26.0));
        let (_, s_white) = bx.to_screen(5.0, 3.0);
        let (_, s_black) = bx.to_screen(5.0, 26.0);
        assert_eq!(s_white > s_black, !flip, "flip_y={flip}: 行 0 の側");
        // 四隅は図の地の色
        assert_eq!(&r.data[0..3], &[theme.fig_bg.r, theme.fig_bg.g, theme.fig_bg.b]);
        let png = render_png(&f, &opts).unwrap();
        let dec = tiny_skia_png_size(&png);
        assert_eq!(dec, (r.width, r.height));
    }
}

/// PNG の IHDR から大きさを読む(復号の依存を足さないため)。
fn tiny_skia_png_size(png: &[u8]) -> (u32, u32) {
    assert_eq!(&png[1..4], b"PNG");
    let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    (w, h)
}

#[test]
fn writes_sample_pngs_when_asked() {
    // ZAHYOU_PLOT_OUT=<フォルダー> を付けると、極と赤経 0h の図を書き出す(目で見るため)
    let Some(dir) = std::env::var_os("ZAHYOU_PLOT_OUT").map(PathBuf::from) else { return };
    let _ = std::fs::create_dir_all(&dir);
    let mut rng = Rng(9);
    let img = image(500, 400, &mut rng);
    for (name, ww, flip) in [
        ("pole", wcs(30.0, 89.99, 6.0, 25.0, 1.0, [250.0, 200.0], None), false),
        ("wrap", wcs(0.01, 20.0, 9.0, 10.0, 1.0, [250.0, 200.0], Some(sip3(2.0, 320.0))), true),
        ("rot90", wcs(120.0, -30.0, 3.0, 90.0, 1.0, [250.0, 200.0], None), false),
    ] {
        let c = ww.pix_to_radec(249.5, 199.5);
        let t = Some((c.0 + 0.3, c.1 - 0.05));
        let f1 = figure_original(&img, &ww, &[], t, flip, false).unwrap();
        let f2 = figure_north_up(&img, &ww, t).unwrap();
        save_png(&f1, &RenderOptions::default(), &dir.join(format!("{name}_original.png"))).unwrap();
        save_png(
            &f2,
            &RenderOptions { theme: Theme::dark(), ..Default::default() },
            &dir.join(format!("{name}_northup.png")),
        )
        .unwrap();
    }
}

#[test]
fn zoomed_view_render() {
    // 行 0〜5 だけ明るい画像。拡大した範囲でも、画像の行が画面の正しい所に描かれ、範囲の外は軸の地の色
    let (w, h) = (40usize, 30usize);
    let data: Vec<f32> = (0..w * h).map(|i| if i / w < 6 { 10000.0 } else { 0.0 } + (i % 7) as f32 * 0.3).collect();
    let img = Image2D::new(w, h, data).unwrap();
    let ww = wcs(120.0, 10.0, 3.0, 25.0, 1.0, [20.0, 15.0], Some(sip3(0.5, 25.0)));
    let c = ww.pix_to_radec(20.0, 15.0);
    for flip in [false, true] {
        let f = figure_original(&img, &ww, &[], Some((c.0 + 0.001, c.1)), flip, false).unwrap();
        let theme = Theme::light();
        let opts = RenderOptions { view: Some([10.0, 30.0, -0.5, 14.5]), theme, ..Default::default() };
        let l = layout_with(&f, &opts, Fonts::system());
        let (bw, bh) = l.axes.size();
        // 縦横比は表示の範囲(20 × 15)のまま
        assert!((bw / bh - 20.0 / 15.0).abs() < 1e-9, "flip_y={flip}: 縦横比 {bw} {bh}");
        let r = render_rgba(&f, &opts).unwrap();
        let px = |x: f64, y: f64| {
            let (sx, sy) = l.axes.to_screen(x, y);
            r.data[(sy as usize * r.width as usize + sx as usize) * 4]
        };
        assert!(px(12.0, 3.0) as i32 > px(12.0, 12.0) as i32 + 40, "flip_y={flip}: 明るい行");
        let (_, s_white) = l.axes.to_screen(12.0, 3.0);
        let (_, s_black) = l.axes.to_screen(12.0, 12.0);
        assert_eq!(s_white > s_black, !flip, "flip_y={flip}: 行 0 の側");
        // 目盛りの文字は、拡大した範囲で選び直したもの(図の全体の目盛りの文字とは別)
        assert!(!l.tick_labels.is_empty());
        // 画像の外だけを見る表示: 軸の中は地の色
        let out = RenderOptions { view: Some([100.0, 140.0, 100.0, 130.0]), theme, ..Default::default() };
        let lo = layout_with(&f, &out, None);
        let ro = render_rgba_with(&f, &out, None).unwrap();
        let (sx, sy) = lo.axes.to_screen(120.0, 115.0);
        let i = (sy as usize * ro.width as usize + sx as usize) * 4;
        assert_eq!(&ro.data[i..i + 3], &[theme.ax_bg.r, theme.ax_bg.g, theme.ax_bg.b]);
        // 壊れた範囲(NaN・逆向き・幅 0)は画像全体として扱う
        for v in
            [[f64::NAN, 1.0, 0.0, 1.0], [30.0, 10.0, 0.0, 10.0], [5.0, 5.0, 0.0, 1.0], [0.0, f64::INFINITY, 0.0, 1.0]]
        {
            let o = RenderOptions { view: Some(v), ..Default::default() };
            let a = layout_with(&f, &o, None).axes;
            let b = layout_with(&f, &RenderOptions::default(), None).axes;
            assert_eq!(a, b, "{v:?}");
            render_png_with(&f, &o, None).unwrap();
        }
        // 極端に小さい・大きい範囲でもパニックしない
        for v in [[10.0, 10.000001, 3.0, 3.000001], [-1e7, 1e7, -1e7, 1e7]] {
            let o = RenderOptions { view: Some(v), width: 400, ..Default::default() };
            render_png_with(&f, &o, Fonts::system()).unwrap();
        }
    }
}
