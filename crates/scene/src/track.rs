// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Поза модели, что едет по gx:Track, перенос `model_pose` из
//! `core/track.py` модуля.
//!
//! Между точками всё идёт линейно по времени, азимут - по кратчайшему
//! повороту. Без gx:angles азимут - направление движения, наклон и крен
//! нулевые. Вне времени трека модели нет.

use crate::overlays::TrackRow;
use crate::tour::bearing;

/// Поза модели в момент: широта, долгота, высота, азимут, наклон, крен.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPose {
    pub lat: f64,
    pub lon: f64,
    pub alt: f64,
    pub heading: f64,
    pub tilt: f64,
    pub roll: f64,
}

/// Поза модели на момент t, секунды UTC. rows - по возрастанию времени.
/// None - t вне времени трека.
pub fn model_pose(rows: &[TrackRow], t: f64) -> Option<ModelPose> {
    let (first, last) = (rows.first()?, rows.last()?);
    if !(t >= first.t && t <= last.t) {
        return None;
    }
    // Последняя точка не позже t, как searchsorted(side="right") - 1.
    let i = rows.partition_point(|r| r.t <= t).saturating_sub(1).min(rows.len() - 1);
    let j = (i + 1).min(rows.len() - 1);
    let (a, b) = (&rows[i], &rows[j]);
    let span = b.t - a.t;
    let k = if span > 0.0 { (t - a.t) / span } else { 0.0 };
    let mix = |x: f64, y: f64| x + (y - x) * k;
    let (lat, lon, alt) = (mix(a.lat, b.lat), mix(a.lon, b.lon), mix(a.alt, b.alt));
    match (a.heading, b.heading) {
        (Some(ha), Some(hb)) => {
            let turn = (hb - ha + 180.0).rem_euclid(360.0) - 180.0;
            Some(ModelPose { lat, lon, alt, heading: (ha + turn * k).rem_euclid(360.0), tilt: mix(a.tilt, b.tilt), roll: mix(a.roll, b.roll) })
        }
        _ => {
            // В последней точке направление - по последнему отрезку.
            let (p, q) = if j != i { (a, b) } else { (&rows[i.saturating_sub(1)], a) };
            let heading = if (p.lat, p.lon) != (q.lat, q.lon) { bearing(p.lat, p.lon, q.lat, q.lon) } else { 0.0 };
            Some(ModelPose { lat, lon, alt, heading, tilt: 0.0, roll: 0.0 })
        }
    }
}
