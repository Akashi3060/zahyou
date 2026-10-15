"""
zahyou : オフライン解析に必要なもの (星図データ) を調べて、足りなければ落とす。

GUI から呼ばれる。ここには画面のコードを一切書かない (log コールバックだけ渡す)。

    st = survey(index_dir)                # 今どうなっているか (走査と検査)
    fetch_index(scales, dest, log)        # 星図データを落とす
    prepare_all(index_dir, scales, log)   # 足りない段を落として、検査する

v6 までは WSL と astrometry.net もここで入れていたが、v7 からは解析を zahyou 自身が
行う (dev/solver_*.py)。要るのは星図データ (astrometry.net 形式の index-*.fits) だけ。
"""
from __future__ import annotations

import concurrent.futures
import math
import os
import re
import shutil
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

_INDEX_RE = re.compile(r"index-(\d{4})(?:-(\d{2}))?\.fits$")


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


# ============================================================ 中身の検査 ===
#  解析エンジンの目録 (SvIndexCatalog) は exec で読むエンジンの中にあり、
#  起動の直後やダウンロードの最中には使えない。ここではヘッダーだけを読む
#  軽い検査を自前で持つ (形式は docs/index-format.md の 2〜4 節)。
#  エンジンの検査 (sv_validate_index_file) は validate として受け取れる。

_BLOCK = 2880
_ENDIAN = "04:03:02:01"           # リトルエンディアン。ほかの値の索引は読めない
_CHECKED = {}                     # (パス, 大きさ, 更新時刻, エンジンも見たか) → (ok, 理由)


def _card_value(text):
    """カードの値の部分 (文字列なら引用符の中、ほかは / の注記を除いたもの)。"""
    t = text.strip()
    if t.startswith("'"):
        end = t.find("'", 1)
        return (t[1:end] if end > 0 else t[1:]).rstrip()
    return t.split("/", 1)[0].strip()


def _read_header(f):
    """FITS のヘッダーを 1 つ読む (2880 バイトのブロック単位)。END が無ければ ValueError。"""
    cards = {}
    while True:
        block = f.read(_BLOCK)
        if len(block) < _BLOCK:
            raise ValueError("ヘッダーの途中でファイルが終わっています")
        for i in range(0, _BLOCK, 80):
            card = block[i:i + 80].decode("ascii", "replace")
            key = card[:8].strip()
            if key == "END":
                return cards
            if card[8:10] == "= " and key not in cards:
                cards[key] = _card_value(card[10:])


def _data_bytes(h):
    """そのヘッダーに続くデータの長さ [バイト] (2880 の倍数に詰める前)。"""
    naxis = int(h.get("NAXIS", "0") or 0)
    if naxis == 0:
        return 0
    n = 1
    for i in range(1, naxis + 1):
        n *= int(h.get(f"NAXIS{i}", "0") or 0)
    n += int(h.get("PCOUNT", "0") or 0)
    n *= int(h.get("GCOUNT", "1") or 1)
    return abs(int(h.get("BITPIX", "8"))) // 8 * n


def _padded(n):
    return (n + _BLOCK - 1) // _BLOCK * _BLOCK


def _check_layout(path):
    """ヘッダーだけで検査する。戻り値 (ok, 理由)。"""
    m = _INDEX_RE.match(os.path.basename(path))
    if not m:
        return False, "索引ファイルの名前ではありません"
    try:
        size = os.path.getsize(path)
        with open(path, "rb") as f:
            prim = _read_header(f)
            pos = f.tell() + _padded(_data_bytes(prim))
            while pos < size:
                f.seek(pos)
                h = _read_header(f)
                pos = f.tell() + _padded(_data_bytes(h))
    except (OSError, ValueError) as e:
        return False, str(e)
    if pos != size:
        return False, (f"大きさが合いません (ヘッダーから {pos:,} バイト、"
                       f"実物 {size:,} バイト。途中で切れたファイルです)")
    try:
        if prim.get("ENDIAN") != _ENDIAN:
            return False, f"エンディアンが違います ({prim.get('ENDIAN')})"
        if int(prim.get("INDEXID", "-1")) != int(m.group(1)):
            return False, f"INDEXID が名前と違います ({prim.get('INDEXID')})"
        if m.group(2) is not None and int(prim.get("HEALPIX", "-1")) != int(m.group(2)):
            return False, f"HEALPIX がタイル番号と違います ({prim.get('HEALPIX')})"
    except ValueError as e:
        return False, f"ヘッダーの値を読めません ({e})"
    return True, "ok"


