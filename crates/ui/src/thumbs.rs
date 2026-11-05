// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Загрузка превью строк каталога «Добавить данные», перенос
//! `_load_base`, `_pump`, `_thumb_url` и `_thumb_done` из
//! `ui/nasamaps.py` модуля. Картинки собирает `data::thumbs`.
//!
//! Сначала приходит тайл подложки [`data::thumbs::TILE`], затем тайлы
//! строк, которые видны в списке: свой тайл подосновы, тайл карты или
//! тайл темы NASA GIBS на последний готовый день. Запросов не больше
//! [`PARALLEL`] сразу, сеть главный поток не ждёт. Превью живут до конца
//! работы программы, смена подложки собирает их заново.

use std::collections::HashMap;

use data::thumbs as th;
use eframe::egui;

/// Запросов превью сразу, `PARALLEL` модуля.
pub const PARALLEL: usize = 4;
/// Наибольший ответ на тайл, байт.
const TILE_BYTES: u64 = 4 << 20;
/// Наибольший ответ ряда дат темы, байт.
const DOMAINS_BYTES: u64 = 4 << 20;

/// Как собирается превью строки.
#[derive(Clone)]
pub enum Recipe {
    /// Свой тайл строки - превью целиком, у подосновы.
    Own(String),
    /// Тайл карты по адресу поверх подложки. kind переводит его
    /// в картинку слоя.
    Map(String, crate::maps::Kind),
    /// Тема NASA GIBS: тайл последнего готового дня поверх подложки.
    Theme(&'static data::gibs::Theme),
    /// Полоса цветов шкалы поверх подложки.
    Ramp(Vec<[u8; 3]>),
    /// Полоса цветов без подложки.
    Flat(Vec<[u8; 3]>),
    /// Одна подложка.
    Base,
}

/// Состояние строки.
enum State {
    /// Ждёт подложки или места среди запросов.
    Waiting,
    /// Просит ряд дат темы.
    Domains(Box<dyn net::Request>),
    /// Просит тайл.
    Tile(Box<dyn net::Request>),
    /// Превью готово.
    Done(egui::TextureHandle),
}

struct Row {
    recipe: Recipe,
    state: State,
}

/// Тайл подложки превью.
enum Base {
    None,
    Asking(String, Box<dyn net::Request>),
    /// Адрес и тайл, приведённый к стороне превью, None - тайла нет.
    Ready(String, Option<Vec<u8>>),
}

/// Превью строк каталога.
pub struct Thumbs {
    base: Base,
    rows: HashMap<String, Row>,
}

impl Default for Thumbs {
    fn default() -> Self {
        Self { base: Base::None, rows: HashMap::new() }
    }
}

impl std::fmt::Debug for Thumbs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Thumbs").field("rows", &self.rows.len()).finish()
    }
}

/// Тайл PNG или JPEG в RGBA и его размер.
fn decode(bytes: &[u8]) -> Option<(Vec<u8>, usize, usize)> {
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = image.dimensions();
    Some((image.into_raw(), w as usize, h as usize))
}

