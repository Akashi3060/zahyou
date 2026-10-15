# ==============================================================================
#  solver_index.py — 自前ソルバーの索引ファイル(astrometry.net の index-41xx / index-52xx-NN)
#
#  docs/design-native-solver.md の 2 節。形式は docs/index-format.md(実物を読んで全件で確かめたもの)。
#  numpy と標準ライブラリだけを使う(第 2 段で別の言語へ移しやすくするため)。
#  build_nb.py が zahyou_engine.py へつなぐので、最上位の名前は sv_ / Sv / _sv_ / SV_ で始める。
#  冒頭に docstring を置かない(組み立て時に import だけを外す約束)。
#
#  読み方の要点
#    - FITS は 2880 バイトのブロックと 80 文字のカード。ヘッダーだけを自前で読み、表は TTYPE1 の
#      名前で探す(列の型 '16A' などは当てにならない。中身は生のバイト列)。
#    - 表の数値はリトルエンディアン(ENDIAN = '04:03:02:01')。ほかの値のファイルは読まない。
#    - データはファイル全体の np.memmap(読み取り専用)から型付きのビューで取り、整数のまま探す。
#      実数へ戻すのは要る分だけ(実数 = minval + 整数 / scale)。
#    - kd-tree はファイルに入っている木をそのまま使う(ヒープ順の完全二分木。作り直さない)。
#      葉 k はデータ lr[k−1]+1 … lr[k]、split の下位 ⌈log₂D⌉ ビットが分割次元、残りが分割位置。
#    - quad i のコード = コード木のデータ i、quad の星番号 = 星の木のデータの行番号。
#    - 5200 系のタイル番号は xy 方式の HEALPix(NN = 基本番号·nside² + x·nside + y)。
#      星はタイルの中だけにある。
# ==============================================================================

import math
import os
import re

import numpy as np

from solver_wcs import *


SV_FITS_BLOCK = 2880
SV_INDEX_NAME_RE = re.compile(r"index-(\d{4})(?:-(\d{2}))?\.fits", re.IGNORECASE)
SV_INDEX_ENDIAN = "04:03:02:01"           # u32 の 0x01020304 が 04 03 02 01 と並ぶ = リトルエンディアン
SV_QUAD_MIN_FRAC = 0.1                    # quad の AB の最小 = 0.1 × 画像の短辺(select の下限)
SV_BALL_CHUNK = 2048                      # code_ball で一度にたどる問い合わせの数(メモリの上限のため)

# kd-tree の整数の型(KDT_INT / KDT_DATA)→ (numpy の型, ビット数)
_SV_KD_TYPES = {"u16": ("<u2", 16), "u32": ("<u4", 32)}


# ------------------------------------------------------------------------------
#  FITS のヘッダー(自前で読む。FITS 規格の 2880 バイトのブロックと 80 文字のカード)
# ------------------------------------------------------------------------------

def _sv_fits_value(text):
    """カードの値の部分(11 桁目から後ろ)を Python の値にする。文字列・T/F・整数・実数。"""
    t = text.strip()
    if t.startswith("'"):
        out, i = [], 1
        while i < len(t):
            ch = t[i]
            if ch == "'":
                if t[i + 1:i + 2] == "'":         # '' は ' 1 文字
                    out.append("'")
                    i += 2
                    continue
                break
            out.append(ch)
            i += 1
        return "".join(out).rstrip()
    if "/" in t:                                   # 値の後ろの注釈
        t = t.split("/", 1)[0].strip()
    if t == "T":
        return True
    if t == "F":
        return False
    if not t:
        return None
    try:
        return int(t)
    except ValueError:
        pass
    try:
        return float(t.replace("D", "E"))
    except ValueError:
        return t


def _sv_fits_int(hdr, key, default=None):
    v = hdr.get(key, default)
    if isinstance(v, bool) or not isinstance(v, int):
        raise ValueError(f"ヘッダーの {key} が整数ではありません({v!r})")
    return v


