# ==============================================================================
#  zahyou 解析エンジン
#
#  zahyou_v7.ipynb のセル A から読み込まれます。単体では実行しません。
#  中身を読む必要はありません (直したいときだけ dev/ の元のファイルを触ってください)。
#  オフライン解析は自前のエンジン (dev/solver_*.py) で行い、WSL は使いません。
# ==============================================================================

from __future__ import annotations

import concurrent.futures
import json
import math
import os
import shutil
import tempfile
import time
import warnings

import numpy as np
import matplotlib
import matplotlib.pyplot as plt
from astropy.coordinates import SkyCoord
from astropy.io import fits
from astropy.stats import sigma_clipped_stats
from astropy.visualization import AsinhStretch, ImageNormalize
from astropy.wcs import WCS
from astropy.wcs.utils import proj_plane_pixel_scales
import astropy.units as u

try:
    from scipy import ndimage
except ImportError:
    ndimage = None
    raise SystemExit("scipy が必要です。1 つ目のセルの pip 行を実行してください。")

# SharpCap などの FITS には DATE-OBS はあっても MJD-OBS が無い。astropy はそれを
# 補ったうえで警告を出すが、直す必要のあるものではないので画面には出さない。
# (抑えるのはこの 1 種類だけ。他の警告は今までどおり表示する)
from astropy.wcs import FITSFixedWarning
warnings.simplefilter("ignore", FITSFixedWarning)

# 各モジュールの冒頭にあった import (組み立てのときに集めたもの)
import textwrap
import re


SV_ARCSEC_PER_RAD = 180.0 / math.pi * 3600.0
SV_LSQ_MAX_SHIFTS = 40       # sv_fit_wcs_lsq で接点を動かす回数の上限(90° の視野で星が隅の 1/4 だけでも収まる)
SV_LSQ_TOL_PX = 1e-8         # CRPIX に残る定数項(画素に直したもの)がこれ以下なら接点を動かすのをやめる
SV_LSQ_FAIL_PX = 0.01        # 上限まで動かしてもこれを超えて残れば、当てはめは失敗(ValueError)


def sv_radec_to_xyz(ra_deg, dec_deg):
    """赤経・赤緯(度)→ 単位ベクトル。配列なら (..., 3) を返す。"""
    ra = np.radians(np.asarray(ra_deg, dtype=np.float64))
    dec = np.radians(np.asarray(dec_deg, dtype=np.float64))
    cd = np.cos(dec)
    return np.stack([cd * np.cos(ra), cd * np.sin(ra), np.sin(dec)], axis=-1)


def sv_xyz_to_radec(xyz):
    """単位ベクトル(長さは問わない)→ (赤経 [0, 360) 度, 赤緯 度)。"""
    v = np.asarray(xyz, dtype=np.float64)
    x, y, z = v[..., 0], v[..., 1], v[..., 2]
    ra = np.degrees(np.arctan2(y, x)) % 360.0
    dec = np.degrees(np.arctan2(z, np.hypot(x, y)))
    return ra, dec


def sv_normalize(v):
    """最後の軸で長さ 1 にする。"""
    v = np.asarray(v, dtype=np.float64)
    n = np.linalg.norm(v, axis=-1, keepdims=True)
    return v / n


def sv_angle_between(a, b):
    """2 つのベクトルのなす角(ラジアン)。小さな角でも桁落ちしない atan2 の形。"""
    a = np.asarray(a, dtype=np.float64)
    b = np.asarray(b, dtype=np.float64)
    cr = np.linalg.norm(np.cross(a, b), axis=-1)
    dt = np.sum(a * b, axis=-1)
    return np.arctan2(cr, dt)


def sv_chord_from_angle(rad):
    """角距離(ラジアン)→ 単位球の上の弦の長さ。"""
    return 2.0 * np.sin(np.asarray(rad, dtype=np.float64) / 2.0)


def sv_angle_from_chord(chord):
    """弦の長さ → 角距離(ラジアン)。"""
    return 2.0 * np.arcsin(np.clip(np.asarray(chord, dtype=np.float64) / 2.0, 0.0, 1.0))


def sv_tan_basis(r):
    """接点 r(単位ベクトル 1 つ)での (東, 北) の単位ベクトル。

    東 = normalize(−r_y, r_x, 0)、北 = r × 東。極(r_x = r_y = 0)では東を +y 軸にとる
    (どの向きを選んでも回転が変わるだけで、当てはめ・コードの結果には効かない)。
    """
    r = np.asarray(r, dtype=np.float64)
    h = math.hypot(r[0], r[1])
    if h < 1e-15:
        east = np.array([0.0, 1.0, 0.0])
    else:
        east = np.array([-r[1] / h, r[0] / h, 0.0])
    north = np.cross(r, east)
    return east, north


def sv_tan_project(xyz, r):
    """単位ベクトル(…, 3)を接点 r で心射(TAN)投影する。

    戻り値 (ξ, η, ok)。ξ = 東、η = 北、単位はラジアン。r の反対側の半球(s·r ≤ 0)は投影できないので
    ok = False(ξ, η は nan)。
    """
    s = np.asarray(xyz, dtype=np.float64)
    east, north = sv_tan_basis(r)
    w = s @ np.asarray(r, dtype=np.float64)
    ok = w > 0.0
    with np.errstate(divide="ignore", invalid="ignore"):
        xi = np.where(ok, (s @ east) / w, np.nan)
        eta = np.where(ok, (s @ north) / w, np.nan)
    return xi, eta, ok


def sv_tan_deproject(xi, eta, r):
    """接平面の (ξ, η)(ラジアン)を接点 r から単位ベクトルへ戻す。"""
    xi = np.asarray(xi, dtype=np.float64)
    eta = np.asarray(eta, dtype=np.float64)
    r = np.asarray(r, dtype=np.float64)
    east, north = sv_tan_basis(r)
    v = r + xi[..., None] * east + eta[..., None] * north
    return sv_normalize(v)


# ------------------------------------------------------------ SIP の多項式 ---

def _sv_poly_terms(order, min_deg=0):
    """多項式の項 (p, q) の並び(min_deg ≤ p+q ≤ order。次数の低い順、同じ次数では p の大きい順)。"""
    return [(p, d - p) for d in range(min_deg, order + 1) for p in range(d, -1, -1)]


def _sv_poly_eval(coef, u, v):
    """Σ coef[p, q] u^p v^q。coef は (n+1, n+1)、u と v は同じ形の配列。"""
    n = coef.shape[0] - 1
    up = [np.ones_like(u)]
    vp = [np.ones_like(v)]
    for _ in range(n):
        up.append(up[-1] * u)
        vp.append(vp[-1] * v)
    out = np.zeros(np.broadcast(u, v).shape)
    for p in range(n + 1):
        for q in range(n + 1 - p):
            c = coef[p, q]
            if c != 0.0:
                out = out + c * up[p] * vp[q]
    return out


def _sv_poly_grad(coef, u, v):
    """Σ coef[p, q] u^p v^q の u と v での偏微分。"""
    n = coef.shape[0] - 1
    up = [np.ones_like(u)]
    vp = [np.ones_like(v)]
    for _ in range(n):
        up.append(up[-1] * u)
        vp.append(vp[-1] * v)
    du = np.zeros(np.broadcast(u, v).shape)
    dv = np.zeros_like(du)
    for p in range(n + 1):
        for q in range(n + 1 - p):
            c = coef[p, q]
            if c == 0.0:
                continue
            if p > 0:
                du = du + (c * p) * up[p - 1] * vp[q]
            if q > 0:
                dv = dv + (c * q) * up[p] * vp[q - 1]
    return du, dv


def _sv_poly_lstsq(u, v, targets, order, min_deg=0, weights=None):
    """targets (N, k) の各列を Σ c[p, q] u^p v^q(min_deg ≤ p+q ≤ order)で重みつき最小二乗に当てる。

    戻り値は (k, order+1, order+1) の係数。u, v は最大の絶対値で割ってから当てる(高い次数で桁が
    開きすぎて解けなくなるのを防ぐ)。点が足りないか並びが縮退していれば ValueError。
    """
    u = np.asarray(u, dtype=np.float64).ravel()
    v = np.asarray(v, dtype=np.float64).ravel()
    t = np.asarray(targets, dtype=np.float64).reshape(len(u), -1)
    terms = _sv_poly_terms(order, min_deg)
    s = max(float(np.max(np.abs(u), initial=0.0)), float(np.max(np.abs(v), initial=0.0)), 1.0)
    us, vs = u / s, v / s
    m = np.stack([us ** p * vs ** q for p, q in terms], axis=1)
    if weights is not None:
        sw = np.sqrt(np.asarray(weights, dtype=np.float64).ravel())
        m = m * sw[:, None]
        t = t * sw[:, None]
    sol, _, rank, _ = np.linalg.lstsq(m, t, rcond=None)
    if rank < len(terms):
        raise ValueError(f"多項式(次数 {order})を当てるには点が足りないか、並びが縮退している"
                         f"(項 {len(terms)}、階数 {rank})")
    coef = np.zeros((t.shape[1], order + 1, order + 1))
    for i, (p, q) in enumerate(terms):
        coef[:, p, q] = sol[i] / s ** (p + q)
    return coef


def _sv_sip_matrix(c, min_deg, order=None):
    """SIP の係数を (n+1, n+1) の実数の配列にそろえる(p+q が範囲の外の値は 0)。"""
    c = np.asarray(c, dtype=np.float64)
    if c.ndim != 2 or c.shape[0] != c.shape[1]:
        raise ValueError(f"SIP の係数は正方行列にする(形 {c.shape})")
    n = c.shape[0] - 1 if order is None else int(order)
    out = np.zeros((n + 1, n + 1))
    k = min(n, c.shape[0] - 1)
    out[:k + 1, :k + 1] = c[:k + 1, :k + 1]
    p, q = np.indices(out.shape)
    out[(p + q < min_deg) | (p + q > n)] = 0.0
    return out


def _sv_sip_solve(a, b, uu, vv, u0, v0, maxiter=50):
    """u + A(u, v) = U、v + B(u, v) = V をニュートン法で解く。戻り値 (u, v, 解けたか)。"""
    good = np.isfinite(uu) & np.isfinite(vv) & np.isfinite(u0) & np.isfinite(v0)
    uu = np.where(good, uu, 0.0)
    vv = np.where(good, vv, 0.0)
    u = np.where(good, u0, 0.0)
    v = np.where(good, v0, 0.0)
    with np.errstate(divide="ignore", invalid="ignore", over="ignore"):
        for _ in range(maxiter):
            fa = u + _sv_poly_eval(a, u, v) - uu
            fb = v + _sv_poly_eval(b, u, v) - vv
            au, av = _sv_poly_grad(a, u, v)
            bu, bv = _sv_poly_grad(b, u, v)
            j11, j12, j21, j22 = 1.0 + au, av, bu, 1.0 + bv
            det = j11 * j22 - j12 * j21
            du = (j22 * fa - j12 * fb) / det
            dv = (j11 * fb - j21 * fa) / det
            u = u - du
            v = v - dv
            step = np.abs(du) + np.abs(dv)
            if not np.any(step > 1e-12 * (1.0 + np.abs(u) + np.abs(v))):
                break
        fa = u + _sv_poly_eval(a, u, v) - uu
        fb = v + _sv_poly_eval(b, u, v) - vv
        ok = good & (np.abs(fa) < 1e-7) & (np.abs(fb) < 1e-7)
    return u, v, ok


# ------------------------------------------------------------------ WCS ---

class SvWcs:
    """TAN(+SIP)の WCS。docs/design-native-solver.md の 3.6・3.8・3.9 節。

    crval_xyz (3,)   接点(単位ベクトル)
    crpix (2,)       接点が写る画素(0 始まり)
    cd (2, 2)        画素 → 中間世界座標 (ξ = 東, η = 北)。度/px
    a, b             SIP の順変換の係数 (n+1, n+1)(2 ≤ p+q ≤ n)。SIP が無ければ None
    ap, bp           SIP の逆変換の係数 (m+1, m+1)(0 ≤ p+q ≤ m)。無ければ None
                     (SIP があって逆変換が無いときは、xyz_to_pix がニュートン法で解く)
    """

    def __init__(self, crval_xyz, crpix, cd, a=None, b=None, ap=None, bp=None):
        self.crval_xyz = sv_normalize(np.asarray(crval_xyz, dtype=np.float64).reshape(3))
        self.crpix = np.array(crpix, dtype=np.float64).reshape(2)
        self.cd = np.array(cd, dtype=np.float64).reshape(2, 2)
        self.a = self.b = self.ap = self.bp = None
        if (a is None) != (b is None) or (ap is None) != (bp is None):
            raise ValueError("SIP の係数は A と B、AP と BP を組で渡す")
        if a is not None:
            n = max(np.shape(a)[0], np.shape(b)[0]) - 1
            if n >= 2:                          # 1 次以下の A, B は項が無い = SIP なし
                self.a = _sv_sip_matrix(a, 2, n)
                self.b = _sv_sip_matrix(b, 2, n)
        if ap is not None and self.a is not None:
            m = max(np.shape(ap)[0], np.shape(bp)[0]) - 1
            self.ap = _sv_sip_matrix(ap, 0, m)
            self.bp = _sv_sip_matrix(bp, 0, m)

    @property
    def a_order(self):
        """SIP の順変換の次数(SIP が無ければ 0)。"""
        return 0 if self.a is None else self.a.shape[0] - 1

    @property
    def ap_order(self):
        """SIP の逆変換の次数(無ければ 0)。"""
        return 0 if self.ap is None else self.ap.shape[0] - 1

    @property
    def has_sip(self):
        return self.a is not None

    def __repr__(self):
        ra, dec = self.crval_radec()
        sip = f", SIP {self.a_order}/{self.ap_order}" if self.has_sip else ""
        return (f"SvWcs(crval=({ra:.6f}, {dec:+.6f}), crpix=({self.crpix[0]:.2f}, {self.crpix[1]:.2f}), "
                f"{self.pixel_scale_arcsec():.4f} arcsec/px, parity {self.parity():+d}{sip})")

    def copy(self):
        return SvWcs(self.crval_xyz, self.crpix, self.cd, self.a, self.b, self.ap, self.bp)

    def crval_radec(self):
        """接点の (赤経, 赤緯)(度)。極ちょうどでは赤経 0(sv_tan_basis の東の取り方と合わせる)。"""
        r = self.crval_xyz
        if math.hypot(r[0], r[1]) < 1e-15:
            return 0.0, (90.0 if r[2] > 0 else -90.0)
        ra, dec = sv_xyz_to_radec(r)
        return float(ra), float(dec)

    def pixel_scale_arcsec(self):
        """画素スケール(″/px)= √|det CD| × 3600。"""
        return math.sqrt(abs(float(np.linalg.det(self.cd)))) * 3600.0

    def parity(self):
        """det CD の符号(+1 / −1)。zahyou の表示の「正 (鏡像なし)」は +1。"""
        return 1 if float(np.linalg.det(self.cd)) >= 0.0 else -1

    def pix_to_iwc(self, x, y):
        """画素(0 始まり)→ 中間世界座標 (ξ, η)(度)。SIP があれば掛ける。"""
        u = np.asarray(x, dtype=np.float64) - self.crpix[0]
        v = np.asarray(y, dtype=np.float64) - self.crpix[1]
        if self.a is not None:
            u, v = u + _sv_poly_eval(self.a, u, v), v + _sv_poly_eval(self.b, u, v)
        cd = self.cd
        return cd[0, 0] * u + cd[0, 1] * v, cd[1, 0] * u + cd[1, 1] * v

    def iwc_to_pix(self, xi, eta, exact=False):
        """中間世界座標 (ξ, η)(度)→ 画素 (x, y, ok)。

        SIP の逆変換 (AP, BP) があればそれを使う(近似)。逆変換が無いとき、または exact=True のときは
        順変換をニュートン法で解く(解けなかった点は ok = False)。
        """
        xi = np.asarray(xi, dtype=np.float64)
        eta = np.asarray(eta, dtype=np.float64)
        ci = np.linalg.inv(self.cd)
        uu = ci[0, 0] * xi + ci[0, 1] * eta
        vv = ci[1, 0] * xi + ci[1, 1] * eta
        ok = np.isfinite(uu) & np.isfinite(vv)
        if self.a is None:
            u, v = uu, vv
        else:
            if self.ap is not None:
                u = uu + _sv_poly_eval(self.ap, uu, vv)
                v = vv + _sv_poly_eval(self.bp, uu, vv)
            else:
                u, v = uu, vv
            if exact or self.ap is None:
                u, v, conv = _sv_sip_solve(self.a, self.b, uu, vv, u, v)
                ok = ok & conv
        x = np.where(ok, u + self.crpix[0], np.nan)
        y = np.where(ok, v + self.crpix[1], np.nan)
        return x, y, ok

    def pix_to_xyz(self, x, y):
        """画素(0 始まり)→ 単位ベクトル (…, 3)。"""
        xi, eta = self.pix_to_iwc(x, y)
        return sv_tan_deproject(np.radians(xi), np.radians(eta), self.crval_xyz)

    def xyz_to_pix(self, xyz, exact=False):
        """単位ベクトル (…, 3) → 画素 (x, y, ok)(0 始まり)。

        接点の反対側の半球の点と、SIP をニュートン法で解けなかった点は ok = False(x, y は nan)。
        exact は iwc_to_pix と同じ。
        """
        xi, eta, ok = sv_tan_project(xyz, self.crval_xyz)
        x, y, ok2 = self.iwc_to_pix(np.degrees(xi), np.degrees(eta), exact=exact)
        return x, y, ok & ok2

    def pix_to_radec(self, x, y):
        """画素(0 始まり)→ (赤経, 赤緯)(度)。"""
        return sv_xyz_to_radec(self.pix_to_xyz(x, y))

    def radec_to_pix(self, ra_deg, dec_deg, exact=False):
        """(赤経, 赤緯)(度)→ 画素 (x, y, ok)(0 始まり)。"""
        return self.xyz_to_pix(sv_radec_to_xyz(ra_deg, dec_deg), exact=exact)

    def to_header_cards(self, width, height, comments=()):
        """FITS のヘッダーのカード [(キー, 値, 注記)]。astropy なら fits.Header(cards) でそのまま読める。

        CRPIX は 1 始まりに直して書く。LONPOLE = 180 を必ず書く(接点が北極ちょうどのとき、FITS の既定は 0 で、
        sv_tan_basis の向きと 180° ずれるため。それ以外では既定値と同じ)。comments は COMMENT のカードに
        する(1 枚 70 文字以内に語の切れ目で分け、ASCII の印字できる文字以外は ? に替える)。
        """
        ra, dec = self.crval_radec()
        sip = self.a is not None
        suffix = "-SIP" if sip else ""
        what = "TAN (gnomonic) projection" + (" + SIP distortions" if sip else "")
        cd = self.cd
        cards = [
            ("CTYPE1", "RA---TAN" + suffix, what),
            ("CTYPE2", "DEC--TAN" + suffix, what),
            ("CUNIT1", "deg", "Unit of CRVAL1 and CD1_*"),
            ("CUNIT2", "deg", "Unit of CRVAL2 and CD2_*"),
            ("RADESYS", "ICRS", "Reference frame of the star catalogue"),
            ("LONPOLE", 180.0, "Native longitude of the celestial pole [deg]"),
            ("CRVAL1", ra, "RA  of the reference point [deg]"),
            ("CRVAL2", dec, "Dec of the reference point [deg]"),
            ("CRPIX1", float(self.crpix[0]) + 1.0, "X of the reference point [px, 1-based]"),
            ("CRPIX2", float(self.crpix[1]) + 1.0, "Y of the reference point [px, 1-based]"),
            ("CD1_1", float(cd[0, 0]), "Linear transformation matrix [deg/px]"),
            ("CD1_2", float(cd[0, 1]), "Linear transformation matrix [deg/px]"),
            ("CD2_1", float(cd[1, 0]), "Linear transformation matrix [deg/px]"),
            ("CD2_2", float(cd[1, 1]), "Linear transformation matrix [deg/px]"),
        ]
        if width is not None:
            cards.append(("IMAGEW", int(width), "Image width [px]"))
        if height is not None:
            cards.append(("IMAGEH", int(height), "Image height [px]"))
        if sip:
            polys = [("A", self.a, 2, "forward, axis 1"), ("B", self.b, 2, "forward, axis 2")]
            if self.ap is not None:
                polys += [("AP", self.ap, 0, "inverse, axis 1"), ("BP", self.bp, 0, "inverse, axis 2")]
            for name, coef, lo, desc in polys:
                n = coef.shape[0] - 1
                cards.append((f"{name}_ORDER", n, f"SIP polynomial order, {desc}"))
                for p, q in _sv_poly_terms(n, lo):
                    cards.append((f"{name}_{p}_{q}", float(coef[p, q]), ""))
        for text in comments:
            s = "".join(c if " " <= c <= "~" else "?" for c in str(text))
            for line in textwrap.wrap(s, 70) or [""]:    # 語の切れ目で分ける(末尾の空白は FITS で落ちるため)
                cards.append(("COMMENT", line, ""))
        return cards

    def to_dict(self):
        """JSON に書ける形(trace と移植先の試験のため)。"""
        ra, dec = self.crval_radec()
        d = {"crval_radec": [ra, dec], "crval_xyz": self.crval_xyz.tolist(),
             "crpix": self.crpix.tolist(), "cd": self.cd.tolist()}
        if self.a is not None:
            d["a"] = self.a.tolist()
            d["b"] = self.b.tolist()
        if self.ap is not None:
            d["ap"] = self.ap.tolist()
            d["bp"] = self.bp.tolist()
        return d


# ------------------------------------------------------------------ 当てはめ ---

def sv_fit_similarity(field_xy, star_xyz):
    """対応する画像の点と星から、回転+等方スケールの TAN を当てる(設計書 3.6 節。4 点からの仮の解)。

    接点 r = 星の単位ベクトルの和の方向、CRPIX = 画像の点の重心。星を r で投影した (ξ, η) と画像の点から
    それぞれ重心を引き、C = Σ fᵢ pᵢᵀ = U S Vᵀ、R = V Uᵀ(行列式の符号は見ない = 鏡像も表せる)、
    スケール = √(Σ|pᵢ|² / Σ|fᵢ|²)、CD = スケール × R。点が縮退していれば ValueError。
    """
    f = np.asarray(field_xy, dtype=np.float64).reshape(-1, 2)
    s = np.asarray(star_xyz, dtype=np.float64).reshape(-1, 3)
    if len(f) != len(s) or len(f) < 2:
        raise ValueError(f"画像の点と星の数が合わないか 2 未満(画像 {len(f)}、星 {len(s)})")
    s = sv_normalize(s)
    tot = s.sum(axis=0)
    if np.linalg.norm(tot) < 1e-12:
        raise ValueError("星の和が 0(接点を決められない)")
    r = tot / np.linalg.norm(tot)
    xi, eta, ok = sv_tan_project(s, r)
    if not np.all(ok):
        raise ValueError("接点から 90° 以上離れた星がある")
    p = np.column_stack([xi, eta])
    p = p - p.mean(axis=0)
    fc = f.mean(axis=0)
    g = f - fc
    sf = float(np.sum(g * g))
    sp = float(np.sum(p * p))
    if sf <= 0.0 or sp <= 0.0:
        raise ValueError("画像の点か星がすべて同じ位置にある")
    u, _, vt = np.linalg.svd(g.T @ p)
    rot = vt.T @ u.T
    cd = np.degrees(math.sqrt(sp / sf) * rot)
    return SvWcs(r, fc, cd)


def sv_fit_sip_inverse(wcs, width=None, height=None, order=None, bounds=None):
    """SIP の逆変換 AP, BP を、画像全体の格子で最小二乗に当てる(設計書 3.8 節)。

    格子は x = −0.5 … width−0.5、y = −0.5 … height−0.5(width, height が無ければ
    bounds = (xmin, xmax, ymin, ymax))に 10 × (次数+1) 点四方。各点を順変換した (U, V) から
    元の (u, v) へ戻す多項式を当てる。次数の既定は順変換の次数 + 1。AP, BP を付けた複製を返す
    (SIP が無ければ逆変換も無い複製)。
    """
    out = wcs.copy()
    if out.a is None:
        return out
    if order is None:
        order = out.a_order + 1
    order = int(order)
    if width is not None and height is not None:
        x0, x1, y0, y1 = -0.5, float(width) - 0.5, -0.5, float(height) - 0.5
    elif bounds is not None:
        x0, x1, y0, y1 = (float(t) for t in bounds)
    else:
        raise ValueError("逆変換を当てる範囲(width, height か bounds)が要る")
    n = 10 * (order + 1)
    gx, gy = np.meshgrid(np.linspace(x0, x1, n), np.linspace(y0, y1, n))
    u = gx.ravel() - out.crpix[0]
    v = gy.ravel() - out.crpix[1]
    uu = u + _sv_poly_eval(out.a, u, v)
    vv = v + _sv_poly_eval(out.b, u, v)
    coef = _sv_poly_lstsq(uu, vv, np.column_stack([u - uu, v - vv]), order, 0)
    return SvWcs(out.crval_xyz, out.crpix, out.cd, out.a, out.b, coef[0], coef[1])


def sv_fit_wcs_lsq(field_xy, star_xyz, weights=None, crpix=None, sip_order=0,
                   crval_xyz=None, iterations=SV_LSQ_MAX_SHIFTS, width=None, height=None):
    """対応する画像の点と星から、TAN(+SIP)を重みつき最小二乗で当てる(設計書 3.8 節)。

    CRPIX は固定(既定は画像の点の重みつき重心)。接点(既定は星の重みつきの和の方向)のまわりで星を
    投影し、ξ と η をそれぞれ u, v(画素 − CRPIX)の多項式(0 ≤ p+q ≤ 次数、次数 = max(1, sip_order))で当てる。
    定数項は接点のずれなので、接点をそのぶん動かして当て直す。定数項を CD⁻¹ で画素に直した大きさが
    SV_LSQ_TOL_PX 以下になるまで繰り返す(iterations は動かす回数の上限)。ずれの縮み方は星の配置しだいで、
    星が CRPIX のまわりに散っていれば 1 回ごとにおよそ「視野の広さ(ラジアン)の 2 乗」倍だが、CRPIX から離れて
    固まっていると遅い(60° の視野で星が隅の 1/4 だけだと、3 回動かしても中心で 5 px 残り、10 回で 0.000 px。
    90° の隅の 1/4 では 30 回近く要る)。上限まで動かしても SV_LSQ_FAIL_PX を超えるずれが残れば ValueError
    (その WCS は CRPIX の位置が定まっていない)。1 次の項が CD、2 次以上に CD⁻¹ を掛けたものが SIP の A, B。
    sip_order ≥ 2 なら逆変換 AP, BP(次数 sip_order+1)も、画像全体(width, height。無ければ点の範囲)の
    格子で当てる。重み 0 の点は使わない。点が足りなければ ValueError。
    """
    f = np.asarray(field_xy, dtype=np.float64).reshape(-1, 2)
    s = np.asarray(star_xyz, dtype=np.float64).reshape(-1, 3)
    if len(f) != len(s):
        raise ValueError(f"画像の点と星の数が合わない(画像 {len(f)}、星 {len(s)})")
    s = sv_normalize(s)
    if weights is None:
        w = np.ones(len(f))
    else:
        w = np.asarray(weights, dtype=np.float64).reshape(-1)
        if len(w) != len(f):
            raise ValueError(f"重みの数が合わない(重み {len(w)}、点 {len(f)})")
    w = np.where(np.isfinite(w) & (w > 0.0) & np.all(np.isfinite(f), axis=1)
                 & np.all(np.isfinite(s), axis=1), w, 0.0)
    use = w > 0.0
    if not np.any(use):
        raise ValueError("重みが正の点が無い")
    f = np.where(use[:, None], f, 0.0)
    s = np.where(use[:, None], s, 0.0)
    if crpix is None:
        crpix = (w[:, None] * f).sum(axis=0) / w.sum()
    crpix = np.asarray(crpix, dtype=np.float64).reshape(2)
    if crval_xyz is None:
        crval_xyz = (w[:, None] * s).sum(axis=0)
        if np.linalg.norm(crval_xyz) < 1e-12:
            raise ValueError("星の重みつきの和が 0(接点を決められない)")
    r = sv_normalize(np.asarray(crval_xyz, dtype=np.float64).reshape(3))
    order = max(1, int(sip_order))
    u = f[:, 0] - crpix[0]
    v = f[:, 1] - crpix[1]
    nshift = max(0, int(iterations))
    for k in range(nshift + 1):
        xi, eta, ok = sv_tan_project(s, r)
        ww = np.where(ok, w, 0.0)
        t = np.column_stack([np.where(ok, np.degrees(xi), 0.0), np.where(ok, np.degrees(eta), 0.0)])
        coef = _sv_poly_lstsq(u, v, t, order, 0, ww)
        cd = np.array([[coef[0, 1, 0], coef[0, 0, 1]],
                       [coef[1, 1, 0], coef[1, 0, 1]]])
        if abs(float(np.linalg.det(cd))) < 1e-300 or not np.all(np.isfinite(coef)):
            raise ValueError("CD が特異(点が一直線に並んでいる)")
        # 定数項 = CRPIX が写る (ξ, η)。画素に直して十分小さければ終わり
        left = float(np.hypot(*np.linalg.solve(cd, coef[:, 0, 0])))
        if left <= SV_LSQ_TOL_PX or k == nshift:
            break
        # 接点をそこへ動かして当て直す。最後の当てはめのあとには動かさない(CD は接点の東・北に対する値で、
        # 極のそばでは接点をわずかに動かすだけで東・北の向きが大きく回るため。残る定数項は捨てる)
        r = sv_tan_deproject(np.radians(coef[0, 0, 0]), np.radians(coef[1, 0, 0]), r)
    if left > SV_LSQ_FAIL_PX:
        raise ValueError(f"接点が定まりません({nshift} 回動かしても CRPIX に {left:.3g} px のずれが残る)")
    if order < 2:
        return SvWcs(r, crpix, cd)
    hi = coef.copy()
    p, q = np.indices(hi.shape[1:])
    hi[:, p + q < 2] = 0.0
    ab = np.einsum("ij,jpq->ipq", np.linalg.inv(cd), hi)
    wcs = SvWcs(r, crpix, cd, ab[0], ab[1])
    if width is not None and height is not None:
        return sv_fit_sip_inverse(wcs, width, height, order + 1)
    fu = f[use]
    return sv_fit_sip_inverse(wcs, order=order + 1,
                              bounds=(fu[:, 0].min(), fu[:, 0].max(), fu[:, 1].min(), fu[:, 1].max()))


