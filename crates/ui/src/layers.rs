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
    Stars,
    Sun,
    Grid,
    /// Карта картинками, номер строки [`crate::maps::ALL`].
    Map(usize),
    /// Слой точек или линий: землетрясения, вулканы, пожары, разломы,
    /// границы плит, кабели.
    Dots(crate::dots::Layer),
    /// Разрез Земли: сектор под точкой взгляда.
    Cutaway,
    /// Палеогеография: основа - карта возраста.
    Paleo,
    /// Геоид: высоты Земли - геоид, растянутый.
    Geoid,
    /// Слой реестра [`crate::registry::ALL`], номер строки.
    Plugged(usize),
}

impl Extra {
    /// Порядок строк включённого, `ACTIVE_ORDER` модуля.
    pub const ALL: [Extra; 4] = [Extra::Air, Extra::Stars, Extra::Sun, Extra::Grid];

    /// Все слои каталога: свои и карты картинками.
    pub fn all() -> Vec<Extra> {
        let mut out = Self::ALL.to_vec();
        out.extend((0..crate::maps::count()).map(Extra::Map));
        out.extend(crate::dots::Layer::ALL.map(Extra::Dots));
        out.push(Extra::Cutaway);
        out.push(Extra::Paleo);
        out.push(Extra::Geoid);
        out.extend((0..crate::registry::ALL.len()).map(Extra::Plugged));
        out
    }

    /// Слой есть только у Земли, `EARTH_EXTRAS` модуля. На любом теле -
    /// атмосфера, звёзды и сетка.
    pub fn earth_only(self) -> bool {
        !matches!(self, Extra::Air | Extra::Stars | Extra::Grid)
    }

    /// Слоя нет у страницы: самолёты ADS-B и кадастр НСПД не отдают
    /// данные браузеру. Страница их строк не показывает. Строка реестра
    /// говорит это сама, её проверка уже учитывает страницу.
    pub fn desktop_only(self) -> bool {
        cfg!(target_arch = "wasm32")
            && match self {
                Extra::Plugged(n) => crate::registry::get(n).is_some_and(|e| e.desktop_only()),
                e => e.map().is_some_and(|m| m.key == "cadastre"),
            }
    }