/// Секунды UTC сейчас.
fn now_utc() -> f64 {
    web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

impl Thumbs {
    /// Превью строки key, если готово. recipe - как его собрать, строка
    /// заводится при первом показе.
    pub fn texture(&mut self, key: &str, recipe: impl FnOnce() -> Recipe) -> Option<egui::TextureId> {
        let row = self.rows.entry(key.to_string()).or_insert_with(|| Row { recipe: recipe(), state: State::Waiting });
        match &row.state {
            State::Done(t) => Some(t.id()),
            _ => None,
        }
    }

    /// Шаг загрузки: подложка base_url, ответы, новые запросы, сборка
    /// готовых превью.
    pub fn step(&mut self, ctx: &egui::Context, base_url: &str, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        let current = match &self.base {
            Base::Asking(u, _) | Base::Ready(u, _) => Some(u.as_str()),
            Base::None => None,
        };
        if current != Some(base_url) {
            // Другая подложка: превью собираются заново.
            let ask = request(TILE_BYTES);
            ask.start(base_url);
            self.base = Base::Asking(base_url.to_string(), ask);
            self.rows.clear();
        }
        if let Base::Asking(url, ask) = &self.base
            && let Some(answer) = ask.take()
        {
            let tile = answer.ok().and_then(|b| decode(&b)).and_then(|(rgba, w, h)| th::fit(&rgba, w, h));
            self.base = Base::Ready(url.clone(), tile);
        }
        let Base::Ready(_, base) = &self.base else {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        };
        let mut busy = self.rows.values().filter(|r| matches!(r.state, State::Domains(_) | State::Tile(_))).count();
        let mut keys: Vec<String> = self.rows.keys().cloned().collect();
        keys.sort();
        for key in keys {
            let Some(row) = self.rows.get_mut(&key) else {
                continue;
            };
            let state = std::mem::replace(&mut row.state, State::Waiting);
            row.state = match state {
                State::Done(t) => State::Done(t),
                State::Waiting => match &row.recipe {
                    Recipe::Ramp(colors) => done(ctx, &key, th::ramp(colors, base.as_deref())),
                    Recipe::Flat(colors) => done(ctx, &key, th::ramp(colors, None)),
                    Recipe::Base => done(ctx, &key, th::over(base.as_deref(), None)),
                    _ if busy >= PARALLEL => State::Waiting,
                    Recipe::Own(url) | Recipe::Map(url, _) => {
                        busy += 1;
                        let ask = request(TILE_BYTES);
                        ask.start(url);
                        State::Tile(ask)
                    }
                    Recipe::Theme(theme) => match theme.catalog_day(now_utc()) {
                        Some(day) => {
                            busy += 1;
                            let ask = request(TILE_BYTES);
                            ask.start(&net::template_url(&theme.url(&day), th::TILE));
                            State::Tile(ask)
                        }
                        None if theme.intervals.is_some() => done(ctx, &key, th::over(base.as_deref(), None)),
                        None => {
                            busy += 1;
                            let ask = request(DOMAINS_BYTES);
                            ask.start(&theme.domains_url());
                            State::Domains(ask)
                        }
                    },
                },
                State::Domains(ask) => match (ask.take(), &row.recipe) {
                    (None, _) => State::Domains(ask),
                    (Some(answer), Recipe::Theme(theme)) => {
                        let intervals = answer.map(|b| data::gibs::parse_domains(&String::from_utf8_lossy(&b))).unwrap_or_default();
                        let ready = data::gibs::ready_intervals(&intervals, now_utc());
                        match data::gibs::pick_day(&ready, None) {
                            Some(day) => {
                                let tile = request(TILE_BYTES);
                                tile.start(&net::template_url(&theme.url(&day), th::TILE));
                                State::Tile(tile)
                            }
                            None => {
                                busy = busy.saturating_sub(1);
                                done(ctx, &key, th::over(base.as_deref(), None))
                            }
                        }
                    }
                    (Some(_), _) => done(ctx, &key, th::over(base.as_deref(), None)),
                },
                State::Tile(ask) => match ask.take() {
                    None => State::Tile(ask),
                    Some(answer) => {
                        busy = busy.saturating_sub(1);
                        let tile = answer.ok().and_then(|b| decode(&b));
                        done(ctx, &key, compose(&row.recipe, base.as_deref(), tile))
                    }
                },
            };
        }
        if busy > 0 || self.rows.values().any(|r| matches!(r.state, State::Waiting)) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

/// Превью строки из пришедшего тайла tile (RGBA, ширина, высота).
fn compose(recipe: &Recipe, base: Option<&[u8]>, tile: Option<(Vec<u8>, usize, usize)>) -> Vec<u8> {
    let key = th::TILE;
    match (recipe, tile) {
        (Recipe::Own(_), Some((rgba, w, h))) => th::over(th::fit(&rgba, w, h).as_deref(), None),
        (Recipe::Map(_, kind), Some((rgba, w, h))) => {
            // Картинка слоя - у тайла 256 × 256, как на глобусе.
            let top = (w == 256 && h == 256).then(|| kind.picture(&rgba, key)).flatten().and_then(|p| th::fit(&p, w, h));
            th::over(base, top.as_deref())
        }
        (Recipe::Theme(theme), Some((rgba, w, h))) => {
            let top = data::gibs::overlay_rgba(&rgba, data::gibs::OPACITY, theme.clear).and_then(|p| th::fit(&p, w, h));
            th::over(base, top.as_deref())
        }
        (Recipe::Own(_), None) => th::over(None, None),
        _ => th::over(base, None),
    }
}

/// Готовое превью в текстуру egui.
fn done(ctx: &egui::Context, key: &str, rgba: Vec<u8>) -> State {
    let image = egui::ColorImage::from_rgba_unmultiplied([th::SIDE, th::SIDE], &rgba);
    State::Done(ctx.load_texture(format!("thumb {key}"), image, egui::TextureOptions::LINEAR))
}