def sv_wcs_residuals(wcs, field_xy, star_xyz, exact=False):
    """画素の残差 (N, 2) = 画像の点 − 星を wcs で画像へ投影した位置。投影できない星の行は nan。"""
    f = np.asarray(field_xy, dtype=np.float64).reshape(-1, 2)
    x, y, ok = wcs.xyz_to_pix(np.asarray(star_xyz, dtype=np.float64).reshape(-1, 3), exact=exact)
    return np.column_stack([f[:, 0] - x, f[:, 1] - y])

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

SV_CHECK_SEC = 0.05            # 締め切りと中止を確かめる間隔の目安(秒)。重い処理の合間に毎回見る
SV_FIRST_BATCH = 24            # 最初の束の quad の数(このあと束ごとに倍、SV_MAX_BATCH まで)
SV_MAX_BATCH = 768
SV_NN_CELLS = 1_500_000        # 検証の距離の表(仮説 × 画像の星 × 参照星)を一度に作る要素数の上限
SV_QUICK_STARS = 10            # 検証の前の安い足切り: 並びの先頭何個の星に参照星が 1 つも無ければ捨てるか(0 で無し)
SV_ACC_CELLS = 3_000_000       # 検証の積み上げで一度に持つ「仮説 × 参照星」の上限
SV_REF_CHUNK = 150_000         # 足切りを通った仮説を、参照星の数の見積もりでこの数ずつに分けて検証する
SV_VERIFY_STEP = 8192          # 系列の仮説をこの数ずつ検証し、そのたびに解を選ぶ(解があればそこで止める)
SV_TWEAK_STEPS = 10            # 改良の反復の回数(3.8)
SV_TWEAK_GAMMA = 0.7           # 反復ごとに quad からの距離による σ の広がりを弱める割合
SV_TWEAK_MIN_MATCHES = 6       # 改良の最後の当て直しを一致だけで行う一致の数(TAN の未知数 6 の 2 倍の式)
SV_SIP_F_Z = 2.3263478740408408  # SIP を付ける F 検定の有意水準(上側 1 %)の標準正規分布の点
SV_SIP_GRID = 3                # SIP を付けるには、当てる星が画像を 3 × 3 に分けた箱の …
SV_SIP_MIN_BOXES = 7           # … 7 つ以上にあること(星の無い側へ多項式を外挿しないため)
SV_TRACE_MAX_HYP = 20000       # trace に書く仮説の数の上限
SV_AREA_GRID = 10              # 有効面積を数える画像の区切り(10 × 10)


class SvCancelled(Exception):
    """中止の合図(cancel() が True)で探索をやめた。"""


class _SvTimeUp(Exception):
    """締め切りを過ぎた(sv_solve の中だけで使う。外には None で返す)。"""


class SvParams:
    """探索の設定(設計書 3.1 節)。既定値は solve-field と Lang et al. (2010) のもの。SvParams(codetol=0.02) のように
    キーワードで一部だけ変える。

    dataclass にしないのは、GUI 版がエンジンを sys.modules に無い名前空間へ exec して読み込むため
    (エンジンの冒頭の from __future__ import annotations のもとでは、dataclasses が注釈を調べるときに
    sys.modules からモジュールを探して落ちる)。
    """
    codetol = 0.01                        # コードの許容誤差(4 次元の L2)
    distractor = 0.25                     # 画像の星が偽物である割合
    verify_pix = 1.0                      # 星の位置誤差 [px]
    logodds_solve = math.log(1e9)         # これ以上で解(Lang et al. の効用から)
    logodds_tune = math.log(1e6)          # これ以上・solve 未満なら 1 次で改良して検証し直す
    logodds_bail = math.log(1e-100)       # 検証の打ち切り
    tune_min_gain = 1                     # tune-up で上がった仮説を解にするのに要る、改良での一致の増え(0 なら確かめない)
    tune_gain_hyps0 = 1e5                 # 一致の増えを求めるのは、試した仮説の数がこれを超えてから(無限大なら求めない)
    accept_hyps0 = 1e5                    # 試した仮説の数 N がこれを超えたら、解とみなす値を solve + ln(N / これ) に上げる
    parity = "both"                       # "both" / "normal"(配列座標で det CD < 0)/ "mirror"(det CD > 0)
    max_quad_stars = 60                   # quad 作りに使う星(並べ替えた順)の上限
    max_verify_stars = 300                # 検証に使う星の上限
    quad_min_frac = 0.1                   # quad の AB の最小 = 0.1 × 短辺
    uniformize_boxes = 10                 # 探索の前の並べ替え(画面全体に散らす)
    tweak_order = 2                       # 改良の SIP の次数(星が足りなければ下げる)
    FIELDS = ("codetol", "distractor", "verify_pix", "logodds_solve", "logodds_tune", "logodds_bail", "tune_min_gain",
              "tune_gain_hyps0", "accept_hyps0", "parity", "max_quad_stars", "max_verify_stars", "quad_min_frac",
              "uniformize_boxes", "tweak_order")

    def __init__(self, **kw):
        for k, v in kw.items():
            if k not in self.FIELDS:
                raise TypeError(f"SvParams に {k} という設定はありません")
            setattr(self, k, v)

    def as_dict(self):
        return {k: getattr(self, k) for k in self.FIELDS}

    def __repr__(self):
        return "SvParams(" + ", ".join(f"{k}={getattr(self, k)!r}" for k in self.FIELDS) + ")"


class SvSolution:
    """sv_solve の解。wcs の crpix は 0 始まり(ヘッダーに書くときに 1 を足す)。

    wcs          SvWcs(TAN か TAN+SIP)
    logodds      改良した後の検証の対数オッズ
    nmatch       一致した画像の星の数(quad の星は含まない)
    ndistract    偽物とした画像の星の数
    nfield       検証に使った画像の星の数(quad の星は含まない)
    nref         検証に使った参照星の数(同)
    index_name   解いた索引(例 'index-5205-02')
    quad_field   (dimquads,) 画像の星の番号(入力の xy の行。索引の quad の A, B, C, D の順)
    quad_index   (dimquads,) 索引の星の番号
    quads_tried  照合に投げた画像の quad の数
    hypotheses   検証した仮説の数
    seconds      かかった時間
    matches      [(画像の星の番号, 参照星の単位ベクトル, ずれ [px])]
    """

    def __init__(self, wcs, logodds, nmatch, ndistract, nfield, nref, index_name, quad_field, quad_index,
                 quads_tried, hypotheses, seconds, matches=None):
        self.wcs = wcs
        self.logodds = logodds
        self.nmatch = nmatch
        self.ndistract = ndistract
        self.nfield = nfield
        self.nref = nref
        self.index_name = index_name
        self.quad_field = quad_field
        self.quad_index = quad_index
        self.quads_tried = quads_tried
        self.hypotheses = hypotheses
        self.seconds = seconds
        self.matches = [] if matches is None else matches

    def __repr__(self):
        return (f"SvSolution({self.index_name}, logodds={self.logodds:.2f}, nmatch={self.nmatch}, "
                f"quads_tried={self.quads_tried}, hypotheses={self.hypotheses}, seconds={self.seconds:.2f}, "
                f"{self.wcs!r})")


class _SvClock:
    """締め切りと中止を確かめる。check() は中止なら SvCancelled、締め切りを過ぎたら _SvTimeUp を投げる。"""

    def __init__(self, deadline=None, cancel=None):
        self.deadline = deadline
        self.cancel = cancel

    def check(self):
        if self.cancel is not None and self.cancel():
            raise SvCancelled("中止しました")
        if self.deadline is not None and time.monotonic() >= self.deadline:
            raise _SvTimeUp()

    def left(self):
        return math.inf if self.deadline is None else self.deadline - time.monotonic()


def _sv_round(v):
    """四捨五入(Python の round は偶数への丸めなので使わない)。"""
    return int(math.floor(v + 0.5))


# ------------------------------------------------------------------------------
#  星の並べ替え(3.2 節)
# ------------------------------------------------------------------------------

def sv_box_order(xy, width, height, nx, ny):
    """画像を nx × ny の箱に分け、「各箱の 1 番目を元の順に → 各箱の 2 番目 → …」と並べた順(添字)。

    xy の行の順を「元の順」とする。箱は画素の範囲 −0.5 … width−0.5 を等分したもの。
    """
    xy = np.asarray(xy, dtype=np.float64).reshape(-1, 2)
    n = len(xy)
    if n == 0 or (nx <= 1 and ny <= 1):
        return np.arange(n)
    bx = np.clip(np.floor((xy[:, 0] + 0.5) / float(width) * nx), 0, nx - 1).astype(np.int64)
    by = np.clip(np.floor((xy[:, 1] + 0.5) / float(height) * ny), 0, ny - 1).astype(np.int64)
    box = by * nx + bx
    order = np.argsort(box, kind="stable")          # 箱ごとに、元の順
    sb = box[order]
    start = np.concatenate([[0], np.nonzero(np.diff(sb))[0] + 1])
    run = np.diff(np.concatenate([start, [n]]))
    rank = np.empty(n, dtype=np.int64)
    rank[order] = np.arange(n) - np.repeat(start, run)   # 箱の中で何番目か
    return np.lexsort((np.arange(n), rank))


def sv_uniformize(xy, width, height, nboxes=10):
    """探索の前の一様化(Lang et al. / solve-field の既定)。

    NX = max(1, round(√(n·W/H)))、NY = max(1, round(n/NX))(n = nboxes)の箱に分けて sv_box_order。
    """
    n = max(1, int(nboxes))
    nx = max(1, _sv_round(math.sqrt(n * float(width) / float(height))))
    ny = max(1, _sv_round(n / nx))
    return sv_box_order(xy, width, height, nx, ny)


def sv_order_stars(xy, flux, width, height, nboxes=10):
    """明るい順(フラックスの降順。同じ値と flux が無いときは入力の順)→ 一様化。入力の添字を返す。"""
    xy = np.asarray(xy, dtype=np.float64).reshape(-1, 2)
    if flux is None:
        bright = np.arange(len(xy))
    else:
        f = np.asarray(flux, dtype=np.float64).reshape(-1)
        f = np.where(np.isfinite(f), f, -np.inf)
        bright = np.argsort(-f, kind="stable")
    return bright[sv_uniformize(xy[bright], width, height, nboxes)]


# ------------------------------------------------------------------------------
#  quad 作り(newpoint 方式、3.3 節)
# ------------------------------------------------------------------------------

def sv_quad_geometry(P, codetol):
    """quad 作りに使う星の組の表。

    戻り値 (L2, inside): L2[a, b] = |P_a − P_b|²、inside[a, b, k] = 星 k が AB を直径とする円の中
    (コードの許容誤差の分だけ広げる: 中心からの距離 ≤ |AB|·(1/2 + codetol/√2)。コード平面で
    半径 1/√2 + codetol にあたる)。
    """
    P = np.asarray(P, dtype=np.float64).reshape(-1, 2)
    d = P[:, None, :] - P[None, :, :]
    L2 = (d * d).sum(axis=-1)
    mid = (P[:, None, :] + P[None, :, :]) / 2.0
    rfac = 0.5 + codetol / math.sqrt(2.0)
    r2 = L2 * rfac * rfac
    e = P[None, None, :, :] - mid[:, :, None, :]
    inside = (e * e).sum(axis=-1) <= r2[:, :, None]
    return L2, inside


def sv_quads_newest(n, dimquads, L2, inside, ab2_lo, ab2_hi):
    """星 n を含み、ほかの星の番号がすべて n 未満の quad(newpoint 方式)。

    (a) n を AB の一方にする: A = a < n、|AB|² が [ab2_lo, ab2_hi] の中なら、a・n 以外で番号 < n の
        円の中の星から内側の星を選ぶ(4 星なら 2 個、3 星なら 1 個)。
    (b) n を内側の星にする: a < b < n で |AB|² が範囲内かつ n が円の中なら、4 星なら残り 1 個を
        番号 < n の円の中の星から選ぶ。
    戻り値 (m, dimquads) の星番号。列は A, B, 内側の星(番号の小さい順)。A < B。
    """
    out = []
    if n < dimquads - 1:
        return np.zeros((0, dimquads), dtype=np.int64)
    okab = (L2[:n + 1, :n + 1] >= ab2_lo) & (L2[:n + 1, :n + 1] <= ab2_hi)
    # (a) n を B にする
    a = np.nonzero(okab[:n, n])[0]
    if len(a):
        m = inside[a, n, :n].copy()
        m[np.arange(len(a)), a] = False
        if dimquads == 3:
            ai, c = np.nonzero(m)
            out.append(np.column_stack([a[ai], np.full(len(ai), n), c]))
        else:
            tri = np.triu(np.ones((n, n), dtype=bool), 1)
            ai, c, d = np.nonzero(m[:, :, None] & m[:, None, :] & tri[None])
            out.append(np.column_stack([a[ai], np.full(len(ai), n), c, d]))
    # (b) n を内側の星にする
    ia, ib = np.nonzero(np.triu(okab[:n, :n], 1) & inside[:n, :n, n])
    if len(ia):
        if dimquads == 3:
            out.append(np.column_stack([ia, ib, np.full(len(ia), n)]))
        else:
            m = inside[ia, ib, :n].copy()
            ar = np.arange(len(ia))
            m[ar, ia] = False
            m[ar, ib] = False
            pi, c = np.nonzero(m)
            out.append(np.column_stack([ia[pi], ib[pi], c, np.full(len(pi), n)]))
    if not out:
        return np.zeros((0, dimquads), dtype=np.int64)
    return np.concatenate(out).astype(np.int64)


# ------------------------------------------------------------------------------
#  コードと正準化(3.4 節)
# ------------------------------------------------------------------------------

def sv_field_codes(P, quads):
    """画像側のコード(ピクセルのまま。A → (0, 0)、B → (1, 1) の相似変換)。

    s = dx² + dy²、cosθ = (dx + dy)/s、sinθ = (dy − dx)/s、星 P は u, v = P − A として
    x = u·cosθ + v·sinθ、y = −u·sinθ + v·cosθ。戻り値 (m, 2·(dimquads−2)) = (cx, cy, dx, dy) か (cx, cy)。
    """
    P = np.asarray(P, dtype=np.float64)
    quads = np.asarray(quads, dtype=np.int64)
    m, dq = quads.shape
    A = P[quads[:, 0]]
    d = P[quads[:, 1]] - A
    s = (d * d).sum(axis=1)
    cos = (d[:, 0] + d[:, 1]) / s
    sin = (d[:, 1] - d[:, 0]) / s
    I = P[quads[:, 2:]] - A[:, None, :]
    x = I[..., 0] * cos[:, None] + I[..., 1] * sin[:, None]
    y = -I[..., 0] * sin[:, None] + I[..., 1] * cos[:, None]
    return np.stack([x, y], axis=-1).reshape(m, -1)


def sv_code_variants(codes, dimquads, codetol, parity="both"):
    """照合に投げるコードの形を全部作る。

    - 正準化は索引と同じ: 4 星は cx + dx ≤ 1(破れていれば A↔B = 全成分 1 − v)、cx ≤ dx(破れていれば C↔D)。
      3 星は cx ≤ 1/2。境界から 1.5·codetol 以内なら入れ替えた形も問い合わせる
      (= 4 通りの並びのうち cx + dx ≤ 1 + 1.5·codetol かつ cx ≤ dx + 1.5·codetol のものを全部)。
    - 鏡像: (x, y) を入れ替えた形 (cy, cx, dy, dx) にも同じことをする(parity が "both" か "mirror")。
      索引のコードは x = 北・y = 東で計算されているので、FITS の向き(y が上)の普通の空は「そのまま」で当たる。

    戻り値 (q, row, perm, mirror): q (k, D) の問い合わせ、row = 元の codes の行、perm (k, dimquads) =
    索引の quad の星 A, B, C(, D) に対応する画像側の quad の列、mirror = 鏡像の形か。
    """
    codes = np.asarray(codes, dtype=np.float64)
    m = len(codes)
    lim = 1.5 * codetol
    if parity == "normal":
        mirrors = (False,)
    elif parity == "mirror":
        mirrors = (True,)
    else:
        mirrors = (False, True)
    if dimquads == 4:
        swap = [1, 0, 3, 2]
        orders = [(False, False), (False, True), (True, False), (True, True)]
    else:
        swap = [1, 0]
        orders = [(False, False), (True, False)]
    qs, rows, perms, mirs = [], [], [], []
    for mir in mirrors:
        base = codes[:, swap] if mir else codes
        for ab, cd in orders:
            c = 1.0 - base if ab else base
            if cd:
                c = c[:, [2, 3, 0, 1]]
            if dimquads == 4:
                ok = (c[:, 0] + c[:, 2] <= 1.0 + lim) & (c[:, 0] <= c[:, 2] + lim)
            else:
                ok = c[:, 0] <= 0.5 + lim
            idx = np.nonzero(ok)[0]
            if len(idx) == 0:
                continue
            perm = [1, 0] if ab else [0, 1]
            if dimquads == 4:
                perm += [3, 2] if cd else [2, 3]
            else:
                perm += [2]
            qs.append(c[idx])
            rows.append(idx)
            perms.append(np.tile(perm, (len(idx), 1)))
            mirs.append(np.full(len(idx), mir))
    D = 2 * (dimquads - 2)
    if not qs:
        return (np.zeros((0, D)), np.zeros(0, np.int64), np.zeros((0, dimquads), np.int64),
                np.zeros(0, bool))
    return (np.concatenate(qs), np.concatenate(rows), np.concatenate(perms).astype(np.int64),
            np.concatenate(mirs))


def sv_quad_ab_range(scale_lo_rad, scale_hi_rad, s_lo, s_hi, width, height, codetol, quad_min_frac):
    """索引の段で使える quad の AB の長さの範囲 [minAB, maxAB](px)(3.5 節)。

    fudge = SCALE_U × codetol / (√2 × s_hi)、minAB = max(SCALE_L / s_hi − fudge, quad_min_frac × 短辺)、
    maxAB = SCALE_U / s_lo + fudge(SCALE は ″、s は ″/px)。
    """
    L = scale_lo_rad * SV_ARCSEC_PER_RAD
    U = scale_hi_rad * SV_ARCSEC_PER_RAD
    fudge = U * codetol / (math.sqrt(2.0) * s_hi)
    lo = max(L / s_hi - fudge, quad_min_frac * min(width, height))
    hi = U / s_lo + fudge
    return lo, hi


# ------------------------------------------------------------------------------
#  4 点からの当てはめ(3.6 節。sv_fit_similarity を仮説の束でまとめて行う形)
# ------------------------------------------------------------------------------

def _sv_tan_basis_many(r):
    """接点 r (H, 3) の (東, 北)。sv_tan_basis と同じ決め方(極では東 = +y 軸)。"""
    h = np.hypot(r[:, 0], r[:, 1])
    pole = h < 1e-15
    hs = np.where(pole, 1.0, h)
    east = np.stack([np.where(pole, 0.0, -r[:, 1] / hs), np.where(pole, 1.0, r[:, 0] / hs),
                     np.zeros(len(r))], axis=-1)
    north = np.cross(r, east)
    return east, north


def sv_fit_similarity_many(field_xy, star_xyz):
    """sv_fit_similarity(回転+等方スケールの TAN)を H 組まとめて当てる。

    field_xy (H, k, 2)、star_xyz (H, k, 3)。戻り値 dict: crval (H, 3)、crpix (H, 2)、cd (H, 2, 2)(度/px)、
    scale (H,)(″/px)、ok (H,)(縮退・90° を越える星が無いもの)。
    """
    F = np.asarray(field_xy, dtype=np.float64)
    S = sv_normalize(np.asarray(star_xyz, dtype=np.float64))
    H = len(F)
    tot = S.sum(axis=1)
    tn = np.linalg.norm(tot, axis=1)
    ok = tn > 1e-12
    r = tot / np.where(ok, tn, 1.0)[:, None]
    east, north = _sv_tan_basis_many(r)
    w = np.einsum("hkj,hj->hk", S, r)
    ok &= np.all(w > 0.0, axis=1)
    ws = np.where(w > 0.0, w, 1.0)
    p = np.stack([np.einsum("hkj,hj->hk", S, east) / ws, np.einsum("hkj,hj->hk", S, north) / ws], axis=-1)
    p = p - p.mean(axis=1, keepdims=True)
    fc = F.mean(axis=1)
    g = F - fc[:, None, :]
    sf = (g * g).sum(axis=(1, 2))
    sp = (p * p).sum(axis=(1, 2))
    ok &= (sf > 0.0) & (sp > 0.0)
    C = np.einsum("hki,hkj->hij", g, p)
    u, _, vt = np.linalg.svd(C)
    rot = np.einsum("hji,hkj->hik", vt, u)          # R = V Uᵀ(行列式の符号は見ない = 鏡像も表せる)
    sc = np.sqrt(np.where(ok, sp, 1.0) / np.where(ok, sf, 1.0))
    cd = np.degrees(sc[:, None, None] * rot)
    return dict(crval=r, crpix=fc, cd=cd, scale=sc * SV_ARCSEC_PER_RAD, ok=ok, east=east, north=north)


def _sv_project_many(xyz, crval, east, north, cdinv, crpix):
    """単位ベクトル (n, 3) を、行ごとの TAN(接点 crval・CD⁻¹・CRPIX、それぞれ (n, …))で画素へ。"""
    w = np.einsum("nj,nj->n", xyz, crval)
    ok = w > 0.0
    ws = np.where(ok, w, 1.0)
    xi = np.degrees(np.einsum("nj,nj->n", xyz, east) / ws)
    eta = np.degrees(np.einsum("nj,nj->n", xyz, north) / ws)
    x = cdinv[:, 0, 0] * xi + cdinv[:, 0, 1] * eta + crpix[:, 0]
    y = cdinv[:, 1, 0] * xi + cdinv[:, 1, 1] * eta + crpix[:, 1]
    return x, y, ok


def _sv_pix_to_xyz_many(x, y, crval, east, north, cd, crpix):
    """画素(行ごとの TAN)→ 単位ベクトル (n, 3)。"""
    u = x - crpix[:, 0]
    v = y - crpix[:, 1]
    xi = np.radians(cd[:, 0, 0] * u + cd[:, 0, 1] * v)
    eta = np.radians(cd[:, 1, 0] * u + cd[:, 1, 1] * v)
    return sv_normalize(crval + xi[:, None] * east + eta[:, None] * north)


# ------------------------------------------------------------------------------
#  検証(対数オッズ、3.7 節)
# ------------------------------------------------------------------------------

def sv_healpix_side_rad(nside):
    """HEALPix の 1 辺の目安(ラジアン)= √(4π / (12·nside²))。"""
    return math.sqrt(4.0 * math.pi / 12.0) / float(nside)


def sv_logodds_weight(lo):
    """1 つの一致の対数オッズ → 改良の重み(lo > 40 は 1、lo < −700 は 0、ほかは e^lo / (1 + e^lo))。"""
    lo = np.asarray(lo, dtype=np.float64)
    with np.errstate(over="ignore"):
        w = 1.0 / (1.0 + np.exp(-np.clip(lo, -700.0, 40.0)))
    return np.where(lo > 40.0, 1.0, np.where(lo < -700.0, 0.0, w))


def sv_accept_logodds(p, nhyp):
    """この探索で nhyp 個の仮説を試したところで、解とみなす対数オッズ(3.7 節の 6)。

    nhyp が accept_hyps0 以下なら logodds_solve、超えたら logodds_solve + ln(nhyp / accept_hyps0)。
    考え方: 偶然の仮説の尤度比が K を超える見込みは 1 つあたり 1/K 以下なので、当て直す前の尤度比に k 個目で 1e9·k/1e5 を
    求めれば、探索全体の見込みは 1e−4·(1 + ln(N/1e5)) 以下になる。ただし実装はこの値を最後の改良(一致した星への当て直し)
    の後の対数オッズと比べ、tune-up を通らない仮説の入口は固定の logodds_solve のままなので、厳密な上限ではない(その経路で
    保証されるのは固定の 1e9 と同じ N/1e9 まで)。当て直しの上げ幅が ln(N/1e5) より小さい偶然の解を止める目安で、
    tune-up 経由の偶然の解は sv_tuneup_accepted が受け持つ。
    代償: 対数オッズ L の本物の解でも、初めて出るのが N > accept_hyps0·e^(L − logodds_solve) 個目なら捨てる
    (L = 24 で 2.7e6、26.9 で 4.8e7)。N は探索 1 回で全部の系列にまたがって数えるので、星図データが多いほど・速いほど
    同じ締め切りの中で基準が厳しくなる(実測の正しい解に L < 50 かつ N > 1e5 は無い。docs/rust-port.md 14.6)。
    """
    n = float(nhyp)
    h0 = float(p.accept_hyps0)
    if h0 > 0.0 and n > h0:
        return float(p.logodds_solve) + math.log(n / h0)
    return float(p.logodds_solve)


def sv_tuneup_accepted(p, nhyp, nmatch_before, nmatch_after):
    """この探索で nhyp 個の仮説を試したところで、tune-up(1 次の改良で検証し直す)で solve に届いた仮説を解にしてよいか。

    改良は検証に使ったのと同じ星に WCS を当て直すので、それだけでオッズが上がる(偶然の仮説で 4〜8.5)。
    本物の仮説なら改良で新しい星が並ぶので、nhyp が tune_gain_hyps0 を超えたら、改良後の一致数が 4 点の検証の一致数より
    tune_min_gain 以上多いことを求める。それまでは求めない: 星の少ない画像の本物の解は、星が全部一致済みで増えようがない
    (星 8 個の 3I_00010 は仮説 4,553 個目に tune-up で 21.9・一致 4 → 4 で解ける)。誤った解が出るのは何百万個も試したあと。
    代償: tune_gain_hyps0 を超えてから tune-up で上がり、一致が増えない本物の候補も捨てる(実測では s-30d-r3 の 1 候補。
    別の仮説で先に解けていて結果は変わらない)。
    """
    if not (int(p.tune_min_gain) > 0 and float(nhyp) > float(p.tune_gain_hyps0)):
        return True
    return int(nmatch_after) >= int(nmatch_before) + int(p.tune_min_gain)


def _sv_chunks(sizes, per, limit):
    """sizes の小さい順に並べた仮説を、「束の数 × per × 束の中の最大」が limit を超えないように区切る。

    戻り値 [(並べた順の添字の配列)]。
    """
    order = np.argsort(sizes, kind="stable")
    sz = np.maximum(np.asarray(sizes, dtype=np.int64)[order], 1)
    out = []
    s = 0
    H = len(order)
    while s < H:
        g = (np.arange(1, H - s + 1) * int(max(per, 1))) * sz[s:]
        e = s + max(1, int(np.searchsorted(g, limit, side="right")))
        out.append(order[s:e])
        s = e
    return out