    /// Строка карты картинками.
    pub fn map(self) -> Option<&'static crate::maps::Entry> {
        match self {
            Extra::Map(n) => crate::maps::entry(n),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Extra::Air => "Atmosphere",
            Extra::Stars => "Stars",
            Extra::Sun => "Sun",
            Extra::Grid => "Grid",
            Extra::Map(_) => self.map().map_or("", |e| e.name),
            Extra::Dots(l) => l.name(),
            Extra::Cutaway => "Earth cutaway",
            Extra::Paleo => "Paleogeography",
            Extra::Geoid => "Geoid",
            Extra::Plugged(n) => crate::registry::get(n).map_or("", |e| e.name()),
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            Extra::Stars => {
                "Stars of the Yale Bright Star Catalogue. The stars show from space and fade when the camera descends into the atmosphere."
            }
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
            Extra::Dots(l) => l.tip(),
            Extra::Plugged(n) => crate::registry::get(n).map_or("", |e| e.tip()),
            Extra::Geoid => "The shape of the Earth level surface - the geoid - after the NGA EGM2008 model. While the row is on, the globe heights are the geoid heights above the ellipsoid, from -107 to +86 m, stretched 10 000 times, and the Earth looks crumpled. Bulges are where gravity is stronger, for example near New Guinea, hollows where it is weaker, the deepest one in the Indian Ocean. Terrain and water are removed for this time, switching the row off brings them back.",
            Extra::Paleo => "Land relief and sea depths in the past, up to 540 million years ago, after the PALEOMAP PaleoDEM maps. The slider at the bottom of the view sets the age. The imagery, borders and labels are removed meanwhile.",
            Extra::Cutaway => {
                "Removes a sector of the Earth under the look-at point, a quarter of a hemisphere 90° wide in longitude. Its faces show the crust, the mantle and the core by the radii of the PREM model, the crust by the CRUST1.0 model. Where a face crosses a subduction zone, the subducting slab shows on it. The sector is placed anew each time the row is switched on."
            }
        }
    }

    /// Категория каталога и источник второй строкой.
    fn category(self) -> Category {
        match self {
            Extra::Grid => Category::Map,
            Extra::Air | Extra::Stars | Extra::Sun => Category::Sky,
            Extra::Map(_) => match self.map().map(|e| e.group) {
                Some(crate::maps::Group::Map) => Category::Map,
                Some(crate::maps::Group::Sky) => Category::Sky,
                Some(crate::maps::Group::Terrain) => Category::Terrain,
                Some(crate::maps::Group::Fire) => Category::Fire,
                Some(crate::maps::Group::Gases) => Category::Gases,
                Some(crate::maps::Group::Water) => Category::Water,
                Some(crate::maps::Group::Life) => Category::Life,
                Some(crate::maps::Group::Soil) => Category::Soil,
                Some(crate::maps::Group::Tiles) => Category::Tiles,
                Some(crate::maps::Group::Weather) => Category::Weather,
                Some(crate::maps::Group::Gibs) => Category::Gibs,
                Some(crate::maps::Group::Local) => Category::Local,
                _ => Category::Depths,
            },
            Extra::Dots(l) => l.category(),
            Extra::Cutaway | Extra::Paleo | Extra::Geoid => Category::Depths,
            Extra::Plugged(n) => crate::registry::get(n).map_or(Category::Map, |e| e.category()),
        }
    }

    /// Ключ в настройках.
    pub fn key(self) -> &'static str {
        match self {
            Extra::Air => "air",
            Extra::Stars => "stars",
            Extra::Sun => "sun",
            Extra::Grid => "grid",
            Extra::Map(_) => self.map().map_or("", |e| e.key),
            Extra::Dots(l) => l.key(),
            Extra::Cutaway => "cutaway",
            Extra::Paleo => "paleo",
            Extra::Geoid => "geoid",
            Extra::Plugged(n) => crate::registry::get(n).map_or("", |e| e.key()),
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
    /// Свои растры рельефа.
    pub own: Vec<String>,
    /// Флажок «Глубины морей и океанов».
    pub sea: bool,
    /// Непрозрачность земли над подземной моделью, None - модели нет.
    pub ground: Option<f32>,
    /// Непрозрачность карт картинками по ключу, без записи - 1.
    pub opacity: &'a std::collections::HashMap<String, f32>,
    /// Пункт «Растягивать кору» меню строки разреза Земли.
    pub stretch: bool,
    /// Масштаб режима «Геоид».
    pub geoid_scale: f64,
    /// Выбранные группы спутников.
    pub sat_groups: &'a [String],
    /// На глобусе Земля. На другом теле земные строки каталога - заглушки.
    pub earth: bool,
    /// Снимки события на глобусе.
    pub events: bool,
    /// «Мои слои» - векторные слои файлов и результатов.
    pub data: &'a crate::datalayers::DataLayers,
    /// Наборы 3D Tiles.
    pub tiles: &'a crate::tilesets::Tilesets,
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
    /// Объекты строки в рамке вида - слоями «Моих слоёв».
    ExtraToLayers(Extra),
    /// Новые строки векторной основы.
    Rows(BTreeSet<Row>),
    /// Группу векторной основы убрать с глобуса: её строки снимаются,
    /// группа скрывается.
    RemoveGroup(usize),
    /// Открыть окно «Источники данных».
    Sources,
    /// Новые группы спутников.
    SatGroups(Vec<String>),
    /// Открыть каталог «Добавить данные».
    Catalog,
    /// Окно «Батиметрия водоёма» своего растра.
    Bathymetry(String),
    /// Окно «Выемка и насыпь»: растр - новая съёмка.
    Volumes(String),
    /// Изолинии растра слоем «Моих слоёв».
    Isolines(String),
    /// Глубины морей и океанов включить или снять.
    Sea(bool),
    /// Непрозрачность земли над подземной моделью.
    Ground(f32),
    /// Открыть окно «Подземный режим».
    Subsurface,
    /// Непрозрачность карты картинками: ключ строки и доля.
    Opacity(&'static str, f32),
    /// «Растягивать кору» разреза Земли.
    Stretch(bool),
    /// Масштаб режима «Геоид».
    GeoidScale(f64),
    /// Окно «Новый источник тайлов».
    AddSource,
    /// Попросить весь каталог NASA GIBS.
    GibsCatalog,
    /// Открыть окно «Снимки событий».
    Events,
    /// Свой источник убрать из списка основ.
    RemoveOwn(Basemap),
    /// Действие меню слоя «Моих слоёв» по его номеру, у папки номер 0.
    Data(u64, crate::datalayers::Act),
    /// Действие меню набора 3D Tiles по его номеру.
    Tiles(u64, crate::tilesets::Act),
}

/// Название основы в строке и меню, как в списке подложек модуля: Esri
/// World Imagery - пример подложки, её условия задаёт Esri.
pub fn basemap_title(lang: Lang, b: Basemap) -> &'static str {
    match b {
        Basemap::Esri => tr(lang, "Esri World Imagery - example"),
        Basemap::Osm => "OpenStreetMap",
        Basemap::Ready(_) | Basemap::Own(_) | Basemap::Paleo(_) | Basemap::Body(_) => b.name(lang),
    }
}

/// Ширина зоны меню «⋯» у правого края строки, `MORE_WIDTH` модуля.
const MORE: f32 = 22.0;

/// Строка списка: флажок или значок, текст, «⋯» или «▾» справа. Возвращает
/// щелчок по флажку, щелчок по тексту и щелчок по зоне меню.
pub(crate) struct RowHit {
    pub check: bool,
    pub text: bool,
    pub menu: bool,
    pub rect: egui::Rect,
    /// Ответ строки: двойной щелчок, перетаскивание.
    pub response: egui::Response,
}

pub(crate) fn row(ui: &mut egui::Ui, indent: f32, check: Option<Option<bool>>, icon: bool, text: &str, marker: &str, tip: &str) -> RowHit {
    row_with(ui, egui::Sense::click(), false, indent, check, icon, text, marker, tip)
}

/// Строка, которую можно тянуть мышью, selected - выделенная строка,
/// как строка слоя «Проекта QGIS» модуля.
#[allow(clippy::too_many_arguments)]
pub(crate) fn drag_row(ui: &mut egui::Ui, selected: bool, indent: f32, check: Option<Option<bool>>, text: &str, marker: &str, tip: &str) -> RowHit {
    row_with(ui, egui::Sense::click_and_drag(), selected, indent, check, false, text, marker, tip)
}

#[allow(clippy::too_many_arguments)]
fn row_with(ui: &mut egui::Ui, sense: egui::Sense, selected: bool, indent: f32, check: Option<Option<bool>>, icon: bool, text: &str, marker: &str, tip: &str) -> RowHit {
    let height = 22.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), sense);
    let p = crate::theme::pal();
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 2.0, p.select.gamma_multiply(0.35));
    } else if response.hovered() {
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
    // Пока строку тянут, подсказка не мешает.
    let response = if tip.is_empty() || response.dragged() { response } else { response.on_hover_text(tip) };
    let at = response.interact_pointer_pos();
    let clicked = response.clicked();
    let secondary = response.secondary_clicked();
    let on_check = at.is_some_and(|p| check_rect.expand(3.0).contains(p));
    let on_menu = at.is_some_and(|p| menu_rect.contains(p));
    RowHit { check: clicked && on_check, text: clicked && !on_check && !on_menu, menu: (clicked && on_menu) || secondary, rect, response }
}

