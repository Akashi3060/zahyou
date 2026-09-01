"""
zahyou : オフライン解析に必要なものを調べて、足りなければ入れる。

GUI から呼ばれる。ここには画面のコードを一切書かない (log コールバックだけ渡す)。

    st = survey()                     # 今どうなっているか
    install_astrometry(log)           # 足りないものを入れる
    fetch_index(scales, dest, log)    # 星図データを落とす
    write_cfg(index_dir, log)

やること自体は dev/setup_wsl.ps1 と同じ。GUI から進み具合を出したいので
Python に置き換えてある。
"""
from __future__ import annotations

import base64
import math
import os
import re
import subprocess
import sys
import time
import urllib.request

# --- 画角の段 (astrometry.net の付番規則。末尾 2 桁が画角 [分角]) -------------
SCALE_FOV = {
    4107: (22, 30), 4108: (30, 42), 4109: (42, 60), 4110: (60, 85),
    4111: (85, 120), 4112: (120, 170), 4113: (170, 240), 4114: (240, 340),
    4115: (340, 480), 4116: (480, 680), 4117: (680, 1000),
    4118: (1000, 1400), 4119: (1400, 2000),
    5200: (2, 2.8), 5201: (2.8, 4), 5202: (4, 5.6), 5203: (5.6, 8),
    5204: (8, 11), 5205: (11, 16), 5206: (16, 22),
}
# おおよその容量 [MB]。選ぶ前に見当が付くように。
SCALE_MB = {
    4107: 158, 4108: 79, 4109: 40, 4110: 20, 4111: 10, 4112: 5, 4113: 3,
    4114: 2, 4115: 1, 4116: 1, 4117: 1, 4118: 1, 4119: 1,
    5200: 15900, 5201: 9300, 5202: 4900, 5203: 2400, 5204: 1200,
    5205: 620, 5206: 320,
}
LITE_BASE = "https://portal.nersc.gov/project/cosmo/temp/dstn/index-5200/LITE"
G41_BASE = "http://data.astrometry.net/4100"
TILES = 48                       # 5200 系は 48 分割 (HEALPix nside=2)

DEFAULT_INDEX_DIR = (r"C:\AstrometryData" if os.name == "nt"
                     else "/usr/share/astrometry")
CPU_LIMIT = 300                  # /etc/astrometry.cfg に書く値 [秒]

# GUI (--windowed) から呼ぶので、コンソール窓を出さない
_NO_WINDOW = 0x08000000 if os.name == "nt" else 0


def human(n):
    for unit in ("B", "KB", "MB", "GB"):
        if abs(n) < 1024:
            return f"{n:,.1f} {unit}"
        n /= 1024
    return f"{n:.1f} TB"


def index_names(scale):
    """その段に含まれるファイル名の一覧。"""
    if scale >= 5200:
        return [f"index-{scale}-{t:02d}.fits" for t in range(TILES)]
    return [f"index-{scale}.fits"]


def index_url(scale, tile=None):
    if scale >= 5200:
        return f"{LITE_BASE}/index-{scale}-{tile:02d}.fits"
    return f"{G41_BASE}/index-{scale}.fits"


# ============================================================ WSL を叩く ===

def _env():
    e = dict(os.environ)
    e["WSL_UTF8"] = "1"          # 既定の UTF-16 だと Python 側で化ける
    return e


def wsl(command, as_root=False, timeout=600):
    """WSL の中で sh -c を回す。戻り値 (returncode, 出力)"""
    if os.name != "nt":
        p = subprocess.run(["sh", "-c", command], capture_output=True,
                           text=True, encoding="utf-8", errors="replace",
                           timeout=timeout)
        return p.returncode, ((p.stdout or "") + (p.stderr or "")).strip()
    args = (["wsl.exe"] + (["-u", "root"] if as_root else [])
            + ["-e", "sh", "-c", command])
    try:
        p = subprocess.run(args, capture_output=True, text=True,
                           encoding="utf-8", errors="replace", timeout=timeout,
                           env=_env(), creationflags=_NO_WINDOW)
    except FileNotFoundError:
        return 127, "wsl.exe がありません"
    except subprocess.TimeoutExpired:
        return 124, f"{timeout} 秒で応答がありませんでした"
    return p.returncode, ((p.stdout or "") + (p.stderr or "")).strip()