def _sv_nearest(test_xy, test_idx, sig2, ref_xy, ref_start, ref_count, clock=None):
    """画像の星ごとに 5σ 以内で最も近い参照星(仮説の束ごとに、詰め物つきの距離の表で)。

    戻り値 (nn, nd2): 参照星の番号(仮説の中での番号。無ければ −1)と距離² (H, NT)。
    """
    H, NT = test_idx.shape
    nn = np.full((H, NT), -1, dtype=np.int64)
    nd2 = np.full((H, NT), np.inf)
    if H == 0 or NT == 0 or len(ref_xy) == 0:
        return nn, nd2
    # 最近傍を選ぶ表は単精度で作る(差は画素の桁なので十分。詰め物は遠くに置いた有限の値で、inf・nan の
    # 判定を省く)。選んだ参照星との距離は倍精度で計算し直す(対数オッズの値を倍精度の計算とそろえるため)
    txy_all = test_xy.astype(np.float32)
    rxy_all = ref_xy.astype(np.float32)
    for hs in _sv_chunks(ref_count, NT, SV_NN_CELLS):
        nrmax = max(1, int(ref_count[hs].max()))
        k = np.arange(nrmax)
        rvalid = k[None, :] < ref_count[hs, None]
        ridx = np.where(rvalid, ref_start[hs, None] + k[None, :], 0)
        rx = np.where(rvalid, rxy_all[ridx, 0], np.float32(-1e7))
        ry = np.where(rvalid, rxy_all[ridx, 1], np.float32(-1e7))
        ti = test_idx[hs]
        tvalid = ti >= 0
        tis = np.where(tvalid, ti, 0)
        txy = txy_all[tis]
        dx = txy[:, :, 0, None] - rx[:, None, :]
        dd = dx * dx
        dy = np.subtract(txy[:, :, 1, None], ry[:, None, :], out=dx)
        dd += dy * dy
        j = np.argmin(dd, axis=2)
        jr = np.minimum(j, ref_count[hs, None] - 1) + ref_start[hs, None]
        e = test_xy[tis] - ref_xy[np.maximum(jr, 0)]
        m = (e * e).sum(axis=-1)
        good = tvalid & (ref_count[hs, None] > 0) & (m <= 25.0 * sig2[hs])
        nn[hs] = np.where(good, j, -1)
        nd2[hs] = np.where(good, m, np.inf)
        if clock is not None:
            clock.check()
    return nn, nd2


def sv_verify_core(test_xy, test_idx, sig2, ref_xy, ref_start, ref_count, logbg, params,
                   record=False, clock=None):
    """対数オッズの積み上げ(3.7 節の 5)を、仮説 H 個まとめて行う。

    test_xy  (NV, 2) 画像の星(検証に使う全部)
    test_idx (H, NT) 仮説ごとの画像の星の並び(test_xy の行。−1 は詰め物)。quad の星と RoR の外は外しておく
    sig2     (H, NT) 星ごとの位置の分散 σᵢ²(px²)
    ref_xy   (R, 2)  参照星の画素の位置。仮説ごとに ref_start[h] から ref_count[h] 個(sweep の順)
    logbg    (H,)    −ln A(A = 有効面積)
    戻り値 dict: best (H,)(並びの先頭から何個かまでを使ったときの最大)、nmatch・ndistract・nconflict・
    nused(best のときの数)。record なら、仮説ごとの一致
    [(列, 画像の星, 参照星の番号, 1 つの一致の対数オッズ)](best までの分、参照星の番号は ref_xy の行)も返す。
    """
    H, NT = test_idx.shape
    d = float(params.distractor)
    bail = float(params.logodds_bail)
    NR = ref_count.astype(np.float64)
    out_n = dict(best=np.full(H, -np.inf), nmatch=np.zeros(H, np.int64), ndistract=np.zeros(H, np.int64),
                 nconflict=np.zeros(H, np.int64), nused=np.zeros(H, np.int64), best_col=np.full(H, -1))
    rec = [[] for _ in range(H)] if record else None
    if H > 0 and NT > 0:
        live = np.arange(H)
        nn, nd2 = _sv_nearest(test_xy, test_idx, sig2, ref_xy, ref_start, ref_count, clock)
        # 積み上げ(参照星の持ち主の表の大きさを抑えるため、仮説を束に分ける)
        for hs in _sv_chunks(ref_count[live], 1, SV_ACC_CELLS):
            _sv_accumulate(live[hs], test_idx, sig2, nn, nd2, NR, logbg, d, bail, out_n["best"], out_n,
                           rec, clock)
    if record:
        out_n["matches"] = [[(c, t, int(ref_start[h]) + r, lom) for c, t, r, lom in rec[h]]
                            for h in range(H)]
    return out_n


def _sv_accumulate(hs, test_idx, sig2, nn, nd2, NR_all, logbg_all, d, bail, best_all, out_n, rec, clock):
    """sv_verify_core の積み上げの本体(仮説の束 hs について、画像の星を 1 つずつ見る)。

    logd(μ) = ln(d + (1 − d)·μ/NR) + logbg(偽物として説明したときの尤度)、
    logfg = ln((1 − d)/(2π σᵢ² NR)) − dist²/(2σᵢ²)(5σ 以内の最近傍の参照星。無ければ −∞)。
    logfg < logd(μ) なら偽物、最近傍がすでに別の星 j の組なら、付け替えたほうが尤度が上がるときだけ
    付け替える(1 対 1)。付け替えの尤度 = logfg + (logd(μⱼ) − logfgⱼ) + Σ(j と i の間で logd を使った星 k の
    logd(μₖ − 1) − logd(μₖ))。logodds += (その星の項) − logbg。bail を割ったら打ち切り。
    """
    H = len(hs)
    NT = test_idx.shape[1]
    NR = NR_all[hs]
    logbg = logbg_all[hs]
    nrmax = max(1, int(NR.max()))
    owner = np.full((H, nrmax), -1, dtype=np.int64)
    fg_at = np.zeros((H, NT))
    mu_at = np.zeros((H, NT))
    S = np.zeros((H, NT + 1))              # logd を使った星の「μ が 1 減ったときの差」の累積
    lo = np.zeros(H)
    best = np.full(H, -np.inf)
    mu = np.zeros(H)
    nmatch = np.zeros(H, np.int64)
    ndis = np.zeros(H, np.int64)
    ncon = np.zeros(H, np.int64)
    nused = np.zeros(H, np.int64)
    b_match = np.zeros(H, np.int64)
    b_dis = np.zeros(H, np.int64)
    b_con = np.zeros(H, np.int64)
    b_used = np.zeros(H, np.int64)
    b_col = np.full(H, -1)
    alive = np.ones(H, dtype=bool)
    ar = np.arange(H)
    lnfg0 = np.log((1.0 - d) / (2.0 * math.pi * NR))   # σ² を除いた前景の係数
    events = [] if rec is not None else None
    t_last = time.monotonic()
    for i in range(NT):
        ti = test_idx[hs, i]
        act = alive & (ti >= 0)
        if not act.any():
            if not alive.any():
                break
            S[:, i + 1] = S[:, i]
            continue
        s2 = sig2[hs, i]
        k = nn[hs, i]
        has = act & (k >= 0)
        with np.errstate(divide="ignore", invalid="ignore"):
            logfg = np.where(has, lnfg0 - np.log(np.where(has, s2, 1.0))
                             - nd2[hs, i] / (2.0 * np.where(has, s2, 1.0)), -np.inf)
            ld = np.log(d + (1.0 - d) * mu / NR) + logbg
            ld_m1 = np.log(d + (1.0 - d) * np.maximum(mu - 1.0, 0.0) / NR) + logbg
        isdis = act & (logfg < ld)
        kk = np.where(has, k, 0)
        j = np.where(has, owner[ar, kk], -1)
        conflict = act & ~isdis & (j >= 0)
        newm = act & ~isdis & (j < 0)
        jj = np.where(conflict, j, 0)
        with np.errstate(invalid="ignore"):
            ldj = np.log(d + (1.0 - d) * mu_at[ar, jj] / NR) + logbg
            switch = logfg + (ldj - fg_at[ar, jj]) + (S[ar, i] - S[ar, jj + 1])
        do_sw = conflict & (switch > ld)
        keepc = conflict & ~do_sw
        term = np.where(isdis | keepc, ld, np.where(do_sw, switch, logfg))
        # 持ち主の表と μ を更新する
        claim = newm | do_sw
        owner[ar[claim], kk[claim]] = i
        fg_at[claim, i] = logfg[claim]
        mu_at[claim, i] = mu[claim]
        if events is not None:
            for h in np.nonzero(claim)[0]:
                events.append((int(h), i, int(test_idx[hs[h], i]), int(kk[h]),
                               float(logfg[h] - ld[h]), int(j[h]) if do_sw[h] else -1))
        mu = mu + newm
        nmatch += newm
        ndis += isdis
        ncon += conflict
        nused += act
        S[:, i + 1] = S[:, i] + np.where(isdis | keepc, ld_m1 - ld, 0.0)
        lo = np.where(act, lo + term - logbg, lo)
        up = act & (lo > best)
        best = np.where(up, lo, best)
        b_match = np.where(up, nmatch, b_match)
        b_dis = np.where(up, ndis, b_dis)
        b_con = np.where(up, ncon, b_con)
        b_used = np.where(up, nused, b_used)
        b_col = np.where(up, i, b_col)
        alive &= ~(act & (lo < bail))
        if clock is not None and time.monotonic() - t_last > SV_CHECK_SEC:
            clock.check()
            t_last = time.monotonic()
    best_all[hs] = best
    out_n["nmatch"][hs] = b_match
    out_n["ndistract"][hs] = b_dis
    out_n["nconflict"][hs] = b_con
    out_n["nused"][hs] = b_used
    out_n["best_col"][hs] = b_col
    if events is not None:
        # best の列の時点で組になっていたもの(参照星ごとに、best の列までで最後に組になった星)
        last = {}
        for h, col, t, r, lom, _ in events:
            if col <= b_col[h]:
                last[(h, r)] = (col, t, r, lom)
        for (h, _), v in sorted(last.items(), key=lambda kv: (kv[0][0], kv[1][0])):
            rec[hs[h]].append(v)


# ------------------------------------------------------------------------------
#  画像の側の準備と、参照星の円錐検索
# ------------------------------------------------------------------------------

class _SvField:
    """1 回の sv_solve で使い回す画像の側の情報(星の並び・検証の並び・有効面積の格子)。"""

    def __init__(self, xy, flux, width, height, params):
        self.xy = np.asarray(xy, dtype=np.float64).reshape(-1, 2)
        self.W = float(width)
        self.H = float(height)
        self.p = params
        n = len(self.xy)
        self.n = n
        if flux is None:
            bright = np.arange(n)
        else:
            f = np.asarray(flux, dtype=np.float64).reshape(-1)
            bright = np.argsort(-np.where(np.isfinite(f), f, -np.inf), kind="stable")
        self.order = bright[sv_uniformize(self.xy[bright], width, height, params.uniformize_boxes)]
        self.nq = min(n, int(params.max_quad_stars))
        self.P = self.xy[self.order[:self.nq]]
        # 検証の星: 並べ替えた順の先頭 max_verify_stars 個を、明るい順に並べ直したもの
        rank = np.empty(n, dtype=np.int64)
        rank[bright] = np.arange(n)
        vset = self.order[:min(n, int(params.max_verify_stars))]
        self.vidx = vset[np.argsort(rank[vset], kind="stable")]
        self.vxy = self.xy[self.vidx]
        self.vpos = np.full(n, -1, dtype=np.int64)
        self.vpos[self.vidx] = np.arange(len(self.vidx))
        self._vorder = {}
        g = SV_AREA_GRID
        gx, gy = np.meshgrid((np.arange(g) + 0.5) / g * self.W - 0.5, (np.arange(g) + 0.5) / g * self.H - 0.5)
        self.grid = np.column_stack([gx.ravel(), gy.ravel()])
        self.center = np.array([(self.W - 1.0) / 2.0, (self.H - 1.0) / 2.0])
        self.half_diag = 0.5 * math.hypot(self.W, self.H)

    def verify_order(self, nw, nh):
        """検証の一様化(箱 nw × nh から 1 個ずつ)をした、検証の星の並び(vxy の行)。"""
        key = (int(nw), int(nh))
        if key not in self._vorder:
            self._vorder[key] = sv_box_order(self.vxy, self.W, self.H, key[0], key[1])
        return self._vorder[key]


def _sv_kd_small_ball(t, qt, r_t):
    """木 t(SvKdTree)を、半径の小さい問い合わせ(木の単位の中心 qt (Q, D)、半径 r_t (Q,))でたどる。

    枝は分割面までの距離だけで捨てる(左は x[dim] ≤ 位置、右は x[dim] ≥ 位置なので、q − r > 位置なら左に、
    q + r < 位置なら右に届かない)。箱を持たないぶん粗いが、半径が小さいとほとんどの問い合わせは 1 本の道しか
    たどらないので、SvKdTree.ball_candidates より速い。問い合わせごとに半径を変えられる。
    戻り値 (qi, pt): 問い合わせの番号と葉の点の番号(候補。距離の最終判定は呼ぶ側)。
    """
    qt = np.asarray(qt, dtype=np.float64)
    r_t = np.asarray(r_t, dtype=np.float64)
    qi = np.arange(len(qt), dtype=np.int64)
    node = np.zeros(len(qt), dtype=np.int64)
    mask = t.mask
    for _ in range(t.levels - 1):
        if len(qi) == 0:
            return np.zeros(0, np.int64), np.zeros(0, np.int64)
        raw = t.split[node].astype(np.int64)
        dim = raw & mask
        val = (raw & ~mask).astype(np.float64)
        q = qt[qi, dim]
        r = r_t[qi]
        il = np.nonzero(q - r <= val)[0]
        ir = np.nonzero(q + r >= val)[0]
        qi = np.concatenate([qi[il], qi[ir]])
        node = np.concatenate([2 * node[il] + 1, 2 * node[ir] + 2])
    if len(qi) == 0:
        return np.zeros(0, np.int64), np.zeros(0, np.int64)
    k = node - t.ninterior
    end = t.lr[k].astype(np.int64) + 1
    start = np.zeros_like(end)
    nz = k > 0
    start[nz] = t.lr[k[nz] - 1].astype(np.int64) + 1
    cnt = end - start
    first = np.cumsum(cnt) - cnt
    pt = np.repeat(start - first, cnt) + np.arange(int(cnt.sum()), dtype=np.int64)
    return np.repeat(qi, cnt), pt


def _sv_cone_pairs(f, centers, radii, small=False):
    """索引 f の星のうち、各中心(単位ベクトル (H, 3))から角距離 radii (H,) 以内のもの。

    中心をまとめて木に投げる(半径が 1.5 倍以上違うものは別の束にする。木をたどる半径は束の最大なので)。
    small なら半径の小さい問い合わせ用のたどり方(_sv_kd_small_ball、半径は問い合わせごと)で 1 回にまとめる。
    判定は stars_in_cone と同じ(正規化した星と中心の弦の長さ)。
    戻り値 (h, 星の番号, 単位ベクトル)。
    """
    t = f.startree
    H = len(centers)
    chord = 2.0 * np.sin(np.minimum(radii, math.pi) / 2.0)
    qt_all = t.to_tree(centers)
    if small:
        order = np.arange(H)
        groups = [order]
    else:
        order = np.argsort(chord, kind="stable")
        groups = []
        s = 0
        while s < H:
            c0 = chord[order[s]]
            e = s + int(np.searchsorted(chord[order[s:]], max(c0, 1e-12) * 1.5, side="right"))
            e = max(e, s + 1)
            groups.append(order[s:e])
            s = e
    hs_out, pt_out, xyz_out = [], [], []
    for grp in groups:
        qt = qt_all[grp]
        # 量子化(3 × 0.25 目盛り²)と、正規化で動く分(|xyz| − 1 ≤ 4e-10 ≒ 0.9 目盛り)を広げる
        if small:
            rt = np.sqrt((chord[grp] * t.scale) ** 2 + 0.75) + 4.0
            qi, pt = _sv_kd_small_ball(t, qt, rt)
        else:
            r_t = math.sqrt((float(chord[grp].max()) * t.scale) ** 2 + 0.75) + 4.0
            qi, pt = t.ball_candidates(qt, r_t)
        if len(pt) == 0:
            continue
        # 葉の中の点は大半が遠いので、木の単位のまま(実数に戻す前に)ふるう。木の単位の距離 = 実数の距離 × scale
        # (to_real は minval + 整数 / scale)。弦 + 2e-9(|v| と 1 の差 4e-10 の分を広げた値。最後の判定より広い)。
        # 先に 1 つめの次元の差だけで落とす(1 列だけ読むので安い)
        lim = (chord[grp][qi] + 2e-9) * t.scale
        k = np.nonzero(np.abs(t.data[pt, 0] - qt[qi, 0]) <= lim)[0]
        dt = t.data[pt[k]].astype(np.float64)
        dt -= qt[qi[k]]
        k = k[np.einsum("ij,ij->i", dt, dt) <= lim[k] * lim[k]]
        if len(k) == 0:
            continue
        qi, pt = qi[k], pt[k]
        v = t.to_real(t.data[pt])
        h = grp[qi]
        # 最後の判定は前と同じ式(stars_in_cone と同じ答えになるように)
        v = v / np.sqrt(np.einsum("ij,ij->i", v, v))[:, None]
        ok2 = np.nonzero(((v - centers[h]) ** 2).sum(axis=1) <= chord[h] ** 2)[0]
        hs_out.append(h[ok2])
        pt_out.append(pt[ok2])
        xyz_out.append(v[ok2])
    if not hs_out:
        return np.zeros(0, np.int64), np.zeros(0, np.int64), np.zeros((0, 3))
    return np.concatenate(hs_out), np.concatenate(pt_out), np.concatenate(xyz_out)


def _sv_series_cone(catalog, indexid, centers, radii, clock=None, small=False):
    """同じ系列の全タイルから、各中心の円の中の星を集める(タイルの境界をまたぐ視野でも欠けないように)。

    small なら半径の小さい問い合わせ用のたどり方を使う(_sv_cone_pairs)。
    戻り値 dict: h, sid(星の番号), fid(系列の中のファイルの位置), xyz, sweep。
    """
    fs = catalog.series[indexid]
    H = len(centers)
    parts = []
    if fs[0].tile_center_xyz is None:
        cand = [(0, np.arange(s, min(H, s + 32768))) for s in range(0, H, 32768)]   # 区切りごとに締め切りを見る
    elif H == 0:
        cand = []
    else:
        tc = np.array([f.tile_center_xyz for f in fs])
        tr = np.array([f.tile_radius_rad for f in fs])
        # 角距離 ≤ 外接半径 + 半径 を内積で(余弦は 0〜π で単調。わずかに広めに取る)。まず半径の最大で
        # タイルごとに 1 つの余弦と比べ(安い)、半径がそろっていないときは残った組だけ自分の半径で比べ直す
        rmax = float(np.max(radii))
        kk, hh = [], []
        for s in range(0, H, 8192):
            c = centers[s:s + 8192]
            dots = tc @ c.T
            k, h = np.nonzero(dots >= np.cos(np.minimum(tr + rmax + 1e-9, math.pi))[:, None])
            if len(k) and float(np.min(radii[s:s + 8192])) < rmax:
                lim = np.minimum(tr[k] + radii[s + h] + 1e-9, math.pi)
                m = dots[k, h] >= np.cos(lim)
                k, h = k[m], h[m]
            kk.append(k)
            hh.append(h + s)
        k = np.concatenate(kk)
        h = np.concatenate(hh)
        o = np.argsort(k, kind="stable")              # タイルの順、タイルの中は中心の番号の順
        k, h = k[o], h[o]
        cut = np.nonzero(np.diff(k))[0] + 1
        cand = [(int(kg[0]), hg) for kg, hg in zip(np.split(k, cut), np.split(h, cut)) if len(kg)]
    for k, hs in cand:
        if len(hs) == 0:
            continue
        f = fs[k]
        if f.broken is not None:                      # 中身が壊れていると分かったファイル(mark_broken)
            continue
        qi, sid, xyz = _sv_cone_pairs(f, centers[hs], radii[hs], small)
        if len(sid):
            parts.append((hs[qi], sid, np.full(len(sid), k), xyz, f.sweep(sid)))
        if clock is not None:
            clock.check()
    if not parts:
        return dict(h=np.zeros(0, np.int64), sid=np.zeros(0, np.int64), fid=np.zeros(0, np.int64),
                    xyz=np.zeros((0, 3)), sweep=np.zeros(0, np.uint8))
    return dict(h=np.concatenate([p[0] for p in parts]), sid=np.concatenate([p[1] for p in parts]),
                fid=np.concatenate([p[2] for p in parts]), xyz=np.concatenate([p[3] for p in parts]),
                sweep=np.concatenate([p[4] for p in parts]))


# ------------------------------------------------------------------------------
#  仮説の束を検証する
# ------------------------------------------------------------------------------

def _sv_hyp_take(hy, idx):
    """仮説の束(dict of arrays)から一部を取り出す。"""
    n = hy["n"]
    out = {}
    for k, v in hy.items():
        out[k] = v[idx] if isinstance(v, np.ndarray) and v.ndim >= 1 and len(v) == n else v
    out["n"] = len(np.arange(n)[idx])
    return out


def _sv_verify_hyps(field, catalog, f, hy, gamma=1.0, record=False, wcs=None, refs=None, clock=None,
                    quick=False):
    """仮説の束 hy を検証する(3.7 節の 1〜5)。

    hy は dict: n、crval・east・north (n, 3)、cd (n, 2, 2)、crpix (n, 2)、scale (n,)、qfield (n, dq)(画像の星の
    番号)、qindex (n, dq)(索引の星の番号)、fid (n,)(quad の索引の、系列の中の位置)、qc (n, 2)、Q2 (n,)。
    f は系列の代表のファイル(JITTER・CUTNSIDE・系列の番号を使う。系列の中では同じ値)。
    wcs を渡すと(仮説 1 個のとき)その WCS(SIP 込み)で参照星を投影する。refs で円錐検索の結果を使い回せる。
    gamma は σ の「quad からの距離による広がり」の掛け率(改良のとき 1 未満)。quick なら先に安い足切り
    (_sv_quick_pass)をかけ、通った仮説だけを検証する(探索のときだけ。改良と最後の検証ではかけない)。
    戻り値 dict: best・nmatch・ndistract・nconflict (n,)、NR・NT (n,)、quick_rejected (n,)、record なら matches
    [[(画像の星の番号, 参照星の単位ベクトル, ずれ [px], 1 つの一致の対数オッズ)]]。
    """
    p = field.p
    n = hy["n"]
    W, Hh = field.W, field.H
    d = float(p.distractor)
    if quick and SV_QUICK_STARS > 0 and wcs is None and refs is None and n > 0:
        keep = _sv_quick_pass(field, catalog, f, hy, gamma, clock)
        out = dict(best=np.full(n, -np.inf), nmatch=np.zeros(n, np.int64), ndistract=np.zeros(n, np.int64),
                   nconflict=np.zeros(n, np.int64), quick_rejected=~keep,
                   NR=np.zeros(n, np.int64), NT=np.zeros(n, np.int64))
        k = np.nonzero(keep)[0]
        if len(k):
            # 通った仮説を、参照星の数の見積もり(円の面積 × 系列の星の密度)の合計が SV_REF_CHUNK を
            # 超えないように分けて検証する(1 回の処理の時間とメモリを抑えるため)
            fs = catalog.series[f.indexid]
            area = 4.0 * math.pi * (1.0 if fs[0].tile_center_xyz is None else 1.0 / (12.0 * f.hpnside ** 2))
            rad = np.minimum(field.half_diag * hy["scale"][k] * 1.05 / SV_ARCSEC_PER_RAD, math.pi)
            est = 2.0 * math.pi * (1.0 - np.cos(rad)) * (f.nstars / area) + 1.0
            grp = np.floor(np.cumsum(est) / SV_REF_CHUNK).astype(np.int64)
            for g in np.unique(grp):
                kk = k[grp == g]
                sub = _sv_verify_hyps(field, catalog, f, _sv_hyp_take(hy, kk), gamma=gamma, clock=clock)
                for key in ("best", "nmatch", "ndistract", "nconflict", "NR", "NT"):
                    out[key][kk] = sub[key]
                if clock is not None:
                    clock.check()
        return out
    # 1. 参照星: 画像中心から半径 hypot(W, H)/2 × スケール × 1.05 の円の中を、同じ系列の全タイルから
    if wcs is not None:
        cen = wcs.pix_to_xyz(field.center[0], field.center[1]).reshape(1, 3)
    else:
        cen = _sv_pix_to_xyz_many(np.full(n, field.center[0]), np.full(n, field.center[1]),
                                  hy["crval"], hy["east"], hy["north"], hy["cd"], hy["crpix"])
    radii = field.half_diag * hy["scale"] * 1.05 / SV_ARCSEC_PER_RAD
    if refs is None:
        refs = _sv_series_cone(catalog, f.indexid, cen, radii, clock)
    rh = refs["h"]
    if wcs is not None:
        rx, ry, ok = wcs.xyz_to_pix(refs["xyz"])
    else:
        cdinv = np.linalg.inv(hy["cd"])
        rx, ry, ok = _sv_project_many(refs["xyz"], hy["crval"][rh], hy["east"][rh], hy["north"][rh],
                                      cdinv[rh], hy["crpix"][rh])
    ok = ok & (rx >= -0.5) & (rx <= W - 0.5) & (ry >= -0.5) & (ry <= Hh - 0.5)
    # quad の星は参照星から外す
    same = refs["fid"] == hy["fid"][rh]
    if same.any():
        isq = (refs["sid"][:, None] == hy["qindex"][rh]).any(axis=1) & same
        ok &= ~isq
    sel = np.nonzero(ok)[0]
    rh, rx, ry = rh[sel], rx[sel], ry[sel]
    srt = np.lexsort((refs["sid"][sel], refs["fid"][sel], refs["sweep"][sel], rh))
    sel, rh, rx, ry = sel[srt], rh[srt], rx[srt], ry[srt]
    NR0 = np.bincount(rh, minlength=n).astype(np.float64)
    # 2・3. 位置の分散と関連半径(RoR)
    pix2 = p.verify_pix ** 2 + (f.jitter_arcsec / hy["scale"]) ** 2
    fac = W * Hh * (1.0 - d) / (4.0 * math.pi * np.maximum(NR0, 1.0) * pix2) - 1.0
    ror2 = hy["Q2"] * np.maximum(1.0, fac)          # NR0 = 0 の仮説はどのみち検証できない
    dq2 = (rx - hy["qc"][rh, 0]) ** 2 + (ry - hy["qc"][rh, 1]) ** 2
    keep = dq2 <= ror2[rh]
    sel, rh, rx, ry = sel[keep], rh[keep], rx[keep], ry[keep]
    NR = np.bincount(rh, minlength=n)
    rstart = np.concatenate([[0], np.cumsum(NR)[:-1]]).astype(np.int64)
    g2 = ((field.grid[None, :, :] - hy["qc"][:, None, :]) ** 2).sum(axis=-1)
    frac = (g2 <= ror2[:, None]).mean(axis=1)
    area = W * Hh * np.maximum(frac, 1.0 / len(field.grid))
    logbg = -np.log(area)
    # 4. 画像の星の並び(検証の一様化。箱 = 索引の CUTNSIDE の HEALPix の 1 辺)
    nw, nh = _sv_verify_boxes(field, f, hy["scale"])
    ref_xy = np.column_stack([rx, ry])
    NR_ok = NR > 0                     # NR が 0 の仮説は検証できない(対数オッズ −∞)
    best = np.full(n, -np.inf)
    out = dict(best=best, nmatch=np.zeros(n, np.int64), ndistract=np.zeros(n, np.int64),
               nconflict=np.zeros(n, np.int64), quick_rejected=np.zeros(n, dtype=bool),
               NR=NR, NT=np.zeros(n, np.int64))
    live = np.nonzero(NR_ok)[0]
    if clock is not None:
        clock.check()
    if len(live):
        test = _sv_test_lists(field, hy, live, nw, nh, ror2, len(field.vxy), len(field.vxy))
        NT = (test >= 0).sum(axis=1)
        test = test[:, :max(1, int(NT.max()))]
        sig2 = _sv_sigma2(field, hy, live, test, pix2, gamma)
        res = sv_verify_core(field.vxy, test, sig2, ref_xy, rstart[live], NR[live], logbg[live], p,
                             record=record, clock=clock)
        for k in ("best", "nmatch", "ndistract", "nconflict"):
            out[k][live] = res[k]
        out["NT"][live] = NT
    if record:
        mm = [[] for _ in range(n)]
        for j, h in enumerate(live):
            for col, t, r, lom in res["matches"][j]:
                r = int(rstart[h]) + (r - int(rstart[live][j]))      # 束の中の番号 → ref_xy の行
                dist = math.hypot(field.vxy[t, 0] - rx[r], field.vxy[t, 1] - ry[r])
                mm[h].append((int(field.vidx[t]), refs["xyz"][sel[r]].copy(), dist, lom))
        out["matches"] = mm
    return out


