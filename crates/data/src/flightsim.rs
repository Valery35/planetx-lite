// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Режим полёта: свой самолёт с клавиатуры, как имитатор полёта Google
//! Earth Pro. Перенос `core/flightsim.py` модуля.
//!
//! Самолёт - точка, высота над уровнем моря, курс, тангаж, крен, скорость
//! и тяга. Модель простая, без аэродинамики:
//! - крен меняется клавишами со скоростью roll_rate вида, без клавиш
//!   выходит к нулю со скоростью [`ROLL_RETURN`], не больше roll_max;
//! - поворот - координированный, угловая скорость g · tg(крен) / скорость;
//! - тангаж меняется клавишами со скоростью pitch_rate и держится без
//!   клавиш, не больше pitch_max;
//! - скорость растёт от тяги (thrust при полной), падает от сопротивления
//!   (наибольшая скорость при полной тяге - max_speed) и от подъёма
//!   g · sin(тангаж);
//! - ниже stall нос опускается сам со скоростью [`STALL_DROP`];
//! - касание поверхности кончает полёт.
//!
//! Самолёт двух видов ([`kind_spec`]), как в имитаторе Google Earth Pro, -
//! истребитель и винтовой, без вида - средний [`MIDDLE`]. Величины - выбор
//! помощника модуля, утверждает автор.

pub const G: f64 = 9.81;
/// Наибольшая скорость при полной тяге, м/с, около 1000 км/ч.
pub const MAX_SPEED: f64 = 280.0;
/// Ускорение при полной тяге, м/с².
pub const THRUST: f64 = 12.0;
pub const DRAG: f64 = THRUST / (MAX_SPEED * MAX_SPEED);
/// Скорость сваливания, м/с, около 100 км/ч.
pub const STALL: f64 = 28.0;
/// Опускание носа при сваливании, °/с.
pub const STALL_DROP: f64 = 15.0;
/// Крен, °/с, выход к нулю, °/с, и предел, °.
pub const ROLL_RATE: f64 = 45.0;
pub const ROLL_RETURN: f64 = 20.0;
pub const ROLL_MAX: f64 = 60.0;
/// Тангаж, °/с, и предел, °.
pub const PITCH_RATE: f64 = 12.0;
pub const PITCH_MAX: f64 = 25.0;
/// Изменение тяги, доля в секунду.
pub const THROTTLE_RATE: f64 = 0.4;
/// Высота старта над поверхностью, м.
pub const START_HEIGHT: f64 = 1000.0;
/// Скорость старта, м/с.
pub const START_SPEED: f64 = 500.0 / 3.6;
/// Шаг расчёта, с, длиннее шаг делится.
pub const STEP_MAX: f64 = 0.1;
/// Шаг длиннее не считается: враждебная пауза не вешает цикл, секунды.
pub const DT_MAX: f64 = 3600.0;
/// Делений ленты прибора не больше.
pub const TAPE_MAX: usize = 10000;
/// Большая полуось WGS84 - радиус по умолчанию.
pub const EARTH_RADIUS: f64 = 6378137.0;

/// Виды самолёта.
pub const FIGHTER: &str = "fighter";
pub const LIGHT: &str = "light";

/// Вид самолёта: наибольшая скорость (м/с), тяга (м/с²), сопротивление,
/// скорость сваливания, крен и тангаж (°/с и предел), старт (м/с),
/// длина (м).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spec {
    pub max_speed: f64,
    pub thrust: f64,
    pub drag: f64,
    pub stall: f64,
    pub roll_rate: f64,
    pub roll_max: f64,
    pub pitch_rate: f64,
    pub pitch_max: f64,
    pub start: f64,
    pub length: f64,
}

impl Spec {
    /// Вид по величинам, сопротивление - из тяги и наибольшей скорости.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(max_speed: f64, thrust: f64, stall: f64, roll_rate: f64, roll_max: f64, pitch_rate: f64, pitch_max: f64, start: f64, length: f64) -> Self {
        Self { max_speed, thrust, drag: thrust / (max_speed * max_speed), stall, roll_rate, roll_max, pitch_rate, pitch_max, start, length }
    }
}

