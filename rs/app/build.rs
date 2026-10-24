//! exe に版情報とマニフェストを埋め込む(Windows のときだけ)。
//!
//! - マニフェスト: 高 DPI(PerMonitorV2)・ComCtl32 v6(ダイアログの見た目)・長いパス・asInvoker
//! - 版情報: エクスプローラーの「プロパティ → 詳細」に出る名前・版・著作権
//! - アイコン: app/zahyou.ico があれば使う(今は無いので省く)
//!
//! 版はどれもパッケージの版(rs/Cargo.toml の `[workspace.package] version`、例: 8.0.0-beta.1)から作る:
//!
//! | 置き場所 | 中身(8.0.0 のとき / 8.0.0-beta.1 のとき) |
//! |---|---|
//! | 画面の版名(環境変数 ZAHYOU_VERSION_LABEL → main.rs の APP_VER) | v8 / v8-beta.1(`version_label`。末尾の 0 の部分を省く) |
//! | マニフェストの assemblyIdentity | 8.0.0.0(数 4 つしか書けないので、プレリリースの部分は落とす) |
//! | FILEVERSION / PRODUCTVERSION(数の版) | 8, 0, 0, 0(同上) |
//! | 文字列の FileVersion | 8.0.0 / 8.0.0-beta.1(パッケージの版のまま) |
//! | 文字列の ProductVersion | v8 / v8-beta.1(画面の版名と同じ) |
//! | FILEFLAGS | プレリリース(-beta.1 など。-dev も)なら VS_FF_PRERELEASE |
//!
//! 版名は v7・v8 のような整数の形(2026-10-24 のユーザーの決定。GitHub のタグ名 v7・v8 と同じ)。パッケージの版は cargo の決まりで
//! 数 3 つの形なので、8.0.0 のように書き、画面に出すときに末尾の 0 を省く。
//!
//! リソースのコンパイラ(GNU ターゲットでは MSYS2 の windres)が見つからないときは、
//! 警告を出して埋め込まずに進む(exe は動く。DPI の扱いは winit が実行時に設定する)。

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0" xmlns:asmv3="urn:schemas-microsoft-com:asm.v3">
  <assemblyIdentity type="win32" name="AstroKIT.zahyou" version="@VERSION4@" processorArchitecture="amd64"/>
  <description>zahyou</description>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0"
                        processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
  <asmv3:application>
    <asmv3:windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2, PerMonitor</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
    </asmv3:windowsSettings>
  </asmv3:application>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <!-- Windows 10 / 11 -->
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
</assembly>
"#;

/// 数の版の 1 つ(0〜65535。読めない・大きすぎるときは 0)
fn version_word(key: &str) -> u16 {
    std::env::var(key).ok().and_then(|s| s.trim().parse::<u16>().ok()).unwrap_or(0)
}

/// 画面に出す版名。パッケージの版に "v" を付け、末尾の 0 の部分を省く:
/// 7.0.0 → v7、8.0.0-beta.1 → v8-beta.1、7.1.0 → v7.1、7.0.1 → v7.0.1。
/// 更新の確認の読み方(zahyou_net::update::parse_version)は、足りない部分を 0 として読むので、同じ版として比べられる。
fn version_label() -> String {
    let part = |key: &str| std::env::var(key).unwrap_or_default();
    let (major, minor, patch, pre) = (
        part("CARGO_PKG_VERSION_MAJOR"),
        part("CARGO_PKG_VERSION_MINOR"),
        part("CARGO_PKG_VERSION_PATCH"),
        part("CARGO_PKG_VERSION_PRE"),
    );
    let mut s = format!("v{major}");
    if minor != "0" || patch != "0" {
        s.push('.');
        s.push_str(&minor);
    }
    if patch != "0" {
        s.push('.');
        s.push_str(&patch);
    }
    if !pre.is_empty() {
        s.push('-');
        s.push_str(&pre);
    }
    s
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=zahyou.ico");
    // 画面に出す版名(main.rs の APP_VER)。Windows 以外でも渡す(env! が無いとコンパイルが通らない)
    let app_ver = version_label();
    println!("cargo:rustc-env=ZAHYOU_VERSION_LABEL={app_ver}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // パッケージの版(Cargo が渡す。版を変えると build.rs も走り直す)
    let pkg_ver = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let (major, minor, patch) =
        (version_word("CARGO_PKG_VERSION_MAJOR"), version_word("CARGO_PKG_VERSION_MINOR"), version_word("CARGO_PKG_VERSION_PATCH"));
    let prerelease = !std::env::var("CARGO_PKG_VERSION_PRE").unwrap_or_default().is_empty();
    let mut res = winresource::WindowsResource::new();
    res.set_manifest(&MANIFEST.replace("@VERSION4@", &format!("{major}.{minor}.{patch}.0")));
    // 数の版(エクスプローラーの「ファイル バージョン」)。winresource の既定と同じ値を、上の数から明示して入れる
    let words = (u64::from(major) << 48) | (u64::from(minor) << 32) | (u64::from(patch) << 16);
    res.set_version_info(winresource::VersionInfo::FILEVERSION, words);
    res.set_version_info(winresource::VersionInfo::PRODUCTVERSION, words);
    if prerelease {
        // β 版(-beta.N)と試作版(-dev)は「正式版ではない」印を付ける
        res.set_version_info(winresource::VersionInfo::FILEFLAGS, winresource::VersionInfo::VS_FF_PRERELEASE);
    }
    // 日本語(0x0411)の版情報
    res.set_language(0x0411);
    res.set("ProductName", "zahyou");
    res.set("FileDescription", "zahyou — 天体画像から目標への向きを求める");
    res.set("FileVersion", &pkg_ver);
    res.set("ProductVersion", &app_ver);
    res.set("OriginalFilename", "zahyou.exe");
    res.set("InternalName", "zahyou");
    res.set("CompanyName", "AstroKIT, Kyushu Institute of Technology");
    res.set("LegalCopyright", "Copyright (c) Yoshitaka Fujikawa / AstroKIT. MIT License");
    let icon = std::path::Path::new("zahyou.ico");
    if icon.exists() {
        res.set_icon("zahyou.ico");
    }
    if let Err(e) = res.compile() {
        // 埋め込めなくても exe は作る(版情報とマニフェストが無いだけ)
        println!("cargo:warning=リソース(版情報・マニフェスト)を埋め込めませんでした: {e}");
    }
}
