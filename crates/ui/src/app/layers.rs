// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Раздел «Слои» и «Мои слои»: строки, каталог «Добавить данные», карты, основа.

use super::*;

impl LiteApp {
    /// Флажок «Глубины морей и океанов» сменился: высоты заново из
    /// глубин, сетки заново, как `_set_sea_floor` модуля.
    pub(super) fn sea_changed(&mut self) {
        let floor = self.earth_floor();
        let relief = &self.relief;
        self.heights.write().refloor(floor, &|tile| relief.patch(tile));
        if let Some(globe) = &mut self.globe {
            globe.stale_meshes(None);
        }
    }

    /// Действия раздела «Слои» и каталога «Добавить данные».
    pub(super) fn apply_layers(&mut self, actions: Vec<crate::layers::Action>, now: f64) {
        use crate::layers::Action;
        for action in actions {
            match action {
                Action::Basemap(b) => self.set_basemap(b),
                Action::Sea(on) => {
                    self.settings.sea = on;
                    self.settings_dirty = true;
                    self.sea_changed();
                }
                Action::AddSource => self.own_dialog = Some(OwnDialog::default()),
                Action::Data(id, act) => self.data_act(id, &act, now),
                Action::Tiles(id, act) => {
                    if act == crate::tilesets::Act::Zoom {
                        let spots = self.tilesets.get(id).map(|s| s.spots.clone()).unwrap_or_default();
                        let camera = &self.nav.camera;
                        if let Some(pose) = marks::fit_points(&spots, camera.fov_y, camera.aspect()) {
                            crate::input::fly(&mut self.nav, pose, now);
                        }
                    } else {
                        self.tilesets.apply(id, act);
                    }
                }
                Action::GibsCatalog => self.ask_gibs_catalog(),
                Action::Events => self.events.open(&*self.loaders.request),
                Action::Wayback => self.open_wayback(),
                Action::RemoveOwn(b) => {
                    if let Some(own) = crate::basemap::own_of(b) {
                        self.settings.own.retain(|o| *o != own);
                        self.settings_dirty = true;
                    }
                    if self.basemap == b {
                        self.set_basemap(Basemap::Esri);
                    }
                }
                Action::GeoidScale(scale) => {
                    self.settings.geoid_scale = scale;
                    self.settings_dirty = true;
                    if let Some(g) = &mut self.geoid {
                        g.scale = scale;
                        self.heights.write().set_scale(scale);
                    }
                }
                Action::Stretch(on) => {
                    self.settings.stretch = on;
                    self.settings_dirty = true;
                    if let Some(c) = &mut self.cutaway {
                        c.set_stretch(on);
                    }
                }
                Action::Opacity(key, alpha) => {
                    self.map_opacity.insert(key.to_string(), alpha);
                    self.settings.opacity = self.map_opacity.iter().map(|(k, v)| (k.clone(), *v)).collect();
                    self.settings_dirty = true;
                    if self.vector.set_opacity(key, alpha)
                        && let Some(globe) = &mut self.globe
                    {
                        globe.clear_overlays();
                    }
                }
                Action::Ground(alpha) => {
                    if let Some(sub) = &mut self.subsurface {
                        sub.options.opacity = alpha;
                    }
                }
                Action::Subsurface => {
                    if let Some(sub) = &mut self.subsurface {
                        sub.window_open = true;
                    }
                }
                Action::Relief(on) => {
                    self.settings.relief = on;
                    self.heights.write().set_scale(if on { self.settings.scale } else { 0.0 });
                }
                Action::Extra(e, on) => {
                    self.extras.insert(e);
                    self.set_extra(e, on);
                }
                Action::RemoveExtra(e) => {
                    self.extras.remove(&e);
                    self.set_extra(e, false);
                }
                Action::Rows(rows) => self.set_vector_rows(rows),
                Action::RemoveGroup(n) => {
                    let mut rows = self.vector.rows.clone();
                    rows.retain(|r| !crate::vector::TREE[n].2.iter().any(|(k, _, _)| k == r));
                    self.vector.seen.remove(&n);
                    self.set_vector_rows(rows);
                }
                Action::Sources => self.credits.sources_open = true,
            Action::SatGroups(groups) => {
                self.settings.sat_groups = groups.clone();
                self.settings_dirty = true;
                if let Some(s) = self.plugged.values_mut().find_map(|l| l.satellites_mut()) {
                    s.set_groups(groups.into_iter().collect());
                }
            }
                Action::Catalog => self.catalog.open = !self.catalog.open,
                Action::Volumes(name) => self.volumes = Some(crate::volumecalc::VolumeCalc::new(name)),
                Action::Isolines(name) => self.raster_isolines(&name),
                Action::ExtraToLayers(extra) => self.extra_to_layers(extra),
                Action::Bathymetry(name) => {
                    self.close_side();
                    let level = self.shore_level(&name);
                    self.bathy = Some(crate::bathymetry::Dialog::new(&name, self.relief.lake(&name), level));
                }
            }
        }
    }

