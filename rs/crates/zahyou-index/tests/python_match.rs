//! Python 基準実装(dev/solver_index.py)の結果(rs/testdata/index_*.json、gen_index.py が作る)との突き合わせ。
//!
//! 星図データ(C:\AstrometryData。ZAHYOU_INDEX_DIR で変えられる)か基準値が無ければ SKIP と出して通す(Windows 以外でも。表を開くのは Windows だけ)。

mod common;

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use common::*;
use rayon::prelude::*;
use serde_json::Value;
use zahyou_index::*;

const BALL_TAGS: [&str; 7] = ["4119", "4115", "4111", "4107", "5206-00", "5205-02", "5202-00"];

// ---------------------------------------------------------------- 走査 ---
#[test]
fn scan_matches_python() {
    let Some(want) = need("scan_matches_python", "index_scan.json", &[]) else { return };
    let t0 = std::time::Instant::now();
    let cat = Catalog::scan(index_dir());
    let dt = t0.elapsed().as_secs_f64();
    let wf = want["files"].as_array().unwrap();
    let n_names = want["n_names"].as_u64().unwrap() as usize;
    let real_names = std::fs::read_dir(index_dir())
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| match_index_name(n).is_some())
        .count();
    if real_names != n_names {
        skip("scan_matches_python", &format!("フォルダーの索引の数が基準値と違う({real_names} 本、基準 {n_names} 本)"));
        return;
    }
    assert_eq!(cat.problems.len(), want["problems"].as_array().unwrap().len(), "problems {:?}", cat.problems);
    assert_eq!(cat.files.len(), wf.len());
    let mut worst_center = 0.0f64;
    let mut worst_radius = 0.0f64;
    let mut bit_center = 0;
    for (f, w) in cat.files.iter().zip(wf) {
        assert_eq!(f.name, w["name"].as_str().unwrap());
        assert_eq!(f.indexid as u64, w["indexid"].as_u64().unwrap());
        assert_eq!(f.healpix as i64, w["healpix"].as_i64().unwrap());
        assert_eq!(f.hpnside, w["hpnside"].as_i64().unwrap());
        assert_eq!(f.dimquads as u64, w["dimquads"].as_u64().unwrap());
        assert_eq!(f.nquads as u64, w["nquads"].as_u64().unwrap());
        assert_eq!(f.nstars as u64, w["nstars"].as_u64().unwrap());
        assert_eq!(f.scale_lo_rad.to_bits(), w["scale_lo_rad"].as_f64().unwrap().to_bits());
        assert_eq!(f.scale_hi_rad.to_bits(), w["scale_hi_rad"].as_f64().unwrap().to_bits());
        assert_eq!(f.jitter_arcsec.to_bits(), w["jitter_arcsec"].as_f64().unwrap().to_bits());
        assert_eq!(f.cutnside.map(|c| c as u64), w["cutnside"].as_u64());
        match (f.tile_center, w["tile_center"].as_array()) {
            (None, None) => {}
            (Some(c), Some(_)) => {
                let wc = vec3(&w["tile_center"]);
                for k in 0..3 {
                    worst_center = worst_center.max((c[k] - wc[k]).abs());
                    bit_center += (c[k].to_bits() == wc[k].to_bits()) as usize;
                }
                let r = f.tile_radius_rad.unwrap();
                worst_radius = worst_radius.max((r - w["tile_radius_rad"].as_f64().unwrap()).abs());
            }
            _ => panic!("{} のタイルの有無が違う", f.name),
        }
        assert_eq!(check_index_file(&f.path, None).unwrap(), w["validate"].as_str().unwrap(), "{}", f.name);
        assert!(validate_index_file(&f.path).is_ok());
    }
    assert!(worst_center == 0.0 && worst_radius == 0.0, "タイルの中心 {worst_center:e}・半径 {worst_radius:e}");
    let ws = want["series"].as_object().unwrap();
    assert_eq!(cat.series.len(), ws.len());
    for (iid, fs) in &cat.series {
        let hp: Vec<i64> = fs.iter().map(|&i| cat.files[i].healpix as i64).collect();
        let w: Vec<i64> = ws[&iid.to_string()].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
        assert_eq!(hp, w, "系列 {iid}");
    }
    let (lo, hi) = cat.coverage_arcmin().unwrap();
    let wc = f64s(&want["coverage_arcmin"]);
    assert_eq!((lo.to_bits(), hi.to_bits()), (wc[0].to_bits(), wc[1].to_bits()));
    eprintln!(
        "走査 {} 本 {:.1} ms(Python {:.1} ms)、タイルの中心 {bit_center} 成分がビットまで一致",
        cat.files.len(),
        dt * 1e3,
        want["python_scan_seconds"].as_f64().unwrap() * 1e3
    );
}

