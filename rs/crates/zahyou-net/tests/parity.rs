//! Python 版(testdata/net_python.json)との突き合わせ。通信はしない。
mod common;

use common::*;
use serde_json::Value;
use zahyou_net::*;

fn f64_of(v: &Value) -> Option<f64> {
    v.as_f64()
}

#[test]
fn env_tables_match_python() {
    let Some(py) = python_values() else { return };
    let e = &py["env"];
    let fov: Vec<(u32, f64, f64)> = e["scale_fov"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r[0].as_u64().unwrap() as u32, r[1].as_f64().unwrap(), r[2].as_f64().unwrap()))
        .collect();
    assert_eq!(fov, SCALE_FOV.to_vec(), "SCALE_FOV");
    let mb: Vec<(u32, u64)> = e["scale_mb"].as_array().unwrap().iter().map(|r| (r[0].as_u64().unwrap() as u32, r[1].as_u64().unwrap())).collect();
    assert_eq!(mb, SCALE_MB.to_vec(), "SCALE_MB");
    assert_eq!(e["lite_base"], LITE_BASE);
    assert_eq!(e["g41_base"], G41_BASE);
    assert_eq!(e["tiles"], TILES);
    let rec: Vec<u32> = serde_json::from_value(e["recommended"].clone()).unwrap();
    assert_eq!(recommended_scales(), rec);
    for (s, names) in e["names"].as_object().unwrap() {
        let want: Vec<String> = serde_json::from_value(names.clone()).unwrap();
        assert_eq!(index_names(s.parse().unwrap()), want, "index_names({s})");
    }
    for r in e["urls"].as_array().unwrap() {
        let s = r[0].as_u64().unwrap() as u32;
        let t = r[1].as_u64().map(|t| t as u32);
        assert_eq!(index_url_for(s, t), r[2].as_str().unwrap());
        // 名前からも同じ URL
        let name = match t {
            Some(t) => format!("index-{s}-{t:02}.fits"),
            None => format!("index-{s}.fits"),
        };
        assert_eq!(index_url(&name), r[2].as_str().unwrap());
    }
    for r in e["scales_mb"].as_array().unwrap() {
        let sc: Vec<u32> = serde_json::from_value(r[0].clone()).unwrap();
        assert_eq!(scales_mb(&sc), r[1].as_u64().unwrap());
    }
    for r in e["fov_arcmin"].as_array().unwrap() {
        let got = fov_arcmin(r[0].as_f64().unwrap(), r[1].as_f64().unwrap());
        match f64_of(&r[2]) {
            None => assert_eq!(got, None),
            Some(w) => {
                let g = got.unwrap();
                assert!((g - w).abs() <= 1e-12 * w.abs(), "fov_arcmin {r:?}: {g} != {w}");
            }
        }
    }
    let mut n = 0;
    for r in e["recommend"].as_array().unwrap() {
        let fov = r[0].as_f64().unwrap();
        let short = r[1].as_f64();
        let want: Vec<u32> = serde_json::from_value(r[2].clone()).unwrap();
        assert_eq!(recommend_scales(fov, short), want, "recommend_scales({fov}, {short:?})");
        n += 1;
    }
    eprintln!("recommend_scales: {n} 通りが Python と一致");
    for r in e["human"].as_array().unwrap() {
        assert_eq!(human(r[0].as_f64().unwrap()), r[1].as_str().unwrap(), "human({})", r[0]);
    }
}