/// Раздел «Слои»: кнопка «Добавить данные» и строки. menu_open - какое
/// меню строки открыто, хранится окном.
pub fn panel(ui: &mut egui::Ui, lang: Lang, state: &State<'_>, menu_open: &mut Option<String>, out: &mut Vec<Action>) {
    let add = ui.add_sized([ui.available_width(), 24.0], egui::Button::new(tr(lang, "Add data")));
    if add.on_hover_text(tr(lang, "Catalog of layers - vector base, coordinate grid, atmosphere and sun. The catalog opens in its own window, it can stay open next to the globe.")).clicked() {
        out.push(Action::Catalog);
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    // «Мои слои» - свой раздел панели над «Слоями», как «Проект QGIS»
    // модуля, решение автора от 6 ноября 2026 года.
    let mut tiles = Vec::new();
    crate::tilesets::rows(ui, lang, state.tiles, menu_open, &mut tiles);
    if !tiles.is_empty() || !state.tiles.list.is_empty() {
        ui.add_space(4.0);
    }
    out.extend(tiles.into_iter().map(|(id, act)| Action::Tiles(id, act)));
    // Основа: щелчок по строке - меню выбора подложки.
    let title = format!("{} · {}", tr(lang, "Base map"), basemap_title(lang, state.basemap));
    let hit = row(ui, 4.0, None, true, &title, "▾", tr(lang, "Imagery or map on the surface. A click on the row chooses the basemap. Esri World Imagery is an example basemap, its terms are set by Esri."));
    // На другом теле основа - снимки тела, выбора нет.
    if (hit.text || hit.menu) && state.earth {
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
        // Свои источники тайлов, «Добавить источник тайлов…» модуля.
        let owns = Basemap::owns();
        if !owns.is_empty() {
            ui.separator();
        }
        for b in owns {
            ui.horizontal(|ui| {
                if ui.radio(state.basemap == b, basemap_title(lang, b)).clicked() {
                    out.push(Action::Basemap(b));
                    chosen = true;
                }
                if ui.small_button("✕").on_hover_text(tr(lang, "Remove this tile source from the list.")).clicked() {
                    out.push(Action::RemoveOwn(b));
                    chosen = true;
                }
            });
        }
        if ui.button(tr(lang, "Add tile source…")).clicked() {
            out.push(Action::AddSource);
            chosen = true;
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
        // Свои растры рельефа: у каждого - батиметрия водоёма, как пункт
        // меню растра «Слоёв проекта» модуля.
        let mut chosen = false;
        for name in &state.own {
            ui.menu_button(name.as_str(), |ui| {
                if ui.button(tr(lang, "Water body bathymetry…")).clicked() {
                    out.push(Action::Bathymetry(name.clone()));
                    chosen = true;
                }
                if ui.button(tr(lang, "Cut and fill…")).on_hover_text(tr(lang, "The raster is a new terrain survey, for example of a pit with dumps. The difference with an earlier survey gives cut and fill, their volumes and a difference raster.")).clicked() {
                    out.push(Action::Volumes(name.clone()));
                    chosen = true;
                }
                if ui.button(tr(lang, "Isolines to My layers")).on_hover_text(tr(lang, "Isolines of the raster values become a layer of «My layers» with the elev field, about 12 lines over the range of values.")).clicked() {
                    out.push(Action::Isolines(name.clone()));
                    chosen = true;
                }
            });
        }
        if chosen {
            return true;
        }
        ui.separator();
        let mut sea = state.sea;
        if ui.checkbox(&mut sea, tr(lang, "Sea and ocean depths")).on_hover_text(tr(lang, "The sea and ocean floor below zero and water above it. Without the check the sea surface is flat, at zero.")).changed() {
            out.push(Action::Sea(sea));
            return true;
        }
        ui.separator();
        if ui.button(tr(lang, "Data sources…")).clicked() {
            out.push(Action::Sources);
            return true;
        }
        false
    });
    // Земля над подземной моделью, видна, пока модель есть, как строка
    // «Земля над гридами» модуля.
    if let Some(alpha) = state.ground {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label(tr(lang, "Ground above the grids"));
            let mut value = alpha;
            let slider = ui.add(egui::Slider::new(&mut value, 0.0..=1.0).show_value(false)).on_hover_text(tr(
                lang,
                "Opacity of the ground above the subsurface model - grids by elevations, roofs, drill holes. To the right the ground is opaque, to the left more transparent, at the left edge it is not seen. Outside the data outline the ground does not change.",
            ));
            if slider.changed() {
                out.push(Action::Ground(value));
            }
            if ui.small_button("…").on_hover_text(tr(lang, "The Subsurface mode window - the block cut and the camera under the ground.")).clicked() {
                out.push(Action::Subsurface);
            }
        });
    }
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
            // Непрозрачность карты, меню «⋯» строки включённого модуля.
            if let Some(entry) = extra.map() {
                let mut value = state.opacity.get(entry.key).copied().unwrap_or(1.0) * 100.0;
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "Opacity"));
                    let slider = ui.add(egui::Slider::new(&mut value, 0.0..=100.0).fixed_decimals(0).suffix(" %"));
                    if slider.on_hover_text(tr(lang, "Opacity of the layer. To the right the layer is opaque, to the left more transparent.")).changed() {
                        out.push(Action::Opacity(entry.key, value / 100.0));
                    }
                });
            }
            if extra == Extra::Geoid {
                // Масштаб ступенями, `_geoid_scale_action` модуля.
                let steps = data::geoid::SCALES;
                let mut step = data::geoid::scale_step(state.geoid_scale);
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "Scale"));
                    let slider = ui.add(egui::Slider::new(&mut step, 0..=steps.len() - 1).show_value(false));
                    ui.label(data::geoid::scale_text(steps[step.min(steps.len() - 1)]));
                    if slider.on_hover_text(tr(lang, "How many times the geoid heights above the ellipsoid are stretched. To the right the bulges and hollows are deeper, to the left the Earth is rounder.")).changed() {
                        out.push(Action::GeoidScale(steps[step.min(steps.len() - 1)]));
                    }
                });
            }
            if extra == Extra::Cutaway {
                let mut stretch = state.stretch;
                if ui.checkbox(&mut stretch, tr(lang, "Stretch the crust")).on_hover_text(tr(lang, "Stretches depths to 400 km, the more the farther the camera, and the crust and the upper mantle show from afar. Without the check all depths are in true scale.")).changed() {
                    out.push(Action::Stretch(stretch));
                    return true;
                }
            }
            // «В проект QGIS…» строки витрины модуля: точки и линии в рамке
            // вида - слоями «Моих слоёв».
            if matches!(extra, Extra::Dots(_))
                && on
                && ui.button(tr(lang, "View objects to My layers")).on_hover_text(tr(lang, "Points and lines of the layer in the frame of the view become layers of «My layers» with the fields of the Objects window and the colours of the globe.")).clicked()
            {
                out.push(Action::ExtraToLayers(extra));
                return true;
            }
            // Карта картинками - картинкой рамки вида, как раскраска вида
            // строки модуля в проект.
            if matches!(extra, Extra::Map(_))
                && on
                && ui.button(tr(lang, "View picture to My layers")).on_hover_text(tr(lang, "The map in the frame of the view, as the globe draws it, becomes a picture layer of «My layers». It can be saved as GeoTIFF.")).clicked()
            {
                out.push(Action::ExtraToLayers(extra));
                return true;
            }
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
pub(crate) fn popup(ui: &mut egui::Ui, menu_open: &mut Option<String>, key: &str, rect: egui::Rect, add: impl FnOnce(&mut egui::Ui) -> bool) {
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
pub enum Category {
    All,
    Base,
    /// Службы WMS районов и свои службы WMS.
    Local,
    Map,
    Fire,
    Water,
    Weather,
    Gases,
    Life,
    Sky,
    Depths,
    Soil,
    Terrain,
    Tiles,
    Infra,
    Transport,
    Wiki,
    Satellites,
    /// Весь каталог NASA GIBS: строки приходят по запросу.
    Gibs,
    /// Архив снимков: снимки событий.
    Archive,
}

/// Знаков поиска, с которых просится каталог GIBS, `GIBS_SEARCH` модуля.
const GIBS_SEARCH: usize = 3;

/// Разделы левого столбца каталога: название, подсказка, категории по
/// порядку, `catalog_sections` модуля, раскладка - решение автора от
/// 27 октября 2026 года. Раздел из одной категории стоит строкой этой
/// категории.
const SECTIONS: [(&str, &str, &[Category]); 7] = [
    ("Basis", "Base maps, local data, the vector map, transparent map overlays and Wikipedia articles.", &[Category::Base, Category::Local, Category::Map, Category::Tiles, Category::Wiki]),
    ("Earth now", "Weather, NASA imagery and maps by days - fire, water, air, life, and soils.", &[Category::Weather, Category::Fire, Category::Water, Category::Gases, Category::Life, Category::Soil, Category::Gibs]),
    ("Archive and survey", "", &[Category::Archive]),
    ("Inside the Earth", "", &[Category::Depths]),
    ("Sky", "The sun, stars, the atmosphere and artificial satellites.", &[Category::Sky, Category::Satellites]),
    ("Energy and transport", "Power plants, power lines, pipelines, cables, trains, ships and aircraft.", &[Category::Infra, Category::Transport]),
    ("", "", &[Category::Terrain]),
];

/// Ширина столбца разделов в точках: «Энергетика и транспорт» жирным
/// встаёт в одну строку. У модуля 150 пикселей при своём шрифте. Выбор
/// помощника.
const CATEGORY_WIDTH: f32 = 196.0;
/// Высота строки списка в точках: две строки текста и флажок по середине.
const ROW_HEIGHT: f32 = 40.0;
/// Высота под списком: описание выбранной строки и кнопка.
const DETAILS_HEIGHT: f32 = 150.0;
/// Высота столбцов окна в точках, от и до: у модуля окно 780 × 640.
const BODY_MIN: f32 = 320.0;
const BODY_MAX: f32 = 660.0;

/// Строка левого столбца: название, подсказка, категории, заголовок
/// раздела. Пустые категории - «Все».
struct Pick<'a> {
    title: &'a str,
    tip: &'a str,
    categories: Vec<Category>,
    head: bool,
    indent: bool,
}

