//! AzerothCore `MO_TRANSPORT` timetable and stateless cycle sampler in WoW coordinates.
//! `TransportMgr::GeneratePath` supplies the period and keyframe times; `MotionTransport::Update`
//! samples `PathProgress % pathTime`. The wire progress is transport elapsed time, not uptime.

use std::f32::consts::PI;

use crate::taxi::TaxiPathNode;

/// A transport's position and heading at one instant of its cycle, in WoW coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransportSample {
    /// The map of `pos`, which changes as a transport crosses continents.
    pub map: u32,
    pub pos: [f32; 3],
    /// Radians in `[0, 2π)`, `atan2(dir.y, dir.x) + π` (`MotionTransport::CalculateSegmentPos`).
    pub heading: f32,
    /// False while parked at a stop or on a keyframe's zero-width instant.
    pub moving: bool,
}

/// A keyframe that survives the map-change skip, with what `sample`
/// needs. Stop and teleport are implicit: a non-stop window is zero-width, and a teleport's
/// `next_dist_from_prev` is 0.
#[derive(Debug, Clone)]
struct Frame {
    map_id: u32,
    pos: [f32; 3],
    initial_orientation: f32,
    dist_since_stop: f32,
    dist_until_stop: f32,
    /// Arc length to the next frame; 0 for a teleport frame and the last frame.
    next_dist_from_prev: f32,
    time_from: f32,
    time_to: f32,
    departure_time: u32,
    next_arrive_time: u32,
    /// The [`Leg`] and control index of this frame's outbound segment.
    leg: usize,
    local_index: usize,
}

/// A kept node before the derived quantities.
#[derive(Debug, Clone, Copy)]
struct RawFrame {
    map_id: u32,
    pos: [f32; 3],
    is_stop: bool,
    delay_secs: f32,
    initial_orientation: f32,
    teleport: bool,
}

/// A run of keyframes ending at a teleport or the path's end: a non-cyclic Catmull-Rom spline
/// (`InitCatmullRom`).
#[derive(Debug, Clone)]
struct Leg {
    controls: Vec<[f32; 3]>,
}

impl Leg {
    /// AC `SplineBase::InitCatmullRom`: orientation defaults to zero, so the leading
    /// virtual point is one yard west; the trailing point duplicates the last control.
    fn control(&self, i: isize) -> [f32; 3] {
        let n = self.controls.len();
        if i < 0 {
            let [x, y, z] = self.controls[0];
            [x - 1.0, y, z]
        } else if (i as usize) >= n {
            self.controls[n - 1]
        } else {
            self.controls[i as usize]
        }
    }

    /// Position on segment `k` at `t` in `[0, 1]` (`SplineBase::EvaluateCatmullRom`).
    fn evaluate_percent(&self, k: usize, t: f32) -> [f32; 3] {
        catmull_rom_eval(
            self.control(k as isize - 1),
            self.control(k as isize),
            self.control(k as isize + 1),
            self.control(k as isize + 2),
            t,
        )
    }

    /// Derivative on segment `k` at `t` (`SplineBase::EvaluateDerivativeCatmullRom`).
    fn evaluate_derivative(&self, k: usize, t: f32) -> [f32; 3] {
        catmull_rom_derivative(
            self.control(k as isize - 1),
            self.control(k as isize),
            self.control(k as isize + 1),
            self.control(k as isize + 2),
            t,
        )
    }

    /// Arc length of segment `k` over `steps` chords, summed in `f64` as
    /// `SplineBase::SegLengthCatmullRom` does (`spline.cpp:159-177`).
    fn seg_length(&self, k: usize, steps: u32) -> f64 {
        let mut cur = self.control(k as isize); // t=0 evaluates to exactly controls[k]
        let mut total = 0.0f64;
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let next = self.evaluate_percent(k, t);
            let dx = next[0] - cur[0];
            let dy = next[1] - cur[1];
            let dz = next[2] - cur[2];
            // G3D Vector3::length returns float, before AC's double chord sum.
            total += f64::from((dx * dx + dy * dy + dz * dz).sqrt());
            cur = next;
        }
        total
    }
}