#[test]
fn layout_check_matches_python() {
    let Some(py) = python_values() else { return };
    let d = temp_dir("layout");
    let mut n = 0;
    for f in py["layout"]["files"].as_array().unwrap() {
        let name = f["name"].as_str().unwrap();
        let blob = b64(f["b64"].as_str().unwrap());
        let p = d.join(name);
        std::fs::write(&p, &blob).unwrap();
        let got = check_index_file(&p, None);
        let ok = f["ok"].as_bool().unwrap();
        assert_eq!(got.is_ok(), ok, "{name}: Rust {got:?} / Python {}", f["why"]);
        if !ok && f["msg_same"].as_bool().unwrap() {
            assert_eq!(got.unwrap_err(), f["why"].as_str().unwrap(), "{name} の理由");
        }
        n += 1;
    }
    eprintln!("ヘッダーの検査: 偽のファイル {n} 本が Python と一致");
    // 追加の検査(validate)も通す
    let p = d.join("index-5206-07.fits");
    let r = check_index_file(&p, Some(&|_p: &std::path::Path| Err("validate が落とした".to_string())));
    assert_eq!(r, Err("validate が落とした".into()));

    // 手元の本物(あれば)
    let dir = py["layout"]["real_dir"].as_str().unwrap();
    let real = py["layout"]["real"].as_array().unwrap();
    if !std::path::Path::new(dir).is_dir() || real.is_empty() {
        eprintln!("SKIP: {dir} が無いので本物の索引ファイルの検査は省く");
        return;
    }
    let t = std::time::Instant::now();
    let mut m = 0;
    for r in real {
        let p = std::path::Path::new(dir).join(r[0].as_str().unwrap());
        if !p.exists() {
            continue;
        }
        assert_eq!(check_index_file(&p, None).is_ok(), r[1].as_bool().unwrap(), "{}", p.display());
        m += 1;
    }
    eprintln!("本物の索引ファイル {m} 本の検査が Python と一致 ({:.3} 秒)", t.elapsed().as_secs_f64());
    // zahyou-index の検査も通す(既定のダウンロードの検査)。Python の軽い検査に通るものは全部通るはず
    let t = std::time::Instant::now();
    for r in real {
        let p = std::path::Path::new(dir).join(r[0].as_str().unwrap());
        if p.exists() && r[1].as_bool().unwrap() {
            assert_eq!(check_index_file_full(&p), Ok(()), "{}", p.display());
        }
    }
    eprintln!("同じ {m} 本が zahyou-index の検査にも通った ({:.3} 秒)", t.elapsed().as_secs_f64());
}

