"""オンライン解析 (solve_online) が nova.astrometry.net へ送る中身の試験。ネットには出ない。

astroquery の送信の関数 (AstrometryNetClass._request) を偽物に差し替え、送るはずだった
request-json を取り出して中身を確かめる。返事も偽物 (ログイン → 受付 → 解けた → WCS) を返す。
本物の nova には 1 回も送らない (socket の接続と requests の送信も塞ぎ、呼ばれたら失敗にする)。

確かめること
  - 星の位置を送るとき (solve_from_source_list) も、画像を送るとき (solve_from_image) も
    publicly_visible = "n" (公開しない) が request-json に入っている
  - 星の位置で解けずに画像へ切り替えたときも、2 回とも "n"
  - photutils があるとき、solve_from_image は画像を送らず astroquery が自分で星を数えて位置を送る。そこも "n"
  - 送るのは星の位置・画像の大きさ・手がかり・公開の指定・セッションだけ (知らない項目が増えたら失敗)
  - API キーはログインのときだけ送り、解析の受付には入らない
  - 対照: astroquery に何も指定しないと publicly_visible は送られない (= サーバーの既定の「公開」になる)

  python test_online_settings.py              dev/zahyou_core.py と、組み立てた直下の zahyou_engine.py の両方で
  python test_online_settings.py --core-only  dev/zahyou_core.py だけで (組み立てる前に)

利用者の %LOCALAPPDATA%\\zahyou には触れない (LOCALAPPDATA・ZAHYOU_CACHE・ZAHYOU_SETTINGS を一時フォルダーに向ける)。
"""
import io
import json
import os
import shutil
import socket
import sys
import tempfile
import time
import types
import warnings

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
sys.path.insert(0, HERE)

_TMP = tempfile.mkdtemp(prefix="zahyou_online_test_")
os.environ["LOCALAPPDATA"] = _TMP
os.environ["ZAHYOU_CACHE"] = os.path.join(_TMP, "cache.json")
os.environ["ZAHYOU_SETTINGS"] = os.path.join(_TMP, "gui.json")

warnings.filterwarnings("ignore")

FAKE_KEY = "test-key-not-real-0000"     # 偽物。本物のキーはここに書かない
RESULTS = []
ALLOWED_KEYS = {"publicly_visible", "session", "x", "y", "image_width", "image_height",
                "center_ra", "center_dec", "radius",
                "scale_units", "scale_type", "scale_lower", "scale_upper"}


def record(name, ok, msg=""):
    RESULTS.append((name, bool(ok), msg))


# ------------------------------------------------------------ ネットを塞ぐ ---

NET_ATTEMPTS = []


def _blocked(*args, **kwargs):
    NET_ATTEMPTS.append(repr(args[1:2]))
    raise OSError("この試験ではネットに出ない (塞いである)")


socket.socket.connect = _blocked
socket.socket.connect_ex = _blocked
try:
    import requests.sessions
    requests.sessions.Session.request = _blocked
except ImportError:
    pass

import numpy as np                                          # noqa: E402
from astropy.io import fits                                 # noqa: E402
import astroquery.astrometry_net.core as anc               # noqa: E402

# monitor_submission は 1 秒ずつ待つので、待たないようにする (time.time はそのまま)
anc.time = types.SimpleNamespace(sleep=lambda s: None, time=time.time)


# ---------------------------------------------------------------- 偽の nova ---

class FakeResponse:
    def __init__(self, payload=None, text="", status_code=200):
        self._payload = payload
        self.text = text
        self.status_code = status_code

    def json(self):
        return self._payload


def _wcs_text():
    h = fits.Header()
    for k, v in (("WCSAXES", 2), ("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN"),
                 ("CRVAL1", 61.89), ("CRVAL2", 41.96), ("CRPIX1", 100.5), ("CRPIX2", 80.5),
                 ("CD1_1", -0.0004), ("CD1_2", 0.0), ("CD2_1", 0.0), ("CD2_2", 0.0004)):
        h[k] = v
    return h.tostring()