/// Истребитель: до 2000 км/ч, старт 900 км/ч, крен 90°/с.
pub const FIGHTER_SPEC: Spec = Spec::new(555.0, 30.0, 70.0, 90.0, 80.0, 20.0, 35.0, 250.0, 15.0);
/// Винтовой четырёхместный: до 340 км/ч, старт 250 км/ч.
pub const LIGHT_SPEC: Spec = Spec::new(95.0, 4.0, 28.0, 45.0, 60.0, 12.0, 25.0, 250.0 / 3.6, 8.0);
/// Средний самолёт без вида.
pub const MIDDLE: Spec = Spec::new(MAX_SPEED, THRUST, STALL, ROLL_RATE, ROLL_MAX, PITCH_RATE, PITCH_MAX, START_SPEED, 16.0);
/// Виды по именам, как `KINDS` модуля.
pub const KINDS: [(&str, Spec); 2] = [(FIGHTER, FIGHTER_SPEC), (LIGHT, LIGHT_SPEC)];

/// Вид по имени, неизвестное имя - [`MIDDLE`].
pub fn kind_spec(kind: Option<&str>) -> Spec {
    KINDS.iter().find(|(k, _)| Some(*k) == kind).map_or(MIDDLE, |(_, s)| *s)
}

/// Тяга ровного полёта на скорости speed.
pub fn cruise_throttle(speed: f64, spec: &Spec) -> f64 {
    (speed / spec.max_speed).powi(2).clamp(0.0, 1.0)
}

/// Состояние самолёта: широта, долгота, высота над уровнем моря (м), курс,
/// тангаж, крен (°), скорость (м/с), тяга 0..1, подъём (м/с). radius -
/// радиус шара, по которому идёт самолёт.
#[derive(Debug, Clone, PartialEq)]
pub struct Plane {
    pub spec: Spec,
    pub kind: Option<String>,
    pub lat: f64,
    pub lon: f64,
    pub alt: f64,
    pub heading: f64,
    pub pitch: f64,
    pub roll: f64,
    pub speed: f64,
    pub throttle: f64,
    pub climb: f64,
    pub crashed: bool,
    pub radius: f64,
}

impl Plane {
    /// Самолёт в точке на высоте alt с курсом heading. speed None -
    /// скорость старта вида, kind - имя вида или None.
    pub fn new(lat: f64, lon: f64, alt: f64, heading: f64, speed: Option<f64>, kind: Option<&str>) -> Self {
        let spec = kind_spec(kind);
        let speed = speed.unwrap_or(spec.start);
        Self {
            spec,
            kind: kind.map(str::to_string),
            lat,
            lon,
            alt,
            heading: heading.rem_euclid(360.0),
            pitch: 0.0,
            roll: 0.0,
            speed,
            throttle: cruise_throttle(speed, &spec),
            climb: 0.0,
            crashed: false,
            radius: EARTH_RADIUS,
        }
    }
}

/// Нажатые клавиши: pitch +1 нос вверх, -1 вниз; roll +1 крен вправо, -1
/// влево; throttle +1 больше тяги, -1 меньше.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Controls {
    pub pitch: f64,
    pub roll: f64,
    pub throttle: f64,
}

/// Точка на шаре радиуса radius на расстоянии metres по азимуту heading,
/// градусы, как `destination` в `core/trains.py` модуля.
pub fn destination(lat: f64, lon: f64, heading: f64, metres: f64, radius: f64) -> (f64, f64) {
    let d = metres / radius;
    let (p1, l1, h) = (lat.to_radians(), lon.to_radians(), heading.to_radians());
    let p2 = (p1.sin() * d.cos() + p1.cos() * d.sin() * h.cos()).clamp(-1.0, 1.0).asin();
    let l2 = l1 + (h.sin() * d.sin() * p1.cos()).atan2(d.cos() - p1.sin() * p2.sin());
    (p2.to_degrees(), (l2.to_degrees() + 540.0).rem_euclid(360.0) - 180.0)
}

fn toward(value: f64, target: f64, rate: f64) -> f64 {
    if value < target { (value + rate).min(target) } else { (value - rate).max(target) }
}

/// Шаг полёта на dt секунд, не больше [`DT_MAX`]. ground - высота
/// поверхности под самолётом над уровнем моря. Касание поверхности ставит
/// crashed.
pub fn step(plane: &mut Plane, dt: f64, controls: Controls, ground: f64) {
    if plane.crashed {
        return;
    }
    let mut left = dt.clamp(0.0, DT_MAX);
    while left > 1e-9 {
        let part = left.min(STEP_MAX);
        left -= part;
        sub_step(plane, part, controls, ground);
        if plane.crashed {
            break;
        }
    }
}

