# =============================================================================
#  zahyou : WSL 側のオフライン解析エンジンをまとめて用意する
#
#  前提 : 管理者 PowerShell で `wsl --install` を実行し、再起動を済ませてあること。
#
#  使い方 (管理者でなくてよい):
#     powershell -ExecutionPolicy Bypass -File setup_wsl.ps1
#     powershell -ExecutionPolicy Bypass -File setup_wsl.ps1 -CopyIndexToWsl
#
#  やること:
#     1. WSL と Ubuntu が動くか確認
#     2. astrometry.net をインストール
#     3. 星図データの場所を決める
#     4. /etc/astrometry.cfg を書く
#     5. 実際に 1 枚解いて疎通確認 (画像を渡したときだけ)
#
#  何度実行しても同じ結果になる (入っているものは飛ばす)。
# =============================================================================
[CmdletBinding()]
param(
    [string]$IndexDir = "C:\AstrometryData",
    [switch]$CopyIndexToWsl,
    [int]$CpuLimit = 300,
    [string]$TestImage = ""
)

$ErrorActionPreference = "Stop"
$env:WSL_UTF8 = "1"          # WSL の出力を UTF-16 ではなく UTF-8 で受け取る

function Say($m)  { Write-Host $m }
function Ok($m)   { Write-Host "  OK   $m" -ForegroundColor Green }
function Warn($m) { Write-Host "  注意 $m" -ForegroundColor Yellow }
function Die($m)  { Write-Host "  失敗 $m" -ForegroundColor Red; exit 1 }

function Wsl {
    # 注意: PowerShell の $args は自動変数なので、別の名前を使うこと
    param([string]$Command, [switch]$AsRoot)
    $wargs = @()
    if ($AsRoot) { $wargs += @("-u", "root") }
    $wargs += @("-e", "sh", "-c", $Command)
    $out = & wsl.exe @wargs
    return [pscustomobject]@{ Code = $LASTEXITCODE; Out = ($out -join "`n").Trim() }
}

Say ""
Say "=== 1. WSL の確認 ==============================================="
$r = Wsl "echo ok"
if ($r.Code -ne 0 -or $r.Out -notmatch "ok") {
    Die @"
WSL が使えません。管理者 PowerShell で次を実行し、PC を再起動してください。

    wsl --install

"@
}
$distro = (Wsl ". /etc/os-release; echo `$PRETTY_NAME").Out
$who = (Wsl "id -un").Out
Ok "WSL 稼働中 : $distro (既定ユーザー: $who)"

Say ""
Say "=== 2. astrometry.net =========================================="
$have = Wsl "command -v solve-field"
if ($have.Code -eq 0 -and $have.Out) {
    Ok "既に入っています : $($have.Out)"
} else {
    Say "  apt でインストールします (数分かかります)..."
    $r = Wsl "export DEBIAN_FRONTEND=noninteractive; apt-get update -qq && apt-get install -y astrometry.net netpbm > /tmp/zahyou-apt.log 2>&1; echo EXIT=`$?" -AsRoot
    if ($r.Out -notmatch "EXIT=0") {
        Say (Wsl "tail -20 /tmp/zahyou-apt.log").Out
        Die "apt が失敗しました。WSL のターミナルで手動で試してください:`n    sudo apt update && sudo apt install astrometry.net -y"
    }
    $have = Wsl "command -v solve-field"
    if (-not $have.Out) { Die "インストールできませんでした。" }
    Ok "インストール完了 : $($have.Out)"
}

Say ""
Say "=== 3. 星図データ =============================================="
if (-not (Test-Path $IndexDir)) { Die "$IndexDir がありません。先に星図データを展開してください。" }
$files = @(Get-ChildItem $IndexDir -Filter "index-*.fits" -File)
if ($files.Count -eq 0) { Die "$IndexDir に index-*.fits がありません。" }
$gb = [math]::Round(($files | Measure-Object Length -Sum).Sum / 1GB, 2)
Ok "$IndexDir : $($files.Count) ファイル / $gb GB"

$wslIndexDir = (Wsl "wslpath -a -u '$IndexDir'").Out
if (-not $wslIndexDir) { Die "パスを WSL 用に変換できませんでした。" }

