// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Анимация узлов 3D-моделей glTF и Collada, перенос `core/animation.py`
//! модуля.
//!
//! Анимация - ключи положения, поворота и масштаба узлов сцены (glTF
//! animations, Collada library_animations). Скиннинг и морфинг не
//! читаются: сетка со скелетом стоит в позе привязки.
//!
//! Часть модели, у которой движется свой узел или узел выше по дереву,
//! получает дорожку - номер вхождения сетки в сцену ([`crate::models::Part::track`]).
//! Дорожка помнит цепочку узлов от корня до сетки. На момент t матрица
//! дорожки - W(t) · W⁻¹, где W(t) - произведение местных матриц цепочки
//! на момент, W - то же в покое, в покое вершины части уже стоят. Итог
//! переводится в оси модели (метры, ось Z вверх) матрицей convert:
//! D' = C · D · C⁻¹.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use globe_core::{DMat3, DMat4, DVec3, DVec4};

/// Способ между ключами.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Linear,
    Step,
    /// CUBICSPLINE glTF: у ключа касательная входа, значение,
    /// касательная выхода.
    Cubic,
}

impl Mode {
    /// Способ по имени glTF, неизвестное - LINEAR.
    pub fn of(name: Option<&str>) -> Self {
        match name {
            Some("STEP") => Self::Step,
            Some("CUBICSPLINE") => Self::Cubic,
            _ => Self::Linear,
        }
    }
}

/// Номер последнего ключа не позже t, как searchsorted(side="right") - 1.
fn segment(times: &[f64], t: f64) -> usize {
    times.partition_point(|x| *x <= t).saturating_sub(1)
}

/// Значение ключей на момент t. times - по возрастанию, values - строки
/// ключей, у Cubic втрое больше: касательная входа, значение,
/// касательная выхода. До первого ключа - первое значение, после
/// последнего - последнее. None - ключей нет.
pub fn sample(times: &[f64], values: &[Vec<f64>], t: f64, mode: Mode) -> Option<Vec<f64>> {
    let cubic = mode == Mode::Cubic && values.len() == 3 * times.len();
    let key = |i: usize| if cubic { values.get(3 * i + 1) } else { values.get(i) };
    let (first, last) = (times.first()?, times.last()?);
    if t <= *first || times.len() == 1 {
        return key(0).cloned();
    }
    if t >= *last {
        return key(times.len() - 1).cloned();
    }
    let i = segment(times, t);
    let span = times.get(i + 1)? - times.get(i)?;
    let s = if span > 0.0 { (t - times.get(i)?) / span } else { 0.0 };
    if mode == Mode::Step {
        return key(i).cloned();
    }
    let (a, b) = (key(i)?, key(i + 1)?);
    if cubic {
        let m0 = values.get(3 * i + 2)?;
        let m1 = values.get(3 * (i + 1))?;
        let (s2, s3) = (s * s, s * s * s);
        let h = [2.0 * s3 - 3.0 * s2 + 1.0, s3 - 2.0 * s2 + s, -2.0 * s3 + 3.0 * s2, s3 - s2];
        return Some(
            (0..a.len().min(b.len()).min(m0.len()).min(m1.len()))
                .map(|n| h[0] * a[n] + h[1] * m0[n] * span + h[2] * b[n] + h[3] * m1[n] * span)
                .collect(),
        );
    }
    Some(a.iter().zip(b).map(|(x, y)| x + (y - x) * s).collect())
}

/// Сферическая интерполяция кватернионов (x, y, z, w).
pub fn slerp(a: DVec4, b: DVec4, s: f64) -> DVec4 {
    let (mut b, mut dot) = (b, a.dot(b));
    if dot < 0.0 {
        (b, dot) = (-b, -dot);
    }
    let out = if dot > 0.9995 {
        a + (b - a) * s
    } else {
        let angle = dot.min(1.0).acos();
        (a * ((1.0 - s) * angle).sin() + b * (s * angle).sin()) / angle.sin()
    };
    let length = out.length();
    if length > 0.0 { out / length } else { DVec4::new(0.0, 0.0, 0.0, 1.0) }
}

fn quat(row: &[f64]) -> DVec4 {
    match row {
        [x, y, z, w, ..] => DVec4::new(*x, *y, *z, *w),
        _ => DVec4::new(0.0, 0.0, 0.0, 1.0),
    }
}

