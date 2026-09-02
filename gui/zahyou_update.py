"""
GitHub の Releases に新しい zahyou.exe があるか調べて、入れ替える。

    st = check()                      # 調べるだけ
    download(st, dest, log, ...)      # 落として、中身を確かめる
    apply_update(dest, log)           # 入れ替えて再起動 (呼んだ側はアプリを閉じる)

見分け方は <b>SHA256 の突き合わせ</b>。GitHub の API は資産ごとに
digest (sha256:...) を返すので、動いている exe のハッシュと比べれば
「同じ版を建て直したもの」まで正確に分かる。バージョン名だけでは分からない。

入れ替えは、動いている exe を自分で上書きできない (Windows) ため、
別プロセス (PowerShell) に任せる。手順は
  1. 新しいものを zahyou.exe.new として横に置く
  2. アプリが閉じるのを待つ
  3. 今のものを zahyou.exe.bak へ退避 → 新しいものを zahyou.exe にする
  4. 起動し直す
失敗したら退避したものを戻すので、消えて無くなることはない。
"""
from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.request

REPO = "Akashi3060/zahyou"
API_URL = f"https://api.github.com/repos/{REPO}/releases/latest"
RELEASES_PAGE = f"https://github.com/{REPO}/releases/latest"
ASSET = "zahyou.exe"
UA = {"User-Agent": "zahyou-updater", "Accept": "application/vnd.github+json"}

_NO_WINDOW = 0x08000000 if os.name == "nt" else 0


def running_exe():
    """動いている exe。ソースから動かしているときは None。"""
    if getattr(sys, "frozen", False):
        return os.path.abspath(sys.executable)
    return None


def sha256_file(path, on_bytes=None, stop=None):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 22), b""):
            if stop is not None and stop():
                return None
            h.update(chunk)
            if on_bytes:
                on_bytes(len(chunk))
    return h.hexdigest().lower()


def latest(timeout=20):
    """Releases の最新版を 1 つ調べる。"""
    req = urllib.request.Request(API_URL, headers=UA)
    with urllib.request.urlopen(req, timeout=timeout) as r:
        data = json.load(r)
    asset = next((a for a in data.get("assets", [])
                  if a.get("name") == ASSET), None)
    if asset is None:
        raise RuntimeError(f"{ASSET} が Releases にありません。")
    digest = (asset.get("digest") or "")
    return {
        "tag": data.get("tag_name", ""),
        "name": data.get("name", ""),
        "published": (data.get("published_at") or "")[:10],
        "url": asset["browser_download_url"],
        "size": int(asset.get("size", 0)),
        "sha256": digest.split(":")[-1].lower() if digest else "",
        "updated": (asset.get("updated_at") or "")[:10],
    }


def check(timeout=20, stop=None):
    """
    更新があるか調べる。戻り値は画面にそのまま出せる形。

      state   : "available" / "up_to_date" / "source" / "error"
      message : 1 行の説明
    """
    exe = running_exe()
    if exe is None:
        return {"state": "source",
                "message": "ソースから動かしているので、更新は確認しません。"}
    try:
        info = latest(timeout=timeout)
    except Exception as e:
        return {"state": "error",
                "message": f"更新を確認できませんでした ({type(e).__name__})。"
                           "インターネットにつながっているか確かめてください。"}

    here_size = os.path.getsize(exe)
    if info["sha256"]:
        here = sha256_file(exe, stop=stop)
        if here is None:
            return {"state": "error", "message": "中止しました。"}
        same = (here == info["sha256"])
        info["here_sha256"] = here
    else:                                   # digest が無い API のとき
        same = (here_size == info["size"])
    info["here_size"] = here_size
    if same:
        info.update(state="up_to_date",
                    message=f"最新版です ({info['tag']} / {info['updated']} 版)。")
    else:
        info.update(state="available",
                    message=f"新しい版があります: {info['tag']} "
                            f"({info['updated']} 版 / "
                            f"{info['size'] / 1e6:.0f} MB)")
    return info


def writable(exe):
    """置いてある場所に書き込めるか (Program Files などだと入れ替えられない)。"""
    d = os.path.dirname(exe) or "."
    try:
        probe = os.path.join(d, ".zahyou_write_test")
        with open(probe, "wb") as f:
            f.write(b"0")
        os.remove(probe)
        return True
    except OSError:
        return False