/// Окно каталога «Добавить данные»: открыто ли, категория, поиск,
/// выбранная строка.
#[derive(Debug, Default)]
pub struct Catalog {
    pub open: bool,
    category: Option<usize>,
    search: String,
    chosen: Option<String>,
    /// Превью строк.
    thumbs: crate::thumbs::Thumbs,
}

/// Сторона превью строки в точках, `ICON` модуля.
const ICON: f32 = 40.0;

/// Как собрать превью строки, `_base_done` и `_layer_thumb` модуля:
/// слои с цветами шкалы - полоса цветов, карты и темы - их тайл поверх
/// подложки, подоснова - свой тайл.
fn recipe(item: Item) -> crate::thumbs::Recipe {
    use crate::thumbs::Recipe;
    use data::thumbs as th;
    let ramp = |c: &[[u8; 3]]| Recipe::Ramp(c.to_vec());
    match item {
        // Районная карта - тайл середины своей страны, `preview_tile` модуля.
        Item::Base(b) => {
            let source = b.source();
            Recipe::Own(net::tile_url(&source, source.region.map_or(th::TILE, |r| r.preview())))
        }
        Item::Group(n) => Recipe::Flat(th::vector_colors(n).to_vec()),
        Item::Sat(n) => Recipe::Flat(data::satellites::GROUPS.get(n).map_or_else(Vec::new, |g| vec![[16, 20, 40], g.1, [16, 20, 40]])),
        Item::Events => Recipe::Base,
        Item::Extra(e) => {
            match e {
                Extra::Dots(l) => l.legend().unwrap_or(Recipe::Base),
                Extra::Plugged(n) => crate::registry::get(n).and_then(|e| e.legend()).unwrap_or(Recipe::Base),
                _ if let Some(c) = th::colors(e.key()) => ramp(c),
                Extra::Map(_) => match crate::maps::kind(e.key()) {
                    Some(crate::maps::Kind::Theme(t)) => Recipe::Theme(t),
                    Some(crate::maps::Kind::Terrain(crate::maps::Analysis::Slope)) => Recipe::Ramp(data::slope::SLOPE_STOPS.iter().map(|s| s.1).collect()),
                    Some(crate::maps::Kind::Terrain(crate::maps::Analysis::Aspect)) => Recipe::Ramp(data::slope::ASPECT_STOPS.iter().map(|s| s.1).collect()),
                    // Карта, которой на мелком тайле нет, - одна подложка.
                    Some(kind) if kind.wants(th::TILE) => kind.sources(e.key()).into_iter().find(|(level, _)| *level <= th::TILE.0).map_or(Recipe::Base, |(_, s)| Recipe::Map(net::tile_url(&s, th::TILE), kind)),
                    _ => Recipe::Base,
                },
                _ => Recipe::Base,
            }
        }
    }
}