def check_index_file(path, validate=None):
    """
    索引ファイル 1 本を検査する。戻り値 (ok, 理由)。

    見るのは、大きさがヘッダーから計算した長さと一致するか (途中で切れたダウンロードは
    ここで分かる)、ENDIAN、INDEXID とファイル名の番号、5200 系の HEALPIX とタイル番号。
    中身 (星や quad) までは読まないので、253 本でも 1 秒かからない。
    validate (エンジンの sv_validate_index_file) を渡すと、こちらの検査に通ったあとで
    それも通す。同じファイルは大きさと更新時刻が変わるまで調べ直さない。
    """
    try:
        st = os.stat(path)
    except OSError as e:
        return False, str(e)
    key = (os.path.normcase(os.path.abspath(path)), st.st_size, st.st_mtime_ns,
           validate is not None)
    hit = _CHECKED.get(key)
    if hit is not None:
        return hit
    ok, why = _check_layout(path)
    if ok and validate is not None:
        try:
            ok, msg = validate(path)
            if not ok:
                why = str(msg)
        except Exception as e:
            ok, why = False, f"検査でエラー ({type(e).__name__}: {e})"
    _CHECKED[key] = (ok, why)
    return ok, why


# ============================================================== 状態調べ ===

def index_state(index_dir):
    """
    星図データの置き場所を走査する (中身は見ない。中身の検査は survey)。

      files / bytes : index-*.fits の数と合計の大きさ
      scales        : ファイルが全部そろっている段 (5200 系は 48 タイル)
      partial       : 一部だけある段 {段: ある数}。落とすのを途中で止めたもの
      fov           : ファイルがある段の画角の範囲 (分角)
    """
    st = {"dir": index_dir, "files": 0, "bytes": 0, "scales": [], "partial": {},
          "fov": None, "ok": False, "names": []}
    if not os.path.isdir(index_dir):
        return st
    count = {}
    for name in sorted(os.listdir(index_dir)):
        m = _INDEX_RE.match(name)
        if not m:
            continue
        st["files"] += 1
        st["names"].append(name)
        try:
            st["bytes"] += os.path.getsize(os.path.join(index_dir, name))
        except OSError:
            pass
        s = int(m.group(1))
        count[s] = count.get(s, 0) + 1
    st["scales"] = sorted(s for s, n in count.items() if n >= len(index_names(s)))
    st["partial"] = {s: n for s, n in sorted(count.items())
                     if n < len(index_names(s))}
    covered = [SCALE_FOV[s] for s in count if s in SCALE_FOV]
    if covered:
        st["fov"] = (min(a for a, _ in covered), max(b for _, b in covered))
    st["ok"] = st["files"] > 0
    return st


def survey(index_dir=None, validate=None):
    """
    星図データを走査して、1 本ずつ検査する。GUI の一覧表はこの戻り値をそのまま出す。

      index : 置き場所の走査 (index_state)。scales は「全部そろって検査に通った段」
      check : 検査の結果 {"ok", "checked", "bad": [(名前, 理由)]}
      ready : オフライン解析できるか (検査に通った星図データが 1 本でもある)
      notes : 足りないもの・壊れているもの (画面とログにそのまま出す)
    """
    index_dir = index_dir or DEFAULT_INDEX_DIR
    idx = index_state(index_dir)
    bad = []
    for name in idx["names"]:
        ok, why = check_index_file(os.path.join(index_dir, name), validate)
        if not ok:
            bad.append((name, why))
    bad_scales = {int(_INDEX_RE.match(n).group(1)) for n, _ in bad}
    idx["scales"] = [s for s in idx["scales"] if s not in bad_scales]
    good = idx["files"] - len(bad)
    st = {"index": idx,
          "check": {"ok": idx["files"] > 0 and not bad, "checked": idx["files"],
                    "bad": bad},
          "ready": good > 0, "notes": []}

    notes = st["notes"]
    if not idx["ok"]:
        notes.append(f"{index_dir} に星図データ (index-*.fits) がありません。")
    for s, n in idx["partial"].items():
        notes.append(f"index-{s} は {len(index_names(s))} ファイル中 {n} しかありません "
                     "(落とすのを途中で止めたもの)。もう一度落とせば続きから補います。")
    if bad:
        notes.append(f"中身のおかしい星図データが {len(bad)} 個あります (解析では読み飛ばします)。"
                     "その段を落とし直してください。")
        for name, why in bad[:5]:
            notes.append(f"  {name}: {why}")
        if len(bad) > 5:
            notes.append(f"  ほか {len(bad) - 5} 個")
    return st


