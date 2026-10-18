"""
zahyou-net の照合用の値を Python 版(基準実装)から作る。

    python gen_net.py            # → net_python.json(この隣)

使うもの(読むだけ。どれも書き換えない):
  gui/zahyou_env.py     星図データの段・名前・URL・画角・おすすめ・ヘッダーの検査・human
  gui/zahyou_update.py  Releases の応答の読み方と確認の文言
  dev/zahyou_core.py    solve_online(nova に送る設定。公開しない指定 publicly_visible='n' も)・
                        resolve_target_name のキャッシュの鍵と書き方
  astroquery / astropy  nova・VizieR・Sesame へ送る要求そのもの(通信は差し替えて、外へは出さない)

Python 版のリポジトリは環境変数 ZAHYOU_PY_REPO で変えられる(既定 ~/Documents/GitHub/zahyou)。
gui/(v6 と試作版の Python の画面)は 7.0.0 の準備でリポジトリから消した。作り直すときは、gui/ が残っている
タグ v7.0.0-beta.2 の木を別の場所に出して ZAHYOU_PY_REPO に指す:

    git worktree add <置き場所> v7.0.0-beta.2
    ZAHYOU_PY_REPO=<置き場所> python gen_net.py

(dev/zahyou_core.py もその木のものを読む。solve_online の送り方は 7.0.0 の準備では変えていない)
"""
from __future__ import annotations

import base64
import io
import json
import math
import os
import random
import struct
import sys
import tempfile
import urllib.error
import urllib.parse

REPO = os.environ.get("ZAHYOU_PY_REPO") or os.path.join(os.path.expanduser("~"), "Documents", "GitHub", "zahyou")
if not os.path.isfile(os.path.join(REPO, "gui", "zahyou_env.py")):
    sys.exit(f"{REPO} に gui/zahyou_env.py がありません。gui/ は 7.0.0 の準備で消したので、"
             "git worktree add <置き場所> v7.0.0-beta.2 で出した木を ZAHYOU_PY_REPO に指してください")
sys.path.insert(0, os.path.join(REPO, "gui"))
sys.path.insert(0, os.path.join(REPO, "dev"))

import zahyou_env as env          # noqa: E402
import zahyou_update as upd       # noqa: E402

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "net_python.json")


def bits(x):
    """実数を 64 ビットの整数で渡す(JSON の実数の読み違いを避ける)。"""
    return struct.unpack("<Q", struct.pack("<d", x))[0]


# ------------------------------------------------------------------ 星図データ ---
def gen_env():
    out = {
        "scale_fov": [[s, lo, hi] for s, (lo, hi) in env.SCALE_FOV.items()],
        "scale_mb": [[s, mb] for s, mb in env.SCALE_MB.items()],
        "lite_base": env.LITE_BASE,
        "g41_base": env.G41_BASE,
        "tiles": env.TILES,
        "recommended": env.recommended_scales(),
        "names": {str(s): env.index_names(s) for s in (4107, 4119, 5200, 5206)},
        "urls": [[s, t, env.index_url(s, t)] for s, t in
                 ((4107, None), (4119, None), (5200, 0), (5206, 7), (5206, 47))],
        "scales_mb": [[list(sc), env.scales_mb(sc)] for sc in
                      ([], [4107], env.recommended_scales(), [5200, 5201, 9999])],
    }
    out["fov_arcmin"] = [[f, w, env.fov_arcmin(f, w)] for f, w in
                         ((800, 7.4), (818, 5.6), (35, 36), (0, 5), (5, 0), (1e-3, 36),
                          (2000, 23.5), (135, 22.3))]
    fovs = [0.5, 1, 1.5, 2, 2.8, 3, 4, 4.97, 5.6, 8, 10, 11, 13.3, 16, 22, 23.5, 30,
            31.8, 42, 60, 85, 100, 120, 170, 240, 340, 480, 680, 1000, 1400, 2000,
            3266, 5000, 0, -3]
    rec = []
    for f in fovs:
        for k in (None, 0, 0.3, 0.56, 1.0, 1.5):
            short = None if k is None else (0 if k == 0 else f * k)
            rec.append([f, short, env.recommend_scales(f, short)])
    out["recommend"] = rec
    out["human"] = [[n, env.human(n)] for n in
                    (0, 1, 512, 999.94, 999.96, 1000, 1023.4, 1023.96, 1024, 1536, 1e6,
                     9802886400, 355536000, 5e12, 2e15, -5, -2048)]
    return out


