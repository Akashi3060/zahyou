//! 速さの物差し: Python 版 `dev/solver_index.py` と同じ問い合わせ・同じ機械で比べる。
//!
//! ```text
//! cargo run --release -p zahyou-index --example bench_index [-- 結果.json]
//! ```
//!
//! 問い合わせは `rs/testdata/index_ball_<tag>.json`(gen_index.py が作ったもの。半径 0.01 の 1000 問と
//! 速さの物差しの円錐 bench_cone)。答えの件数と sha256 も基準値と比べる(違えば終了コード 1)。
//! 星図データは `C:\AstrometryData`(環境変数 ZAHYOU_INDEX_DIR)、基準値は `rs/testdata`(ZAHYOU_TESTDATA)。
//! Python 側は `rs/testdata/bench_index.py` が同じ項目を測る。
//!
//! 測るもの
//! - code_ball: 4107・5205-02・5202-00 に 1000 問まとめて(初回 = 開く + mmap + 検査込み、2 回目からの中央値、
//!   並列なし = rayon のスレッド 1 本)と、1 問ずつ 1000 回呼んだ合計
//! - stars_in_cone: 同じ 3 本の bench_cone(968×548・1.4616″/px の対角の半分 × 1.05)
//! - 253 本の走査(Catalog::scan)
//! - 5202 の 48 タイルを全部開いて探したときのプライベートメモリ(Windows の PagefileUsage)の増加

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use zahyou_index::{Catalog, IndexFile};

const TAGS: [&str; 3] = ["4107", "5205-02", "5202-00"];

fn index_dir() -> PathBuf {
    std::env::var_os("ZAHYOU_INDEX_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\AstrometryData"))
}

fn testdata_dir() -> PathBuf {
    std::env::var_os("ZAHYOU_TESTDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("testdata"))
}

fn f64s(v: &Value) -> Vec<f64> {
    v.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(f64::NAN)).collect()).unwrap_or_default()
}

