//! cache.json の場所をプロセスごと差し替える `cache::override_path`(app の `--selftest` が使う)。
//! 差し替えはプロセス全体に効くので、ほかの試験と混ぜないよう 1 つの試験だけのファイルにしてある。
mod common;

use common::*;
use zahyou_net::cache;

#[test]
fn override_path_wins_and_is_set_once() {
    let d = temp_dir("cache-override");
    let p = d.join("selftest-cache.json");
    cache::override_path(p.clone());
    // 2 回目は効かない(起動直後に 1 回だけ決める)
    cache::override_path(d.join("other.json"));
    assert_eq!(cache::cache_path(), p);
    assert!(!p.exists(), "場所を返すだけでファイルは作らない");

    // 覚える・引くの両方が差し替えた場所を使う(利用者の %LOCALAPPDATA%\zahyou\cache.json に触れない)
    cache::remember_target("UCAC4 660-021020", 61.91199157844, 41.98626242835).unwrap();
    cache::remember_scale("cam||968x548|bin1|f", 1.2345).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.contains("\"UCAC4 660-021020\"") && text.contains("\"cam||968x548|bin1|f\": 1.2345"), "{text}");
    let hit = cache::cached_target("ucac4 660-021020").unwrap();
    assert_eq!((hit.ra, hit.dec), (61.91199157844, 41.98626242835));
    assert_eq!(cache::remembered_scale("cam||968x548|bin1|f"), Some(1.2345));
    assert!(!d.join("other.json").exists());
}