/// AzerothCore's `s_catmullRomCoeffs` (`spline.cpp:61-65`) expanded as `t³ t² t 1` times the matrix:
/// the Catmull-Rom blending polynomials, τ = 0.5.
fn catmull_rom_weights(t: f32) -> [f32; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [
        -0.5 * t3 + t2 - 0.5 * t,
        1.5 * t3 - 2.5 * t2 + 1.0,
        -1.5 * t3 + 2.0 * t2 + 0.5 * t,
        0.5 * t3 - 0.5 * t2,
    ]
}

/// `d/dt` of [`catmull_rom_weights`]: `C_Evaluate_Derivative`'s `3t² 2t 1 0` times the matrix.
fn catmull_rom_deriv_weights(t: f32) -> [f32; 4] {
    let t2 = t * t;
    [
        -1.5 * t2 + 2.0 * t - 0.5,
        4.5 * t2 - 5.0 * t,
        -4.5 * t2 + 4.0 * t + 0.5,
        1.5 * t2 - t,
    ]
}

fn blend(p0: [f32; 3], p1: [f32; 3], p2: [f32; 3], p3: [f32; 3], w: [f32; 4]) -> [f32; 3] {
    [
        p0[0] * w[0] + p1[0] * w[1] + p2[0] * w[2] + p3[0] * w[3],
        p0[1] * w[0] + p1[1] * w[1] + p2[1] * w[2] + p3[1] * w[3],
        p0[2] * w[0] + p1[2] * w[1] + p2[2] * w[2] + p3[2] * w[3],
    ]
}

fn catmull_rom_eval(p0: [f32; 3], p1: [f32; 3], p2: [f32; 3], p3: [f32; 3], t: f32) -> [f32; 3] {
    blend(p0, p1, p2, p3, catmull_rom_weights(t))
}

fn catmull_rom_derivative(
    p0: [f32; 3],
    p1: [f32; 3],
    p2: [f32; 3],
    p3: [f32; 3],
    t: f32,
) -> [f32; 3] {
    blend(p0, p1, p2, p3, catmull_rom_deriv_weights(t))
}

/// Wrap to `[0, 2π)`, as `Geometry::NormalizeOrientation`.
fn normalize_orientation(o: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let wrapped = o % tau;
    if wrapped < 0.0 {
        wrapped + tau
    } else {
        wrapped
    }
}

/// AC `GeneratePath`: float seconds accumulated across frames, truncated to ms per frame.
fn accumulate_azerothcore(
    raws: &[RawFrame],
    time_to: &[f32],
) -> (Vec<u32>, Vec<u32>, Vec<u32>, u32) {
    let n = raws.len();
    let mut arrive = vec![0u32; n];
    let mut depart = vec![0u32; n];
    let mut next_arrive = vec![0u32; n];

    let mut cur_path_time = 0.0f32;
    if raws[0].is_stop {
        cur_path_time = raws[0].delay_secs;
        depart[0] = (cur_path_time * 1000.0) as u32;
    }
    for i in 1..n {
        cur_path_time += time_to[i - 1];
        if raws[i].is_stop {
            arrive[i] = (cur_path_time * 1000.0) as u32;
            next_arrive[i - 1] = arrive[i];
            cur_path_time += raws[i].delay_secs;
            depart[i] = (cur_path_time * 1000.0) as u32;
        } else {
            cur_path_time -= time_to[i];
            arrive[i] = (cur_path_time * 1000.0) as u32;
            next_arrive[i - 1] = arrive[i];
            depart[i] = arrive[i];
        }
    }
    next_arrive[n - 1] = depart[n - 1];
    let period = depart[n - 1];
    (arrive, depart, next_arrive, period)
}