fn b64(s: &str) -> Vec<u8> {
    // 標準の base64(試験の中だけで使う小さな復号)
    let mut map = [255u8; 256];
    for (i, c) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".iter().enumerate() {
        map[*c as usize] = i as u8;
    }
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for &c in s.as_bytes() {
        if c == b'=' {
            break;
        }
        let v = map[c as usize];
        if v == 255 {
            continue;
        }
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

#[test]
fn formatting_matches_python() {
    let Some(py) = python_values() else { return };
    let f = &py["format"];
    let mut n = 0;
    for r in f["floats"].as_array().unwrap() {
        let x = f64::from_bits(r[0].as_u64().unwrap());
        assert_eq!(pyfmt::py_float_repr(x), r[1].as_str().unwrap(), "repr({x:e})");
        n += 1;
    }
    eprintln!("repr(float): {n} 個が Python と一致");

    // cache.json の書き方(読んで書き直すとバイト単位で同じ)
    let text = f["cache_text"].as_str().unwrap();
    let v: serde_json::Map<String, Value> = serde_json::from_str(text).unwrap();
    assert_eq!(cache::cache_to_string(&v), text);
    let compact = f["compact_text"].as_str().unwrap();
    let v: Value = serde_json::from_str(compact).unwrap();
    assert_eq!(pyfmt::py_json(&v, None, true), compact);

    for r in f["target_key"].as_array().unwrap() {
        assert_eq!(target_key(r[0].as_str().unwrap()), r[1].as_str().unwrap(), "target_key({:?})", r[0]);
    }
    let pairs: Vec<(String, String)> = f["urlencode"]["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p[0].as_str().unwrap().to_string(), p[1].as_str().unwrap().to_string()))
        .collect();
    let refs: Vec<(&str, &str)> = pairs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    assert_eq!(pyfmt::py_urlencode(&refs), f["urlencode"]["text"].as_str().unwrap());
    for r in f["quote"].as_array().unwrap() {
        assert_eq!(pyfmt::py_quote(r[0].as_str().unwrap(), "/"), r[1].as_str().unwrap());
    }
}

#[test]
fn cache_file_roundtrip() {
    let d = temp_dir("cache");
    let p = d.join("cache.json");
    // 無いファイル・壊れたファイル・配列は空として読む
    assert!(cache::load_cache_at(&p).is_empty());
    std::fs::write(&p, b"{broken").unwrap();
    assert!(cache::load_cache_at(&p).is_empty());
    std::fs::write(&p, b"[1,2]").unwrap();
    assert!(cache::load_cache_at(&p).is_empty());
    // 知らないキーは残す。targets が壊れていれば作り直す
    std::fs::write(&p, "{\"other\": {\"x\": 1}, \"targets\": [1]}".as_bytes()).unwrap();
    cache::remember_target_at(&p, "  ucac4   660-021020 ", 61.91199157844, 41.98626242835).unwrap();
    cache::remember_scale_at(&p, "cam||968x548|bin1|f", 1.2345).unwrap();
    cache::remember_scale_at(&p, "cam||968x548|bin1|f2", 5000.0).unwrap(); // 範囲外は覚えない
    let text = std::fs::read_to_string(&p).unwrap();
    assert_eq!(
        text,
        "{\n \"other\": {\n  \"x\": 1\n },\n \"targets\": {\n  \"UCAC4 660-021020\": {\n   \"ra\": 61.91199157844,\n   \"dec\": 41.98626242835,\n   \"name\": \"  ucac4   660-021020 \"\n  }\n },\n \"rigs\": {\n  \"cam||968x548|bin1|f\": 1.2345\n }\n}"
    );
    let c = cache::load_cache_at(&p);
    let hit = cache::lookup_target(&c, "UCAC4 660-021020").unwrap();
    assert_eq!((hit.ra, hit.dec), (61.91199157844, 41.98626242835));
    assert_eq!(cache::remembered_scale_at(&p, "cam||968x548|bin1|f"), Some(1.2345));
    assert_eq!(cache::remembered_scale_at(&p, "cam||968x548|bin1|f2"), None);
    assert_eq!(rig_key(" ZWO ", "C8", 968, 548, "1", "2032.0"), "ZWO|C8|968x548|bin1|f2032.0");

    // 置き場所の決め方(ZAHYOU_CACHE → LOCALAPPDATA\zahyou → 古い Temp のものを引き継ぐ)
    let over = cache::cache_path_from(Some(d.join("x.json").into()), None, None, d.clone());
    assert_eq!(over, d.join("x.json"));
    let tmp = d.join("tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("zahyou_cache.json"), b"{\"targets\": {}}").unwrap();
    let la = d.join("la");
    let p2 = cache::cache_path_from(None, Some(la.clone().into()), None, tmp.clone());
    assert_eq!(p2, la.join("zahyou").join("cache.json"));
    assert_eq!(std::fs::read(&p2).unwrap(), b"{\"targets\": {}}", "古いファイルを引き継ぐ");
}

#[test]
fn target_requests_match_python() {
    let Some(py) = python_values() else { return };
    let t = &py["target"];
    for r in t["sesame_urls"].as_array().unwrap() {
        let name = r[0].as_str().unwrap();
        let urls: Vec<String> = target::SESAME_URLS.iter().map(|b| target::sesame_url(b, name)).collect();
        let want: Vec<String> = serde_json::from_value(r[1].clone()).unwrap();
        assert_eq!(urls, want, "Sesame の URL ({name})");
    }
    for r in t["sesame_parse"].as_array().unwrap() {
        let got = target::parse_sesame(r[0].as_str().unwrap());
        match r[1].as_array() {
            None => assert_eq!(got, None, "{:?}", r[0]),
            Some(w) => assert_eq!(got, Some((w[2].as_f64().unwrap(), w[3].as_f64().unwrap())), "{:?}", r[0]),
        }
    }
    for r in t["vizier"].as_array().unwrap() {
        let ra = f64::from_bits(r[0].as_u64().unwrap());
        let dec = f64::from_bits(r[1].as_u64().unwrap());
        assert_eq!(target::vizier_query_body(ra, dec), r[2].as_str().unwrap(), "VizieR の本文 ({ra}, {dec})");
    }
    let tsv = "#Column\tRA_ICRS\n\nRA_ICRS\tDE_ICRS\tGmag\ndeg\tdeg\tmag\n---------------\t---------------\t---------\n\
               061.91199157844\t+41.98626242835\t13.059022\n061.91300000000\t+41.98700000000\t\n\n";
    let rows = target::parse_vizier_tsv(tsv).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!((rows[0].ra, rows[0].dec, rows[0].gmag), (61.91199157844, 41.98626242835, Some(13.059022)));
    assert_eq!(rows[1].gmag, None);
    assert!(target::parse_vizier_tsv("#INFO only\n").unwrap().is_empty());
    assert!(target::parse_vizier_tsv("RA_ICRS\tDE_ICRS\n---\t---\nabc\tdef\n").is_err());
}

#[test]
fn nova_bodies_match_python() {
    let Some(py) = python_values() else { return };
    let nv = &py["nova"];
    let src: Vec<(f64, f64)> = nv["sources"].as_array().unwrap().iter().map(|s| (s["x"].as_f64().unwrap(), s["y"].as_f64().unwrap())).collect();
    for c in nv["cases"].as_array().unwrap() {
        let hint = c["hint"].as_array().map(|h| (h[0].as_f64().unwrap(), h[1].as_f64().unwrap()));
        let scale = c["scale"].as_f64();
        let reqs = c["requests"].as_array().unwrap();
        assert_eq!(nova::login_body("KEY-0123456789"), reqs[0]["body"].as_str().unwrap());
        let settings = nova::hint_settings(hint, 5.0, scale);
        let xs: Vec<f64> = src.iter().map(|p| p.0 + 1.0).collect();
        let ys: Vec<f64> = src.iter().map(|p| p.1 + 1.0).collect();
        assert_eq!(
            nova::source_list_body(&settings, &xs, &ys, 968, 548, "sess-abc"),
            reqs[1]["body"].as_str().unwrap(),
            "url_upload の本文 (hint {hint:?}, scale {scale:?})"
        );
    }
    let up = &nv["image_upload"]["requests"][1];
    let settings = nova::hint_settings(Some((10.5, -20.25)), 5.0, Some(1.2345));
    let mut s = settings.clone();
    s.push(("session".into(), Value::String("sess-img".into())));
    assert_eq!(nova::request_json(&s), up["request_json"].as_str().unwrap());
    // WCS のヘッダーをカードに分ける
    let cards = nova::split_cards(nv["wcs_text"].as_str().unwrap());
    let want: Vec<String> = serde_json::from_value(nv["wcs_cards"].clone()).unwrap();
    assert_eq!(cards, want);
}

#[test]
fn update_parse_matches_python() {
    let Some(py) = python_values() else { return };
    let u = &py["update"];
    let info = update::parse_release(u["release_json"].as_str().unwrap()).unwrap();
    let w = &u["info"];
    assert_eq!(info.tag, w["tag"].as_str().unwrap());
    assert_eq!(info.name, w["name"].as_str().unwrap());
    assert_eq!(info.published, w["published"].as_str().unwrap());
    assert_eq!(info.url, w["url"].as_str().unwrap());
    assert_eq!(info.size, w["size"].as_u64().unwrap());
    assert_eq!(info.sha256, w["sha256"].as_str().unwrap());
    assert_eq!(info.updated, w["updated"].as_str().unwrap());
    let nd = update::parse_release(u["no_digest_json"].as_str().unwrap()).unwrap();
    assert_eq!(nd.sha256, "");
    assert_eq!(nd.updated, "");
    assert!(update::parse_release("{\"assets\": []}").unwrap_err().contains("zahyou.exe"));
    assert!(update::parse_release("not json").is_err());
}

/// repr(float) を大量に突き合わせる(手で走らせるとき。ZAHYOU_FLOAT_BULK に「ビット列 repr」の行のファイル)。
#[test]
fn float_repr_bulk() {
    let Some(p) = std::env::var_os("ZAHYOU_FLOAT_BULK") else {
        eprintln!("SKIP: ZAHYOU_FLOAT_BULK が無いので repr(float) の大量の突き合わせは省く");
        return;
    };
    let text = std::fs::read_to_string(&p).unwrap();
    let t = std::time::Instant::now();
    let mut n = 0;
    for line in text.lines() {
        let (b, want) = line.split_once(' ').unwrap();
        let x = f64::from_bits(b.parse().unwrap());
        assert_eq!(pyfmt::py_float_repr(x), want, "repr({x:e})");
        n += 1;
    }
    eprintln!("repr(float): {n} 個が Python と一致 ({:.2} 秒)", t.elapsed().as_secs_f64());
}
