// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! «Муниципальные образования России»: границы СберИндекса на 1 января
//! 2025 года, ядро - `data::munis`. Строка каталога выключена при
//! открытии. Включённая строка скачивает файл слоя один раз, границы
//! ложатся линиями в картинку тайлов, у точки взгляда - названия.
//! Щелчок даёт в окне «Объекты» тип, ОКТМО, регион и центр. Пока строка
//! включена, внизу вида - подпись «СберИндекс, CC BY-SA 4.0».

use std::sync::Arc;

use data::linesets::Line;
use data::munis::{self, Munis};
use globe_core::DVec3;

use crate::i18n::{Lang, tr};
use crate::identify::Feature;
use crate::layers::Category;
use crate::registry;
use crate::thumbs::Recipe;

/// Подписей на экране сразу.
const LABELS: usize = 150;
/// Подписи - при ширине вида не больше стольких метров.
const LABEL_VIEW: f64 = 3.0e6;
/// Пауза между сборками подписей, секунды.
const REBUILD: f64 = 0.5;

/// Строка реестра.
pub struct Entry;

/// Строка «Муниципальные образования» в [`crate::registry::ALL`].
pub static ENTRY: Entry = Entry;

impl registry::Entry for Entry {
    fn key(&self) -> &'static str {
        "munis"
    }
    fn name(&self) -> &'static str {
        "Municipalities of Russia"
    }
    fn tip(&self) -> &'static str {
        "Borders of the municipal districts, municipal and urban okrugs of Russia as of 1 January 2025 after the SberIndex versioned reference book, about 2570 units. Names show near the look-at point. A click inside a unit shows its type, OKTMO code, region and centre. The first switch-on downloads about 4 MB."
    }
    fn category(&self) -> Category {
        Category::Local
    }
    fn source(&self) -> &'static str {
        "SberIndex"
    }
    fn words(&self) -> &'static [&'static str] {
        &["муниципалитеты", "муниципальные", "районы", "округа", "границы", "октмо", "сбериндекс", "municipalities", "districts", "borders", "oktmo", "sberindex"]
    }
    fn legend(&self) -> Option<Recipe> {
        let c = munis::COLOR;
        Some(Recipe::Ramp(vec![[c[0], c[1], c[2]]]))
    }
    fn make(&self, cx: &registry::Make<'_>) -> (Box<dyn registry::Layer>, Option<String>) {
        let ask = (cx.request)(munis::BYTES);
        ask.start(munis::URL);
        (Box::new(Shown { state: State::Asking(ask), lines: None, labels: Vec::new(), built: f64::NEG_INFINITY }), None)
    }
}

type Parsed = Result<Munis, String>;

/// Файл слоя: идёт, разбирается, разобран или не пришёл.
enum State {
    Asking(Box<dyn net::Request>),
    #[cfg(not(target_arch = "wasm32"))]
    Parsing(std::sync::mpsc::Receiver<Parsed>),
    Ready(Munis),
    Failed,
}

/// Включённая строка.
struct Shown {
    state: State,
    lines: Option<Arc<Vec<Line>>>,
    /// Подписи: название и точка глобуса.
    labels: Vec<(String, DVec3)>,
    built: f64,
}

impl Shown {
    fn munis(&self) -> Option<&Munis> {
        match &self.state {
            State::Ready(m) => Some(m),
            _ => None,
        }
    }
}

/// Разбор в рабочем потоке программы, у страницы - сразу.
fn start_parse(bytes: Vec<u8>) -> State {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(munis::parse(&bytes));
        });
        State::Parsing(rx)
    }
    #[cfg(target_arch = "wasm32")]
    {
        match munis::parse(&bytes) {
            Ok(m) => State::Ready(m),
            Err(_) => State::Failed,
        }
    }
}

impl registry::Layer for Shown {
    fn step(&mut self, cx: &registry::Ctx<'_>) -> registry::Stepped {
        let mut stepped = registry::Stepped::default();
        if let State::Asking(ask) = &self.state
            && let Some(answer) = ask.take()
        {
            self.state = match answer {
                Ok(bytes) => start_parse(bytes),
                Err(_) => State::Failed,
            };
            if matches!(self.state, State::Failed) {
                stepped.message = Some(tr(cx.lang, "The municipal borders did not load.").to_string());
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let State::Parsing(rx) = &self.state
            && let Ok(parsed) = rx.try_recv()
        {
            self.state = match parsed {
                Ok(m) => State::Ready(m),
                Err(_) => {
                    stepped.message = Some(tr(cx.lang, "The municipal borders did not load.").to_string());
                    State::Failed
                }
            };
        }
        if self.lines.is_none()
            && let Some(m) = self.munis()
        {
            self.lines = Some(Arc::new(m.lines()));
            stepped.lines = true;
        }
        if cx.now - self.built >= REBUILD
            && let State::Ready(m) = &self.state
        {
            self.built = cx.now;
            let (lat, lon, _, width) = cx.view;
            let e = cx.camera.ellipsoid;
            let mut near: Vec<(f64, &munis::Unit)> = if width <= LABEL_VIEW {
                m.units.iter().map(|u| (data::linesets::distance(&[(u.lat, u.lon)], lat, lon), u)).filter(|(d, _)| *d <= width).collect()
            } else {
                Vec::new()
            };
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            self.labels = near.into_iter().take(LABELS).map(|(_, u)| (u.name.clone(), e.geodetic_to_ecef(u.lat, u.lon, (cx.ground)(u.lat, u.lon)))).collect();
        }
        stepped
    }
    fn lines(&self) -> Option<Arc<Vec<Line>>> {
        self.lines.clone()
    }
    fn labels(&self) -> &[(String, DVec3)] {
        &self.labels
    }
    fn busy(&self) -> bool {
        match &self.state {
            State::Asking(_) => true,
            #[cfg(not(target_arch = "wasm32"))]
            State::Parsing(_) => true,
            _ => false,
        }
    }
    fn attribution(&self) -> Option<(&'static str, &'static str)> {
        Some(munis::ATTRIBUTION)
    }
    fn identify(&self, lang: Lang, lat: f64, lon: f64) -> Option<(String, Vec<Feature>)> {
        let u = self.munis()?.unit_at(lat, lon)?;
        let row = |a: &'static str, b: String| (tr(lang, a).to_string(), b);
        let mut values = Vec::new();
        for (name, value) in [("Type", &u.kind), ("OKTMO", &u.oktmo), ("Region", &u.region), ("Administrative centre", &u.center)] {
            if !value.is_empty() {
                values.push(row(name, value.clone()));
            }
        }
        if u.from > 0 {
            values.push(row("In these borders since", u.from.to_string()));
        }
        values.push(row("Source", munis::ATTRIBUTION.1.to_string()));
        Some((tr(lang, "Municipalities of Russia").to_string(), vec![(u.name.clone(), values)]))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn short_credit_keeps_the_licence() {
        let short = crate::credits::short_credits(&[data::munis::ATTRIBUTION.0], 0);
        assert_eq!(short, vec!["SberIndex - CC BY-SA 4.0".to_string()]);
    }
}