if ($CopyIndexToWsl) {
    Say "  WSL の中 (~/AstrometryData) へコピーします..."
    $r = Wsl "mkdir -p ~/AstrometryData && cp -n '$wslIndexDir'/index-*.fits ~/AstrometryData/ 2>/dev/null; ls -1 ~/AstrometryData/index-*.fits | wc -l"
    if ($r.Code -ne 0) { Die "コピーに失敗しました。" }
    $wslIndexDir = (Wsl "echo `$HOME/AstrometryData").Out
    Ok "コピー完了 : $wslIndexDir ($($r.Out) ファイル)"
    Warn "WSL の仮想ディスクが $gb GB ぶん増えます (自動では縮みません)。"
} else {
    Ok "$wslIndexDir をそのまま参照します。"
    Say "     ※ Windows 側 (/mnt/c) の読み出しは ext4 の 1/10 ほどの速さですが、"
    Say "       solve-field が実際に読むのは星図データのごく一部なので、"
    Say "       解析時間はほとんど変わりません (実測 約 3 秒)。"
    Say "       それでも詰めたい場合だけ -CopyIndexToWsl を付けてください。"
}

Say ""
Say "=== 4. /etc/astrometry.cfg ====================================="
$cfg = "inparallel`ncpulimit $CpuLimit`nautoindex`nadd_path $wslIndexDir`n"
$b64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($cfg))
$r = Wsl "if [ -f /etc/astrometry.cfg ] && [ ! -f /etc/astrometry.cfg.bak-zahyou ]; then cp /etc/astrometry.cfg /etc/astrometry.cfg.bak-zahyou; fi; echo '$b64' | base64 -d > /etc/astrometry.cfg; cat /etc/astrometry.cfg" -AsRoot
if ($r.Code -ne 0) { Die "書き込めませんでした。" }
Ok "設定しました:"
$r.Out.Split("`n") | ForEach-Object { Say "         $_" }

Say ""
Say "=== 5. 疎通確認 ================================================"
$seen = (Wsl "ls -1 '$wslIndexDir'/index-*.fits 2>/dev/null | wc -l").Out
if ([int]$seen -eq 0) { Die "WSL から星図データが見えません。" }
Ok "WSL から $seen 個の index ファイルが見えています"

$scales = (Wsl "ls -1 '$wslIndexDir'/index-*.fits | sed -E 's#.*/index-([0-9]{4}).*#\1#' | sort -u | tr '\n' ' '").Out
Ok "そろっている段 : $scales"

# 段の末尾 2 桁 -> 画角 (arcmin)
$fov = @{ '00'=@(2,2.8); '01'=@(2.8,4); '02'=@(4,5.6); '03'=@(5.6,8); '04'=@(8,11);
          '05'=@(11,16); '06'=@(16,22); '07'=@(22,30); '08'=@(30,42); '09'=@(42,60);
          '10'=@(60,85); '11'=@(85,120); '12'=@(120,170); '13'=@(170,240);
          '14'=@(240,340); '15'=@(340,480); '16'=@(480,680); '17'=@(680,1000);
          '18'=@(1000,1400); '19'=@(1400,2000) }
$lo = $null; $hi = $null
foreach ($s in $scales.Split(" ")) {
    if ($s.Length -lt 2) { continue }
    $k = $s.Substring($s.Length - 2, 2)
    if (-not $fov.ContainsKey($k)) { continue }
    if ($null -eq $lo -or $fov[$k][0] -lt $lo) { $lo = $fov[$k][0] }
    if ($null -eq $hi -or $fov[$k][1] -gt $hi) { $hi = $fov[$k][1] }
}
if ($null -ne $lo) { Ok "対応できる画角 : $lo' 〜 $hi'" }

if ($TestImage) {
    if (-not (Test-Path $TestImage)) { Die "テスト画像がありません: $TestImage" }
    Say "  テスト画像で 1 枚解いてみます..."
    $py = (Get-Command python -ErrorAction SilentlyContinue).Source
    if (-not $py) { Warn "python が見つからないので、この確認は飛ばします。" }
    else {
        & $py (Join-Path $PSScriptRoot "test_wsl_e2e.py")
    }
}

Say ""
Say "=== 完了 ======================================================="
Say "ノートブック zahyou_v6.ipynb の セル C で SOLVE_MODE='OFFLINE' にすると"
Say "オフライン解析だけを試せます。"
Say ""