# ============================================================ 星図データ ===

def fov_arcmin(focal_mm, sensor_mm):
    """焦点距離とセンサー横幅から画角 [分角] を出す。"""
    if not focal_mm or not sensor_mm:
        return None
    return math.degrees(2 * math.atan(sensor_mm / 2.0 / focal_mm)) * 60.0


# 焦点距離が分からない人向けの「おすすめ」= 画角 4′ 以上をぜんぶ。
# 2′〜4′ の 5200 / 5201 は 2 段で 25 GB あり、掩蔽観測で画面を大きく
# 切り出す人しか使わないので入れていない。
RECOMMENDED_MIN_FOV = 4.0


def recommended_scales():
    """おすすめの段 (画角 4′ 以上)。焦点距離を聞かずに決められる。"""
    return sorted(s for s, (lo, _hi) in SCALE_FOV.items()
                  if lo >= RECOMMENDED_MIN_FOV)


def scales_mb(scales):
    """その段ぜんぶで何 MB になるか。"""
    return sum(SCALE_MB.get(s, 0) for s in scales)


def recommend_scales(fov, fov_short=None):
    """
    その画角を解くのに要る段。導入誤差ぶん、前後 1 段ずつ足す。

    fov_short を渡すと、短辺の画角から長辺の画角までをまとめて覆う。
    掩蔽観測では、ローリングシャッターの影響を減らしコマ落ちを避けるために
    読み出す範囲 (特に縦) を切り詰めるので、短辺だけが極端に狭くなる。
    quad の大きさは短いほうの辺で決まるため、横幅だけで選ぶと足りない。

    「前後 1 段」は画角の順で数える。段の番号順ではない。
    4100 系 (22′〜2000′) と 5200 系 (2′〜22′) は別のはしごなので、
    番号で並べると 4119 (1400′〜2000′) の隣が 5200 (2′〜2.8′) になってしまう。
    実際、35mm レンズ (画角 54°) で 15.9 GB の 5200 を勧めていた。
    """
    if not fov or fov <= 0:
        return []
    lo_fov = min(fov, fov_short) if fov_short else fov
    hi_fov = max(fov, fov_short) if fov_short else fov
    keys = sorted(SCALE_FOV, key=lambda s: SCALE_FOV[s])
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


def _finish(part, path, log, validate=None):
    """落とし終えた .part を本来の名前へ付け替えて、ヘッダーを検査する (validate は survey と同じ検査)。"""
    name = os.path.basename(path)
    try:
        os.replace(part, path)
    except OSError as e:
        log(f"    {name}: 名前を付け替えられません ({e})。"
            "解析で使っている最中なら、終わってからもう一度押してください。")
        return False
    ok, why = check_index_file(path, validate)
    if ok:
        return True
    log(f"    {name}: 落としたファイルの中身がおかしいので消します ({why})。")
    try:
        os.remove(path)
    except OSError:
        pass
    return False