// ------------------------------------------------------- 壊れたファイル ---
fn patch(p: &Path, old: &[u8], new: &[u8]) {
    let mut b = std::fs::read(p).unwrap();
    let i = b.windows(old.len()).position(|w| w == old).expect("置き換える文字列が無い");
    b[i..i + new.len()].copy_from_slice(new);
    std::fs::write(p, b).unwrap();
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// Python 版の試験(case_02)と同じ壊れたファイルを tmp に作る。
fn make_broken(tmp: &Path, junk: &[u8]) {
    let cp = |tag: &str, name: &str| {
        let p = tmp.join(name);
        std::fs::copy(index_path(tag).unwrap(), &p).unwrap();
        p
    };
    cp("4119", "index-4119.fits");
    cp("4119", "index-4118.fits");
    cp("5206-00", "index-5206-01.fits");
    cp("5206-00", "index-5206.fits");
    let p = cp("4117", "index-4117.fits");
    let n = std::fs::metadata(&p).unwrap().len();
    std::fs::OpenOptions::new().write(true).open(&p).unwrap().set_len(n - 1000).unwrap();
    let p = cp("4116", "index-4116.fits");
    patch(&p, b"'04:03:02:01'", b"'01:02:03:04'");
    let p = cp("4115", "index-4115.fits");
    let mut b = std::fs::read(&p).unwrap();
    b.extend(std::iter::repeat_n(0u8, 2880));
    std::fs::write(&p, b).unwrap();
    std::fs::write(tmp.join("index-4114.fits"), junk).unwrap();
    let p = cp("4113", "index-4113.fits");
    patch(&p, b"TTYPE1  = 'sweep", b"TTYPE1  = 'sweeq");
    cp("4119", "index-4119.fits.part");
    cp("4119", "index-4119-copy.fits");
    let p = cp("4112", "index-4112.fits");
    patch(&p, b"DIMQUADS=                    3", b"DIMQUADS=                  3.0");
    let p = cp("4110", "index-4110.fits");
    patch(&p, b"KDT_NNOD=", b"KDT_NNOX=");
    let p = cp("4109", "index-4109.fits");
    patch(&p, b"KDT_EXT = 'double", b"KDT_EXT = 'single");
}

pub fn temp_dir(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("zahyou-index-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn broken_files_match_python() {
    let tags = ["4119", "4117", "4116", "4115", "4113", "4112", "4110", "4109", "5206-00"];
    let Some(want) = need("broken_files_match_python", "index_broken.json", &tags) else { return };
    let tmp = temp_dir("broken");
    make_broken(&tmp, &hex(want["junk_hex"].as_str().unwrap()));
    let checks = want["checks"].as_object().unwrap();
    for (nm, w) in checks {
        let p = tmp.join(nm);
        let got = check_index_file(&p, None);
        let (ok, msg) = match &got {
            Ok(m) => (true, m.clone()),
            Err(m) => (false, m.clone()),
        };
        assert_eq!(ok, w["ok"].as_bool().unwrap(), "{nm}: {msg}");
        assert_eq!(msg, w["message"].as_str().unwrap(), "{nm}");
        let opened = IndexFile::open(&p);
        assert_eq!(opened.is_ok(), w["opens"].as_bool().unwrap(), "{nm}");
        if let Err(e) = opened {
            assert_eq!(e.to_string(), w["message"].as_str().unwrap(), "{nm}");
        }
    }
    let part = tmp.join("index-4119.fits.part");
    for (n, w) in want["part_named"].as_object().unwrap() {
        let got = check_index_file(&part, Some(n));
        let w = w.as_array().unwrap();
        assert_eq!(got.is_ok(), w[0].as_bool().unwrap(), "{n}");
        assert_eq!(got.unwrap_or_else(|e| e), w[1].as_str().unwrap());
    }
    let cat = Catalog::scan(&tmp);
    let names: Vec<&str> = cat.files.iter().map(|f| f.name.as_str()).collect();
    let wn: Vec<&str> = want["catalog_files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(names, wn);
    let probs: Vec<(String, String)> = cat
        .problems
        .iter()
        .map(|(p, m)| (p.file_name().unwrap().to_string_lossy().into_owned(), m.clone()))
        .collect();
    let wp: Vec<(String, String)> = want["catalog_problems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v[0].as_str().unwrap().to_string(), v[1].as_str().unwrap().to_string()))
        .collect();
    assert_eq!(probs, wp);
    cat.close();
    let missing = Catalog::scan(tmp.join("無いフォルダー"));
    assert!(missing.files.is_empty());
    let mp: Vec<&str> = missing.problems.iter().map(|(_, m)| m.as_str()).collect();
    let wm: Vec<&str> = want["missing_dir_problems"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(mp, wm);
    drop(cat);
    let _ = std::fs::remove_dir_all(&tmp);
    eprintln!("壊れたファイル {} 本の検査の文が Python と全部同じ", checks.len());
}

// ------------------------------------------------------------- HEALPix ---
#[test]
fn healpix_matches_python() {
    let Some(want) = load_json("index_healpix.json") else {
        skip("healpix_matches_python", "基準値が無い");
        return;
    };
    let pts = f64s(&want["points"]);
    let mut n = 0;
    for ns in u64s(&want["nsides"]) {
        let w = u64s(&want["hp"][ns.to_string()]);
        for (i, p) in pts.chunks_exact(3).enumerate() {
            assert_eq!(healpix_xy_from_xyz([p[0], p[1], p[2]], ns as u32) as u64, w[i], "nside {ns} 点 {i}");
            n += 1;
        }
    }
    let mut worst = 0.0f64;
    let mut bits = 0;
    let mut total = 0;
    for ns in [2u32, 3] {
        let c = &want["centers"][ns.to_string()];
        let wc = f64s(&c["center"]);
        let wr = f64s(&c["radius"]);
        for hp in 0..12 * ns * ns {
            let v = healpix_xy_center(hp, ns);
            for k in 0..3 {
                let w = wc[hp as usize * 3 + k];
                worst = worst.max((v[k] - w).abs());
                bits += (v[k].to_bits() == w.to_bits()) as usize;
                total += 1;
            }
            let r = healpix_xy_radius(hp, ns);
            worst = worst.max((r - wr[hp as usize]).abs());
            bits += (r.to_bits() == wr[hp as usize].to_bits()) as usize;
            total += 1;
        }
    }
    let inner = &want["inner"];
    let (hp, dx, dy, xyz) = (u64s(&inner["hp"]), f64s(&inner["dx"]), f64s(&inner["dy"]), f64s(&inner["xyz"]));
    for i in 0..hp.len() {
        let v = healpix_xy_point(hp[i], 3, dx[i], dy[i]);
        for k in 0..3 {
            worst = worst.max((v[k] - xyz[i * 3 + k]).abs());
            bits += (v[k].to_bits() == xyz[i * 3 + k].to_bits()) as usize;
            total += 1;
        }
    }
    assert!(worst < 1e-15, "中心・半径・点の差 {worst:e}");
    eprintln!("HEALPix: 番号 {n} 個が一致、中心・半径・点 {bits}/{total} 成分がビットまで一致(最大差 {worst:e})");
}

// --------------------------------------------- code_ball・円錐・値の見本 ---
struct Ball {
    f: IndexFile,
    want: Value,
}

fn load_ball(tag: &str, test: &str) -> Option<Ball> {
    let want = need(test, &format!("index_ball_{tag}.json"), &[tag])?;
    let f = IndexFile::open(index_path(tag)?).unwrap();
    Some(Ball { f, want })
}

#[test]
fn code_ball_matches_python_and_bruteforce() {
    let mut rows = Vec::new();
    for tag in BALL_TAGS {
        let Some(Ball { f, want }) = load_ball(tag, "code_ball_matches_python_and_bruteforce") else { continue };
        let d = f.data().unwrap();
        let dim = d.code_dim();
        assert_eq!(dim as u64, want["code_dim"].as_u64().unwrap());
        let q = f64s(&want["queries"]);
        // 総当たり用に全コードを実数へ(第 1 成分で並べ、|c₀ − q₀| ≤ r の帯だけ比べる)
        let codes: Vec<[f64; 4]> = (0..d.nquads() as u32).into_par_iter().map(|i| d.code(i).unwrap()).collect();
        let mut order: Vec<u32> = (0..codes.len() as u32).collect();
        order.par_sort_by(|a, b| codes[*a as usize][0].total_cmp(&codes[*b as usize][0]));
        let c0: Vec<f64> = order.iter().map(|&i| codes[i as usize][0]).collect();
        for b in want["balls"].as_array().unwrap() {
            let radius = b["radius"].as_f64().unwrap();
            let nq = b["nq"].as_u64().unwrap() as usize;
            let qq = &q[..nq * dim];
            let got = d.code_ball(qq, radius).unwrap();
            // Python と: 件数・問い合わせごとの件数・並び全体の sha256・先頭 3 問の中身
            assert_eq!(got.len() as u64, b["total"].as_u64().unwrap(), "{tag} r={radius}");
            let mut counts = vec![0u64; nq];
            for &(qi, _) in &got {
                counts[qi as usize] += 1;
            }
            assert_eq!(counts, u64s(&b["counts"]), "{tag} r={radius}");
            assert_eq!(sha_u32(got.iter().flat_map(|&(a, c)| [a, c])), b["sha256"].as_str().unwrap(), "{tag} r={radius}");
            for (j, w) in b["first"].as_array().unwrap().iter().enumerate() {
                let mine: Vec<u64> = got.iter().filter(|p| p.0 as usize == j).map(|p| p.1 as u64).collect();
                assert_eq!(mine, u64s(w), "{tag} r={radius} 問 {j}");
            }
            // 総当たりと(同じ判定の式。帯で絞る版と、先頭の何問かは全点と)
            let rr = radius * radius;
            let brute: Vec<Vec<u32>> = qq
                .par_chunks_exact(dim)
                .map(|x| {
                    let a = c0.partition_point(|v| *v < x[0] - radius - 1e-9);
                    let e = c0.partition_point(|v| *v <= x[0] + radius + 1e-9);
                    let mut hit: Vec<u32> = order[a..e]
                        .iter()
                        .copied()
                        .filter(|&i| {
                            let c = &codes[i as usize];
                            let mut s = 0.0;
                            for k in 0..dim {
                                let t = c[k] - x[k];
                                s += t * t;
                            }
                            s <= rr
                        })
                        .collect();
                    hit.sort_unstable();
                    hit
                })
                .collect();
            let flat: Vec<(u32, u32)> =
                brute.iter().enumerate().flat_map(|(j, h)| h.iter().map(move |&c| (j as u32, c))).collect();
            assert_eq!(got, flat, "{tag} r={radius} 総当たり(帯)");
            let npure = if codes.len() < 1_000_000 { 100 } else { 10 };
            for (j, x) in qq.chunks_exact(dim).take(npure).enumerate() {
                let pure: Vec<u32> = (0..codes.len() as u32)
                    .into_par_iter()
                    .filter(|&i| {
                        let c = &codes[i as usize];
                        let mut s = 0.0;
                        for k in 0..dim {
                            let t = c[k] - x[k];
                            s += t * t;
                        }
                        s <= rr
                    })
                    .collect();
                assert_eq!(pure, brute[j], "{tag} r={radius} 問 {j} 全点");
            }
            if radius == 0.01 {
                rows.push(format!("{tag}(D={dim}){nq} 問 {} 件", got.len()));
            }
        }
        // 1 問ずつ投げても一緒にまとめても同じ
        let all = d.code_ball(&q[..50 * dim], 0.01).unwrap();
        for j in 0..50 {
            let one = d.code_ball(&q[j * dim..(j + 1) * dim], 0.01).unwrap();
            let a: Vec<u32> = all.iter().filter(|p| p.0 == j as u32).map(|p| p.1).collect();
            let b: Vec<u32> = one.iter().map(|p| p.1).collect();
            assert_eq!(a, b);
        }
        // 形の違う問い合わせ・負の半径・NaN
        assert!(d.code_ball(&q[..dim + 1], 0.01).is_err());
        assert!(d.code_ball(&q[..dim], -1.0).unwrap().is_empty());
        assert!(d.code_ball(&q[..dim], f64::NAN).unwrap().is_empty());
        assert!(d.code_ball(&vec![f64::NAN; dim], 0.01).unwrap().is_empty());
        assert_eq!(f.code_ball(&q[..dim + 1], 0.01), vec![]);
        // NaN を含む問い合わせは空で、ほかの問い合わせの答えは変わらない(Python 版も同じ)
        let mut mixed = q[..10 * dim].to_vec();
        mixed[3 * dim + 1] = f64::NAN;
        let base: Vec<(u32, u32)> = d.code_ball(&q[..10 * dim], 0.01).unwrap().into_iter().filter(|p| p.0 != 3).collect();
        assert_eq!(d.code_ball(&mixed, 0.01).unwrap(), base, "{tag} NaN 混じり");
        // NaN では木をたどらずに返す(f64::max が NaN を無視するので、放っておくと全部たどる)
        let nan_q = vec![f64::NAN; dim];
        let t0 = Instant::now();
        for _ in 0..100 {
            assert!(d.code_ball(&nan_q, 0.01).unwrap().is_empty());
        }
        assert!(t0.elapsed().as_secs_f64() < 0.05, "{tag} NaN の問い合わせが遅い({:?})", t0.elapsed());
    }
    eprintln!("code_ball が Python・総当たりと一致: {}", rows.join("、"));
}

#[test]
fn cones_and_samples_match_python() {
    let mut rows = Vec::new();
    for tag in BALL_TAGS {
        let Some(Ball { f, want }) = load_ball(tag, "cones_and_samples_match_python") else { continue };
        let d = f.data().unwrap();
        let stars: Vec<[f64; 3]> = (0..d.nstars() as u32).into_par_iter().map(|i| d.star_xyz(i).unwrap()).collect();
        let mut nfound = 0;
        let cones = want["cones"].as_array().unwrap();
        for c in cones.iter().chain(std::iter::once(&want["bench_cone"])) {
            let center = vec3(&c["center"]);
            let r = c["radius"].as_f64().unwrap();
            let got = f.stars_in_cone(center, r);
            assert_eq!(got.len() as u64, c["count"].as_u64().unwrap(), "{tag} r={r}");
            assert_eq!(sha_u32(got.iter().copied()), c["sha256"].as_str().unwrap(), "{tag} r={r}");
            if let Some(ids) = c.get("ids").filter(|v| v.is_array()) {
                assert_eq!(got.iter().map(|&i| i as u64).collect::<Vec<_>>(), u64s(ids));
            }
            // 総当たり(弦の長さで同じ式)
            let n = norm(center);
            let cc = [center[0] / n, center[1] / n, center[2] / n];
            let ch = 2.0 * (r / 2.0).sin();
            let want_ids: Vec<u32> = (0..stars.len() as u32)
                .into_par_iter()
                .filter(|&i| {
                    let s = stars[i as usize];
                    let mut d2 = 0.0;
                    for k in 0..3 {
                        let t = s[k] - cc[k];
                        d2 += t * t;
                    }
                    d2 <= ch * ch
                })
                .collect();
            assert_eq!(got, want_ids, "{tag} r={r} 総当たり");
            nfound += got.len();
        }
        // 端の場合: 半径 0・負・NaN・π 以上
        // 半径 0 は、中心を正規化し直すと最後のビットが動くことがあるので自分自身も入るとは限らない
        // (Python 版も同じ)。入るなら星 7 と同じ位置の星だけ。自分自身が入ることはごく小さい半径で確かめる
        let s7 = stars[7];
        assert!(f.stars_in_cone(s7, 0.0).iter().all(|&i| stars[i as usize] == s7));
        assert!(f.stars_in_cone(s7, 1e-9).contains(&7));
        assert!(f.stars_in_cone(s7, -1.0).is_empty());
        assert!(f.stars_in_cone(s7, f64::NAN).is_empty());
        assert_eq!(f.stars_in_cone(s7, std::f64::consts::PI).len(), f.nstars);
        assert!(f.stars_in_cone([0.0, 0.0, 0.0], 0.1).is_empty());
        // 中心が NaN・長さ 0 のときは木をたどらずに空を返す
        let t0 = Instant::now();
        for _ in 0..100 {
            assert!(f.stars_in_cone([0.0, 0.0, 0.0], 3.0).is_empty());
            assert!(f.stars_in_cone([f64::NAN, 0.0, 1.0], 3.0).is_empty());
        }
        assert!(t0.elapsed().as_secs_f64() < 0.05, "{tag} NaN の円錐が遅い({:?})", t0.elapsed());
        // 値の見本: 星の単位ベクトル・sweep・quad の星・コード(実数)・星から計算し直したコード
        let s = &want["samples"];
        let sid = u64s(&s["star_ids"]);
        let sx = f64s(&s["star_xyz"]);
        let sw = u64s(&s["sweep"]);
        for (j, &i) in sid.iter().enumerate() {
            let v = f.star_xyz(i as u32);
            for k in 0..3 {
                assert_eq!(v[k].to_bits(), sx[j * 3 + k].to_bits(), "{tag} 星 {i}");
            }
            assert_eq!(f.sweep(i as u32) as u64, sw[j]);
        }
        let qid = u64s(&s["quad_ids"]);
        let qs = s["quad_stars"].as_array().unwrap();
        let codes = f64s(&s["codes"]);
        let recomputed = f64s(&s["codes_from_xyz"]);
        let dq = f.dimquads;
        let dim = f.code_dim();
        let mut worst = 0.0f64;
        for (j, &q) in qid.iter().enumerate() {
            let st = f.quad_stars(q as u32);
            let w = u64s(&qs[j]);
            for k in 0..dq {
                assert_eq!(st[k] as u64, w[k]);
            }
            if dq == 3 {
                assert_eq!(st[3], u32::MAX);
            }
            let c = d.code(q as u32).unwrap();
            for k in 0..dim {
                assert_eq!(c[k].to_bits(), codes[j * dim + k].to_bits(), "{tag} quad {q}");
            }
            let xyz: Vec<[f64; 3]> = st[..dq].iter().map(|&i| f.star_xyz(i)).collect();
            let rc = quad_code_from_xyz(&xyz).unwrap();
            for k in 0..dim {
                worst = worst.max((rc[k] - recomputed[j * dim + k]).abs());
            }
        }
        assert!(worst < 1e-12, "{tag} 星から計算したコードの差 {worst:e}");
        // 範囲の外の番号
        assert!(f.star_xyz(u32::MAX)[0].is_nan());
        assert_eq!(f.sweep(f.nstars as u32), u8::MAX);
        assert_eq!(f.quad_stars(f.nquads as u32), [u32::MAX; 4]);
        rows.push(format!("{tag} {} 回 {nfound} 星(星から計算したコードの差 {worst:.1e})", cones.len() + 1));
    }
    eprintln!("stars_in_cone が Python・総当たりと一致: {}", rows.join("、"));
}

fn norm(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

// ------------------------------------------------ コードを星から計算し直す ---
#[test]
fn codes_recomputed_from_stars() {
    let mut rows = Vec::new();
    for tag in ["4119", "4115", "4111", "4110", "5206-00", "4107", "5202-00"] {
        let Some(p) = index_path(tag) else {
            skip("codes_recomputed_from_stars", &format!("index-{tag}.fits が無い"));
            continue;
        };
        let f = IndexFile::open(p).unwrap();
        let d = f.data().unwrap();
        let dq = f.dimquads;
        let dim = f.code_dim();
        // 全 quad(4107 は 445 万、5202-00 は 310 万)
        let (same, maxerr) = (0..d.nquads() as u32)
            .into_par_iter()
            .map(|q| {
                let st = d.quad_stars(q).unwrap();
                let xyz: Vec<[f64; 3]> = st[..dq].iter().map(|&i| d.star_xyz(i).unwrap()).collect();
                let c = quad_code_from_xyz(&xyz).unwrap();
                let t = d.code_to_tree(&c);
                let ints = d.code_ints(q).unwrap();
                let real = d.code(q).unwrap();
                let ok = (0..dim).all(|k| t[k].round_ties_even() == ints[k] as f64);
                let err = (0..dim).map(|k| (c[k] - real[k]).abs()).fold(0.0, f64::max);
                (ok as usize, err)
            })
            .reduce(|| (0, 0.0), |a, b| (a.0 + b.0, a.1.max(b.1)));
        assert_eq!(same, d.nquads(), "{tag}");
        rows.push(format!("{tag}{} {}/{}(最大差 {maxerr:.2e})", if dq == 3 { "(三角形)" } else { "" }, same, d.nquads()));
    }
    if !rows.is_empty() {
        eprintln!("整数まで一致: {}", rows.join("、"));
    }
}

// ------------------------------------------------------ select・tiles_near ---
#[test]
fn select_and_tiles_near_match_python() {
    let Some(want) = need("select_and_tiles_near_match_python", "index_select.json", &[]) else { return };
    let Some(scan) = load_json("index_scan.json") else { return };
    let cat = Catalog::scan(index_dir());
    let names: Vec<&str> = cat.files.iter().map(|f| f.name.as_str()).collect();
    let wnames: Vec<&str> = scan["files"].as_array().unwrap().iter().map(|v| v["name"].as_str().unwrap()).collect();
    if names != wnames {
        skip("select_and_tiles_near_match_python", "フォルダーの索引が基準値を作ったときと違う");
        return;
    }
    let mut n = 0;
    for c in want["select"].as_array().unwrap() {
        let hint = c["hint"].as_array().map(|_| (vec3(&c["hint"]), c["hint_radius"].as_f64().unwrap_or(0.0)));
        let got = cat.try_select(
            c["scale_lo"].as_f64().unwrap(),
            c["scale_hi"].as_f64().unwrap(),
            c["width"].as_f64().unwrap(),
            c["height"].as_f64().unwrap(),
            hint,
        );
        match (got, c["result"].as_array()) {
            (Ok(sel), Some(_)) => {
                let w: Vec<usize> = u64s(&c["result"]).into_iter().map(|v| v as usize).collect();
                assert_eq!(sel, w, "{c}");
            }
            (Err(e), None) => assert_eq!(e, c["error"].as_str().unwrap()),
            (g, _) => panic!("{c}: {g:?}"),
        }
        n += 1;
    }
    let mut m = 0;
    for c in want["tiles_near"].as_array().unwrap() {
        let got = cat.tiles_near(c["indexid"].as_u64().unwrap() as u32, vec3(&c["center"]), c["radius"].as_f64().unwrap());
        let w: Vec<usize> = u64s(&c["result"]).into_iter().map(|v| v as usize).collect();
        assert_eq!(got, w, "{c}");
        m += 1;
    }
    // 星のあるタイルは全部 tiles_near に入る(Python 版 case_11 と同じ確かめ方)
    let tiles = &cat.series[&5206];
    let mut miss = 0;
    for c in want["tiles_near"].as_array().unwrap().iter().filter(|c| c["indexid"] == 5206) {
        let (center, r) = (vec3(&c["center"]), c["radius"].as_f64().unwrap());
        let near: Vec<usize> = cat.tiles_near(5206, center, r);
        for &i in tiles {
            if !cat.files[i].stars_in_cone(center, r).is_empty() && !near.contains(&i) {
                miss += 1;
            }
        }
    }
    cat.close();
    assert_eq!(miss, 0);
    eprintln!("select {n} 通り・tiles_near {m} 通りが Python と同じ順で一致");
}

// --------------------------------------------- close でファイルを放す ---
#[test]
fn close_releases_file() {
    let Some(src) = index_path("4119") else {
        skip("close_releases_file", "index-4119.fits が無い");
        return;
    };
    let tmp = temp_dir("close");
    let p = tmp.join("index-4119.fits");
    std::fs::copy(src, &p).unwrap();
    let f = IndexFile::open(&p).unwrap();
    assert!(!f.is_open());
    let d = f.data().unwrap();
    let c = d.code(0).unwrap();
    let hits = f.code_ball(&c[..2], 0.01);
    let cone = f.stars_in_cone(f.star_xyz(5), 30f64.to_radians());
    assert!(!hits.is_empty() && !cone.is_empty());
    drop(d);
    // 開いている間は消せない・書き込み用に開けない(同じプロセスからでも。表を開くのは Windows だけ)
    assert!(std::fs::remove_file(&p).is_err(), "開いている間にファイルが消せた");
    assert!(std::fs::OpenOptions::new().write(true).open(&p).is_err(), "開いている間に書き込み用に開けた");
    f.close();
    assert!(!f.is_open());
    let x = f.star_xyz(0); // 閉じた後に使えばまた開く
    assert!(x[0].is_finite() && f.is_open());
    let held = f.data().unwrap();
    f.close();
    // 外にハンドルが残っている間は放されない
    assert!(std::fs::remove_file(&p).is_err());
    drop(held);
    std::fs::remove_file(&p).expect("close() の後もファイルが消せない");
    let _ = std::fs::remove_dir_all(&tmp);
    // ファイルが無くなった後: 開けないので空・番兵の値(panic しない)
    assert!(f.code_ball(&c[..2], 0.01).is_empty());
    assert!(f.data().is_err());
    assert!(f.star_xyz(0)[0].is_nan());
    eprintln!("開いている間は削除・書き込みを拒否、close() とハンドルを放した後は削除できた(問い合わせ {} 件・円錐 {} 星)", hits.len(), cone.len());
}

#[test]
fn mag_table_reads_big_endian() {
    let Some(p) = index_path("4107") else {
        skip("mag_table_reads_big_endian", "index-4107.fits が無い");
        return;
    };
    let f = IndexFile::open(p).unwrap();
    let d = f.data().unwrap();
    // 等級は -2〜20 に入る(リトルエンディアンで読むと約 37 % が外れる。index-format.md 3 節)
    let mut n = 0;
    let mut inside = 0;
    let mut min = f32::MAX;
    for i in (0..d.nstars() as u32).step_by(97) {
        if let Some(m) = d.mag(i) {
            n += 1;
            inside += (-2.0..20.0).contains(&m) as usize;
            min = min.min(m);
        }
    }
    assert!(n > 10_000 && inside == n, "{inside}/{n}");
    if let Some(p5) = index_path("5206-00") {
        let g = IndexFile::open(p5).unwrap();
        assert!(g.data().unwrap().mag(0).is_none()); // 5200 系には等級の表が無い
    }
    let _ = HashMap::<u8, u8>::new();
    eprintln!("4107 の等級 {n} 星が全部 -2〜20 等(最も明るい {min:.3})");
}