/// Строка каталога.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Item {
    Base(Basemap),
    Group(usize),
    Extra(Extra),
    /// Группа спутников CelesTrak, номер в satellites::GROUPS.
    Sat(usize),
    /// «Снимки событий» Vantor и Planet: строка открывает окно.
    Events,
}

impl Item {
    fn key(self) -> String {
        match self {
            Item::Base(b) => format!("base:{}", b.key()),
            Item::Group(n) => format!("vector:{n}"),
            Item::Extra(e) => e.key().to_string(),
            Item::Sat(n) => format!("sat:{}", sat_key(n)),
            Item::Events => "events".to_string(),
        }
    }

    fn category(self) -> Category {
        match self {
            Item::Base(_) => Category::Base,
            Item::Group(_) => Category::Map,
            Item::Extra(e) => e.category(),
            Item::Sat(_) => Category::Satellites,
            Item::Events => Category::Archive,
        }
    }

    fn name(self, lang: Lang) -> String {
        match self {
            Item::Base(b) => basemap_title(lang, b).to_string(),
            Item::Group(n) => tr(lang, TREE[n].0).to_string(),
            Item::Extra(e) => tr(lang, e.name()).to_string(),
            Item::Sat(n) => crate::satellites::group_names(lang).get(n).map_or(String::new(), |g| g.1.to_string()),
            Item::Events => tr(lang, "Event imagery").to_string(),
        }
    }

    fn tip(self, lang: Lang) -> String {
        match self {
            Item::Base(Basemap::Esri) => tr(lang, "Imagery of the whole Earth, the example basemap. Its terms are set by Esri.").to_string(),
            Item::Base(Basemap::Osm) => tr(lang, "The OpenStreetMap map.").to_string(),
            Item::Base(b) => b.attribution(lang).0.to_string(),
            Item::Group(n) => tr(lang, TREE[n].1).to_string(),
            Item::Extra(e) => tr(lang, e.tip()).to_string(),
            Item::Sat(n) => crate::satellites::group_names(lang).get(n).map_or(String::new(), |g| g.2.to_string()),
            Item::Events => tr(lang, "Images of earthquakes, floods, fires and hurricanes from two open catalogs. Vantor (formerly Maxar) gives WorldView strips, Planet - PlanetScope, SkySat and Pelican scenes and the quarterly Planet mosaic before the event. The row opens the window «Event imagery» with the list of events. The license of the images is CC BY-NC 4.0 - non-commercial use only.").to_string(),
        }
    }