def _sv_verify_boxes(field, f, scale):
    """検証の一様化の箱の数 (nw, nh)(仮説ごと)。箱 = 索引の CUTNSIDE の HEALPix の 1 辺(3.7 節の 4)。"""
    scale = np.asarray(scale, dtype=np.float64)
    if f.cutnside:
        cutpix = sv_healpix_side_rad(f.cutnside) * SV_ARCSEC_PER_RAD / scale
        nw = np.maximum(1, np.floor(field.W / cutpix + 0.5)).astype(np.int64)
        nh = np.maximum(1, np.floor(field.H / cutpix + 0.5)).astype(np.int64)
        return nw, nh
    return np.ones(len(scale), np.int64), np.ones(len(scale), np.int64)


def _sv_quick_pass(field, catalog, f, hy, gamma=1.0, clock=None):
    """検証の前の安い足切り。仮説ごとに、検証の並びの先頭 SV_QUICK_STARS 個の画像の星のどれかから 5σ 以内に
    系列の星(quad の星を除く)があるかを返す (n,) bool。

    画像の星を仮説の WCS で空へ戻し、半径 5σ(× 画素スケール)の小さい円で星の木を引く(全部の参照星を集めて
    投影するより桁違いに安い)。RoR はここでは見ない(参照星の数が要るため)。真の解では明るい星から一致するので
    落ちない。探索の仮説の 9 割以上がここで落ちる。
    """
    n = hy["n"]
    K = SV_QUICK_STARS
    hs = np.arange(n)
    nw, nh = _sv_verify_boxes(field, f, hy["scale"])
    inf = np.full(n, np.inf)
    tq = _sv_test_lists(field, hy, hs, nw, nh, inf, K, min(len(field.vxy), K + hy["qfield"].shape[1]))
    pix2 = field.p.verify_pix ** 2 + (f.jitter_arcsec / hy["scale"]) ** 2
    s2 = _sv_sigma2(field, hy, hs, tq, pix2, gamma)
    hh, cc = np.nonzero(tq >= 0)
    ok = np.zeros(n, dtype=bool)
    if len(hh) == 0:
        return ok
    txy = field.vxy[tq[hh, cc]]
    P = _sv_pix_to_xyz_many(txy[:, 0], txy[:, 1], hy["crval"][hh], hy["east"][hh], hy["north"][hh],
                            hy["cd"][hh], hy["crpix"][hh])
    rad = 5.0 * np.sqrt(s2[hh, cc]) * hy["scale"][hh] / SV_ARCSEC_PER_RAD
    got = _sv_series_cone(catalog, f.indexid, P, rad, clock, small=True)
    ph = hh[got["h"]]
    isq = (got["fid"] == hy["fid"][ph]) & (got["sid"][:, None] == hy["qindex"][ph]).any(axis=1)
    ok[ph[~isq]] = True
    return ok


def _sv_test_lists(field, hy, hs, nw, nh, ror2, ncols, ncand):
    """仮説 hs の画像の星の並び (len(hs), ncols)(field.vxy の行。−1 は詰め物)。

    検証の一様化の並び(箱 nw × nh)の先頭 ncand 個から、quad の星と RoR の外の星を除いて先頭 ncols 個。
    """
    NV = len(field.vxy)
    out = np.full((len(hs), ncols), -1, dtype=np.int64)
    if NV == 0 or ncols == 0:
        return out
    m = min(NV, ncand)
    keys = np.stack([nw[hs], nh[hs]], axis=1)
    for key in {tuple(k) for k in keys.tolist()}:
        g = np.nonzero((keys[:, 0] == key[0]) & (keys[:, 1] == key[1]))[0]
        h = hs[g]
        order = field.verify_order(*key)
        inv = np.empty(NV, dtype=np.int64)
        inv[order] = np.arange(NV)
        cand = order[:m]
        use = np.ones((len(g), m), dtype=bool)
        qpos = inv[field.vpos[hy["qfield"][h]]]               # quad の星の、並びの中の位置
        qpos = np.where(field.vpos[hy["qfield"][h]] >= 0, qpos, m)
        rr, cc = np.nonzero(qpos < m)
        use[rr, qpos[rr, cc]] = False
        cxy = field.vxy[cand]
        far = ((cxy[None, :, :] - hy["qc"][h][:, None, :]) ** 2).sum(axis=-1) > ror2[h][:, None]
        use &= ~far
        pos = np.argsort(~use, axis=1, kind="stable")[:, :ncols]
        got = np.take_along_axis(use, pos, axis=1)
        out[g, :pos.shape[1]] = np.where(got, cand[pos], -1)
    return out


def _sv_sigma2(field, hy, hs, test, pix2, gamma):
    """位置の分散 σᵢ² = pix² × (1 + γ·Rᵢ²/Q²)(Rᵢ = 星 i と quad の中心の距離。3.7 節の 2)。"""
    txy = field.vxy[np.where(test >= 0, test, 0)]
    R2 = ((txy - hy["qc"][hs][:, None, :]) ** 2).sum(axis=-1)
    return pix2[hs][:, None] * (1.0 + gamma * R2 / hy["Q2"][hs][:, None])


# ------------------------------------------------------------------------------
#  改良(tweak、3.8 節)
# ------------------------------------------------------------------------------

def _sv_wcs_of(hy, i=0):
    return SvWcs(hy["crval"][i], hy["crpix"][i], hy["cd"][i])


def _sv_hyp_from_wcs(hy1, wcs):
    """仮説 1 個の WCS を wcs(TAN 部分)に置き換えた束(検証の参照星の範囲を決めるのに使う)。"""
    h = dict(hy1)
    r = wcs.crval_xyz.reshape(1, 3)
    e, nn = _sv_tan_basis_many(r)
    h["crval"], h["east"], h["north"] = r, e, nn
    h["cd"] = wcs.cd.reshape(1, 2, 2)
    h["crpix"] = wcs.crpix.reshape(1, 2)
    h["scale"] = np.array([wcs.pixel_scale_arcsec()])
    return h


def _sv_fit_pairs(field, matches, hy1, quad_xyz):
    """改良の当てはめに使う組: 一致した星(重み = 1 つの一致の対数オッズから)と quad の星(重み 1)。"""
    fxy = [field.xy[m[0]] for m in matches]
    sxyz = [m[1] for m in matches]
    w = list(sv_logodds_weight([m[3] for m in matches])) if matches else []
    for k, fi in enumerate(hy1["qfield"][0]):
        fxy.append(field.xy[fi])
        sxyz.append(quad_xyz[k])
        w.append(1.0)
    return np.array(fxy).reshape(-1, 2), np.array(sxyz).reshape(-1, 3), np.array(w, dtype=np.float64)


def _sv_wrms(wcs, fxy, sxyz, w):
    """重みつきの残差の RMS(px)。"""
    r = sv_wcs_residuals(wcs, fxy, sxyz)
    r2 = (r * r).sum(axis=1)
    ok = np.isfinite(r2) & (w > 0)
    if not ok.any():
        return math.inf
    return math.sqrt(float((w[ok] * r2[ok]).sum() / w[ok].sum()))


def sv_f_upper(d1, d2, z=SV_SIP_F_Z):
    """F 分布(自由度 d1, d2)の上側の点。z は同じ確率の標準正規分布の点(既定は上側 1 %)。

    Paulson の近似(F^(1/3) を正規分布で近似する式を F について解く)。d1 ≥ 6・d2 ≥ 12 で正確な値との差は 1 % 以下。
    """
    d1, d2 = float(d1), float(d2)
    a, b = 1.0 - 2.0 / (9.0 * d2), 1.0 - 2.0 / (9.0 * d1)
    c2, c1 = 2.0 / (9.0 * d2), 2.0 / (9.0 * d1)
    A = a * a - z * z * c2
    B = a * b
    C = b * b - z * z * c1
    if A <= 0.0 or B * B - A * C < 0.0:
        return math.inf
    return ((B + math.sqrt(B * B - A * C)) / A) ** 3


def _sv_spread_boxes(xy, W, H, g=SV_SIP_GRID):
    """点が画像を g × g に分けた箱のいくつに入っているか。"""
    if len(xy) == 0:
        return 0
    ix = np.clip(np.floor((xy[:, 0] + 0.5) / W * g), 0, g - 1).astype(np.int64)
    iy = np.clip(np.floor((xy[:, 1] + 0.5) / H * g), 0, g - 1).astype(np.int64)
    return int(len(np.unique(iy * g + ix)))


def _sv_refine(field, catalog, f, hy1, order, clock=None, steps=SV_TWEAK_STEPS, final=True):
    """仮説 1 個を改良する(3.8 節)。

    一致した組を今の WCS で投影し直した参照星との最近傍で取り直し、重みつき最小二乗で TAN(CD と CRVAL)を
    当て直す。これを steps 回、σ の「quad からの距離による広がり」を γ = 0.7^k で弱めながら繰り返す。
    反復の当てはめも CRPIX を画像の中心に置く(最後の形と同じ。当てた点の重心に置くと、点が画像の片側に
    寄ったとき、広い視野では接点の違う TAN になって中心のスケールが狂い(短辺 19° の合成星野で −2 %)、
    その WCS で取り直した一致がさらに片側へ寄った)。接点の初めの値は今の WCS で画像の中心が写る位置。
    final(解として返す前の改良)なら、最後の当て直しは quad の星を除いた一致が SV_TWEAK_MIN_MATCHES 個以上あれば
    一致だけで行う(quad の星はコードの許容(AB の 1 % ≒ 数 px)の中で選ばれた組なので、偽の星が混じっても照合を
    通る。合成星野 few20 では quad の 1 星が索引の星から 3.4 px 離れた偽の星で、それを重み 1 で入れた当てはめは
    スケールが 0.2 % ずれた。検証の一致は quad と別の証拠)。足りなければ quad の星(重み 1)も入れる。
    tune-up(final=False。solve 未満の仮説を引き上げるか決める)では、これまでどおり quad の星を入れたまま
    当てる(quad の星を外すと、偶然の一致だけに当てはめて対数オッズを押し上げやすくなるため)。
    order ≥ 2 で SIP を付けるのは、当てる組が 3 × (未知数 = 2 軸 × 項の数。2 次なら 12)以上あり、
    その星が画像を 3 × 3 に分けた箱の 7 つ以上にあり、TAN に比べた残差の減り方が F 検定(上側 1 %)で
    有意なときだけ(一致が 20 個前後で 2 次を当てると中心まで 1 px 動いた。星が左 1/3 だけの画像に付けると
    隅が 7〜13 px 動いた)。最後に γ = 1 で検証し直す。
    戻り値 (wcs, 検証の結果)。
    """
    wcs = _sv_wcs_of(hy1)
    fs = catalog.series[f.indexid]
    quad_xyz = fs[int(hy1["fid"][0])].star_xyz(hy1["qindex"][0])
    cx, cy = float(field.center[0]), float(field.center[1])
    # 参照星は 1 回だけ集める(少し広めに。改良で中心とスケールが少し動いても足りるように)
    cen = wcs.pix_to_xyz(cx, cy).reshape(1, 3)
    rad = np.array([field.half_diag * float(hy1["scale"][0]) * 1.25 / SV_ARCSEC_PER_RAD])
    refs = _sv_series_cone(catalog, f.indexid, cen, rad)
    fxy = sxyz = w = None
    nmatch = 0
    for k in range(int(steps)):
        gamma = SV_TWEAK_GAMMA ** k
        res = _sv_verify_hyps(field, catalog, f, _sv_hyp_from_wcs(hy1, wcs), gamma=gamma, record=True,
                              wcs=wcs, refs=refs, clock=clock)
        fxy, sxyz, w = _sv_fit_pairs(field, res["matches"][0], hy1, quad_xyz)
        nmatch = len(res["matches"][0])
        if (w > 0).sum() < 3:
            break
        try:
            wcs = sv_fit_wcs_lsq(fxy, sxyz, weights=w, crpix=field.center, sip_order=0,
                                 crval_xyz=wcs.pix_to_xyz(cx, cy).reshape(3))
        except ValueError:
            break
    if final and fxy is not None and int((w[:nmatch] > 0).sum()) >= SV_TWEAK_MIN_MATCHES:
        fxy, sxyz, w = fxy[:nmatch], sxyz[:nmatch], w[:nmatch]     # quad の星(後ろに付いている)を外す
    nm = int((w > 0).sum()) if w is not None else 0
    cv = wcs.pix_to_xyz(cx, cy).reshape(3)
    if int(order) >= 2 and fxy is not None:
        terms = (int(order) + 1) * (int(order) + 2) // 2
        # 未知数 = 2 軸 × 項の数。星が画像の大部分に散っているときだけ(片寄っていると、星の無い側へ
        # 多項式を外挿して大きくずれる。左 1/3 だけの星で隅が 7〜13 px 動いた)
        if nm >= 3 * 2 * terms and _sv_spread_boxes(fxy[w > 0], field.W, field.H) >= SV_SIP_MIN_BOXES:
            try:
                tan_c = sv_fit_wcs_lsq(fxy, sxyz, weights=w, crpix=field.center, sip_order=0, crval_xyz=cv)
                sip_c = sv_fit_wcs_lsq(fxy, sxyz, weights=w, crpix=field.center, sip_order=int(order),
                                       width=field.W, height=field.H, crval_xyz=cv)
                # 残差の減り方の F 検定(上側 1 %)。「自由度で割った残差が下がれば採る」(F > 1)だと、
                # 歪みが無くても 4 割ほどで SIP が付いた
                rss_t = _sv_wrms(tan_c, fxy, sxyz, w) ** 2
                rss_s = _sv_wrms(sip_c, fxy, sxyz, w) ** 2
                d1, d2 = 2 * terms - 2 * 3, 2 * nm - 2 * terms
                if rss_s > 0.0:
                    take = (rss_t - rss_s) / d1 > sv_f_upper(d1, d2) * rss_s / d2
                else:
                    take = rss_t > 0.0
                if take:
                    wcs = sip_c
            except ValueError:
                pass
    if not wcs.has_sip and fxy is not None and nm >= 3:
        try:
            wcs = sv_fit_wcs_lsq(fxy, sxyz, weights=w, crpix=field.center, sip_order=0, crval_xyz=cv)
        except ValueError:
            pass
    res = _sv_verify_hyps(field, catalog, f, _sv_hyp_from_wcs(hy1, wcs), gamma=1.0, record=True,
                          wcs=wcs, refs=refs, clock=clock)
    return wcs, res


def sv_verify_wcs(xy, flux, width, height, catalog, indexid, wcs, *, quad=None, params=None):
    """与えた WCS をそのまま検証して対数オッズを返す(試験と、移植先で同じ値が出るかの確かめ用。探索では使わない)。

    参照星は系列 indexid の全タイルから集める(探索の検証と同じ)。quad = (画像の星の番号, 索引の名前,
    索引の星の番号) を渡すと、探索と同じく σ の広がりと RoR を quad から決め、quad の星を両側から外す
    (SvSolution の quad_field・index_name・quad_index をそのまま渡せば、解の logodds と同じ値になる)。
    quad が無ければ σ = pix(広がりなし)、RoR は画像全体。
    戻り値 dict: logodds, nmatch, ndistract, nfield, nref, matches [(画像の星の番号, 参照星の単位ベクトル, ずれ [px])]。
    """
    p = params if params is not None else SvParams()
    field = _SvField(xy, flux, width, height, p)
    fs = catalog.series[int(indexid)]
    r = np.asarray(wcs.crval_xyz, dtype=np.float64).reshape(1, 3)
    e, nn = _sv_tan_basis_many(r)
    hy = dict(n=1, crval=r, east=e, north=nn, cd=np.asarray(wcs.cd, dtype=np.float64).reshape(1, 2, 2),
              crpix=np.asarray(wcs.crpix, dtype=np.float64).reshape(1, 2),
              scale=np.array([wcs.pixel_scale_arcsec()]))
    if quad is None:
        hy.update(qfield=np.zeros((1, 0), np.int64), qindex=np.zeros((1, 0), np.int64),
                  fid=np.array([-1]), qc=field.center.reshape(1, 2), Q2=np.array([np.inf]))
        gamma = 0.0
    else:
        qf, name, qi = quad
        qf = np.asarray(qf, dtype=np.int64).reshape(-1)
        fid = next(i for i, g in enumerate(fs) if g.name == name)
        P = field.xy[qf]
        hy.update(qfield=qf.reshape(1, -1), qindex=np.asarray(qi, dtype=np.int64).reshape(1, -1),
                  fid=np.array([fid]), qc=((P[0] + P[1]) / 2.0).reshape(1, 2),
                  Q2=np.array([float(((P[1] - P[0]) ** 2).sum()) / 4.0]))
        gamma = 1.0
    res = _sv_verify_hyps(field, catalog, fs[0], hy, gamma=gamma, record=True, wcs=wcs)
    return dict(logodds=float(res["best"][0]), nmatch=int(res["nmatch"][0]),
                ndistract=int(res["ndistract"][0]), nfield=int(res["NT"][0]), nref=int(res["NR"][0]),
                matches=[(m[0], m[1], m[2]) for m in res["matches"][0]])


# ------------------------------------------------------------------------------
#  照合(索引 1 本ぶん)
# ------------------------------------------------------------------------------

def _sv_match_file(field, f, fid, batch, s_lo, s_hi, hint, clock):
    """quad の束を索引 f に照合し、安く捨ててから 4 点で当てはめた仮説の束を返す(3.4〜3.6 節)。

    batch は dict: quads (m, dq)(field.P の行)、ab (m,)(|AB| px)、q・row・perm・mirror(sv_code_variants)。
    戻り値 (仮説の束か None, 当たりの数)。
    """
    p = field.p
    tol = float(p.codetol)
    lo, hi = sv_quad_ab_range(f.scale_lo_rad, f.scale_hi_rad, s_lo, s_hi, field.W, field.H, tol,
                              p.quad_min_frac)
    abv = batch["ab"][batch["row"]]
    vsel = np.nonzero((abv >= lo) & (abv <= hi))[0]
    if len(vsel) == 0:
        return None, 0
    # 三角形(2 次元のコード)は、同じ位置のずれでコードの L2 が √(D/4) 倍になるので許容誤差もそう縮める
    # (Lang et al. の再現率 95 % の値も quad 0.0095・三角形 0.0064 で、比はほぼ 1/√2)
    qi, qid = f.code_ball(batch["q"][vsel], tol * math.sqrt(f.code_dim / 4.0),
                          check=None if clock is None else clock.check)
    if len(qi) == 0:
        return None, 0
    if clock is not None:
        clock.check()
    nhit = len(qi)
    v = vsel[qi]
    row = batch["row"][v]
    perm = batch["perm"][v]
    stars = f.quad_stars(qid)
    dq = stars.shape[1]
    S = f.star_xyz(stars.reshape(-1)).reshape(-1, dq, 3)
    # 安い棄却 1: 索引の AB の角距離 ÷ 画像の |AB| が [s_lo(1 − tol), s_hi(1 + tol)] の外
    th = sv_angle_between(S[:, 0], S[:, 1]) * SV_ARCSEC_PER_RAD
    s_impl = th / batch["ab"][row]
    ok = (s_impl >= s_lo * (1.0 - tol)) & (s_impl <= s_hi * (1.0 + tol))
    # 安い棄却 2: 索引の quad の中心がヒントの円(半径 + 画像の対角の半分)の外
    if hint is not None and ok.any():
        hc, hr = hint
        qc = sv_normalize(S[:, 0] + S[:, 1])
        reach = hr + field.half_diag * s_impl * 1.05 / SV_ARCSEC_PER_RAD
        ok &= sv_angle_between(qc, hc[None, :]) <= reach
    k = np.nonzero(ok)[0]
    if len(k) == 0:
        return None, nhit
    local = np.take_along_axis(batch["quads"][row[k]], perm[k], axis=1)
    F = field.P[local]
    fit = sv_fit_similarity_many(F, S[k])
    # スケールの判定は安い棄却 1 と同じ許容で(許容なしだと、範囲を 1 つの値で渡したときにほぼ当たらない)
    good = fit["ok"] & (fit["scale"] >= s_lo * (1.0 - tol)) & (fit["scale"] <= s_hi * (1.0 + tol))
    g = np.nonzero(good)[0]
    if len(g) == 0:
        return None, nhit
    # 同じ組(画像の quad の行・索引の quad・星の対応)は 1 つにまとめ、newpoint の順(quad の行の順)に並べる
    key = np.column_stack([row[k[g]], qid[k[g]], local[g]])
    _, first = np.unique(key, axis=0, return_index=True)
    first = np.sort(first)
    first = first[np.argsort(row[k[g]][first], kind="stable")]
    gg = g[first]
    kk = k[gg]
    loc = local[gg]
    Fq = field.P[loc]
    hy = dict(n=len(gg), crval=fit["crval"][gg], east=fit["east"][gg], north=fit["north"][gg],
              cd=fit["cd"][gg], crpix=fit["crpix"][gg], scale=fit["scale"][gg],
              qfield=field.order[loc], qindex=stars[kk], fid=np.full(len(gg), fid, dtype=np.int64),
              qc=(Fq[:, 0] + Fq[:, 1]) / 2.0, Q2=batch["ab"][row[kk]] ** 2 / 4.0,
              row=row[kk], mirror=batch["mirror"][v[kk]], quad_id=qid[kk])
    return hy, nhit


# ------------------------------------------------------------------------------
#  探索の本体
# ------------------------------------------------------------------------------

def sv_blind_scale_bounds(catalog, width):
    """スケール不明のときの範囲(″/px)。手持ちの段の quad の大きさから画像の幅で逆算する
    (zahyou_core の blind_scale_bounds と同じ: 下限 × 0.9、上限 × 1.1)。"""
    cov = catalog.coverage_arcmin()
    lo_fov, hi_fov = cov if cov else (2.0, 2000.0)
    low = lo_fov * 60.0 / float(width) * 0.9
    high = hi_fov * 60.0 / float(width) * 1.1
    return max(low, 0.01), min(high, 3600.0)


_SV_TILE_EDGE = {}                 # (healpix, nside) → (境界の点 (4·64, 3), 隣り合う点の間の角の最大の半分)


def _sv_tile_edge(f):
    """タイルの境界を辺ごとに 64 点で区切った点と、境界上のどの点からも最寄りの点までの角距離の上限。"""
    key = (int(f.healpix), int(f.hpnside))
    if key not in _SV_TILE_EDGE:
        t = np.linspace(0.0, 1.0, 65)[:-1]
        zero, one = np.zeros_like(t), np.ones_like(t)
        dx = np.concatenate([t, one, 1.0 - t, zero])          # 境界を一周する順
        dy = np.concatenate([zero, t, one, 1.0 - t])
        pts = sv_healpix_xy_point(key[0], key[1], dx, dy)
        gap = sv_angle_between(pts, np.roll(pts, -1, axis=0))
        _SV_TILE_EDGE[key] = (pts, float(gap.max()) / 2.0)
    return _SV_TILE_EDGE[key]


def _sv_hint_tiles(files, field, s_hi, hint_t, p):
    """ヒントがあるとき、タイルの索引を「その段の quad が照合で残りうるタイル」に絞る。

    catalog.select はタイルを「ヒントの円(半径 + 画像の対角の半分 × スケールの上限)とタイルの外接円が重なるか」で
    選ぶが、スケール不明だと上限が大きく、細かい段でも 20° 以上先のタイルまで入る。段ごとには、照合で残る
    quad の画素スケールは s_impl = AB の角距離 ÷ AB の画素 ≤ SCALE_U ÷ (quad_min_frac × 短辺)(AB の画素は
    quad 作りの下限以上)で、s_hi·(1 + codetol) も超えない。_sv_match_file は quad の中心がヒントの円
    (半径 + 対角の半分 × s_impl × 1.05)の外なら捨てるので、残る quad の星 A は、ヒントから
    R = その円の半径 + SCALE_U/2 以内にある。星はタイルの中だけにあるので、タイルの中に R 以内の点が無ければ
    そのタイルからは仮説が出ない。「中に R 以内の点がある」は、ヒントがタイルの中にあるか、境界の標本点のどれかが
    R + 標本の間隔の半分 以内にあるかで判定する(取りこぼさない側)。参照星はこれまでどおり系列の全タイルから集める。
    全天の索引はそのまま。索引の quad の AB は SCALE_U 以下(手元の索引で 1e-7 以内まで確かめた)だが 0.1 % の余裕をとる。
    """
    hc, hr = hint_t
    short = min(field.W, field.H)
    out = []
    home = {}
    for f in files:
        if f.tile_center_xyz is None:
            out.append(f)
            continue
        U = f.scale_hi_rad * 1.001
        s_max = s_hi * (1.0 + float(p.codetol))
        if p.quad_min_frac > 0:
            s_max = min(s_max, U * SV_ARCSEC_PER_RAD / (float(p.quad_min_frac) * short))
        R = hr + field.half_diag * s_max * 1.05 / SV_ARCSEC_PER_RAD + U / 2.0
        if float(sv_angle_between(f.tile_center_xyz, hc)) > f.tile_radius_rad + R + 1e-9:
            continue
        if f.hpnside not in home:
            home[f.hpnside] = int(sv_healpix_xy_from_xyz(hc, f.hpnside))
        if home[f.hpnside] != f.healpix:
            pts, half_gap = _sv_tile_edge(f)
            if float(np.min(sv_angle_between(pts, hc[None, :]))) > R + half_gap + 1e-6:
                continue
        out.append(f)
    return out


def sv_solve(xy, flux, width, height, catalog, *, scale_lo=None, scale_hi=None, hint=None,
             deadline=None, cancel=None, log=None, params=None, trace=None):
    """星のリストから WCS を求める(設計書 3 節)。解けなければ None。

    xy: (N, 2) の星の位置(0 始まり、x = 列、y = 行)。flux: 明るさ(並べ替えに使う。None なら xy の順)。
    catalog: SvIndexCatalog。scale_lo / scale_hi: 画素スケールの範囲(″/px。無ければ手持ちの段から逆算)。
    hint: (赤経, 赤緯, 半径)(度)か None。deadline: time.monotonic() の締め切り。cancel: 呼ぶと True/False を
    返す関数(True なら SvCancelled を投げる)。log: 1 行ずつの文字列を受ける関数。trace: JSON を書くパス。
    hint の値が数でない・赤緯が ±90° の外・半径が負なら ValueError(黙って別の場所を探したり、何も当たらない
    探索を締め切りまで続けたりしないように)。
    """
    if hint is not None:
        try:
            h_ra, h_dec, h_rad = (float(v) for v in hint)
        except (TypeError, ValueError):
            raise ValueError(f"ヒントは (赤経, 赤緯, 半径) の 3 つの数で渡してください({hint!r})")
        if not (math.isfinite(h_ra) and math.isfinite(h_dec) and math.isfinite(h_rad)):
            raise ValueError(f"ヒントに数でない値があります(赤経 {h_ra}、赤緯 {h_dec}、半径 {h_rad})")
        if not -90.0 <= h_dec <= 90.0:
            raise ValueError(f"ヒントの赤緯 {h_dec}° が −90°〜+90° の外です")
        if h_rad < 0.0:
            raise ValueError(f"ヒントの半径 {h_rad}° が負です")
    t0 = time.monotonic()
    p = params if params is not None else SvParams()
    clock = _SvClock(deadline, cancel)
    st = dict(quads=0, hits=0, hyps=0, flushes=[], hyp_log=[], t0=t0, trace=bool(trace))
    sol = None
    status = "unsolved"
    try:
        sol = _sv_search(xy, flux, width, height, catalog, scale_lo, scale_hi, hint, clock, log, p, st)
        status = "solved" if sol is not None else "unsolved"
    except _SvTimeUp:
        status = "deadline"
        if log:
            log(f"締め切り: quad {st['quads']:,} 個・仮説 {st['hyps']:,} 個を試して解けず"
                f"(対数オッズの最大 {st.get('logodds_max', -math.inf):.1f}、{time.monotonic() - t0:.1f} 秒)")
    except SvCancelled:
        status = "cancelled"
        raise
    finally:
        if trace:
            _sv_write_trace(trace, st, p, sol, status, width, height, scale_lo, scale_hi, hint, xy, flux)
    return sol