class FakeNova:
    """送られたものを覚え、受付番号ごとに決めた結果 ("success" / "failure") を返す。"""

    def __init__(self, outcomes=("success",)):
        self.outcomes = list(outcomes)
        self.calls = []          # (method, url, request-json の辞書 or None, files の名前 or None)
        self.subs = {}           # subid -> job の結果

    def request(self, client, method, url, params=None, data=None, headers=None,
                files=None, **kw):
        req = None
        if data and "request-json" in data:
            req = json.loads(data["request-json"])
        fname = None
        if files:
            f = files.get("file")
            fname = os.path.basename(getattr(f, "name", "") or "")
        self.calls.append((method, url, req, fname))
        tail = url.rstrip("/").split("/")
        if method == "POST" and tail[-1] == "login":
            return FakeResponse({"status": "success", "session": "fake-session"})
        if method == "POST" and tail[-1] in ("url_upload", "upload"):
            subid = 100 + len(self.subs)
            self.subs[subid] = self.outcomes.pop(0) if self.outcomes else "failure"
            return FakeResponse({"status": "success", "subid": subid})
        if method == "GET" and tail[-2] == "submissions":
            return FakeResponse({"jobs": [int(tail[-1]) + 1000]})
        if method == "GET" and tail[-1] == "info":
            return FakeResponse({"status": self.subs[int(tail[-2]) - 1000]})
        if method == "GET" and tail[-2] == "wcs_file":
            return FakeResponse(text=_wcs_text())
        raise AssertionError(f"思っていない問い合わせ: {method} {url}")

    def submissions(self):
        """解析の受付 (星の位置 / 画像) だけ。"""
        return [c for c in self.calls if c[0] == "POST" and c[1].rstrip("/").endswith(("url_upload", "upload"))]

    def logins(self):
        return [c for c in self.calls if c[0] == "POST" and c[1].rstrip("/").endswith("login")]


class patched_nova:
    """with の間だけ astroquery の送信を FakeNova へ向ける。photutils の有無も切り替えられる。"""

    def __init__(self, nova, no_source_detector=None):
        self.nova = nova
        self.nsd = no_source_detector

    def __enter__(self):
        cls = anc.AstrometryNetClass
        self._old_req = cls.__dict__.get("_request")
        self._old_nsd = cls._no_source_detector
        nova = self.nova
        cls._request = lambda client, *a, **k: nova.request(client, *a, **k)
        if self.nsd is not None:
            cls._no_source_detector = self.nsd
        return nova

    def __exit__(self, *exc):
        cls = anc.AstrometryNetClass
        if self._old_req is None:
            del cls._request
        else:
            cls._request = self._old_req
        cls._no_source_detector = self._old_nsd
        return False


# ------------------------------------------------------------- 試験の材料 ---

def make_star_fits(path, ny=160, nx=200, n=25, seed=7):
    """星を植えた小さな FITS (photutils の経路で星が見つかるように)。"""
    r = np.random.default_rng(seed)
    img = np.full((ny, nx), 300.0) + r.normal(0, 5, (ny, nx))
    yy, xx = np.mgrid[0:ny, 0:nx]
    for x, y, a in zip(r.uniform(10, nx - 10, n), r.uniform(10, ny - 10, n), r.uniform(800, 6000, n)):
        img += a * np.exp(-(((xx - x) ** 2 + (yy - y) ** 2) / (2 * 1.3 ** 2)))
    fits.PrimaryHDU(img.astype(np.float32)).writeto(path, overwrite=True)
    return path


SOURCES = [{"x": 12.25, "y": 30.5}, {"x": 150.0, "y": 44.75}, {"x": 77.5, "y": 120.0},
           {"x": 190.0, "y": 10.0}, {"x": 5.0, "y": 155.5}]