    /// Вторая строка: источник, `subtitle` модуля.
    fn subtitle(self) -> &'static str {
        match self {
            Item::Base(Basemap::Esri) => "Esri",
            Item::Base(Basemap::Osm) => "OpenStreetMap",
            Item::Base(b) => b.ready().map_or("", |r| r.group.name()),
            Item::Group(_) => "OpenFreeMap",
            Item::Extra(Extra::Dots(l)) => l.source(),
            Item::Sat(_) => "CelesTrak",
            Item::Events => "Vantor, Planet",
            Item::Extra(Extra::Plugged(n)) => crate::registry::get(n).map_or("", |e| e.source()),
            Item::Extra(Extra::Cutaway) => "PREM, CRUST1.0, Slab2",
            Item::Extra(Extra::Paleo) => "PALEOMAP",
            Item::Extra(Extra::Geoid) => "EGM2008, NGA",
            Item::Extra(e) => e.map().map_or("", |m| m.source),
        }
    }

    /// Строка недоступна странице: самолёты ADS-B и кадастр НСПД
    /// не отдают данные браузеру.
    fn desktop_only(self) -> bool {
        matches!(self, Item::Extra(e) if e.desktop_only())
    }

    /// Почему строка сейчас недоступна, подсказка заглушки, или None:
    /// служба не отдаёт данные странице или на глобусе не Земля.
    fn unavailable(self, state: &State<'_>) -> Option<&'static str> {
        if self.desktop_only() {
            return Some(DESKTOP_ONLY);
        }
        let earth_only = match self {
            Item::Extra(e) => e.earth_only(),
            Item::Base(_) | Item::Group(_) | Item::Sat(_) | Item::Events => true,
        };
        (earth_only && !state.earth).then_some(EARTH_ONLY)
    }

    fn is_on(self, state: &State<'_>) -> bool {
        match self {
            Item::Base(b) => state.basemap == b,
            Item::Group(n) => TREE[n].2.iter().any(|(r, _, _)| state.vector.rows.contains(r)),
            Item::Extra(e) => state.on.contains(&e),
            Item::Sat(n) => satellites_row().is_some_and(|e| state.on.contains(&e)) && state.sat_groups.iter().any(|g| g == sat_key(n)),
            Item::Events => state.events,
        }
    }
}

/// Подсказка заглушки у страницы: служба не отдаёт данные браузеру.
pub const DESKTOP_ONLY: &str = "Only in the desktop program - the service does not give its data to a web page.";
/// Подсказка заглушки на другом теле.
pub const EARTH_ONLY: &str = "Only on the Earth. The menu «Body» returns the Earth to the globe.";

/// Ключ группы спутников по номеру.
fn sat_key(n: usize) -> &'static str {
    data::satellites::GROUPS.get(n).map_or("", |g| g.0)
}

/// Строки реестра со своим местом в каталоге, остальные идут общим списком.
const PLACED: [&str; 3] = ["buildings", "overture", "satellites"];

/// Строки реестра по ключам, по порядку ключей.
fn placed(keys: &[&str]) -> Vec<Item> {
    keys.iter().filter_map(|k| Extra::from_key(k)).map(Item::Extra).collect()
}

/// Строка «Спутники» реестра: ей подчинены строки групп спутников.
pub fn satellites_row() -> Option<Extra> {
    Extra::from_key("satellites")
}