def download(info, log, progress=None, stop=None, timeout=120):
    """
    新しい exe を「zahyou.exe.new」として横へ落とし、中身を確かめる。
    戻り値は置いた場所 (失敗なら None)。
    """
    exe = running_exe()
    if exe is None:
        log("ソースから動かしているので、更新できません。")
        return None
    dest = exe + ".new"
    total = info["size"]
    log(f"新しい版を落としています ({total / 1e6:.0f} MB)...")
    done = 0
    try:
        req = urllib.request.Request(info["url"], headers=UA)
        with urllib.request.urlopen(req, timeout=timeout) as r, \
                open(dest, "wb") as f:
            while True:
                if stop is not None and stop():
                    log("中止しました。")
                    f.close()
                    os.remove(dest)
                    return None
                chunk = r.read(1 << 20)
                if not chunk:
                    break
                f.write(chunk)
                done += len(chunk)
                if progress:
                    progress(done, total)
    except Exception as e:
        log(f"落とせませんでした: {type(e).__name__}: {e}")
        if os.path.exists(dest):
            os.remove(dest)
        return None

    got = os.path.getsize(dest)
    if total and got != total:
        log(f"大きさが違います ({got} != {total})。捨てます。")
        os.remove(dest)
        return None
    if info.get("sha256"):
        log("中身を確かめています...")
        h = sha256_file(dest, stop=stop)
        if h != info["sha256"]:
            log(f"中身が違います。捨てます。\n  こちら {h}"
            f"\n  向こう {info['sha256']}")
            os.remove(dest)
            return None
        log(f"  SHA256 一致: {h[:16]}…")
    return dest


_SWAP_PS1 = """\
$ErrorActionPreference = 'Stop'
$target = {pid}
$cur = '{cur}'
$new = '{new}'
$bak = '{bak}'

for ($i = 0; $i -lt 240; $i++) {{
    if (-not (Get-Process -Id $target -ErrorAction SilentlyContinue)) {{ break }}
    Start-Sleep -Milliseconds 500
}}
Start-Sleep -Milliseconds 700

try {{
    if (Test-Path $bak) {{ Remove-Item $bak -Force }}
    Move-Item -LiteralPath $cur -Destination $bak -Force
    Move-Item -LiteralPath $new -Destination $cur -Force
}} catch {{
    # 戻せるなら戻す (消えて無くなることは無いように)
    if ((-not (Test-Path $cur)) -and (Test-Path $bak)) {{
        Move-Item -LiteralPath $bak -Destination $cur -Force
    }}
    Start-Process -FilePath $cur
    exit 1
}}
Start-Process -FilePath $cur
"""


def apply_update(new_file, log, relaunch=True):
    """
    入れ替えを別プロセスに任せて起動する。
    True を返したら、呼んだ側はすぐアプリを閉じること。
    """
    exe = running_exe()
    if exe is None or not new_file or not os.path.exists(new_file):
        log("入れ替えるものがありません。")
        return False
    if os.name != "nt":
        log("Windows 以外では入れ替えできません。")
        return False

    script = _SWAP_PS1.format(pid=os.getpid(), cur=exe, new=new_file,
                              bak=exe + ".bak")
    if not relaunch:                    # 動作確認のとき (勝手に窓を開かせない)
        script = script.replace("Start-Process -FilePath $cur", "")
    path = os.path.join(tempfile.gettempdir(),
                        f"zahyou_update_{int(time.time())}.ps1")
    with open(path, "w", encoding="utf-8-sig", newline="\r\n") as f:
        f.write(script)
    log("入れ替えて、起動し直します...")
    try:
        subprocess.Popen(
            ["powershell.exe", "-NoProfile", "-WindowStyle", "Hidden",
             "-ExecutionPolicy", "Bypass", "-File", path],
            creationflags=_NO_WINDOW)
    except Exception as e:
        log(f"入れ替えを始められませんでした: {type(e).__name__}: {e}")
        return False
    return True


if __name__ == "__main__":                  # 手で確かめたいとき
    st = check()
    for k, v in st.items():
        print(f"{k:12s} {v}")
