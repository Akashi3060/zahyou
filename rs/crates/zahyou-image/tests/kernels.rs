//! numpy・scipy・astropy の部品との照合(rs/testdata/image_kernels.json、gen_image.py の gen_kernels)。
//! どれもビット単位で同じ値になることを確かめる。

mod common;

use common::*;
use zahyou_image::kernels::*;

fn bits(v: f64) -> u32 {
    (v as f32).to_bits()
}

#[test]
fn sum_mean_std_median() {
    let j = load_json("image_kernels.json");
    let mut n_ok = 0;
    for e in j["sum_f32"].as_array().unwrap() {
        let n = e["n"].as_u64().unwrap() as usize;
        let a = synth(n, e["seed"].as_u64().unwrap(), f(&e["center"]), f(&e["spread"]), 0);
        assert_eq!(sum_f32(&a).to_bits(), bits(f(&e["sum"])), "sum n={n}");
        assert_eq!(mean_f32(&a).to_bits(), bits(f(&e["mean"])), "mean n={n}");
        assert_eq!(std_f32(&a).to_bits(), bits(f(&e["std"])), "std n={n}");
        assert_eq!(median_f32(&a).to_bits(), bits(f(&e["median"])), "median n={n}");
        n_ok += 1;
    }
    println!("和・平均・標準偏差・中央値: {n_ok} 件すべてビット一致");
}

#[test]
fn sigma_clip() {
    let j = load_json("image_kernels.json");
    let mut n_ok = 0;
    for (k, e) in j["sigma_clip"].as_array().unwrap().iter().enumerate() {
        let a = match e.get("x") {
            Some(x) => b64_f32(x.as_str().unwrap()),
            None => synth(
                e["n"].as_u64().unwrap() as usize,
                e["seed"].as_u64().unwrap(),
                f(&e["center"]),
                f(&e["spread"]),
                e["outlier_every"].as_u64().unwrap(),
            ),
        };
        let s = sigma_clipped_stats(&a);
        assert_eq!(s.mean.to_bits(), bits(f(&e["mean"])), "mean #{k} n={}", a.len());
        assert_eq!(s.median.to_bits(), bits(f(&e["median"])), "median #{k} n={}", a.len());
        assert_eq!(s.std.to_bits(), bits(f(&e["std"])), "std #{k} n={}", a.len());
        assert_eq!(sigma_clipped_std(&a).to_bits(), s.std.to_bits());
        n_ok += 1;
    }
    println!("sigma_clipped_stats: {n_ok} 件すべてビット一致");
}

#[test]
fn filters_2d() {
    let j = load_json("image_kernels.json");
    let mut n_ok = 0;
    for e in j["filters"].as_array().unwrap() {
        let (ny, nx) = (e["ny"].as_u64().unwrap() as usize, e["nx"].as_u64().unwrap() as usize);
        let x = b64_f32(e["x"].as_str().unwrap());
        let chk = |name: &str, got: Vec<f32>| {
            let want = b64_f32(e[name].as_str().unwrap());
            assert_eq!(got.len(), want.len(), "{name} {ny}x{nx}");
            for (i, (g, w)) in got.iter().zip(want.iter()).enumerate() {
                assert_eq!(g.to_bits(), w.to_bits(), "{name} {ny}x{nx} 画素 {i}: {g} vs {w}");
            }
        };
        chk("median3", median3(&x, ny, nx));
        chk("uniform3", uniform3(&x, ny, nx));
        chk("gauss1", gaussian1(&x, ny, nx));
        chk("binom3", binomial3(&x, ny, nx));
        n_ok += 1;
    }
    println!("median_filter・uniform_filter・gaussian_filter・correlate1d: {n_ok} 形すべてビット一致");
}

#[test]
fn zoom_order1() {
    let j = load_json("image_kernels.json");
    for e in j["zoom"].as_array().unwrap() {
        let g = |k: &str| e[k].as_u64().unwrap() as usize;
        let c = b64_f32(e["x"].as_str().unwrap());
        let (z, oy, ox) = zoom(&c, g("by"), g("bx"), g("ny"), g("nx"));
        let shape: Vec<usize> =
            e["out_shape"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as usize).collect();
        assert_eq!(vec![oy, ox], shape);
        let first: Vec<f32> = e["first"].as_array().unwrap().iter().map(|v| f(v) as f32).collect();
        assert_eq!(&z[..first.len()], &first[..]);
        assert_eq!(ahash_f32(&z), e["hash"].as_str().unwrap(), "zoom {:?}", shape);
    }
}

#[test]
fn background_map() {
    let j = load_json("image_kernels.json");
    for e in j["background"].as_array().unwrap() {
        let (ny, nx) = (e["ny"].as_u64().unwrap() as usize, e["nx"].as_u64().unwrap() as usize);
        let a = synth(
            ny * nx,
            e["seed"].as_u64().unwrap(),
            f(&e["center"]),
            f(&e["spread"]),
            e["outlier_every"].as_u64().unwrap(),
        );
        let img = zahyou_image::Image2D::new(nx, ny, a).unwrap();
        let bk = background(&img);
        assert_eq!(ahash_f32(&bk), e["hash"].as_str().unwrap(), "背景 {ny}x{nx}");
    }
}

#[test]
fn label_4conn() {
    let j = load_json("image_kernels.json");
    for e in j["label"].as_array().unwrap() {
        let (ny, nx) = (e["ny"].as_u64().unwrap() as usize, e["nx"].as_u64().unwrap() as usize);
        let b: Vec<bool> = e["bits"].as_str().unwrap().bytes().map(|c| c == b'1').collect();
        let (lab, n) = label4(&b, ny, nx);
        assert_eq!(n as u64, e["n"].as_u64().unwrap());
        let want: Vec<u32> = e["labels"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
        assert_eq!(lab, want);
    }
}

#[test]
fn luminance_weights() {
    let j = load_json("image_kernels.json");
    let w = lum_weights();
    for (k, v) in j["lum_weights"].as_array().unwrap().iter().enumerate() {
        assert_eq!(w[k].to_bits(), bits(f(v)));
    }
}