fn items() -> Vec<Item> {
    let mut out: Vec<Item> = Basemap::all().into_iter().map(Item::Base).collect();
    out.extend((0..TREE.len()).map(Item::Group));
    out.push(Item::Extra(Extra::Grid));
    // Строки зданий реестра - сразу за сеткой, как прежде.
    out.extend(placed(&["buildings", "overture"]));
    out.push(Item::Extra(Extra::Stars));
    out.push(Item::Extra(Extra::Air));
    out.push(Item::Extra(Extra::Sun));
    out.extend(placed(&["satellites"]));
    // Пожары - карта витрины группы «Планета огня», перед темами огня.
    out.push(Item::Extra(Extra::Dots(crate::dots::Layer::Fires)));
    out.extend((0..crate::maps::count()).map(|n| Item::Extra(Extra::Map(n))));
    out.extend(crate::dots::Layer::CATALOG.map(|l| Item::Extra(Extra::Dots(l))));
    // Порядок LAYERS витрины модуля.
    out.extend((0..crate::registry::ALL.len()).filter(|n| crate::registry::get(*n).is_some_and(|e| !PLACED.contains(&e.key()))).map(|n| Item::Extra(Extra::Plugged(n))));
    out.push(Item::Extra(Extra::Geoid));
    out.push(Item::Extra(Extra::Cutaway));
    out.push(Item::Extra(Extra::Paleo));
    out.extend((0..data::satellites::GROUPS.len()).map(Item::Sat));
    out.push(Item::Events);
    // Строк, которых нет у страницы, она не показывает.
    out.retain(|i| !i.desktop_only());
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
    pub fn window(&mut self, ctx: &egui::Context, lang: Lang, state: &State<'_>, request: &dyn Fn(u64) -> Box<dyn net::Request>, out: &mut Vec<Action>) {
        if !self.open {
            return;
        }
        // Превью: тайл подложки глобуса, затем тайлы показанных строк.
        self.thumbs.step(ctx, &net::tile_url(&state.basemap.source(), data::thumbs::TILE), request);
        let mut open = self.open;
        let categories = [
            (Category::All, tr(lang, "All"), tr(lang, "All maps and layers of the catalog.")),
            (Category::Base, tr(lang, "Base maps"), tr(lang, "Basemaps of the globe - imagery and maps. There is one base on the globe, under the layers.")),
            (Category::Local, tr(lang, "Local data"), tr(lang, "Detailed imagery and maps of separate areas from the services of their owners, over the base map of the globe. Outside the area the base map is seen. Own WMS services of the window «New tile source» are here too.")),
            (Category::Map, tr(lang, "Map"), tr(lang, "The OpenFreeMap vector base and the coordinate grid.")),
            (Category::Fire, tr(lang, "Planet of Fire"), tr(lang, "Fire spots, smoke, aerosol and carbon monoxide of fires and industry from NASA data.")),
            (Category::Water, tr(lang, "Planet of Water"), tr(lang, "Precipitation, soil moisture, snow, ice, water vapor, chlorophyll, salinity and floods from NASA data, currents from NOAA data.")),
            (Category::Weather, tr(lang, "Weather"), tr(lang, "NOAA GFS forecast fields by the time slider, the aurora forecast and the surface temperature of land and sea.")),
            (Category::Gases, tr(lang, "Planet of Air"), tr(lang, "Nitrogen dioxide, sulfur dioxide, methane, carbon dioxide and ozone from NASA data.")),
            (Category::Life, tr(lang, "Land and Life"), tr(lang, "Vegetation, dust, night lights and land cover types from NASA data.")),
            (Category::Sky, tr(lang, "Sky and light"), tr(lang, "The sun and the night side, the atmosphere.")),
            (Category::Depths, tr(lang, "Inside the Earth"), tr(lang, "The geological and lithological maps, earthquakes, plate boundaries and faults, mineral deposits and mining areas, the Earth cutaway and palaeogeography.")),
            (Category::Infra, tr(lang, "Energy"), tr(lang, "Power plants of the world, power lines, substations, pipelines and submarine cables.")),
            (Category::Transport, tr(lang, "Transport"), tr(lang, "Trains of Finland, Baltic ships and aircraft in real time from Fintraffic open data and the adsb.lol ADS-B network.")),
            (Category::Soil, tr(lang, "Soils"), tr(lang, "Properties of the topsoil 0-5 cm from SoilGrids - clay, sand, organic carbon and pH, a 250 m grid.")),
            (Category::Terrain, tr(lang, "Terrain analysis"), tr(lang, "Colouring of the surface by slope or by the direction a slope faces, one of the two is on. Terrain contours lie over them.")),
            (Category::Tiles, tr(lang, "Map overlays"), tr(lang, "Transparent maps over the base map - sea marks, hiking, cycling, mountain bike and riding routes, ski slopes. Several can be on at once.")),
            (Category::Satellites, tr(lang, "Satellites"), tr(lang, "Groups of artificial satellites from CelesTrak. A click on a group shows it on the globe or removes it.")),
            (Category::Wiki, tr(lang, "Encyclopedia"), tr(lang, "Wikipedia articles with coordinates near the look-at point. A click on an article gives its link and first paragraph.")),
            (Category::Archive, tr(lang, "Imagery archive"), tr(lang, "Images of extraordinary events from the open catalogs of Vantor and Planet.")),
            (
                Category::Gibs,
                tr(lang, "All NASA GIBS layers"),
                tr(lang, "The whole NASA GIBS catalog - more than a thousand raster layers of imagery and maps. The list comes from the GIBS catalog on the first show of the category or on a search from three characters. The search finds a layer by all words of the query, the layer names are in English. These rows are not shown in «All»."),
            ),
        ];
        // Левый столбец: «Все», разделы модуля жирным и их категории
        // с отступом. Раздел из одной категории - строка категории.
        let named = |c: Category| categories.iter().find(|(k, _, _)| *k == c).map_or(("", ""), |(_, t, h)| (*t, *h));
        let mut picks: Vec<Pick<'_>> = vec![Pick { title: categories[0].1, tip: categories[0].2, categories: Vec::new(), head: false, indent: false }];
        for (title, tip, members) in SECTIONS {
            let (title, tip) = (tr(lang, title), tr(lang, tip));
            if let [one] = members {
                let (t, h) = named(*one);
                picks.push(Pick { title: t, tip: h, categories: vec![*one], head: false, indent: false });
                continue;
            }
            // Весь каталог GIBS - только своей строкой: в нём больше
            // тысячи слоёв.
            picks.push(Pick { title, tip, categories: members.iter().copied().filter(|c| *c != Category::Gibs).collect(), head: true, indent: false });
            for c in members {
                let (t, h) = named(*c);
                picks.push(Pick { title: t, tip: h, categories: vec![*c], head: false, indent: true });
            }
        }
        let current = self.category.filter(|i| *i < picks.len()).unwrap_or(0);
        // Высота столбцов - от экрана, а не от остатка окна: окно egui
        // подгоняет размер под содержимое, и столбец «во всю высоту»
        // растягивал его без конца, окно не показывалось.
        let body = (ctx.content_rect().height() - 220.0).clamp(BODY_MIN, BODY_MAX);
        egui::Window::new(tr(lang, "Add data")).open(&mut open).default_size([820.0, 720.0]).show(ctx, |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(tr(lang, "snow, mines, wind")).desired_width(f32::INFINITY));
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(CATEGORY_WIDTH);
                    egui::ScrollArea::vertical().id_salt("catalog categories").auto_shrink([false, false]).max_height(body).min_scrolled_height(body).show(ui, |ui| {
                        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for (i, pick) in picks.iter().enumerate() {
                                // Категория раздела - с отступом под заголовком.
                                let semibold = egui::FontFamily::Name(crate::fonts::SEMIBOLD.into());
                                let text = match (pick.head, pick.indent) {
                                    (true, _) => egui::RichText::new(pick.title).family(semibold).strong(),
                                    (false, true) => egui::RichText::new(format!("    {}", pick.title)),
                                    (false, false) => egui::RichText::new(pick.title),
                                };
                                let r = ui.selectable_label(current == i, text);
                                let r = if pick.tip.is_empty() { r } else { r.on_hover_text(pick.tip) };
                                if r.clicked() {
                                    self.category = Some(i);
                                }
                            }
                        });
                    });
                });
                ui.separator();
                ui.vertical(|ui| {
                    let pick = &picks[current];
                    let all = pick.categories.is_empty();
                    let gibs_row = pick.categories == [Category::Gibs];
                    let text = self.search.trim().to_lowercase();
                    if (gibs_row || text.chars().count() >= GIBS_SEARCH) && !crate::maps::has_catalog() && !out.contains(&Action::GibsCatalog) {
                        out.push(Action::GibsCatalog);
                    }
                    let words: Vec<String> = text.split_whitespace().map(String::from).collect();
                    let fits = |item: Item| {
                        let gibs = item.category() == Category::Gibs;
                        if !text.is_empty() && gibs {
                            data::gibs::fits_words(&format!("{} {}", item.name(lang), item.key()), &words)
                        } else if !text.is_empty() {
                            item.name(lang).to_lowercase().contains(&text) || item.tip(lang).to_lowercase().contains(&text)
                        } else if all {
                            !matches!(item, Item::Base(_)) && !gibs
                        } else {
                            pick.categories.contains(&item.category())
                        }
                    };
                    let shown: Vec<Item> = items().into_iter().filter(|i| fits(*i)).collect();
                    let list_height = (body - DETAILS_HEIGHT).max(ROW_HEIGHT * 3.0);
                    egui::ScrollArea::vertical().id_salt("catalog rows").auto_shrink([false, false]).max_height(list_height).min_scrolled_height(list_height).show(ui, |ui| {
                        let bases: Vec<Item> = shown.iter().copied().filter(|i| matches!(i, Item::Base(_))).collect();
                        let layers: Vec<Item> = shown.iter().copied().filter(|i| !matches!(i, Item::Base(_)) && i.category() != Category::Gibs).collect();
                        let gibs: Vec<Item> = shown.iter().copied().filter(|i| i.category() == Category::Gibs).collect();
                        if gibs_row && gibs.is_empty() && !crate::maps::has_catalog() {
                            ui.label(tr(lang, "The NASA GIBS catalog is loading, the first time about 10 seconds."));
                        }
                        for (head, list) in [(tr(lang, "Base map on the globe - one"), bases), (tr(lang, "Layers - switched on together"), layers), (tr(lang, "NASA GIBS layers - also one map on the globe"), gibs)] {
                            if list.is_empty() {
                                continue;
                            }
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(head).family(egui::FontFamily::Name(crate::fonts::SEMIBOLD.into())).strong());
                            for item in list {
                                let mut on = item.is_on(state);
                                let missing = item.unavailable(state).filter(|_| !on);
                                // Строки одной высоты, флажок по середине
                                // строки, источник - второй строкой
                                // приглушённым цветом, как у модуля.
                                let size = egui::vec2(ui.available_width(), ROW_HEIGHT);
                                ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    ui.set_min_size(size);
                                    let check = ui.add_enabled_ui(missing.is_none(), |ui| crate::theme::checkbox(ui, &mut on).clicked()).inner;
                                    if check {
                                        self.chosen = Some(item.key());
                                        click(item, state, out);
                                    }
                                    // Превью строки, до прихода - тёмный квадрат.
                                    let icon = egui::vec2(ICON, ICON);
                                    match self.thumbs.texture(&item.key(), || recipe(item)) {
                                        Some(id) => {
                                            ui.add(egui::Image::new((id, icon)).corner_radius(3.0));
                                        }
                                        None => {
                                            let (rect, _) = ui.allocate_exact_size(icon, egui::Sense::hover());
                                            let [r, g, b] = data::thumbs::BACKGROUND;
                                            ui.painter().rect_filled(rect, 3.0, egui::Color32::from_rgb(r, g, b));
                                        }
                                    }
                                    let sub = item.subtitle();
                                    let mut label = egui::text::LayoutJob::default();
                                    let body = egui::TextFormat { font_id: egui::TextStyle::Body.resolve(ui.style()), color: ui.visuals().text_color(), ..Default::default() };
                                    label.append(&item.name(lang), 0.0, body.clone());
                                    if !sub.is_empty() {
                                        label.append(&format!("\n{sub}"), 0.0, egui::TextFormat { color: ui.visuals().weak_text_color(), ..body });
                                    }
                                    let selected = self.chosen.as_deref() == Some(&item.key());
                                    let tip = match missing {
                                        Some(why) => format!("{}\n\n{}", item.tip(lang), tr(lang, why)),
                                        None => item.tip(lang),
                                    };
                                    // Две строки целиком: с обрезкой egui
                                    // сводил их в одну с «…».
                                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                                    // Текст влево во всю ширину, по высоте -
                                    // по середине строки, как флажок.
                                    let lines = if sub.is_empty() { 1.0 } else { 2.0 };
                                    let tall = lines * ui.text_style_height(&egui::TextStyle::Body) + 2.0 * ui.spacing().button_padding.y;
                                    let r = ui
                                        .with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                                            ui.add_space(((ROW_HEIGHT - tall) / 2.0).max(0.0));
                                            ui.selectable_label(selected, label)
                                        })
                                        .inner
                                        .on_hover_text(tip);
                                    if r.clicked() {
                                        self.chosen = Some(item.key());
                                    }
                                    if r.double_clicked() && missing.is_none() {
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
                                _ if !on && let Some(why) = item.unavailable(state) => (tr(lang, why), false),
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
        // Снимки событий: окно со списком событий, снимает их окно.
        Item::Events => out.push(Action::Events),
        // Группа спутников: щелчок показывает её или убирает, строка
        // «Спутники» включается вместе с группой.
        Item::Sat(n) => {
            let key = sat_key(n).to_string();
            let mut groups: Vec<String> = state.sat_groups.to_vec();
            if item.is_on(state) {
                groups.retain(|g| *g != key);
            } else {
                if !groups.contains(&key) {
                    groups.push(key);
                }
                if let Some(row) = satellites_row().filter(|e| !state.on.contains(e)) {
                    out.push(Action::Extra(row, true));
                }
            }
            out.push(Action::SatGroups(groups));
        }
        Item::Extra(e) => {
            if state.on.contains(&e) {
                out.push(Action::RemoveExtra(e));
            } else {
                out.push(Action::Extra(e, true));
            }
        }
    }
}
