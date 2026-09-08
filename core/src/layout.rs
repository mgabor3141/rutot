//! Geometry for drawing a yard: polylines in a plain 2D space, no engine
//! types.
//!
//! The *main* runs from `(-main_len, 0)` (left throat) to `(0, 0)` (right
//! throat). Right-hand sidings fan up-right from the right throat, left-hand
//! sidings fan up-left from the left throat, and the run-round loop (if any)
//! arcs over the main between the two throats.
//!
//! Every siding gets a *path*: from the far end of the main, through the
//! near throat, into the siding. A string on the move is one scalar `s`
//! along the active path with the loco at `s` and cars trailing at
//! `s + (i+1)·car_len`; that holds for either side because `held` is ordered
//! loco-first.

use crate::yard::{Side, Yard};

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct P2 {
    pub x: f32,
    pub y: f32,
}

impl P2 {
    pub const fn new(x: f32, y: f32) -> Self {
        P2 { x, y }
    }
    fn lerp(self, o: P2, t: f32) -> P2 {
        P2::new(self.x + (o.x - self.x) * t, self.y + (o.y - self.y) * t)
    }
    fn dist(self, o: P2) -> f32 {
        ((o.x - self.x).powi(2) + (o.y - self.y).powi(2)).sqrt()
    }
}

#[derive(Clone, Debug)]
pub struct Polyline {
    pub pts: Vec<P2>,
    cum: Vec<f32>,
}

impl Polyline {
    pub fn new(pts: Vec<P2>) -> Self {
        let mut cum = vec![0.0];
        for w in pts.windows(2) {
            cum.push(cum.last().unwrap() + w[0].dist(w[1]));
        }
        Polyline { pts, cum }
    }
    pub fn length(&self) -> f32 {
        *self.cum.last().unwrap_or(&0.0)
    }
    /// Point at distance `d` from the start, clamped to the ends.
    pub fn point_at(&self, d: f32) -> P2 {
        let d = d.clamp(0.0, self.length());
        let i = self.cum.partition_point(|&c| c <= d).clamp(1, self.pts.len() - 1);
        let seg = self.cum[i] - self.cum[i - 1];
        let t = if seg > 0.0 { (d - self.cum[i - 1]) / seg } else { 0.0 };
        self.pts[i - 1].lerp(self.pts[i], t)
    }
    /// Direction angle (radians) of the segment containing `d`.
    pub fn heading_at(&self, d: f32) -> f32 {
        let d = d.clamp(0.0, self.length());
        let i = self.cum.partition_point(|&c| c <= d).clamp(1, self.pts.len() - 1);
        let (a, b) = (self.pts[i - 1], self.pts[i]);
        (b.y - a.y).atan2(b.x - a.x)
    }
    /// Concatenate, assuming `self` ends where `other` starts.
    pub fn join(&self, other: &Polyline) -> Polyline {
        let mut pts = self.pts.clone();
        pts.extend(other.pts.iter().skip(1).copied());
        Polyline::new(pts)
    }
    pub fn reversed(&self) -> Polyline {
        let mut pts = self.pts.clone();
        pts.reverse();
        Polyline::new(pts)
    }
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub car_len: f32,
    /// Left throat → right throat.
    pub main: Polyline,
    /// Throat → stop block, one per siding.
    pub sidings: Vec<Polyline>,
    /// Far end of main → near throat → siding, one per siding.
    pub paths: Vec<Polyline>,
    /// Left throat → over the top → right throat, if the yard has a loop.
    pub loop_track: Option<Polyline>,
    /// Distance from the start of the siding polyline to the first slot's
    /// leading edge (the diagonal plus a little buffer).
    siding_start: Vec<f32>,
}

impl Layout {
    /// Ladders fanning upward from each throat at `angle_deg`. Siding index
    /// 0 on each side runs straight on; each further siding branches one
    /// `pitch` higher.
    pub fn ladder(yard: &Yard, car_len: f32, pitch: f32, angle_deg: f32) -> Layout {
        // Cells 0..=H+1: room for the loco at either end plus H cars.
        let main_len = (yard.headshunt as f32 + 2.0) * car_len;
        let main = Polyline::new(vec![P2::new(-main_len, 0.0), P2::new(0.0, 0.0)]);
        let tan = angle_deg.to_radians().tan();
        let buffer = 0.5 * car_len;

        let mut sidings = Vec::new();
        let mut siding_start = Vec::new();
        let mut rank = [0usize; 2];
        for sd in &yard.sidings {
            let (throat, dir, r) = match sd.side {
                Side::Right => (P2::new(0.0, 0.0), 1.0, &mut rank[1]),
                Side::Left => (P2::new(-main_len, 0.0), -1.0, &mut rank[0]),
            };
            let y = *r as f32 * pitch;
            *r += 1;
            let x0 = if tan > 0.0 { y / tan } else { 0.0 };
            let run = buffer + sd.capacity as f32 * car_len;
            let mut pts = vec![throat];
            if y > 0.0 {
                pts.push(P2::new(throat.x + dir * x0, y));
            }
            pts.push(P2::new(throat.x + dir * (x0 + run), y));
            let pl = Polyline::new(pts);
            siding_start.push(pl.length() - sd.capacity as f32 * car_len);
            sidings.push(pl);
        }

        let paths = yard
            .sidings
            .iter()
            .zip(&sidings)
            .map(|(sd, pl)| match sd.side {
                Side::Right => main.join(pl),
                Side::Left => main.reversed().join(pl),
            })
            .collect();

        let loop_track = yard.runaround.then(|| {
            // Sits one pitch above the main, clear of the (outward) ladders.
            let y = pitch;
            let dx = if tan > 0.0 { y / tan } else { 0.0 };
            Polyline::new(vec![
                P2::new(-main_len, 0.0),
                P2::new(-main_len + dx, y),
                P2::new(-dx, y),
                P2::new(0.0, 0.0),
            ])
        });

        Layout { car_len, main, sidings, paths, loop_track, siding_start }
    }