def card(key, value=None, comment=None):
    """80 文字のカード(値は FITS の固定書式に近い形)。"""
    if value is None:
        s = f"{key:<8}"
    else:
        if isinstance(value, bool):
            v = "T" if value else "F"
            v = f"{v:>20}"
        elif isinstance(value, str) and not value.startswith("!"):
            v = f"'{value:<8}'"
            v = f"{v:<20}"
        elif isinstance(value, str):
            v = f"{value[1:]:>20}"          # 生のまま(壊れた値の試験用)
        else:
            v = f"{value:>20}"
        s = f"{key:<8}= {v}"
        if comment:
            s += f" / {comment}"
    return s[:80].ljust(80)


def header(cards, end=True):
    s = "".join(cards) + (card("END") if end else "")
    s += " " * ((-len(s)) % 2880)
    return s.encode("latin-1")


def fake_index(indexid, healpix, endian="04:03:02:01", naxis1=16, naxis2=10,
               cut=0, end=True, extra=(), naxis_raw=None):
    prim = [card("SIMPLE", True), card("BITPIX", 8), card("NAXIS", 0), card("EXTEND", True),
            card("ENDIAN", endian, "endianness"), card("INDEXID", indexid),
            card("HEALPIX", healpix), card("HPNSIDE", 2)] + list(extra)
    ext = [card("XTENSION", "BINTABLE"), card("BITPIX", 8),
           card("NAXIS", naxis_raw if naxis_raw else 2),
           card("NAXIS1", naxis1), card("NAXIS2", naxis2), card("PCOUNT", 0),
           card("GCOUNT", 1), card("TFIELDS", 1)]
    data = bytes(range(256)) * ((naxis1 * naxis2) // 256 + 1)
    data = data[:naxis1 * naxis2]
    data += b"\0" * ((-len(data)) % 2880)
    blob = header(prim, end) + header(ext) + data
    return blob[:len(blob) - cut] if cut else blob


def gen_layout():
    files = {
        "index-5206-07.fits": fake_index(5206, 7),
        "index-4119.fits": fake_index(4119, -1),
        "index-5206-08.fits": fake_index(5206, 8, cut=1000),
        "index-5206-09.fits": fake_index(5206, 9, endian="01:02:03:04"),
        "index-5205-10.fits": fake_index(5206, 10),
        "index-5206-11.fits": fake_index(5206, 3),
        "foo.fits": fake_index(5206, 7),
        "index-5206-12.fits": fake_index(5206, 12, end=False),
        "index-5206-13.fits": fake_index(5206, 13, naxis_raw="!2.0"),
        "index-5206-14.fits": fake_index(5206, 14, extra=[
            (card("COMMENT") [:10] + "caf\xe9 \xff").ljust(80)]),
        "index-5206-15.fits": fake_index(5206, 15, naxis1=2880, naxis2=3),
        "index-5206-16.fits": fake_index(5206, 16)[:2000],
    }
    same_prefix = ("索引ファイルの名前ではありません", "大きさが合いません", "エンディアンが違います",
                   "INDEXID が名前と違います", "HEALPIX がタイル番号と違います",
                   "ヘッダーの途中でファイルが終わっています")
    out = []
    with tempfile.TemporaryDirectory() as d:
        for name, blob in files.items():
            p = os.path.join(d, name)
            with open(p, "wb") as f:
                f.write(blob)
            ok, why = env._check_layout(p)
            out.append({"name": name, "b64": base64.b64encode(blob).decode(),
                        "ok": ok, "why": why, "msg_same": why.startswith(same_prefix) or ok})
    real = []
    rd = env.DEFAULT_INDEX_DIR
    if os.path.isdir(rd):
        for name in sorted(os.listdir(rd)):
            if env._INDEX_RE.match(name):
                ok, why = env._check_layout(os.path.join(rd, name))
                real.append([name, ok, why])
    return {"files": out, "real_dir": rd, "real": real}


# ------------------------------------------------------------------- 書き方 ---
def gen_format():
    rnd = random.Random(20261015)
    xs = [0.1, 1.0, 1e16, 1e-5, 1e-4, 0.0001234, 1234567890123456.0, 9999999999999998.0,
          -2.5e-7, 1.5e300, 5e-324, 1.7976931348623157e308, 61.91199157844, 41.98626242835,
          0.30000000000000004, 2.0 / 3.0, 123456.789, 1e15, 1e-4 * 0.99999]
    for _ in range(2000):
        e = rnd.uniform(-30, 30)
        xs.append(rnd.choice((-1, 1)) * 10 ** e)
    for _ in range(300):
        xs.append(round(rnd.uniform(-1e6, 1e6), rnd.randint(0, 12)))
    floats = [[bits(x), repr(x)] for x in xs]

    cache = {
        "targets": {
            "UCAC4 660-021020": {"ra": 61.91199157844, "dec": 41.98626242835,
                                 "name": "UCAC4 660-021020"},
            "ベガ": {"ra": 279.23473479, "dec": 38.78368896, "name": "ベガ"},
            "Q\"\\/\t": {"ra": 1e-05, "dec": -0.0, "name": "制御\x01文字\x7f"},
        },
        "rigs": {"ZWO ASI294MC Pro|C8|4144x2822|bin1|f2032.0": 0.414,
                 "?||968x548|bin1|f": 1.2345678901234567},
        "empty": {}, "list": [1, 2.5, [], {}, None, True, False, "x"],
        "unknown": {"keep": [1, {"a": "b"}]},
    }
    text = json.dumps(cache, ensure_ascii=False, indent=1)
    compact = json.dumps({"apikey": "abc", "日本": [1.5, 2, None], "q": "a\"b"})
    keys = ["  ucac4  660-021020 ", "vega", "ベガ", "m 42\t", "ß star", "a\u3000b", "x\x1cy",
            "\u00e9toile", "", "   ", "ǆ"]
    pairs = [["request-json", '{"apikey": "a b+c~*/"}'], ["x y", "日本&="]]
    return {
        "floats": floats,
        "cache_text": text,
        "compact_text": compact,
        "target_key": [[k, " ".join(str(k).split()).upper()] for k in keys],
        "urlencode": {"pairs": pairs, "text": urllib.parse.urlencode([tuple(p) for p in pairs])},
        "quote": [[s, urllib.parse.quote(s)] for s in
                  ("UCAC4 660-021020", "M 42", "ベガ", "a+b/c?d&e", "NGC 1234~", "*'()")],
    }


# ------------------------------------------------------------- Sesame / VizieR ---
def gen_target():
    from astropy.coordinates import SkyCoord, name_resolve
    import astropy.units as u
    from astroquery.vizier import Vizier

    names = ["UCAC4 660-021020", "M 42", "ベガ", "a+b/c?d&e", "NGC 1234~"]
    sesame = []
    orig = name_resolve.download_file
    for n in names:
        urls = []

        def fake(url, **kw):
            urls.append(url)
            raise urllib.error.URLError("差し替え")
        name_resolve.download_file = fake
        try:
            name_resolve.get_icrs_coordinates(n)
        except Exception:
            pass
        finally:
            name_resolve.download_file = orig
        sesame.append([n, urls])

    responses = [
        "# UCAC4 660-021020\t#Q10495713\n#=Vl=VizieR (local):    1    12ms\n"
        "%J 061.911993600 +41.986275900 = 04:07:38.87 +41:59:10.5\n%I.0 {UCAC4} 660-021020\n",
        "#=S=Simbad:    1\n%J 279.23473479 +38.78368896 = 18:36:56.33 +38:47:01.2\n",
        "#=S=Simbad: *** Nothing found ***\n",
        "%J 12.5-3.25",
        "%J12345 x",
        "%J 1.2.3 +4",
        "%J  83.822 -05.391 \n%J 1 2\n",
        "noise %J . +1\n%J 10 20",
    ]
    parsed = []
    for r in responses:
        ra, dec = name_resolve._parse_response(r)
        if ra is None:
            parsed.append([r, None])
            continue
        try:
            c = SkyCoord(ra=ra, dec=dec, unit=(u.degree, u.degree), frame="icrs")
            parsed.append([r, [float(ra), float(dec), c.ra.deg, c.dec.deg]])
        except Exception:
            parsed.append([r, None])

    viz = []
    for ra, dec in ((61.9119936, 41.9862759), (0.0, -89.5), (359.99999999, 0.0),
                    (123.456789012345, -12.3456789), (359.9999999999, 1e-9)):
        v = Vizier(catalog="I/355/gaiadr3", columns=["RA_ICRS", "DE_ICRS", "Gmag"])
        v.ROW_LIMIT = 50
        body = v.query_region(SkyCoord(ra, dec, unit="deg"), radius=5 * u.arcsec,
                              get_query_payload=True)
        viz.append([bits(ra), bits(dec), body])
    return {"sesame_urls": sesame, "sesame_parse": parsed, "vizier": viz,
            "vizier_url": "https://" + Vizier.VIZIER_SERVER + "/viz-bin/votable"}


# ------------------------------------------------------------------- nova ---
class FakeResponse:
    def __init__(self, obj=None, text=None, status=200):
        self._obj, self.text, self.status_code = obj, text, status

    def json(self):
        return self._obj


def wcs_header_text():
    from astropy.io import fits
    h = fits.Header()
    h["WCSAXES"] = 2
    h["CTYPE1"] = "RA---TAN-SIP"
    h["CTYPE2"] = "DEC--TAN-SIP"
    h["CRVAL1"] = 61.9119
    h["CRVAL2"] = 41.9863
    h["CRPIX1"] = 484.5
    h["CRPIX2"] = 274.5
    h["CD1_1"] = -0.000318
    h["CD2_2"] = 0.000318
    h["COMMENT"] = "faked by gen_net.py"
    return h.tostring(), [c.image for c in h.cards]


def capture_nova(script, image_path=None):
    """astroquery の _request を差し替えて、送る要求を順に記録する。script は応答の並び。"""
    from astroquery.astrometry_net import core as anc
    import requests

    log = []
    replies = list(script)

    def fake_request(self, method, url, **kw):
        data = kw.get("data")
        body = (requests.models.RequestEncodingMixin._encode_params(data)
                if isinstance(data, dict) else data)
        files = kw.get("files")
        rec = {"method": method, "url": url, "body": body}
        if isinstance(data, dict):
            rec["request_json"] = data.get("request-json")
        if files:
            rec["files"] = {k: os.path.basename(getattr(v, "name", "")) for k, v in files.items()}
        log.append(rec)
        return replies.pop(0)

    orig_req, orig_sleep = anc.AstrometryNetClass._request, anc.time.sleep
    anc.AstrometryNetClass._request = fake_request
    anc.time.sleep = lambda s: None
    try:
        result = script_runner(image_path)
    finally:
        anc.AstrometryNetClass._request = orig_req
        anc.time.sleep = orig_sleep
    return log, result


def gen_nova():
    import zahyou_core as zc
    global script_runner
    text, images = wcs_header_text()
    sources = [{"x": 100.25, "y": 50.5}, {"x": 0.0, "y": 0.0}, {"x": 967.75, "y": 547.125},
               {"x": 1e-3, "y": 3.3333333333333335}]
    shape = (548, 968)
    cases = []
    for hint, scale in (((61.9119936, 41.9862759), 1.2345), (None, None),
                        ((359.5, -89.0), None), (None, 0.414)):
        script = [
            FakeResponse({"status": "success", "session": "sess-abc"}),
            FakeResponse({"status": "success", "subid": 4242, "hash": "x"}),
            FakeResponse({"processing_started": "x", "jobs": [777]}),
            FakeResponse({"status": "solving"}),
            FakeResponse({"status": "success"}),
            FakeResponse(text=text),
        ]

        def run(_img, hint=hint, scale=scale):
            return zc.solve_online("unused.fits", "KEY-0123456789", sources=sources, shape=shape,
                                   timeout=120, hint_radec=hint, hint_radius_deg=5.0,
                                   scale_hint=scale)
        script_runner = run
        log, hdr = capture_nova(script)
        cases.append({"hint": hint, "scale": scale, "requests": log,
                      "solved": hdr is not None and len(hdr) > 0})

    # 画像を送るとき。solve_online に星のリストを渡さず(= 画像そのものを送る道)、astroquery が
    # 自前で星を検出しないように _no_source_detector を立てる(photutils があると、画像を送らずに
    # 星のリストを送ってしまうため)。こうすると solve_online が astroquery に渡す設定(公開しない指定と
    # 手がかり)を、そのまま astroquery の upload の要求として記録できる
    from astroquery.astrometry_net import core as anc
    with tempfile.TemporaryDirectory() as d:
        img = os.path.join(d, "撮影 01.fits")
        with open(img, "wb") as f:
            f.write(b"SIMPLE  =                    T" + b" " * 2850)
        script = [
            FakeResponse({"status": "success", "session": "sess-img"}),
            FakeResponse({"status": "success", "subid": 9}),
            FakeResponse({"jobs": [None]}),
            FakeResponse({"jobs": [55]}),
            FakeResponse({"status": "failure"}),
        ]

        def run_img(_img):
            return zc.solve_online(img, "KEY-0123456789", sources=None, shape=None,
                                   timeout=60, hint_radec=(10.5, -20.25), hint_radius_deg=5.0,
                                   scale_hint=1.2345)
        script_runner = run_img
        orig_nsd = anc.AstrometryNetClass._no_source_detector
        anc.AstrometryNetClass._no_source_detector = True
        try:
            log, hdr = capture_nova(script)
        finally:
            anc.AstrometryNetClass._no_source_detector = orig_nsd
    out = {"cases": cases, "sources": sources, "shape": list(shape), "wcs_text": text,
           "wcs_cards": images, "image_upload": {"requests": log, "result_empty": not hdr}}
    check_private(out)
    return out


def check_private(nova):
    """送る要求(星のリスト・画像)の request-json に publicly_visible='n'(公開しない)が入っていること。

    2026-10-16 の決定。Python 版(dev/zahyou_core.py の solve_online)が付けていなければ、
    照合用の値を作らずに止める(Rust 版は必ず付けるので、付けていない記録とは合わない)。
    """
    sent = [r for c in nova["cases"] for r in c["requests"]] + nova["image_upload"]["requests"]
    posts = [r for r in sent if r["url"].endswith(("/api/url_upload", "/api/upload"))]
    if len(posts) != len(nova["cases"]) + 1:
        raise SystemExit(f"送る要求の数が合いません ({len(posts)})")
    for r in posts:
        rj = json.loads(r["request_json"])
        if rj.get("publicly_visible") != "n":
            raise SystemExit(
                f"{r['url']} の request-json に publicly_visible='n' がありません "
                f"(Python 版 {REPO} の dev/zahyou_core.py の solve_online が付けていない)")


# ------------------------------------------------------------------- 更新 ---
def gen_update():
    release = {
        "tag_name": "v6.2", "name": "zahyou v6.2", "published_at": "2026-09-30T12:34:56Z",
        "assets": [
            {"name": "README.txt", "size": 10, "browser_download_url": "https://x/README.txt"},
            {"name": "zahyou.exe", "size": 107288075,
             "browser_download_url": "https://github.com/Akashi3060/zahyou/releases/download/v6.2/zahyou.exe",
             "digest": "sha256:5F771F339432588395171ACD2FD8EC8C7C94080419A38B0D703594C5238E3605",
             "updated_at": "2026-09-30T13:00:00Z"},
        ],
    }
    text = json.dumps(release)

    class R(io.BytesIO):
        def __enter__(self):
            return self

        def __exit__(self, *a):
            return False

    orig = upd.urllib.request.urlopen
    upd.urllib.request.urlopen = lambda req, timeout=None: R(text.encode())
    try:
        info = upd.latest()
    finally:
        upd.urllib.request.urlopen = orig

    no_digest = dict(release)
    no_digest["assets"] = [dict(release["assets"][1])]
    del no_digest["assets"][0]["digest"]
    del no_digest["assets"][0]["updated_at"]

    msgs = {}
    with tempfile.TemporaryDirectory() as d:
        exe = os.path.join(d, "zahyou.exe")
        with open(exe, "wb") as f:
            f.write(b"fake exe")
        o1, o2 = upd.running_exe, upd.latest
        upd.running_exe = lambda: exe
        try:
            upd.latest = lambda timeout=20: dict(info)
            msgs["available"] = upd.check(version="v6")["message"]
            same = dict(info, sha256=upd.sha256_file(exe))
            upd.latest = lambda timeout=20: dict(same)
            msgs["up_to_date"] = upd.check(version="v6")["message"]
            msgs["dev"] = upd.check(version="v7-dev")["message"]
        finally:
            upd.running_exe, upd.latest = o1, o2
        msgs["fake_exe_sha256"] = upd.sha256_file(exe)
    return {"release_json": text, "info": info, "no_digest_json": json.dumps(no_digest),
            "messages": msgs}


def main():
    out = {
        "_note": "gen_net.py が Python 版(gui/zahyou_env.py・gui/zahyou_update.py・dev/zahyou_core.py)と "
                 "astroquery / astropy から作った照合用の値。手で書き換えない。",
        "env": gen_env(),
        "layout": gen_layout(),
        "format": gen_format(),
        "target": gen_target(),
        "nova": gen_nova(),
        "update": gen_update(),
    }
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        json.dump(out, f, ensure_ascii=False, indent=1)
        f.write("\n")
    print(f"wrote {OUT} ({os.path.getsize(OUT):,} bytes)")


script_runner = None

if __name__ == "__main__":
    main()
