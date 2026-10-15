//! 速さの実測: 画像を読み、前処理と検出の時間を測って JSON で出す。
//!
//!     cargo run --release -p zahyou-image --example bench_image -- <画像> [回数]
//!
//! 6000×4000 の合成画像は `python rs/testdata/gen_image.py --big <DIR>` で作る(Python の時間も
//! `<DIR>\big_expected.json` に入る)。各段の時間は `回数` 回のうち最も速いもの。

use std::time::Instant;

use zahyou_image::{DetectOpts, detect_sources, detect_sources_deep, load_image_any, preprocess};

fn best<T>(n: usize, mut f: impl FnMut() -> T) -> (f64, T) {
    let mut best = f64::INFINITY;
    let mut out = None;
    for _ in 0..n.max(1) {
        let t = Instant::now();
        let r = f();
        best = best.min(t.elapsed().as_secs_f64());
        out = Some(r);
    }
    (best, out.expect("1 回は回る"))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("使い方: bench <画像> [回数]");
        std::process::exit(2);
    };
    let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(3);
    let (t_load, loaded) = best(n, || load_image_any(path));
    let loaded = match loaded {
        Ok(l) => l,
        Err(e) => {
            eprintln!("読めません: {e}");
            std::process::exit(1);
        }
    };
    if let Some(p) = std::env::var_os("ZAHYOU_BENCH_DUMP") {
        // 検出用の面を float32(リトルエンディアン)で書き出す(Python の値と画素単位で比べるため)
        let b: Vec<u8> = loaded.data.data.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(p, b).expect("書き出し");
    }
    if std::env::var_os("ZAHYOU_BENCH_PARTS").is_some() {
        // 前処理の内訳(同じ順で部品を呼ぶ)
        use zahyou_image::kernels::*;
        let img = &loaded.data;
        let (t_bg, bg) = best(n, || background(img));
        let sub: Vec<f32> = img.data.iter().zip(bg.iter()).map(|(a, b)| a - b).collect();
        let (t_clip, sig) = best(n, || sigma_clipped_std(&sub));
        let (t_uni, _) = best(n, || uniform3(&sub, img.height, img.width));
        let (t_med, _) = best(n, || median_f32(&img.data));
        let mut mask = vec![false; sub.len()];
        let (t_ov, _) = best(n, || mask_overlay_bands(&sub, img.height, img.width, &mut mask));
        let (t_gauss, _) = best(n, || gaussian1(&sub, img.height, img.width));
        eprintln!(
            "内訳: 背景 {t_bg:.4} / σ クリップ 1 回 {t_clip:.4} (σ={sig}) / 3×3 平均 {t_uni:.4} / 中央値 {t_med:.4} / 焼き込み文字 {t_ov:.4} / ガウス {t_gauss:.4}"
        );
    }
    let (t_prep, prep) = best(n, || preprocess(&loaded.data, true));
    let (t_det12, d12) = best(n, || detect_sources(&prep, &DetectOpts::default()));
    let (t_det60, d60) = best(n, || detect_sources(&prep, &DetectOpts::with_min_sources(60)));
    let (t_deep, deep) = best(n, || detect_sources_deep(&prep, &DetectOpts::deep()));
    println!(
        "{{\"path\": {:?}, \"width\": {}, \"height\": {}, \"threads\": {}, \"seconds\": {{\"load\": {:.4}, \"preprocess\": {:.4}, \"detect12\": {:.4}, \"detect60\": {:.4}, \"deep\": {:.4}}}, \"sigma\": {}, \"mask_count\": {}, \"det12\": {}, \"det60\": {}, \"deep\": {}}}",
        path,
        loaded.data.width,
        loaded.data.height,
        rayon::current_num_threads(),
        t_load,
        t_prep,
        t_det12,
        t_det60,
        t_deep,
        prep.sigma,
        prep.mask.iter().filter(|&&m| m).count(),
        d12.0.len(),
        d60.0.len(),
        deep.0.len()
    );
}