    pub fn main_len(&self) -> f32 {
        self.main.length()
    }

    /// The main oriented away from `side` (the loco's side), so `s = 0` is
    /// the loco's rest end.
    pub fn main_from(&self, side: Side) -> Polyline {
        match side {
            Side::Left => self.main.clone(),
            Side::Right => self.main.reversed(),
        }
    }

    pub fn throat(&self, side: Side) -> P2 {
        match side {
            Side::Left => self.main.pts[0],
            Side::Right => *self.main.pts.last().unwrap(),
        }
    }

    /// Loco path for a run-round starting from rest on `from`: out through
    /// the near throat, over the loop, arriving at the far throat.
    pub fn loop_path(&self, from: Side) -> Option<Polyline> {
        let lp = self.loop_track.as_ref()?;
        let rest = self.main_from(from).point_at(self.rest_s());
        let over = match from {
            Side::Left => lp.clone(),
            Side::Right => lp.reversed(),
        };
        let mut pts = vec![rest];
        pts.extend(over.pts.iter().copied());
        Some(Polyline::new(pts))
    }

    /// Path distance (along `paths[siding]`) of the centre of `slot`, where
    /// slot 0 is nearest the ladder and slot `capacity-1` is at the stop block.
    pub fn slot_s(&self, siding: usize, slot: usize) -> f32 {
        self.main_len() + self.siding_start[siding] + (slot as f32 + 0.5) * self.car_len
    }

    pub fn slot_pose(&self, siding: usize, slot: usize) -> (P2, f32) {
        let s = self.slot_s(siding, slot);
        (self.paths[siding].point_at(s), self.paths[siding].heading_at(s))
    }

    /// Loco rest position (its centre) measured from its own end of the main.
    pub fn rest_s(&self) -> f32 {
        0.5 * self.car_len
    }

    pub fn bounds(&self) -> (P2, P2) {
        let mut lo = P2::new(f32::MAX, f32::MAX);
        let mut hi = P2::new(f32::MIN, f32::MIN);
        let all = self
            .main
            .pts
            .iter()
            .chain(self.sidings.iter().flat_map(|s| s.pts.iter()))
            .chain(self.loop_track.iter().flat_map(|l| l.pts.iter()));
        for p in all {
            lo.x = lo.x.min(p.x);
            lo.y = lo.y.min(p.y);
            hi.x = hi.x.max(p.x);
            hi.y = hi.y.max(p.y);
        }
        (lo, hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polyline_point_and_length() {
        let pl = Polyline::new(vec![P2::new(0.0, 0.0), P2::new(3.0, 0.0), P2::new(3.0, 4.0)]);
        assert_eq!(pl.length(), 7.0);
        assert_eq!(pl.point_at(3.0), P2::new(3.0, 0.0));
        assert_eq!(pl.point_at(5.0), P2::new(3.0, 2.0));
        assert_eq!(pl.point_at(99.0), P2::new(3.0, 4.0));
        assert!((pl.heading_at(5.0) - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    }

    #[test]
    fn slots_are_spaced_by_car_len_and_end_at_stop_block() {
        for y in [Yard::inglenook(), Yard::timesaver()] {
            let l = Layout::ladder(&y, 40.0, 36.0, 30.0);
            for (i, sd) in y.sidings.iter().enumerate() {
                let last = l.slot_s(i, sd.capacity - 1);
                assert!((last + 20.0 - l.paths[i].length()).abs() < 1e-3, "{}: siding {i} last slot flush", y.name);
                assert!((l.slot_s(i, 1) - l.slot_s(i, 0) - 40.0).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn left_sidings_are_mirrored_and_paths_start_at_the_far_end() {
        let y = Yard::timesaver();
        let l = Layout::ladder(&y, 40.0, 36.0, 30.0);
        let (p, _) = l.slot_pose(2, 0); // "Long", left side
        assert!(p.x < -l.main_len(), "left siding lies beyond the left throat: {p:?}");
        assert_eq!(l.paths[2].pts[0], P2::new(0.0, 0.0), "left-siding path starts at the right end");
        assert_eq!(l.paths[0].pts[0], P2::new(-l.main_len(), 0.0));
        let lp = l.loop_path(Side::Left).unwrap();
        assert_eq!(*lp.pts.last().unwrap(), l.throat(Side::Right));
        let lp = l.loop_path(Side::Right).unwrap();
        assert_eq!(*lp.pts.last().unwrap(), l.throat(Side::Left));
        assert!(Layout::ladder(&Yard::inglenook(), 40.0, 36.0, 30.0).loop_path(Side::Left).is_none());
    }
}
