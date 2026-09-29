"""Fit hair under a cap: pull every point the cap covers in toward the head
until it sits inside the cap. Used by the character importers
(smd_to_mixamo.py, mk8_to_mixamo.py) for the hair shown while a cap is on;
jiggle.rs swaps to the full hair once the cap lifts or comes off.

A point is covered when the cap's surface lies in the same direction from the
head's centre (within a few degrees); it then stays `margin` inside the
nearest cap point in that direction, so the cap's lining hides it seated and
there is hair, not a bare scalp, under the cap when it tilts or lifts.
"""
import numpy as np


def squash(points, cap, centre, margin, cone=10.0):
    """points (N, 3) and cap (M, 3) in the same space; returns the fitted
    points and which of them moved."""
    points = np.asarray(points, float)
    cap_dirs = np.asarray(cap, float) - centre
    cap_reach = np.linalg.norm(cap_dirs, axis=1)
    cap_dirs /= np.maximum(cap_reach[:, None], 1e-9)
    d = points - centre
    r = np.linalg.norm(d, axis=1)
    u = d / np.maximum(r[:, None], 1e-9)
    inner = np.full(len(points), np.inf)
    cos = np.cos(np.radians(cone))
    for start in range(0, len(points), 2048):
        near = u[start:start + 2048] @ cap_dirs.T > cos
        inner[start:start + 2048] = np.where(near, cap_reach[None, :], np.inf).min(1)
    limit = np.maximum(inner - margin, 0.5 * inner)
    moved = r > limit
    fitted = points.copy()
    fitted[moved] = centre + u[moved] * limit[moved, None]
    return fitted, moved
