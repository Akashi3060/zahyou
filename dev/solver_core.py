# ==============================================================================
#  solver_core.py — 自前ソルバーの解法(星の並べ替え・quad・コード・照合・検証・改良・探索の本体)
#
#  docs/design-native-solver.md の 3 節。式と既定値の出どころは Lang et al. (2010) と、
#  astrometry.net の公開文書と BSD-3 の部分のソース(solver.c・verify.c・tweak2.c など)を研究係が読んで
#  まとめた調べ物(claude-research の notes/zahyou-native-solver-20261015.md の 1〜6 節。ソースの表現は
#  写さず、式と条件を自分の言葉で書き直したもの)。このファイルを書くときは astrometry.net のコードを読んでいない。
#  numpy と標準ライブラリだけを使う(第 2 段で別の言語へ移しやすくするため)。
#  build_nb.py が zahyou_engine.py へつなぐので、最上位の名前は sv_ / Sv / _sv_ / SV_ で始める。
#  冒頭に docstring を置かない(組み立て時に import だけを外す約束)。
#
#  流れ(sv_solve)
#    1. 星を明るい順 → 一様化(3.2)。quad には先頭 max_quad_stars 個、検証には先頭 max_verify_stars 個。
#       ヒントがあれば、タイルの索引は「その段の quad が照合で残りうるタイル」に絞る(_sv_hint_tiles)。
#    2. 星を 1 つずつ足し、足した星を含む quad だけを作る(newpoint、3.3)。quad が束にたまったら照合へ。
#    3. 索引ごとに AB の長さで quad を選び、コード(通常と鏡像、正準化の境界の近くは両方の並び)を
#       code_ball にまとめて投げる(3.4・3.5)。当たりはスケールとヒントで安く捨ててから、4 点で
#       回転+等方スケールを当てる(3.6)。
#    4. 仮説を系列ごとに newpoint の順で SV_VERIFY_STEP 個ずつ検証する(対数オッズ、3.7)。先に安い足切り
#       (検証の並びの先頭 10 個の星のどれかの 5σ 以内に索引の星があるか。小さい円で星の木を引く)をかけ、
#       通った仮説(探索では 1 割前後)だけ参照星を全部集める。tune 以上 solve 未満なら 1 次で改良して検証し直す。
#    5. solve 以上になった最初の仮説を改良(3.8)して返す(区切りごとに選ぶので、見つかればそこで止まる)。
#       ただし、改良した後の対数オッズが「試した仮説の数で上げた値」(sv_accept_logodds)に届かないもの、
#       試した仮説が 1e5 個を超えてから tune-up で上がったのに改良で一致が増えないもの(sv_tuneup_accepted)は
#       解にしない(astrometry.net には無い条件。3.7 の 6)。
#  sv_verify_wcs は与えた WCS をそのまま検証する(試験と、移植先で同じ値が出るかの確かめ用)。
#
#  座標の約束は solver_wcs.py と同じ(画素は 0 始まり、x = 列、y = 行。CD は度/px)。
#  締め切り(time.monotonic の値)と中止(cancel())は、重い処理の区切りごと(照合の code_ball の問い合わせ
#  SV_BALL_CHUNK 個・タイル 1 枚の検索・検証の束 1 つ。積み上げの中は SV_CHECK_SEC ごと)に確かめる。
#  間隔が 0.2 秒を超えないように束を分けてある(束のコードは数万個になることがあるので、索引 1 本の照合の
#  途中でも確かめる)。中身の壊れた索引は、そのファイル(か系列)を外して探索を続ける(_sv_flush)。
# ==============================================================================

import json
import math
import time

import numpy as np

from solver_wcs import *
from solver_index import *


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


def sv_passed_add(st, kind, logodds, need=None):
    """solve を超えて(tune-up で届いたものを含む)改良まで済ませたのに、改良の後の判定で解にしなかった候補を st に数える。

    kind は "short"(改良の後の対数オッズ logodds が sv_accept_logodds の値 need に届かない。最大の候補のときの need も
    覚える)か "nogain"(届いたが、tune-up で上がって改良で一致が増えない。sv_tuneup_accepted)。Rust の parts::PassedOver。
    """
    k = f"passed_{kind}"
    if st.get(k, 0) == 0 or logodds > st[k + "_best"]:
        st[k + "_best"] = logodds
        if need is not None:
            st[k + "_need"] = need
    st[k] = st.get(k, 0) + 1


def sv_passed_note(st):
    """sv_passed_add で数えた候補を、探索の終わりの「解けませんでした」「解けず」の行の末尾に足す文にする(無ければ空)。
    件数と最大だけを出す(1 候補ごとに出すと、乱数の星野で行が増えるので)。Rust の PassedOver::note と同じ文。"""
    out = ""
    if st.get("passed_short", 0):
        out += (f"。改良の後の対数オッズが足りず見送った候補 {st['passed_short']:,} 件"
                f"(最大 {st['passed_short_best']:.1f}、要った値 {st['passed_short_need']:.1f})")
    if st.get("passed_nogain", 0):
        out += (f"。tune-up で一致が増えず見送った候補 {st['passed_nogain']:,} 件"
                f"(改良の後の対数オッズの最大 {st['passed_nogain_best']:.1f})")
    return out


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
                f"(対数オッズの最大 {st.get('logodds_max', -math.inf):.1f}、{time.monotonic() - t0:.1f} 秒)"
                + sv_passed_note(st))
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
            f"対数オッズの最大 {st.get('logodds_max', -math.inf):.1f}、{time.monotonic() - t0:.1f} 秒)"
            + sv_passed_note(st))
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
                sv_passed_add(st, "short", lo, accept)
                continue
            # 試した仮説が多くなってから tune-up で上がった仮説は、改良で一致が増えたときだけ解にする
            # (同じ星への当て直しだけで上がったものは偶然)
            if tuned and not sv_tuneup_accepted(p, st["hyps"], int(nmatch[i]), int(res1["nmatch"][0])):
                sv_passed_add(st, "nogain", lo)
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
