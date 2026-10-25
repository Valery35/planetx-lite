// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Разрез Земли, строка «Разрез Земли» каталога, перенос `_set_cutaway`
//! (`ui/globe/earthdata.py`) и граней разреза `render/view.py` модуля.
//!
//! Сектор встаёт под точкой взгляда в момент включения. Поверхность
//! внутри него не рисуется, на гранях - оболочки PREM, кора CRUST1.0,
//! когда модель пришла, и плиты Slab2 зон, которые задевает дуга грани.
//! Издалека кора и верхняя мантия растянуты ступенями `gain_for`, пункт
//! «Растягивать кору» меню строки это снимает.
//!
//! Нет пока: правки углов сектора ручками, томографии мантии, подписей
//! оболочек, легенды, выгрузки коры.

use std::collections::{BTreeMap, HashMap};

use data::crust::{self, Crust};
use data::cutaway::{self, Faces, Wedge};
use data::slabs::{self, Frame, Slab};

/// Индекс зон Slab2, файл модуля.
pub const SLAB_INDEX: &str = "https://raw.githubusercontent.com/Valery35/planetx/main/planetx/data/slab2_index.json";
/// Пределы ответов: архив CRUST1.0, индекс и файл зоны.
pub const CRUST_BYTES: u64 = 16 << 20;
pub const SLAB_BYTES: u64 = 16 << 20;
pub const INDEX_BYTES: u64 = 1 << 20;

/// Загрузка: идёт, готово, не удалось.
enum Ask<T> {
    Idle,
    Asking(Box<dyn net::Request>),
    Ready(T),
    Failed(String),
}

/// Разрез Земли.
pub struct Cutaway {
    pub wedge: Wedge,
    /// Растягивать кору издалека.
    pub stretch: bool,
    gain: f64,
    crust: Ask<Crust>,
    index: Ask<BTreeMap<String, Frame>>,
    zones: HashMap<String, Ask<Slab>>,
    /// Грани собрать заново.
    dirty: bool,
}

/// Сетки разреза для видеокарты: грани и полосы плит.
pub struct Meshes {
    pub faces: Option<render::SubMesh>,
    pub slabs: Option<render::SubMesh>,
}

fn to_sub(mesh: Option<data::subsurface::Mesh>, bias: bool) -> Option<render::SubMesh> {
    let mesh = mesh?;
    let vertices = mesh
        .vertices
        .iter()
        .map(|v| render::SubVertex {
            offset: v.position,
            normal: [f32::from(v.normal[0]) / 127.0, f32::from(v.normal[1]) / 127.0, f32::from(v.normal[2]) / 127.0],
            color: v.color,
        })
        .collect();
    Some(render::SubMesh { center: globe_core::DVec3::from_array(mesh.center), vertices, indices: mesh.indices, cull_front: false, bias })
}

impl Cutaway {
    /// Разрез под точкой (lat, lon). crust - архив CRUST1.0 из кэша.
    pub fn new(lat: f64, lon: f64, stretch: bool, crust: Option<&[u8]>) -> Cutaway {
        let crust = match crust.map(crust::from_archive) {
            Some(Ok(c)) => Ask::Ready(c),
            // CRUST1.0 без права распространения не копируется, сайт
            // не отдаёт архив странице: у неё разрез без коры.
            _ if cfg!(target_arch = "wasm32") => Ask::Failed(crate::layers::DESKTOP_ONLY.to_string()),
            _ => Ask::Idle,
        };
        Cutaway { wedge: cutaway::wedge_at(lat, lon, cutaway::HALF), stretch, gain: f64::NAN, crust, index: Ask::Idle, zones: HashMap::new(), dirty: true }
    }

    /// Числа сектора для шейдера тайла.
    pub fn uniform(&self) -> [f64; 5] {
        cutaway::uniform(&self.wedge)
    }

    /// Растягивать кору или нет.
    pub fn set_stretch(&mut self, on: bool) {
        self.stretch = on;
        self.dirty = true;
    }

    /// Идёт ли загрузка.
    pub fn busy(&self) -> bool {
        matches!(self.crust, Ask::Asking(_)) || matches!(self.index, Ask::Asking(_)) || self.zones.values().any(|z| matches!(z, Ask::Asking(_)))
    }