def wsl_stream(command, log, as_root=False, timeout=3600):
    """出力を 1 行ずつ log に流しながら回す。apt のような長い処理向け。"""
    if os.name != "nt":
        args = ["sh", "-c", command]
    else:
        args = (["wsl.exe"] + (["-u", "root"] if as_root else [])
                + ["-e", "sh", "-c", command])
    try:
        p = subprocess.Popen(args, stdout=subprocess.PIPE,
                             stderr=subprocess.STDOUT, text=True,
                             encoding="utf-8", errors="replace",
                             env=_env() if os.name == "nt" else None,
                             creationflags=_NO_WINDOW)
    except FileNotFoundError:
        log("wsl.exe がありません。")
        return 127
    t0 = time.time()
    for line in p.stdout:
        line = line.rstrip()
        if line:
            log("    " + line)
        if time.time() - t0 > timeout:
            p.kill()
            log(f"    ⚠️ {timeout} 秒を超えたので打ち切りました。")
            return 124
    return p.wait()


def to_wsl_path(win_path):
    if os.name != "nt":
        return win_path
    code, out = wsl("wslpath -a -u '%s'" % win_path, timeout=60)
    if code == 0 and out:
        return out.splitlines()[0].strip()
    drive, rest = os.path.splitdrive(os.path.abspath(win_path))
    return "/mnt/" + drive[0].lower() + rest.replace("\\", "/")


# ============================================================== 状態調べ ===

def index_state(index_dir):
    """Windows 側の星図データの置き場所を調べる。"""
    st = {"dir": index_dir, "files": 0, "bytes": 0, "scales": [],
          "fov": None, "ok": False}
    if not os.path.isdir(index_dir):
        return st
    scales = set()
    for name in os.listdir(index_dir):
        m = re.match(r"index-(\d{4})(?:-\d{2})?\.fits$", name)
        if not m:
            continue
        st["files"] += 1
        try:
            st["bytes"] += os.path.getsize(os.path.join(index_dir, name))
        except OSError:
            pass
        scales.add(int(m.group(1)))
    st["scales"] = sorted(scales)
    covered = [SCALE_FOV[s] for s in st["scales"] if s in SCALE_FOV]
    if covered:
        st["fov"] = (min(a for a, _ in covered), max(b for _, b in covered))
    st["ok"] = st["files"] > 0
    return st


def survey(index_dir=None):
    """4 つの前提を順に確かめる。GUI の一覧表はこの戻り値をそのまま出す。"""
    index_dir = index_dir or DEFAULT_INDEX_DIR
    st = {"wsl": False, "distro": "", "solver": "", "cfg": "", "cfg_paths": [],
          "index": index_state(index_dir), "visible": 0, "ready": False,
          "notes": []}

    code, out = wsl("echo ok", timeout=90)
    st["wsl"] = (code == 0 and "ok" in out)
    if not st["wsl"]:
        st["notes"].append("WSL が使えません。「WSL を入れる」を押してください。")
        return st
    st["distro"] = wsl(". /etc/os-release; echo $PRETTY_NAME", timeout=90)[1]

    code, out = wsl("command -v solve-field", timeout=120)
    st["solver"] = out if code == 0 else ""
    if not st["solver"]:
        st["notes"].append("WSL の中に astrometry.net (solve-field) がありません。")
        return st

    st["cfg"] = wsl("cat /etc/astrometry.cfg 2>/dev/null", timeout=90)[1]
    st["cfg_paths"] = [ln.split(None, 1)[1].strip()
                       for ln in st["cfg"].splitlines()
                       if ln.strip().startswith("add_path")
                       and len(ln.split(None, 1)) > 1]

    if not st["index"]["ok"]:
        st["notes"].append(f"{index_dir} に星図データ (index-*.fits) がありません。")
        return st

    want = to_wsl_path(index_dir)
    if not any(p.rstrip("/") == want.rstrip("/") for p in st["cfg_paths"]):
        st["notes"].append("/etc/astrometry.cfg が星図データの場所を指していません。")
        return st

    code, out = wsl("ls -1 '%s'/index-*.fits 2>/dev/null | wc -l" % want,
                    timeout=300)
    digits = out.strip().splitlines()[-1].strip() if out.strip() else ""
    st["visible"] = int(digits) if digits.isdigit() else 0
    if st["visible"] == 0:
        st["notes"].append("WSL から星図データが見えません。")
        return st

    st["ready"] = True
    return st


# ================================================================ 直す側 ===

def install_wsl(log):
    """
    `wsl --install` は管理者権限が要るので、UAC を出して PowerShell に任せる。
    終わったら Windows の再起動が必要。
    """
    if os.name != "nt":
        log("Windows ではないので何もしません。")
        return False
    log("WSL を入れます。UAC (管理者の確認) が出たら「はい」を押してください。")
    ps = ('Start-Process -FilePath wsl.exe '
          '-ArgumentList "--install","--no-launch" -Verb RunAs -Wait')
    p = subprocess.run(["powershell.exe", "-NoProfile", "-ExecutionPolicy",
                        "Bypass", "-Command", ps],
                       capture_output=True, text=True, encoding="utf-8",
                       errors="replace", creationflags=_NO_WINDOW)
    if p.returncode != 0:
        log("  失敗しました: " + (p.stderr or "").strip()[:400])
        log("  管理者の PowerShell で  wsl --install  を実行してください。")
        return False
    log("  インストールを実行しました。")
    log("  ★ Windows を再起動してから、もう一度「状態を確認」を押してください。")
    return True