    /// Слой каталога включить или выключить.
    pub(super) fn set_extra(&mut self, extra: crate::layers::Extra, on: bool) {
        use crate::layers::Extra;
        match extra {
            Extra::Air => self.settings.air = on,
            Extra::Stars => self.settings.stars = on,
            Extra::Sun => self.settings.sun = on,
            Extra::Grid => self.settings.grid = on,
            Extra::Map(_) => {}
            Extra::Plugged(n) => {
                if on {
                    if let Some(entry) = crate::registry::get(n).filter(|_| !self.plugged.contains_key(&n)) {
                        let data_dir = self.data_dir();
                        let loaders = &self.loaders;
                        let request = |limit: u64| (loaders.request)(limit);
                        let cx = crate::registry::Make { lang: self.lang, data_dir: data_dir.as_deref(), request: &request };
                        let (mut layer, message) = entry.make(&cx);
                        // Спутники - выбранных в настройках групп.
                        if let Some(s) = layer.satellites_mut() {
                            *s = crate::satellites::Satellites::new(&self.settings.sat_groups, self.satellite_dir());
                        }
                        self.plugged.insert(n, layer);
                        if message.is_some() {
                            self.open_message = message;
                        }
                    }
                } else if let Some(mut layer) = self.plugged.remove(&n) {
                    // Живые объекты снятой строки: камера следом отпускает,
                    // модели, точки и подписи собираются заново.
                    if layer.satellites().is_some() {
                        self.sat_points.clear();
                        self.sat_shown.clear();
                        self.dots.dirty = true;
                    }
                    if let Some(kind) = layer.live().map(|l| l.kind) {
                        if self.live_follow.as_ref().is_some_and(|(k, _)| *k == kind) {
                            self.live_follow = None;
                        }
                        self.live_keys.clear();
                        if let Some(globe) = &mut self.globe {
                            globe.set_live_models(&[]);
                        }
                        self.live_points.clear();
                        self.live_labels.clear();
                        self.dots.dirty = true;
                    }
                    // Сетки снятого слоя уходят из видеокарты.
                    if let Some(globe) = &mut self.globe {
                        layer.clear(globe);
                    }
                    self.refresh_painters();
                }
            }
            Extra::Geoid => {
                if on && self.geoid.is_none() {
                    let source = net::Source { name: "geoid", url: data::geoid::TILE_URL, max_requests: 4, max_level: data::geoid::MAX_LEVEL, ext: "png", placeholders: false, raw: false, tilejson: None, region: None };
                    self.geoid = Some(GeoidState { loader: (self.loaders.make)(source), scale: self.settings.geoid_scale });
                    let mut store = self.heights.write();
                    store.clear();
                    store.max_level = data::geoid::MAX_LEVEL;
                    store.set_scale(self.settings.geoid_scale);
                } else if !on && self.geoid.take().is_some() {
                    let mut store = self.heights.write();
                    store.clear();
                    store.max_level = terrain::MAX_LEVEL;
                    store.set_scale(if self.settings.relief { self.settings.scale } else { 0.0 });
                }
                if let Some(globe) = &mut self.globe {
                    globe.stale_meshes(None);
                }
            }
            Extra::Paleo => {
                if on && self.paleo.is_none() {
                    let state = PaleoState { index: 0, playing: false, last: 0.0, before: self.basemap, rows: self.vector.rows.clone() };
                    self.paleo = Some(state);
                    self.set_vector_rows(std::collections::BTreeSet::new());
                    self.heights.write().set_scale(0.0);
                    self.set_basemap(Basemap::Paleo(data::paleo::AGES[0]));
                } else if !on && let Some(state) = self.paleo.take() {
                    self.set_basemap(state.before);
                    self.set_vector_rows(state.rows);
                    let scale = if self.settings.relief { self.settings.scale } else { 0.0 };
                    self.heights.write().set_scale(scale);
                }
            }
            Extra::Cutaway => {
                // Сектор под точкой взгляда в момент включения, как
                // `_set_cutaway` модуля.
                self.cutaway = on.then(|| crate::cutaway::Cutaway::new(self.nav.pose.lat, self.nav.pose.lon, self.settings.stretch, self.crust_cache().as_deref()));
                self.nav.camera.deep = on;
                if !on {
                    self.labels.wedge = None;
                }
                if let Some(globe) = &mut self.globe
                    && !on
                {
                    globe.set_wedge(None);
                    globe.set_subsurface("cutaway", None);
                    globe.set_subsurface("slabs", None);
                }
            }
            Extra::Dots(layer) => {
                if on {
                    let loaders = &self.loaders;
                    self.dots.enable(layer, &|| (loaders.request)(layer.bytes()));
                } else {
                    self.dots.disable(layer);
                }

                self.settings.dots = crate::dots::Layer::ALL.into_iter().filter(|l| if *l == layer { on } else { self.extras_on.contains(&Extra::Dots(*l)) }).map(|l| l.key().to_string()).collect();
            }
        }
        if on {
            // Карта значений или готовая карта на глобусе одна, как
            // у модуля: включение другой снимает прежнюю.
            if let Some(slot) = extra_slot(extra) {
                let others: Vec<Extra> = self.extras_on.iter().copied().filter(|e| *e != extra && extra_slot(*e) == Some(slot)).collect();
                for other in others {
                    self.set_extra(other, false);
                }
            }
            self.extras_on.insert(extra);
            self.extras.insert(extra);
        } else {
            self.extras_on.remove(&extra);
        }
        if matches!(extra, Extra::Map(_)) {
            self.sync_maps();
        }
        self.settings_dirty = true;
    }

