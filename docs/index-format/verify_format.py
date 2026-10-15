"""verify_format.py -- measure every claim in FORMAT.md against the real index files.

usage:  python verify_format.py [--quick] [files...]
        (default: 4119 4118 4115 4111 4110 4107 5206-00 5202-00, plus a header scan of all files)

Prints a report and writes verify_results.json next to this script.
"""
from __future__ import annotations

import glob
import json
import math
import os
import subprocess
import sys
import time

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import anindex as A  # noqa: E402

DATA = r"C:\AstrometryData"
DEFAULT = ["4119", "4118", "4115", "4111", "4110", "4107", "5206-00", "5206-01", "5202-00"]
RESULTS = {}


def path_of(tag):
    return os.path.join(DATA, f"index-{tag}.fits")


def log(*a):
    print(*a, flush=True)


# ------------------------------------------------------------------ 0. all-file header scan
def header_scan():
    rows = []
    for p in sorted(glob.glob(os.path.join(DATA, "index-*.fits"))):
        hd = A.read_hdus(p)
        H0 = hd[0]["header"]
        names = [h["header"].get("TTYPE1") for h in hd[1:]]
        endians = {h["header"].get("ENDIAN") for h in hd if "ENDIAN" in h["header"]}
        # every table's byte size == NAXIS1*NAXIS2 and the file ends exactly after the last padded HDU
        sizes_ok = all(h["data_size"] == h["header"].get("NAXIS1", 0) * h["header"].get("NAXIS2", 0)
                       for h in hd[1:])
        last = hd[-1]
        end = last["data_offset"] + ((last["data_size"] + 2879) // 2880) * 2880
        rows.append(dict(file=os.path.basename(p), nhdu=len(hd), names=",".join(n for n in names if n),
                         dimquads=H0["DIMQUADS"], nquads=H0["NQUADS"], nstars=H0["NSTARS"],
                         scale_l=H0["SCALE_L"], scale_u=H0["SCALE_U"], healpix=H0["HEALPIX"],
                         hpnside=H0["HPNSIDE"], endian=sorted(endians), sizes_ok=sizes_ok,
                         file_size_ok=(end == os.path.getsize(p))))
    layouts = {}
    for r in rows:
        layouts.setdefault(r["names"], []).append(r["file"])
    log(f"[scan] {len(rows)} files; table layouts:")
    for k, v in layouts.items():
        log(f"   {len(v):3d} files: {k}")
    log("[scan] ENDIAN values:", sorted({e for r in rows for e in r["endian"]}),
        " all table sizes == NAXIS1*NAXIS2:", all(r["sizes_ok"] for r in rows),
        " file size == sum of padded HDUs:", all(r["file_size_ok"] for r in rows))
    ser = {}
    for r in rows:
        s = r["file"][6:10]
        ser.setdefault(s, []).append(r)
    log("[scan] series  files dimquads  scale arcmin [L,U)  healpix/nside  NQUADS(min..max)  NSTARS(min..max)")
    for s, rs in sorted(ser.items()):
        dq = sorted({r["dimquads"] for r in rs})
        L = rs[0]["scale_l"] * 180 / math.pi * 60
        U = rs[0]["scale_u"] * 180 / math.pi * 60
        hp = sorted({r["healpix"] for r in rs})
        log(f"   {s}  {len(rs):3d}   {dq}   [{L:7.2f},{U:7.2f})  {hp[0]}..{hp[-1]}/{rs[0]['hpnside']}"
            f"  {min(r['nquads'] for r in rs)}..{max(r['nquads'] for r in rs)}"
            f"  {min(r['nstars'] for r in rs)}..{max(r['nstars'] for r in rs)}")
    RESULTS["scan"] = rows


# ------------------------------------------------------------------ 1. endianness evidence
def check_endian(p, d):
    hd = A.read_hdus(p)
    raw = np.memmap(p, dtype=np.uint8, mode="r")
    H0 = hd[0]["header"]
    res = {}
    for h in hd:
        t = h["header"].get("TTYPE1")
        b = raw[h["data_offset"]:h["data_offset"] + h["data_size"]]
        if t == "quads":
            for e in "<>":
                q = b.view(e + "u4")
                res[f"quads{e}_all_lt_nstars"] = bool((q < H0["NSTARS"]).all())
        if t in ("kdtree_lr_stars", "kdtree_lr_codes"):
            for e in "<>":
                a = b.view(e + "u4").astype(np.int64)
                res[f"{t}{e}_strictly_increasing"] = bool((np.diff(a) > 0).all())
        if t == "kdtree_range_stars":
            for e in "<>":
                res[f"range_stars{e}"] = [float(x) for x in b.view(e + "f8")]
        if t == "MAG_BT":
            for e in "<>":
                m = b.view(e + "f4")
                res[f"mag{e}_frac_in_-2..20"] = float(((m > -2) & (m < 20)).mean())
    return res


# ------------------------------------------------------------------ 2. ranges / unit vectors
def check_stars(d):
    t = d["tree"]["stars"]
    out = dict(minval=t["minval"].tolist(), maxval=t["maxval"].tolist(), scale=t["scale"],
               scale_expected=(2 ** 32 - 1) / 2.0)
    ints = t["data_int"].astype(np.float64)
    for off in (0.0, 0.5):
        xyz = t["minval"] + (ints + off) / t["scale"]
        n = np.linalg.norm(xyz, axis=1)
        out[f"norm_err_off{off}"] = dict(max=float(np.abs(n - 1).max()), rms=float(np.sqrt(((n - 1) ** 2).mean())))
    out["half_quantum"] = 0.5 / t["scale"]
    ct = d["tree"]["codes"]
    out["codes_minval"] = ct["minval"].tolist()
    out["codes_maxval"] = ct["maxval"].tolist()
    out["codes_scale"] = ct["scale"]
    out["codes_scale_expected"] = 65535 / (ct["maxval"][0] - ct["minval"][0])
    out["codes_min_int"] = int(ct["data_int"].min())
    out["codes_max_int"] = int(ct["data_int"].max())
    out["stars_min_int"] = int(t["data_int"].min())
    out["stars_max_int"] = int(t["data_int"].max())
    return out


# ------------------------------------------------------------------ 3./4. codes and quads
def _codes_variants(P, center_mode, parity):
    if center_mode == "midAB":
        cen = P[:, 0] + P[:, 1]
    else:
        cen = P.mean(1)
    x, y = A.tangent_xy(P, cen)
    if parity == "swapped":  # (x,y) -> (y,x): the other handedness
        x, y = y, x
    z = x + 1j * y
    w = (z - z[:, :1]) / (z[:, 1:2] - z[:, :1]) * (1 + 1j)
    w = w[:, 2:]
    return np.stack([w.real, w.imag], -1).reshape(len(P), -1)


def check_codes(d, chunk=400_000):
    xyz = d["stars_xyz"] / np.linalg.norm(d["stars_xyz"], axis=1, keepdims=True)
    quads = d["quads"].astype(np.int64)
    codes = d["codes"]
    nq, dq = quads.shape
    D = codes.shape[1]
    half = 0.5 / d["tree"]["codes"]["scale"]
    out = {"half_quantum": half}
    variants = [("midAB", "file"), ("midAB", "swapped"), ("mean", "file"), ("mean", "swapped")]
    acc = {v: [0.0, 0.0, 0] for v in variants}  # maxerr, sumsq, n_within_1.01*half
    ab = []
    ab_chord = []
    ab_is_max = 0
    ab_is_max_tan = 0
    prop = dict(cx_le_dx=0, cxdx_le_1=0, circle=0, tri_cx_le_half=0, tri_cx_plus_cy_le_1=0)
    circle_r2_max = 0.0
    sidx = np.arange(nq)
    for s in range(0, nq, chunk):
        e = min(nq, s + chunk)
        P = xyz[quads[s:e]]
        cs = codes[s:e]
        for v in variants:
            c = _codes_variants(P, *v)
            err = np.abs(c - cs).max(1)
            acc[v][0] = max(acc[v][0], float(err.max()))
            acc[v][1] += float((err ** 2).sum())
            acc[v][2] += int((err <= half * 1.0001 + 1e-12).sum())
        # quad scale: angle AB
        dot = np.clip((P[:, 0] * P[:, 1]).sum(1), -1, 1)
        ang = np.arccos(dot)
        ab.append(ang)
        ab_chord.append(np.linalg.norm(P[:, 0] - P[:, 1], axis=1))
        # AB is the widest pair (on the sphere)?
        pair = []
        for i in range(dq):
            for j in range(i + 1, dq):
                pair.append(((i, j), (P[:, i] * P[:, j]).sum(1)))
        # largest separation <=> smallest dot product; AB is pair[0]
        others = np.min(np.stack([pp[1] for pp in pair if pp[0] != (0, 1)], 0), 0)
        ab_is_max += int((pair[0][1] <= others + 1e-15).sum())
        # same in the code plane: C,D distances to A,B vs |AB| (=sqrt2 in code units)
        c = cs
        pts = [np.zeros((e - s, 2)), np.ones((e - s, 2))] + [c[:, 2 * k:2 * k + 2] for k in range(D // 2)]
        mx = np.zeros(e - s)
        for i in range(len(pts)):
            for j in range(i + 1, len(pts)):
                if (i, j) == (0, 1):
                    continue
                mx = np.maximum(mx, np.linalg.norm(pts[i] - pts[j], axis=1))
        ab_is_max_tan += int((mx <= math.sqrt(2) + 4 * half).sum())
        q = half * 2 + 1e-12  # one quantum tolerance
        if D == 4:
            prop["cx_le_dx"] += int((cs[:, 0] <= cs[:, 2] + q).sum())
            prop["cxdx_le_1"] += int((cs[:, 0] + cs[:, 2] <= 1 + q).sum())
        else:
            prop["tri_cx_le_half"] += int((cs[:, 0] <= 0.5 + q).sum())
            prop["tri_cx_plus_cy_le_1"] += int((cs[:, 0] + cs[:, 1] <= 1 + q).sum())
        r2 = np.zeros(e - s)
        for k in range(D // 2):
            rr = (cs[:, 2 * k] - 0.5) ** 2 + (cs[:, 2 * k + 1] - 0.5) ** 2
            r2 = np.maximum(r2, rr)
        circle_r2_max = max(circle_r2_max, float(r2.max()))
        prop["circle"] += int((r2 <= 0.5 + 4 * half).sum())
    out["variants"] = {f"{a}/{b}": dict(max_err=acc[(a, b)][0], rms=math.sqrt(acc[(a, b)][1] / nq),
                                          frac_within_half_quantum=acc[(a, b)][2] / nq) for (a, b) in variants}
    ab = np.concatenate(ab)
    abc = np.concatenate(ab_chord)
    L, U = d["scale_l"], d["scale_u"]
    out["ab_angle"] = dict(min=float(ab.min()), max=float(ab.max()), p01=float(np.percentile(ab, 1)),
                           p50=float(np.percentile(ab, 50)), p99=float(np.percentile(ab, 99)),
                           frac_in_LU=float(((ab >= L) & (ab <= U)).mean()),
                           frac_below_L=float((ab < L).mean()), frac_above_U=float((ab > U).mean()),
                           min_over_L=float(ab.min() / L), max_over_U=float(ab.max() / U))
    out["ab_chord"] = dict(frac_in_LU=float(((abc >= L) & (abc <= U)).mean()),
                           min_over_L=float(abc.min() / L), max_over_U=float(abc.max() / U))
    out["ab_widest_pair_frac_sphere"] = ab_is_max / nq
    out["ab_widest_pair_frac_codeplane"] = ab_is_max_tan / nq
    out["props"] = {k: v / nq for k, v in prop.items() if (D == 4 and not k.startswith("tri")) or
                    (D == 2 and k.startswith("tri")) or k == "circle"}
    out["circle_r2_max"] = circle_r2_max
    out["code_minmax"] = dict(min=codes.min(0).tolist(), max=codes.max(0).tolist())
    # usage
    used = np.zeros(d["nstars"], bool)
    used[quads.ravel()] = True
    cnt = np.bincount(quads.ravel(), minlength=d["nstars"])
    out["stars_used_frac"] = float(used.mean())
    out["quads_per_used_star"] = dict(mean=float(cnt[used].mean()), max=int(cnt.max()))
    srt = np.sort(quads, axis=1)
    uq = np.unique(srt, axis=0)
    out["duplicate_starsets"] = int(nq - len(uq))
    out["distinct_stars_within_quad"] = bool((np.diff(srt, axis=1) > 0).all())
    return out


# ------------------------------------------------------------------ 5. kd-tree
def check_tree(t):
    N, D, NN = t["ndata"], t["ndim"], t["nnodes"]
    lr = t["lr"]
    nint, nleaf = t["ninterior"], t["nleaves"]
    data = np.asarray(t["data_int"]).astype(np.int64)
    out = dict(ndata=N, ndim=D, nnodes=NN, nleaves=nleaf, ninterior=nint, levels=t["levels"],
               nnodes_is_2powL_minus_1=(NN + 1) & NN == 0, lr_last_is_N_minus_1=int(lr[-1]) == N - 1,
               dim_bits=t["dim_bits"])
    cnts = np.diff(np.concatenate([[-1], lr]))
    out["leaf_count_min_max"] = [int(cnts.min()), int(cnts.max())]
    starts = np.concatenate([[0], lr[:-1] + 1])
    lmin = np.minimum.reduceat(data, starts, axis=0)
    lmax = np.maximum.reduceat(data, starts, axis=0)
    nmin = np.zeros((NN, D), np.int64)
    nmax = np.zeros((NN, D), np.int64)
    nmin[nint:] = lmin
    nmax[nint:] = lmax
    for lev in range(t["levels"] - 2, -1, -1):
        idx = np.arange((1 << lev) - 1, (1 << (lev + 1)) - 1)
        nmin[idx] = np.minimum(nmin[2 * idx + 1], nmin[2 * idx + 2])
        nmax[idx] = np.maximum(nmax[2 * idx + 1], nmax[2 * idx + 2])
    ii = np.arange(nint)
    dim = t["split_dim"]
    sv = t["split_val"]
    lmx = nmax[2 * ii + 1, dim]
    rmn = nmin[2 * ii + 2, dim]
    mask = (1 << t["dim_bits"]) - 1
    out["dim_values"] = np.unique(dim).tolist()
    out["left_le_split"] = float((lmx <= sv).mean())
    out["right_ge_split"] = float((rmn >= sv).mean())
    out["left_lt_split"] = float((lmx < sv).mean())
    out["split_eq_rightmin_masked"] = float((sv == (rmn & ~mask)).mean())
    out["alt_shift_hypothesis_left_le"] = float((lmx <= (t["split_raw"] >> t["dim_bits"])).mean())
    ext = nmax[:nint] - nmin[:nint]
    out["split_dim_is_widest"] = float((ext.argmax(1) == dim).mean())
    out["root_box_int"] = [nmin[0].tolist(), nmax[0].tolist()]
    return out


def check_search(d, t, nq=40, seed=1):
    """kd_box/ball search via the stored tree == brute force."""
    rng = np.random.default_rng(seed)
    real = A.tree_to_real(t, t["data_int"])
    ok = okbox = 0
    nres = 0
    tsec = 0.0
    for _ in range(nq):
        c = real[rng.integers(len(real))] + rng.normal(0, 0.002, t["ndim"])
        r = 0.01 if t["name"] == "codes" else 3 * d["scale_u"]
        t0 = time.perf_counter()
        got = A.kd_ball_search(t, c, r)
        tsec += time.perf_counter() - t0
        bf = np.nonzero(((real - c) ** 2).sum(1) <= r * r)[0]
        ok += int(np.array_equal(got, bf))
        nres += len(bf)
        # box search with the same center, half-width r
        gb = A.kd_box_search(t, c - r, c + r)
        bb = np.nonzero(np.all((real >= c - r) & (real <= c + r), axis=1))[0]
        okbox += int(np.array_equal(gb, bb))
    return dict(queries=nq, identical_to_bruteforce=ok, box_identical_to_bruteforce=okbox,
                mean_hits=nres / nq, mean_ms=1000 * tsec / nq)


# ------------------------------------------------------------------ 6. sweep / mags
def check_sweep(d):
    sw = np.asarray(d["sweep"])
    n = d["nstars"]
    sh = d["header"]["stars"]
    counts = [sh[f"SWEEP{i}"] for i in range(1, sh["CUTNSWEP"] + 1)]
    out = dict(sum_SWEEPn_eq_N=sum(counts) == n, CUTNSIDE=sh.get("CUTNSIDE"), CUTNSWEP=sh["CUTNSWEP"],
               SWEEPn=counts, byte_min=int(sw.min()), byte_max=int(sw.max()))
    r = np.arange(n)
    cnt = np.bincount(sw, minlength=256)
    f32 = np.floor(np.float32(256) * (r.astype(np.float32) / np.float32(n))).astype(int)
    f64 = np.floor(256 * r / n).astype(int)
    out["hist_eq_floor256_r_over_N_f32"] = bool((np.bincount(f32, minlength=256) == cnt).all())
    out["hist_eq_floor256_r_over_N_f64"] = bool((np.bincount(f64, minlength=256) == cnt).all())
    out["file_order_sorted_by_sweep"] = bool((np.diff(sw.astype(int)) >= 0).all())
    if d["mags"] is not None:
        M = d["mags"]["MAG"].astype(np.float64)
        VT, HP, BT = (d["mags"][k].astype(np.float64) for k in ("MAG_VT", "MAG_HP", "MAG_BT"))
        out["MAG_eq_VT_or_HP_when_VT0"] = float(np.mean(np.where(VT != 0, np.isclose(M, VT), np.isclose(M, HP))))
        out["zero_counts"] = dict(BT=int((BT == 0).sum()), VT=int((VT == 0).sum()), HP=int((HP == 0).sum()),
                                  MAG=int((M == 0).sum()))
        out["MAG_range"] = [float(M.min()), float(M.max())]
        out["MAG_p50_p90_p99"] = [float(x) for x in np.percentile(M, [50, 90, 99])]
        cum = np.concatenate([[0], np.cumsum(counts)])
        viol = tot = 0
        for v in range(255):
            lo, hi = v * n / 256, (v + 2) * n / 256
            if np.searchsorted(cum, lo, "right") == np.searchsorted(cum, hi - 1e-9, "right"):
                tot += 1
                a, b = M[sw == v], M[sw == v + 1]
                viol += int(a.max() > b.min() + 1e-6)
        out["mag_monotone_within_group"] = f"{tot - viol}/{tot}"
        g = A.sweep_group(d)
        out["median_MAG_by_group"] = [round(float(np.median(M[g == k])), 3) for k in range(1, sh["CUTNSWEP"] + 1)]
        # global rank correlation byte vs MAG
        from scipy.stats import spearmanr
        out["spearman_byte_vs_MAG"] = float(spearmanr(sw, M).statistic)
    return out


# ------------------------------------------------------------------ encoding rule (round vs floor)
def check_rounding(d, chunk=500_000):
    """Re-encode the exactly recomputed codes and compare with the stored integers."""
    t = d["tree"]["codes"]
    st = np.asarray(t["data_int"]).astype(np.int64)
    n_round = n_floor = 0
    nq = d["nquads"]
    for s in range(0, nq, chunk):
        e = min(nq, s + chunk)
        f = (A.compute_codes(d["stars_xyz"], d["quads"][s:e]) - t["minval"]) * t["scale"]
        n_round += int((np.rint(f) == st[s:e]).all(1).sum())
        n_floor += int((np.floor(f) == st[s:e]).all(1).sum())
    return dict(round_exact_frac=n_round / nq, floor_exact_frac=n_floor / nq)


# ------------------------------------------------------------------ parity vs a FITS TAN WCS
def check_parity_wcs(d, n=300, seed=7):
    """Project quad stars to pixels with astropy TAN WCS (det(CD) < 0 and > 0), canonicalize
    with anindex.canonical_code, compare with the stored code and look it up in the code tree."""
    from astropy.wcs import WCS
    rng = np.random.default_rng(seed)
    S, Q, C = d["stars_xyz"], d["quads"].astype(np.int64), d["codes"]
    out = {}
    for name, sgn in (("detCD<0", -1.0), ("detCD>0", +1.0)):
        direct = mirrored = found = 0
        for _ in range(n):
            qi = int(rng.integers(len(Q)))
            P = S[Q[qi]]
            mid = P[0] + P[1]  # put the tangent point at the AB midpoint (see projection-center note)
            cra, cdec = A.xyz_to_radec(mid)
            pix = np.degrees(d["scale_u"]) / 500.0
            th = rng.uniform(0, 2 * np.pi)
            w = WCS(naxis=2)
            w.wcs.ctype = ["RA---TAN", "DEC--TAN"]
            w.wcs.crval = [float(cra), float(cdec)]
            w.wcs.crpix = [1000.0, 1000.0]
            R = np.array([[np.cos(th), -np.sin(th)], [np.sin(th), np.cos(th)]])
            w.wcs.cd = np.diag([sgn * pix, pix]) @ R
            ra, dec = A.xyz_to_radec(P)
            px, py = w.all_world2pix(ra, dec, 0)
            xy = np.stack([px, py], -1)[rng.permutation(len(P))]
            c1, _ = A.canonical_code(xy)
            c2, _ = A.canonical_code(xy[:, ::-1])
            direct += int(np.abs(c1 - C[qi]).max() < 1e-4)
            mirrored += int(np.abs(c2 - C[qi]).max() < 1e-4)
            c = c1 if sgn < 0 else c2
            found += int(qi in A.kd_ball_search(d["tree"]["codes"], c, 1e-4))
        out[name] = dict(direct_match=direct / n, xy_swapped_match=mirrored / n, tree_lookup_found=found / n)
    return out


# ------------------------------------------------------------------ sky sanity (4100 has magnitudes)
BRIGHT = [("Rigel", 78.63446707, -8.20163837), ("Betelgeuse", 88.79293899, 7.40706399),
          ("Polaris", 37.95456067, 89.26410897), ("Altair", 297.6958273, 8.8683212),
          ("Arcturus", 213.9153003, 19.18240916), ("Vega", 279.23473479, 38.78368896),
          ("Sirius", 101.28715533, -16.71611586), ("Canopus", 95.98795782, -52.69566138)]


def check_bright(d):
    S = d["stars_xyz"]
    M = d["mags"]["MAG"] if d["mags"] else None
    out = {}
    for name, ra, dec in BRIGHT:
        v = A.radec_to_xyz(ra, dec)
        j = int(np.argmax(S @ v))
        sep = float(np.degrees(np.arctan2(np.linalg.norm(np.cross(S[j], v)), S[j] @ v)) * 3600)
        out[name] = dict(nearest_sep_arcsec=round(sep, 3), MAG=None if M is None else round(float(M[j]), 3))
    return out


# ------------------------------------------------------------------ healpix (5200)
def check_healpix(d):
    if d["hpnside"] <= 0:
        return None
    from astropy_healpix import HEALPix
    import astropy.units as u
    from astropy.coordinates import Longitude, Latitude
    ns = d["hpnside"]
    hn = HEALPix(nside=ns, order="nested")
    ra, dec = A.xyz_to_radec(d["stars_xyz"])
    nest = np.asarray(hn.lonlat_to_healpix(Longitude(ra * u.deg), Latitude(dec * u.deg)))
    base, sub = nest // (ns * ns), nest % (ns * ns)
    x = np.zeros_like(sub)
    y = np.zeros_like(sub)
    for b in range(16):
        x |= ((sub >> (2 * b)) & 1) << b
        y |= ((sub >> (2 * b + 1)) & 1) << b
    xy = base * ns * ns + x * ns + y
    cen = d["stars_xyz"].mean(0)
    cra, cdec = A.xyz_to_radec(cen)
    cn = cen / np.linalg.norm(cen)
    rad = np.degrees(np.arccos(np.clip(d["stars_xyz"] @ cn, -1, 1))).max()
    return dict(frac_xy_scheme=float((xy == d["healpix"]).mean()), frac_nested=float((nest == d["healpix"]).mean()),
                star_center_radec=[float(cra), float(cdec)], star_radius_deg=float(rad))


def healpix_all(series=("5206", "5205", "5204")):
    """Every 52xx-NN file: all stars fall in healpix NN of the 'xy' numbering (nside 2)."""
    rows = []
    for s in series:
        for hp in range(48):
            p = path_of(f"{s}-{hp:02d}")
            d = A.load_index(p, with_tree=False)
            r = check_healpix(d)
            rows.append((s, hp, r["frac_xy_scheme"], r["frac_nested"], r["star_center_radec"], r["star_radius_deg"]))
    ok = all(r[2] == 1.0 for r in rows)
    nest_ok = sum(r[3] == 1.0 for r in rows)
    log(f"[healpix-all] {len(rows)} files ({','.join(series)}): all stars in pixel NN under xy scheme: {ok}; "
        f"under plain NESTED: {nest_ok}/{len(rows)} files; star-radius deg max {max(r[5] for r in rows):.2f}")
    RESULTS["healpix_all"] = rows


# ------------------------------------------------------------------ 7. speed / memory (fresh process)
def speed(tag):
    code = (
        "import sys,time,psutil,os,numpy as np;sys.path.insert(0,%r);import anindex as A;"
        "p=psutil.Process();r0=p.memory_info().rss;t0=time.perf_counter();"
        "d=A.load_index(%r);t1=time.perf_counter();"
        "m=p.memory_info();print(t1-t0,(m.rss-r0)/2**20,getattr(m,'peak_wset',0)/2**20)"
    ) % (HERE, path_of(tag))
    code_mm = code.replace("A.load_index(%r)" % path_of(tag), "A.load_index(%r,mmap=True)" % path_of(tag))
    code_ht = (
        "import sys,time,psutil,os,numpy as np;sys.path.insert(0,%r);import anindex as A;"
        "t0=time.perf_counter();h=A.read_hdus(%r);t1=time.perf_counter();print(t1-t0)"
    ) % (HERE, path_of(tag))
    res = {}
    for k, c in (("load", code), ("load_mmap", code_mm)):
        best = None
        for _ in range(2):
            o = subprocess.run([sys.executable, "-c", c], capture_output=True, text=True, check=True).stdout.split()
            v = [float(x) for x in o]
            best = v if best is None or v[0] < best[0] else best
        res[k] = dict(seconds=best[0], rss_delta_MiB=best[1], peak_wset_MiB=best[2])
    o = subprocess.run([sys.executable, "-c", code_ht], capture_output=True, text=True, check=True).stdout
    res["header_walk_seconds"] = float(o)
    res["file_MiB"] = os.path.getsize(path_of(tag)) / 2 ** 20
    return res


# ------------------------------------------------------------------ main
def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    quick = "--quick" in sys.argv
    tags = args or DEFAULT
    if not args:
        header_scan()
        healpix_all()
    for tag in tags:
        p = path_of(tag)
        log(f"\n================ index-{tag}")
        t0 = time.perf_counter()
        d = A.load_index(p)
        log(f"load {time.perf_counter() - t0:.2f}s  INDEXID={d['indexid']} HEALPIX={d['healpix']}/{d['hpnside']} "
            f"DIMQUADS={d['dimquads']} NQUADS={d['nquads']} NSTARS={d['nstars']} "
            f"SCALE=[{d['scale_l']:.6g},{d['scale_u']:.6g}] rad")
        R = RESULTS.setdefault(tag, {})
        R["endian"] = check_endian(p, d)
        log("[endian]", R["endian"])
        R["stars"] = check_stars(d)
        log("[ranges]", R["stars"])
        R["codes"] = check_codes(d)
        log("[codes]", json.dumps(R["codes"], indent=None))
        for nm in ("codes", "stars"):
            R[f"tree_{nm}"] = check_tree(d["tree"][nm])
            log(f"[tree {nm}]", R[f"tree_{nm}"])
            if not quick:
                R[f"search_{nm}"] = check_search(d, d["tree"][nm])
                log(f"[search {nm}]", R[f"search_{nm}"])
        R["rounding"] = check_rounding(d)
        log("[rounding]", R["rounding"])
        R["parity_wcs"] = check_parity_wcs(d, n=100 if quick else 300)
        log("[parity vs WCS]", R["parity_wcs"])
        if d["mags"] is not None:
            R["bright"] = check_bright(d)
            log("[bright stars]", R["bright"])
        R["sweep"] = check_sweep(d)
        log("[sweep/mag]", R["sweep"])
        hp = check_healpix(d)
        if hp:
            R["healpix"] = hp
            log("[healpix]", hp)
        del d
        if not quick:
            R["speed"] = speed(tag)
            log("[speed]", R["speed"])
    with open(os.path.join(HERE, "verify_results.json"), "w", encoding="utf-8") as f:
        json.dump(RESULTS, f, indent=1, default=lambda o: o.item() if hasattr(o, "item") else str(o))
    log("\nwrote verify_results.json")


if __name__ == "__main__":
    main()