def install_astrometry(log):
    """WSL の中に astrometry.net を入れる。何度実行してもよい。"""
    code, out = wsl("command -v solve-field", timeout=120)
    if code == 0 and out:
        log(f"  既に入っています: {out}")
        return True
    log("  apt で astrometry.net を入れます (回線しだいで数分かかります)...")
    cmd = ("export DEBIAN_FRONTEND=noninteractive; "
           "apt-get update -qq && apt-get install -y astrometry.net netpbm")
    rc = wsl_stream(cmd, log, as_root=True, timeout=2400)
    if rc != 0:
        log(f"  apt が失敗しました (終了コード {rc})。")
        log("  WSL のターミナルで  sudo apt update && sudo apt install astrometry.net -y")
        return False
    code, out = wsl("command -v solve-field", timeout=120)
    if code != 0 or not out:
        log("  入れたはずですが solve-field が見つかりません。")
        return False
    log(f"  入りました: {out}")
    return True


def write_cfg(index_dir, log, cpu_limit=CPU_LIMIT):
    """/etc/astrometry.cfg を書く。元のファイルは 1 度だけ退避する。"""
    wsl_dir = to_wsl_path(index_dir)
    if not wsl_dir:
        log("  星図データの場所を WSL 用のパスに変換できませんでした。")
        return False
    cfg = f"inparallel\ncpulimit {cpu_limit}\nautoindex\nadd_path {wsl_dir}\n"
    b64 = base64.b64encode(cfg.encode("utf-8")).decode("ascii")
    cmd = ("if [ -f /etc/astrometry.cfg ] && [ ! -f /etc/astrometry.cfg.bak-zahyou ]; "
           "then cp /etc/astrometry.cfg /etc/astrometry.cfg.bak-zahyou; fi; "
           f"echo '{b64}' | base64 -d > /etc/astrometry.cfg; cat /etc/astrometry.cfg")
    code, out = wsl(cmd, as_root=True, timeout=180)
    if code != 0:
        log("  書き込めませんでした: " + out[:300])
        return False
    for line in out.splitlines():
        log("    " + line)
    return True


# ============================================================ 星図データ ===

def fov_arcmin(focal_mm, sensor_mm):
    """焦点距離とセンサー横幅から画角 [分角] を出す。"""
    if not focal_mm or not sensor_mm:
        return None
    return math.degrees(2 * math.atan(sensor_mm / 2.0 / focal_mm)) * 60.0


def recommend_scales(fov, fov_short=None):
    """
    その画角を解くのに要る段。導入誤差ぶん、前後 1 段ずつ足す。

    fov_short を渡すと、短辺の画角から長辺の画角までをまとめて覆う。
    掩蔽観測では、ローリングシャッターの影響を減らしコマ落ちを避けるために
    読み出す範囲 (特に縦) を切り詰めるので、短辺だけが極端に狭くなる。
    quad の大きさは短いほうの辺で決まるため、横幅だけで選ぶと足りない。
    """
    if not fov or fov <= 0:
        return []
    lo_fov = min(fov, fov_short) if fov_short else fov
    hi_fov = max(fov, fov_short) if fov_short else fov
    keys = sorted(SCALE_FOV)
    hit = [s for s in keys
           if SCALE_FOV[s][1] >= lo_fov and SCALE_FOV[s][0] <= hi_fov]
    if not hit:                                   # 範囲外なら一番近い段
        hit = [min(keys, key=lambda s: min(abs(SCALE_FOV[s][0] - lo_fov),
                                           abs(SCALE_FOV[s][1] - hi_fov)))]
    out = set(hit)
    for s in (hit[0], hit[-1]):
        i = keys.index(s)
        for j in (i - 1, i + 1):
            if 0 <= j < len(keys):
                out.add(keys[j])
    return sorted(out)


def _remote_size(url, timeout=60):
    req = urllib.request.Request(url, method="HEAD")
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return int(r.headers["Content-Length"])