    /// Попросить весь каталог NASA GIBS, один раз за запуск.
    pub(super) fn ask_gibs_catalog(&mut self) {
        if self.gibs_catalog.is_none() && !crate::maps::has_catalog() {
            let request = (self.loaders.request)(GIBS_CATALOG_BYTES);
            request.start(data::gibs::CAPABILITIES);
            self.gibs_catalog = Some(request);
        }
    }

    /// Карты картинками на глобусе - по включённым строкам: снизу карта
    /// значений или готовая карта, над ней наложения. Загрузчики заводятся
    /// и убираются вместе со строками.
    pub(super) fn sync_maps(&mut self) {
        let mut on: Vec<(&'static str, crate::maps::Kind)> = (0..crate::maps::count())
            .filter(|n| self.extras_on.contains(&crate::layers::Extra::Map(*n)))
            .filter_map(crate::maps::entry)
            .filter_map(|e| crate::maps::kind(e.key).map(|k| (e.key, k)))
            .collect();
        // Слои каталога GIBS, который ещё не пришёл, остаются в настройках.
        self.settings.maps = on.iter().map(|(id, _)| id.to_string()).chain(self.gibs_pending.iter().cloned()).collect();
        // Строка температуры поверхности - две картинки: море под сушей.
        if let Some(n) = on.iter().position(|(id, _)| *id == crate::maps::TEMPERATURE) {
            on.insert(n, (crate::maps::TEMPERATURE_SEA, crate::maps::Kind::Temperature(crate::maps::Surface::Sea)));
        }
        // Тема GIBS ложится, когда известен день ряда: до ответа ряда
        // дат картинки тайлов ждали бы её и не рисовались. У слоя всего
        // каталога ряд пришёл с каталогом.
        let now_utc = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
        for (id, kind) in &on {
            if let crate::maps::Kind::Theme(theme) = kind
                && !self.theme_days.contains_key(id)
            {
                let state = if theme.intervals.is_some() {
                    let ready = theme.intervals.map(|i| data::gibs::ready_intervals(i, now_utc)).unwrap_or_default();
                    theme.catalog_day(now_utc).map_or(ThemeDay::Failed, |day| ThemeDay::Day(day, ready))
                } else {
                    let request = (self.loaders.request)(ANSWER_BYTES);
                    request.start(&theme.domains_url());
                    ThemeDay::Asking(request)
                };
                self.theme_days.insert(id, state);
            }
        }
        on.retain(|(id, kind)| !matches!(kind, crate::maps::Kind::Theme(_)) || matches!(self.theme_days.get(id), Some(ThemeDay::Day(..))));
        // Цвета горизонталей - по подложке: на снимке жёлтые, на карте
        // коричневые, как у модуля.
        let source = self.basemap.source();
        let palette = if data::contours::is_imagery(self.basemap.key(), source.url, false) { data::contours::Palette::Imagery } else { data::contours::Palette::Map };
        for (_, kind) in &mut on {
            if let crate::maps::Kind::Contours(p) = kind {
                *p = palette;
            }
        }
        on.sort_by_key(|(_, k)| !k.is_single());
        // Границы полей - своим загрузчиком архива PMTiles.
        let year = on.iter().find_map(|(_, k)| if let crate::maps::Kind::Fields(y) = k { Some(*y) } else { None });
        if self.field_tiles.as_ref().map(|f| f.year) != year {
            let loaders = &self.loaders;
            self.field_tiles = year.map(|y| crate::fieldmap::FieldTiles::new(y, &|limit: u64| (loaders.request)(limit)));
        }
        self.map_loaders.retain(|k, _| on.iter().any(|(id, _)| id == k));
        for (id, kind) in &on {
            if !self.map_loaders.contains_key(id) {
                let mut sources = kind.sources(id);
                if let (crate::maps::Kind::Theme(theme), Some(ThemeDay::Day(day, _))) = (kind, self.theme_days.get(id)) {
                    let url = lasting(&mut self.lasting, theme.url(day));
                    let name = lasting(&mut self.lasting, format!("gibs-{}-{day}", theme.key));
                    let max_level = theme.level;
                    let ext = if theme.ext == "jpeg" { "jpg" } else { "png" };
                    sources = vec![(0, net::Source { name, url, max_requests: 4, max_level, ext, placeholders: false, raw: false, tilejson: None, region: None })];
                    // Шкала темы - по её цветовой карте.
                    if !self.theme_scales.contains_key(id) {
                        let state = match theme.colormap_url() {
                            Some(url) => {
                                let request = (self.loaders.request)(ANSWER_BYTES);
                                request.start(&url);
                                ThemeScale::Asking(request)
                            }
                            None => ThemeScale::Ready(None),
                        };
                        self.theme_scales.insert(id, state);
                    }
                }
                let loaders = sources.into_iter().map(|(level, source)| (level, (self.loaders.make)(source))).collect();
                self.map_loaders.insert(id, loaders);
            }
        }
        // Непрозрачность карт из настроек.
        for (key, alpha) in &self.map_opacity {
            if let Some(entry) = crate::maps::index(key).and_then(crate::maps::entry) {
                self.vector.set_opacity(entry.key, *alpha);
            }
        }
        if self.vector.set_maps(on)
            && let Some(globe) = &mut self.globe
        {
            globe.clear_overlays();
        }
    }

    /// Новые строки векторной основы: линии рисуются заново, подписи - по
    /// новым классам.
    pub(super) fn set_vector_rows(&mut self, rows: std::collections::BTreeSet<crate::vector::Row>) {
        if self.vector.set_rows(rows)
            && let Some(globe) = &mut self.globe
        {
            globe.clear_overlays();
        }
        self.labels.kinds = self.vector.label_kinds();
        self.settings.vector = self.vector.to_text();
        self.settings_dirty = true;
    }

    /// «Мои слои» прошлого запуска - в раздел, первым кадром: в программе
    /// объекты лежат файлами в папке данных, на странице - в самом
    /// списке. Слои, открытые до этого, остаются сверху.
    pub(super) fn restore_my_layers(&mut self) {
        let Some(text) = self.my_layers_saved.take() else {
            return;
        };
        let dir = self.user_dir.as_ref().map(|d| d.join(crate::datalayers::DIR));
        let read = |name: &str| dir.as_ref().and_then(|d| std::fs::read(d.join(name)).ok());
        let (saved, lost) = crate::datalayers::DataLayers::from_index(&text, &read);
        self.data_layers.restore(saved);
        if !lost.is_empty() {
            self.open_message = Some(trf(self.lang, "My layers: the objects of these layers were not found: {names}", &[("names", &lost.join(", "))]));
        }
    }

    /// «Мои слои» - в хранилище, если изменились. В программе объекты
    /// новых слоёв пишутся файлами GeoJSON в папку данных, файлы убранных
    /// слоёв удаляются. На странице объекты идут в сам список, пока он
    /// меньше [`crate::datalayers::INLINE_LIMIT`].
    pub(super) fn save_my_layers(&mut self, storage: &mut dyn eframe::Storage) {
        // Список прошлого запуска ещё не прочитан - не затирать его.
        if !self.data_layers.save_dirty || self.my_layers_saved.is_some() {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            // Без папки данных слои живут до закрытия программы.
            let Some(dir) = self.user_dir.as_ref().map(|d| d.join(crate::datalayers::DIR)) else {
                return;
            };
            if let Err(e) = std::fs::create_dir_all(&dir) {
                self.open_message = Some(format!("{}: {e}", dir.display()));
                return;
            }
            for (id, name) in self.data_layers.unstored() {
                let Some(layer) = self.data_layers.get(id) else {
                    continue;
                };
                match std::fs::write(dir.join(&name), crate::datalayers::DataLayers::stored_bytes(layer)) {
                    Ok(()) => self.data_layers.stored(id, name),
                    Err(e) => self.open_message = Some(format!("{}: {e}", dir.join(&name).display())),
                }
            }
            for name in std::mem::take(&mut self.data_layers.dropped) {
                // Файла уже нет - удалять нечего.
                let _ = std::fs::remove_file(dir.join(name));
            }
            storage.set_string(crate::datalayers::KEY, self.data_layers.index(false));
        }
        #[cfg(target_arch = "wasm32")]
        {
            let text = self.data_layers.index(true);
            if text.len() > crate::datalayers::INLINE_LIMIT {
                self.open_message = Some(tr(self.lang, "My layers exceed 1 MB, the page keeps them until it is closed").to_string());
            } else {
                storage.set_string(crate::datalayers::KEY, text);
            }
        }
        self.data_layers.save_dirty = false;
    }

    /// Изолинии слоем «Моих слоёв» сверху списка: коричневые линии, как
    /// горизонтали модуля в проекте. Возвращает текст сообщения.
    pub(super) fn isolines_to_layers(&mut self, data: data::layers::LayerData, step: f64) -> String {
        let (name, count) = (data.name.clone(), data.items.len());
        // RustPlanetX: «Моих слоёв» нет, изолинии - в проект RustGIS.
        if count > 0
            && self.edition.linked()
            && let Some(text) = self.layers_to_rustgis(&name, vec![data.clone()], Vec::new())
        {
            return text;
        }
        if self.data_layers.add(vec![data], &name) == 0 {
            return tr(self.lang, "The view has no contours: the terrain is flat or the heights are not loaded.").to_string();
        }
        if let Some(id) = self.data_layers.list.first().map(|l| l.id) {
            let [r, g, b] = data::contours::LAND;
            let style = crate::datalayers::Style { color: [r, g, b], width: crate::isolines::WIDTH, ..crate::datalayers::Style::new(0) };
            self.data_layers.apply(id, &crate::datalayers::Act::Style(style));
        }
        trf(self.lang, "Contours added to «My layers»: {count}, interval {step} m.", &[("count", &count.to_string()), ("step", &data::contours::label_text(step))])
    }

    /// Объекты строки витрины в рамке вида - слоями «Моих слоёв» в папку
    /// «Слои глобуса», как «В проект QGIS…» строки модуля в группу
    /// «PlanetX - слои глобуса». Землетрясения и пожары - в окне шкалы.
    pub(super) fn extra_to_layers(&mut self, extra: crate::layers::Extra) {
        let lang = self.lang;
        if let Some(entry) = extra.map() {
            self.map_to_layers(entry.key, tr(lang, extra.name()));
            return;
        }
        let crate::layers::Extra::Dots(layer) = extra else {
            return;
        };
        let found = self.dots.export(lang, layer, self.view_box(), self.timebar.window());
        let count: usize = found.iter().map(|l| l.items.len()).sum();
        let title = tr(lang, layer.name()).to_string();
        if count > 0
            && self.edition.linked()
            && let Some(text) = self.layers_to_rustgis(&title, found.clone(), Vec::new())
        {
            self.open_message = Some(text);
            return;
        }
        self.open_message = Some(if count == 0 {
            trf(lang, "No objects of the layer {title} in the view.", &[("title", &title)])
        } else {
            self.data_layers.add_to(found, &title, tr(lang, "Globe layers"));
            trf(lang, "Added to «My layers», objects {count}: {title}.", &[("count", &count.to_string()), ("title", &title)])
        });
    }

    /// Карта key в рамке вида, как её рисует глобус, - слоем-картинкой
    /// «Моих слоёв» в папку «Слои глобуса», раскраска вида строки модуля
    /// в проект. Карты строки сводятся одна на другую. Сторона картинки -
    /// не больше `analysis::PICTURE_SIDE`.
    fn map_to_layers(&mut self, key: &'static str, title: &str) {
        let lang = self.lang;
        let [west, south, east, north] = self.view_box();
        // Рамка через линию перемены дат - долготы за 180.
        let east = if east < west { east + 360.0 } else { east };
        let vector = &self.vector;
        // Температура поверхности - две карты под одной строкой: море снизу,
        // суша сверху, как их рисует глобус.
        let ids: &[&str] = if key == crate::maps::TEMPERATURE { &[crate::maps::TEMPERATURE_SEA, crate::maps::TEMPERATURE] } else { &[key] };
        let sample = |points: &[(f64, f64)]| {
            let mut out = vec![[0u8; 4]; points.len()];
            for id in ids {
                for (low, top) in out.iter_mut().zip(vector.sample_map(id, points)) {
                    *low = crate::vector::over(top, *low);
                }
            }
            out
        };
        let layer = crate::analysis::picture(title.to_string(), key, (south, north, west, east), 1.0, &sample, Vec::new());
        let png = layer.rgba.iter().any(|p| p[3] > 0).then(|| layer.png()).flatten();
        let Some(png) = png else {
            self.open_message = Some(trf(lang, "The map {title} is not drawn in the view yet.", &[("title", title)]));
            return;
        };
        let mut ground = scene::overlays::Overlay::new(scene::overlays::Kind::Ground);
        ground.set_box(layer.frame());
        let mut overlay = scene::kml::Overlay::new(title, ground);
        overlay.image = Some(png);
        if self.edition.linked()
            && let Some(text) = self.layers_to_rustgis(title, Vec::new(), vec![overlay.clone()])
        {
            self.open_message = Some(text);
            return;
        }
        self.data_layers.add_picture(overlay, tr(lang, "Globe layers"));
        self.open_message = Some(trf(lang, "Added to «My layers» as a picture: {title}.", &[("title", title)]));
    }

    /// Изолинии своего растра name - слоем «Моих слоёв», «Изолинии
    /// в проект QGIS…» меню растра модуля.
    pub(super) fn raster_isolines(&mut self, name: &str) {
        let lang = self.lang;
        let Some((_, bytes)) = self.relief.files().into_iter().find(|(n, _)| n == name) else {
            return;
        };
        self.open_message = Some(match data::grid::Grid::read(bytes.to_vec(), name).and_then(|grid| crate::isolines::raster(&grid).map_err(|key| tr(lang, key).to_string())) {
            Ok((step, lines)) => {
                let title = trf(lang, "{name} · isolines {step}", &[("name", name), ("step", &data::contours::label_text(step))]);
                self.isolines_to_layers(crate::isolines::layer(&title, lines), step)
            }
            Err(e) => e,
        });
    }

    /// Действие меню слоя «Моих слоёв».
    pub(super) fn data_act(&mut self, id: u64, act: &crate::datalayers::Act, now: f64) {
        use crate::datalayers::Act;
        match act {
            Act::ToRustGis => {
                #[cfg(not(target_arch = "wasm32"))]
                if self.rustgis.as_ref().is_some_and(|s| s.connected)
                    && let Some(layer) = self.data_layers.get(id)
                {
                    // Слой-картинка - растром GeoTIFF, остальные - объектами.
                    let name = data::layers::stem(&layer.title).to_string();
                    if layer.picture.is_some() {
                        if let Err(e) = self.picture_to_rustgis(id, &name) {
                            self.open_message = Some(trf(self.lang, "Not saved: {e}", &[("e", &e)]));
                        }
                    } else if let Some(sync) = &self.rustgis {
                        sync.link.ask(crate::rustgislink::Ask::AddLayer { name, geojson: data::layers::to_geojson(&layer.data) });
                    }
                }
            }
            Act::FolderToRustGis(folder) => {
                #[cfg(not(target_arch = "wasm32"))]
                if self.rustgis.as_ref().is_some_and(|s| s.connected) {
                    // Картинки - растрами GeoTIFF, остальные слои - объектами.
                    let layers: Vec<(u64, String, bool)> = self.data_layers.list.iter().filter(|l| &l.folder == folder && l.live.is_none()).map(|l| (l.id, format!("{folder} · {}", data::layers::stem(&l.title)), l.picture.is_some())).collect();
                    let mut failed = None;
                    for (layer_id, name, picture) in layers {
                        if picture {
                            if let Err(e) = self.picture_to_rustgis(layer_id, &name) {
                                failed = Some(e);
                            }
                        } else if let (Some(sync), Some(layer)) = (&self.rustgis, self.data_layers.get(layer_id)) {
                            sync.link.ask(crate::rustgislink::Ask::AddLayer { name, geojson: data::layers::to_geojson(&layer.data) });
                        }
                    }
                    if let Some(e) = failed {
                        self.open_message = Some(trf(self.lang, "Not saved: {e}", &[("e", &e)]));
                    }
                }
                #[cfg(target_arch = "wasm32")]
                let _ = folder;
            }
            // Живой слой RustGIS перечитывается из RustGIS: объекты
            // и оформление, файл - с диска.
            Act::Reload => match self.data_layers.get(id).and_then(|l| l.live) {
                #[cfg(not(target_arch = "wasm32"))]
                Some(live) => {
                    if let Some(sync) = self.rustgis.as_ref().filter(|s| s.connected) {
                        sync.link.ask(crate::rustgislink::Ask::Restyle { layer: live });
                    }
                }
                _ => self.reload_layer(id),
            },
            Act::Export => {
                if let Some(layer) = self.data_layers.get(id) {
                    let name = format!("{}.geojson", data::layers::stem(&layer.title));
                    match crate::export::save_bytes(&name, ("GeoJSON", "geojson"), data::layers::to_geojson(&layer.data).as_bytes()) {
                        Ok(Some(path)) => self.open_message = Some(trf(self.lang, "Saved: {to}", &[("to", &path)])),
                        Ok(None) => {}
                        Err(e) => self.open_message = Some(e),
                    }
                }
            }
            Act::ExportPicture => {
                let lang = self.lang;
                self.open_message = match self.data_layers.picture_geotiff(id) {
                    Ok((name, bytes)) => match crate::export::save_bytes(&name, ("GeoTIFF", "tif"), &bytes) {
                        Ok(Some(path)) => Some(trf(lang, "Saved: {to}", &[("to", &path)])),
                        Ok(None) => None,
                        Err(e) => Some(e),
                    },
                    Err(key) => Some(tr(lang, key).to_string()),
                };
            }
            Act::Zoom => {
                let points = self.data_layers.get(id).map(crate::datalayers::DataLayer::extent_points).unwrap_or_default();
                let camera = &self.nav.camera;
                if let Some(pose) = marks::fit_points(&points, camera.fov_y, camera.aspect()) {
                    crate::input::fly(&mut self.nav, pose, now);
                }
            }
            Act::ToPlaces => {
                if let Some(layer) = self.data_layers.get(id) {
                    // Слой-картинка - наложением на землю, иначе - метками.
                    let folder = match &layer.picture {
                        Some(p) => {
                            let mut f = scene::kml::Folder::new(&layer.title);
                            f.children.push(scene::kml::Item::Overlay(p.clone()));
                            f
                        }
                        None => crate::gpkg::folder(vec![layer.data.clone()], &layer.title),
                    };
                    let title = layer.title.clone();
                    self.marks.add(folder, &title, marks::root_name(self.lang));
                    self.marks_changed();
                }
            }
            // Трек по точечному слою - в окно «Треки», шкала растит путь.
            Act::Track => {
                let lang = self.lang;
                let made = self.data_layers.get(id).and_then(|l| crate::tracks::Layer::from_layer_with(&l.data, &l.title, l.time.map(|t| t.0)));
                self.open_message = Some(match made {
                    Some(track) => {
                        let title = track.title.clone();
                        self.tracks.add(vec![track], &title);
                        self.tracks.open = true;
                        self.time_extent();
                        trf(lang, "Track added: {title}. The time slider grows the path.", &[("title", &title)])
                    }
                    None => tr(lang, "The layer has no field with dates: choose it in the Time submenu.").to_string(),
                });
            }
            // Команда аддина: сначала окно её параметров.
            #[cfg(feature = "addins")]
            Act::Addin { addin, command } => {
                if let Some(l) = self.data_layers.get(id) {
                    let count = if l.selected.is_empty() { l.data.items.len() } else { l.selected.len() };
                    let title = l.title.clone();
                    self.addins.ask_params(id, &title, count, addin, command);
                }
            }
            // Год карты: охват шкалы заново, шкала открывается на годе
            // карты, как `edit_map_year` модуля.
            Act::Year(year) => {
                self.data_layers.apply(id, act);
                self.data_extent_seen = self.data_layers.extent();
                self.time_extent();
                if let Some(y) = year {
                    self.timebar.show_time(&scene::oldmaps::year_time(*y));
                }
            }
            _ => {
                self.data_layers.apply(id, act);
            }
        }
    }

    /// Команда аддина над объектами слоя: выделенными, без выделения -
    /// всеми. Результат - новый слой «Моих слоёв».
    #[cfg(feature = "addins")]
    pub(super) fn run_addin(&mut self, run: crate::addins::Run) {
        use data::layers::Shape as Geom;
        let lang = self.lang;
        let Some(layer) = self.data_layers.get(run.layer) else {
            return;
        };
        let picked: Vec<usize> = if layer.selected.is_empty() { (0..layer.data.items.len()).collect() } else { layer.selected.clone() };
        let mut features: Vec<addin_api::Feature> = Vec::new();
        for n in &picked {
            let Some(item) = layer.data.items.get(*n) else {
                continue;
            };
            let name = layer.feature_name(*n);
            let all = || -> Vec<Vec<[f64; 3]>> {
                item.parts.iter().enumerate().map(|(p, part)| part.iter().enumerate().map(|(k, (lat, lon))| [*lat, *lon, item.heights.get(p).and_then(|h| h.get(k)).copied().unwrap_or(0.0)]).collect()).collect()
            };
            match item.shape {
                Geom::Point => features.push(addin_api::Feature { kind: addin_api::Kind::Point, parts: all(), name }),
                Geom::Line => features.push(addin_api::Feature { kind: addin_api::Kind::Line, parts: all(), name }),
                // Части полигона Lite - внешние кольца и дыры вперемешку:
                // каждый внешний контур с его дырами - свой объект аддина.
                Geom::Polygon => {
                    for (outer, holes) in crate::datalayers::polygons(&item.parts) {
                        let ring = |r: &[(f64, f64)]| r.iter().map(|(lat, lon)| [*lat, *lon, 0.0]).collect::<Vec<_>>();
                        let mut parts = vec![ring(&outer)];
                        parts.extend(holes.iter().map(|h| ring(h)));
                        features.push(addin_api::Feature { kind: addin_api::Kind::Polygon, parts, name: name.clone() });
                    }
                }
                Geom::None => {}
            }
        }
        let title = self.addins.manifest(&run.addin).and_then(|m| m.commands.iter().find(|c| c.id == run.command)).map_or(run.command.clone(), |c| c.title.get(lang == crate::i18n::Lang::RU).trim_end_matches('…').to_string());
        let source = layer.title.clone();
        let params = serde_json::Value::Object(run.values);
        // Числа параметров - в название слоя: «ЛЭП · Буфер 100 м».
        let note: Vec<String> = params.as_object().map(|o| o.values().filter_map(|v| v.as_f64().map(|x| format!("{x}")).or_else(|| v.as_str().map(str::to_string))).collect()).unwrap_or_default();
        let unit = self.addins.manifest(&run.addin).and_then(|m| m.commands.iter().find(|c| c.id == run.command)).and_then(|c| c.params.first()).and_then(|p| match &p.kind {
            addin_host::ParamKind::Number { unit, .. } => Some(unit.clone()),
            _ => None,
        });
        let name = match (note.first(), unit) {
            (Some(n), Some(u)) if !u.is_empty() => format!("{source} · {title} {n} {}", if u == "m" && lang == crate::i18n::Lang::RU { "м" } else { u.as_str() }),
            _ => format!("{source} · {title}"),
        };
        self.open_message = Some(match self.addins.run_command(&run.addin, &run.command, &params, &features) {
            Ok(out) => {
                let items: Vec<data::layers::Item> = out
                    .iter()
                    .map(|f| data::layers::Item {
                        shape: match f.kind {
                            addin_api::Kind::Point => Geom::Point,
                            addin_api::Kind::Line => Geom::Line,
                            addin_api::Kind::Polygon => Geom::Polygon,
                        },
                        parts: f.parts.iter().map(|p| p.iter().map(|v| (v[0], v[1])).collect()).collect(),
                        heights: Vec::new(),
                        values: vec![f.name.clone()],
                    })
                    .collect();
                let count = items.len();
                let data = data::layers::LayerData { name: name.clone(), epsg: Some(4326), fields: vec!["name".into()], numeric: vec![false], items };
                if count == 0 || self.data_layers.add(vec![data], &name) == 0 {
                    trf(lang, "The add-in gave no objects: {name}.", &[("name", &name)])
                } else {
                    trf(lang, "Added to «My layers», objects {count}: {title}.", &[("count", &count.to_string()), ("title", &name)])
                }
            }
            Err(e) => trf(lang, "The add-in failed: {e}", &[("e", &e)]),
        });
    }

    /// Состояние для раздела «Слои» и каталога.
    pub(super) fn layers_state(&self) -> crate::layers::State<'_> {
        crate::layers::State {
            basemap: self.basemap,
            relief: self.settings.relief,
            extras: &self.extras,
            on: &self.extras_on,
            vector: &self.vector,
            own: self.relief.names().into_iter().map(str::to_string).collect(),
            sea: self.settings.sea,
            ground: self.subsurface.as_ref().map(|s| s.options.opacity),
            opacity: &self.map_opacity,
            stretch: self.settings.stretch,
            geoid_scale: self.settings.geoid_scale,
            sat_groups: &self.settings.sat_groups,
            earth: self.planet.earth,
            events: self.events.shown.is_some(),
            wayback: self.wayback.is_some(),
            data: &self.data_layers,
            tiles: &self.tilesets,
        }
    }

