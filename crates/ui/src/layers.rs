// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Раздел «Слои» левой панели и окно «Добавить данные», как `ui/panel.py`
//! (список `geo`) и `ui/nasamaps.py` (каталог) модуля.
//!
//! Строки сверху вниз: «Основа · название» с меню выбора, «Рельеф ·
//! Terrarium» с флажком и меню, включённые слои каталога, группы
//! векторной основы. Строка слоя каталога, снятая флажком, остаётся до
//! «Убрать с глобуса». Группа векторной основы видна, пока в ней что-то
//! включено, и после снятия флажков - до «Убрать с глобуса». В каталоге
//! Lite то, что Lite показывает: подоснова, векторная основа и сетка,
//! атмосфера и солнце.

use std::collections::BTreeSet;

use eframe::egui;

use crate::basemap::Basemap;
use crate::i18n::{Lang, tr};
use crate::vector::{Row, TREE, VectorBase};

/// Слой каталога, строка раздела «Слои» после основы и рельефа.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Extra {
    Air,
    Sun,
    Grid,
    /// Карта картинками, номер строки [crate::maps::ALL].
    Map(usize),
}

impl Extra {
    /// Порядок строк включённого, `ACTIVE_ORDER` модуля.
    pub const ALL: [Extra; 3] = [Extra::Air, Extra::Sun, Extra::Grid];

    /// Все слои каталога: свои и карты картинками.
    pub fn all() -> Vec<Extra> {
        let mut out = Self::ALL.to_vec();
        out.extend((0..crate::maps::ALL.len()).map(Extra::Map));
        out
    }

    /// Строка карты картинками.
    pub fn map(self) -> Option<&'static crate::maps::Entry> {
        match self {
            Extra::Map(n) => crate::maps::ALL.get(n),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Extra::Air => "Atmosphere",
            Extra::Sun => "Sun",
            Extra::Grid => "Grid",
            Extra::Map(_) => self.map().map_or("", |e| e.name),
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            Extra::Air => {
                "The blue glow of the air around the planet, the sky over the horizon and the haze over the distant surface. Without the check the background of the view is black space, the surface is seen without haze."
            }
            Extra::Sun => {
                "Light of the terrain and the air by the position of the sun. The night side of the Earth is dark. The time of the sun is the end of the period of the open time slider, without it the computer clock. Without the check the light falls from the north-west, as on a relief map."
            }
            Extra::Grid => {
                "Parallels and meridians with degree labels, the equator, the tropics and the polar circles. The step of the grid changes with the camera height."
            }
            Extra::Map(_) => self.map().map_or("", |e| e.tip),
        }
    }

    /// Категория каталога и источник второй строкой.
    fn category(self) -> Category {
        match self {
            Extra::Grid => Category::Map,
            Extra::Air | Extra::Sun => Category::Sky,
            Extra::Map(_) => match self.map().map(|e| e.group) {
                Some(crate::maps::Group::Fire) => Category::Fire,
                Some(crate::maps::Group::Gases) => Category::Gases,
                Some(crate::maps::Group::Water) => Category::Water,
                Some(crate::maps::Group::Life) => Category::Life,
                Some(crate::maps::Group::Soil) => Category::Soil,
                Some(crate::maps::Group::Tiles) => Category::Tiles,
                _ => Category::Depths,
            },
        }
    }

    /// Ключ в настройках.
    pub fn key(self) -> &'static str {
        match self {
            Extra::Air => "air",
            Extra::Sun => "sun",
            Extra::Grid => "grid",
            Extra::Map(_) => self.map().map_or("", |e| e.key),
        }
    }

    /// Слой по ключу настроек.
    pub fn from_key(key: &str) -> Option<Extra> {
        Self::all().into_iter().find(|e| e.key() == key)
    }
}

/// Что включено на глобусе, для раздела и каталога.
pub struct State<'a> {
    pub basemap: Basemap,
    pub relief: bool,
    /// Строки слоёв каталога в разделе: включённые и снятые флажком.
    pub extras: &'a BTreeSet<Extra>,
    /// Включённые слои каталога.
    pub on: &'a BTreeSet<Extra>,
    pub vector: &'a VectorBase,
}