fn sub_step(plane: &mut Plane, dt: f64, keys: Controls, ground: f64) {
    let spec = plane.spec;
    plane.throttle = (plane.throttle + keys.throttle * THROTTLE_RATE * dt).clamp(0.0, 1.0);
    if keys.roll != 0.0 {
        plane.roll = (plane.roll + keys.roll * spec.roll_rate * dt).clamp(-spec.roll_max, spec.roll_max);
    } else {
        plane.roll = toward(plane.roll, 0.0, ROLL_RETURN * dt);
    }
    if keys.pitch != 0.0 {
        plane.pitch += keys.pitch * spec.pitch_rate * dt;
    }
    if plane.speed < spec.stall {
        plane.pitch -= STALL_DROP * dt;
    }
    plane.pitch = plane.pitch.clamp(-spec.pitch_max, spec.pitch_max);
    let pitch = plane.pitch.to_radians();
    let accel = spec.thrust * plane.throttle - spec.drag * plane.speed * plane.speed - G * pitch.sin();
    plane.speed = (plane.speed + accel * dt).max(0.0);
    let speed = plane.speed.max(1.0);
    let turn = (G * plane.roll.to_radians().tan() / speed).to_degrees();
    plane.heading = (plane.heading + turn * dt).rem_euclid(360.0);
    plane.climb = plane.speed * pitch.sin();
    plane.alt += plane.climb * dt;
    let run = plane.speed * pitch.cos() * dt;
    if run > 0.0 {
        (plane.lat, plane.lon) = destination(plane.lat, plane.lon, plane.heading, run, plane.radius);
    }
    if plane.alt <= ground {
        plane.alt = ground;
        plane.crashed = true;
    }
}

/// Деления ленты прибора: (отметка, отступ от value) для отметок, кратных
/// step, в пределах value ± half. Шаг не больше нуля или делений больше
/// [`TAPE_MAX`] - пусто.
pub fn tape(value: f64, step: f64, half: f64) -> Vec<(f64, f64)> {
    let first = ((value - half) / step).ceil();
    let last = ((value + half) / step).floor();
    if !(step > 0.0 && first.is_finite() && last.is_finite()) || last - first >= TAPE_MAX as f64 {
        return Vec::new();
    }
    let (first, last) = (first as i64, last as i64);
    (first..=last).map(|n| (n as f64 * step, n as f64 * step - value)).collect()
}

/// Деления ленты курса: отметки 0..359 с переходом через север.
pub fn heading_tape(heading: f64, step: f64, half: f64) -> Vec<(f64, f64)> {
    tape(heading, step, half).into_iter().map(|(mark, offset)| (mark.rem_euclid(360.0), offset)).collect()
}

/// Ступени шкалы тангажа у текущего тангажа: (отметка, угол от оси
/// взгляда), отметки не круче ±90°. У модуля по умолчанию step 5,
/// reach 25.
pub fn ladder(pitch: f64, step: f64, reach: f64) -> Vec<(f64, f64)> {
    tape(pitch, step, reach).into_iter().filter(|(mark, _)| mark.abs() <= 90.0).map(|(mark, _)| (mark, mark - pitch)).collect()
}

/// Отступ ступени шкалы тангажа на экране вверх от середины в пикселях:
/// focal - фокусное расстояние кадра в тех же пикселях.
pub fn ladder_offset(angle: f64, focal: f64) -> f64 {
    focal * angle.clamp(-89.0, 89.0).to_radians().tan()
}

/// Показания для строки на экране.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Readout {
    /// Скорость, км/ч.
    pub speed: f64,
    /// Высота над уровнем моря, м.
    pub alt: f64,
    /// Высота над поверхностью, м.
    pub height: f64,
    /// Курс, °.
    pub heading: f64,
    /// Подъём, м/с.
    pub climb: f64,
    /// Тяга, %.
    pub throttle: f64,
}

/// Показания самолёта над поверхностью высоты ground.
pub fn readout(plane: &Plane, ground: f64) -> Readout {
    Readout {
        speed: plane.speed * 3.6,
        alt: plane.alt,
        height: plane.alt - ground,
        heading: plane.heading,
        climb: plane.climb,
        throttle: 100.0 * plane.throttle,
    }
}