    /// Раздел «Мои слои» левой панели между «Метками» и «Слоями», как
    /// «Проект QGIS» модуля, решение автора от 6 ноября 2026 года: слои
    /// файлов и результаты окон анализа в порядке отрисовки. Высота - по
    /// строкам, не больше [`MY_LAYERS_ROWS`], дальше прокрутка.
    pub(super) fn my_layers_section(&mut self, ui: &mut egui::Ui, lang: Lang, now: f64) {
        if !crate::theme::section(ui, "my-layers", tr(lang, "My layers")) {
            return;
        }
        let mut data = Vec::new();
        let mut menu = self.layers_menu.take();
        let mut folder = std::mem::take(&mut self.new_folder);
        // Пустой раздел - подсказка в четыре строки, ей нужно место трёх строк слоёв.
        let rows = crate::datalayers::row_count(&self.data_layers).max(3) as f32;
        let room = (22.0 * rows.min(MY_LAYERS_ROWS) + 36.0).min(ui.available_height() - SECTION_MIN).max(48.0);
        egui::ScrollArea::vertical().id_salt("my-layers").max_height(room).auto_shrink([false, true]).show(ui, |ui| {
            crate::datalayers::rows(ui, lang, &self.data_layers, &mut menu, &mut folder, &mut data);
        });
        self.layers_menu = menu;
        self.new_folder = folder;
        // «Шторка слоёв» модуля: кнопка видна, пока у слоёв есть стороны.
        if self.data_layers.has_sides() {
            let mut on = self.layer_swipe.is_some();
            if ui.toggle_value(&mut on, tr(lang, "⇆ Layer swipe")).on_hover_text(tr(lang, "Comparing layers with a swipe. Layers «Left» are seen left of the line, layers «Right» - right of it, the others on both sides. The side of a layer is the Swipe submenu of the layer menu, the line is dragged with the mouse.")).changed() {
                self.layer_swipe = on.then_some(0.5);
            }
        } else {
            self.layer_swipe = None;
        }
        for (id, act) in data {
            self.data_act(id, &act, now);
        }
    }