/// Что попросили строки раздела или каталог.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Basemap(Basemap),
    Relief(bool),
    /// Слой каталога включить или снять, строка остаётся.
    Extra(Extra, bool),
    /// Строку слоя каталога убрать с глобуса.
    RemoveExtra(Extra),
    /// Новые строки векторной основы.
    Rows(BTreeSet<Row>),
    /// Группу векторной основы убрать с глобуса: её строки снимаются,
    /// группа скрывается.
    RemoveGroup(usize),
    /// Открыть окно «Источники данных».
    Sources,
    /// Открыть каталог «Добавить данные».
    Catalog,
}

/// Название основы в строке и меню, как в списке подложек модуля: Esri
/// World Imagery - пример подложки, её условия задаёт Esri.
pub fn basemap_title(lang: Lang, b: Basemap) -> &'static str {
    match b {
        Basemap::Esri => tr(lang, "Esri World Imagery - example"),
        Basemap::Osm => "OpenStreetMap",
        Basemap::Ready(_) => b.name(lang),
    }
}

/// Ширина зоны меню «⋯» у правого края строки, `MORE_WIDTH` модуля.
const MORE: f32 = 22.0;

/// Строка списка: флажок или значок, текст, «⋯» или «▾» справа. Возвращает
/// щелчок по флажку, щелчок по тексту и щелчок по зоне меню.
struct RowHit {
    check: bool,
    text: bool,
    menu: bool,
    rect: egui::Rect,
}

fn row(ui: &mut egui::Ui, indent: f32, check: Option<Option<bool>>, icon: bool, text: &str, marker: &str, tip: &str) -> RowHit {
    let height = 22.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::click());
    let p = crate::theme::pal();
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, 2.0, p.hover);
    }
    let mut x = rect.left() + indent;
    let mut check_rect = egui::Rect::NOTHING;
    if let Some(state) = check {
        check_rect = egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 7.0), egui::vec2(14.0, 14.0));
        match state {
            Some(true) => {
                painter.rect_filled(check_rect, 2.0, p.select);
                let q = |a: f32, b: f32| check_rect.left_top() + egui::vec2(14.0 * a, 14.0 * b);
                painter.line(vec![q(0.22, 0.52), q(0.42, 0.72), q(0.80, 0.30)], egui::Stroke::new(1.8, egui::Color32::WHITE));
            }
            // Часть строк группы включена - серый квадрат, как у модуля.
            None => {
                painter.rect_filled(check_rect, 2.0, p.muted);
            }
            Some(false) => {
                painter.rect(check_rect, 2.0, p.field, egui::Stroke::new(1.0, p.muted), egui::StrokeKind::Inside);
            }
        }
        x += 20.0;
    }
    if icon {
        // Значок растра у строки основы: клетки, как mIconRaster QGIS.
        let r = egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 7.0), egui::vec2(14.0, 14.0));
        let cell = 14.0 / 3.0;
        for i in 0..3 {
            for j in 0..3 {
                let c = if (i + j) % 2 == 0 { p.icon } else { p.muted };
                painter.rect_filled(egui::Rect::from_min_size(r.left_top() + egui::vec2(i as f32 * cell, j as f32 * cell), egui::vec2(cell, cell)), 0.0, c);
            }
        }
        x += 20.0;
    }
    let menu_rect = egui::Rect::from_min_max(egui::pos2(rect.right() - MORE, rect.top()), rect.right_bottom());
    let font = egui::FontId::proportional(13.0);
    let clip = painter.with_clip_rect(egui::Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(menu_rect.left(), rect.bottom())));
    clip.text(egui::pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, text, font.clone(), p.text);
    if !marker.is_empty() {
        painter.text(menu_rect.center(), egui::Align2::CENTER_CENTER, marker, font, p.muted);
    }
    let response = if tip.is_empty() { response } else { response.on_hover_text(tip) };
    let at = response.interact_pointer_pos();
    let clicked = response.clicked();
    let secondary = response.secondary_clicked();
    let on_check = at.is_some_and(|p| check_rect.expand(3.0).contains(p));
    let on_menu = at.is_some_and(|p| menu_rect.contains(p));
    RowHit { check: clicked && on_check, text: clicked && !on_check && !on_menu, menu: (clicked && on_menu) || secondary, rect }
}

