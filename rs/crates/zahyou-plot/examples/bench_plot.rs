//! 図の速さを測る: `cargo run --release -p zahyou-plot --example bench_plot -- [幅 高さ 回数]`
//!
//! 合成した画像(既定 6000×4000、星を散らした float32)と、30° 回して SIP 3 次を付けた WCS で、
//! 伸張・格子・図 1・北が上の描き直し・図 2・PNG の書き出し・拡大した表示の格子の時間(回数のうち最速)を出す。
//! ZAHYOU_BENCH_OUT=<フォルダー> を付けると 2 枚の PNG(明るい配色と暗い配色)を書き出す。
//! ZAHYOU_BENCH_DUMP=<フォルダー> を付けると、Python 版と比べるための入力(画像と WCS のカード)と、
//! 北が上の描き直しの結果(bench_northup.f32 と .json)を書き出す。
//! ZAHYOU_BENCH_CHECK=1 を付けると、灰色への速い写しが画素ごとの式と全画素で同じかを数える。

use std::time::Instant;

use zahyou_image::{Image2D, Source};
use zahyou_plot::{
    RenderOptions, Theme, View, axes_box, compute_norm, compute_norm_f64, figure_north_up, figure_original,
    figure_size, gray_rgba, grid_for_view, render_png, reproject_north_up, wcs_grid,
};
use zahyou_wcs::{Sip, Wcs, fit_sip_inverse, radec_to_xyz};

fn synth(w: usize, h: usize) -> Image2D {
    // 背景 + 雑音(整数の擬似乱数)+ 星(ガウス)
    let mut data = vec![0f32; w * h];
    let mut s: u64 = 12345;
    let mut rnd = || {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((s >> 33) as f64) / (1u64 << 31) as f64
    };
    for (i, v) in data.iter_mut().enumerate() {
        let (x, y) = ((i % w) as f32, (i / w) as f32);
        *v = 1000.0 + 0.002 * x + 0.001 * y + (rnd() as f32 - 0.5) * 40.0;
    }
    for _ in 0..3000 {
        let (cx, cy) = (rnd() * w as f64, rnd() * h as f64);
        let amp = 50.0 + rnd() * 20000.0;
        let r = 3i64;
        for dy in -r * 2..=r * 2 {
            for dx in -r * 2..=r * 2 {
                let (x, y) = (cx as i64 + dx, cy as i64 + dy);
                if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                    let d2 = ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)) / (2.0 * 1.5 * 1.5);
                    data[y as usize * w + x as usize] += (amp * (-d2).exp()) as f32;
                }
            }
        }
    }
    // NaN の帯(表示で透明になる所)
    for v in data.iter_mut().take(w * 3) {
        *v = f32::NAN;
    }
    Image2D::new(w, h, data).unwrap()
}

fn wcs_for(w: usize, h: usize) -> Wcs {
    let s = 1.2 / 3600.0;
    let t = 30f64.to_radians();
    let cd = [[-s * t.cos(), s * t.sin()], [s * t.sin(), s * t.cos()]];
    let m = 4;
    let mut a = vec![0.0; m * m];
    let mut b = vec![0.0; m * m];
    a[2 * m] = 2e-7; // u²
    a[m + 1] = -1e-7; // u v
    b[2] = 1.5e-7; // v²
    a[3 * m] = 1e-11;
    b[3] = -1e-11;
    let sip = Sip::new(3, a, b).unwrap();
    let wcs = Wcs::new(radec_to_xyz(83.82, -5.39), [w as f64 / 2.0 + 37.0, h as f64 / 2.0 - 21.0], cd, Some(sip));
    // 解析が返す WCS と同じく、SIP の逆変換 AP・BP を付ける(ZAHYOU_BENCH_NOINV=1 なら付けない)
    if std::env::var_os("ZAHYOU_BENCH_NOINV").is_some() {
        return wcs;
    }
    fit_sip_inverse(&wcs, w as f64, h as f64, None).unwrap_or(wcs)
}