/// A `MO_TRANSPORT`'s cyclic timetable, built once per `taxiPathId`.
#[derive(Debug, Clone)]
pub struct TransportTimetable {
    /// Cycle length in ms; `progress % period_ms` is the position in the loop.
    pub period_ms: u32,
    frames: Vec<Frame>,
    legs: Vec<Leg>,
    move_speed: f32,
    accel_rate: f32,
    accel_time: f32,
    accel_dist: f32,
}

impl TransportTimetable {
    /// Build from `TaxiPathNode` rows sorted by `node_index` and the `gameobject_template`
    /// `data1`/`data2` (`moveSpeed`, `accelRate`). The first and last nodes are never travelled,
    /// so fewer than 3 is `None`.
    pub fn build(nodes: &[TaxiPathNode], move_speed: f32, accel_rate: f32) -> Option<Self> {
        Self::build_native(nodes, move_speed, accel_rate)
    }

    /// The arrival instant of the first frame on `map_id`, scanning from the frame `from_cycle_ms`
    /// is in and wrapping. A worldport places a rider with it until the server re-anchors the
    /// transport's clock, a round trip after the ack (`Map::SendInitSelf`).
    pub fn first_cycle_on_map(&self, from_cycle_ms: u32, map_id: u32) -> Option<u32> {
        let n = self.frames.len();
        if n == 0 {
            return None;
        }
        // Start at the current frame: a clock already on the map answers here.
        let start = self
            .frames
            .iter()
            .position(|f| from_cycle_ms < f.next_arrive_time)
            .unwrap_or(n - 1);
        (0..n)
            .map(|k| (start + k) % n)
            .find(|&i| self.frames[i].map_id == map_id)
            .map(|i| match i {
                0 => 0,
                _ => self.frames[i - 1].next_arrive_time,
            })
    }

    /// Whether any keyframe is on `map_id`; a worldport keeps such a transport across the switch.
    pub fn touches_map(&self, map_id: u32) -> bool {
        self.frames.iter().any(|f| f.map_id == map_id)
    }

