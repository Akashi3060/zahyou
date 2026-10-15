"""How much does the code change if the tangent point is not the AB midpoint?
(the image-side code is computed in the camera's projection, centered at the field center)."""
import sys, os, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import anindex as A
rng = np.random.default_rng(3)
def codes_at(P, cen):
    x, y = A.tangent_xy(P, cen)
    z = x + 1j*y
    w = (z - z[:, :1]) / (z[:, 1:2] - z[:, :1]) * (1 + 1j)
    w = w[:, 2:]
    return np.stack([w.real, w.imag], -1).reshape(len(P), -1)
for tag in sys.argv[1:]:
    d = A.load_index(r'C:\AstrometryData\index-%s.fits' % tag, with_tree=False)
    S, Q, C = d['stars_xyz'], d['quads'].astype(int), d['codes']
    qi = rng.choice(len(Q), min(20000, len(Q)), replace=False)
    P = S[Q[qi]]
    ab = np.arccos(np.clip((P[:,0]*P[:,1]).sum(1), -1, 1))
    out = []
    for off_frac in [0.0, 0.5, 1.0, 2.0]:
        # move tangent point away from AB midpoint by off_frac * |AB| in a random direction
        mid = P[:,0] + P[:,1]; mid /= np.linalg.norm(mid, axis=1, keepdims=True)
        rnd = rng.normal(size=mid.shape); rnd -= (rnd*mid).sum(1, keepdims=True)*mid; rnd /= np.linalg.norm(rnd, axis=1, keepdims=True)
        cen = mid*np.cos(off_frac*ab)[:,None] + rnd*np.sin(off_frac*ab)[:,None]
        c = codes_at(P, cen)
        dist = np.linalg.norm(c - C[qi], axis=1)
        out.append('off=%.1f|AB|: p50 %.4f p95 %.4f max %.4f' % (off_frac, np.median(dist), np.percentile(dist, 95), dist.max()))
    print('%-8s |AB|~%.2f deg  ' % (tag, np.degrees(np.median(ab))), ' ; '.join(out))
