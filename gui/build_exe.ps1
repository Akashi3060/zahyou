# =============================================================================
#  zahyou : GUI 版を exe に固める
#
#     powershell -ExecutionPolicy Bypass -File build_exe.ps1
#     powershell -ExecutionPolicy Bypass -File build_exe.ps1 -Folder
#     powershell -ExecutionPolicy Bypass -File build_exe.ps1 -Clean -Test
#
#  既定 (1 ファイル版):
#     dist\onefile\zahyou.exe   102 MB の exe 1 個。起動 4 秒 (実測)
#  -Folder:
#     dist\zahyou\zahyou.exe    221 MB のフォルダー。起動 2 秒。zip も作る
#
#  -Test を付けると、出来た exe で自己診断 (画像を 1 枚解く) まで走らせる。
# =============================================================================
[CmdletBinding()]
param(
    [switch]$Folder,
    [switch]$Clean,
    [switch]$Test,
    [switch]$NoZip,
    [string]$TestImage = "$env:USERPROFILE\Downloads\Capture_00001 00_10_33.fits",
    [string]$Python = ""
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

function Say($m)  { Write-Host $m }
function Ok($m)   { Write-Host "  OK   $m" -ForegroundColor Green }
function Warn($m) { Write-Host "  注意 $m" -ForegroundColor Yellow }
function Die($m)  { Write-Host "  失敗 $m" -ForegroundColor Red; exit 1 }

# --- 1. Python を決める ------------------------------------------------------
if (-not $Python) {
    $cand = @("$env:LOCALAPPDATA\Programs\Python\Python313\python.exe",
              "$env:LOCALAPPDATA\Programs\Python\Python312\python.exe")
    $Python = $cand | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $Python) {
        $g = Get-Command python -ErrorAction SilentlyContinue
        if ($g) { $Python = $g.Source }
    }
}
if (-not $Python -or -not (Test-Path $Python)) {
    Die "python が見つかりません。-Python でパスを指定してください。"
}
$ver = & $Python -c "import sys;print('.'.join(map(str,sys.version_info[:3])))"
Ok "Python $ver  ($Python)"

# --- 2. 必要なものがそろっているか ------------------------------------------
$need = @("PyInstaller", "numpy", "scipy", "astropy", "astroquery", "matplotlib",
          "PIL", "reproject", "dask")
$missing = @()
foreach ($m in $need) {
    & $Python -c "import $m" 2>$null
    if ($LASTEXITCODE -ne 0) { $missing += $m }
}
if ($missing.Count -gt 0) {
    Say "  足りないものを入れます: $($missing -join ', ')"
    $pipNames = $missing -replace '^PIL$', 'Pillow' -replace '^PyInstaller$', 'pyinstaller'
    & $Python -m pip install --upgrade $pipNames
    if ($LASTEXITCODE -ne 0) { Die "pip install に失敗しました。" }
}
Ok "必要なパッケージはそろっています"

# --- 3. エンジンがあるか ----------------------------------------------------
$engine = Join-Path (Split-Path $PSScriptRoot -Parent) "zahyou_engine.py"
if (-not (Test-Path $engine)) { Die "$engine がありません。dev\build_nb.py で作ってください。" }
Ok "解析エンジン: $engine"

# --- 4. 建てる --------------------------------------------------------------
if ($Clean) {
    Remove-Item -Recurse -Force build, dist -ErrorAction SilentlyContinue
    Ok "build / dist を消しました"
}
$oneFile = -not $Folder
$env:ZAHYOU_ONEFILE = if ($oneFile) { "1" } else { "0" }
$distPath = if ($oneFile) { "dist\onefile" } else { "dist" }
$workPath = if ($oneFile) { "build\onefile" } else { "build" }

Say ""
Say "=== PyInstaller ($(if ($oneFile) {'1 ファイル'} else {'フォルダー'})) ==="
$t0 = Get-Date
& $Python -m PyInstaller zahyou.spec --noconfirm --distpath $distPath --workpath $workPath
if ($LASTEXITCODE -ne 0) { Die "ビルドに失敗しました。" }

if ($oneFile) {
    $exe = Join-Path $distPath "zahyou.exe"
    $size = (Get-Item $exe).Length
} else {
    $exe = Join-Path $distPath "zahyou\zahyou.exe"
    $size = (Get-ChildItem (Join-Path $distPath "zahyou") -Recurse |
             Measure-Object Length -Sum).Sum
}
Ok ("{0}  ({1:N0} MB / {2:N0} 秒)" -f $exe, ($size / 1MB), ((Get-Date) - $t0).TotalSeconds)

# --- 5. 自己診断 ------------------------------------------------------------
if ($Test) {
    Say ""
    Say "=== 自己診断 ==="
    if (-not (Test-Path $TestImage)) {
        Warn "テスト画像がないので飛ばします: $TestImage"
    } else {
        $out = Join-Path $PSScriptRoot "selftest.txt"
        Remove-Item $out -ErrorAction SilentlyContinue
        # Start-Process は配列の要素を引用符で囲まないので、1 本の文字列で渡す
        $p = Start-Process -FilePath $exe -PassThru -Wait `
             -ArgumentList "--selftest `"$TestImage`" --out `"$out`""
        if (Test-Path $out) { Get-Content $out -Encoding utf8 | ForEach-Object { Say $_ } }
        if ($p.ExitCode -ne 0) { Die "自己診断に失敗しました。上のログを見てください。" }
        Ok "自己診断に通りました"
    }
}

# --- 6. 配る形にまとめる ----------------------------------------------------
if (-not $NoZip -and -not $oneFile) {
    $zip = Join-Path $PSScriptRoot "dist\zahyou-windows.zip"
    Remove-Item $zip -ErrorAction SilentlyContinue
    Compress-Archive -Path (Join-Path $distPath "zahyou\*") -DestinationPath $zip
    Ok ("{0}  ({1:N0} MB)" -f $zip, ((Get-Item $zip).Length / 1MB))
}

Say ""
Say "=== 完了 ==="
Say "  $exe を実行すると起動します。"
Say "  オフライン解析を使うには、[準備] タブで「まとめて準備する」を押してください。"
Say ""