fn best<T>(n: usize, mut f: impl FnMut() -> T) -> (f64, T) {
    let mut bt = f64::INFINITY;
    let mut out = None;
    for _ in 0..n.max(1) {
        let t0 = Instant::now();
        let r = f();
        bt = bt.min(t0.elapsed().as_secs_f64());
        out = Some(r);
    }
    (bt, out.unwrap())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let w: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(6000);
    let h: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4000);
    let reps: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(3);
    eprintln!("画像を作っています ({w}×{h}) …");
    let img = synth(w, h);
    let wcs = wcs_for(w, h);
    let sources: Vec<Source> = (0..400)
        .map(|i| Source {
            x: (i * 7919 % w) as f64,
            y: (i * 104729 % h) as f64,
            flux: 1.0,
            peak: 1.0,
            area: 9,
            elong: 1.0,
            fwhm: 3.0,
            snr: 10.0,
        })
        .collect();
    let center = wcs.pix_to_radec(w as f64 / 2.0, h as f64 / 2.0);
    let target = Some((center.0 + 0.05, center.1 + 0.03));
    println!("スレッド数 {}", rayon::current_num_threads());
    let (t, norm) = best(reps, || compute_norm(&img.data));
    println!("伸張の範囲 (_norm)            {:8.3} 秒  {:?}", t, norm);
    let (t, _) = best(reps, || gray_rgba(&img, norm.as_ref()));
    println!("8 bit の灰色へ               {:8.3} 秒", t);
    let (t, g) = best(reps, || wcs_grid(&wcs, w, h, true));
    println!("格子と目盛り                 {:8.3} 秒  線 {} 本・目盛り {} 本", t, g.lines.len(), g.ticks.len());
    let (t, f1) = best(reps, || figure_original(&img, &wcs, &sources, target, true, true).unwrap());
    println!("図 1(撮影時の向き)         {:8.3} 秒", t);
    let (t, nu) = best(reps, || reproject_north_up(&img, &wcs).unwrap());
    println!("北が上の描き直し             {:8.3} 秒  {}×{}", t, nu.image.width, nu.image.height);
    let (t, n2) = best(reps, || compute_norm_f64(&nu.image.data));
    println!("北が上の伸張の範囲(float64)  {:8.3} 秒  {:?}", t, n2);
    let (t, _) = best(reps, || wcs_grid(&nu.wcs, nu.image.width, nu.image.height, false));
    println!("北が上の格子と目盛り         {:8.3} 秒", t);
    let (t, f2) = best(reps, || figure_north_up(&img, &wcs, target).unwrap());
    println!("図 2(北が上、描き直し込み) {:8.3} 秒", t);
    let opts = RenderOptions::default();
    let (t1, p1) = best(reps, || render_png(&f1, &opts).unwrap());
    let (t2, p2) = best(reps, || render_png(&f2, &opts).unwrap());
    println!("PNG 図 1 / 図 2              {:8.3} / {:.3} 秒  ({} / {} バイト)", t1, t2, p1.len(), p2.len());
    for (name, f) in [("図 1", &f1), ("図 2", &f2)] {
        let (fw, fh) = figure_size(f, &opts);
        let b = axes_box(f, &opts);
        let (bw, bh) = b.size();
        println!(
            "{name} の配置: 図 {fw}×{fh} px、軸の矩形 x {:.1}〜{:.1}・y {:.1}〜{:.1}(左上が原点)",
            b.x0,
            b.x0 + bw,
            b.y0,
            b.y0 + bh
        );
    }
    let (tt, _) = best(reps, || {
        let a = figure_original(&img, &wcs, &sources, target, true, true).unwrap();
        let b = figure_north_up(&img, &wcs, target).unwrap();
        (render_png(&a, &opts).unwrap(), render_png(&b, &opts).unwrap())
    });
    println!("図 2 枚の作成と PNG(通し)  {:8.3} 秒", tt);
    // 画面でホイールで拡大したとき(UI のスレッドが毎フレーム作り直す)の格子。WCS は上と同じ SIP 3 次 + 逆変換
    let zoom = View::new((w as f64 * 0.42, w as f64 * 0.58), (h as f64 * 0.6, h as f64 * 0.45));
    let (t, g) = best(reps, || grid_for_view(&wcs, w, h, &zoom));
    println!("拡大した格子と目盛り(SIP)   {:8.3} 秒  線 {} 本・目盛り {} 本", t, g.lines.len(), g.ticks.len());
    if std::env::var_os("ZAHYOU_BENCH_CHECK").is_some() {
        // 灰色への速い写し(gray_rgba)が、画素ごとの式(Norm::gray8)と全画素で同じか
        let nu = reproject_north_up(&img, &wcs).unwrap();
        let n2 = compute_norm_f64(&nu.image.data);
        for (name, im, n) in [("図 1", &img, norm), ("図 2", &nu.image, n2)] {
            let Some(n) = n else { continue };
            let fast = gray_rgba(im, Some(&n));
            let bad = im
                .data
                .iter()
                .zip(fast.chunks_exact(4))
                .filter(|(v, px)| n.gray8(**v).map_or([0, 0, 0, 0], |g| [g, g, g, 255]) != **px)
                .count();
            println!("{name}: 灰色の速い写しと式の違い {bad} / {} 画素", im.data.len());
        }
    }
    if let Ok(dir) = std::env::var("ZAHYOU_BENCH_DUMP") {
        // Python 版と同じ入力で比べるため、画像(float32 の生データ、行優先)と WCS のカードを書き出す
        let dir = std::path::PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        let bytes: Vec<u8> = img.data.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(dir.join("bench_image.f32"), bytes).unwrap();
        let cards: Vec<serde_json::Value> = wcs
            .header_cards(w, h, &[])
            .into_iter()
            .map(|c| {
                let v = match c.value {
                    zahyou_wcs::CardValue::Int(i) => serde_json::json!(i),
                    zahyou_wcs::CardValue::Float(f) => serde_json::json!(f),
                    zahyou_wcs::CardValue::Str(s) => serde_json::json!(s),
                    zahyou_wcs::CardValue::Bool(b) => serde_json::json!(b),
                    zahyou_wcs::CardValue::None => serde_json::Value::Null,
                };
                serde_json::json!({"k": c.key, "v": v})
            })
            .collect();
        let meta = serde_json::json!({"width": w, "height": h, "cards": cards, "target": target});
        std::fs::write(dir.join("bench_wcs.json"), meta.to_string()).unwrap();
        // 北が上の描き直しの結果(float32 の生データ、行優先)と大きさ。Python の reproject_interp と比べるため
        let nu = reproject_north_up(&img, &wcs).unwrap();
        let bytes: Vec<u8> = nu.image.data.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(dir.join("bench_northup.f32"), bytes).unwrap();
        let (ra, dec) = nu.wcs.crval_radec();
        let info = serde_json::json!({
            "width": nu.image.width, "height": nu.image.height,
            "crpix0": nu.wcs.crpix, "crval": [ra, dec], "cd": nu.wcs.cd,
        });
        std::fs::write(dir.join("bench_northup.json"), info.to_string()).unwrap();
        eprintln!("入力を {} に書きました", dir.display());
    }
    if let Ok(dir) = std::env::var("ZAHYOU_BENCH_OUT") {
        let dir = std::path::PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        let dark = RenderOptions { theme: Theme::dark(), ..Default::default() };
        std::fs::write(dir.join("bench_original_light.png"), &p1).unwrap();
        std::fs::write(dir.join("bench_northup_light.png"), &p2).unwrap();
        std::fs::write(dir.join("bench_original_dark.png"), render_png(&f1, &dark).unwrap()).unwrap();
        std::fs::write(dir.join("bench_northup_dark.png"), render_png(&f2, &dark).unwrap()).unwrap();
        // 拡大した表示(ツールバーで拡大して保存したのと同じ。x 1000〜2500・y 500〜1500)
        let zoom = RenderOptions { view: Some([1000.0, 2500.0, 500.0, 1500.0]), ..Default::default() };
        std::fs::write(dir.join("bench_original_zoom.png"), render_png(&f1, &zoom).unwrap()).unwrap();
        eprintln!("PNG を {} に書きました", dir.display());
    }
}