    /// Шаг кадра: запросы и ответы, сетки, если что-то сменилось.
    /// distance - расстояние до точки взгляда, request - новый запрос
    /// с пределом ответа. Возвращает новые сетки и байты пришедшего
    /// архива коры для кэша.
    pub fn step(&mut self, distance: f64, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> (Option<Meshes>, Option<Vec<u8>>) {
        let mut saved = None;
        if matches!(self.crust, Ask::Idle) {
            let ask = request(CRUST_BYTES);
            ask.start(crust::URL);
            self.crust = Ask::Asking(ask);
        }
        if let Ask::Asking(ask) = &self.crust
            && let Some(answer) = ask.take()
        {
            self.crust = match answer.map_err(|e| e.to_string()).and_then(|b| crust::from_archive(&b).map(|c| (c, b)).map_err(|e| e.to_string())) {
                Ok((c, bytes)) => {
                    saved = Some(bytes);
                    Ask::Ready(c)
                }
                Err(e) => Ask::Failed(e),
            };
            self.dirty = true;
        }
        if matches!(self.index, Ask::Idle) {
            let ask = request(INDEX_BYTES);
            ask.start(SLAB_INDEX);
            self.index = Ask::Asking(ask);
        }
        if let Ask::Asking(ask) = &self.index
            && let Some(answer) = ask.take()
        {
            self.index = match answer.and_then(|b| slabs::read_index(&b).map_err(|e| e.0)) {
                Ok(i) => Ask::Ready(i),
                Err(e) => Ask::Failed(e),
            };
            // Зоны, которые задевают дуги граней.
            if let Ask::Ready(index) = &self.index {
                let (lats, lons) = cutaway::arc_points(&self.wedge, cutaway::SLAB_STEP);
                for code in slabs::touched(index, &lats, &lons) {
                    let ask = request(SLAB_BYTES);
                    ask.start(&slabs::url_of(&code, None));
                    self.zones.insert(code, Ask::Asking(ask));
                }
            }
        }
        for (code, zone) in self.zones.iter_mut() {
            if let Ask::Asking(ask) = zone
                && let Some(answer) = ask.take()
            {
                *zone = match answer {
                    Ok(b) => match slabs::load(&b, code) {
                        Ok(s) => Ask::Ready(s),
                        Err(e) => Ask::Failed(e.0),
                    },
                    Err(e) => Ask::Failed(e),
                };
                self.dirty = true;
            }
        }
        let gain = if self.stretch { cutaway::gain_for(distance) } else { 1.0 };
        if gain != self.gain {
            self.gain = gain;
            self.dirty = true;
        }
        if !self.dirty {
            return (None, saved);
        }
        self.dirty = false;
        let crust = match &self.crust {
            Ask::Ready(c) => Some(c),
            _ => None,
        };
        let faces = cutaway::faces(&self.wedge, &Faces { crust, gain: self.gain, ..Faces::default() });
        let zones: Vec<&Slab> = self
            .zones
            .values()
            .filter_map(|z| match z {
                Ask::Ready(s) => Some(s),
                _ => None,
            })
            .collect();
        let bands = if zones.is_empty() { None } else { cutaway::slab_bands(&self.wedge, &zones, self.gain, cutaway::SLAB_STEP, None) };
        (Some(Meshes { faces: to_sub(faces, false), slabs: to_sub(bands, true) }), saved)
    }

    /// Ошибка загрузки коры, если была.
    pub fn crust_error(&self) -> Option<&str> {
        match &self.crust {
            Ask::Failed(e) => Some(e),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Never(net::Stats);

    impl net::Request for Never {
        fn start(&self, _url: &str) {}
        fn take(&self) -> Option<Result<Vec<u8>, String>> {
            None
        }
        fn stats(&self) -> &net::Stats {
            &self.0
        }
    }

    #[test]
    fn faces_without_data_and_gain_steps() {
        let mut c = Cutaway::new(58.0, 56.0, true, None);
        let ask = |_: u64| Box::new(Never(net::Stats::default())) as Box<dyn net::Request>;
        let (meshes, saved) = c.step(2.0e7, &ask);
        assert!(saved.is_none());
        assert!(meshes.is_some());
        let Some(m) = meshes else {
            return;
        };
        assert!(m.faces.is_some_and(|f| !f.vertices.is_empty()));
        assert!(m.slabs.is_none());
        assert!(c.busy());
        // Тот же шаг растяжения - граней заново нет.
        assert!(c.step(2.0e7, &ask).0.is_none());
        c.set_stretch(false);
        assert!(c.step(2.0e7, &ask).0.is_some());
        let u = c.uniform();
        assert!(u.iter().all(|v| v.is_finite()));
    }
}