    /// AC `TransportMgr::GeneratePath`, including its spline length precision.
    fn build_native(nodes: &[TaxiPathNode], move_speed: f32, accel_rate: f32) -> Option<Self> {
        let n_raw = nodes.len();
        if n_raw < 3
            || !move_speed.is_finite()
            || move_speed <= 0.0
            || !accel_rate.is_finite()
            || accel_rate <= 0.0
        {
            return None;
        }

        // AC skips the teleport guard and the following node before removing end guards.
        let mut raws: Vec<RawFrame> = Vec::new();
        let mut map_change = false;
        for i in 0..n_raw {
            if map_change {
                map_change = false;
                continue;
            }
            let node = &nodes[i];
            if i + 1 < n_raw && (node.flags & 1 != 0 || node.map_id != nodes[i + 1].map_id) {
                if let Some(last) = raws.last_mut() {
                    last.teleport = true;
                }
                map_change = true;
            } else {
                // AzerothCore's whole-path spline derivative at t = 0 is this central difference of
                // the raw neighbours, kept or not.
                let prev = nodes[i.saturating_sub(1)].pos;
                let nxt = nodes[(i + 1).min(n_raw - 1)].pos;
                let initial_orientation =
                    normalize_orientation((nxt[1] - prev[1]).atan2(nxt[0] - prev[0]) + PI);
                raws.push(RawFrame {
                    map_id: node.map_id,
                    pos: node.pos,
                    // AC KeyFrame::IsStopFrame tests equality, not a bit mask.
                    is_stop: node.flags == 2,
                    delay_secs: node.delay as f32,
                    initial_orientation,
                    teleport: false,
                });
            }
        }
        // AC removes the first/last *kept* guard points after the teleport skip.
        if raws.len() >= 2 {
            raws.remove(0);
            raws.pop();
        }
        if raws.is_empty() {
            return None;
        }
        // The last frame always teleports, even on a closed path (`GeneratePath`).
        raws.last_mut().unwrap().teleport = true;
        let n = raws.len();

        // Phase B: a new leg starts after every teleport frame.
        let mut leg_of = vec![0usize; n];
        let mut local_index_of = vec![0usize; n];
        let mut leg_controls: Vec<Vec<[f32; 3]>> = vec![Vec::new()];
        {
            let mut leg = 0usize;
            for j in 0..n {
                if j > 0 && raws[j - 1].teleport {
                    leg += 1;
                    leg_controls.push(Vec::new());
                }
                leg_of[j] = leg;
                local_index_of[j] = leg_controls[leg].len();
                leg_controls[leg].push(raws[j].pos);
            }
        }
        let legs: Vec<Leg> = leg_controls
            .into_iter()
            .map(|controls| Leg { controls })
            .collect();

        // Phase D: `DistFromPrev`, `NextDistFromPrev`.
        let mut dist_from_prev = vec![0.0f32; n];
        // AC TransportSpline stores cumulative lengths in double, then narrows each difference.
        for (leg_id, leg) in legs.iter().enumerate() {
            let mut cumulative = 0.0f64;
            for j in 0..n {
                if leg_of[j] == leg_id && local_index_of[j] > 0 {
                    let previous = cumulative;
                    cumulative += f64::from(leg.seg_length(
                        local_index_of[j] - 1,
                        crate::transport_period::AC_CHORDS_PER_SEGMENT,
                    ) as f32);
                    dist_from_prev[j] = (cumulative - previous) as f32;
                }
            }
        }
        let mut next_dist_from_prev = vec![0.0f32; n];
        for j in 0..n {
            next_dist_from_prev[j] = if raws[j].teleport || j == n - 1 {
                0.0
            } else {
                dist_from_prev[j + 1]
            };
        }

        // `firstStop`, `lastStop`.
        let mut first_stop: Option<usize> = None;
        let mut last_stop: Option<usize> = None;
        for (j, r) in raws.iter().enumerate() {
            if r.is_stop {
                first_stop.get_or_insert(j);
                last_stop = Some(j);
            }
        }
        let (first_stop, last_stop) = (first_stop.unwrap_or(0), last_stop.unwrap_or(0));

        // Phase E: `DistSinceStop`, `DistUntilStop`.
        let mut dist_since_stop = vec![0.0f32; n];
        let mut tmp = 0.0f32;
        for i in 0..n {
            let j = (i + last_stop) % n;
            tmp = if raws[j].is_stop || j == last_stop {
                0.0
            } else {
                tmp + dist_from_prev[j]
            };
            dist_since_stop[j] = tmp;
        }
        let mut dist_until_stop = vec![0.0f32; n];
        let mut tmp = 0.0f32;
        for i in (0..n).rev() {
            let j = (i + first_stop) % n;
            tmp += dist_from_prev[(j + 1) % n];
            dist_until_stop[j] = tmp;
            if raws[j].is_stop || j == first_stop {
                tmp = 0.0;
            }
        }

        // Phase F: `TimeTo`, the four-regime trapezoid.
        let accel_dist = 0.5 * move_speed * move_speed / accel_rate;
        let accel_time = move_speed / accel_rate;
        let mut time_to = vec![0.0f32; n];
        for j in 0..n {
            let (since, until) = (dist_since_stop[j], dist_until_stop[j]);
            let total = since + until;
            time_to[j] = if total < 2.0 * accel_dist {
                if since < until {
                    let segment_time = 2.0 * ((until + since) / accel_rate).sqrt();
                    segment_time - (2.0 * since / accel_rate).sqrt()
                } else {
                    (2.0 * until / accel_rate).sqrt()
                }
            } else if since < accel_dist {
                let segment_time = (until + since) / move_speed + (move_speed / accel_rate);
                segment_time - (2.0 * since / accel_rate).sqrt()
            } else if until < accel_dist {
                (2.0 * until / accel_rate).sqrt()
            } else {
                (until / move_speed) + (0.5 * move_speed / accel_rate)
            };
        }

        // Phase G: `TimeFrom`.
        let mut time_from = vec![0.0f32; n];
        let mut segment_time = 0.0f32;
        for i in 0..n {
            let j = (i + last_stop) % n;
            if raws[j].is_stop || j == last_stop {
                segment_time = time_to[j];
            }
            time_from[j] = segment_time - time_to[j];
        }

        // AC pathTime is the final DepartureTime, with no client/sniffed period override.
        let (_arrive_time, departure_time, next_arrive_time, period_ms) =
            accumulate_azerothcore(&raws, &time_to);
        if period_ms == 0 {
            return None;
        }

        let frames: Vec<Frame> = (0..n)
            .map(|j| Frame {
                map_id: raws[j].map_id,
                pos: raws[j].pos,
                initial_orientation: raws[j].initial_orientation,
                dist_since_stop: dist_since_stop[j],
                dist_until_stop: dist_until_stop[j],
                next_dist_from_prev: next_dist_from_prev[j],
                time_from: time_from[j],
                time_to: time_to[j],
                departure_time: departure_time[j],
                next_arrive_time: next_arrive_time[j],
                leg: leg_of[j],
                local_index: local_index_of[j],
            })
            .collect();

        Some(TransportTimetable {
            period_ms,
            frames,
            legs,
            move_speed,
            accel_rate,
            accel_time,
            accel_dist,
        })
    }

