//! Geometry for drawing a yard: polylines in a plain 2D space, no engine
//! types. The throat (where the headshunt meets the ladder) is the origin.
//!
//! Every siding gets a *path*: headshunt far end → throat → siding stop block.
//! A vehicle string on the move is described by one scalar `s` along the
//! active path; `s = 0` is the far end of the headshunt.

use crate::yard::Yard;

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
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub car_len: f32,
    /// Far end → throat.
    pub headshunt: Polyline,
    /// Throat → stop block, one per siding.
    pub sidings: Vec<Polyline>,
    /// `headshunt.join(sidings[i])`.
    pub paths: Vec<Polyline>,
    /// Distance from the start of the siding polyline to the first slot's
    /// leading edge (i.e. the diagonal plus a little buffer).
    siding_start: Vec<f32>,
}

impl Layout {
    /// A ladder fanning upward from the throat at `angle_deg`. Siding 0 runs
    /// straight on; each further siding branches one `pitch` higher.
    pub fn ladder(yard: &Yard, car_len: f32, pitch: f32, angle_deg: f32) -> Layout {
        let head_len = (yard.headshunt as f32 + 1.5) * car_len;
        let headshunt = Polyline::new(vec![P2::new(-head_len, 0.0), P2::new(0.0, 0.0)]);
        let tan = angle_deg.to_radians().tan();
        let buffer = 0.5 * car_len;
        let mut sidings = Vec::new();
        let mut siding_start = Vec::new();
        for (i, sd) in yard.sidings.iter().enumerate() {
            let y = i as f32 * pitch;
            let x0 = if tan > 0.0 { y / tan } else { 0.0 };
            let run = buffer + sd.capacity as f32 * car_len;
            let mut pts = vec![P2::new(0.0, 0.0)];
            if i > 0 {
                pts.push(P2::new(x0, y));
            }
            pts.push(P2::new(x0 + run, y));
            let pl = Polyline::new(pts);
            siding_start.push(pl.length() - sd.capacity as f32 * car_len);
            sidings.push(pl);
        }
        let paths = sidings.iter().map(|s| headshunt.join(s)).collect();
        Layout { car_len, headshunt, sidings, paths, siding_start }
    }

    pub fn head_len(&self) -> f32 {
        self.headshunt.length()
    }

    /// Path distance (along `paths[siding]`) of the centre of `slot`, where
    /// slot 0 is nearest the ladder and slot `capacity-1` is at the stop block.
    pub fn slot_s(&self, siding: usize, slot: usize) -> f32 {
        self.head_len() + self.siding_start[siding] + (slot as f32 + 0.5) * self.car_len
    }

    pub fn slot_pose(&self, siding: usize, slot: usize) -> (P2, f32) {
        let s = self.slot_s(siding, slot);
        (self.paths[siding].point_at(s), self.paths[siding].heading_at(s))
    }

    /// Loco rest position on the headshunt (its centre), string trailing
    /// toward the throat.
    pub fn rest_s(&self) -> f32 {
        0.5 * self.car_len
    }

    pub fn bounds(&self) -> (P2, P2) {
        let mut lo = P2::new(f32::MAX, f32::MAX);
        let mut hi = P2::new(f32::MIN, f32::MIN);
        for p in self.headshunt.pts.iter().chain(self.sidings.iter().flat_map(|s| s.pts.iter())) {
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
        let y = Yard::inglenook();
        let l = Layout::ladder(&y, 40.0, 36.0, 30.0);
        for (i, sd) in y.sidings.iter().enumerate() {
            let last = l.slot_s(i, sd.capacity - 1);
            assert!((last + 20.0 - l.paths[i].length()).abs() < 1e-3, "siding {i} last slot flush");
            assert!((l.slot_s(i, 1) - l.slot_s(i, 0) - 40.0).abs() < 1e-3);
        }
    }
}