def _sv_search(xy, flux, width, height, catalog, scale_lo, scale_hi, hint, clock, log, p, st):
    t0 = st["t0"]
    field = _SvField(xy, flux, width, height, p)
    clock.check()
    if field.n < 3:
        if log:
            log(f"星が {field.n} 個しかないので解けません")
        return None
    b_lo, b_hi = sv_blind_scale_bounds(catalog, width)
    s_lo = float(scale_lo) if scale_lo is not None else b_lo
    s_hi = float(scale_hi) if scale_hi is not None else b_hi
    if s_hi < s_lo:
        s_lo, s_hi = s_hi, s_lo
    hint_t = None
    if hint is not None:
        hint_t = (sv_radec_to_xyz(float(hint[0]), float(hint[1])), math.radians(float(hint[2])))
    files = catalog.select(s_lo, s_hi, width, height,
                           hint_xyz=None if hint_t is None else hint_t[0],
                           hint_radius_rad=None if hint_t is None else hint_t[1])
    if hint_t is not None:
        files = _sv_hint_tiles(files, field, s_hi, hint_t, p)
    st["files"] = [f.name for f in files]
    st["scale"] = (s_lo, s_hi)
    if not files:
        if log:
            log("使える星図データ(索引)がありません")
        return None
    # 索引ごとの AB の範囲と、系列の中の位置
    info = []
    for f in files:
        lo, hi = sv_quad_ab_range(f.scale_lo_rad, f.scale_hi_rad, s_lo, s_hi, field.W, field.H,
                                  p.codetol, p.quad_min_frac)
        fid = next(i for i, g in enumerate(catalog.series[f.indexid]) if g is f)
        if hi >= lo and f.dimquads <= max(3, field.nq):
            info.append((f, fid, lo, hi))
    if not info:
        return None
    dqs = sorted({f.dimquads for f, _, _, _ in info})
    ab = {dq: (min(lo for f, _, lo, _ in info if f.dimquads == dq),
               max(hi for f, _, _, hi in info if f.dimquads == dq)) for dq in dqs}
    if log:
        series = sorted({f.indexid for f, _, _, _ in info})
        log(f"星 {field.n} 個(quad に {field.nq} 個)、スケール {s_lo:.3g}〜{s_hi:.3g}″/px、"
            f"索引 {len(info)} 本(系列 {', '.join(map(str, series))})"
            + ("" if hint is None else f"、ヒント ({hint[0]:.3f}, {hint[1]:+.3f}) 半径 {hint[2]:g}°"))
    L2, inside = sv_quad_geometry(field.P, p.codetol)
    pending = {dq: [] for dq in dqs}
    npend = 0
    target = SV_FIRST_BATCH
    for n in range(field.nq):
        for dq in dqs:
            qd = sv_quads_newest(n, dq, L2, inside, ab[dq][0] ** 2, ab[dq][1] ** 2)
            if len(qd):
                pending[dq].append(qd)
                npend += len(qd)
        if npend < target and n < field.nq - 1:
            continue
        if npend == 0:
            continue
        clock.check()
        sol = _sv_flush(field, catalog, info, pending, s_lo, s_hi, hint_t, clock, log, p, st, n)
        pending = {dq: [] for dq in dqs}
        npend = 0
        target = min(SV_MAX_BATCH, target * 2)
        if sol is not None:
            sol.seconds = time.monotonic() - t0
            if log:
                log(f"解けました: {sol.index_name}、対数オッズ {sol.logodds:.1f}、一致 {sol.nmatch} 個、"
                    f"quad {sol.quads_tried:,} 個・仮説 {sol.hypotheses:,} 個、{sol.seconds:.2f} 秒")
            return sol
    if log:
        log(f"quad を全部試しても解けませんでした(quad {st['quads']:,} 個・仮説 {st['hyps']:,} 個、"
            f"対数オッズの最大 {st.get('logodds_max', -math.inf):.1f}、{time.monotonic() - t0:.1f} 秒)")
    return None


def _sv_flush(field, catalog, info, pending, s_lo, s_hi, hint_t, clock, log, p, st, newest):
    """たまった quad の束を、選んだ索引に順に照合して検証する。解けたら SvSolution。

    照合は索引 1 本ずつ、検証は系列ごと(5200 系のタイルの仮説をまとめて。参照星も系列の全タイルから
    集めるので)に行う。系列の順は info の順(catalog.select の順)。
    """
    batches = {}
    nq = 0
    for dq, lst in pending.items():
        if not lst:
            continue
        quads = np.concatenate(lst)
        nq += len(quads)
        codes = sv_field_codes(field.P, quads)
        q, row, perm, mir = sv_code_variants(codes, dq, p.codetol, p.parity)
        ab = np.sqrt(((field.P[quads[:, 1]] - field.P[quads[:, 0]]) ** 2).sum(axis=1))
        batches[dq] = dict(quads=quads, ab=ab, q=q, row=row, perm=perm, mirror=mir)
    st["quads"] += nq
    fl = dict(newest=int(newest), quads=nq, hits=0, hyps=0, t=0.0)
    st["flushes"].append(fl)
    groups = []
    for f, fid, _, _ in info:
        if groups and groups[-1][0] == f.indexid:
            groups[-1][1].append((f, fid))
        else:
            groups.append((f.indexid, [(f, fid)]))
    bad_series = st.setdefault("bad_series", set())
    for iid, members in groups:
        if iid in bad_series:
            continue
        while True:
            # 中身の壊れた索引(SvIndexCorrupt)はそのファイルを目録から外して、系列の照合と検証をやり直す。
            # それ以外の読み取りの例外は、どのファイルか分からないので系列ごとこの探索から外す
            # (例外が sv_solve の外へ出ると、ほかの索引で解ける画像まで解けなくなるため)
            try:
                sol = _sv_flush_series(field, catalog, members, batches, s_lo, s_hi, hint_t, clock, log, p,
                                       st, fl)
            except SvIndexCorrupt as e:
                if e.file.broken is not None:          # 外したはずのファイルがまた出た(起きないはず)
                    bad_series.add(iid)
                    sol = None
                    break
                catalog.mark_broken(e.file, str(e))
                st.setdefault("broken", []).append(str(e))
                if log:
                    log(f"星図データが壊れているので、この先は使いません: {e}")
                members = [(f, fid) for f, fid in members if f.broken is None]
                continue
            except (ValueError, IndexError) as e:
                bad_series.add(iid)
                st.setdefault("broken", []).append(f"index-{iid}: {type(e).__name__}: {e}")
                if log:
                    log(f"星図データ index-{iid} を読めないので、この探索では使いません"
                        f"(中身が壊れているかもしれません。落とし直してください): {type(e).__name__}: {e}")
                sol = None
            break
        if sol is not None:
            fl["t"] = round(time.monotonic() - st["t0"], 4)
            return sol
    fl["t"] = round(time.monotonic() - st["t0"], 4)
    return None


def _sv_flush_series(field, catalog, members, batches, s_lo, s_hi, hint_t, clock, log, p, st, fl):
    """系列 1 つぶん: 索引ごとに照合し、仮説をまとめて検証する。解けたら SvSolution。"""
    hys = []
    for f, fid in members:
        if f.broken is not None:
            continue
        b = batches.get(f.dimquads)
        if b is None:
            continue
        clock.check()
        hy, nhit = _sv_match_file(field, f, fid, b, s_lo, s_hi, hint_t, clock)
        st["hits"] += nhit
        fl["hits"] += nhit
        if hy is not None:
            hys.append(hy)
    if not hys:
        return None
    hy = _sv_hyp_concat(hys)
    return _sv_verify_and_pick(field, catalog, members[0][0], hy, clock, log, p, st, fl)


def _sv_hyp_concat(hys):
    """仮説の束をつなぐ(同じ系列のタイルの束。newpoint の順 = quad の行の順に並べ直す)。"""
    if len(hys) == 1:
        return hys[0]
    out = {}
    for k, v in hys[0].items():
        if isinstance(v, np.ndarray) and v.ndim >= 1 and len(v) == hys[0]["n"]:
            out[k] = np.concatenate([h[k] for h in hys])
        else:
            out[k] = v
    out["n"] = sum(h["n"] for h in hys)
    order = np.argsort(out["row"], kind="stable")
    return _sv_hyp_take(out, order)


def _sv_verify_and_pick(field, catalog, f, hy, clock, log, p, st, fl):
    """仮説の束を newpoint の順に SV_VERIFY_STEP 個ずつ検証し、solve 以上の最初の仮説(なければ tune-up で
    上がった最初のもの)を改良して返す。区切りごとに選ぶので、解が見つかればそこで止まる(後ろの仮説は検証しない)。
    区切りは順に進むので、選ばれる仮説は束全体を検証してから選んだときと同じ。
    改良した後の対数オッズが sv_accept_logodds(試した数で上げた値)に届かないもの、試した仮説が多くなってから
    tune-up で上がったのに改良で一致が増えないもの(sv_tuneup_accepted)は解にしない(設計書 3.7 節の 6)。"""
    n = hy["n"]
    fs = catalog.series[f.indexid]
    solve = float(p.logodds_solve)
    tune = float(p.logodds_tune)
    for s in range(0, n, SV_VERIFY_STEP):
        part = _sv_hyp_take(hy, slice(s, min(n, s + SV_VERIFY_STEP)))
        m = part["n"]
        res = _sv_verify_hyps(field, catalog, f, part, clock=clock, quick=True)
        best, nmatch, quick = res["best"], res["nmatch"], res["quick_rejected"]
        st["hyps"] += m
        fl["hyps"] += m
        st["quick_rejected"] = st.get("quick_rejected", 0) + int(quick.sum())
        if m and float(best.max()) > st.get("logodds_max", -math.inf):
            st["logodds_max"] = float(best.max())           # 改良する前の、検証した仮説の対数オッズの最大
        room = SV_TRACE_MAX_HYP - len(st["hyp_log"]) if st.get("trace") else 0
        for i in range(min(m, max(0, room))):
            ra, dec = sv_xyz_to_radec(part["crval"][i])
            st["hyp_log"].append(dict(index=fs[int(part["fid"][i])].name, row=int(part["row"][i]),
                                      mirror=bool(part["mirror"][i]),
                                      quad_field=part["qfield"][i].tolist(), quad_index=part["qindex"][i].tolist(),
                                      crval=[float(ra), float(dec)], crpix=part["crpix"][i].tolist(),
                                      cd=part["cd"][i].tolist(), scale=round(float(part["scale"][i]), 6),
                                      quick_rejected=bool(quick[i]),
                                      logodds=round(float(best[i]), 4) if np.isfinite(best[i]) else None,
                                      nmatch=int(nmatch[i])))
        # 解とみなす値は、この区切りの終わりまでに試した仮説の数で上げる(区切りの中は同じ値)
        accept = sv_accept_logodds(p, st["hyps"])
        for i in np.nonzero(best >= tune)[0]:
            hy1 = _sv_hyp_take(part, slice(i, i + 1))
            tuned = bool(best[i] < solve)
            if tuned:
                # tune-up: 1 次で改良して検証し直す
                _, r1 = _sv_refine(field, catalog, f, hy1, 1, clock=clock, final=False)
                st.setdefault("tuneups", []).append(dict(index=fs[int(part["fid"][i])].name,
                                                         before=round(float(best[i]), 4),
                                                         after=round(float(r1["best"][0]), 4)))
                if not (r1["best"][0] >= solve):
                    continue
            try:
                wcs, res1 = _sv_refine(field, catalog, f, hy1, p.tweak_order, clock=clock)
            except _SvTimeUp:
                # solve を超えた仮説があるので、締め切りを過ぎても改良は最後まで済ませる(数十 ms。中止の合図はそのまま見る)。
                # ここで None を返すと、解けている画像が「解けず」になる。解として返すかは、改良の後の判定(試した数で上げる値・
                # tune-up の一致の増え)で決まるので、ログは「解が見つかった」と言い切らない
                if log:
                    log("締め切りを過ぎましたが、solve を超えた候補があるので改良を済ませます")
                wcs, res1 = _sv_refine(field, catalog, f, hy1, p.tweak_order,
                                       clock=_SvClock(None, clock.cancel))
            lo = float(res1["best"][0])
            if not (lo >= accept):
                continue
            # 試した仮説が多くなってから tune-up で上がった仮説は、改良で一致が増えたときだけ解にする
            # (同じ星への当て直しだけで上がったものは偶然)
            if tuned and not sv_tuneup_accepted(p, st["hyps"], int(nmatch[i]), int(res1["nmatch"][0])):
                continue
            return SvSolution(
                wcs=wcs, logodds=lo, nmatch=int(res1["nmatch"][0]), ndistract=int(res1["ndistract"][0]),
                nfield=int(res1["NT"][0]), nref=int(res1["NR"][0]), index_name=fs[int(hy1["fid"][0])].name,
                quad_field=hy1["qfield"][0].copy(), quad_index=hy1["qindex"][0].copy(),
                quads_tried=int(st["quads"]), hypotheses=int(st["hyps"]), seconds=0.0,
                matches=[(mm[0], mm[1], mm[2]) for mm in res1["matches"][0]])
        clock.check()
    return None


def _sv_write_trace(path, st, p, sol, status, width, height, scale_lo, scale_hi, hint, xy, flux):
    """途中の値を JSON に書く(第 2 段の移植先で同じ値が出るかを確かめるため。設計書 12 節)。

    入力の星(xy・flux)、照合した quad の数・当たりの数、束ごとの経過、検証した仮説(先頭 SV_TRACE_MAX_HYP 個:
    quad の星・4 点で当てはめた WCS・足切りで落ちたか・対数オッズ・一致数)、tune-up、解(WCS・一致)。
    """
    xy = np.asarray(xy, dtype=np.float64).reshape(-1, 2)
    out = dict(status=status, width=int(width), height=int(height), nstars=int(len(xy)),
               xy=xy.tolist(), flux=None if flux is None else np.asarray(flux, dtype=np.float64).tolist(),
               scale_given=[scale_lo, scale_hi], scale_used=list(st.get("scale", ())),
               hint=None if hint is None else [float(v) for v in hint],
               params=p.as_dict(), files=st.get("files", []),
               quads_tried=int(st["quads"]), hits=int(st["hits"]), hypotheses=int(st["hyps"]),
               quick_rejected=int(st.get("quick_rejected", 0)),
               logodds_max=st.get("logodds_max") if math.isfinite(st.get("logodds_max", -math.inf)) else None,
               seconds=round(time.monotonic() - st["t0"], 4), flushes=st["flushes"],
               tuneups=st.get("tuneups", []), hypotheses_logged=len(st["hyp_log"]),
               hypotheses_log=st["hyp_log"])
    if sol is not None:
        out["solution"] = dict(index=sol.index_name, logodds=sol.logodds, nmatch=sol.nmatch,
                               ndistract=sol.ndistract, nfield=sol.nfield, nref=sol.nref,
                               quad_field=np.asarray(sol.quad_field).tolist(),
                               quad_index=np.asarray(sol.quad_index).tolist(),
                               wcs=sol.wcs.to_dict(),
                               matches=[[int(a), np.asarray(b).tolist(), float(c)] for a, b, c in sol.matches])
    try:
        with open(path, "w", encoding="utf-8") as fh:
            json.dump(out, fh, ensure_ascii=False, separators=(",", ":"))   # 字下げなし(大きいので速さ優先)
    except OSError:
        pass

def _log(msg=""):
    print(msg, flush=True)


class SolveInput:
    """解析エンジンに渡す準備が整った画像。"""

    def __init__(self, data, header, note, display_data=None, flip_y=False):
        self.data = data                 # 2次元 float32 (星検出用)
        self.header = header if header is not None else fits.Header()
        self.note = note                 # 読み込み方法の説明
        # 表示用。検出用と同じ幾何であること (同じ WCS を当てるため)
        self.display = display_data if display_data is not None else data
        # 表示するとき上下を反転するか (行 0 が画面の上に来る画像なら True)
        self.flip_y = flip_y

    @property
    def shape(self):
        return self.data.shape


# ============================================================== 画像読み込み ===

_COLOR_WEIGHTS = np.array([0.2126, 0.7152, 0.0722], dtype=np.float32)
_FITS_EXT = (".fits", ".fit", ".fts", ".fz", ".fits.gz", ".fit.gz")


def _planes_to_mono(cube, axis):
    """RGB(A) の面を輝度 1 面にまとめる。"""
    cube = np.moveaxis(np.asarray(cube), axis, 0)[:3]
    n = cube.shape[0]
    w = _COLOR_WEIGHTS[:n] / _COLOR_WEIGHTS[:n].sum()
    return np.tensordot(w, cube.astype(np.float32), axes=(0, 0))


def _reduce_to_2d(data, max_frames=200):
    """3 次元以上を 2 次元へ。戻り値 (2次元 float32, 説明)"""
    data = np.asarray(data)
    while data.ndim > 3 and 1 in data.shape[:-2]:
        data = np.squeeze(data, axis=int(np.argmax([s == 1 for s in data.shape[:-2]])))
    if data.ndim > 3:
        data = data[(0,) * (data.ndim - 3)]
    if data.ndim == 2:
        return np.array(data, dtype=np.float32), ""
    if data.ndim == 3:
        nz, ny, nx = data.shape
        if nz in (3, 4) and ny > 8 and nx > 8:
            return _planes_to_mono(data, 0), "カラー 3 面を輝度に合成"
        if nx in (3, 4) and ny > 8 and nz > 8:
            return _planes_to_mono(data, 2), "カラー 3 面を輝度に合成"
        use = int(min(nz, max_frames))
        acc = np.zeros((ny, nx), dtype=np.float64)
        for i in range(use):                  # 巨大キューブでも一度に載せない
            acc += np.asarray(data[i], dtype=np.float64)
        return (acc / use).astype(np.float32), \
               f"{nz} 枚のキューブのうち先頭 {use} 枚を平均 (SNR 稼ぎ)"
    raise ValueError(f"2 次元画像に変換できない形状です: {data.shape}")


def _image_hdus(hdul):
    for hdu in hdul:
        if isinstance(hdu, (fits.PrimaryHDU, fits.ImageHDU, fits.CompImageHDU)):
            naxis = int(hdu.header.get("NAXIS", 0) or 0)
            if naxis >= 2:
                yield hdu


def _pick_fits_hdu(hdul):
    """一番大きい画像 HDU を選ぶ (拡張 HDU・圧縮 HDU も対象)。"""
    best, best_size = None, -1
    for hdu in _image_hdus(hdul):
        h = hdu.header
        size = 1
        for i in range(1, int(h.get("NAXIS", 0)) + 1):
            size *= int(h.get(f"NAXIS{i}", 1))
        if size > best_size:
            best, best_size = hdu, size
    if best is not None:
        return best
    # ヘッダが壊れていて HDU の種別を判別できないファイル向けの保険
    for hdu in hdul:
        try:
            arr = hdu.data
        except Exception:
            continue
        if arr is not None and getattr(arr, "dtype", None) is not None \
                and arr.dtype.fields is None and np.ndim(arr) >= 2:
            return hdu
    return None


def _bayer_pattern(header):
    pat = header.get("BAYERPAT") or header.get("COLORTYP") or header.get("CFAIMAGE")
    if pat is None and ("XBAYROFF" in header or "BAYOFFX" in header):
        pat = "RGGB"
    if pat is None:
        return None
    pat = str(pat).strip().upper()
    return pat if pat in ("RGGB", "BGGR", "GRBG", "GBRG") else None


def _load_fits(path):
    """戻り値 (2次元配列, header, notes, flip_y)"""
    notes = []
    # memmap は巨大キューブで効くが、BZERO/BSCALE 付き (= 一般的な 16bit FITS)
    # では astropy が拒否するので、そのときだけ読み込みに切り替える。
    for use_memmap in (True, False):
        notes = []
        try:
            with fits.open(path, memmap=use_memmap,
                           ignore_missing_simple=True) as hdul:
                hdu = _pick_fits_hdu(hdul)
                if hdu is None:
                    raise ValueError("画像データを含む HDU が FITS の中にありません。")
                header = hdu.header.copy()
                idx = list(hdul).index(hdu)
                raw = hdu.data
                notes.append(f"FITS HDU[{idx}] {tuple(np.shape(raw))} "
                             f"{np.asarray(raw).dtype}")
                data, note = _reduce_to_2d(raw)      # ここで必ずコピーが作られる
                if note:
                    notes.append(note)
                blank = header.get("BLANK")
                if blank is not None:
                    data[data == float(blank)] = np.nan
            # SharpCap / FireCapture は 1 行目が画面の上
            flip = str(header.get("ROWORDER", "")).upper().startswith("TOP")
            return data, header, notes, flip
        except ValueError as e:
            if use_memmap and "memory-mapped" in str(e):
                continue
            raise
    raise ValueError("FITS を読めませんでした。")


def _load_pil(path):
    from PIL import Image, ImageOps
    notes = []
    with Image.open(path) as im:
        try:
            im = ImageOps.exif_transpose(im)
        except Exception:
            pass
        notes.append(f"{im.format} {im.size} mode={im.mode}")
        if im.mode in ("I;16", "I;16B", "I;16L", "I", "F"):
            arr = np.asarray(im)
        elif im.mode in ("RGB", "RGBA", "P", "YCbCr", "CMYK", "LA"):
            arr = np.asarray(im.convert("RGB"))
            notes.append("カラー 3 面を輝度に合成")
        else:
            arr = np.asarray(im.convert("F"))
    data, note = _reduce_to_2d(arr)
    if note and note not in notes:
        notes.append(note)
    return data, fits.Header(), notes, True     # 画像ファイルは 1 行目が上


def looks_inverted(data):
    """
    白地に黒い星 (星図・ネガ) かどうか。

    見るのは「背景 (中央値) が、明るさの幅のどのあたりに居るか」。
      ふつうの天体写真 … 背景は下のほう。上へ大きく伸びる (星・月)
      星図            … 背景がいちばん明るい。下へ大きく伸びる (星・文字)

    裾の「太さ」(パーセンタイル) では測れない。星図に描かれた星は
    画素数でいえばごく一部なので、0.5 パーセンタイルでも背景のままになる
    (合成した星図で実際に見落とした)。端の値との距離で測る。

    逆に、周辺減光やケラレで隅が真っ黒な写真は「下へ長い裾」を持つが、
    背景の上にはもっと長い裾 (星) があるので取り違えない
    (これも実際に取り違えたので、比を 5% と厳しくしてある)。
    """
    v = data[np.isfinite(data)]
    if v.size == 0:
        return False
    med = float(np.median(v))
    up = float(v.max()) - med
    down = med - float(v.min())
    return down > 0 and up <= down * 0.05


def load_image_any(path):
    """FITS / PNG / JPEG / TIFF / キューブ を読み、2 次元 float32 にして返す。"""
    lower = str(path).lower()
    order = (_load_fits, _load_pil) if lower.endswith(_FITS_EXT) \
        else (_load_pil, _load_fits)

    first_error = None
    for loader in order:
        try:
            data, header, notes, flip_y = loader(path)
            break
        except Exception as e:                # 拡張子と中身が食い違うファイル対策
            if first_error is None:
                first_error = e
    else:
        raise ValueError(f"画像として読み込めませんでした: {first_error}")

    data = np.array(data, dtype=np.float32, copy=True)
    if data.ndim != 2:
        raise ValueError(f"2 次元画像になりませんでした: shape={data.shape}")
    if min(data.shape) < 16:
        raise ValueError(f"画像が小さすぎて解析できません: {data.shape}")

    display = data.copy()

    pat = _bayer_pattern(header)
    if pat and ndimage is not None:
        # 2x2 の平均 (v6 まで) は窓が片側に寄るので、星の位置が x・y とも +0.5 px
        # ずれていた。対称な 3x3 の二項カーネル [1,2,1]⊗[1,2,1]/16 ならずれない。
        # どの画素を中心にしても R:G:B = 1:2:1 の重みになるので、2x2 の平均と
        # 同じく市松模様も消える。
        k = np.array([0.25, 0.5, 0.25], dtype=np.float32)
        data = ndimage.correlate1d(data, k, axis=0, mode="nearest")
        data = ndimage.correlate1d(data, k, axis=1, mode="nearest")
        notes.append(f"ベイヤー配列 ({pat}) を 3x3 の二項フィルターで平滑化")

    bad = ~np.isfinite(data)
    if bad.any():
        good = data[~bad]
        data[bad] = float(np.median(good)) if good.size else 0.0
        notes.append(f"非有限値 {int(bad.sum())} 画素を中央値で補完")

    if float(np.ptp(data)) <= 0:
        raise ValueError("画像が一様です (全画素が同じ値)。露出やファイルを確認してください。")

    # 星図 (白地に黒い星) は明暗が逆。星の検出は「明るい点」を
    # 探すので、そのままでは 1 つも拾えない。検出に使う面だけ反転する。
    # 表示は元のまま (白地の星図は白地で見せたほうが自然)。
    if looks_inverted(data):
        data = float(np.nanmax(data)) - data
        notes.append("白地に黒い星 (星図) とみて明暗を反転")

    return SolveInput(data, header, " / ".join(notes), display, flip_y)


# ================================================================== 前処理 ===

def _mask_overlay_bands(data, mask, max_band=0.08):
    """
    SharpCap / FireCapture などが焼き込む日時オーバーレイを検出してマスクする。
    文字は「画面の縁に張り付き」「1 行の中で横に広く散らばり」「飽和に近い」。
    """
    ny, nx = data.shape
    med = float(np.median(data))
    sigma = float(sigma_clipped_stats(data, sigma=3.0)[2]) or 1.0
    hot = data > med + 8.0 * sigma
    if not hot.any():
        return 0

    masked = 0
    band_rows = max(3, int(ny * max_band))
    band_cols = max(3, int(nx * max_band))

    def scan(count_fn, length, span, band, apply_fn):
        nonlocal masked
        for seq in (range(0, band), range(length - 1, length - band - 1, -1)):
            streak = []
            for i in seq:
                idxs = count_fn(i)
                if idxs.size >= 12 and (idxs.max() - idxs.min()) > 0.10 * span:
                    streak.append(i)
                elif streak:
                    break
            if streak:
                lo = max(0, min(streak) - 3)
                hi = min(length - 1, max(streak) + 3)
                apply_fn(lo, hi)
                masked += (hi - lo + 1) * span

    scan(lambda y: np.flatnonzero(hot[y]), ny, nx, band_rows,
         lambda lo, hi: mask.__setitem__((slice(lo, hi + 1), slice(None)), True))
    scan(lambda x: np.flatnonzero(hot[:, x]), nx, ny, band_cols,
         lambda lo, hi: mask.__setitem__((slice(None), slice(lo, hi + 1)), True))
    return int(masked)


def _mask_dead_edges(data, mask, frac=0.02):
    """周辺減光やデベイヤー端で潰れた縁をマスクする。"""
    ny, nx = data.shape
    med = float(np.median(data))
    for y in range(max(1, int(ny * frac))):
        for row in (y, ny - 1 - y):
            line = data[row]
            if float(np.ptp(line)) < 1e-6 or float(np.median(line)) < med * 0.05:
                mask[row, :] = True
    for x in range(max(1, int(nx * frac))):
        for col in (x, nx - 1 - x):
            line = data[:, col]
            if float(np.ptp(line)) < 1e-6 or float(np.median(line)) < med * 0.05:
                mask[:, col] = True