/// Кватернион ключей на момент t: Linear - по дуге, прочие - как
/// [`sample`] с нормировкой.
pub fn sample_rotation(times: &[f64], values: &[Vec<f64>], t: f64, mode: Mode) -> Option<DVec4> {
    let (first, last) = (times.first()?, times.last()?);
    if mode != Mode::Linear || times.len() < 2 || t <= *first || t >= *last {
        let out = quat(&sample(times, values, t, mode)?);
        let length = out.length();
        return Some(if length > 0.0 { out / length } else { out });
    }
    let i = segment(times, t);
    let span = times.get(i + 1)? - times.get(i)?;
    let s = if span > 0.0 { (t - times.get(i)?) / span } else { 0.0 };
    Some(slerp(quat(values.get(i)?), quat(values.get(i + 1)?), s))
}

/// Матрица 3×3 поворота кватерниона (x, y, z, w).
pub fn quaternion_matrix(q: DVec4) -> DMat3 {
    let DVec4 { x, y, z, w } = q;
    DMat3::from_cols(
        DVec3::new(1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + z * w), 2.0 * (x * z - y * w)),
        DVec3::new(2.0 * (x * y - z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + x * w)),
        DVec3::new(2.0 * (x * z + y * w), 2.0 * (y * z - x * w), 1.0 - 2.0 * (x * x + y * y)),
    )
}

/// Матрица 4×4 из сдвига, кватерниона и масштаба, как узел glTF.
pub fn trs(translation: DVec3, rotation: DVec4, scale: DVec3) -> DMat4 {
    let mut out = DMat4::from_mat3(quaternion_matrix(rotation) * DMat3::from_diagonal(scale));
    out.w_axis = translation.extend(1.0);
    out
}

/// Местная матрица узла на момент: узел, секунды.
pub type Local = Arc<dyn Fn(usize, f64) -> DMat4 + Send + Sync>;

/// Анимация модели: длительность и матрицы дорожек на момент.
#[derive(Clone)]
pub struct Animation {
    pub duration: f64,
    local: Local,
    /// Дорожка - узлы от корня до сетки.
    pub chains: BTreeMap<usize, Vec<usize>>,
    /// Дорожка - W⁻¹ в покое.
    pub rest: BTreeMap<usize, DMat4>,
    convert: DMat4,
    back: DMat4,
}

impl fmt::Debug for Animation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Animation").field("duration", &self.duration).field("tracks", &self.chains.len()).finish()
    }
}

impl Animation {
    /// convert - матрица из осей файла в оси модели ([`scaled_convert`]).
    pub fn new(duration: f64, local: Local, chains: BTreeMap<usize, Vec<usize>>, rest: BTreeMap<usize, DMat4>, convert: DMat4) -> Self {
        let back = if convert.determinant().abs() > 0.0 { convert.inverse() } else { DMat4::IDENTITY };
        Self { duration, local, chains, rest, convert, back }
    }

    pub fn tracks(&self) -> Vec<usize> {
        self.chains.keys().copied().collect()
    }

    /// Дорожка - матрица 4×4 в осях модели на момент t секунд от
    /// начала, время идёт по кругу длительности.
    pub fn deltas(&self, t: f64) -> BTreeMap<usize, DMat4> {
        let t = if self.duration > 0.0 { t.rem_euclid(self.duration) } else { t };
        self.chains
            .iter()
            .map(|(track, chain)| {
                let world = chain.iter().fold(DMat4::IDENTITY, |m, node| m * (self.local)(*node, t));
                let rest = self.rest.get(track).copied().unwrap_or(DMat4::IDENTITY);
                (*track, self.convert * world * rest * self.back)
            })
            .collect()
    }
}

/// Матрица из осей файла в оси модели: единица в метры и, у файла
/// с осью Y вверх, поворот как [`crate::models::from_y_up`].
pub fn scaled_convert(unit: f64, y_up: bool) -> DMat4 {
    let out = DMat4::from_diagonal(DVec4::new(unit, unit, unit, 1.0));
    if y_up {
        let turn = DMat4::from_cols(DVec4::X, DVec4::new(0.0, 0.0, 1.0, 0.0), DVec4::new(0.0, -1.0, 0.0, 0.0), DVec4::W);
        turn * out
    } else {
        out
    }
}

/// (линейная часть 3×3, сдвиг) матрицы дорожки в смещениях ECEF от
/// точки модели. frame - матрица [`crate::models::frame`].
pub fn frame_delta(frame: &DMat3, delta: &DMat4) -> (DMat3, DVec3) {
    let back = if frame.determinant().abs() > 0.0 { frame.inverse() } else { DMat3::IDENTITY };
    (*frame * DMat3::from_mat4(*delta) * back, *frame * delta.w_axis.truncate())
}