SHAPE = (160, 200)


def _check_submission(tag, sub, expect_kind, extra=None):
    """1 件の受付を確かめる。戻り値は (ok, 説明)。"""
    method, url, req, fname = sub
    if req is None:
        return False, "request-json が無い"
    probs = []
    if req.get("publicly_visible") != "n":
        probs.append(f"publicly_visible={req.get('publicly_visible')!r}")
    unknown = set(req) - ALLOWED_KEYS
    if unknown:
        probs.append(f"知らない項目 {sorted(unknown)}")
    if FAKE_KEY in json.dumps(req) or FAKE_KEY in url:
        probs.append("API キーが受付に入っている")
    kind = "画像" if url.rstrip("/").endswith("/upload") else "星の位置"
    if kind != expect_kind:
        probs.append(f"送ったのが{kind} ({expect_kind}のはず)")
    for k, v in (extra or {}).items():
        if req.get(k) != v:
            probs.append(f"{k}={req.get(k)!r} ({v!r} のはず)")
    desc = (f"{kind}: publicly_visible={req.get('publicly_visible')!r}、項目 "
            f"{sorted(k for k in req if k not in ('x', 'y'))}"
            + (f"、星 {len(req.get('x', []))} 個" if "x" in req else "")
            + (f"、ファイル名 {fname}" if fname else ""))
    return not probs, ("; ".join(probs) + " / " if probs else "") + desc


# ------------------------------------------------------------------ 試験 ---

def run_cases(label, solve_online, img_path):
    # 1. 星の位置を送る (いつもの経路)
    nova = FakeNova(["success"])
    with patched_nova(nova):
        hdr = solve_online(img_path, FAKE_KEY, sources=SOURCES, shape=SHAPE, timeout=30)
    subs = nova.submissions()
    ok, msg = (len(subs) == 1), f"受付 {len(subs)} 件"
    if subs:
        ok2, msg2 = _check_submission(label, subs[0], "星の位置",
                                      {"image_width": SHAPE[1], "image_height": SHAPE[0],
                                       "x": [s["x"] + 1.0 for s in SOURCES],
                                       "y": [s["y"] + 1.0 for s in SOURCES]})
        ok, msg = ok and ok2, msg2
    record(f"[{label}] 星の位置を送る → publicly_visible='n'", ok and bool(hdr), msg)

    # 2. 手がかり (目標の方向・画素スケール) を付けても公開の指定が残る
    nova = FakeNova(["success"])
    with patched_nova(nova):
        hdr = solve_online(img_path, FAKE_KEY, sources=SOURCES, shape=SHAPE, timeout=30,
                           hint_radec=(61.9, 41.9), hint_radius_deg=5.0, scale_hint=1.46)
    subs = nova.submissions()
    ok, msg = len(subs) == 1, f"受付 {len(subs)} 件"
    if subs:
        ok, msg = _check_submission(label, subs[0], "星の位置",
                                    {"center_ra": 61.9, "center_dec": 41.9, "radius": 5.0,
                                     "scale_units": "arcsecperpix", "scale_type": "ul"})
    record(f"[{label}] 手がかり付きでも publicly_visible='n'", ok and bool(hdr), msg)

    # 3. 画像そのものを送る (photutils が無いときの astroquery の経路)
    nova = FakeNova(["success"])
    with patched_nova(nova, no_source_detector=True):
        hdr = solve_online(img_path, FAKE_KEY, sources=None, shape=SHAPE, timeout=30)
    subs = nova.submissions()
    ok, msg = len(subs) == 1, f"受付 {len(subs)} 件"
    if subs:
        ok, msg = _check_submission(label, subs[0], "画像")
    record(f"[{label}] 画像を送る (photutils なし) → publicly_visible='n'", ok and bool(hdr), msg)

    # 4. 星の位置で解けず、画像へ切り替える → 2 回とも "n"
    nova = FakeNova(["failure", "success"])
    with patched_nova(nova, no_source_detector=True):
        hdr = solve_online(img_path, FAKE_KEY, sources=SOURCES, shape=SHAPE, timeout=30)
    subs = nova.submissions()
    ok = len(subs) == 2
    msgs = [f"受付 {len(subs)} 件"]
    for s, kind in zip(subs, ("星の位置", "画像")):
        o, m = _check_submission(label, s, kind)
        ok, msgs = ok and o, msgs + [m]
    record(f"[{label}] 星の位置 → 画像へ切り替え、2 回とも 'n'", ok and bool(hdr), " | ".join(msgs))

    # 5. photutils があるとき: solve_from_image は画像を送らず、astroquery が数えた星の位置を送る
    nova = FakeNova(["success"])
    with patched_nova(nova, no_source_detector=False):
        hdr = solve_online(img_path, FAKE_KEY, sources=None, shape=SHAPE, timeout=30)
    subs = nova.submissions()
    ok, msg = len(subs) == 1, f"受付 {len(subs)} 件"
    if subs:
        ok, msg = _check_submission(label, subs[0], "星の位置")
    record(f"[{label}] photutils の経路 (画像は送らない) → publicly_visible='n'", ok and bool(hdr), msg)

    # 6. API キーはログインのときだけ
    logins = nova.logins()
    ok = len(logins) == 1 and (logins[0][2] or {}).get("apikey") == FAKE_KEY
    record(f"[{label}] API キーはログインのときだけ送る", ok, f"ログイン {len(logins)} 回")