def _background(data, box=64):
    """粗いグリッドの中央値を伸ばして背景面を作る (かぶり・周辺減光対策)。"""
    ny, nx = data.shape
    if ndimage is None:
        return np.full_like(data, float(np.median(data)))
    by, bx = max(1, ny // box), max(1, nx // box)
    if by < 3 or bx < 3:
        return np.full_like(data, float(np.median(data)))
    ys = np.linspace(0, ny, by + 1).astype(int)
    xs = np.linspace(0, nx, bx + 1).astype(int)
    coarse = np.empty((by, bx), dtype=np.float32)
    for i in range(by):
        for j in range(bx):
            tile = data[ys[i]:ys[i + 1], xs[j]:xs[j + 1]]
            coarse[i, j] = np.median(tile) if tile.size else 0.0
    coarse = ndimage.median_filter(coarse, size=3, mode="nearest")
    back = ndimage.zoom(coarse, (ny / by, nx / bx), order=1)
    if back.shape != data.shape:                     # zoom の丸め対策
        out = np.full(data.shape, float(np.median(coarse)), dtype=np.float32)
        h = min(ny, back.shape[0])
        w = min(nx, back.shape[1])
        out[:h, :w] = back[:h, :w]
        back = out
    return back


def _kill_hot_pixels(sub, sigma):
    """
    まわりに何の広がりも持たない 1 画素だけの突出 (ホットピクセル / 宇宙線) を消す。

    「隣 8 画素の合計が中心の 30% 未満」を条件にする。星は FWHM が 1 画素を
    下回らないので、アンダーサンプルの星でも隣接画素が必ずついてくる。
    個数でしきい値を切ると、光跡などで数が増えたときに一切効かなくなるので
    条件そのもので判定する。
    """
    if ndimage is None:
        return sub, 0
    neigh_sum = ndimage.uniform_filter(sub, size=3, mode="nearest") * 9.0 - sub
    hot = (sub > 8.0 * sigma) & (neigh_sum < 0.30 * sub)
    n = int(hot.sum())
    if n == 0 or n > 0.05 * sub.size:
        return sub, 0
    med3 = ndimage.median_filter(sub, size=3, mode="nearest")
    return np.where(hot, med3, sub), n


def preprocess(data, mask_overlay=True):
    """背景を引き、マスクを作る。戻り値 (背景差引画像, mask, ノイズσ, 説明)"""
    ny, nx = data.shape
    mask = np.zeros(data.shape, dtype=bool)
    info = []

    sub = data - _background(data)
    sigma = float(sigma_clipped_stats(sub, sigma=3.0)[2])
    if not np.isfinite(sigma) or sigma <= 0:
        sigma = float(np.std(sub)) or 1.0

    # 焼き込み文字の判定より先に外す。ホットピクセルが行に散っていると
    # 「文字列がある」と誤判定してしまうため。
    sub, n_hot = _kill_hot_pixels(sub, sigma)
    if n_hot:
        info.append(f"ホットピクセル {n_hot} 画素を平滑化")

    _mask_dead_edges(data, mask)
    if mask_overlay:
        n = _mask_overlay_bands(sub + float(np.median(data)), mask)
        if n:
            info.append(f"焼き込みオーバーレイとみられる {n} 画素をマスク")

    ref = sub[~mask] if (~mask).any() else sub
    s2 = float(sigma_clipped_stats(ref, sigma=3.0)[2])
    if np.isfinite(s2) and s2 > 0:
        sigma = s2

    if mask.any():
        info.append(f"マスク合計 {int(mask.sum())} / {ny * nx} 画素")
    return sub, mask, sigma, info


# ================================================================ 星の検出 ===

def _measure(sub, labels, keep):
    """ラベル領域ごとに重心・フラックス・伸びを測る。"""
    out = []
    slices = ndimage.find_objects(labels)
    for lab in keep:
        sl = slices[lab - 1]
        if sl is None:
            continue
        tile = np.asarray(sub[sl], dtype=np.float64)
        m = labels[sl] == lab
        w = np.where(m, tile, 0.0)
        tot = float(w.sum())
        if tot <= 0:
            continue
        yy, xx = np.mgrid[sl[0].start:sl[0].stop, sl[1].start:sl[1].stop]
        cx = float((w * xx).sum() / tot)
        cy = float((w * yy).sum() / tot)
        vx = float((w * (xx - cx) ** 2).sum() / tot)
        vy = float((w * (yy - cy) ** 2).sum() / tot)
        vxy = float((w * (xx - cx) * (yy - cy)).sum() / tot)
        tr, det = vx + vy, vx * vy - vxy ** 2
        root = np.sqrt(max(tr * tr / 4.0 - det, 0.0))
        a2 = max(tr / 2.0 + root, 1e-6)
        b2 = max(tr / 2.0 - root, 1e-6)
        out.append(dict(x=cx, y=cy, flux=tot, peak=float(tile[m].max()),
                        area=int(m.sum()), elong=float(np.sqrt(a2 / b2)),
                        fwhm=float(2.3548 * np.sqrt(max((vx + vy) / 2.0, 1e-6)))))
    return out


def detect_sources(sub, mask, sigma, min_sources=12, max_sources=400,
                   max_elongation=4.0, min_area=3, min_snr=5.0):
    """
    しきい値を下げながら星を探す。少なくとも min_sources 個見つかるまで粘る。
    解析エンジンは 4 個で 1 quad を作るが、実用上は 10 個以上ほしい。
    run() は min_sources=60 で呼ぶ (自前のソルバーは星が多いほど quad を選べる)。

    min_snr を 3.0 に下げると暗い画像で星は増えるが (実測 8 → 15 個)、
    疎な星野では雑音のこぶも同じだけ増える (合成テストで純度 100% → 30%)。
    どの値でも「本物だけ増やす」ことはできなかったので 5.0 のままにしてある。
    これは図に緑の丸で出す星で、確かなものだけを残す。解析エンジンには、暗い星まで
    拾う detect_sources_deep の星を先に渡す (solve_offline)。

    しきい値の段の「σ」は smooth_sigma = sigma / 2 を単位にしているが、σ=1 の
    ガウスでぼかした白色雑音の実際の標準偏差は sigma / (2√π) ≈ 0.28 sigma
    (3I の画像の実測 0.31 sigma)。つまり最後の段 2.0 は実際には約 3.2σ で、
    書いてある値より 1.6〜1.8 倍高い。図の星を変えないために、このままにしてある。
    """
    if ndimage is None:
        raise RuntimeError("scipy が必要です:  pip install scipy")

    work = np.where(mask, 0.0, sub)
    ny, nx = work.shape
    smooth = ndimage.gaussian_filter(work, sigma=1.0)
    smooth_sigma = sigma / 2.0          # 平滑化でノイズが下がる分

    best, used = [], None
    for thr in (12.0, 8.0, 6.0, 5.0, 4.0, 3.0, 2.5, 2.0):
        binary = smooth > thr * smooth_sigma
        n_on = int(binary.sum())
        if n_on == 0:
            continue
        if n_on > 0.25 * binary.size:   # 何かおかしい (雲・月明かり) ので下げない
            break
        labels, n = ndimage.label(binary)
        if n == 0:
            continue
        if n > 20000:                   # ノイズが噴き出している
            break
        sizes = ndimage.sum(binary, labels, range(1, n + 1))
        keep = np.flatnonzero(sizes >= min_area) + 1
        if keep.size == 0:
            continue
        cand = []
        for c in _measure(work, labels, keep):
            # 開口 SNR。ノイズのこぶは面積のわりに光量が無いのでここで落ちる。
            c["snr"] = c["flux"] / (sigma * np.sqrt(max(c["area"], 1)))
            if c["snr"] < min_snr or c["elong"] > max_elongation:
                continue
            if not (2.0 <= c["x"] <= nx - 3 and 2.0 <= c["y"] <= ny - 3):
                continue
            cand.append(c)
        if len(cand) > len(best):
            best, used = cand, thr
        if len(cand) >= min_sources:
            best, used = cand, thr
            break

    best.sort(key=lambda c: -c["flux"])
    return best[:max_sources], used


# ------------------------------------------------ 解析エンジンに渡す星 (深い検出) ---
#
# detect_sources は図に出す星なので、領域の面積と開口 SNR で確かなものだけを残す。
# 星の少ない画像ではそれが数個にとどまり、解析エンジンの検証 (対数オッズ) の
# 証拠が足りなくなる (3I: 星 8 個で対数オッズ 21.6〜21.9、合格は 20.7。この検出の星 15 個で 47.7)。
# 解析エンジンに渡す星は次の考え方で別に拾う (solve-field の image2xy と同じ方針。コードは自前):
#   背景を引く → 星の幅に合わせてガウスでぼかす (整合フィルター) →
#   局所の雑音の u 倍を超える局所の極大を星とする。
# u は「雑音だけの画像で偽の極大が画像全体で 2 個ほど」になる値にする (下の式)。
# 偽の星が多いと検証で不利になるので、数は増やしすぎない。

_DEEP_FALSE_RATE = 0.045        # 偽の極大の数の係数 (白色雑音で数えて合わせた値)


def _block_map(img, valid, box, fn):
    """
    box 画素の升ごとに fn (升の値を最後の軸に並べた配列 → 升ごとの値) を計算し、
    3x3 の中央値でならしてから升の中心を節点に双一次で引き伸ばす (外側は端の値)。
    マスクの画素は NaN にして渡す。升が縦横 3 つ未満なら None。
    """
    ny, nx = img.shape
    by, bx = ny // box, nx // box
    if by < 3 or bx < 3:
        return None
    h, w = by * box, bx * box
    oy, ox = (ny - h) // 2, (nx - w) // 2
    t = np.where(valid, img, np.nan)[oy:oy + h, ox:ox + w].astype(np.float32)
    t = t.reshape(by, box, bx, box).transpose(0, 2, 1, 3).reshape(by, bx, box * box)
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")              # 全部マスクの升 (All-NaN slice)
        coarse = fn(t)
    good = np.isfinite(coarse)
    if not good.any():
        return None
    coarse = np.where(good, coarse, float(np.median(coarse[good])))
    coarse = ndimage.median_filter(coarse, size=3, mode="nearest")
    fy = np.clip((np.arange(ny) - (oy + 0.5 * box - 0.5)) / box, 0, by - 1)
    fx = np.clip((np.arange(nx) - (ox + 0.5 * box - 0.5)) / box, 0, bx - 1)
    iy = np.minimum(fy.astype(int), by - 2)
    wy = (fy - iy)[:, None]
    rows = coarse[iy] * (1 - wy) + coarse[iy + 1] * wy
    ix = np.minimum(fx.astype(int), bx - 2)
    wx = (fx - ix)[None, :]
    return (rows[:, ix] * (1 - wx) + rows[:, ix + 1] * wx).astype(np.float32)


def _median_last(t):
    """最後の軸の中央値 (NaN を除く)。NaN が無ければ速い np.median で済ませる。"""
    if np.isnan(t).any():
        return np.nanmedian(t, axis=-1)
    return np.median(t, axis=-1)


def _mad_sigma_last(t):
    """最後の軸の MAD から出した標準偏差。有効な画素が 1/4 未満の升は NaN。"""
    med = _median_last(t)
    mad = _median_last(np.abs(t - med[..., None]))
    n = np.sum(np.isfinite(t), axis=-1)
    return np.where(n >= t.shape[-1] // 4, 1.4826 * mad, np.nan)


def _clipped_stats(values, limit=500_000):
    """sigma_clipped_stats (3σ) の (中央値, 標準偏差)。多いときは等間隔に間引いて速くする。"""
    values = np.ravel(values)
    if values.size > limit:
        values = values[::int(math.ceil(values.size / limit))]
    _, med, std = sigma_clipped_stats(values, sigma=3.0, maxiters=5)
    return float(med), float(std)


def _sharpness(work, y, x):
    """中心の値 / 上下左右の平均。ベイヤーの補間で十字に広がったホットピクセルは大きい。"""
    n4 = (float(work[y - 1, x]) + float(work[y + 1, x])
          + float(work[y, x - 1]) + float(work[y, x + 1])) / 4.0
    return float(work[y, x]) / n4 if n4 > 0 else math.inf


def _psf_sigma(sm1, work, valid, ns1, sigma, max_stars=40, min_snr=10.0):
    """
    明るい星 (σ=1 でぼかした画像で雑音の min_snr 倍以上) の半値の面積から、
    星像の σ を測る。ぼかした分 (σ=1) は二乗で引く。
    戻り値 (σ [画素] か None, 使った星の数, 鋭さの上限)。
    鋭さ (_sharpness) が明るい星の典型 (下から 1/4) の 2 倍を超える点は
    ホットピクセルとみなして幅の計算に使わない。頂が平ら (飽和) な星も使わない。
    """
    ny, nx = sm1.shape
    mx = ndimage.maximum_filter(sm1, size=7)
    pk = (sm1 == mx) & (sm1 > min_snr * ns1) & valid
    pk[:2] = pk[-2:] = False
    pk[:, :2] = pk[:, -2:] = False
    ys, xs = np.nonzero(pk)
    if len(ys) < 3:
        return None, 0, math.inf
    found = []                                      # (σ, 鋭さ か None)
    # 同じ値の候補は元の順に(安定な並べ替え。同点の順が numpy の実装や CPU で変わらないように)
    for i in np.argsort(-sm1[ys, xs], kind="stable")[:max_stars]:
        y, x = int(ys[i]), int(xs[i])
        v = float(sm1[y, x])
        sharp = _sharpness(work, y, x) if work[y, x] > 10.0 * sigma else None
        for r in (8, 16, 32):                       # 半値の領域が窓に収まるまで広げる
            y0, y1 = max(0, y - r), min(ny, y + r + 1)
            x0, x1 = max(0, x - r), min(nx, x + r + 1)
            t = sm1[y0:y1, x0:x1]
            lab, _ = ndimage.label(t > 0.5 * v)
            k = lab[y - y0, x - x0]
            if k == 0:
                break
            reg = lab == k
            rr, cc = np.nonzero(reg)
            touches = (rr.min() == 0 or cc.min() == 0 or rr.max() == reg.shape[0] - 1
                       or cc.max() == reg.shape[1] - 1)
            if touches:
                if r < 32:
                    continue
                break                               # 65 画素の窓より大きい = 星ではない
            if int((t[reg] > 0.98 * v).sum()) > 4:  # 頂が平ら = 飽和
                break
            fwhm = 2.0 * math.sqrt(float(reg.sum()) / math.pi)
            found.append((math.sqrt(max((fwhm / 2.3548) ** 2 - 1.0, 0.0)), sharp))
            break
    sharps = [s for _, s in found if s is not None]
    limit = max(2.5, 2.0 * float(np.percentile(sharps, 25))) if len(sharps) >= 3 else math.inf
    widths = [w for w, s in found if (s is None or s <= limit) and w > 0.3]
    if len(widths) < 3:
        return None, len(widths), limit
    return float(np.percentile(widths, 40)), len(widths), limit


def _false_peaks(u, area, s):
    """
    雑音だけの画像を σ=s でぼかしたとき、雑音の u 倍を超える局所の極大の数の見込み。
    ガウス場の極大の数の式 (面積 / s² · u · exp(-u²/2) に比例) の係数を、
    白色雑音で数えて合わせた (s = 1〜3、968x548 と 1608x1104 で係数 0.04〜0.045)。
    """
    return _DEEP_FALSE_RATE * area / (s * s) * u * math.exp(-0.5 * u * u)


def _peak_offset(a, b, c):
    """
    3 点 (左, 中央, 右) の極大の位置を -0.5〜0.5 で返す。値が全部正ならガウスに
    ちょうど合う対数の放物線、そうでなければ普通の放物線を当てる。
    """
    if a > 0.0 and b > 0.0 and c > 0.0:
        la, lb, lc = math.log(a), math.log(b), math.log(c)
        den = la - 2.0 * lb + lc
        if den < 0.0:
            return min(max(0.5 * (la - lc) / den, -0.5), 0.5)
    den = a - 2.0 * b + c
    if den < 0.0:
        return min(max(0.5 * (a - c) / den, -0.5), 0.5)
    return 0.0


def detect_sources_deep(sub, mask, sigma, max_sources=400, target_false=2.0, min_u=4.0,
                        info=None):
    """
    深い検出 (解析エンジンに渡す星)。detect_sources より暗い星まで拾う。

      1. σ=1 でぼかした画像の明るい星から、星像の幅 (σ_psf) を測る
      2. 画像を σ = 1.25 σ_psf (測れなければ 1.5、1〜8 に収める) のガウスでぼかし、
         升ごとの中央値で背景を引く。雑音は升ごとの MAD で場所ごとに測る
      3. 局所の極大のうち、その場所の雑音の u 倍を超えるものを候補にする。
         u は _false_peaks が target_false 個 (画像全体) になる値 (min_u 以上)
      4. 明るい順に見て、次のものは捨てる:
         ・もっと明るい星との間の谷が雑音の 3 倍より浅い (同じ星の肩・光条のこぶ)
         ・ぼかした像の曲率で見た伸びが 4 倍を超える (光跡・線)
         ・鋭さが明るい星の典型の 2 倍を超える (ベイヤーで十字に広がったホットピクセル)
         ・まわりの輪 (ぼかした星像の σ の 3 倍の半径) の中央値より雑音の 3 倍以上高くない
           (大きなかぶりの上で、背景の当てはめの残りに雑音が乗っただけの極大。月明かりの
           合成画像で偽の星が 42 → 5 個。前処理の背景は 64 画素の升を双一次で引き伸ばすので、
           急なかぶりでは升の節点に折れ目のこぶが残る)
      5. 位置はぼかした像の 3x3 に (対数の) 放物線を当てて小数まで出す

    戻り値は detect_sources と同じ形 (星のリスト, しきい値 u)。リストは明るい順。
    flux はぼかした像の頂の値から出したガウスの全光量 (並べ替えの目安)、
    snr は頂の値 / その場所の雑音、area は FWHM の円の面積 (目安)。
    info に dict を渡すと、測った値 (s_psf, s, u, 候補の数, 捨てた理由ごとの数, 秒) を入れる。

    以前の版は画像を 2x2 にまとめて detect_sources をかけていたが、効くのは FWHM が
    7 画素以上の星だけだった。ここでは星の幅に合わせてぼかすので、細い星でも太い星でも
    同じように効く。
    """
    if ndimage is None:
        raise RuntimeError("scipy が必要です:  pip install scipy")
    from scipy.spatial import cKDTree

    t_start = time.perf_counter()
    ny, nx = sub.shape
    valid = ~np.asarray(mask, dtype=bool)
    if ny < 16 or nx < 16 or int(valid.sum()) < 256:
        return [], None
    work = np.where(valid, sub, 0.0).astype(np.float32)
    sigma = float(sigma)

    # 1. 星像の幅
    sm1 = ndimage.gaussian_filter(work, 1.0)
    back = _block_map(sm1, valid, 32, _median_last)
    sm1 = sm1 - (back if back is not None else float(np.median(sm1[valid])))
    ns1 = _clipped_stats(sm1[valid])[1]
    if not np.isfinite(ns1) or ns1 <= 0:
        ns1 = 0.28 * sigma
    s_psf, n_psf, sharp_max = _psf_sigma(sm1, work, valid, ns1, sigma)
    del sm1, back
    s = min(max(1.25 * s_psf if s_psf else 1.5, 1.0), 8.0)

    # 2. ぼかす・背景を引く・雑音を測る
    sm = ndimage.gaussian_filter(work, s)
    box = int(max(16, round(8 * s)))
    back = _block_map(sm, valid, box, _median_last)
    sm = sm - (back if back is not None else float(np.median(sm[valid])))
    del back
    # マスクの縁はぼかしでマスクの 0 が混ざって沈むので、ぼかしの幅だけ広げて除く
    bad = (ndimage.binary_dilation(~valid, iterations=int(math.ceil(2 * s)) + 1)
           if not valid.all() else np.zeros(valid.shape, dtype=bool))
    if int((~bad).sum()) < 256:
        return [], None
    med, ns = _clipped_stats(sm[~bad])
    if not np.isfinite(ns) or ns <= 0:
        ns = max(ns1, 1e-12)
    sm -= med
    nmap = _block_map(sm, ~bad, int(max(32, round(16 * s))), _mad_sigma_last)
    if nmap is None:
        nmap = np.full(sm.shape, ns, dtype=np.float32)
    nmap = np.maximum(nmap, 0.7 * ns)              # 星で埋まった升で雑音を低く見積もらない

    # 3. しきい値と候補
    area = float((~bad).sum())
    u = float(min_u)
    while u < 8.0 and _false_peaks(u, area, s) > target_false:
        u += 0.05
    u = round(u, 2)
    mx = ndimage.maximum_filter(sm, size=2 * int(math.ceil(s)) + 1)
    pk = (sm == mx) & (sm > u * nmap) & ~bad
    edge = max(2, int(math.ceil(s)))
    pk[:edge] = False
    pk[-edge:] = False
    pk[:, :edge] = False
    pk[:, -edge:] = False
    ys, xs = np.nonzero(pk)
    del mx, pk
    vals = sm[ys, xs]
    order = np.argsort(-vals, kind="stable")[:max(4000, 10 * max_sources)]
    ys, xs, vals = ys[order], xs[order], vals[order].astype(np.float64)
    n = len(ys)

    # 4. 明るい順に選ぶ
    keep = np.zeros(n, dtype=bool)
    rejected = {"saddle": 0, "elong": 0, "sharp": 0, "ring": 0}
    # 局所の背景を測る輪: ぼかした星像の σ (√(σ_psf² + s²)) の 3 倍の半径。星の光はそこで頂の 1% まで落ちる
    sp = s_psf if s_psf else s / 1.25
    r_ring = 3.0 * math.sqrt(sp * sp + s * s)
    ang = np.linspace(0.0, 2.0 * math.pi, max(16, int(2.0 * math.pi * r_ring)), endpoint=False)
    ring_dx = np.round(r_ring * np.cos(ang)).astype(int)
    ring_dy = np.round(r_ring * np.sin(ang)).astype(int)

    def reach(v):
        # 明るい星の像が雑音に沈むまでの半径の目安 (肩のこぶを探す範囲)
        return min(max(4.0 * s, 2.0 * s * math.sqrt(max(v / (u * ns), 1.0))), 40.0 * s)

    if n:
        pts = np.column_stack([xs, ys]).astype(np.float64)
        tree = cKDTree(pts)
        r_max = reach(vals[0])
        for i in range(n):
            y, x, v = int(ys[i]), int(xs[i]), float(vals[i])
            if work[y, x] > 10.0 * sigma and _sharpness(work, y, x) > sharp_max:
                rejected["sharp"] += 1
                continue
            shoulder = False
            for j in tree.query_ball_point(pts[i], r_max):
                if j >= i or not keep[j]:
                    continue
                d = math.hypot(x - xs[j], y - ys[j])
                if d > reach(vals[j]):
                    continue
                m = int(math.ceil(2 * d)) + 2
                line = ndimage.map_coordinates(
                    sm, [np.linspace(y, ys[j], m), np.linspace(x, xs[j], m)], order=1)
                if v - float(line.min()) < 3.0 * float(nmap[y, x]):
                    shoulder = True
                    break
            if shoulder:
                rejected["saddle"] += 1
                continue
            c = sm[y - 1:y + 2, x - 1:x + 2].astype(np.float64)
            dxx = c[1, 2] - 2.0 * c[1, 1] + c[1, 0]
            dyy = c[2, 1] - 2.0 * c[1, 1] + c[0, 1]
            dxy = (c[2, 2] - c[2, 0] - c[0, 2] + c[0, 0]) / 4.0
            tr, det = dxx + dyy, dxx * dyy - dxy * dxy
            elong = math.inf
            if det > 0 and tr < 0:
                root = math.sqrt(max(tr * tr / 4.0 - det, 0.0))
                l1, l2 = tr / 2.0 - root, tr / 2.0 + root   # どちらも負。l1 が急なほう
                if l2 < 0:
                    elong = math.sqrt(l1 / l2)
            if elong > 4.0:
                rejected["elong"] += 1
                continue
            ry = np.clip(y + ring_dy, 0, ny - 1)
            rx = np.clip(x + ring_dx, 0, nx - 1)
            ring = sm[ry, rx][~bad[ry, rx]]               # マスクの側は沈んでいるので使わない
            if len(ring) >= 8 and v - float(np.median(ring)) < 3.0 * float(nmap[y, x]):
                rejected["ring"] += 1
                continue
            keep[i] = True

    # 5. 位置と明るさ
    fwhm = 2.3548 * sp
    out = []
    for i in np.flatnonzero(keep):
        y, x, v = int(ys[i]), int(xs[i]), float(vals[i])
        c = sm[y - 1:y + 2, x - 1:x + 2].astype(np.float64)
        ox = _peak_offset(c[1, 0], c[1, 1], c[1, 2])
        oy = _peak_offset(c[0, 1], c[1, 1], c[2, 1])
        out.append(dict(x=x + ox, y=y + oy, flux=v * 2.0 * math.pi * (sp * sp + s * s),
                        peak=float(sub[y, x]), area=int(round(math.pi * fwhm * fwhm / 4.0)),
                        elong=1.0, fwhm=fwhm, snr=v / float(nmap[y, x])))
    out.sort(key=lambda c: -c["flux"])
    if info is not None:
        info.update(s_psf=s_psf, n_psf=n_psf, s=s, u=u, noise=ns, candidates=n,
                    kept=len(out), rejected=rejected, sharp_max=sharp_max,
                    seconds=time.perf_counter() - t_start)
    return out[:max_sources], u


# ============================================================ スケール推定 ===

def _first_float(header, keys):
    for k in keys:
        if k in header:
            try:
                v = float(header[k])
            except (TypeError, ValueError):
                continue
            if np.isfinite(v) and v != 0:
                return v, k
    return None, None


def rig_key(header, shape):
    """
    「同じ機材・同じ画像サイズ」を表す鍵。
    画素スケールは焦点距離とセンサーで決まるので、これが同じなら前回の実測値が使える。
    """
    ny, nx = shape
    cam = str(header.get("INSTRUME") or header.get("CAMID")
              or header.get("CAMERA") or "?").strip()
    bx = header.get("XBINNING") or header.get("BINX") or 1
    tel = str(header.get("TELESCOP") or "").strip()
    focal = header.get("FOCALLEN") or ""
    return f"{cam}|{tel}|{nx}x{ny}|bin{bx}|f{focal}"


def remembered_scale(header, shape):
    """前に同じ機材で解いたときの実測スケール [arcsec/px]。無ければ None。"""
    return _cache_load().get("rigs", {}).get(rig_key(header, shape))


def remember_scale(header, shape, scale_arcsec):
    """解けたので、この機材の画素スケールを覚えておく。"""
    if not (scale_arcsec and 0.01 < scale_arcsec < 3600):
        return
    cache = _cache_load()
    cache.setdefault("rigs", {})[rig_key(header, shape)] = float(scale_arcsec)
    _cache_save(cache)


def estimate_pixel_scale(header, focal_length_mm=None, shape=None):
    """
    1 画素の秒角を推定する。戻り値 (arcsec/px or None, 説明)

    上ほど信用できる順:
      1. ユーザーが焦点距離を書いた
      2. この画像自身が持っている WCS
      3. ヘッダの画素スケールキーワード
      4. ヘッダの焦点距離 x 画素サイズ
      5. 前に同じ機材で解いたときの実測値  ← 2 回目以降は設定不要で速くなる

    5 を最後に置いているのは、鏡筒を替えてもカメラが同じなら鍵が一致してしまうため。
    ヘッダが焦点距離を書いてくれているなら、そちらを信じた方が安全。
    """
    if focal_length_mm:
        pix, pk = _first_float(header, ["XPIXSZ", "PIXSIZE1", "XPIXELSZ",
                                        "PIXSZ", "PIXSIZEX"])
        if pix:
            s = 206.264806 * pix / float(focal_length_mm)
            if 0.01 < s < 3600:
                return s, f"{pk}={pix}um と FOCAL_LENGTH_MM={focal_length_mm:g}mm から"

    try:
        w = WCS(header)
        if w.is_celestial:
            from astropy.wcs.utils import proj_plane_pixel_scales
            s = float(np.mean(proj_plane_pixel_scales(w.celestial)) * 3600.0)
            if 0.01 < s < 3600:
                return s, "画像に既にあった WCS から"
    except Exception:
        pass

    v, k = _first_float(header, ["SECPIX", "SECPIX1", "PIXSCALE", "SCALE",
                                 "PLTSCALE", "CDELT1"])
    if v is not None:
        s = abs(v) * (3600.0 if k == "CDELT1" else 1.0)
        if 0.01 < s < 3600:
            return s, f"ヘッダ {k} から"

    pix, pk = _first_float(header, ["XPIXSZ", "PIXSIZE1", "XPIXELSZ",
                                    "PIXSZ", "PIXSIZEX"])
    focal, fk = _first_float(header, ["FOCALLEN", "FOCAL", "FOCALLENGTH"])
    if pix and focal:
        s = 206.264806 * pix / focal
        if 0.01 < s < 3600:
            return s, f"{pk}={pix}um, {fk}={focal:g}mm から"

    if shape is not None:
        s = remembered_scale(header, shape)
        if s:
            return float(s), "前回この機材で解いたときの実測値"

    return None, "手がかりなし (星図データが対応する画角を総当たりします)"


def implied_focal_length(header, scale_arcsec):
    """解けたスケールから焦点距離を逆算する (ヘッダに画素サイズがあるときだけ)。"""
    pix, _ = _first_float(header, ["XPIXSZ", "PIXSIZE1", "XPIXELSZ",
                                   "PIXSZ", "PIXSIZEX"])
    if not pix or not scale_arcsec:
        return None
    return 206.264806 * pix / scale_arcsec


# ============================================================== 星図データ ===

# 星図データ (astrometry.net 形式の index-*.fits) の既定の置き場所。
# GUI は［準備］タブの欄、ノートブックはセル C の INDEX_DIR で変えられる。
DEFAULT_INDEX_DIR = (r"C:\AstrometryData" if os.name == "nt"
                     else "/usr/share/astrometry")

# --- インデックスがカバーする画角 (arcmin)。astrometry.net の付番規則 ---------
_INDEX_SCALES = {
    0: (2, 2.8), 1: (2.8, 4), 2: (4, 5.6), 3: (5.6, 8), 4: (8, 11),
    5: (11, 16), 6: (16, 22), 7: (22, 30), 8: (30, 42), 9: (42, 60),
    10: (60, 85), 11: (85, 120), 12: (120, 170), 13: (170, 240),
    14: (240, 340), 15: (340, 480), 16: (480, 680), 17: (680, 1000),
    18: (1000, 1400), 19: (1400, 2000),
}


def index_coverage(index_names):
    """index-XXXX[-YY].fits の一覧から、扱える画角 (arcmin) の範囲を返す。"""
    lo = hi = None
    for name in index_names:
        core = os.path.basename(str(name))
        if not core.startswith("index-"):
            continue
        num = core[len("index-"):].split("-")[0].split(".")[0]
        if not num.isdigit() or len(num) < 2:
            continue
        s = _INDEX_SCALES.get(int(num[-2:]))
        if not s:
            continue
        lo = s[0] if lo is None else min(lo, s[0])
        hi = s[1] if hi is None else max(hi, s[1])
    return lo, hi


def blind_scale_bounds(width_px, index_names=None):
    """
    画角が全く分からないときに探す画素スケールの範囲 [arcsec/px]。

    手持ちの index が扱える画角から逆算するので、
    「絶対に当たらない範囲」を探索しない = 総当たりでも無駄がない。
    """
    lo_fov, hi_fov = index_coverage(index_names or [])
    if lo_fov is None:
        lo_fov, hi_fov = 2.0, 2000.0        # astrometry.net が配布する全範囲
    # index の quad は画角の 10%〜100% 程度まで使えるので下側に少し余裕を持たせる
    low = lo_fov * 60.0 / float(width_px) * 0.9
    high = hi_fov * 60.0 / float(width_px) * 1.1
    return max(low, 0.01), min(high, 3600.0)


def missing_index_advice(fov_arcmin, index_names):
    """
    この画角を解くにはどの index を足せばよいかを教える。
    戻り値 (足りているか, 案内文のリスト)
    """
    have = set()
    for name in index_names or []:
        core = os.path.basename(str(name))
        if core.startswith("index-"):
            num = core[len("index-"):].split("-")[0].split(".")[0]
            if num.isdigit() and len(num) >= 2:
                have.add(int(num[-2:]))
    need = [k for k, (lo, hi) in _INDEX_SCALES.items() if lo <= fov_arcmin < hi]
    if not need:
        return False, [f"画角 {fov_arcmin:.1f}′ は星図データの範囲 (2′〜2000′) の外なので、"
                       "この画角のままでは解けません。"]
    if need[0] in have:
        return True, []
    lo, hi = _INDEX_SCALES[need[0]]
    series = (5200 if need[0] <= 6 else 4100) + need[0]
    files = f"index-{series}-00〜47.fits" if need[0] <= 6 else f"index-{series}.fits"
    return False, [
        f"画角 {fov_arcmin:.1f}′ に対応する星図データ ({lo:g}′〜{hi:g}′ 段) がありません。",
        f"必要なファイル: {files}",
        _index_get_advice(series),
    ]


def _in_gui():
    """GUI から動いているか。GUI はエンジンの名前空間に CANCEL_EVENT を置く (ノートブックには無い)。"""
    return globals().get("CANCEL_EVENT") is not None


def _index_get_advice(series):
    """足りない段の取り方。GUI は［準備］タブ、ノートブックは配布ページの zip とセル C の INDEX_DIR。"""
    if _in_gui():
        return (f"取得: ［準備］タブの段の一覧で index-{series} に印を付け、"
                "「選んだ段を落とす」を押してください。")
    if series in (5202, 5203, 5204):
        z = f"AstrometryData-index-{series}.zip"
    elif series >= 5205 or 4100 <= series < 5200:
        z = "AstrometryData.zip (基本セット)"
    else:
        return (f"取得: この段は配布ページに zip がありません。原典 "
                f"https://portal.nersc.gov/project/cosmo/temp/dstn/index-5200/LITE/ から "
                f"index-{series}-00〜47.fits を落として、セル C の INDEX_DIR のフォルダーに"
                "入れてください。")
    return (f"取得: 配布ページ (README の 5 章) から {z} を落として展開し、"
            "セル C の INDEX_DIR のフォルダーに index-*.fits が並ぶようにしてください。")


def _index_place_advice():
    """星図データの置き場所の直し方 (GUI とノートブックで違う)。"""
    if _in_gui():
        return "［準備］タブで星図データを落とすか、置き場所を指定し直してください。"
    return ("配布ページ (README の 5 章) の zip を展開するか、"
            "セル C の INDEX_DIR を星図データのある場所に直してください。")


# 星図データの目録 (SvIndexCatalog) を覚えておく。作るときにヘッダーを 1 本ずつ
# 検査するので、置き場所と中身が前回と同じなら解析のたびに作り直さない。
_INDEX_CATALOG = {"key": None, "catalog": None}


def _index_dir_key(index_dir):
    """置き場所の「中身の目印」: フォルダーの更新時刻と、索引ファイルの名前・大きさ・更新時刻。"""
    d = os.path.abspath(index_dir)
    try:
        dir_mtime = os.stat(d).st_mtime_ns
        files = []
        with os.scandir(d) as it:
            for e in it:
                if e.name.startswith("index-") and e.name.endswith(".fits"):
                    s = e.stat()
                    files.append((e.name, s.st_size, s.st_mtime_ns))
    except OSError:
        return None
    return os.path.normcase(d), dir_mtime, tuple(sorted(files))


def index_catalog(index_dir=None):
    """星図データの目録 (SvIndexCatalog) を返す。置き場所と中身が前回と同じなら使い回す。"""
    index_dir = index_dir or DEFAULT_INDEX_DIR
    key = _index_dir_key(index_dir)
    if (key is not None and key == _INDEX_CATALOG["key"]
            and _INDEX_CATALOG["catalog"] is not None):
        return _INDEX_CATALOG["catalog"]
    release_index_catalog()
    catalog = SvIndexCatalog(index_dir)
    _INDEX_CATALOG.update(key=key, catalog=catalog)
    return catalog


def release_index_catalog():
    """
    覚えている目録を手放す (索引ファイルの mmap を閉じる)。

    Windows では mmap 中のファイルを消したり置き換えたりできない。
    GUI は星図データを落とし直す前にこれを呼ぶ。
    """
    catalog = _INDEX_CATALOG["catalog"]
    _INDEX_CATALOG.update(key=None, catalog=None)
    close = getattr(catalog, "close", None)
    if callable(close):
        try:
            close()
        except Exception:
            pass


def _catalog_names(catalog):
    """目録に入っている (検査に通った) 索引ファイルの名前の一覧。"""
    names = set()
    for f in getattr(catalog, "files", None) or []:
        names.add(os.path.basename(str(getattr(f, "path", f))))
    return sorted(names)


def _problem_text(p):
    """目録が読み飛ばしたファイル 1 つぶんの説明。(パス, 理由) でも文字列でも受ける。"""
    if isinstance(p, (tuple, list)) and len(p) >= 2:
        return f"{os.path.basename(str(p[0]))}: {p[1]}"
    return str(p)


def solver_diagnostics(index_dir=None):
    """
    星図データが本当に使えるかを事前に確かめる。置き場所を走査し、ヘッダーを検査する。

    戻り値の形は v6 (WSL 版) と同じ {"ok", "messages", "indexes", "cfg", "paths"}。
    indexes は検査に通ったファイルの名前、cfg は空、paths は置き場所だけ。
    """
    index_dir = index_dir or DEFAULT_INDEX_DIR
    rep = {"ok": False, "messages": [], "indexes": [], "cfg": "", "paths": [index_dir]}
    if not os.path.isdir(index_dir):
        rep["messages"].append(
            f"星図データの置き場所 {index_dir} がありません。" + _index_place_advice())
        return rep
    try:
        catalog = index_catalog(index_dir)
    except Exception as e:
        rep["messages"].append(
            f"星図データを読めませんでした ({type(e).__name__}: {e})。")
        return rep

    rep["indexes"] = _catalog_names(catalog)
    problems = list(getattr(catalog, "problems", None) or [])
    if problems:
        rep["messages"].append(
            f"使えない星図データが {len(problems)} 個あるので読み飛ばします。"
            + ("［準備］タブで落とし直してください。" if _in_gui() else
               "配布ページの zip から入れ直してください。"))
        for p in problems[:5]:
            rep["messages"].append("  " + _problem_text(p))
        if len(problems) > 5:
            rep["messages"].append(f"  ほか {len(problems) - 5} 個")
    if not rep["indexes"]:
        rep["messages"].append(
            f"使える星図データ (index-*.fits) が {index_dir} に 1 つもありません。"
            + _index_place_advice())
        return rep

    rep["ok"] = True
    return rep


# ================================================================ オフライン解析 ===

def _cancel_checker(cancel):
    """中止の合図を「呼ぶと True/False を返す関数」にそろえる (threading.Event でも関数でも受ける)。"""
    if cancel is None:
        return lambda: False
    if callable(getattr(cancel, "is_set", None)):
        return cancel.is_set
    return lambda: bool(cancel())


def _solution_comments(sol):
    """ヘッダーの COMMENT に残す解の素性 (FITS のカードは ASCII だけなので英語で書く)。"""
    def num(attr, fmt):
        v = getattr(sol, attr, None)
        try:
            return fmt.format(v) if v is not None else "?"
        except (TypeError, ValueError):
            return "?"
    return ["Solved by the zahyou native solver (after Lang et al. 2010, AJ 139, 1782).",
            f"index {getattr(sol, 'index_name', '?')}, "
            f"log-odds {num('logodds', '{:.2f}')}, matched {num('nmatch', '{:d}')} stars, "
            f"{num('quads_tried', '{:d}')} quads tried"]


def _solution_header(sol, width, height):
    """
    sv_solve の解 (SvSolution) を astropy で読める FITS ヘッダーにする。

    カードは SvWcs.to_header_cards が作る ([(キー, 値, 注記)]。CRPIX はそこで 1 始まりになる)。
    ソルバー側は astropy を使わない約束なので、fits.Header にするのはこちらの仕事。
    """
    cards = sol.wcs.to_header_cards(int(width), int(height),
                                    comments=_solution_comments(sol))
    h = fits.Header()
    for key, value, comment in cards:
        if key == "COMMENT":
            h.add_comment(str(value))
        elif key == "HISTORY":
            h.add_history(str(value))
        else:
            h[key] = (value, comment or "")
    if not WCS(h).is_celestial:
        raise ValueError("天球座標の WCS になっていません")
    return h


def _solution_note(sol):
    """解の中身を 1 行に (索引・一致した星・対数オッズ・試した quad・時間)。"""
    parts = []
    name = getattr(sol, "index_name", None)
    if name:
        parts.append(f"索引 {name}")
    for attr, fmt in (("nmatch", "一致 {:d} 個"), ("logodds", "対数オッズ {:.1f}"),
                      ("quads_tried", "quad {:,d} 個"), ("seconds", "{:.1f} 秒")):
        v = getattr(sol, attr, None)
        if v is None:
            continue
        try:
            parts.append(fmt.format(int(v) if "d}" in fmt else float(v)))
        except (TypeError, ValueError):
            continue
    return " / ".join(parts)


def solve_offline(bundle, sources, work_dir, timeout=300, focal_length_mm=None,
                  hint_radec=None, hint_radius_deg=5.0, verbose=True,
                  diagnostics=None, index_dir=None, prep=None, cancel=None):
    """
    オフライン (zahyou 自前の解析エンジン sv_solve) で解く。条件を変えながら粘り強く挑む。
    星は深い検出 (detect_sources_deep) の星を先に、sources (図に出す標準の検出の星) を
    予備に使う。戻り値: WCS ヘッダ (CRPIX は 1 始まり) or None。

    例外は投げない (中止したときも None)。全体の締め切りは timeout 秒の壁時計で、
    試行ごとに残り時間の一部を割り当て、その締め切りを sv_solve に渡す。
    v6 (WSL) は試行の打ち切りが --cpulimit 任せで、合計が 45 秒ほどはみ出していた。

      prep   : run() が作った前処理の結果 (sub, mask, sigma)。深い検出に使う
      cancel : 中止の合図 (threading.Event か、呼ぶと True を返す関数)
      work_dir は使わない (v6 の引数の並びを保つために残してある)
    """
    try:
        return _solve_offline(bundle, sources, timeout, focal_length_mm, hint_radec,
                              hint_radius_deg, verbose, index_dir, prep,
                              _cancel_checker(cancel))
    except Exception as e:
        _log(f"\n  ❌ オフライン解析が途中で失敗しました: {type(e).__name__}: {e}")
        return None
    finally:
        # 索引ファイルの mmap を放す (目録とヘッダーの検査結果は覚えたまま。次の解析で開き直す)。
        # 開いたままだと、GUI を開いている間は Explorer やほかのプロセスから星図データを
        # 消したり置き換えたりできない (Windows)
        cat = _INDEX_CATALOG["catalog"]
        if cat is not None:
            try:
                cat.close()
            except Exception:
                pass


def _solve_offline(bundle, sources, timeout, focal_length_mm, hint_radec,
                   hint_radius_deg, verbose, index_dir, prep, stopped):
    t_end = time.monotonic() + float(timeout)
    ny, nx = bundle.shape
    sources = list(sources or [])

    scale, how = estimate_pixel_scale(bundle.header, focal_length_mm, bundle.shape)
    fov_w = nx * scale / 60.0 if scale else None
    fov_h = ny * scale / 60.0 if scale else None
    fov_short = min(fov_w, fov_h) if scale else None
    # 星の並び (quad) は「短いほうの辺」に収まる大きさまでしか作れないので、
    # 必要な index は短辺で決まる。横幅だけ見ていると足りない段を見落とす。
    #   実測: 23.6′ x 13.4′ の画像 (ごく普通の 16:9) で、横幅に合う 22′〜30′ の
    #   段だけでは 120 秒かけても解けず、短辺に合う 11′〜16′ を足したら 10 秒で
    #   解けた。掩蔽観測のように縦を切り詰めるとこの差はさらに開く。
    if scale:
        _log(f"  - 画素スケール {scale:.3f}″/px ({how}) "
             f"→ 画角 {fov_w:.1f}′ x {fov_h:.1f}′ (短辺 {fov_short:.1f}′)")
    else:
        _log(f"  - 画素スケール: {how}")

    catalog = index_catalog(index_dir)
    installed = _catalog_names(catalog)
    if not installed:
        _log("  ❌ 使える星図データがありません。" + _index_place_advice())
        return None
    lo, hi = index_coverage(installed)
    if lo is not None:
        _log(f"  - 手持ちの星図データが対応する画角: {lo:g}′ 〜 {hi:g}′"
             f" (index ファイル {len(installed)} 個)")
        if scale:
            # 判定は短辺で行う (上のコメント参照)
            ok, msgs = missing_index_advice(fov_short, installed)
            for m in msgs:
                _log(("  ⚠️ " if not ok else "  - ") + m)
            if not ok:
                _log(f"     ※ 必要な段は横幅 {fov_w:.1f}′ ではなく"
                     f"短辺 {fov_short:.1f}′ で決まります。")

    hint = None
    if hint_radec is not None:
        hint = (float(hint_radec[0]), float(hint_radec[1]), float(hint_radius_deg))
    narrow = (max(scale * 0.75, 0.01), scale * 1.25) if scale else None
    # 画角が分からないときは「手持ちの index で解ける範囲」を総当たりする。
    # 固定値 (v6.0 は 0.2〜120) だと、長焦点の画像がそもそも範囲外になってしまう。
    wide = blind_scale_bounds(nx, installed)
    if not scale:
        _log(f"  - 探索するスケール: {wide[0]:.3f}〜{wide[1]:.3f}″/px "
             f"(画角 {wide[0] * nx / 60:.1f}′〜{wide[1] * nx / 60:.0f}′ 相当)")

    # 星の組は 2 つ。
    #   深い = 解析エンジンのために暗い星まで拾い直した星 (detect_sources_deep)
    #   標準 = run() が検出して図に緑の丸で出す星 (detect_sources)
    # 深い検出の星を先に渡す。解析エンジンは quad に使わなかった星で検証するので、
    # 星が多いほど対数オッズの余裕が増える。実測 (sv_solve にブラインドで直接):
    #   3I は標準 8 個で 21.6 (合格 20.7 のぎりぎり) → 深い 15 個で 47.7、
    #   c2025a6 は 159 → 317、ほかの実写は同等。標準の星は予備に回す。
    if stopped():
        _log("\n  ■ 中止しました。")
        return None
    deep_stars, dinfo = [], {}
    try:
        if prep is None:
            prep = preprocess(bundle.data)[:3]
        deep_stars, u = detect_sources_deep(*prep, info=dinfo)
    except Exception as e:
        _log(f"\n  ⚠️ 深い検出に失敗したので、標準の検出の星だけで解きます: {type(e).__name__}: {e}")
    if deep_stars:
        s_psf = dinfo.get("s_psf")
        how = (f"星の幅 (FWHM {2.3548 * s_psf:.1f} px) に合わせてぼかし" if s_psf else
               f"星の幅を測れなかったので σ {dinfo.get('s', 1.5):.1f} px でぼかし")
        _log(f"\n  - 深い検出 (解析エンジンに渡す星): {how}、星を {len(deep_stars)} 個検出"
             f" (しきい値 {u:g}σ)")
    elif dinfo:
        _log("\n  - 深い検出: 星が見つかりませんでした。")
    star_sets = [(name, stars) for name, stars in (("深い検出", deep_stars), ("標準の検出", sources))
                 if len(stars) >= 3]
    if not star_sets:
        n_max = max(len(sources), len(deep_stars))
        _log("  ❌ 星が 1 つも無いので解析できません。" if n_max == 0 else
             f"  ❌ 星が {n_max} 個しかないので解析できません (3 個以上要ります)。")
        return None

    # 試行のはしご: 探す範囲の狭い (速い) 条件から順に、条件ごとに 深い → 標準 の星で。
    # (ラベル, 星, ヒント, スケールの範囲, 残り時間の何割まで使うか)
    # sv_solve は quad を試し尽くせば締め切り前に戻るので、狭い条件は両方の星で試しても安い。
    conds = []
    if hint and narrow:
        conds.append(("座標ヒント + スケール既知", hint, narrow, 0.25))
    if hint:
        conds.append(("座標ヒント + スケール全域", hint, wide, 0.25))
    if narrow:
        conds.append(("全天 + スケール既知", None, narrow, 0.25))
    conds.append(("全天 + スケール全域", None, wide, 0.5))
    attempts = [(f"{name}の星 {len(stars)} 個 + {cond}", stars, hint_i, rng, share)
                for cond, hint_i, rng, share in conds for name, stars in star_sets]

    solver_log = (lambda m: _log(f"    · {m}")) if verbose else None
    for i, (label, stars, hint_i, (s_lo, s_hi), share) in enumerate(attempts, 1):
        if stopped():
            _log("\n  ■ 中止しました。")
            return None
        left = t_end - time.monotonic()
        if left < 2.0:
            _log("  ⏱ 制限時間に達したので、これ以上の再試行はしません。")
            break
        t = left if i == len(attempts) else left * share
        _log(f"\n  [{i}/{len(attempts)}] {label}  (最大 {t:.0f} 秒)")
        xy = np.array([[s["x"], s["y"]] for s in stars], dtype=np.float64)
        flux = np.array([s["flux"] for s in stars], dtype=np.float64)
        try:
            sol = sv_solve(xy, flux, nx, ny, catalog, scale_lo=s_lo, scale_hi=s_hi,
                           hint=hint_i, deadline=time.monotonic() + t,
                           cancel=stopped, log=solver_log)
        except SvCancelled:
            _log("  ■ 中止しました。")
            return None
        except Exception as e:
            _log(f"  ⚠️ 解析エンジンでエラー: {type(e).__name__}: {e}")
            continue
        if sol is None:
            continue
        try:
            hdr = _solution_header(sol, nx, ny)
        except Exception as e:
            _log(f"  ⚠️ 解の WCS を読めませんでした: {type(e).__name__}: {e}")
            continue
        note = _solution_note(sol)
        if note:
            _log(f"    {note}")
        _log(f"  ✅ 解析成功 ({label})")
        return hdr

    if stopped():
        _log("\n  ■ 中止しました。")
        return None
    _log("\n  ❌ すべての条件で解けませんでした。心当たりの順に:")
    n_best = max(len(sources), len(deep_stars))
    if n_best < 15:
        _log(f"     ・写っている星が {n_best} 個と少なめです。"
             "露出を伸ばすか、複数フレームを重ねてください。")
    lo_fov, hi_fov = index_coverage(installed)
    if lo_fov is not None:
        _log(f"     ・手持ちの星図データは画角 {lo_fov:g}′〜{hi_fov:g}′ しか解けません。")
        if fov_short:
            ok2, msgs2 = missing_index_advice(fov_short, installed)
            if not ok2:
                _log(f"       この画像は短辺が {fov_short:.1f}′ です"
                     f"(横幅 {fov_w:.1f}′)。星の並びは短いほうの辺で決まるので:")
                for m in msgs2:
                    _log("         " + m)
        else:
            _log("       画角が分かれば、足りない段を名指しできます。"
                 "FOCAL_LENGTH_MM を書いてもう一度試してください。")
    if not scale:
        _log("     ・FOCAL_LENGTH_MM に焦点距離を書くと画角が確定し、"
             "総当たりをやめるので格段に速く・確実になります。")
    _log("     ・ピントや雲、極端な露出過多も疑ってください。")
    return None


# ============================================================ オンライン解析 ===

def internet_available(timeout=4):
    """astrometry.net に本当に届くかを見る (google では意味がない)。"""
    import urllib.request
    for url in ("https://nova.astrometry.net/api/", "https://nova.astrometry.net/"):
        try:
            urllib.request.urlopen(url, timeout=timeout)
            return True
        except Exception:
            continue
    return False


def _with_deadline(fn, timeout, label, cancel=None):
    """
    指定時間で必ず戻る呼び出し。中止の合図 (cancel) が立てば 0.5 秒以内に None で戻る。
    v5 は with ThreadPoolExecutor(...) を使っていたため、タイムアウト後も
    ブロックの出口でワーカーの終了を待ってしまい、実際には打ち切れなかった。

    fn 自身が TimeoutError を投げたとき (astroquery の solve_timeout 切れ) は、こちらの締め切りとは
    区別して知らせる。Python 3.11 以降は concurrent.futures.TimeoutError と組み込みの TimeoutError が
    同じクラスなので、区別しないと「〜秒で終わりませんでした」と誤って出る。
    """
    stopped = _cancel_checker(cancel)
    executor = concurrent.futures.ThreadPoolExecutor(max_workers=1)
    try:
        future = executor.submit(fn)
        t_end = time.monotonic() + float(timeout)
        while True:
            if stopped():
                future.cancel()
                _log(f"  ■ {label}を中止しました。")
                return None
            left = t_end - time.monotonic()
            if left <= 0:
                future.cancel()
                _log(f"⚠️ {label}が {timeout:.0f} 秒で終わりませんでした。")
                return None
            try:
                return future.result(timeout=min(0.5, left))
            except concurrent.futures.TimeoutError as e:
                if future.done():               # fn が投げた TimeoutError
                    why = e.args[0] if e.args else "時間切れ"
                    _log(f"⚠️ {label}: 時間内に解けませんでした ({why})。")
                    return None
    finally:
        # wait=False にしないと、ここで居残りスレッドの終了を待ってしまう
        executor.shutdown(wait=False, cancel_futures=True)


def solve_online(image_path, api_key, sources=None, shape=None, timeout=120,
                 hint_radec=None, hint_radius_deg=5.0, scale_hint=None, cancel=None):
    """
    nova.astrometry.net に投げる。指定時間で必ず戻る。解けなければ None。

    星のリストがあるならそれを送る。画像そのものより桁違いに軽く、
    オフラインとまったく同じ検出結果で解けるので結果もぶれない。
    cancel (threading.Event か、呼ぶと True を返す関数) が立てば 0.5 秒以内に None で戻り、
    画像そのものは送らない (中止を押した人の画像を、押したあとに外へ送らない)。
    nova が「解けない」と空の結果を返したときも None (成功扱いにしてオフラインを飛ばさない)。

    星の位置で解けなかったとき (星が 0 個ならはじめから) は、画像ファイルそのもの (ファイル名も) を
    api/upload で送る。photutils が入っていても同じ (exe と同じ動き。下の _no_source_detector)。

    送るときは必ず「公開しない」(publicly_visible='n') を付ける。付けないと nova は公開 ('y') として
    受け付け、画像のページや「最近送られた画像」の一覧などに、API キーの持ち主のアカウント名で載る
    (astroquery も既定値を自分では付けない)。'n' にしても解き方は変わらない。
    ただし 'n' でも一覧や画像ファイル・API の結果から完全には隠れない可能性がある (サーバーのコードの読み。
    実サイトでは未確認) ので、利用者には「nova に送ったものは第三者に見られることがある」と説明している。
    """
    from astroquery.astrometry_net import AstrometryNet
    stopped = _cancel_checker(cancel)

    ast = AstrometryNet()
    ast.api_key = api_key
    # astroquery 0.4.10 の solve_from_image は、photutils を import できると画像を上げず、
    # CCDData.read (FITS しか読めない) と DAOStarFinder で星を数え直して位置を送る。そのままだと
    # JPEG・PNG・TIFF はそこで例外になり、FITS でも「画像そのものを送ります」が事実と違う (v6 の exe がそうだった)。
    # 内部の旗をこのインスタンスにだけ立てて、いつも画像を上げさせる。force_image_upload=True は 0.4.8 から非推奨で、
    # 引数が消えると「Setting force_image_upload is not allowed」で画像の段が必ず失敗するので使わない。
    # 予告どおり photutils の経路が消えれば、この旗は読まれなくなるだけで画像は送られる。
    ast._no_source_detector = True
    settings = {"publicly_visible": "n"}      # 星の位置を送るときも画像を送るときも
    if hint_radec is not None:
        settings.update(center_ra=float(hint_radec[0]),
                        center_dec=float(hint_radec[1]),
                        radius=float(hint_radius_deg))
    if scale_hint:
        settings.update(scale_units="arcsecperpix", scale_type="ul",
                        scale_lower=float(scale_hint) * 0.75,
                        scale_upper=float(scale_hint) * 1.25)

    if sources and shape is not None:
        ny, nx = shape
        xs = [s["x"] + 1.0 for s in sources]      # xylist は FITS 規約で 1 始まり
        ys = [s["y"] + 1.0 for s in sources]
        _log(f"  - 検出した {len(xs)} 個の星の位置だけを送ります。")
        hdr = _with_deadline(
            lambda: ast.solve_from_source_list(
                xs, ys, int(nx), int(ny),
                solve_timeout=int(timeout), verbose=False, **settings),
            timeout + 15, "オンライン解析", cancel)
        if hdr:
            return hdr
        if stopped():
            return None
        _log("  - 星の位置だけでは解けなかったので、画像そのものを送ります。")

    if stopped():
        return None
    hdr = _with_deadline(
        lambda: ast.solve_from_image(image_path, solve_timeout=int(timeout),
                                     verbose=False, **settings),
        timeout + 30, "オンライン解析", cancel)
    return hdr if hdr else None


# ============================================================== 目標の座標 ===

def _cache_path():
    """
    目標の座標と機材ごとの画素スケールを覚えておくファイル。

    以前は Temp に置いていたが、そこは Windows の「ディスクの クリーンアップ」や
    ストレージセンサーが消しにいく場所だった。覚えた座標は
    「一度オンラインで引いた天体名を、山の中でもそのまま使う」ための命綱なので、
    消えない場所 (%LOCALAPPDATA%\\zahyou) へ移した。
    前の場所にファイルが残っていれば、一度だけ引き継ぐ。
    """
    override = os.environ.get("ZAHYOU_CACHE")
    if override:
        return override
    base = os.environ.get("LOCALAPPDATA") or os.path.expanduser("~")
    old_path = os.path.join(tempfile.gettempdir(), "zahyou_cache.json")
    try:
        d = os.path.join(base, "zahyou")
        os.makedirs(d, exist_ok=True)
    except OSError:
        return old_path
    new_path = os.path.join(d, "cache.json")
    if not os.path.exists(new_path) and os.path.exists(old_path):
        try:
            shutil.copyfile(old_path, new_path)
        except OSError:
            return old_path
    return new_path


def _cache_load():
    try:
        with open(_cache_path(), encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return {}


def _cache_save(cache):
    try:
        with open(_cache_path(), "w", encoding="utf-8") as f:
            json.dump(cache, f, ensure_ascii=False, indent=1)
    except Exception:
        pass


def resolve_target_name(name, online=True):
    """
    天体名 / カタログ名 → (RA, Dec) [度]。
    一度引いた名前はキャッシュするので、次からはオフラインでも使える。
    """
    from astropy.coordinates import SkyCoord
    import astropy.units as u

    key = " ".join(str(name).split()).upper()
    cache = _cache_load()
    targets = cache.setdefault("targets", {})

    if not online:
        hit = targets.get(key)
        if hit:
            _log(f"  (オフライン) 以前調べた座標を再利用します: {name}")
            return hit["ra"], hit["dec"]
        _log(f"  ❌ オフラインでは '{name}' を座標に変換できません。"
             " INPUT_MODE='COORDS' で赤経赤緯を直接入力してください。")
        return None, None

    _log(f"\n'{name}' の座標を調べています...")
    try:
        base = SkyCoord.from_name(name)
    except Exception as e:
        _log(f"  ❌ 名前を解決できませんでした: {e}")
        hit = targets.get(key)
        if hit:
            _log("  → 以前調べた座標を使います。")
            return hit["ra"], hit["dec"]
        return None, None
    _log(f"  - 名前解決 (SIMBAD/Sesame): {base.to_string('hmsdms', precision=3)}")

    ra, dec = base.ra.deg, base.dec.deg
    try:
        from astroquery.vizier import Vizier
        v = Vizier(catalog="I/355/gaiadr3",
                   columns=["RA_ICRS", "DE_ICRS", "Gmag"])
        v.ROW_LIMIT = 50
        res = v.query_region(base, radius=5 * u.arcsec)
        if res and len(res[0]):
            t = res[0]
            cand = SkyCoord(np.asarray(t["RA_ICRS"], dtype=float),
                            np.asarray(t["DE_ICRS"], dtype=float), unit="deg")
            # v5 は先頭行を無条件に採用していた。5″以内に複数あると別の星を掴む。
            i = int(np.argmin(base.separation(cand).arcsec))
            ra, dec = float(t["RA_ICRS"][i]), float(t["DE_ICRS"][i])
            extra = ""
            if "Gmag" in t.colnames:
                try:
                    extra = f" (G={float(t['Gmag'][i]):.2f}, 候補 {len(t)} 個中で最も近い星)"
                except Exception:
                    pass
            _log(f"  - Gaia DR3 で精密化: RA={ra:.6f}° Dec={dec:.6f}°{extra}")
    except Exception as e:
        _log(f"  - Gaia DR3 の照合はできませんでした ({e})。SIMBAD の座標を使います。")

    targets[key] = {"ra": ra, "dec": dec, "name": str(name)}
    _cache_save(cache)
    return ra, dec

def normalize_wcs_header(header, shape):
    """
    解析が返す WCS ヘッダーは画像データを持たないので、そのまま WCS() に渡すと
    「軸数が合わない」と警告が出る。画像に合わせて整えたコピーを返す。
    """
    h = header.copy()
    h["SIMPLE"] = True
    h["NAXIS"] = 2
    h["NAXIS1"] = int(shape[1])
    h["NAXIS2"] = int(shape[0])
    # 観測日時由来のキーワードが中途半端に残っていると FITSFixedWarning が出る
    for key in ("MJD-OBS", "DATE-OBS", "MJD-AVG", "MJD-END"):
        h.pop(key, None)
    # astrometry.net (オンライン解析) は EQUINOX しか書かないので astropy は FK5 と
    # 解釈するが、インデックスの元カタログ (Tycho-2 / Gaia) は ICRS。明示しておく。
    h.pop("EQUINOX", None)
    h.pop("RADECSYS", None)
    h["RADESYS"] = "ICRS"
    return h


def as_icrs(coord):
    """フレームが違う SkyCoord 同士を安全に比べられるよう ICRS に揃える。"""
    try:
        return coord.icrs
    except Exception:
        return coord


def wcs_summary(wcs, shape, flip_y=None):
    """
    解けた WCS の要点をまとめる。

    flip_y は「表示で上下を反転するか」(SolveInput.flip_y。図 1 も同じ向きで描く)。
    渡すと、位置角とパリティを**画面の見た目**で出す:
      画面の上   … flip_y なら配列の -y 方向 (行 0 が画面の上に来る画像)
      パリティ   … 画面 (x 右・y 上) で「北が上なら東が左」= 空を見上げたままの
                    向きを「正 (鏡像なし)」とする。画面での det(CD) < 0 がそれ
    None (既定) のときは以前 (v6) の動き (配列の +y を上、det(CD) > 0 を「正」) のまま。
    この動きは、flip_y=True の画像 (PNG・JPEG・上から下へ並んだ FITS) では位置角が 180° ずれ
    (パリティは正しい)、下から上へ並んだ FITS (flip_y=False) では位置角は正しくパリティが逆に出ていた。
    v6 の記録の位置角を直すなら、直すのは flip_y=True の画像のほう (180° を足し引きする)。
    """
    ny, nx = shape
    scales = proj_plane_pixel_scales(wcs.celestial) * 3600.0
    scale = float(np.mean(scales))
    cx, cy = (nx - 1) / 2.0, (ny - 1) / 2.0
    center = as_icrs(wcs.pixel_to_world(cx, cy))
    # 画面の上向きが空のどちらを向いているか = 位置角
    up = as_icrs(wcs.pixel_to_world(cx, cy + (-10 if flip_y else 10)))
    pa = center.position_angle(up).to(u.deg).value
    det = float(np.linalg.det(wcs.pixel_scale_matrix))
    if flip_y is None:
        mirrored = not det > 0
    else:
        mirrored = (-det if flip_y else det) > 0
    parity = "負 (鏡像)" if mirrored else "正 (鏡像なし)"
    return dict(center=center, scale=scale, pa=pa, parity=parity,
                fov_x=nx * scale / 60.0, fov_y=ny * scale / 60.0)


def print_solution(wcs, shape, flip_y=None):
    """解析結果を出す。flip_y は wcs_summary と同じ (渡すと画面の見た目で出す)。"""
    s = wcs_summary(wcs, shape, flip_y=flip_y)
    _log("\n" + "=" * 62)
    _log("  解析結果")
    _log("=" * 62)
    _log(f"  画像中心      : {s['center'].to_string('hmsdms', precision=2)}")
    _log(f"                  (RA {s['center'].ra.deg:.6f}°, Dec {s['center'].dec.deg:.6f}°)")
    _log(f"  画素スケール  : {s['scale']:.4f} 秒角/px")
    _log(f"  視野          : {s['fov_x']:.2f}′ x {s['fov_y']:.2f}′")
    _log(f"  画面上方向    : 位置角 {s['pa']:.2f}°(北から東回り)")
    _log(f"  パリティ      : {s['parity']}")
    return s


def report_offsets(center, target):
    """画像中心 → 目標 のずれを、赤経赤緯の軸ごとに出す。"""
    center, target = as_icrs(center), as_icrs(target)
    sep = center.separation(target)
    d_ra, d_dec = center.spherical_offsets_to(target)
    ra_min = d_ra.to(u.arcmin).value
    dec_min = d_dec.to(u.arcmin).value
    ra_dir = "東" if ra_min > 0 else "西"
    dec_dir = "北" if dec_min > 0 else "南"
    _log("\n  画像中心から目標までのずれ")
    _log(f"    全角距離      : {sep.arcmin:.3f}′  ({sep.arcsec:.1f}″)")
    _log(f"    赤経(RA) 方向 : {abs(ra_min):.3f}′ {ra_dir}へ")
    _log(f"    赤緯(Dec)方向 : {abs(dec_min):.3f}′ {dec_dir}へ")
    return sep, ra_min, dec_min, ra_dir, dec_dir


def _norm(data):
    """
    星が見えるように整える。

    v5 は 1〜99 パーセンタイルを使っていたが、星野では明るい画素が全体の
    1% に満たないので上限がノイズの中に落ち、星が軒並み真っ白になっていた。
    背景の中央値とノイズ σ を基準にする。
    """
    from astropy.stats import sigma_clipped_stats

    finite = np.isfinite(data)
    if not finite.any():
        return None
    vals = data[finite]
    _, med, sigma = sigma_clipped_stats(vals, sigma=3.0)
    if not np.isfinite(sigma) or sigma <= 0:
        sigma = float(np.std(vals)) or 1.0
    lo = float(med) - 1.0 * float(sigma)
    hi = max(float(np.percentile(vals, 99.9)), float(med) + 12.0 * float(sigma))
    if not (hi > lo):
        hi = lo + 1e-6
    return ImageNormalize(vmin=lo, vmax=hi, stretch=AsinhStretch(1.0))


def clip_arrow_to_boundary(x0, y0, x1, y1, xlim, ylim):
    """画面外の目標へ向かう矢印を、画面の縁で止める。"""
    xmin, xmax = min(xlim), max(xlim)
    ymin, ymax = min(ylim), max(ylim)
    if xmin <= x1 <= xmax and ymin <= y1 <= ymax:
        return x1, y1, True
    dx, dy = x1 - x0, y1 - y0
    if abs(dx) < 1e-12 and abs(dy) < 1e-12:
        return x1, y1, True
    ts = []
    if abs(dx) > 1e-12:
        ts += [(xmin - x0) / dx, (xmax - x0) / dx]
    if abs(dy) > 1e-12:
        ts += [(ymin - y0) / dy, (ymax - y0) / dy]
    best = None
    for t in ts:
        if t <= 1e-9:
            continue
        ix, iy = x0 + t * dx, y0 + t * dy
        if xmin - 1e-6 <= ix <= xmax + 1e-6 and ymin - 1e-6 <= iy <= ymax + 1e-6:
            best = t if best is None else min(best, t)
    if best is None:
        return x1, y1, False
    return x0 + best * 0.97 * dx, y0 + best * 0.97 * dy, False


def _draw_markers(ax, wcs, shape, target, label_target=True, draw_arrow=True):
    ny, nx = shape
    center = as_icrs(wcs.pixel_to_world((nx - 1) / 2.0, (ny - 1) / 2.0))
    sx, sy = wcs.world_to_pixel(center)
    ax.scatter(sx, sy, color="cyan", marker="x", s=180, linewidths=2.0, zorder=5)
    if target is None or not draw_arrow:
        return None
    target = as_icrs(target)

    tx, ty = wcs.world_to_pixel(target)
    if not np.all(np.isfinite([tx, ty])):
        # 反対の空を指しているなど、投影が破綻するとき: 方向だけ求めて遠くを指す
        pa = center.position_angle(target)
        near = center.directional_offset_by(pa, 1 * u.arcmin)
        nxp, nyp = wcs.world_to_pixel(near)
        tx, ty = sx + (nxp - sx) * 1e5, sy + (nyp - sy) * 1e5

    cx, cy, inside = clip_arrow_to_boundary(sx, sy, tx, ty,
                                            ax.get_xlim(), ax.get_ylim())
    ax.annotate("", xy=(cx, cy), xytext=(sx, sy), zorder=6,
                arrowprops=dict(facecolor="red", edgecolor="red",
                                width=2.4, headwidth=13, headlength=16,
                                alpha=0.9))
    if inside:
        ax.scatter(tx, ty, s=520, facecolors="none", edgecolors="yellow",
                   linewidths=2.0, zorder=6)
        if label_target:
            ax.annotate("TARGET", xy=(tx, ty), xytext=(14, 14),
                        textcoords="offset points", color="yellow",
                        fontsize=12, fontweight="bold", zorder=7)
    return inside


def plot_original(wcs, data, target, flip_y, title_extra="", sources=None,
                  draw_arrow=True):
    """撮影したときと同じ向きの画像に、目標への矢印を描く。"""
    ny, nx = data.shape
    fig = plt.figure(figsize=(12, 12 * ny / nx + 1.2))
    ax = fig.add_subplot(111, projection=wcs)
    ax.imshow(data, origin="lower", cmap="gray", norm=_norm(data))
    ax.set_xlim(-0.5, nx - 0.5)
    ax.set_ylim(-0.5, ny - 0.5)
    if flip_y:
        ax.invert_yaxis()      # 表示だけ上下反転 (座標系はそのまま)
    ax.coords.grid(color="deepskyblue", linestyle="--", linewidth=0.9, alpha=0.65)
    ax.coords[0].set_axislabel("赤経 (RA)")
    ax.coords[1].set_axislabel("赤緯 (Dec)")

    if sources:
        ax.scatter([s["x"] for s in sources], [s["y"] for s in sources],
                   s=90, facecolors="none", edgecolors="lime",
                   linewidths=0.8, alpha=0.55, zorder=4)

    _draw_markers(ax, wcs, data.shape, target, draw_arrow=draw_arrow)
    subtitle = (f"目標: {target.to_string('hmsdms', precision=2)}"
                if (target is not None and draw_arrow) else "目標未指定 (中心のみ表示)")
    ax.set_title(f"撮影時の向き{title_extra}\n{subtitle}", size=14)
    plt.tight_layout()
    plt.show()


def plot_north_up(wcs, data, target):
    """北を上・東を左にそろえた画像を作る。赤道儀の軸操作と向きが揃う。"""
    from reproject import reproject_interp

    ny, nx = data.shape
    scale = float(np.mean(proj_plane_pixel_scales(wcs.celestial)))
    center = as_icrs(wcs.pixel_to_world((nx - 1) / 2.0, (ny - 1) / 2.0))

    out = WCS(naxis=2)
    out.wcs.ctype = ["RA---TAN", "DEC--TAN"]
    out.wcs.crval = [center.ra.deg, center.dec.deg]
    out.wcs.cdelt = np.array([-scale, scale])
    out.wcs.crpix = [1.0, 1.0]

    corners_x = np.array([0, nx - 1, 0, nx - 1], dtype=float)
    corners_y = np.array([0, 0, ny - 1, ny - 1], dtype=float)
    sky = wcs.pixel_to_world(corners_x, corners_y)
    px, py = out.world_to_pixel(sky)
    # FITS の CRPIX は 1 始まりなので +1 する (v5 はここが 1px ずれていた)
    out.wcs.crpix = [1.0 - float(np.min(px)), 1.0 - float(np.min(py))]
    shape_out = (int(np.ceil(np.ptp(py))) + 1, int(np.ceil(np.ptp(px))) + 1)

    # 大きな画像では、この再投影が全体でいちばんメモリを食う。
    # block_size を渡すと小分けに処理する。実測 (19.1 メガ画素):
    #   そのまま 2,581 MB / 7.1 秒  →  小分け 557 MB / 8.8 秒
    # 結果は同一 (平均も有効画素数も一致)。時間は 2 割増だが、
    # 一眼レフの画像でメモリが足りずに落ちるほうが困る。
    try:
        rep, foot = reproject_interp((np.nan_to_num(data), wcs), out,
                                     shape_out=shape_out,
                                     block_size=(512, 512))
    except TypeError:            # 古い reproject には block_size が無い
        rep, foot = reproject_interp((np.nan_to_num(data), wcs), out,
                                     shape_out=shape_out)
    rep = np.where(foot > 0, rep, np.nan)

    fig = plt.figure(figsize=(12, 12 * shape_out[0] / shape_out[1] + 1.2))
    ax = fig.add_subplot(111, projection=out)
    ax.imshow(rep, origin="lower", cmap="gray", norm=_norm(rep))
    ax.coords.grid(color="deepskyblue", linestyle="--", linewidth=0.9, alpha=0.65)
    ax.coords[0].set_axislabel("赤経 (RA)")
    ax.coords[1].set_axislabel("赤緯 (Dec)")

    _draw_markers(ax, out, shape_out, target)

    sep, ra_min, dec_min, ra_dir, dec_dir = report_offsets(center, target)
    ax.set_title(f"北が上・東が左\n"
                 f"距離 {sep.arcmin:.2f}′ | "
                 f"RA {abs(ra_min):.2f}′({ra_dir}) "
                 f"Dec {abs(dec_min):.2f}′({dec_dir})", size=14)
    plt.tight_layout()
    plt.show()
    return sep, ra_min, dec_min

def setup_japanese_font():
    """図の日本語が豆腐 (□□□) にならないようフォントを選ぶ。"""
    import matplotlib.font_manager as fm
    have = {f.name for f in fm.fontManager.ttflist}
    for name in ("Yu Gothic", "Meiryo", "MS Gothic", "Noto Sans CJK JP",
                 "IPAexGothic", "Hiragino Sans"):
        if name in have:
            matplotlib.rcParams["font.family"] = name
            matplotlib.rcParams["axes.unicode_minus"] = False
            return name
    return None


ZAHYOU_FONT = setup_japanese_font()

# 選ばれた画像はここに入る。セル C から参照される。
ZAHYOU_PICKED = {"path": None, "name": None}


def _widget_files(w):
    """ipywidgets 7 系 (dict) と 8 系 (tuple) の両方に対応する。"""
    v = w.value
    if not v:
        return []
    if isinstance(v, dict):                      # v7: {filename: {...}}
        return [{"name": k, "content": d["content"]} for k, d in v.items()]
    return [{"name": f["name"], "content": f["content"]} for f in v]


def pick_image():
    """
    画像を選ぶボタンを出す。選んだファイルは ZAHYOU_PICKED に入る。

    戻り値は None にしておくこと。ウィジェットを返すと、display() で出した分と
    「セルの最後の式の値」として自動表示される分の 2 つが並んでしまう。
    """
    import ipywidgets as widgets
    from IPython.display import display

    status = widgets.Output()
    uploader = widgets.FileUpload(
        accept=".fits,.fit,.fts,.jpg,.jpeg,.png,.tif,.tiff",
        multiple=False,
        description="画像ファイルを選択",
        button_style="primary",
        layout=widgets.Layout(width="260px"),
    )

    def on_upload(_change):
        files = _widget_files(uploader)
        if not files:
            return
        f = files[0]
        suffix = os.path.splitext(f["name"])[1] or ".fits"
        tmp = tempfile.NamedTemporaryFile(delete=False, suffix=suffix,
                                          prefix="zahyou_")
        tmp.write(bytes(f["content"]))
        tmp.close()
        ZAHYOU_PICKED["path"] = tmp.name
        ZAHYOU_PICKED["name"] = f["name"]
        uploader.button_style = "success"
        with status:
            status.clear_output(wait=True)
            print(f"✅ 選択しました: {f['name']}  ({len(f['content']) / 1024:.0f} KB)")

    uploader.observe(on_upload, names="value")
    display(widgets.VBox([uploader, status]))
    print("画像を選んだら、次のセル C を実行してください。")
    print("※ ボタンを使わず、セル C の IMAGE_PATH にパスを直接書いても構いません。")
    return None

# ============================================================== 実行本体 ===

def _pick_image_path():
    if IMAGE_PATH:
        if not os.path.exists(IMAGE_PATH):
            raise FileNotFoundError(f"IMAGE_PATH が見つかりません: {IMAGE_PATH}")
        return IMAGE_PATH, os.path.basename(IMAGE_PATH)
    picked = globals().get("ZAHYOU_PICKED") or {}
    if picked.get("path"):
        return picked["path"], picked.get("name") or os.path.basename(picked["path"])
    raise RuntimeError(
        "画像が選ばれていません。1 つ目のセルのボタンで選ぶか、"
        "IMAGE_PATH にファイルのパスを書いてください。")


def _resolve_target(online):
    """設定から目標の SkyCoord を作る。作れなければ None。"""
    # 完全ブラインド: 目標を決めずに、画像がどこを向いているかだけ知りたいとき。
    # 天体名が空のときも同じ扱いにする (打ち忘れで止まるより親切)。
    if INPUT_MODE == 'NONE' or (INPUT_MODE == 'STAR_NAME'
                                and not str(TARGET_STAR_NAME or '').strip()):
        _log("  目標は指定されていません。画像がどこを向いているかだけ求めます。")
        return None
    if INPUT_MODE == 'COORDS':
        try:
            return SkyCoord(RA_INPUT_STR, DEC_INPUT_STR, frame='icrs')
        except Exception as e:
            _log(f"❌ 赤経赤緯を読み取れません ({e})。書式を確認してください。")
            return None
    if INPUT_MODE == 'STAR_NAME':
        ra, dec = resolve_target_name(TARGET_STAR_NAME, online=online)
        if ra is not None:
            return SkyCoord(ra=ra * u.deg, dec=dec * u.deg, frame='icrs')
        if RA_INPUT_STR and DEC_INPUT_STR:
            try:
                c = SkyCoord(RA_INPUT_STR, DEC_INPUT_STR, frame='icrs')
                _log("  → 代わりに RA_INPUT_STR / DEC_INPUT_STR の座標を使います。")
                return c
            except Exception:
                pass
        return None
    _log(f"❌ INPUT_MODE が不正です: {INPUT_MODE!r} ('STAR_NAME' か 'COORDS')")
    return None


def _existing_wcs_header(header, shape):
    """画像に元から入っている WCS が使えるなら、その FITS ヘッダを返す。"""
    try:
        h = normalize_wcs_header(header, shape)
        w = WCS(h)
        if not w.is_celestial:
            return None
        s = float(np.mean(proj_plane_pixel_scales(w.celestial)) * 3600.0)
        return h if 0.01 < s < 3600 else None
    except Exception:
        return None


def _cancel_requested(say=True):
    """
    GUI の「中止」が押されたか。合図は CANCEL_EVENT (threading.Event) で届く。

    ノートブックにはこの設定が無い (None) ので、いつも False。
    解析の段の間で確かめ、オフライン解析の中には同じ Event をそのまま渡す。
    """
    ev = globals().get("CANCEL_EVENT")
    if ev is not None and ev.is_set():
        if say:
            _log("\n■ 中止しました。")
        return True
    return False


def _sanity_ok(wcs, shape):
    """解いた結果がまともかどうか、描画の前に確かめる。"""
    try:
        s = float(np.mean(proj_plane_pixel_scales(wcs.celestial)) * 3600.0)
        c = wcs.pixel_to_world(shape[1] / 2.0, shape[0] / 2.0)
        return (0.01 < s < 3600) and np.isfinite(c.ra.deg) and np.isfinite(c.dec.deg)
    except Exception as e:
        _log(f"  ⚠️ WCS の検証に失敗しました: {e}")
        return False


# ログの見出しの版。exe と同じ版名にする(rs/Cargo.toml の [workspace.package] version に "v" を付け、末尾の 0 の部分を
# 省いたもの。8.0.0 → "v8"、β版は "v<大きな版>-beta.<番号>" の形。rs/app/build.rs の version_label と同じ決まり)。
# 食い違うと rs/tools/make_release_assets.py の notebook の検査と、試験 version_in_docs_matches_cargo が止める。
NB_VERSION = "v8"


def run():
    t0 = time.time()
    img_path, img_name = _pick_image_path()
    _log("=" * 62)
    _log(f"  zahyou {NB_VERSION}   {img_name}")
    _log("=" * 62)

    # ---------------------------------------------------------------- 画像 ---
    bundle = load_image_any(img_path)
    ny, nx = bundle.shape
    _log(f"読み込み : {nx} x {ny} 画素   [{bundle.note}]")
    if _cancel_requested():
        return None

    # ------------------------------------------------------------ 星の検出 ---
    # オンラインでもオフラインでも同じ星のリストを使う。
    # v5 は 1〜99 パーセンタイルで 8bit PNG に変換していたため、星が軒並み
    # 真っ白に飽和し、ノイズと区別がつかなくなって解けなくなっていた。
    _log("\n星を検出しています...")
    sub, mask, sigma, info = preprocess(bundle.data)
    for line in info:
        _log(f"  - {line}")
    # 自前のソルバーは星が多いほど quad を選べるので、60 個見つかるまで粘る
    # (v6 は solve-field に渡すので 12 個で止めていた)
    sources, thr = detect_sources(sub, mask, sigma, min_sources=60)
    if sources:
        fwhm = float(np.median([s["fwhm"] for s in sources]))
        _log(f"  - 星を {len(sources)} 個検出 "
             f"(しきい値 {thr:g}σ / 典型 FWHM {fwhm:.1f} px)")
        if len(sources) < 8:
            _log("  ⚠️ 星が少なすぎます。露出を伸ばすか複数フレームを重ねてください。")
    else:
        _log("  ⚠️ 星を検出できませんでした。露出・ピント・雲を確認してください。")
    if _cancel_requested():
        return None

    # ------------------------------------------------------------- つながり ---
    if SOLVE_MODE == 'ONLINE':
        online = True
    elif SOLVE_MODE == 'OFFLINE':
        online = False
    else:
        online = internet_available()
    _log("\n🌍 オンライン環境で実行します。" if online else "\n🔌 オフライン環境で実行します。")

    # ---------------------------------------------------------------- 目標 ---
    target = _resolve_target(online)
    hint = (target.ra.deg, target.dec.deg) if (target is not None and USE_TARGET_AS_HINT) else None
    scale_hint, scale_why = estimate_pixel_scale(bundle.header, FOCAL_LENGTH_MM,
                                                 bundle.shape)

    # ---------------------------------------------------------------- 解析 ---
    wcs_header, how = None, ""

    if not IGNORE_EXISTING_WCS:
        h = _existing_wcs_header(bundle.header, bundle.shape)
        if h is not None:
            _log("\n✅ 画像に入っていた WCS をそのまま使います。")
            wcs_header, how = h, "画像に入っていた WCS"

    work_dir = tempfile.mkdtemp(prefix="zahyou_solve_")
    try:
        if wcs_header is None and online:
            _log(f"\n[オンライン] nova.astrometry.net に問い合わせます "
                 f"(最大 {ONLINE_TIMEOUT} 秒)")
            if scale_hint:
                _log(f"  - 画素スケールの手がかり: {scale_hint:.3f}″/px ({scale_why})")
            if hint:
                _log(f"  - 目標の方向 (半径 {SEARCH_RADIUS_DEG:g}°) を手がかりにします。")
            try:
                wcs_header = solve_online(
                    img_path, API_KEY, sources=sources, shape=bundle.shape,
                    timeout=ONLINE_TIMEOUT, hint_radec=hint,
                    hint_radius_deg=SEARCH_RADIUS_DEG, scale_hint=scale_hint,
                    cancel=globals().get("CANCEL_EVENT"))
            except Exception as e:
                _log(f"⚠️ オンライン解析でエラー: {type(e).__name__}: {e}")
                wcs_header = None
            if wcs_header is not None:
                _log("✅ オンライン解析に成功しました。")
                how = "オンライン (nova.astrometry.net)"
            elif SOLVE_MODE == 'ONLINE':
                _log("SOLVE_MODE='ONLINE' なのでここで終了します。")
            else:
                _log("   → オフラインの解析に切り替えます。")
            if wcs_header is None and _cancel_requested():
                return None

        if wcs_header is None and SOLVE_MODE != 'ONLINE':
            if _cancel_requested():
                return None
            index_dir = globals().get("INDEX_DIR") or DEFAULT_INDEX_DIR
            _log("\n[オフライン] 星図データを確認しています...")
            diag = solver_diagnostics(index_dir)
            for m in diag["messages"]:
                _log(f"⚠️ {m}")
            if diag["ok"]:
                _log(f"  - 星図データ {len(diag['indexes'])} ファイル ({index_dir})")
                wcs_header = solve_offline(
                    bundle, sources, work_dir, timeout=OFFLINE_TIMEOUT,
                    focal_length_mm=FOCAL_LENGTH_MM, hint_radec=hint,
                    hint_radius_deg=SEARCH_RADIUS_DEG, diagnostics=diag,
                    index_dir=index_dir, prep=(sub, mask, sigma),
                    cancel=globals().get("CANCEL_EVENT"))
                if wcs_header is not None:
                    how = "オフライン (zahyou 内蔵の解析エンジン)"

        if wcs_header is None:
            if _cancel_requested(say=False):    # 中止はオフライン解析が知らせ済み
                return None
            _log("\n❌ 座標を特定できませんでした。")
            return None

        # ------------------------------------------------------------ 検算 ---
        wcs = WCS(normalize_wcs_header(wcs_header, bundle.shape))
        if not _sanity_ok(wcs, bundle.shape):
            _log("❌ 解析結果がおかしいので採用しません。")
            return None

        # 位置角とパリティは、図 1 と同じ「画面の見た目」で出す (上下反転を考える)
        summary = print_solution(wcs, bundle.shape, flip_y=bundle.flip_y)
        _log(f"  解いた方法    : {how}")
        _log(f"  所要時間      : {time.time() - t0:.1f} 秒")

        # 次回この機材で撮った画像は、設定を書かなくても一発で絞り込める
        f_mm = implied_focal_length(bundle.header, summary["scale"])
        if f_mm:
            _log(f"  逆算した焦点距離: {f_mm:.0f} mm")
        if scale_hint is None:
            _log("  → この機材の画素スケールを記憶しました "
                 "(次からは画角の総当たりをしません)")
        remember_scale(bundle.header, bundle.shape, summary["scale"])

        # ------------------------------------------------------------ 描画 ---
        if _cancel_requested():
            return None
        shown = sources if SHOW_DETECTED_SOURCES else None

        if target is None:
            _log("\n⚠️ 目標の座標が無いので、矢印は描きません。")
            _log("   オフラインで天体名を使いたいときは、一度オンラインで実行して"
                 "座標を覚えさせるか、INPUT_MODE='COORDS' にしてください。")
            plot_original(wcs, bundle.display, None, bundle.flip_y,
                          sources=shown, draw_arrow=False)
            return wcs

        tx, ty = wcs.world_to_pixel(target)
        if np.all(np.isfinite([tx, ty])) and 0 <= tx < nx and 0 <= ty < ny:
            _log(f"\n  目標は画面内です: ピクセル ({float(tx):.1f}, {float(ty):.1f})")
            if sources:
                d = [float(np.hypot(s["x"] - tx, s["y"] - ty)) for s in sources]
                i = int(np.argmin(d))
                near_px = max(4.0, 3.0 * float(np.median([s["fwhm"] for s in sources])))
                if d[i] <= near_px:
                    _log(f"  検出した星と {d[i]:.1f} px "
                         f"({d[i] * summary['scale']:.1f}″) で一致しました。")
                else:
                    _log("  目標そのものは、この検出しきい値では拾えていません "
                         "(暗い星なら正常です)。")
        else:
            _log("\n  目標は画面の外です。矢印の向きへ動かしてください。")

        plot_original(wcs, bundle.display, target, bundle.flip_y, sources=shown)
        if _cancel_requested():                 # 北が上の図の描き直しは重い
            return None
        plot_north_up(wcs, bundle.display, target)
        return wcs

    finally:
        shutil.rmtree(work_dir, ignore_errors=True)
