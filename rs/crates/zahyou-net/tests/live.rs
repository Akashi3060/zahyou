//! 本物の外のサービスに問い合わせる試験。
//!
//! - 天体名(Sesame + VizieR)は 1 回だけ実際に引く。つながらなければ SKIP
//! - GitHub の Releases と nova.astrometry.net への疎通は `cargo test -- --ignored` で手動のときだけ
//!   (GitHub の API は認証なしだと 1 時間 60 回まで。nova へは画像も星のリストも送らない)
mod common;

use common::*;
use std::sync::Mutex;
use zahyou_net::*;

#[test]
fn resolve_ucac4_live_then_offline() {
    if std::env::var_os("ZAHYOU_OFFLINE_TESTS").is_some() {
        eprintln!("SKIP: ZAHYOU_OFFLINE_TESTS があるので外へ問い合わせない");
        return;
    }
    let d = temp_dir("live-resolve");
    let cache = d.join("cache.json");
    let log = Mutex::new(Vec::<String>::new());
    let t = std::time::Instant::now();
    let r = match resolve_target("UCAC4 660-021020", true, &cache, &|m| log.lock().unwrap().push(m.into())) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("SKIP: 天体名を引けません(オフライン?): {e}");
            return;
        }
    };
    let secs = t.elapsed().as_secs_f64();
    for l in log.lock().unwrap().iter() {
        eprintln!("[log] {l}");
    }
    eprintln!("UCAC4 660-021020 → RA {:.11} Dec {:.11} ({}) {secs:.2} 秒", r.ra, r.dec, r.note);
    if r.gaia.is_none() {
        eprintln!("SKIP: VizieR に届かず Sesame の座標どまり");
        return;
    }
    assert!((r.ra - 61.91199157844).abs() < 1e-9, "RA {}", r.ra);
    assert!((r.dec - 41.98626242835).abs() < 1e-9, "Dec {}", r.dec);
    let (sra, sdec) = r.sesame.unwrap();
    assert!(target::separation_deg(sra, sdec, r.ra, r.dec) * 3600.0 < 5.0);
    // オフラインではキャッシュから
    let off = resolve_target("ucac4  660-021020", false, &cache, &|_| {}).unwrap();
    assert!(off.from_cache);
    assert_eq!((off.ra, off.dec), (r.ra, r.dec));
    assert!(resolve_target("NO SUCH NAME", false, &cache, &|_| {}).is_err());
}

#[test]
#[ignore]
fn github_latest_release_live() {
    let exe = std::env::current_exe().unwrap();
    let st = update::check_update_at(update::API_URL, &exe, "v6", std::time::Duration::from_secs(20), &|| false);
    eprintln!("{:?}: {}", st.state, st.message);
    let rel = st.release.expect("Releases を読めない");
    eprintln!("tag {} / size {} / sha256 {} / url {}", rel.tag, rel.size, rel.sha256, rel.url);
    assert_eq!(st.state, update::UpdateState::Available);
    assert_eq!(rel.sha256.len(), 64);
}

#[test]
#[ignore]
fn nova_reachable_live() {
    let t = std::time::Instant::now();
    let ok = internet_available();
    eprintln!("internet_available = {ok} ({:.2} 秒)", t.elapsed().as_secs_f64());
    assert!(ok);
}

/// 5206 の 48 タイルの大きさを本物の置き場所に HEAD で聞く(落とさない)。手で走らせるときだけ。
#[test]
#[ignore]
fn remote_sizes_live() {
    let names = index_names(5206);
    let t = std::time::Instant::now();
    let sizes = remote_sizes(&names, &|| false);
    let secs = t.elapsed().as_secs_f64();
    let known: Vec<u64> = sizes.iter().filter_map(|s| s.1).collect();
    eprintln!("HEAD 48 本: {} 本で大きさが分かった、合計 {} ({secs:.2} 秒)", known.len(), human(known.iter().sum::<u64>() as f64));
    assert_eq!(sizes.len(), 48);
    assert_eq!(sizes[0].0, "index-5206-00.fits");
    assert_eq!(known.len(), 48);
}
