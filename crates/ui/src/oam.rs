// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Снимки OpenAerialMap»: снимки с дронов и самолётов в рамке вида,
//! перенос `ui/oam.py` модуля. Поиск и разбор ответа - `data::oam`.
//!
//! Рамки снимков на глобусе рисуются как рамки карт Рамсея. Выбранный
//! снимок ложится строкой карт поверх подложки, файл не скачивается. Окно
//! есть только у программы: служба не отдаёт ответ странице.

use eframe::egui;
use serde_json::Value;

use data::oam::{self, Image};
use data::rumsey::Bbox;

use crate::i18n::{Lang, tr, trf};

/// Наибольший ответ службы и картинки, байт.
const ANSWER_BYTES: u64 = 16 << 20;
/// Пикселей на сторону картинки снимка, `PREVIEW` модуля.
const PREVIEW: f32 = 220.0;

/// Пиксель снимка словами.
fn gsd_text(lang: Lang, gsd: f64) -> String {
    if gsd <= 0.0 {
        String::new()
    } else if gsd < 1.0 {
        trf(lang, "{value} cm", &[("value", &format!("{:.1}", gsd * 100.0))])
    } else {
        trf(lang, "{value} m", &[("value", &format!("{gsd:.1}"))])
    }
}

/// Размер файла словами.
fn size_text(lang: Lang, size: u64) -> String {
    if size >= 1_000_000_000 {
        trf(lang, "{value} GB", &[("value", &format!("{:.1}", size as f64 / 1e9))])
    } else {
        trf(lang, "{value} MB", &[("value", &format!("{:.0}", size as f64 / 1e6))])
    }
}

/// Окно снимков.
pub struct Oam {
    pub open: bool,
    images: Vec<Image>,
    bbox: Bbox,
    page: u32,
    found: u64,
    /// Поиск идёт, первая страница заменяет список.
    ask: Option<Box<dyn net::Request>>,
    /// Картинка снимка: номер снимка, запрос.
    picture_ask: Option<(usize, Box<dyn net::Request>)>,
    picture: Option<(usize, egui::TextureHandle)>,
    filter: String,
    selected: Option<usize>,
    status: String,
    /// Перелёт к рамке выбранного снимка.
    pub fly: Option<Bbox>,
    /// Снимок на глобус.
    pub add: Option<Image>,
    /// Новый поиск в нынешней рамке вида.
    pub search: bool,
}