def _download(url, path, log, on_bytes=None, stop=None, retries=5):
    """途中まで落ちていれば Range で続きから。戻り値 True/False"""
    try:
        total = _remote_size(url)
    except Exception as e:
        log(f"    {os.path.basename(path)}: 大きさを取れません ({e})")
        return False
    if os.path.exists(path) and os.path.getsize(path) == total:
        return True

    for attempt in range(1, retries + 1):
        have = os.path.getsize(path) if os.path.exists(path) else 0
        if have == total:
            return True
        if have > total:                          # 壊れている
            os.remove(path)
            have = 0
        req = urllib.request.Request(url)
        if have:
            req.add_header("Range", f"bytes={have}-")
        try:
            with urllib.request.urlopen(req, timeout=120) as r, \
                    open(path, "ab" if have else "wb") as f:
                while True:
                    if stop is not None and stop():
                        return False
                    chunk = r.read(1 << 20)
                    if not chunk:
                        break
                    f.write(chunk)
                    if on_bytes:
                        on_bytes(len(chunk))
            if os.path.getsize(path) == total:
                return True
        except Exception as e:
            if stop is not None and stop():
                return False
            log(f"    {os.path.basename(path)}: {type(e).__name__} "
                f"({attempt}/{retries}) 続きから再開します")
            time.sleep(min(2 ** attempt, 20))
    log(f"    {os.path.basename(path)}: あきらめました。")
    return False


def plan_size(scales, dest):
    """まだ落ちていないぶんの、おおよその容量 [byte]。"""
    need = 0
    for s in scales:
        per = SCALE_MB.get(s, 100) * 1024 * 1024 / len(index_names(s))
        for name in index_names(s):
            path = os.path.join(dest, name)
            if not os.path.exists(path):
                need += per
    return int(need)


def fetch_index(scales, dest, log, progress=None, stop=None):
    """
    選んだ段の index ファイルを dest へ落とす。

    progress(done, total) が呼ばれる (byte)。stop() が True を返したら止める。
    """
    os.makedirs(dest, exist_ok=True)
    jobs = []
    for s in sorted(scales):
        for i, name in enumerate(index_names(s)):
            jobs.append((s, i if s >= 5200 else None, name))
    todo = [j for j in jobs if not os.path.exists(os.path.join(dest, j[2]))]
    if not todo:
        log("  すべて落ちています。")
        return True

    total = plan_size(scales, dest)
    log(f"  {len(todo)} ファイル / 約 {human(total)} を {dest} へ落とします。")
    done = [0]

    def on_bytes(n):
        done[0] += n
        if progress:
            progress(done[0], total)

    ok = 0
    for n, (s, tile, name) in enumerate(jobs, 1):
        if stop is not None and stop():
            log("  中止しました。")
            return False
        path = os.path.join(dest, name)
        fresh = not os.path.exists(path)
        if _download(index_url(s, tile), path, log, on_bytes, stop):
            ok += 1
            if fresh:
                log(f"    [{n}/{len(jobs)}] {name}  "
                    f"{human(os.path.getsize(path))}")
        elif stop is not None and stop():
            log("  中止しました。")
            return False
    log(f"  完了: {ok}/{len(jobs)} ファイル")
    return ok == len(jobs)


# ============================================================== まとめて ===

def prepare_all(index_dir, scales, log, progress=None, stop=None):
    """
    「まとめて準備」。足りない工程だけを順に実行する。戻り値は survey()。
    """
    log("=" * 56)
    log("1. WSL")
    st = survey(index_dir)
    if not st["wsl"]:
        install_wsl(log)
        return survey(index_dir)
    log(f"  OK  {st['distro']}")

    log("=" * 56)
    log("2. astrometry.net")
    if not st["solver"]:
        if not install_astrometry(log):
            return survey(index_dir)
    else:
        log(f"  OK  {st['solver']}")

    log("=" * 56)
    log("3. 星図データ")
    have = index_state(index_dir)
    need = [s for s in scales if s not in have["scales"]]
    if need:
        log("  足りない段: " + ", ".join(str(s) for s in need))
        if not fetch_index(need, index_dir, log, progress, stop):
            return survey(index_dir)
    else:
        log(f"  OK  {have['files']} ファイル / {human(have['bytes'])}")

    log("=" * 56)
    log("4. /etc/astrometry.cfg")
    write_cfg(index_dir, log)

    log("=" * 56)
    log("5. 確認")
    st = survey(index_dir)
    if st["ready"]:
        log(f"  準備できました。WSL から {st['visible']} ファイルが見えています。")
        fov = st["index"]["fov"]
        if fov:
            log(f"  解ける画角: {fov[0]:g}′ 〜 {fov[1]:g}′")
    else:
        for m in st["notes"]:
            log("  ⚠️ " + m)
    return st


if __name__ == "__main__":                        # 手で確かめたいとき
    s = survey(sys.argv[1] if len(sys.argv) > 1 else None)
    for k, v in s.items():
        print(f"{k:10s} {v}")
