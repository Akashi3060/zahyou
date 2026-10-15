"""anindex.py -- reference decoder for astrometry.net index files (index-41xx / index-52xx-NN).

Written from the files themselves (headers + measured data), not from astrometry.net source.
See FORMAT.md next to this file for the specification and the measured evidence.

Only numpy is required.  No astropy: the FITS walker below is deliberately minimal so that it
can be ported line-by-line to another language (Rust etc.).

    idx = load_index(r"C:\\AstrometryData\\index-4119.fits")
    idx["stars_xyz"]   (NSTARS, 3) float64 unit vectors (J2000 ICRS-ish, x=cos d cos a, y=cos d sin a, z=sin d)
    idx["codes"]       (NQUADS, 2*(DIMQUADS-2)) float64 geometric hash codes
    idx["quads"]       (NQUADS, DIMQUADS) uint32 star indices: columns A, B, C(, D)
    idx["sweep"]       (NSTARS,) uint8 brightness-order byte (0 = earliest/brightest)
    idx["mags"]        dict of float32 arrays (MAG_BT, MAG_VT, MAG_HP, MAG) or None (5200 series)
    idx["header"]      dict: primary / codes / stars header dicts
    idx["tree"]        {"codes": KdTree, "stars": KdTree} if with_tree=True

Tree helpers: kd_box_search(tree, lo, hi), kd_ball_search(tree, center, radius) use the
stored tree as-is (no rebuild).
"""
from __future__ import annotations

import math
import numpy as np

FITS_BLOCK = 2880

# ---------------------------------------------------------------- FITS walker


def _parse_value(v: str):
    v = v.strip()
    if v.startswith("'"):
        out = []
        i = 1
        while i < len(v):
            if v[i] == "'":
                if i + 1 < len(v) and v[i + 1] == "'":
                    out.append("'")
                    i += 2
                    continue
                break
            out.append(v[i])
            i += 1
        return "".join(out).rstrip()
    if "/" in v:
        v = v.split("/", 1)[0].strip()
    if v == "T":
        return True
    if v == "F":
        return False
    if v == "":
        return None
    try:
        return int(v)
    except ValueError:
        try:
            return float(v.replace("D", "E"))
        except ValueError:
            return v