/// Раздел «Слои»: кнопка «Добавить данные» и строки. menu_open - какое
/// меню строки открыто, хранится окном.
pub fn panel(ui: &mut egui::Ui, lang: Lang, state: &State<'_>, menu_open: &mut Option<String>, out: &mut Vec<Action>) {
    let add = ui.add_sized([ui.available_width(), 24.0], egui::Button::new(tr(lang, "Add data")));
    if add.on_hover_text(tr(lang, "Catalog of layers - vector base, coordinate grid, atmosphere and sun. The catalog opens in its own window, it can stay open next to the globe.")).clicked() {
        out.push(Action::Catalog);
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    // Основа: щелчок по строке - меню выбора подложки.
    let title = format!("{} · {}", tr(lang, "Base map"), basemap_title(lang, state.basemap));
    let hit = row(ui, 4.0, None, true, &title, "▾", tr(lang, "Imagery or map on the surface. A click on the row chooses the basemap. Esri World Imagery is an example basemap, its terms are set by Esri."));
    if hit.text || hit.menu {
        *menu_open = Some("basemap".into());
    }
    popup(ui, menu_open, "basemap", hit.rect, |ui| {
        for b in Basemap::ALL {
            if ui.radio(state.basemap == b, basemap_title(lang, b)).clicked() {
                out.push(Action::Basemap(b));
                return true;
            }
        }
        // Готовые источники подменю по группам, как `READY_GROUPS` модуля.
        ui.separator();
        let mut chosen = false;
        for group in crate::basemap::ReadyGroup::ALL {
            ui.menu_button(tr(lang, group.name()), |ui| {
                for b in Basemap::all().into_iter().filter(|b| b.ready().is_some_and(|r| r.group == group)) {
                    if ui.radio(state.basemap == b, basemap_title(lang, b)).clicked() {
                        out.push(Action::Basemap(b));
                        chosen = true;
                    }
                }
            });
        }
        chosen
    });
    // Рельеф: флажок - рельеф, текст - меню.
    let title = format!("{} · Terrarium", tr(lang, "Terrain"));
    let hit = row(ui, 4.0, Some(Some(state.relief)), false, &title, "▾", tr(lang, "Heights raise the surface and shade the slopes. The check turns the terrain on, a click on the name chooses the heights. Heights of the Earth - Mapzen Terrain Tiles. The vertical scale is in the view properties."));
    if hit.check {
        out.push(Action::Relief(!state.relief));
    }
    if hit.text || hit.menu {
        *menu_open = Some("relief".into());
    }
    popup(ui, menu_open, "relief", hit.rect, |ui| {
        let _ = ui.radio(true, tr(lang, "No - height tiles only"));
        ui.separator();
        if ui.button(tr(lang, "Data sources…")).clicked() {
            out.push(Action::Sources);
            return true;
        }
        false
    });
    // Включённые слои каталога.
    for extra in Extra::all() {
        if !state.extras.contains(&extra) {
            continue;
        }
        let on = extra_on(state, extra);
        let tip = crate::i18n::trf(lang, "{name}. The check turns the layer off, the row stays. The «⋯» menu removes it from the globe.", &[("name", tr(lang, extra.name()))]);
        let hit = row(ui, 4.0, Some(Some(on)), false, tr(lang, extra.name()), "⋯", &tip);
        if hit.check || hit.text {
            out.push(Action::Extra(extra, !on));
        }
        let key = format!("extra:{}", extra.key());
        if hit.menu {
            *menu_open = Some(key.clone());
        }
        popup(ui, menu_open, &key, hit.rect, |ui| {
            if ui.button(tr(lang, "Remove from the globe")).clicked() {
                out.push(Action::RemoveExtra(extra));
                return true;
            }
            false
        });
    }
    // Группы векторной основы.
    for (n, (title, tip, rows)) in TREE.iter().enumerate() {
        if !state.vector.seen.contains(&n) {
            continue;
        }
        let on = rows.iter().filter(|(r, _, _)| state.vector.rows.contains(r)).count();
        let check = if on == 0 {
            Some(false)
        } else if on == rows.len() {
            Some(true)
        } else {
            None
        };
        let id = ui.make_persistent_id(("vector-group", n));
        let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(id, false));
        let arrow = if open { "▾" } else { "▸" };
        let hit = row(ui, 4.0, Some(check), false, &format!("{arrow} {}", tr(lang, title)), "⋯", tr(lang, tip));
        if hit.check {
            let mut rows_now = state.vector.rows.clone();
            for (r, _, _) in rows.iter() {
                if check == Some(true) {
                    rows_now.remove(r);
                } else {
                    rows_now.insert(*r);
                }
            }
            out.push(Action::Rows(rows_now));
        } else if hit.text {
            open = !open;
            ui.data_mut(|d| d.insert_persisted(id, open));
        }
        let key = format!("group:{n}");
        if hit.menu {
            *menu_open = Some(key.clone());
        }
        popup(ui, menu_open, &key, hit.rect, |ui| {
            if ui.button(tr(lang, "Remove from the globe")).clicked() {
                out.push(Action::RemoveGroup(n));
                return true;
            }
            false
        });
        if open {
            for (r, name, row_tip) in rows.iter() {
                let checked = state.vector.rows.contains(r);
                let hit = row(ui, 24.0, Some(Some(checked)), false, tr(lang, name), "", tr(lang, row_tip));
                if hit.check || hit.text {
                    let mut rows_now = state.vector.rows.clone();
                    if checked {
                        rows_now.remove(r);
                    } else {
                        rows_now.insert(*r);
                    }
                    out.push(Action::Rows(rows_now));
                }
            }
        }
    }
}