def _download(url, path, log, on_bytes=None, stop=None, retries=5, validate=None):
    """
    途中まで落ちていれば Range で続きから。戻り値 True/False

    書き込みは「名前.part」へ。最後まで落ちて大きさが合い、ヘッダーの検査に
    通ってから本来の名前へ付け替える。v6 は本来の名前へ直接書いていたので、
    途中で止めると中途半端なファイルが残り、オフラインの「状態を確認」が
    そろっていると誤って答えた。validate (エンジンの検査) は survey と同じものを渡す
    (渡さないと、エンジンの検査だけで落ちるファイルを「完成済み」として何度押しても落とし直さない)。
    """
    name = os.path.basename(path)
    part = path + ".part"
    try:
        total = _remote_size(url)
    except Exception as e:
        log(f"    {name}: 大きさを取れません ({e})")
        return False
    if os.path.exists(path):
        if os.path.getsize(path) == total:
            ok, why = check_index_file(path, validate)
            if ok:
                return True
            log(f"    {name}: 中身がおかしいので落とし直します ({why})")
            try:
                os.remove(path)
            except OSError as e:
                log(f"    {name}: 消せません ({e})")
                return False
        else:                            # v6 が残した途中のファイル → 続きから
            try:
                if os.path.exists(part):
                    os.remove(path)
                else:
                    os.replace(path, part)
            except OSError as e:
                log(f"    {name}: 続きから落とす準備ができません ({e})")
                return False

    for attempt in range(1, retries + 1):
        have = os.path.getsize(part) if os.path.exists(part) else 0
        if have > total:                          # 壊れている
            os.remove(part)
            have = 0
        try:
            if have < total:
                req = urllib.request.Request(url)
                if have:
                    req.add_header("Range", f"bytes={have}-")
                with urllib.request.urlopen(req, timeout=120) as r:
                    if have and getattr(r, "status", 206) != 206:
                        have = 0                  # 続きから貰えなかったので最初から
                    with open(part, "ab" if have else "wb") as f:
                        while True:
                            if stop is not None and stop():
                                return False
                            chunk = r.read(1 << 20)
                            if not chunk:
                                break
                            f.write(chunk)
                            if on_bytes:
                                on_bytes(len(chunk))
            if os.path.getsize(part) == total:
                return _finish(part, path, log, validate)
        except Exception as e:
            if stop is not None and stop():
                return False
            log(f"    {name}: {type(e).__name__} "
                f"({attempt}/{retries}) 続きから再開します")
            time.sleep(min(2 ** attempt, 20))
    log(f"    {name}: あきらめました。")
    return False


def plan_size(scales, dest):
    """まだ落ちていないぶんの、おおよその容量 [byte]。確認画面の目安用。"""
    need = 0
    for s in scales:
        per = SCALE_MB.get(s, 100) * 1024 * 1024 / len(index_names(s))
        for name in index_names(s):
            if not os.path.exists(os.path.join(dest, name)):
                need += per
    return int(need)


def remote_sizes(jobs, log=None, stop=None, workers=8):
    """
    落とす予定のファイルの実サイズを HEAD で調べる。{名前: byte or None}

    表に持っている目安 (SCALE_MB) は 6% ほどずれていて、進み具合が
    100% に届かなかったり超えたりする。実物を聞いてから始める。
    """
    out = {}

    def one(job):
        s, tile, name = job
        try:
            return name, _remote_size(index_url(s, tile))
        except Exception:
            return name, None

    with concurrent.futures.ThreadPoolExecutor(workers) as ex:
        for name, size in ex.map(one, jobs):
            if stop is not None and stop():
                break
            out[name] = size
    return out