def read_hdus(path):
    """Walk all HDUs. Returns list of dict(header=dict, cards=[str], data_offset=int, data_size=int)."""
    hdus = []
    with open(path, "rb") as f:
        f.seek(0, 2)
        fsize = f.tell()
        pos = 0
        while pos < fsize:
            f.seek(pos)
            hdr, cards, nblk, done = {}, [], 0, False
            while not done:
                blk = f.read(FITS_BLOCK)
                nblk += 1
                if len(blk) < FITS_BLOCK:
                    raise IOError(f"{path}: truncated FITS header at {pos}")
                for i in range(36):
                    c = blk[i * 80:(i + 1) * 80].decode("ascii", "replace")
                    cards.append(c.rstrip())
                    key = c[:8].strip()
                    if key == "END":
                        done = True
                        break
                    if key and c[8:10] == "= ":
                        hdr.setdefault(key, _parse_value(c[10:]))  # first occurrence wins
            data_off = pos + nblk * FITS_BLOCK
            naxis = hdr.get("NAXIS", 0)
            n = 0
            if naxis > 0:
                n = 1
                for k in range(1, naxis + 1):
                    n *= hdr[f"NAXIS{k}"]
            n = (n + hdr.get("PCOUNT", 0)) * hdr.get("GCOUNT", 1) * abs(hdr["BITPIX"]) // 8
            hdus.append(dict(header=hdr, cards=cards, data_offset=data_off, data_size=n))
            pos = data_off + ((n + FITS_BLOCK - 1) // FITS_BLOCK) * FITS_BLOCK
    return hdus


def _endian_prefix(endian_card):
    """ENDIAN = '04:03:02:01' means u32 0x01020304 is stored as bytes 04 03 02 01 -> little endian."""
    if endian_card in (None, "04:03:02:01"):
        return "<"
    if endian_card == "01:02:03:04":
        return ">"
    raise ValueError(f"unknown ENDIAN card {endian_card!r}")


_TYPES = {"u8": "u1", "u16": "u2", "u32": "u4", "u64": "u8", "s16": "i2", "s32": "i4",
          "double": "f8", "float": "f4"}

# ---------------------------------------------------------------- kd-tree


class KdTree(dict):
    """Plain dict with attribute access. Keys:
    name, ndim, ndata, nnodes, nleaves, ninterior, levels, int_dtype, data_dtype,
    lr (int64 [nleaves]), split_raw, split_dim (int64 [ninterior]), split_val (int64 [ninterior]),
    minval, maxval (float64 [ndim]), scale (float64), data_int (uint [ndata, ndim])
    """
    __getattr__ = dict.__getitem__


def _dim_bits(ndim):
    return max(1, int(math.ceil(math.log2(ndim))))


def tree_to_real(tree, ints):
    return tree["minval"] + np.asarray(ints, dtype=np.float64) / tree["scale"]


def real_to_tree(tree, x):
    """Real -> tree units (float, not rounded). Encoding used by the files is round-to-nearest."""
    return (np.asarray(x, dtype=np.float64) - tree["minval"]) * tree["scale"]


def leaf_range(tree, node):
    """Data index range [lo, hi) owned by any node (interior or leaf)."""
    nint = tree["ninterior"]
    lo = hi = node
    while lo < nint:
        lo = 2 * lo + 1
    while hi < nint:
        hi = 2 * hi + 2
    kl, kh = lo - nint, hi - nint
    start = 0 if kl == 0 else int(tree["lr"][kl - 1]) + 1
    end = int(tree["lr"][kh]) + 1
    return start, end


def kd_box_search(tree, lo, hi):
    """All data indices with lo <= x <= hi (real units, per dim). Uses stored splits only."""
    lo_i = np.floor(real_to_tree(tree, lo)).astype(np.int64)
    hi_i = np.ceil(real_to_tree(tree, hi)).astype(np.int64)
    data = tree["data_int"]
    nint = tree["ninterior"]
    out = []
    stack = [0]
    while stack:
        n = stack.pop()
        if n >= nint:
            s, e = leaf_range(tree, n)
            pts = data[s:e].astype(np.int64)
            ok = np.all((pts >= lo_i) & (pts <= hi_i), axis=1)
            out.append(np.nonzero(ok)[0] + s)
            continue
        d = tree["split_dim"][n]
        sv = tree["split_val"][n]
        # left child: x[d] <= sv ; right child: x[d] >= sv  (both inclusive; ties go either way)
        if lo_i[d] <= sv:
            stack.append(2 * n + 1)
        if hi_i[d] >= sv:
            stack.append(2 * n + 2)
    if not out:
        return np.zeros(0, np.int64)
    res = np.concatenate(out)
    # exact real-unit filter (the integer box was widened by floor/ceil)
    x = tree_to_real(tree, data[res])
    ok = np.all((x >= np.asarray(lo)) & (x <= np.asarray(hi)), axis=1)
    return np.sort(res[ok])


def kd_ball_search(tree, center, radius):
    """All data indices with |x - center| <= radius (Euclidean in real units).
    Prunes with per-node bounding boxes derived from the root range + split values."""
    center = np.asarray(center, dtype=np.float64)
    data = tree["data_int"]
    nint = tree["ninterior"]
    D = tree["ndim"]
    c_i = real_to_tree(tree, center)
    r_i2 = (radius * tree["scale"]) ** 2
    full = float(np.iinfo(data.dtype).max)
    out = []
    stack = [(0, np.zeros(D), np.full(D, full))]
    while stack:
        n, blo, bhi = stack.pop()
        # min squared distance from center to box
        dd = np.where(c_i < blo, blo - c_i, np.where(c_i > bhi, c_i - bhi, 0.0))
        if float(dd @ dd) > r_i2 * (1 + 1e-12) + 1e-9:
            continue
        if n >= nint:
            s, e = leaf_range(tree, n)
            x = tree_to_real(tree, data[s:e])
            ok = ((x - center) ** 2).sum(1) <= radius * radius
            out.append(np.nonzero(ok)[0] + s)
            continue
        d = tree["split_dim"][n]
        sv = float(tree["split_val"][n])
        lhi = bhi.copy(); lhi[d] = min(bhi[d], sv)
        rlo = blo.copy(); rlo[d] = max(blo[d], sv)
        stack.append((2 * n + 1, blo, lhi))
        stack.append((2 * n + 2, rlo, bhi))
    if not out:
        return np.zeros(0, np.int64)
    return np.sort(np.concatenate(out))


# ---------------------------------------------------------------- geometry


def radec_to_xyz(ra_deg, dec_deg):
    a, d = np.radians(ra_deg), np.radians(dec_deg)
    return np.stack([np.cos(d) * np.cos(a), np.cos(d) * np.sin(a), np.sin(d)], -1)


def xyz_to_radec(xyz):
    xyz = np.asarray(xyz, dtype=np.float64)
    ra = np.degrees(np.arctan2(xyz[..., 1], xyz[..., 0])) % 360.0
    dec = np.degrees(np.arcsin(np.clip(xyz[..., 2] / np.linalg.norm(xyz, axis=-1), -1, 1)))
    return ra, dec


def tangent_xy(xyz, center):
    """Gnomonic projection used by the index codes (verified against the stored codes).

    c   = center / |center|
    eta = normalize(-c_y, c_x, 0)            (points to increasing RA, 'east')
    xi  = c x eta                            (points to increasing Dec, 'north')
    x = (s . xi) / (s . c),  y = (s . eta) / (s . c)
    i.e. x = north-ish, y = east-ish  (frame (x, y) has x cross y = -c).
    For a center exactly at a pole use eta = (0, 1, 0) (not exercised by the data check).
    """
    c = np.asarray(center, dtype=np.float64)
    c = c / np.linalg.norm(c, axis=-1, keepdims=True)
    eta = np.stack([-c[..., 1], c[..., 0], np.zeros_like(c[..., 0])], -1)
    en = np.linalg.norm(eta, axis=-1, keepdims=True)
    eta = np.where(en > 0, eta / np.where(en > 0, en, 1), np.array([0.0, 1.0, 0.0]))
    xi = np.cross(c, eta)
    s = np.asarray(xyz, dtype=np.float64)
    dot = (s * c[..., None, :]).sum(-1) if s.ndim == c.ndim + 1 else (s * c).sum(-1)
    if s.ndim == c.ndim + 1:
        x = (s * xi[..., None, :]).sum(-1) / dot
        y = (s * eta[..., None, :]).sum(-1) / dot
    else:
        x = (s * xi).sum(-1) / dot
        y = (s * eta).sum(-1) / dot
    return x, y


def compute_codes(stars_xyz, quads):
    """Recompute the stored code of each quad from its stars (reference definition).
    Center of projection = normalized midpoint of A and B; similarity transform A->(0,0), B->(1,1).
    Returns (nq, 2*(dq-2)) array (cx, cy, dx, dy ...) in the stored column order."""
    P = stars_xyz[np.asarray(quads, dtype=np.int64)]  # nq, dq, 3
    cen = P[:, 0] + P[:, 1]
    x, y = tangent_xy(P, cen)
    z = x + 1j * y
    w = (z - z[:, :1]) / (z[:, 1:2] - z[:, :1]) * (1 + 1j)
    w = w[:, 2:]
    return np.stack([w.real, w.imag], -1).reshape(len(P), -1)


def canonical_code(xy):
    """Image side: code of 3 or 4 planar points (any order) with the index's canonical rules.

    xy: (k, 2), k in (3, 4), e.g. pixel coordinates.  Returns (code[2*(k-2)], order[k]) where
    order gives the input indices of A, B, C(, D).
      * A,B = the widest pair
      * k == 4: if cx + dx > 1 swap A<->B  (code -> 1 - code), then if cx > dx swap C<->D
      * k == 3: if cx > 1/2 swap A<->B
    A pixel frame with det(CD) < 0 (normal un-mirrored sky in FITS x-right / y-up pixels, i.e. a
    y-up frame showing north up / east left) matches the stored codes directly; for the other
    parity call canonical_code(xy[:, ::-1]) (swap x and y) -- see FORMAT.md 'parity'.
    """
    xy = np.asarray(xy, dtype=np.float64)
    k = len(xy)
    z = xy[:, 0] + 1j * xy[:, 1]
    best = (-1.0, 0, 1)
    for i in range(k):
        for j in range(i + 1, k):
            dd = abs(z[i] - z[j])
            if dd > best[0]:
                best = (dd, i, j)
    _, a, b = best
    rest = [i for i in range(k) if i not in (a, b)]

    def calc(a, b, rest):
        return (z[rest] - z[a]) / (z[b] - z[a]) * (1 + 1j)

    w = calc(a, b, rest)
    if k == 4:
        if w[0].real + w[1].real > 1.0:
            a, b = b, a
            w = calc(a, b, rest)
        if w[0].real > w[1].real:
            rest = rest[::-1]
            w = w[::-1]
    elif k == 3:
        if w[0].real > 0.5:
            a, b = b, a
            w = calc(a, b, rest)
    else:
        raise ValueError("need 3 or 4 points")
    return np.stack([w.real, w.imag], -1).ravel(), [a, b] + rest


# ---------------------------------------------------------------- loader


def _table(hdus, ttype):
    for h in hdus:
        if h["header"].get("TTYPE1") == ttype:
            return h
    return None


def _kdt_header(hdus, name):
    for h in hdus:
        if h["header"].get("KDT_NAME") == name:
            return h["header"]
    raise KeyError(f"no kdtree header {name}")


def load_index(path, with_tree=True, mmap=False, normalize_stars=True):
    """Decode an astrometry.net index file. See module docstring for the result layout.

    mmap=True keeps the big integer arrays as read-only views on a np.memmap (lazy, low RSS);
    the float64 'stars_xyz' / 'codes' are always materialized.
    """
    hdus = read_hdus(path)
    raw = np.memmap(path, dtype=np.uint8, mode="r")

    def blob(h):
        b = raw[h["data_offset"]:h["data_offset"] + h["data_size"]]
        return b if mmap else np.array(b)

    H0 = hdus[0]["header"]
    e0 = _endian_prefix(H0.get("ENDIAN"))
    nq, ns, dq = H0["NQUADS"], H0["NSTARS"], H0["DIMQUADS"]
    out = dict(path=str(path), indexid=H0["INDEXID"], healpix=H0["HEALPIX"], hpnside=H0["HPNSIDE"],
               scale_l=H0["SCALE_L"], scale_u=H0["SCALE_U"], dimquads=dq, nquads=nq, nstars=ns)

    hq = _table(hdus, "quads")
    assert hq["header"]["NAXIS1"] == 4 * dq and hq["header"]["NAXIS2"] == nq
    out["quads"] = blob(hq).view(e0 + "u4").reshape(nq, dq)

    trees = {}
    for name in ("codes", "stars"):
        kh = _kdt_header(hdus, name)
        e = _endian_prefix(kh.get("ENDIAN"))
        D, N, NN = kh["KDT_NDIM"], kh["KDT_NDAT"], kh["KDT_NNOD"]
        it, dt = _TYPES[kh["KDT_INT"]], _TYPES[kh["KDT_DATA"]]
        if kh["KDT_EXT"] != "double":
            raise ValueError("unexpected KDT_EXT")
        rng = blob(_table(hdus, f"kdtree_range_{name}")).view(e + "f8")
        assert len(rng) == 2 * D + 1
        data_int = blob(_table(hdus, f"kdtree_data_{name}")).view(e + dt).reshape(N, D)
        t = KdTree(name=name, ndim=D, ndata=N, nnodes=NN, minval=np.array(rng[:D]),
                   maxval=np.array(rng[D:2 * D]), scale=float(rng[2 * D]),
                   int_dtype=kh["KDT_INT"], data_dtype=kh["KDT_DATA"], data_int=data_int,
                   header=kh)
        if with_tree:
            lr = np.asarray(blob(_table(hdus, f"kdtree_lr_{name}")).view(e + "u4")).astype(np.int64)
            sp = np.asarray(blob(_table(hdus, f"kdtree_split_{name}")).view(e + it)).astype(np.int64)
            bits = _dim_bits(D)
            mask = (1 << bits) - 1
            assert len(lr) + len(sp) == NN and len(lr) == (NN + 1) // 2
            t.update(nleaves=len(lr), ninterior=len(sp), levels=int(round(math.log2(NN + 1))),
                     lr=lr, split_raw=sp, split_dim=sp & mask, split_val=sp & ~mask, dim_bits=bits)
        trees[name] = t
        out[f"{name}_header"] = kh

    ct, st = trees["codes"], trees["stars"]
    out["codes"] = tree_to_real(ct, ct["data_int"])
    xyz = tree_to_real(st, st["data_int"])
    if normalize_stars:
        xyz /= np.linalg.norm(xyz, axis=1, keepdims=True)
    out["stars_xyz"] = xyz

    hs = _table(hdus, "sweep")
    out["sweep"] = blob(hs).view("u1") if hs else None

    hm = _table(hdus, "MAG_BT")
    if hm is not None:
        hh = hm["header"]
        nf = hh["TFIELDS"]
        cols = [hh[f"TTYPE{j}"] for j in range(1, nf + 1)]
        forms = [hh[f"TFORM{j}"] for j in range(1, nf + 1)]
        assert all(f.strip() == "1E" for f in forms)
        # Standard FITS binary table => BIG-endian float32 (unlike the 'native-endian' kd-tree blobs).
        arr = np.asarray(blob(hm)).view(">f4").reshape(hh["NAXIS2"], nf).astype("<f4")
        out["mags"] = {c: arr[:, j].copy() for j, c in enumerate(cols)}
    else:
        out["mags"] = None

    out["header"] = dict(primary=H0, codes=out.pop("codes_header"), stars=out.pop("stars_header"))
    out["tree"] = trees if with_tree else None
    return out


def sweep_group(idx):
    """Sweep number (1..CUTNSWEP) of each star, reconstructed from the sweep byte and SWEEPn cards.
    Exact except at the <=1 byte value straddling a group boundary (assigned by byte midpoint)."""
    sh = idx["header"]["stars"]
    n = idx["nstars"]
    counts = [sh[f"SWEEP{i}"] for i in range(1, sh["CUTNSWEP"] + 1)]
    cum = np.cumsum(counts)
    mid_rank = (idx["sweep"].astype(np.float64) + 0.5) * n / 256.0
    return np.searchsorted(cum, mid_rank, side="right") + 1


if __name__ == "__main__":
    import sys, time
    for p in sys.argv[1:]:
        t0 = time.perf_counter()
        d = load_index(p)
        print(p, d["indexid"], d["healpix"], d["nstars"], d["nquads"], d["dimquads"],
              "%.2fs" % (time.perf_counter() - t0))