fn extra_on(state: &State<'_>, extra: Extra) -> bool {
    state.on.contains(&extra)
}

/// Меню строки под строкой rect, открыто, пока menu_open - его ключ.
/// add возвращает true, когда пункт выбран: меню закрывается. Щелчок
/// мимо меню и Escape тоже закрывают его.
fn popup(ui: &mut egui::Ui, menu_open: &mut Option<String>, key: &str, rect: egui::Rect, add: impl FnOnce(&mut egui::Ui) -> bool) {
    if menu_open.as_deref() != Some(key) {
        return;
    }
    let id = egui::Id::unique(("layers-menu", key));
    let area = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(rect.left_bottom()).show(ui.ctx(), |ui| {
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            ui.set_min_width(rect.width().min(260.0));
            add(ui)
        })
        .inner
    });
    let outside = ui.ctx().input(|i| i.pointer.any_click() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p) && !rect.contains(p)));
    let escape = ui.ctx().input(|i| i.key_pressed(egui::Key::Escape));
    if area.inner || outside || escape {
        *menu_open = None;
    }
}

/// Категория каталога, как у модуля.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    All,
    Base,
    Map,
    Fire,
    Water,
    Gases,
    Life,
    Sky,
    Depths,
    Soil,
    Tiles,
}

/// Окно каталога «Добавить данные»: открыто ли, категория, поиск,
/// выбранная строка.
#[derive(Debug, Default)]
pub struct Catalog {
    pub open: bool,
    category: Option<usize>,
    search: String,
    chosen: Option<String>,
}

/// Строка каталога.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Item {
    Base(Basemap),
    Group(usize),
    Extra(Extra),
}

impl Item {
    fn key(self) -> String {
        match self {
            Item::Base(b) => format!("base:{}", b.key()),
            Item::Group(n) => format!("vector:{n}"),
            Item::Extra(e) => e.key().to_string(),
        }
    }

    fn category(self) -> Category {
        match self {
            Item::Base(_) => Category::Base,
            Item::Group(_) => Category::Map,
            Item::Extra(e) => e.category(),
        }
    }

    fn name(self, lang: Lang) -> String {
        match self {
            Item::Base(b) => basemap_title(lang, b).to_string(),
            Item::Group(n) => tr(lang, TREE[n].0).to_string(),
            Item::Extra(e) => tr(lang, e.name()).to_string(),
        }
    }