fn sha_u32(v: impl IntoIterator<Item = u32>) -> String {
    let mut h = Sha256::new();
    for x in v {
        h.update(x.to_le_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// n 回測って中央値(秒)と最後の結果を返す。
fn median_of<T>(n: usize, mut f: impl FnMut() -> T) -> (f64, T) {
    let mut ts = Vec::with_capacity(n);
    let mut last = None;
    for _ in 0..n.max(1) {
        let t0 = Instant::now();
        let r = f();
        ts.push(t0.elapsed().as_secs_f64());
        last = Some(r);
    }
    ts.sort_by(|a, b| a.total_cmp(b));
    (ts[ts.len() / 2], last.expect("1 回は測る"))
}

/// プライベートメモリ(バイト)。Windows では PagefileUsage(= プロセスのコミット)。
fn private_bytes() -> Option<u64> {
    memory_stats::memory_stats().map(|m| m.virtual_mem as u64)
}

fn working_set() -> Option<u64> {
    memory_stats::memory_stats().map(|m| m.physical_mem as u64)
}

fn mib(b: Option<u64>) -> Value {
    b.map(|v| json!(v as f64 / 1048576.0)).unwrap_or(Value::Null)
}

fn main() -> ExitCode {
    let out_path = std::env::args().nth(1);
    let dir = index_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: 星図データが無い({})", dir.display());
        return ExitCode::SUCCESS;
    }
    let threads = rayon::current_num_threads();
    let one = match rayon::ThreadPoolBuilder::new().num_threads(1).build() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("rayon のスレッドを作れません: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut ok = true;
    let mut result = serde_json::Map::new();
    result.insert("threads".into(), json!(threads));
    println!("rayon のスレッド {threads} 本");

    // ---- code_ball と円錐 ----
    let mut balls = serde_json::Map::new();
    for tag in TAGS {
        let path = dir.join(format!("index-{tag}.fits"));
        let jp = testdata_dir().join(format!("index_ball_{tag}.json"));
        let Some(want) = std::fs::read_to_string(&jp).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) else {
            eprintln!("SKIP {tag}: 基準値 {} が無い", jp.display());
            continue;
        };
        if !path.is_file() {
            eprintln!("SKIP {tag}: {} が無い", path.display());
            continue;
        }
        let q = f64s(&want["queries"]);
        let b = &want["balls"][0];
        let radius = b["radius"].as_f64().unwrap_or(0.01);
        let nq = b["nq"].as_u64().unwrap_or(0) as usize;
        let dim = want["code_dim"].as_u64().unwrap_or(4) as usize;
        let q = &q[..nq * dim];

        // 初回(開く + mmap + 木の検査 + 1000 問)。内訳も取る
        let t0 = Instant::now();
        let f = match IndexFile::open(&path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("{tag}: {e}");
                ok = false;
                continue;
            }
        };
        let t_hdr = t0.elapsed().as_secs_f64();
        if let Err(e) = f.data() {
            eprintln!("{tag}: {e}");
            ok = false;
            continue;
        }
        let t_map = t0.elapsed().as_secs_f64() - t_hdr;
        let first = f.code_ball(q, radius);
        let t_first = t0.elapsed().as_secs_f64();
        println!(
            "  初回の内訳 {tag}: ヘッダー {:.2} ms・mmap と表の検査 {:.2} ms・最初の 1000 問 {:.2} ms",
            t_hdr * 1e3,
            t_map * 1e3,
            (t_first - t_hdr - t_map) * 1e3
        );
        let same = first.len() as u64 == b["total"].as_u64().unwrap_or(u64::MAX)
            && sha_u32(first.iter().flat_map(|&(a, c)| [a, c])) == b["sha256"].as_str().unwrap_or("");
        ok &= same;

        let d = f.data().expect("開けたはず");
        let (t_par, r) = median_of(30, || d.code_ball(q, radius).unwrap_or_default());
        ok &= r == first;
        let (t_one, r) = one.install(|| median_of(30, || d.code_ball(q, radius).unwrap_or_default()));
        ok &= r == first;
        // 1 問ずつ(呼ぶたびに IndexFile を通す = 錠と Arc の分も込み)
        let (t_each, n_each) = median_of(5, || {
            let mut n = 0;
            for qq in q.chunks_exact(dim) {
                n += f.code_ball(qq, radius).len();
            }
            n
        });
        ok &= n_each == first.len();

        let c = &want["bench_cone"];
        let center = {
            let v = f64s(&c["center"]);
            [v[0], v[1], v[2]]
        };
        let cr = c["radius"].as_f64().unwrap_or(0.0);
        let (t_cone, got) = median_of(200, || d.stars_in_cone(center, cr));
        let cone_same = got.len() as u64 == c["count"].as_u64().unwrap_or(u64::MAX)
            && sha_u32(got.iter().copied()) == c["sha256"].as_str().unwrap_or("");
        ok &= cone_same;

        println!(
            "code_ball {tag:8} {nq} 問 → {} 件{}: 初回 {:.2} ms、2 回目から {:.3} ms(並列なし {:.3} ms)、1 問ずつ {nq} 回 {:.2} ms",
            first.len(),
            if same { "(Python と一致)" } else { "(Python と違う!)" },
            t_first * 1e3,
            t_par * 1e3,
            t_one * 1e3,
            t_each * 1e3
        );
        println!(
            "円錐     {tag:8} 半径 {:.2}′ → {} 星{}: {:.1} μs",
            cr.to_degrees() * 60.0,
            got.len(),
            if cone_same { "(Python と一致)" } else { "(Python と違う!)" },
            t_cone * 1e6
        );
        balls.insert(
            tag.into(),
            json!({
                "nq": nq, "radius": radius, "total": first.len(), "match_python": same,
                "first_s": t_first, "warm_s": t_par, "warm_1thread_s": t_one, "each_s": t_each,
                "cone": {"radius": cr, "count": got.len(), "match_python": cone_same, "s": t_cone},
            }),
        );
        drop(d);
        f.close();
    }
    result.insert("ball".into(), Value::Object(balls));

    // ---- 走査 ----
    let (t_scan, cat) = median_of(10, || Catalog::scan(&dir));
    println!(
        "走査 {} 本(使える {}・使えない {}): {:.1} ms",
        cat.files.len() + cat.problems.len(),
        cat.files.len(),
        cat.problems.len(),
        t_scan * 1e3
    );
    result.insert(
        "scan".into(),
        json!({"files": cat.files.len(), "problems": cat.problems.len(), "s": t_scan}),
    );

    // ---- 5202 の 48 タイルを全部開く ----
    if let Some(ids) = cat.series.get(&5202) {
        let p0 = private_bytes();
        let w0 = working_set();
        let t0 = Instant::now();
        let mut hold = Vec::new();
        let mut hits = 0usize;
        let mut stars = 0usize;
        for &i in ids {
            let f = &cat.files[i];
            match f.data() {
                Ok(d) => {
                    // 実際に探して、木のページに触れる
                    hits += d.code_ball(&[0.4, 0.45, 0.6, 0.5], 0.01).map(|v| v.len()).unwrap_or(0);
                    if let Some(c) = f.tile_center {
                        stars += d.stars_in_cone(c, 0.5f64.to_radians()).len();
                    }
                    hold.push(d);
                }
                Err(e) => {
                    eprintln!("{}: {e}", f.name);
                    ok = false;
                }
            }
        }
        let t_open = t0.elapsed().as_secs_f64();
        let p1 = private_bytes();
        let w1 = working_set();
        let mapped: u64 = ids.iter().map(|&i| std::fs::metadata(&cat.files[i].path).map(|m| m.len()).unwrap_or(0)).sum();
        drop(hold);
        cat.close();
        let p2 = private_bytes();
        let dp = match (p0, p1) {
            (Some(a), Some(b)) => Some(b.saturating_sub(a)),
            _ => None,
        };
        println!(
            "5202 の {} タイルを開いて探す({} 件・{} 星、mmap {:.0} MiB): {:.0} ms、プライベートメモリ {:.1} → {:.1} MiB(増加 {:.1} MiB)、\
             ワーキングセット {:.1} → {:.1} MiB、閉じた後 {:.1} MiB",
            ids.len(),
            hits,
            stars,
            mapped as f64 / 1048576.0,
            t_open * 1e3,
            p0.unwrap_or(0) as f64 / 1048576.0,
            p1.unwrap_or(0) as f64 / 1048576.0,
            dp.unwrap_or(0) as f64 / 1048576.0,
            w0.unwrap_or(0) as f64 / 1048576.0,
            w1.unwrap_or(0) as f64 / 1048576.0,
            p2.unwrap_or(0) as f64 / 1048576.0
        );
        result.insert(
            "open_5202".into(),
            json!({
                "tiles": ids.len(), "mapped_mib": mapped as f64 / 1048576.0, "s": t_open,
                "private_before_mib": mib(p0), "private_after_mib": mib(p1), "private_closed_mib": mib(p2),
                "private_increase_mib": mib(dp), "working_set_before_mib": mib(w0), "working_set_after_mib": mib(w1),
            }),
        );
    }

    result.insert("all_match_python".into(), json!(ok));
    if let Some(p) = out_path
        && let Err(e) = std::fs::write(&p, serde_json::to_string_pretty(&Value::Object(result)).unwrap_or_default())
    {
        eprintln!("{p} に書けません: {e}");
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        eprintln!("Python の基準値と違う結果がありました");
        ExitCode::FAILURE
    }
}