    /// Загрузчик основы: встроенный или готовой подосновы.
    pub(super) fn base_loader(&self, basemap: Basemap) -> Option<&dyn TileLoader> {
        self.loaders.basemap(basemap).or_else(|| self.ready_loader.as_ref().filter(|(b, _)| *b == basemap).map(|(_, l)| l.as_ref()))
    }

    /// Сменить основу: снимки прежней забываются.
    pub(super) fn set_basemap(&mut self, basemap: Basemap) {
        if basemap == self.basemap {
            return;
        }
        self.basemap = basemap;
        self.pending.clear();
        // Подробная мозаика тела COG - свой загрузчик уровней после тайлов.
        let deep = match basemap {
            Basemap::Body(key) => scene::planets::planet_by_key(key).imagery.and_then(|i| i.deep.map(|d| (d, u32::from(i.max_level)))),
            _ => None,
        };
        let loaders = &self.loaders;
        self.cog_base = deep.map(|(d, from)| crate::cogtiles::CogTiles::new(d.url, d.clon, from + 1, u32::from(d.max_level), &|limit| (loaders.request)(limit)));
        if matches!(basemap, Basemap::Ready(_) | Basemap::Own(_) | Basemap::Paleo(_) | Basemap::Body(_)) && self.ready_loader.as_ref().is_none_or(|(b, _)| *b != basemap) {
            self.ready_loader = Some((basemap, (self.loaders.make)(basemap.source())));
        }
        // Карта палеогеографии - не выбор основы, в настройки не идёт.
        if !matches!(basemap, Basemap::Paleo(_) | Basemap::Body(_)) {
            self.settings.basemap = basemap.key().to_string();
            self.settings_dirty = true;
        }
        self.sync_maps();
        if let Some(globe) = &mut self.globe {
            globe.clear_textures();
        }
    }
}