    fn tip(self, lang: Lang) -> String {
        match self {
            Item::Base(Basemap::Esri) => tr(lang, "Imagery of the whole Earth, the example basemap. Its terms are set by Esri.").to_string(),
            Item::Base(Basemap::Osm) => tr(lang, "The OpenStreetMap map.").to_string(),
            Item::Base(b) => b.attribution(lang).0.to_string(),
            Item::Group(n) => tr(lang, TREE[n].1).to_string(),
            Item::Extra(e) => tr(lang, e.tip()).to_string(),
        }
    }

    /// Вторая строка: источник, `subtitle` модуля.
    fn subtitle(self) -> &'static str {
        match self {
            Item::Base(Basemap::Esri) => "Esri",
            Item::Base(Basemap::Osm) => "OpenStreetMap",
            Item::Base(b) => b.ready().map_or("", |r| r.group.name()),
            Item::Group(_) => "OpenFreeMap",
            Item::Extra(e) => e.map().map_or("", |m| m.source),
        }
    }

    fn is_on(self, state: &State<'_>) -> bool {
        match self {
            Item::Base(b) => state.basemap == b,
            Item::Group(n) => TREE[n].2.iter().any(|(r, _, _)| state.vector.rows.contains(r)),
            Item::Extra(e) => state.on.contains(&e),
        }
    }
}

fn items() -> Vec<Item> {
    let mut out: Vec<Item> = Basemap::all().into_iter().map(Item::Base).collect();
    out.extend((0..TREE.len()).map(Item::Group));
    out.push(Item::Extra(Extra::Grid));
    out.push(Item::Extra(Extra::Air));
    out.push(Item::Extra(Extra::Sun));
    out.extend((0..crate::maps::ALL.len()).map(|n| Item::Extra(Extra::Map(n))));
    out
}

/// Строки группы n включить или снять, `_clicked` каталога модуля:
/// что-то включено - снимаются все, иначе включаются все.
pub fn toggle_group(vector: &VectorBase, n: usize) -> BTreeSet<Row> {
    let keys: BTreeSet<Row> = TREE[n].2.iter().map(|(r, _, _)| *r).collect();
    let mut rows = vector.rows.clone();
    if keys.iter().any(|k| rows.contains(k)) {
        rows.retain(|r| !keys.contains(r));
    } else {
        rows.extend(keys);
    }
    rows
}

