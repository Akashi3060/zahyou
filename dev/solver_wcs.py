# ==============================================================================
#  solver_wcs.py — 自前ソルバーの幾何(単位ベクトル・TAN 投影・WCS の当てはめ)
#
#  docs/design-native-solver.md の 1 節・3.6 節・3.8 節・3.9 節。
#  numpy と標準ライブラリだけを使う(第 2 段で別の言語へ移しやすくするため)。
#  build_nb.py が zahyou_engine.py へつなぐので、最上位の名前は sv_ / Sv / _sv_ / SV_ で始める。
#  冒頭に docstring を置かない(組み立て時に import だけを外す約束)。
#
#  座標の約束
#    - 天球の点は単位ベクトル (x, y, z) = (cosδ cosα, cosδ sinα, sinδ)(赤道座標、ICRS)。
#    - 接平面の座標は (ξ, η) = (東, 北)、単位はラジアン。FITS の中間世界座標と同じ向き
#      (ξ は赤経が増える向き)。
#    - 画素は 0 始まり(x = 列、y = 行)。FITS のヘッダーに書くときだけ CRPIX に 1 を足す。
#    - SvWcs の CD は度/px、中間世界座標 (ξ, η) も度(FITS と同じ)。投影の関数はラジアンのまま。
#    - SIP は FITS の規約(Shupe et al. 2005)どおり。u, v = 画素 − CRPIX として
#        (ξ, η) = CD · (u + A(u, v), v + B(u, v))          A, B は 2 ≤ p+q ≤ 次数
#        U, V = CD⁻¹ · (ξ, η)、u = U + AP(U, V)、v = V + BP(U, V)   AP, BP は 0 ≤ p+q ≤ 次数
#      係数は (次数+1, 次数+1) の配列で、[p, q] が u^p v^q の係数(範囲の外は 0)。
# ==============================================================================

import math
import textwrap

import numpy as np


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