def case_control():
    """対照: astroquery にそのまま渡すと publicly_visible は送られない (サーバーの既定 'y' = 公開になる)。"""
    from astroquery.astrometry_net import AstrometryNet
    nova = FakeNova(["success"])
    with patched_nova(nova):
        ast = AstrometryNet()
        ast.api_key = FAKE_KEY
        ast.solve_from_source_list([1.0, 2.0, 3.0], [4.0, 5.0, 6.0], 200, 160,
                                   solve_timeout=30, verbose=False)
    subs = nova.submissions()
    sent = subs[0][2] if subs else {}
    record("[対照] astroquery に何も指定しないと publicly_visible は送られない",
           bool(subs) and "publicly_visible" not in sent,
           f"送った項目 {sorted(k for k in sent if k not in ('x', 'y'))}")


def load_built_engine(path):
    """GUI の Engine.load と同じく、1 つの名前空間へ exec する。"""
    ns = {"__name__": "zahyou_engine", "__file__": path}
    with io.open(path, encoding="utf-8") as f:
        exec(compile(f.read(), path, "exec"), ns)
    return ns


def main():
    core_only = "--core-only" in sys.argv
    img = make_star_fits(os.path.join(_TMP, "stars.fits"))
    try:
        import zahyou_core as zc
        run_cases("dev/zahyou_core.py", zc.solve_online, img)
        if not core_only:
            path = os.path.join(REPO, "zahyou_engine.py")
            try:
                ns = load_built_engine(path)
                run_cases("直下の zahyou_engine.py", ns["solve_online"], img)
            except Exception as e:
                record("[直下の zahyou_engine.py] 読み込み", False, f"{type(e).__name__}: {e}")
        case_control()
        record("ネットに出ていない", not NET_ATTEMPTS, f"接続しようとした回数 {len(NET_ATTEMPTS)}")
    finally:
        shutil.rmtree(_TMP, ignore_errors=True)

    width = max(len(r[0]) for r in RESULTS)
    n_ok = 0
    print()
    for name, ok, msg in RESULTS:
        n_ok += bool(ok)
        print(f"  {'PASS' if ok else 'FAIL'}  {name.ljust(width)}  {msg}")
    print(f"\n  {n_ok}/{len(RESULTS)} passed")
    return 0 if n_ok == len(RESULTS) else 1


if __name__ == "__main__":
    sys.exit(main())