impl Catalog {
    /// Окно каталога. Действия - в out.
    pub fn window(&mut self, ctx: &egui::Context, lang: Lang, state: &State<'_>, out: &mut Vec<Action>) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        let categories = [
            (Category::All, tr(lang, "All"), tr(lang, "All maps and layers of the catalog.")),
            (Category::Base, tr(lang, "Base maps"), tr(lang, "Basemaps of the globe - imagery and maps. There is one base on the globe, under the layers.")),
            (Category::Map, tr(lang, "Map"), tr(lang, "The OpenFreeMap vector base and the coordinate grid.")),
            (Category::Fire, tr(lang, "Planet of Fire"), tr(lang, "Fire spots, smoke, aerosol and carbon monoxide of fires and industry from NASA data.")),
            (Category::Water, tr(lang, "Planet of Water"), tr(lang, "Precipitation, soil moisture, snow, ice, water vapor, chlorophyll, salinity and floods from NASA data, currents from NOAA data.")),
            (Category::Gases, tr(lang, "Planet of Air"), tr(lang, "Nitrogen dioxide, sulfur dioxide, methane, carbon dioxide and ozone from NASA data.")),
            (Category::Life, tr(lang, "Land and Life"), tr(lang, "Vegetation, dust, night lights and land cover types from NASA data.")),
            (Category::Sky, tr(lang, "Sky and light"), tr(lang, "The sun and the night side, the atmosphere.")),
            (Category::Depths, tr(lang, "Inside the Earth"), tr(lang, "The geological and lithological maps, earthquakes, plate boundaries and faults, mineral deposits and mining areas, the Earth cutaway and palaeogeography.")),
            (Category::Soil, tr(lang, "Soils"), tr(lang, "Properties of the topsoil 0-5 cm from SoilGrids - clay, sand, organic carbon and pH, a 250 m grid.")),
            (Category::Tiles, tr(lang, "Map overlays"), tr(lang, "Transparent maps over the base map - sea marks, hiking and cycling routes. Several can be on at once.")),
        ];
        egui::Window::new(tr(lang, "Add data")).open(&mut open).default_size([780.0, 520.0]).show(ctx, |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(tr(lang, "snow, mines, wind")).desired_width(f32::INFINITY));
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(116.0);
                    for (i, (_, title, tip)) in categories.iter().enumerate() {
                        let current = self.category.unwrap_or(0) == i;
                        if ui.selectable_label(current, *title).on_hover_text(*tip).clicked() {
                            self.category = Some(i);
                        }
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    let category = categories[self.category.unwrap_or(0)].0;
                    let text = self.search.trim().to_lowercase();
                    let fits = |item: Item| {
                        if !text.is_empty() {
                            item.name(lang).to_lowercase().contains(&text) || item.tip(lang).to_lowercase().contains(&text)
                        } else if category == Category::All {
                            !matches!(item, Item::Base(_))
                        } else {
                            item.category() == category
                        }
                    };
                    let shown: Vec<Item> = items().into_iter().filter(|i| fits(*i)).collect();
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        let bases: Vec<Item> = shown.iter().copied().filter(|i| matches!(i, Item::Base(_))).collect();
                        let layers: Vec<Item> = shown.iter().copied().filter(|i| !matches!(i, Item::Base(_))).collect();
                        for (head, list) in [(tr(lang, "Base map on the globe - one"), bases), (tr(lang, "Layers - switched on together"), layers)] {
                            if list.is_empty() {
                                continue;
                            }
                            ui.label(egui::RichText::new(head).strong());
                            for item in list {
                                let mut on = item.is_on(state);
                                ui.horizontal(|ui| {
                                    if crate::theme::checkbox(ui, &mut on).clicked() {
                                        self.chosen = Some(item.key());
                                        click(item, state, out);
                                    }
                                    let sub = item.subtitle();
                                    let label = if sub.is_empty() { item.name(lang) } else { format!("{}\n{}", item.name(lang), sub) };
                                    let selected = self.chosen.as_deref() == Some(&item.key());
                                    let r = ui.selectable_label(selected, label).on_hover_text(item.tip(lang));
                                    if r.clicked() {
                                        self.chosen = Some(item.key());
                                    }
                                    if r.double_clicked() {
                                        click(item, state, out);
                                    }
                                });
                            }
                        }
                    });
                    ui.separator();
                    match items().into_iter().find(|i| self.chosen.as_deref() == Some(&i.key())) {
                        Some(item) => {
                            ui.label(egui::RichText::new(item.name(lang)).strong());
                            if !item.subtitle().is_empty() {
                                ui.label(egui::RichText::new(item.subtitle()).italics());
                            }
                            ui.label(item.tip(lang));
                            let on = item.is_on(state);
                            let (text, enabled) = match item {
                                Item::Base(_) if on => (tr(lang, "Base map on the globe"), false),
                                Item::Base(_) => (tr(lang, "Set as base map"), true),
                                _ if on => (tr(lang, "Remove from the globe"), true),
                                _ => (tr(lang, "Show on the globe"), true),
                            };
                            if ui.add_enabled(enabled, egui::Button::new(text)).clicked() {
                                click(item, state, out);
                            }
                        }
                        None => {
                            ui.label(tr(lang, "The check shows a layer on the globe. A row - its description. Layers are turned on together."));
                        }
                    }
                });
            });
        });
        self.open = open;
    }
}

/// Строка каталога на глобус или с глобуса.
fn click(item: Item, state: &State<'_>, out: &mut Vec<Action>) {
    match item {
        // Подоснова не снимается, щелчок по включённой - ничего.
        Item::Base(b) => {
            if state.basemap != b {
                out.push(Action::Basemap(b));
            }
        }
        Item::Group(n) => out.push(Action::Rows(toggle_group(state.vector, n))),
        Item::Extra(e) => {
            if state.on.contains(&e) {
                out.push(Action::RemoveExtra(e));
            } else {
                out.push(Action::Extra(e, true));
            }
        }
    }
}
