// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Связь с открытым RustGIS: опрос проекта, слои и картинки проекта, раздел «Проект RustGIS», расчёты в проект. Зона сессии Rust.

use super::*;

impl LiteApp {
    /// Связь с RustGIS: запуск и остановка по настройке или `--rustgis`,
    /// вести потока - в этом кадре. Слои проекта - живыми слоями папки
    /// «RustGIS», вид карты - на глобус, вид глобуса после покоя - на
    /// карту, как синхронизация модуля с окном карты QGIS.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn step_rustgis(&mut self, ctx: &egui::Context, now: f64) {
        use crate::rustgislink::{Bridge, FOLDER, Link, News};
        if !self.settings.rustgis && !self.rustgis_cli {
            if self.rustgis.take().is_some() {
                self.data_layers.drop_live();
                crate::datalayers::RUSTGIS_LINKED.store(false, std::sync::atomic::Ordering::Relaxed);
            }
            return;
        }
        if self.rustgis.is_none() {
            let wake_ctx = ctx.clone();
            self.rustgis = Some(Bridge::new(Link::start(Arc::new(move || wake_ctx.request_repaint()))));
        }
        // Направление синхронизации - из «Свойств вида».
        if let Some(sync) = &mut self.rustgis {
            sync.direction = crate::rustgislink::Direction::from_key(&self.settings.sync).unwrap_or_default();
        }
        let mut news = Vec::new();
        if let Some(sync) = &self.rustgis {
            while let Some(n) = sync.link.take() {
                news.push(n);
            }
        }
        for n in news {
            match n {
                News::Connected => {
                    if let Some(sync) = &mut self.rustgis {
                        sync.connected = true;
                        sync.status.clear();
                    }
                    self.open_message = Some(tr(self.lang, "Linked with RustGIS.").to_string());
                }
                News::Waiting(why) => {
                    let was = self.rustgis.as_ref().is_some_and(|s| s.connected);
                    if let Some(sync) = &mut self.rustgis {
                        sync.connected = false;
                        sync.status = why.clone();
                    }
                    self.data_layers.drop_live();
                    if was || self.rustgis_cli {
                        let why = if why == crate::rustgislink::RELAY_OFF { tr(self.lang, crate::rustgislink::RELAY_OFF).to_string() } else { why };
                        self.open_message = Some(trf(self.lang, "No link with RustGIS: {error}. In RustGIS start the MCP Relay command.", &[("error", &why)]));
                    }
                }
                News::State(state) => {
                    let order: Vec<i64> = state.layers.iter().filter(|l| l.drawn()).map(|l| l.id).collect();
                    self.data_layers.sync_live(&order);
                    let follow = self.settings.rustgis_follows(&state.project);
                    let saved = self.settings.rustgis_shown_of(&state.project).to_vec();
                    let mut pictures = Vec::new();
                    let moved = self.rustgis.as_mut().and_then(|sync| {
                        let other = sync.take_state(&state);
                        // Режим карты - видимость с карты RustGIS на каждом
                        // опросе, `_legend_changed` модуля. Другой проект -
                        // его отметки из настроек глобуса.
                        if follow || other {
                            for id in &order {
                                self.data_layers.show_live(*id, sync.wanted(*id, follow, &saved));
                            }
                            // Растры и тайловые слои - только те, чья
                            // видимость сменилась.
                            for row in sync.rows.iter().filter(|r| r.kind.is_picture()) {
                                let on = Bridge::wanted_row(row, follow, &saved);
                                if on != sync.pictures_on.contains(&row.key) {
                                    pictures.push((row.key.clone(), on));
                                }
                            }
                        }
                        // Оформление растра на глобусе сменилось - он
                        // режется заново, прежние тайлы видны до новых.
                        let restyled: Vec<(i64, String, Option<[f64; 4]>, String)> = sync
                            .rows
                            .iter()
                            .filter_map(|r| match r.key {
                                crate::rustgislink::RowKey::Layer(id) if r.kind == crate::rustgislink::RowKind::Raster && sync.pictures_on.contains(&r.key) && sync.cut.get(&id).is_some_and(|s| *s != r.style) => Some((id, r.style.clone(), r.extent, r.name.clone())),
                                _ => None,
                            })
                            .collect();
                        for (id, style, extent, name) in restyled {
                            sync.cut.insert(id, style);
                            let max_zoom = extent.map_or(10, |e| data::rustgis::cut_level(e, crate::rustgislink::CUT_TILES, crate::rustgislink::CUT_LEVEL));
                            sync.link.ask(crate::rustgislink::Ask::Raster { layer: id, name, max_zoom });
                        }
                        state.view.and_then(|v| sync.map_view(v))
                    });
                    for (key, on) in pictures {
                        self.rustgis_picture(&key, on);
                    }
                    // Карта ведёт глобус: перелёт к её охвату, наклон
                    // и азимут глобуса остаются, `fly_view` модуля.
                    if let Some((lat, lon, w, h)) = moved.filter(|_| self.planet.earth) {
                        let (fov_y, aspect) = (self.nav.camera.fov_y, self.nav.camera.aspect());
                        let distance = data::rustgis::distance_for(w, h, fov_y, aspect);
                        let (heading, tilt) = (self.nav.pose.heading, self.nav.pose.tilt);
                        crate::input::fly(&mut self.nav, Pose::new(lat, lon, distance, heading, tilt), now);
                        // Вид в конце перелёта: ширина полосы, как у view_width.
                        let spot = (lat, lon, 2.0 * distance * (fov_y.to_radians() / 2.0).tan() * aspect);
                        if let Some(sync) = &mut self.rustgis {
                            sync.applied(spot);
                        }
                    }
                }
                News::Layer { id, data, style } => {
                    let name = self.rustgis.as_ref().map_or_else(|| format!("RustGIS {id}"), |s| s.name(id));
                    let n = self.data_layers.list.len();
                    let (colors, look, label) = crate::rustgislink::paint(&data, &style, n);
                    let new = self.data_layers.live_shown(id).is_none();
                    self.data_layers.set_live(FOLDER, id, &name, (*data, colors), look, label);
                    // Новый слой: на глобусе, если он включён на карте
                    // в режиме карты или отмечен в настройках проекта.
                    if new && let Some(sync) = &self.rustgis {
                        let follow = self.settings.rustgis_follows(&sync.project);
                        let on = sync.wanted(id, follow, self.settings.rustgis_shown_of(&sync.project));
                        self.data_layers.show_live(id, on);
                    }
                }
                News::Done(name) => self.open_message = Some(trf(self.lang, "The layer {name} is added to the RustGIS project.", &[("name", &name)])),
                News::Added(group) => self.open_message = Some(trf(self.lang, "The calculation is added to the RustGIS project, group {group}.", &[("group", &group)])),
                News::Tiles { layer, template, max_zoom } => self.rustgis_tiles(layer, &template, max_zoom),
                News::CutFailed { layer, error } => {
                    if let Some(sync) = &mut self.rustgis {
                        sync.cut.remove(&layer);
                        sync.pictures_on.remove(&crate::rustgislink::RowKey::Layer(layer));
                    }
                    self.open_message = Some(trf(self.lang, "RustGIS: {error}", &[("error", &error)]));
                }
                News::Failed(why) => self.open_message = Some(trf(self.lang, "RustGIS: {error}", &[("error", &why)])),
            }
        }
        crate::datalayers::RUSTGIS_LINKED.store(self.rustgis.as_ref().is_some_and(|s| s.connected), std::sync::atomic::Ordering::Relaxed);
        // Вид глобуса - на карту, когда глобус постоял. На других телах
        // синхронизации нет, как у модуля.
        let spot = (self.nav.pose.lat, self.nav.pose.lon, self.view_width());
        let earth = self.planet.earth;
        if let Some(sync) = &mut self.rustgis
            && sync.connected
            && earth
        {
            if let Some(ask) = sync.globe_view(spot, now) {
                sync.link.ask(ask);
            }
            // Покой глобуса досчитывается и без движения мыши.
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(crate::rustgislink::SETTLE_SECONDS));
        }
    }

    /// Расчёт окна анализа - в проект RustGIS, как результаты инструментов
    /// модуля - в проект QGIS (`results.py`): подгруппа «Название ЧЧ:ММ:СС
    /// UTC» группы «RustPlanetX - анализ участка», сверху контур участка
    /// и векторы, под ними растры файлами GeoTIFF files. Названия слоёв -
    /// со временем расчёта: группа находит свои слои по названию.
    /// Наложения и модели расчёта в проект не идут, как у модуля.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn analysis_to_rustgis(&mut self, saved: &crate::analysis::Saved, files: &[String], failed: Option<String>, unix: f64) {
        use scene::kml::{Folder, Item, Place};
        let lang = self.lang;
        let stamp = crate::analysis::clock(unix);
        let group = format!("{} {stamp} UTC", saved.title);
        let as_layer = |folder: &Folder| crate::datalayers::folder_layer(folder).map(|l| data::layers::to_geojson(&l));
        let mut vectors = Vec::new();
        if saved.ring.len() >= 3 {
            let site = tr(lang, "Area|site");
            let mut outline = Folder::new(site);
            outline.children.push(Item::Place(Place::new(site, scene::features::Kind::Polygon, saved.ring.clone())));
            vectors.extend(as_layer(&outline).map(|g| (format!("{site} {stamp}"), g)));
        }
        let mut loose = Folder::new(&saved.title);
        for item in &saved.extra {
            match item {
                Item::Folder(f) => vectors.extend(as_layer(f).map(|g| (format!("{} {stamp}", f.name), g))),
                Item::Place(p) => loose.children.push(Item::Place(p.clone())),
                Item::Overlay(_) => {}
            }
        }
        vectors.extend(as_layer(&loose).map(|g| (format!("{} {stamp}", saved.title), g)));
        let rasters: Vec<(String, String)> = saved
            .layers
            .iter()
            .zip(files)
            .filter(|(_, f)| !f.is_empty())
            .map(|(l, f)| (format!("{} {stamp}", if l.name.is_empty() { l.stem.as_str() } else { l.name.as_str() }), f.clone()))
            .collect();
        let top = tr(lang, "RustPlanetX - area analysis").to_string();
        if let Some(b) = &self.rustgis {
            b.link.ask(crate::rustgislink::Ask::AddResult { top, group: group.clone(), vectors, rasters });
        }
        self.open_message = Some(match failed {
            Some(e) => trf(lang, "The calculation goes to the RustGIS project, group {group}. The GeoTIFF file is not written: {e}", &[("group", &group), ("e", &e)]),
            None => trf(lang, "The calculation goes to the RustGIS project, group {group}.", &[("group", &group)]),
        });
    }

    /// Издание программы, `doc/EDITIONS.md`. По умолчанию PlanetX Lite.
    pub fn with_edition(mut self, edition: crate::Edition) -> Self {
        self.edition = edition;
        self
    }

    /// Издание программы.
    pub fn edition(&self) -> crate::Edition {
        self.edition
    }

    /// Связь с RustGIS в этом запуске, `planetx-lite --rustgis`, без
    /// записи в настройки. Так глобус открывает расширение RustGIS.
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    pub fn with_rustgis(mut self) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.rustgis_cli = true;
        }
        self
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Растр или тайловый слой проекта RustGIS - на глобус или с него,
    /// строкой карт [`crate::maps::register_tiles`]. Растр ещё не нарезан -
    /// нарезку просят у RustGIS, на глобус он встанет, когда тайлы придут.
    pub(super) fn rustgis_picture(&mut self, key: &crate::rustgislink::RowKey, on: bool) {
        use crate::rustgislink::{Ask, CUT_LEVEL, CUT_TILES, RowKey};
        let Some(sync) = &mut self.rustgis else {
            return;
        };
        if on {
            sync.pictures_on.insert(key.clone());
        } else {
            sync.pictures_on.remove(key);
        }
        let mut n = sync.pictures.get(key).copied();
        if on && n.is_none() {
            match key {
                RowKey::Tiles(id) => {
                    n = sync.tiles.get(id).and_then(|t| crate::maps::register_tiles(&t.name, &t.url, &t.attribution, t.max_zoom));
                    if let Some(n) = n {
                        sync.pictures.insert(key.clone(), n);
                    }
                }
                RowKey::Layer(id) => {
                    let row = sync.row(key).map(|r| (r.style.clone(), r.extent, r.name.clone()));
                    if let Some((style, extent, name)) = row
                        && !sync.cut.contains_key(id)
                    {
                        sync.cut.insert(*id, style);
                        let max_zoom = extent.map_or(10, |e| data::rustgis::cut_level(e, CUT_TILES, CUT_LEVEL));
                        sync.link.ask(Ask::Raster { layer: *id, name, max_zoom });
                    }
                }
            }
        }
        if let Some(n) = n {
            if on {
                self.extras.insert(crate::layers::Extra::Map(n));
            }
            self.set_extra(crate::layers::Extra::Map(n), on);
        }
    }

    /// Тайлы растра layer нарезаны: строка карт по ним, прежняя нарезка
    /// снимается. На глобус - если слой отмечен.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn rustgis_tiles(&mut self, layer: i64, template: &str, max_zoom: u32) {
        let key = crate::rustgislink::RowKey::Layer(layer);
        let Some(sync) = &mut self.rustgis else {
            return;
        };
        let name = sync.row(&key).map_or_else(|| format!("RustGIS {layer}"), |r| r.name.clone());
        let Some(n) = crate::maps::register_tiles(&name, template, "RustGIS", max_zoom) else {
            return;
        };
        let old = sync.pictures.insert(key.clone(), n).filter(|o| *o != n);
        let on = sync.pictures_on.contains(&key);
        if let Some(o) = old {
            self.set_extra(crate::layers::Extra::Map(o), false);
            self.extras.remove(&crate::layers::Extra::Map(o));
        }
        if on {
            self.extras.insert(crate::layers::Extra::Map(n));
            self.set_extra(crate::layers::Extra::Map(n), true);
        }
    }

    /// Раздел «Проект RustGIS» левой панели между «Метками» и «Слоями», как
    /// раздел «Проект QGIS» модуля: все слои открытого проекта RustGIS
    /// в порядке карты, флажок показывает слой на глобусе. По умолчанию
    /// слои выключены: свои слои глобуса нарисованы лучше. Без связи -
    /// строка с причиной.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn rustgis_section(&mut self, ui: &mut egui::Ui, lang: Lang) {
        let Some(sync) = &self.rustgis else {
            return;
        };
        if !crate::theme::section(ui, "rustgis", tr(lang, "RustGIS project")) {
            return;
        }
        let muted = crate::theme::pal().muted;
        if !sync.connected {
            let why = if sync.status == crate::rustgislink::RELAY_OFF { tr(lang, crate::rustgislink::RELAY_OFF).to_string() } else { sync.status.clone() };
            ui.label(egui::RichText::new(trf(lang, "No link with RustGIS: {error}. In RustGIS start the MCP Relay command.", &[("error", &why)])).color(muted));
            return;
        }
        let rows: Vec<crate::rustgislink::Row> = sync.rows.clone();
        let pictures_on = sync.pictures_on.clone();
        let cutting: Vec<crate::rustgislink::RowKey> = sync.cut.keys().map(|id| crate::rustgislink::RowKey::Layer(*id)).filter(|k| !sync.pictures.contains_key(k)).collect();
        let project = sync.project.clone();
        // Режим «Слои как на карте RustGIS», флажок «Слои как на карте QGIS»
        // модуля: хранится по проекту.
        let mut follow = self.settings.rustgis_follows(&project);
        let follow_box = ui.checkbox(&mut follow, tr(lang, "Layers as on the RustGIS map")).on_hover_text(tr(
            lang,
            "With the check the globe shows the layers switched on in RustGIS, and a check in the list of the globe switches the layer on the map too, the opacity sets its transparency there. Without the check the checks of the globe are its own and do not change the map. The check is kept for the project.",
        ));
        let mut pictures = Vec::new();
        if follow_box.changed() {
            self.settings.set_rustgis_follow(&project, follow);
            self.settings_dirty = true;
            let saved = self.settings.rustgis_shown_of(&project).to_vec();
            if let Some(sync) = &self.rustgis {
                for row in &rows {
                    match row.key {
                        crate::rustgislink::RowKey::Layer(id) if !row.kind.is_picture() => self.data_layers.show_live(id, sync.wanted(id, follow, &saved)),
                        _ if row.kind.is_picture() => pictures.push((row.key.clone(), crate::rustgislink::Bridge::wanted_row(row, follow, &saved))),
                        _ => {}
                    }
                }
            }
        }
        for (key, on) in pictures.drain(..) {
            self.rustgis_picture(&key, on);
        }
        if rows.is_empty() {
            ui.label(egui::RichText::new(tr(lang, "The RustGIS project has no layers.")).color(muted));
            return;
        }
        let mut toggled = Vec::new();
        let mut faded = Vec::new();
        let mut fly = None;
        // Не больше восьми строк: под разделом - «Слои».
        let room = (22.0 * RUSTGIS_ROWS).min(ui.available_height() - SECTION_MIN).max(22.0);
        egui::ScrollArea::vertical().id_salt("rustgis").max_height(room).auto_shrink([false, true]).show(ui, |ui| {
            for row in &rows {
                // Строка как у списка «Проект QGIS» модуля: «название · вид»,
                // флажок - на глобусе, двойной щелчок - перелёт к слою.
                let text = format!("{} · {}", row.name, tr(lang, row.kind.word()));
                let picture = row.kind.is_picture();
                let id = match row.key {
                    crate::rustgislink::RowKey::Layer(id) => Some(id),
                    crate::rustgislink::RowKey::Tiles(_) => None,
                };
                let shown = if picture { Some(pictures_on.contains(&row.key)) } else { id.and_then(|id| self.data_layers.live_shown(id)) };
                let tip = if row.kind == crate::rustgislink::RowKind::Table {
                    tr(lang, "A table without geometry does not go to the globe.")
                } else if cutting.contains(&row.key) && pictures_on.contains(&row.key) {
                    tr(lang, "RustGIS is cutting the raster into tiles in its look.")
                } else if picture {
                    tr(lang, crate::maps::RUSTGIS_TIP)
                } else if shown.is_some() {
                    tr(lang, "A layer of the open RustGIS project in its RustGIS colours. The check shows it on the globe.")
                } else {
                    tr(lang, "The features of the layer are being read from RustGIS.")
                };
                ui.horizontal(|ui| {
                    let mut on = shown.unwrap_or(false);
                    let usable = shown.is_some() && row.kind != crate::rustgislink::RowKind::Table;
                    let check = ui.add_enabled(usable, egui::Checkbox::without_text(&mut on)).on_hover_text(tip);
                    if check.changed() {
                        toggled.push((row.key.clone(), row.name.clone(), on, picture));
                    }
                    // Надпись не выделяется: двойной щелчок - перелёт, а не
                    // выделение слова.
                    let label = ui.add(egui::Label::new(text.as_str()).truncate().selectable(false).sense(egui::Sense::click())).on_hover_text(tip);
                    if label.double_clicked() {
                        fly = row.extent;
                    }
                    // Непрозрачность слоя на глобусе, в режиме карты - и на
                    // карте, когда ползунок отпущен.
                    if let Some(opacity) = id.filter(|_| !picture).and_then(|id| self.data_layers.live_opacity(id)) {
                        let id = id.unwrap_or_default();
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let mut percent = (opacity * 100.0).round();
                            let drag = ui.add(egui::DragValue::new(&mut percent).range(5.0..=100.0).speed(1.0).suffix(" %")).on_hover_text(tr(lang, "Opacity of the layer on the globe. In the mode Layers as on the RustGIS map - on the map too."));
                            if drag.changed() || drag.drag_stopped() || drag.lost_focus() {
                                faded.push((id, percent / 100.0, !drag.dragged()));
                            }
                        });
                    }
                });
            }
        });
        // Перелёт к охвату слоя, камера смотрит вниз, как у модуля.
        if let Some([w, s, e, n]) = fly {
            let camera = &self.nav.camera;
            let (lat, lon, distance) = globe_core::flight::fit_view(w, s, e, n, camera.fov_y, camera.aspect());
            if lat.is_finite() && lon.is_finite() && distance.is_finite() {
                let now = ui.ctx().input(|i| i.time);
                crate::input::fly(&mut self.nav, Pose::new(lat, lon, distance, 0.0, 0.0), now);
            }
        }
        for (key, name, on, picture) in toggled {
            if picture {
                self.rustgis_picture(&key, on);
            } else if let crate::rustgislink::RowKey::Layer(id) = key {
                self.data_layers.show_live(id, on);
            }
            if follow {
                if let Some(sync) = &mut self.rustgis {
                    match &key {
                        crate::rustgislink::RowKey::Layer(id) => {
                            sync.on_map.retain(|i| i != id);
                            if on {
                                sync.on_map.push(*id);
                            }
                            sync.link.ask(crate::rustgislink::Ask::Visibility { layer: *id, visible: on });
                        }
                        crate::rustgislink::RowKey::Tiles(id) => sync.link.ask(crate::rustgislink::Ask::TilesVisibility { id: id.clone(), visible: on }),
                    }
                    // Строка уже показывает новое состояние до ответа карты.
                    if let Some(row) = sync.rows.iter_mut().find(|r| r.key == key) {
                        row.on_map = on;
                    }
                }
            } else {
                let mut saved = self.settings.rustgis_shown_of(&project).to_vec();
                saved.retain(|n| *n != name);
                if on {
                    saved.push(name);
                }
                self.settings.set_rustgis_shown(&project, saved);
                self.settings_dirty = true;
            }
        }
        for (id, opacity, settled) in faded {
            self.data_layers.set_live_opacity(id, opacity);
            if follow
                && settled
                && let Some(sync) = &self.rustgis
            {
                let percent = (100.0 - opacity * 100.0).round().clamp(0.0, 100.0) as u8;
                sync.link.ask(crate::rustgislink::Ask::Transparency { layer: id, percent });
            }
        }
    }
}
