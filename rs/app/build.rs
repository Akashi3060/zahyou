//! exe に版情報とマニフェストを埋め込む(Windows のときだけ)。
//!
//! - マニフェスト: 高 DPI(PerMonitorV2)・ComCtl32 v6(ダイアログの見た目)・長いパス・asInvoker
//! - 版情報: エクスプローラーの「プロパティ → 詳細」に出る名前・版・著作権
//! - アイコン: app/zahyou.ico があれば使う(今は無いので省く)
//!
//! リソースのコンパイラ(GNU ターゲットでは MSYS2 の windres)が見つからないときは、
//! 警告を出して埋め込まずに進む(exe は動く。DPI の扱いは winit が実行時に設定する)。

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0" xmlns:asmv3="urn:schemas-microsoft-com:asm.v3">
  <assemblyIdentity type="win32" name="AstroKIT.zahyou" version="0.7.0.0" processorArchitecture="amd64"/>
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

/// 画面に出す版名(main.rs の APP_VER と同じ)
const APP_VER: &str = "v7-dev";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=zahyou.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_manifest(MANIFEST);
    // 日本語(0x0411)の版情報
    res.set_language(0x0411);
    res.set("ProductName", "zahyou");
    res.set("FileDescription", "zahyou — 天体画像から目標への向きを求める");
    res.set("ProductVersion", APP_VER);
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
