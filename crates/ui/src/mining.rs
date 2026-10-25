// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Строка «Горные разработки»: контуры карьеров, отвалов
//! и хвостохранилищ мира, Maus и др. 2022. Перенос `_set_mining`,
//! `_open_mining` и `_identify_mining` модуля.
//!
//! Первое включение скачивает GeoPackage около 25 МБ в папку данных
//! программы, дальше файл читается без сети. У страницы файлов нет,
//! файл скачивается при каждом включении. Разбор у программы идёт
//! в рабочем потоке. Контуры рисует слой над картами
//! ([`crate::vector::Painter`]).

use std::sync::Arc;

use data::areas::{Area, Areas};

use crate::i18n::{Lang, tr, trf};

/// Заливка и обвод, `MINING_FILL` и `MINING_LINE` модуля.
pub const FILL: [u8; 4] = [235, 125, 35, 90];
pub const LINE: [u8; 4] = [205, 85, 20, 255];
/// Толщина обвода, пиксели картинки тайла: 0.35 мм модуля.
pub const WIDTH: f32 = 1.3;
/// Наибольший ответ, байт.
const ANSWER_BYTES: u64 = 64 << 20;

/// Контуры и поля: страна и площадь, км².
pub struct Mines {
    pub areas: Areas,
    pub fields: Vec<(String, f64)>,
}

type Parsed = Result<Mines, String>;

enum State {
    Asking(Box<dyn net::Request>),
    #[cfg(not(target_arch = "wasm32"))]
    Parsing(std::sync::mpsc::Receiver<Parsed>),
    Ready(Arc<Mines>),
    Failed,
}

pub struct Mining {
    state: State,
    /// Файл в папке данных, у страницы None.
    path: Option<std::path::PathBuf>,
    /// Слой готов в этом шаге: картинки тайлов рисуются заново.
    pub changed: bool,
}

/// Контуры из байтов GeoPackage.
fn parse(bytes: Vec<u8>) -> Parsed {
    let layers = data::layers::read(bytes, data::mining::POLYGONS_FILE)?;
    let mut items = Vec::new();
    let mut fields = Vec::new();
    for layer in layers {
        let find = |name: &str| layer.fields.iter().position(|f| f.eq_ignore_ascii_case(name));
        let (country, area) = (find("COUNTRY_NAME"), find("AREA"));
        for item in layer.items {
            if item.shape != data::layers::Shape::Polygon {
                continue;
            }
            let value = |k: Option<usize>| k.and_then(|k| item.values.get(k)).cloned().unwrap_or_default();
            fields.push((value(country), value(area).parse::<f64>().unwrap_or(0.0)));
            items.push(Area::new(item.parts));
        }
    }
    if items.is_empty() {
        return Err("no polygons".into());
    }
    Ok(Mines { areas: Areas::new(items), fields })
}

impl Mining {
    /// Строка включена: файл из папки данных или запрос.
    pub fn new(lang: Lang, path: Option<std::path::PathBuf>, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> (Self, Option<String>) {
        if let Some(bytes) = path.as_ref().and_then(|p| std::fs::read(p).ok()) {
            return (Self { state: Self::parse_state(bytes), path, changed: false }, None);
        }
        let ask = request(ANSWER_BYTES);
        // У страницы - копия planetx-live: у PANGAEA нет CORS.
        #[cfg(not(target_arch = "wasm32"))]
        ask.start(data::mining::POLYGONS_URL);
        #[cfg(target_arch = "wasm32")]
        ask.start(data::mining::POLYGONS_WEB);
        (Self { state: State::Asking(ask), path, changed: false }, Some(tr(lang, "Loading mining area outlines, about 25 MB…").to_string()))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn parse_state(bytes: Vec<u8>) -> State {
        let (tx, rx) = std::sync::mpsc::channel();
        match std::thread::Builder::new().name("planetx-mining".into()).spawn(move || {
            let _ = tx.send(parse(bytes));
        }) {
            Ok(_) => State::Parsing(rx),
            Err(_) => State::Failed,
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn parse_state(bytes: Vec<u8>) -> State {
        match parse(bytes) {
            Ok(m) => State::Ready(Arc::new(m)),
            Err(_) => State::Failed,
        }
    }

    /// Идёт загрузка или разбор.
    pub fn busy(&self) -> bool {
        match self.state {
            State::Asking(_) => true,
            #[cfg(not(target_arch = "wasm32"))]
            State::Parsing(_) => true,
            _ => false,
        }
    }

    /// Шаг: ответ - в файл и разбор, разбор - в слой. Возвращает
    /// сообщение пользователю.
    pub fn step(&mut self, lang: Lang) -> Option<String> {
        let mut message = None;
        let next = match &self.state {
            State::Asking(ask) => match ask.take() {
                Some(Ok(bytes)) => {
                    if let Some(path) = &self.path {
                        // Файл пишется рядом и подменяет прежний целиком.
                        let part = path.with_extension("part");
                        if let Err(e) = std::fs::write(&part, &bytes).and_then(|_| std::fs::rename(&part, path)) {
                            message = Some(trf(lang, "Mining area outlines are not written: {error}", &[("error", &e.to_string())]));
                        }
                    }
                    Some(Self::parse_state(bytes))
                }
                Some(Err(_)) => {
                    message = Some(tr(lang, "Mining area outlines did not load, check the network and switch the row on again.").to_string());
                    Some(State::Failed)
                }
                None => None,
            },
            #[cfg(not(target_arch = "wasm32"))]
            State::Parsing(rx) => match rx.try_recv() {
                Ok(Ok(m)) => Some(State::Ready(Arc::new(m))),
                Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    message = Some(trf(lang, "The mining area outline file cannot be read: {path}", &[("path", data::mining::POLYGONS_FILE)]));
                    Some(State::Failed)
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            },
            _ => None,
        };
        if let Some(state) = next {
            self.changed = matches!(state, State::Ready(_));
            self.state = state;
        }
        message
    }

    /// Готовые контуры.
    pub fn mines(&self) -> Option<Arc<Mines>> {
        match &self.state {
            State::Ready(m) => Some(m.clone()),
            _ => None,
        }
    }

    /// Группа окна «Объекты»: контуры под точкой.
    pub fn identify(&self, lang: Lang, lat: f64, lon: f64) -> Option<(String, Vec<crate::identify::Feature>)> {
        let mines = self.mines()?;
        let features: Vec<crate::identify::Feature> = mines
            .areas
            .at(lat, lon)
            .into_iter()
            .filter_map(|i| mines.fields.get(i))
            .map(|(country, area)| {
                let values = vec![
                    (tr(lang, "Country").to_string(), country.clone()),
                    (tr(lang, "Area").to_string(), trf(lang, "{value} km²", &[("value", &format!("{area:.2}"))])),
                    (tr(lang, "Dataset").to_string(), "Maus et al. 2022, PANGAEA".to_string()),
                ];
                (tr(lang, "Mining area").to_string(), values)
            })
            .collect();
        (!features.is_empty()).then(|| (tr(lang, "Mining areas").to_string(), features))
    }
}

/// Картинка тайлов слоя контуров.
pub fn painter(mines: Arc<Mines>) -> crate::vector::Painter {
    Arc::new(move |key| data::areas::draw(key, &mines.areas, FILL, LINE, WIDTH))
}