impl Oam {
    pub fn new(bbox: Bbox, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Self {
        let mut out = Self {
            open: true,
            images: Vec::new(),
            bbox,
            page: 0,
            found: 0,
            ask: None,
            picture_ask: None,
            picture: None,
            filter: String::new(),
            selected: None,
            status: String::new(),
            fly: None,
            add: None,
            search: false,
        };
        out.ask_page(request);
        out
    }

    /// Новый поиск в рамке bbox.
    pub fn show_view(&mut self, bbox: Bbox, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        self.open = true;
        self.bbox = bbox;
        self.images.clear();
        self.found = 0;
        self.page = 0;
        self.selected = None;
        self.picture = None;
        self.picture_ask = None;
        self.ask_page(request);
    }

    fn ask_page(&mut self, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        let ask = request(ANSWER_BYTES);
        ask.start(&oam::search_url(self.bbox, self.page + 1));
        self.ask = Some(ask);
    }

    /// Идёт загрузка.
    pub fn busy(&self) -> bool {
        self.ask.is_some() || self.picture_ask.is_some()
    }

    /// Строки списка под строкой поиска.
    fn rows(&self) -> Vec<usize> {
        let text = self.filter.trim().to_lowercase();
        (0..self.images.len()).filter(|n| text.is_empty() || self.images.get(*n).is_some_and(|i| format!("{} {}", i.title, i.provider).to_lowercase().contains(&text))).collect()
    }

    /// Рамки снимков списка и выбранного, для глобуса.
    pub fn footprints(&self) -> (Vec<Bbox>, Option<Bbox>) {
        (self.rows().iter().filter_map(|n| self.images.get(*n).map(|i| i.bbox)).collect(), self.selected.and_then(|n| self.images.get(n)).map(|i| i.bbox))
    }

    /// Шаг: ответ поиска и картинка снимка.
    pub fn step(&mut self, lang: Lang, ctx: &egui::Context) {
        if let Some(answer) = self.ask.as_ref().and_then(|a| a.take()) {
            self.ask = None;
            match answer.and_then(|b| serde_json::from_slice::<Value>(&b).map_err(|e| e.to_string())) {
                Ok(value) => {
                    let (images, found) = oam::parse(&value);
                    self.page += 1;
                    self.images.extend(images);
                    self.found = found;
                }
                Err(_) => {
                    self.status = tr(lang, "OpenAerialMap did not answer.").to_string();
                    return;
                }
            }
        }
        if let Some((n, ask)) = &self.picture_ask
            && let Some(answer) = ask.take()
        {
            let n = *n;
            self.picture_ask = None;
            if let Some(rgba) = answer.ok().and_then(|b| image::load_from_memory(&b).ok()).map(|i| i.to_rgba8()) {
                let size = [rgba.width() as usize, rgba.height() as usize];
                let picture = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                self.picture = Some((n, ctx.load_texture("planetx-oam", picture, egui::TextureOptions::LINEAR)));
            }
        }
    }

    /// Строка над списком.
    fn counted(&self, lang: Lang, shown: usize) -> String {
        if self.ask.is_some() {
            tr(lang, "Searching for images…").to_string()
        } else if !self.status.is_empty() {
            self.status.clone()
        } else if self.found == 0 {
            tr(lang, "There are no images in the view frame. Images cover the world unevenly, zoom out and press «Search in the view».").to_string()
        } else {
            trf(lang, "Images in the view frame: {found}, in the list {count}.", &[("found", &self.found.to_string()), ("count", &shown.to_string())])
        }
    }

    fn choose(&mut self, n: usize, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        self.selected = Some(n);
        self.picture = None;
        self.picture_ask = None;
        if let Some(url) = self.images.get(n).map(|i| i.thumbnail.as_str()).filter(|u| !u.is_empty()) {
            let ask = request(ANSWER_BYTES);
            ask.start(url);
            self.picture_ask = Some((n, ask));
        }
    }

    /// Окно.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        if !self.open {
            return;
        }
        let rows = self.rows();
        let mut open = true;
        let mut chosen = None;
        let mut add = false;
        let mut more = false;
        let mut fly = false;
        egui::Window::new(tr(lang, "OpenAerialMap imagery")).open(&mut open).default_size([860.0, 500.0]).show(ctx, |ui| {
            ui.add(egui::Label::new(self.counted(lang, rows.len())).wrap());
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(tr(lang, "Title or provider")).desired_width(f32::INFINITY));
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_max_width((ui.available_width() - PREVIEW - 12.0).max(200.0));
                    egui::ScrollArea::both().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
                        egui::Grid::new("planetx-oam").num_columns(5).striped(true).spacing([10.0, 3.0]).show(ui, |ui| {
                            for name in ["Date", "Image", "Pixel", "File", "License"] {
                                ui.strong(tr(lang, name));
                            }
                            ui.end_row();
                            for n in &rows {
                                let Some(image) = self.images.get(*n) else {
                                    continue;
                                };
                                ui.label(&image.day);
                                let r = ui.selectable_label(self.selected == Some(*n), &image.title).on_hover_text(format!("{} - {}", image.title, image.provider));
                                if r.clicked() {
                                    chosen = Some(*n);
                                }
                                if r.double_clicked() {
                                    chosen = Some(*n);
                                    add = true;
                                }
                                ui.label(gsd_text(lang, image.gsd));
                                ui.label(size_text(lang, image.size));
                                ui.label(&image.license);
                                ui.end_row();
                            }
                        });
                    });
                });
                let (rect, _) = ui.allocate_exact_size(egui::vec2(PREVIEW, PREVIEW), egui::Sense::hover());
                if let Some((n, texture)) = &self.picture
                    && Some(*n) == self.selected
                {
                    let size = texture.size_vec2();
                    let scale = (PREVIEW / size.x.max(size.y).max(1.0)).min(1.0);
                    let shown = egui::Rect::from_center_size(rect.center(), size * scale);
                    ui.painter().image(texture.id(), shown, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                }
            });
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "Search in the view")).on_hover_text(tr(lang, "Images in the present view frame of the globe, newest first.")).clicked() {
                    self.search = true;
                }
                let later = (self.images.len() as u64) < self.found && self.ask.is_none();
                if ui.add_enabled(later, egui::Button::new(tr(lang, "More"))).on_hover_text(tr(lang, "The next images of the same frame, older ones.")).clicked() {
                    more = true;
                }
                let has = self.selected.is_some();
                if ui.add_enabled(has, egui::Button::new(tr(lang, "On the globe"))).on_hover_text(tr(lang, "The image lies as a tile layer over the base map, a row in the catalogue. The file is not downloaded.")).clicked() {
                    add = true;
                }
                if ui.add_enabled(has, egui::Button::new(tr(lang, "Show on the globe"))).on_hover_text(tr(lang, "Flight to the frame of the chosen image.")).clicked() {
                    fly = true;
                }
            });
            ui.add(egui::Label::new(tr(lang, "The images are uploaded by contributors of Open Imagery Network. Each image has its own license, it is shown in the list and written to the credit of the layer. Images with NC are for non-commercial use only.")).wrap());
        });
        if let Some(n) = chosen {
            self.choose(n, request);
        }
        if more {
            self.status.clear();
            self.ask_page(request);
        }
        let current = self.selected.and_then(|n| self.images.get(n));
        if fly {
            self.fly = current.map(|i| i.bbox);
        }
        if add && let Some(image) = current {
            if image.tiles.is_empty() {
                self.status = trf(lang, "The image did not open: {title}.", &[("title", &image.title)]);
            } else {
                self.status = trf(lang, "The image is on the globe: {title}.", &[("title", &image.title)]);
                self.fly = Some(image.bbox);
                self.add = Some(image.clone());
            }
        }
        if !open {
            self.open = false;
            self.ask = None;
            self.picture_ask = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Oam;
    use crate::i18n::Lang;
    use eframe::egui;
    use std::cell::RefCell;

    /// Запрос с готовым ответом, запоминает адрес.
    struct Stub(RefCell<Option<Result<Vec<u8>, String>>>, net::Stats, std::rc::Rc<RefCell<Vec<String>>>);

    impl net::Request for Stub {
        fn start(&self, url: &str) {
            self.2.borrow_mut().push(url.to_string());
        }
        fn take(&self) -> Option<Result<Vec<u8>, String>> {
            self.0.borrow_mut().take()
        }
        fn stats(&self) -> &net::Stats {
            &self.1
        }
    }

    fn row(title: &str, tiles: &str) -> serde_json::Value {
        serde_json::json!({"uuid": format!("https://a/{title}.tif"), "title": title, "provider": "P", "bbox": [120.1, -8.4, 120.2, -8.3], "gsd": 0.5, "file_size": 2000000, "acquisition_end": "2026-08-29T00:00:00Z", "properties": {"license": "CC-BY 4.0", "tms": tiles}})
    }

    fn window(rows: serde_json::Value, found: u64) -> (Oam, std::rc::Rc<RefCell<Vec<String>>>) {
        let urls = std::rc::Rc::new(RefCell::new(Vec::new()));
        let answer = serde_json::to_vec(&serde_json::json!({"meta": {"found": found}, "results": rows})).unwrap_or_default();
        let seen = urls.clone();
        let request = move |_: u64| -> Box<dyn net::Request> { Box::new(Stub(RefCell::new(Some(Ok(answer.clone()))), net::Stats::default(), seen.clone())) };
        let mut oam = Oam::new([1.0, 2.0, 3.0, 4.0], &request);
        oam.step(Lang::RU, &egui::Context::default());
        (oam, urls)
    }

    #[test]
    fn answer_fills_list_and_frames() {
        let (mut oam, urls) = window(serde_json::json!([row("Один", "https://tiles.openaerialmap.org/a/{z}/{x}/{y}"), row("Два", "")]), 150);
        assert!(urls.borrow()[0].contains("bbox=1.00000%2C2.00000"));
        assert!(!oam.busy());
        assert_eq!(oam.footprints().0.len(), 2);
        // Строка поиска сужает список и рамки.
        oam.filter = "оди".to_string();
        assert_eq!(oam.footprints().0.len(), 1);
        // Новый поиск снимает прежний список.
        let answer = |_: u64| -> Box<dyn net::Request> { Box::new(Stub(RefCell::new(None), net::Stats::default(), std::rc::Rc::default())) };
        oam.show_view([5.0, 6.0, 7.0, 8.0], &answer);
        assert!(oam.busy());
        assert!(oam.footprints().0.is_empty());
    }

    #[test]
    fn bad_answer_says_so() {
        let answer = |_: u64| -> Box<dyn net::Request> { Box::new(Stub(RefCell::new(Some(Ok(b"<html>".to_vec()))), net::Stats::default(), std::rc::Rc::default())) };
        let mut oam = Oam::new([1.0, 2.0, 3.0, 4.0], &answer);
        oam.step(Lang::EN, &egui::Context::default());
        assert_eq!(oam.status, "OpenAerialMap did not answer.");
        assert!(!oam.busy());
    }
}