def fetch_index(scales, dest, log, progress=None, stop=None, validate=None):
    """
    選んだ段の index ファイルを dest へ落とす。

    progress(done, total) が呼ばれる (byte)。stop() が True を返したら止める。
    validate (エンジンの sv_validate_index_file) を渡すと、「完成済み」の判定と落としたあとの検査に
    survey と同じ検査を使う。

    数え方をひとつに揃えてある ―― 見出しの「N ファイル」、途中の [i/N]、
    最後の「完了 M/N」は、すべて<b>これから落とすぶん</b>の数。
    既にあるファイルを混ぜて数えると、表示がとびとびになって合わなくなる。
    """
    os.makedirs(dest, exist_ok=True)
    jobs = []
    for s in sorted(scales):
        for i, name in enumerate(index_names(s)):
            jobs.append((s, i if s >= 5200 else None, name))

    log("  必要な容量を調べています...")
    sizes = remote_sizes(jobs, log, stop)
    if stop is not None and stop():
        log("  中止しました。")
        return False

    todo, total, already = [], 0, 0
    for s, tile, name in jobs:
        path = os.path.join(dest, name)
        got = os.path.getsize(path) if os.path.exists(path) else 0
        if not got and os.path.exists(path + ".part"):
            got = os.path.getsize(path + ".part")       # 途中まで落ちているぶん
            done_file = False
        else:
            done_file = bool(got)
        want = sizes.get(name)
        if want is None:                    # 大きさを聞けなかった
            if done_file:                   # あるなら完成しているとみなす
                already += 1
                continue
            todo.append((s, tile, name))
            total += int(SCALE_MB.get(s, 100) * 1024 * 1024
                         / len(index_names(s)))
            continue
        if done_file and got == want:
            if check_index_file(path, validate)[0]:
                already += 1
                continue
            got = 0                         # 中身がおかしいので最初から落とし直す
        todo.append((s, tile, name))
        total += want - got if 0 < got < want else want   # 途中まで落ちているぶんは引く

    if not todo:
        log(f"  すべて落ちています ({already} ファイル)。")
        return True
    msg = f"  {len(todo)} ファイル / {human(total)} を {dest} へ落とします。"
    if already:
        msg += f"  (既にある {already} ファイルは飛ばします)"
    log(msg)
    try:
        free = shutil.disk_usage(dest).free
    except OSError:
        free = None
    if free is not None and free < total * 1.05 + (64 << 20):
        log(f"  空き容量が足りません (要 {human(total)} / 空き {human(free)})。"
            "置き場所を変えるか、ディスクを空けてください。")
        return False

    done = [0]

    def on_bytes(n):
        done[0] += n
        if progress:
            progress(done[0], total)

    ok = 0
    for i, (s, tile, name) in enumerate(todo, 1):
        if stop is not None and stop():
            log("  中止しました。")
            return False
        path = os.path.join(dest, name)
        if _download(index_url(s, tile), path, log, on_bytes, stop, validate=validate):
            ok += 1
            log(f"    [{i}/{len(todo)}] {name}  "
                f"{human(os.path.getsize(path))}")
        elif stop is not None and stop():
            log("  中止しました。")
            return False
    log(f"  完了: {ok}/{len(todo)} ファイル")
    return ok == len(todo)


# ============================================================== まとめて ===

def report(st, log):
    """survey() の結果をログに書く。"""
    idx, chk = st["index"], st["check"]
    log(f"  星図データ : "
        + (f"{idx['files']} ファイル / {human(idx['bytes'])}  ({idx['dir']})"
           if idx["ok"] else f"ありません  ({idx['dir']})"))
    if chk["checked"]:
        log("  検査       : "
            + (f"{chk['checked']} ファイルとも正常" if not chk["bad"]
               else f"{len(chk['bad'])} / {chk['checked']} ファイルに問題"))
    if idx["fov"]:
        log(f"  解ける画角 : {idx['fov'][0]:g}′ 〜 {idx['fov'][1]:g}′")
    for m in st["notes"]:
        log("  ⚠️ " + m)


def prepare_all(index_dir, scales, log, progress=None, stop=None, validate=None, allowed=None):
    """
    「まとめて準備」。足りない段を落として、検査する。戻り値は survey()。

    scales を渡せばその段を、空ならおすすめの段 (画角 4′ 以上) をそろえる。
    すでに全部そろって検査に通っている段は落とさない。
    allowed: 利用者に容量を見せて了承を得た段 (GUI が押したときに数え直したもの)。渡されていて、
    ここで数え直した足りない段がそれより多ければ、落とさずに止める (了承していない量を落とさない)。
    """
    log("=" * 56)
    log("1. 星図データ")
    st = survey(index_dir, validate)
    want = sorted(scales) if scales else recommended_scales()
    need = [s for s in want if s not in st["index"]["scales"]]
    extra = [s for s in need if allowed is not None and s not in allowed]
    if extra:
        log("  確認したあとで足りない段が増えました ("
            + ", ".join(str(s) for s in extra) + ")。落とさずに止めます。"
            "もう一度押すと、増えたぶんも含めて容量を確かめます。")
        report(st, log)
        return st
    if need:
        log("  足りない段: " + ", ".join(str(s) for s in need))
        if not fetch_index(need, index_dir, log, progress, stop, validate=validate):
            st = survey(index_dir, validate)
            report(st, log)
            return st
    else:
        idx = st["index"]
        log(f"  OK  {idx['files']} ファイル / {human(idx['bytes'])}")

    log("=" * 56)
    log("2. 検査")
    st = survey(index_dir, validate)
    report(st, log)
    if st["ready"]:
        log("  準備できました。オフライン解析できます。")
    return st


if __name__ == "__main__":                        # 手で確かめたいとき
    s = survey(sys.argv[1] if len(sys.argv) > 1 else None)
    report(s, print)
    print("ready", s["ready"])