    /// Position and heading at `cycle_ms` (`progress % period_ms`), as `MotionTransport::Update` and
    /// `CalculateSegmentPos` compute them (`MotionTransport::Update`), with no state between calls.
    pub fn sample(&self, cycle_ms: u32) -> TransportSample {
        let n = self.frames.len();
        if n == 0 {
            return TransportSample {
                map: 0,
                pos: [0.0; 3],
                heading: 0.0,
                moving: false,
            };
        }
        let cycle_ms = cycle_ms % self.period_ms;

        // The frame whose `[ArriveTime, NextArriveTime)`, stop then travel, holds `cycle_ms`.
        let idx = self
            .frames
            .iter()
            .position(|f| cycle_ms < f.next_arrive_time)
            .unwrap_or(n - 1);
        let frame = &self.frames[idx];

        if cycle_ms < frame.departure_time || frame.next_dist_from_prev <= 0.0 {
            // At a stop, on a keyframe's instant, or on a teleport's zero-length travel.
            return TransportSample {
                map: frame.map_id,
                pos: frame.pos,
                heading: frame.initial_orientation,
                moving: false,
            };
        }

        // Moving: distance from the nearer stop under the same trapezoid as `TimeTo`
        // (`MotionTransport::CalculateSegmentPos`).
        let now = cycle_ms as f32 * 0.001;
        let since_departure = now - frame.departure_time as f32 * 0.001;
        let time_since_stop = frame.time_from + since_departure;
        let time_until_stop = frame.time_to - since_departure;
        let dist = if time_since_stop < time_until_stop {
            let d = if time_since_stop < self.accel_time {
                0.5 * self.accel_rate * time_since_stop * time_since_stop
            } else {
                self.accel_dist + (time_since_stop - self.accel_time) * self.move_speed
            };
            d - frame.dist_since_stop
        } else {
            let d = if time_until_stop < self.accel_time {
                0.5 * self.accel_rate * time_until_stop * time_until_stop
            } else {
                self.accel_dist + (time_until_stop - self.accel_time) * self.move_speed
            };
            frame.dist_until_stop - d
        };
        let t = dist / frame.next_dist_from_prev;

        let leg = &self.legs[frame.leg];
        let pos = leg.evaluate_percent(frame.local_index, t);
        let dir = leg.evaluate_derivative(frame.local_index, t);
        let heading = normalize_orientation(dir[1].atan2(dir[0]) + PI);

        TransportSample {
            map: frame.map_id,
            pos,
            heading,
            moving: true,
        }
    }
}

#[cfg(test)]
mod tests;