def _sv_read_fits_hdus(path):
    """ファイルの HDU を頭から順に読む。データは読まずに飛ばす。

    戻り値 (hdus, end, size)。hdus は dict(header, offset, size) の並び(offset = データの先頭、
    size = データのバイト数)、end = ヘッダーから計算したファイルの長さ(詰め物込み)、size = 実際の大きさ。
    最後の HDU の後ろに XTENSION で始まらないものがあれば、そこで止める(end < size になる)。
    FITS として読めないときは ValueError(日本語の文)。
    """
    size = os.path.getsize(path)
    hdus = []
    pos = 0
    with open(path, "rb") as f:
        while pos < size:
            f.seek(pos)
            hdr = {}
            nblk = 0
            done = False
            while not done:
                blk = f.read(SV_FITS_BLOCK)
                if nblk == 0:
                    first = blk[:8].decode("ascii", "replace").strip()
                    if not hdus and first != "SIMPLE":
                        raise ValueError(f"FITS ではありません(先頭が {first!r})")
                    if hdus and first != "XTENSION":
                        return hdus, pos, size              # 後ろに余計なもの(呼ぶ側が大きさで気づく)
                if len(blk) < SV_FITS_BLOCK:
                    raise ValueError(f"ヘッダーの途中でファイルが終わっています"
                                     f"({len(hdus) + 1} 個目の HDU、{pos} バイト目から)")
                nblk += 1
                for i in range(0, SV_FITS_BLOCK, 80):
                    card = blk[i:i + 80].decode("ascii", "replace")
                    key = card[:8].strip()
                    if key == "END":
                        done = True
                        break
                    if key and card[8:10] == "= ":
                        hdr.setdefault(key, _sv_fits_value(card[10:]))   # 同じキーは最初のものを使う
            naxis = _sv_fits_int(hdr, "NAXIS", 0)
            n = 0
            if naxis > 0:
                n = 1
                for k in range(1, naxis + 1):
                    n *= _sv_fits_int(hdr, f"NAXIS{k}")
                bitpix = _sv_fits_int(hdr, "BITPIX")
                n = (abs(bitpix) // 8) * _sv_fits_int(hdr, "GCOUNT", 1) * (
                    _sv_fits_int(hdr, "PCOUNT", 0) + n)
            if n < 0:
                raise ValueError("ヘッダーのデータの大きさが負です")
            data = pos + nblk * SV_FITS_BLOCK
            hdus.append(dict(header=hdr, offset=data, size=n))
            pos = data + -(-n // SV_FITS_BLOCK) * SV_FITS_BLOCK
    if not hdus:
        raise ValueError("空のファイルです")
    return hdus, pos, size


# ------------------------------------------------------------------------------
#  索引ファイルのヘッダーの検査
# ------------------------------------------------------------------------------

def _sv_index_layout(path, name=None):
    """索引ファイルのヘッダーを読んで検査し、表の位置と木の形をまとめて返す。

    データは読まない(ヘッダーと os.path.getsize だけ)。問題があれば ValueError(日本語の文)。
    name: 名前の検査に使うファイル名(省略時は path の名前)。
    """
    name = os.path.basename(path) if name is None else os.path.basename(name)
    m = SV_INDEX_NAME_RE.fullmatch(name)
    if not m:
        raise ValueError("ファイル名が index-NNNN.fits / index-NNNN-TT.fits の形ではありません")
    try:
        hdus, end, size = _sv_read_fits_hdus(path)
    except OSError as e:
        raise ValueError(f"読めません({e})")
    if end != size:
        if size < end:
            raise ValueError(f"ファイルが途中で切れています(大きさ {size:,} バイト、ヘッダーから計算した長さ "
                             f"{end:,} バイト。ダウンロードし直してください)")
        raise ValueError(f"大きさがヘッダーから計算した長さと合いません({size:,} バイト、計算 {end:,} バイト)")

    H0 = hdus[0]["header"]
    for key in ("INDEXID", "HEALPIX", "HPNSIDE", "DIMQUADS", "NQUADS", "NSTARS", "SCALE_L", "SCALE_U"):
        if key not in H0:
            raise ValueError(f"索引のヘッダーに {key} がありません")
    for h in hdus:
        e = h["header"].get("ENDIAN")
        if h is hdus[0] and e is None:
            raise ValueError("索引のヘッダーに ENDIAN がありません")
        if e is not None and e != SV_INDEX_ENDIAN:
            raise ValueError(f"ENDIAN = {e!r} の索引は読めません(読めるのは {SV_INDEX_ENDIAN!r} だけ)")

    indexid = _sv_fits_int(H0, "INDEXID")
    healpix = _sv_fits_int(H0, "HEALPIX")
    hpnside = _sv_fits_int(H0, "HPNSIDE")
    dq = _sv_fits_int(H0, "DIMQUADS")
    nq = _sv_fits_int(H0, "NQUADS")
    ns = _sv_fits_int(H0, "NSTARS")
    try:
        sl, su = float(H0["SCALE_L"]), float(H0["SCALE_U"])
    except (TypeError, ValueError):
        raise ValueError("SCALE_L / SCALE_U が数ではありません")
    if indexid != int(m.group(1)):
        raise ValueError(f"名前と中身の系列が違います(名前 {m.group(1)}、INDEXID = {indexid})")
    if m.group(2) is None:
        if healpix != -1:
            raise ValueError(f"全天の名前なのにタイルの索引です(HEALPIX = {healpix})")
    else:
        if healpix != int(m.group(2)):
            raise ValueError(f"名前と中身のタイル番号が違います(名前 {m.group(2)}、HEALPIX = {healpix})")
        if hpnside < 1 or healpix >= 12 * hpnside * hpnside:
            raise ValueError(f"タイル番号が HEALPix の範囲の外です(HEALPIX = {healpix}、HPNSIDE = {hpnside})")
    if dq not in (3, 4):
        raise ValueError(f"DIMQUADS = {dq} は扱えません(3 か 4)")
    if nq < 1 or ns < dq:
        raise ValueError(f"quad か星がありません(NQUADS = {nq}、NSTARS = {ns})")
    if not (0.0 < sl < su < math.pi):
        raise ValueError(f"quad の大きさの範囲がおかしい(SCALE_L = {sl}、SCALE_U = {su})")

    tables = {}
    for h in hdus[1:]:
        t = h["header"].get("TTYPE1")
        if isinstance(t, str):
            tables.setdefault(t, h)

    def table(t):
        if t not in tables:
            raise ValueError(f"表 {t} がありません")
        return tables[t]

    def need_bytes(t, nbytes):
        h = table(t)
        if h["size"] != nbytes:
            raise ValueError(f"表 {t} の大きさが合いません({h['size']:,} バイト、期待 {nbytes:,} バイト)")
        return h["offset"]

    lay = dict(path=path, header=H0, indexid=indexid, healpix=healpix, hpnside=hpnside,
               dimquads=dq, nquads=nq, nstars=ns, scale_l=sl, scale_u=su, size=size)
    lay["quads"] = need_bytes("quads", 4 * dq * nq)
    lay["sweep"] = need_bytes("sweep", ns)

    for tname, ndim, ndata in (("codes", 2 * (dq - 2), nq), ("stars", 3, ns)):
        kh = table(f"kdtree_header_{tname}")["header"]
        if kh.get("KDT_NAME") != tname:
            raise ValueError(f"kdtree_header_{tname} の KDT_NAME が {kh.get('KDT_NAME')!r} です")
        D = _sv_fits_int(kh, "KDT_NDIM")
        N = _sv_fits_int(kh, "KDT_NDAT")
        NN = _sv_fits_int(kh, "KDT_NNOD")
        if D != ndim or N != ndata:
            raise ValueError(f"{tname} の木の次元か点の数が合いません(KDT_NDIM = {D}、KDT_NDAT = {N})")
        if NN < 1 or (NN + 1) & NN:
            raise ValueError(f"{tname} の木の節点の数 {NN} が 2^L − 1 ではありません")
        if kh.get("KDT_EXT") != "double":
            raise ValueError(f"{tname} の木の KDT_EXT = {kh.get('KDT_EXT')!r} は扱えません")
        ti, td = kh.get("KDT_INT"), kh.get("KDT_DATA")
        if ti not in _SV_KD_TYPES or td not in _SV_KD_TYPES:
            raise ValueError(f"{tname} の木の整数の型 {ti!r} / {td!r} は扱えません")
        isz, dsz = _SV_KD_TYPES[ti][1] // 8, _SV_KD_TYPES[td][1] // 8
        lay[tname] = dict(
            name=tname, header=kh, ndim=D, ndata=N, nnodes=NN, int_type=ti, data_type=td,
            lr=need_bytes(f"kdtree_lr_{tname}", 4 * ((NN + 1) // 2)),
            split=need_bytes(f"kdtree_split_{tname}", isz * ((NN - 1) // 2)),
            range=need_bytes(f"kdtree_range_{tname}", 8 * (2 * D + 1)),
            data=need_bytes(f"kdtree_data_{tname}", dsz * N * D))
    return lay


def sv_validate_index_file(path, name=None):
    """索引ファイル 1 本をヘッダーだけで検査する。戻り値 (ok, message)。

    見ること: ファイル名の形、大きさ = ヘッダーから計算した長さ(途中で切れたダウンロード)、ENDIAN、
    INDEXID と名前の系列、HEALPIX と名前のタイル番号、必要な表がそろって大きさが合うこと。
    ok なら message は中身の要約。name を渡すと、名前の検査にはそちらを使う
    (ダウンロード中の .part を、名前を変える前に検査するとき)。
    """
    try:
        lay = _sv_index_layout(path, name)
    except ValueError as e:
        return False, str(e)
    except OSError as e:
        return False, f"読めません({e})"
    lo = math.degrees(lay["scale_l"]) * 60.0
    hi = math.degrees(lay["scale_u"]) * 60.0
    where = "全天" if lay["healpix"] < 0 else f"タイル {lay['healpix']}"
    kind = "三角形" if lay["dimquads"] == 3 else "4 星"
    return True, (f"{where}、{kind}の quad {lay['nquads']:,} 個・星 {lay['nstars']:,} 個、"
                  f"quad の大きさ {lo:.1f}′〜{hi:.1f}′")


# ------------------------------------------------------------------------------
#  HEALPix(astrometry.net の xy 方式の番号)
#
#  番号 = 基本番号(0〜11)× nside² + x × nside + y。(x, y) は基本ピクセルの中の座標で、
#  標準の NESTED の副番号は x を偶数ビット・y を奇数ビットに混ぜたもの。nside は 2 の累乗でなくてよい。
#  式は Górski et al. (2005) の HEALPix の投影(赤道帯 |z| ≤ 2/3 は円筒、極帯は σ = √(3(1−|z|)))。
#  基本ピクセルの中で x は北東、y は北西へ増える((0, 0) が南の角、(nside, nside) が北の角)。
# ------------------------------------------------------------------------------

def _sv_hp_face_center(face):
    """基本ピクセルの中心の、HEALPix の平面(x_s, y_s)での位置。face は配列でよい。"""
    face = np.asarray(face)
    row, col = face // 4, face % 4
    xc = np.where(row == 1, col * (math.pi / 2), math.pi / 4 + col * (math.pi / 2))
    yc = np.where(row == 0, math.pi / 4, np.where(row == 1, 0.0, -math.pi / 4))
    return xc, yc


def sv_healpix_xy_point(hp, nside, dx=0.5, dy=0.5):
    """xy 方式の番号 hp のピクセルの中の点(dx, dy ∈ [0, 1]。0.5, 0.5 が中心)の単位ベクトル。

    hp・dx・dy は配列でよい(ブロードキャスト)。戻り値 (..., 3)。
    """
    hp = np.asarray(hp, dtype=np.int64)
    nside = int(nside)
    face = hp // (nside * nside)
    rest = hp % (nside * nside)
    x = rest // nside + np.asarray(dx, dtype=np.float64)
    y = rest % nside + np.asarray(dy, dtype=np.float64)
    xc, yc = _sv_hp_face_center(face)
    xs = xc + (math.pi / 4) * (x - y) / nside
    ys = yc + (math.pi / 4) * ((x + y) / nside - 1.0)
    ays = np.abs(ys)
    polar = ays > math.pi / 4
    # 赤道帯: z = 8 y_s / (3π)、φ = x_s
    z = ys * (8.0 / (3.0 * math.pi))
    sth = np.sqrt(np.maximum(0.0, 1.0 - z * z))
    phi = np.array(xs, dtype=np.float64, copy=True)
    # 極帯: σ = 2 − 4|y_s|/π、z = ±(1 − σ²/3)、φ = x_c + (x_s − x_c)/σ(x_c はこの基本ピクセルの列の中心)
    sig = 2.0 - 4.0 * ays / math.pi
    with np.errstate(divide="ignore", invalid="ignore"):
        phi_p = np.where(sig > 0, xc + (xs - xc) / np.where(sig > 0, sig, 1.0), xc)
    z_p = np.sign(ys) * (1.0 - sig * sig / 3.0)
    sth_p = sig * np.sqrt(np.maximum(0.0, (2.0 - sig * sig / 3.0) / 3.0))   # √(1 − z²) を桁落ちなしに
    z = np.where(polar, z_p, z)
    sth = np.where(polar, sth_p, sth)
    phi = np.where(polar, phi_p, phi)
    return np.stack([sth * np.cos(phi), sth * np.sin(phi), z], axis=-1)


def sv_healpix_xy_center(hp, nside):
    """xy 方式の番号 hp のピクセルの中心の単位ベクトル。"""
    return sv_healpix_xy_point(hp, nside, 0.5, 0.5)


def sv_healpix_xy_from_xyz(xyz, nside):
    """単位ベクトル(長さは問わない。(..., 3))が入る xy 方式の HEALPix の番号(int64)。"""
    v = np.asarray(xyz, dtype=np.float64)
    nside = int(nside)
    norm = np.linalg.norm(v, axis=-1)
    z = v[..., 2] / norm
    za = np.abs(z)
    sth = np.hypot(v[..., 0], v[..., 1]) / norm
    tt = np.mod(np.arctan2(v[..., 1], v[..., 0]) * (2.0 / math.pi), 4.0)   # φ / (π/2) ∈ [0, 4)
    tt = np.where(tt >= 4.0, 0.0, tt)

    # 赤道帯(|z| ≤ 2/3): 斜めの 2 本の格子線の番号から基本ピクセルと (x, y) を出す
    t1 = nside * (0.5 + tt)
    t2 = nside * (z * 0.75)
    jp = np.floor(t1 - t2).astype(np.int64)          # 右上がりの線の番号
    jm = np.floor(t1 + t2).astype(np.int64)          # 右下がりの線の番号
    ifp = jp // nside
    ifm = jm // nside
    face_e = np.where(ifp == ifm, ifp | 4, np.where(ifp < ifm, ifp, ifm + 8))
    x_e = jm % nside
    y_e = nside - (jp % nside) - 1

    # 極帯(|z| > 2/3)
    ntt = np.minimum(np.floor(tt), 3.0)
    tp = tt - ntt
    tmp = nside * sth * np.sqrt(3.0 / (1.0 + za))   # = nside·√(3(1 − |z|))
    jp2 = np.minimum(np.floor(tp * tmp).astype(np.int64), nside - 1)
    jm2 = np.minimum(np.floor((1.0 - tp) * tmp).astype(np.int64), nside - 1)
    north = z > 0
    face_p = np.where(north, ntt, ntt + 8).astype(np.int64)
    x_p = np.where(north, nside - jm2 - 1, jp2)
    y_p = np.where(north, nside - jp2 - 1, jm2)

    eq = za <= 2.0 / 3.0
    face = np.where(eq, face_e, face_p)
    x = np.where(eq, x_e, x_p)
    y = np.where(eq, y_e, y_p)
    return face * (nside * nside) + x * nside + y


def sv_healpix_xy_radius(hp, nside, samples=64):
    """ピクセル hp の外接半径(中心から境界までの最大の角距離、ラジアン)。

    境界を辺ごとに samples 点で調べた最大に、取りこぼしの分の余裕(0.2 % + 1e-5 rad)を足す
    (安全側。中心からいちばん遠い点は必ず境界にある)。
    """
    t = np.linspace(0.0, 1.0, int(samples) + 1)
    zero, one = np.zeros_like(t), np.ones_like(t)
    dx = np.concatenate([t, one, t, zero])
    dy = np.concatenate([zero, t, one, t])
    edge = sv_healpix_xy_point(int(hp), nside, dx, dy)
    c = sv_healpix_xy_center(int(hp), nside)
    r = float(np.max(sv_angle_between(edge, c[None, :])))
    return r * 1.002 + 1e-5


# ------------------------------------------------------------------------------
#  kd-tree(ファイルの木をそのまま使う)
# ------------------------------------------------------------------------------

class SvKdTree:
    """ファイルに入っている kd-tree 1 本(コード木か星の木)。配列はどれも mmap のビューで、整数のまま。

    data    (ndata, ndim) の整数。実数 = minval + 整数 / scale
    lr      葉 k のデータは lr[k−1]+1 … lr[k](lr[−1] = −1)
    split   内部節点ごとの値。下位 dim_bits ビットが分割次元、残り(下位を 0 にしたもの)が分割位置
            左の部分木は x[dim] ≤ 位置、右は x[dim] ≥ 位置(両側とも等号あり)
    節点はヒープ順(根 = 0、子 = 2i+1 と 2i+2)。内部節点は 0 … ninterior−1、葉はその後ろで全部同じ深さ。
    """

    def __init__(self, buf, info):
        D, N, NN = info["ndim"], info["ndata"], info["nnodes"]
        itype, _ = _SV_KD_TYPES[info["int_type"]]
        dtype, dbits = _SV_KD_TYPES[info["data_type"]]
        self.name = info["name"]
        self.ndim, self.ndata, self.nnodes = D, N, NN
        self.levels = NN.bit_length()                 # NN = 2^levels − 1
        self.ninterior = (NN - 1) // 2
        self.nleaves = (NN + 1) // 2
        self.dim_bits = max(1, (D - 1).bit_length())  # ⌈log₂D⌉(D = 2 で 1、3・4 で 2)
        self.mask = (1 << self.dim_bits) - 1
        self.full = float((1 << dbits) - 1)           # 根の箱は整数の全域 [0, 2^bits − 1]
        o = info["lr"]
        self.lr = buf[o:o + 4 * self.nleaves].view("<u4")
        o = info["split"]
        self.split = buf[o:o + np.dtype(itype).itemsize * self.ninterior].view(itype)
        o = info["data"]
        self.data = buf[o:o + np.dtype(dtype).itemsize * N * D].view(dtype).reshape(N, D)
        o = info["range"]
        rng = np.array(buf[o:o + 8 * (2 * D + 1)].view("<f8"), dtype=np.float64)
        self.minval = rng[:D].copy()
        self.maxval = rng[D:2 * D].copy()
        self.scale = float(rng[2 * D])
        if not (np.all(np.isfinite(rng)) and self.scale > 0):
            raise ValueError(f"{self.name} の木の範囲の表がおかしい(scale = {self.scale})")

    def to_tree(self, x):
        """実数 → 木の単位(丸めない実数)。"""
        return (np.asarray(x, dtype=np.float64) - self.minval) * self.scale

    def to_real(self, ints):
        """木の整数 → 実数。"""
        return self.minval + np.asarray(ints, dtype=np.float64) / self.scale

    def ball_candidates(self, qt, r_t):
        """木の単位の問い合わせ qt (Q, D) のそれぞれについて、中心から r_t 以内に届く葉の点を全部挙げる。

        全問い合わせを幅優先で一斉にたどる(1 段ごとに numpy の配列演算)。箱までの最短距離が r_t を超えた
        枝は捨てる。箱そのものは持たず、次元ごとの「箱までの距離の 2 乗」だけを持つ(Arya & Mount の増分距離):
        分割面に対して問い合わせと同じ側の子は親と同じ距離、反対側の子は分割次元の距離が |q − 位置| に
        変わるだけ(子の箱は親の箱を分割面で切ったものなので)。根の箱は整数の全域 [0, 2^bits − 1]。
        戻り値 (qi, pt): 問い合わせの番号とデータの番号の組(候補。距離の最終判定は呼ぶ側)。
        """
        qt = np.asarray(qt, dtype=np.float64)
        Q, D = qt.shape
        none = (np.zeros(0, np.int64), np.zeros(0, np.int64))
        if Q == 0 or not (r_t >= 0):
            return none
        r2 = float(r_t) * float(r_t)
        # frontier: 問い合わせの番号、節点、箱までの距離の次元ごとの 2 乗 g2 とその和 gs
        gap = np.maximum(np.maximum(-qt, qt - self.full), 0.0)
        g2 = gap * gap
        gs = g2.sum(axis=1)
        keep = np.nonzero(gs <= r2)[0]
        qi = keep.astype(np.int64)
        node = np.zeros(len(qi), dtype=np.int64)
        g2, gs = g2[keep], gs[keep]
        mask = self.mask
        for _ in range(self.levels - 1):            # 内部節点の段(葉は最後の段にだけある)
            if len(qi) == 0:
                return none
            raw = self.split[node].astype(np.int64)
            dim = raw & mask
            diff = qt[qi, dim] - (raw - dim)        # q − 分割の位置
            right = diff > 0.0                      # 同じ側の子が右(q > 位置)
            far2 = diff * diff
            ar = np.arange(len(qi))
            gs_far = gs - g2[ar, dim] + far2        # 反対側の子までの距離の 2 乗
            f = np.nonzero(gs_far <= r2)[0]
            near = 2 * node + 1 + right
            if len(f) == 0:
                node = near
                continue
            g2f = g2[f]
            g2f[np.arange(len(f)), dim[f]] = far2[f]
            qi = np.concatenate([qi, qi[f]])
            node = np.concatenate([near, 2 * node[f] + 2 - right[f]])
            g2 = np.concatenate([g2, g2f])
            gs = np.concatenate([gs, gs_far[f]])
        if len(qi) == 0:
            return none
        # 葉の点を全部並べる
        k = node - self.ninterior
        end = self.lr[k].astype(np.int64) + 1
        start = np.zeros_like(end)
        nz = k > 0
        start[nz] = self.lr[k[nz] - 1].astype(np.int64) + 1
        cnt = end - start
        tot = int(cnt.sum())
        first = np.cumsum(cnt) - cnt
        pt = np.repeat(start - first, cnt) + np.arange(tot, dtype=np.int64)
        return np.repeat(qi, cnt), pt


# ------------------------------------------------------------------------------
#  索引ファイル 1 本
# ------------------------------------------------------------------------------

def _sv_close_raw_mmap(raw):
    """np.memmap の下の mmap を閉じる。配列のビューが全部消えていれば、ここでファイルが放される。"""
    if raw is None:
        return
    try:
        raw.close()
    except BufferError:
        pass                                        # 外にビューが残っている(それが消えれば放される)


def sv_index_codes_from_xyz(P):
    """quad の星(単位ベクトル。(n, dimquads, 3)、並びは A, B, C(, D))から、索引に入っているコードを計算する。

    投影中心 = A と B の中点の方向、x = 北・y = 東の心射投影、A → (0, 0)・B → (1, 1) の相似変換
    (index-format.md 8.1。この式を符号化すると保存されている整数と全 quad で一致する)。
    戻り値 (n, 2·(dimquads − 2)): (cx, cy, dx, dy) か (cx, cy)。
    """
    P = np.asarray(P, dtype=np.float64)
    n = P.shape[0]
    c = sv_normalize(P[:, 0] + P[:, 1])
    h = np.hypot(c[:, 0], c[:, 1])
    hs = np.where(h > 0, h, 1.0)
    east = np.stack([np.where(h > 0, -c[:, 1] / hs, 0.0),
                     np.where(h > 0, c[:, 0] / hs, 1.0),
                     np.zeros(n)], axis=-1)
    north = np.cross(c, east)
    w = np.einsum("nkj,nj->nk", P, c)
    x = np.einsum("nkj,nj->nk", P, north) / w
    y = np.einsum("nkj,nj->nk", P, east) / w
    z = x + 1j * y
    zc = (z[:, 2:] - z[:, :1]) / (z[:, 1:2] - z[:, :1]) * (1 + 1j)
    return np.stack([zc.real, zc.imag], axis=-1).reshape(n, -1)


class SvIndexCorrupt(ValueError):
    """索引ファイルの中身が壊れている(ヘッダーと大きさは正しいが、木の葉の範囲や quad の星番号が範囲の外)。
    file にその SvIndexFile が入る。sv_solve はそのファイルを目録から外して探索を続ける。"""

    def __init__(self, file, message):
        super().__init__(f"{file.name}: {message}")
        self.file = file


def _sv_check_lr(tree):
    """木の葉の範囲 lr が単調非減少で、最後が ndata − 1 か(壊れていると点の番号が範囲の外になる)。"""
    lr = tree.lr.astype(np.int64)
    if len(lr) == 0 or int(lr[-1]) != tree.ndata - 1:
        return f"{tree.name} の木の葉の範囲(lr)の最後が点の数と合いません(中身が壊れています。落とし直してください)"
    if len(lr) > 1 and bool(np.any(lr[1:] < lr[:-1])):
        return f"{tree.name} の木の葉の範囲(lr)が増える順になっていません(中身が壊れています。落とし直してください)"
    return None


class SvIndexFile:
    """索引ファイル 1 本。ヘッダーは作るときに読み、データは使うときに mmap する(close() で放す)。

    属性(design-native-solver.md 2 節):
      path, name(拡張子なしのファイル名。例 'index-5206-02'), indexid, healpix(全天は −1), hpnside,
      dimquads(3 か 4), nquads, nstars, code_dim(= 2·(dimquads − 2)),
      scale_lo_rad, scale_hi_rad(quad の AB の角距離の範囲。SCALE_L / SCALE_U),
      jitter_arcsec(星の木のヘッダーの JITTER。無ければ 1.0), cutnside(一様化の HEALPix の nside),
      tile_center_xyz, tile_radius_rad(タイルの中心と外接半径。全天の索引は None)
    """

    def __init__(self, path, _layout=None):
        lay = _layout if _layout is not None else _sv_index_layout(path)
        self.path = os.fspath(path)
        self.name = os.path.splitext(os.path.basename(self.path))[0]
        self.indexid = lay["indexid"]
        self.healpix = lay["healpix"]
        self.hpnside = lay["hpnside"]
        self.dimquads = lay["dimquads"]
        self.nquads = lay["nquads"]
        self.nstars = lay["nstars"]
        self.code_dim = 2 * (self.dimquads - 2)
        self.scale_lo_rad = lay["scale_l"]
        self.scale_hi_rad = lay["scale_u"]
        sh = lay["stars"]["header"]
        jit = sh.get("JITTER")
        self.jitter_arcsec = (float(jit) if isinstance(jit, (int, float)) and not isinstance(jit, bool)
                              and jit > 0 else 1.0)
        cut = sh.get("CUTNSIDE")
        self.cutnside = cut if isinstance(cut, int) and not isinstance(cut, bool) and cut > 0 else None
        if self.healpix >= 0 and self.hpnside > 0:
            self.tile_center_xyz = sv_healpix_xy_center(self.healpix, self.hpnside)
            self.tile_radius_rad = sv_healpix_xy_radius(self.healpix, self.hpnside)
        else:
            self.tile_center_xyz = None
            self.tile_radius_rad = None
        self._layout = lay
        self._mm = None
        self._buf = None
        self._quads = None
        self._sweep = None
        self._codetree = None
        self._startree = None
        self.broken = None                     # 中身が壊れていると分かったら理由(SvIndexCatalog.mark_broken)

    def __repr__(self):
        return (f"SvIndexFile({self.name!r}, dimquads={self.dimquads}, nquads={self.nquads}, "
                f"nstars={self.nstars}, {'open' if self._buf is not None else 'closed'})")

    @property
    def allsky(self):
        return self.healpix < 0

    @property
    def scale_lo_arcsec(self):
        return self.scale_lo_rad * SV_ARCSEC_PER_RAD

    @property
    def scale_hi_arcsec(self):
        return self.scale_hi_rad * SV_ARCSEC_PER_RAD

    # --- 開く・閉じる ---------------------------------------------------------

    @property
    def is_open(self):
        return self._buf is not None

    def open(self):
        """データを mmap する(読み取り専用。何度呼んでもよい)。"""
        if self._buf is not None:
            return self
        lay = self._layout
        mm = np.memmap(self.path, dtype=np.uint8, mode="r")
        if mm.shape[0] != lay["size"]:
            del mm
            raise ValueError(f"{self.name}: 開いている間にファイルの大きさが変わりました")
        buf = np.asarray(mm)                       # ndarray のビュー(memmap の派生型の手間を省く)
        try:
            o = lay["quads"]
            quads = buf[o:o + 4 * self.dimquads * self.nquads].view("<u4").reshape(
                self.nquads, self.dimquads)
            o = lay["sweep"]
            sweep = buf[o:o + self.nstars]
            codetree = SvKdTree(buf, lay["codes"])
            startree = SvKdTree(buf, lay["stars"])
            # 葉の範囲の検査(表 1 つあたり葉の数 × 4 バイトだけ読む。quads の表全体は読まず、quad_stars で見る)
            for t in (codetree, startree):
                bad = _sv_check_lr(t)
                if bad:
                    raise SvIndexCorrupt(self, bad)
        except Exception:
            raw = getattr(mm, "_mmap", None)
            del mm, buf
            quads = sweep = codetree = startree = None
            _sv_close_raw_mmap(raw)
            raise
        self._mm = mm
        self._buf, self._quads, self._sweep = buf, quads, sweep
        self._codetree, self._startree = codetree, startree
        return self

    def close(self):
        """mmap を放す(Windows でファイルのロックが残らないように)。あとで使えばまた開く。

        このファイルの配列のビューを外で持ち続けていると放せない(codetree.data などを直接持たないこと。
        公開している関数の戻り値はどれも写しなので問題ない)。
        """
        raw = getattr(self._mm, "_mmap", None)
        self._mm = self._buf = self._quads = self._sweep = None
        self._codetree = self._startree = None
        _sv_close_raw_mmap(raw)

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()

    @property
    def codetree(self):
        self.open()
        return self._codetree

    @property
    def startree(self):
        self.open()
        return self._startree

    # --- 中身 -----------------------------------------------------------------

    def quad_stars(self, quad_ids):
        """quad の星番号 (n, dimquads) int64。列は A, B, C(, D)。"""
        self.open()
        out = self._quads[np.asarray(quad_ids, dtype=np.int64)].astype(np.int64)
        if out.size and (int(out.max()) >= self.nstars or int(out.min()) < 0):
            raise SvIndexCorrupt(self, f"quad の星番号が星の数 {self.nstars:,} の外です"
                                       "(中身が壊れています。落とし直してください)")
        return out

    def codes(self, quad_ids):
        """quad のコード(実数に戻したもの)(n, code_dim)。"""
        t = self.codetree
        return t.to_real(t.data[np.asarray(quad_ids, dtype=np.int64)])

    def star_xyz(self, star_ids):
        """星の単位ベクトル (n, 3)(長さ 1 に正規化済み)。"""
        t = self.startree
        return sv_normalize(t.to_real(t.data[np.asarray(star_ids, dtype=np.int64)]))

    def sweep(self, star_ids):
        """星の sweep(uint8。小さいほど早く選ばれた = そのあたりで明るい星)。"""
        self.open()
        return self._sweep[np.asarray(star_ids, dtype=np.int64)].copy()

    def code_ball(self, queries, radius, check=None):
        """コードが ‖q − c‖ ≤ radius の quad を、問い合わせ (Q, code_dim) のそれぞれについて全部探す。

        check: 引数なしの関数(締め切りと中止の確かめ)。問い合わせ SV_BALL_CHUNK 個ごとに呼ぶ(例外はそのまま通す。
        束のコードは数万個になることがあり、1 回の照合が 0.4 秒を超えた)。

        戻り値 (qi, quad): 問い合わせの番号と quad の番号(int64。qi・quad の順に並べてある)。
        木の単位で量子化の分(D × 0.25 目盛り²)だけ広げて候補を取り、最後に実数で判定する。
        候補(葉の点の全部で、当たりの数十倍ある)はまず木の単位の単精度で距離を測り、境界から十分内側
        (単精度の誤差の 2 倍以上)は当たり、十分外側は外れと決め、境界のそばだけを実数で判定する
        (木の単位の距離 = 実数の距離 × scale なので、結果は全部を実数で判定したときと同じ)。
        """
        t = self.codetree
        q = np.asarray(queries, dtype=np.float64)
        if q.ndim == 1:
            q = q[None, :]
        if q.ndim != 2 or q.shape[1] != t.ndim:
            raise ValueError(f"問い合わせの形 {q.shape} が (Q, {t.ndim}) ではありません")
        r = float(radius)
        none = (np.zeros(0, np.int64), np.zeros(0, np.int64))
        if len(q) == 0 or not (r >= 0):
            return none
        r_t = math.sqrt((r * t.scale) ** 2 + 0.25 * t.ndim) * (1.0 + 1e-9) + 1e-6
        # 単精度の距離² の誤差: 目盛り(≤ 2^32)の丸め 2^-24 相対 × 2|d| × D と、2 乗と和の丸め。
        # 余裕は誤差の見積もりの数倍(rs2 · 1e-4 + 半径に比例する分 + 1 目盛り²)
        rs2 = (r * t.scale) ** 2
        band = rs2 * 1e-4 + 2.0 * t.ndim * (t.full * 2.0 ** -23) * (math.sqrt(rs2) + 1.0) + 1.0
        lo32 = np.float32(max(0.0, rs2 - band))
        hi32 = np.float32(rs2 + band)
        out_q, out_c = [], []
        for s in range(0, len(q), SV_BALL_CHUNK):
            if check is not None and s > 0:
                check()
            qq = q[s:s + SV_BALL_CHUNK]
            qt = t.to_tree(qq)
            qi, pt = t.ball_candidates(qt, r_t)
            if len(pt) == 0:
                continue
            qt32 = qt.astype(np.float32)
            # 先に 1 つめの次元の差だけで落とす(|d₀| > 半径なら距離も半径を超える。1 列だけ読むので安い)
            k = np.nonzero(np.abs(t.data[pt, 0] - qt32[qi, 0]) <= np.float32(math.sqrt(rs2) + 1.0))[0]
            qi, pt = qi[k], pt[k]
            d = t.data[pt].astype(np.float32)
            d -= qt32[qi]
            d2 = np.einsum("ij,ij->i", d, d)
            inner = d2 <= lo32
            edge = np.nonzero(~inner & (d2 <= hi32))[0]
            if len(edge):
                c = t.to_real(t.data[pt[edge]])
                inner[edge[((c - qq[qi[edge]]) ** 2).sum(axis=1) <= r * r]] = True
            ok = np.nonzero(inner)[0]
            out_q.append(qi[ok] + s)
            out_c.append(pt[ok])
        if not out_q:
            return none
        qi = np.concatenate(out_q)
        quad = np.concatenate(out_c)
        # (qi, quad) の順に並べる。1 つの整数の鍵にして並べるほうが lexsort より速い
        key = qi * np.int64(t.ndata) + quad
        key.sort()
        return key // t.ndata, key % t.ndata

    def stars_in_cone(self, center_xyz, radius_rad):
        """center から角距離 radius_rad 以内の星の番号(int64、昇順)。

        判定は弦の長さ(正規化した星のベクトルと中心の差 ≤ 2 sin(r/2))。
        """
        c = sv_normalize(np.asarray(center_xyz, dtype=np.float64).reshape(3))
        r = float(radius_rad)
        if not (r >= 0):
            return np.zeros(0, np.int64)
        if r >= math.pi:
            return np.arange(self.nstars, dtype=np.int64)
        if self.tile_center_xyz is not None:
            if float(sv_angle_between(c, self.tile_center_xyz)) > self.tile_radius_rad + r:
                return np.zeros(0, np.int64)      # タイルに届かない
        chord = 2.0 * math.sin(r / 2.0)
        t = self.startree
        # 量子化(3 × 0.25 目盛り²)と、正規化で動く分(|xyz| − 1 ≤ 4e-10 ≒ 0.9 目盛り)を広げる
        r_t = math.sqrt((chord * t.scale) ** 2 + 0.75) + 4.0
        _, pt = t.ball_candidates(t.to_tree(c[None, :]), r_t)
        if len(pt) == 0:
            return np.zeros(0, np.int64)
        s = sv_normalize(t.to_real(t.data[pt]))
        ok = ((s - c) ** 2).sum(axis=1) <= chord * chord
        return np.sort(pt[ok])


# ------------------------------------------------------------------------------
#  置き場所のフォルダー全体
# ------------------------------------------------------------------------------

class SvIndexCatalog:
    """星図データのフォルダー。index-*.fits を走査してヘッダーを検査する(データは使うときに mmap)。

    files     使える索引(INDEXID・HEALPIX の順)
    problems  使えないファイル [(パス, 理由)]
    series    INDEXID → [SvIndexFile](全天の系列は 1 本、5200 系はタイルの数だけ。HEALPIX の順)
    """

    def __init__(self, index_dir):
        self.index_dir = os.fspath(index_dir)
        self.files = []
        self.problems = []
        self.series = {}
        if not os.path.isdir(self.index_dir):
            self.problems.append((self.index_dir, "フォルダーがありません"))
            return
        seen = {}
        for nm in sorted(os.listdir(self.index_dir)):
            if not SV_INDEX_NAME_RE.fullmatch(nm):
                continue
            p = os.path.join(self.index_dir, nm)
            if not os.path.isfile(p):
                continue
            try:
                f = SvIndexFile(p, _layout=_sv_index_layout(p))
            except (ValueError, OSError) as e:
                self.problems.append((p, str(e)))
                continue
            key = (f.indexid, f.healpix)
            if key in seen:
                self.problems.append((p, f"同じ索引が 2 つあります({os.path.basename(seen[key])} と同じ中身)"))
                continue
            seen[key] = p
            self.files.append(f)
        self.files.sort(key=lambda f: (f.indexid, f.healpix))
        for f in self.files:
            self.series.setdefault(f.indexid, []).append(f)
        # 同じ系列の中で段の値がそろっていないファイルは外す(いちばん多い形に合わせる)
        for iid in list(self.series):
            fs = self.series[iid]
            sig = [(f.dimquads, f.scale_lo_rad, f.scale_hi_rad, f.hpnside) for f in fs]
            best = max(set(sig), key=sig.count)
            bad = [f for f, s in zip(fs, sig) if s != best]
            for f in bad:
                self.problems.append((f.path, "同じ系列のほかのファイルと段の値(DIMQUADS・SCALE・HPNSIDE)が違います"))
                self.files.remove(f)
            self.series[iid] = [f for f, s in zip(fs, sig) if s == best]

    def __repr__(self):
        return (f"SvIndexCatalog({self.index_dir!r}, files={len(self.files)}, "
                f"series={sorted(self.series)}, problems={len(self.problems)})")

    def close(self):
        """開いている索引の mmap を全部放す。"""
        for f in self.files:
            f.close()

    def mark_broken(self, f, reason):
        """中身が壊れていると分かったファイルを files から外し、problems に入れる(mmap も放す)。

        series からは外さない(探索の仮説は「系列の中の位置」でファイルを指すので、並びを変えない)。
        代わりに f.broken を立て、select・tiles_near と検証の参照星の検索がそのファイルを飛ばす。
        """
        if f.broken is not None:
            return
        f.broken = str(reason)
        f.close()
        if f in self.files:
            self.files.remove(f)
        self.problems.append((f.path, str(reason)))

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()

    def coverage_arcmin(self):
        """手持ちの段の quad の大きさの範囲 (lo, hi)(分角)。索引が無ければ None。"""
        if not self.files:
            return None
        lo = min(f.scale_lo_rad for f in self.files)
        hi = max(f.scale_hi_rad for f in self.files)
        return math.degrees(lo) * 60.0, math.degrees(hi) * 60.0

    def select(self, scale_lo, scale_hi, width, height, hint_xyz=None, hint_radius_rad=None):
        """画像に使う索引を選んで、試す順に並べる。

        scale_lo, scale_hi: 画素スケールの範囲(″/px)。width, height: 画像の大きさ(px)。
        quad の大きさ(px)が [0.1 × 短辺, 対角] に入りうる段 = [0.1·min(W,H)·scale_lo, hypot(W,H)·scale_hi](″)と
        [SCALE_L, SCALE_U] が重なる段を選ぶ。重なる段が無ければ最も近い段 1 つ。
        順は「quad の大きさが画像の短辺の半分くらい(0.5·min(W,H)·√(scale_lo·scale_hi))になる段」から。
        ヒント(hint_xyz と半径)があれば、タイルの系列はヒントの円(半径 + 画像の対角の半分)と
        重なるタイルだけを、ヒントに近い順に。無ければ番号の順。
        """
        s_lo, s_hi = float(scale_lo), float(scale_hi)
        W, H = float(width), float(height)
        if not (s_lo > 0 and s_hi >= s_lo and W > 0 and H > 0):
            raise ValueError(f"スケール {scale_lo}〜{scale_hi}″/px・大きさ {width}×{height} がおかしい")
        short, diag = min(W, H), math.hypot(W, H)
        qlo = SV_QUAD_MIN_FRAC * short * s_lo / SV_ARCSEC_PER_RAD
        qhi = diag * s_hi / SV_ARCSEC_PER_RAD
        target = 0.5 * short * math.sqrt(s_lo * s_hi) / SV_ARCSEC_PER_RAD
        rng = {iid: (fs[0].scale_lo_rad, fs[0].scale_hi_rad) for iid, fs in self.series.items()}
        ids = [iid for iid, (L, U) in rng.items() if L <= qhi and U >= qlo]
        if not ids and rng:
            def gap(iid):
                L, U = rng[iid]
                return math.log(L / qhi) if L > qhi else math.log(qlo / U)
            ids = [min(rng, key=lambda iid: (gap(iid), iid))]
        ids.sort(key=lambda iid: (abs(math.log(math.sqrt(rng[iid][0] * rng[iid][1]) / target)), iid))

        hint = None
        if hint_xyz is not None:
            hint = sv_normalize(np.asarray(hint_xyz, dtype=np.float64).reshape(3))
            reach = (0.0 if hint_radius_rad is None else float(hint_radius_rad)) + 0.5 * diag * s_hi / SV_ARCSEC_PER_RAD
        out = []
        for iid in ids:
            fs = [f for f in self.series[iid] if f.broken is None]
            if not fs:
                continue
            if hint is not None and fs[0].tile_center_xyz is not None:
                cen = np.array([f.tile_center_xyz for f in fs])
                ang = sv_angle_between(cen, hint[None, :])
                rad = np.array([f.tile_radius_rad for f in fs])
                hit = np.nonzero(ang <= rad + reach)[0]
                fs = [fs[i] for i in hit[np.argsort(ang[hit], kind="stable")]]
            out.extend(fs)
        return out

    def tiles_near(self, indexid, center_xyz, radius_rad):
        """系列 indexid のうち、円(中心 center_xyz、半径 radius_rad)と重なりうるファイル(近い順)。

        全天の系列はそのファイル。検証でタイルの境界をまたぐ視野の星を集めるのに使う。
        """
        fs = [f for f in self.series.get(indexid, []) if f.broken is None]
        if not fs or fs[0].tile_center_xyz is None:
            return list(fs)
        c = sv_normalize(np.asarray(center_xyz, dtype=np.float64).reshape(3))
        cen = np.array([f.tile_center_xyz for f in fs])
        ang = sv_angle_between(cen, c[None, :])
        rad = np.array([f.tile_radius_rad for f in fs])
        hit = np.nonzero(ang <= rad + float(radius_rad))[0]
        return [fs[i] for i in hit[np.argsort(ang[hit], kind="stable")]]
