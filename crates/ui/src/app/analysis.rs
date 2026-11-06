// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окна анализа участка: шаг расчётов, общий слой вида, сохранение расчёта в «Мои слои», разрез и подземный режим.

use super::*;

impl LiteApp {
    /// Окно анализа открыто у точки: участок окна на случай, если точка
    /// внутри общего участка.
    pub(super) fn join_site(&mut self, choice: &crate::globemenu::Choice, lat: f64, lon: f64) {
        use crate::globemenu::Choice;
        let edit = match choice {
            Choice::Insolation => self.insolation.as_mut().and_then(crate::pointcalc::Calc::site_edit),
            Choice::Shadows => self.shadows.as_mut().and_then(crate::pointcalc::Calc::site_edit),
            Choice::Changes => self.aef_change.as_mut().and_then(crate::aefcalc::ChangeCalc::site_edit),
            Choice::Classes => self.aef_class.as_mut().and_then(crate::aefclass::ClassCalc::site_edit),
            Choice::Similar => self.similar.as_mut().and_then(crate::aefclass::ClassCalc::site_edit),
            Choice::Trace => self.tracing.as_mut().and_then(crate::aefclass::ClassCalc::site_edit),
            Choice::Objects => self.object_tasks.as_mut().and_then(crate::aefclass::ClassCalc::site_edit),
            Choice::Recognize => self.recognize.as_mut().and_then(crate::aefclass::ClassCalc::site_edit),
            Choice::Spectral => self.spectral.as_mut().and_then(crate::spectral::Spectral::site_edit),
            Choice::Emit => self.emit.as_mut().and_then(crate::emit::Emit::site_edit),
            Choice::SlopeHere => self.relief_calc.as_mut().and_then(crate::reliefcalc::ReliefCalc::site_edit),
            Choice::Flood => self.hydro.flood_site.as_mut(),
            Choice::Flow => self.hydro.flow_site.as_mut(),
            Choice::Earthwork => self.earthwork.site_edit(),
            _ => None,
        };
        if let Some(edit) = edit {
            edit.open_at(self.area_site.as_deref(), lat, lon);
        }
    }

    /// Пункты гидрологии подменю «Анализ участка» меню на глобусе.
    pub(super) fn hydro_choice(&mut self, choice: crate::globemenu::Choice, point: Option<(f64, f64)>, place: Option<(Vec<usize>, String)>) {
        use crate::globemenu::Choice;
        if choice == Choice::SeaLevel {
            self.hydro.sea_open = true;
            return;
        }
        let Some((lat, lon)) = point else {
            return;
        };
        let name = place.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
        let coords = format!("{lat:.5}, {lon:.5}");
        let title = if name.is_empty() { coords } else { format!("{name}  {coords}") };
        if choice == Choice::Flood {
            self.hydro.flood_here(lat, lon, title);
        } else if choice == Choice::Earthwork {
            self.earthwork.open_here(lat, lon, title);
        } else {
            self.hydro.flow_here(lat, lon, title);
        }
        self.join_site(&choice, lat, lon);
    }

    /// Расчёты у точки: окна, шаги, тайлы высот, слои над картами,
    /// шкала инсоляции.
    pub(super) fn step_calcs(&mut self, ctx: &egui::Context, now: f64) {
        let lang = self.lang;
        if let Some(w) = &mut self.etomesto
            && !w.show(ctx, lang)
        {
            self.etomesto = None;
        }
        let mut keys = Vec::new();
        let mut changed = false;
        {
            let store = self.heights.clone();
            let has = |k: TileKey| store.read().tile(k).is_some();
            let heights = self.true_heights();
            // Тени идут за правым бегунком шкалы времени, без шкалы - за
            // часами компьютера, как у модуля.
            let moment = self.timebar.window().map_or_else(crate::pointcalc::clock_now, |w| w.1);
            if let Some(c) = &mut self.shadows {
                c.set_moment(lang, moment);
            }
            for calc in [&mut self.viewshed, &mut self.insolation, &mut self.shadows].into_iter().flatten() {
                calc.show(ctx, lang, now);
                keys.extend(calc.step(lang, now, &has, &heights));
                changed |= std::mem::take(&mut calc.changed);
            }
            // Квадраты анализа рельефа и гидрологии: радиус тела и уровень
            // тайлов высот - у тела на глобусе.
            let radius = self.nav.camera.ellipsoid.a;
            let max_level = terrain_source(&self.planet).map_or(terrain::MAX_LEVEL, |s| s.max_level);
            if let Some(calc) = &mut self.relief_calc {
                calc.show(ctx, lang, now, radius, max_level);
                calc.step(lang, now, &has, &heights);
                keys.extend(calc.wanted(&has));
                changed |= std::mem::take(&mut calc.changed);
            }
            keys.extend(self.hydro.step(ctx, lang, now, radius, max_level, &has, &heights));
            let boxes = self.relief.boxes();
            keys.extend(self.earthwork.step(ctx, lang, now, radius, max_level, &boxes, &has, &heights));
            if let Some(calc) = &mut self.aef_change {
                calc.show(ctx, lang, &*self.loaders.request);
                calc.step(lang, &*self.loaders.request);
                changed |= std::mem::take(&mut calc.changed);
            }
            let rasters = self.open_rasters();
            // Подложка на глобусе - источник «Подложка глобуса».
            let base = crate::aefclass::Backdrop { name: self.basemap.name(lang).to_string(), source: self.basemap.source(), credit: self.basemap.attribution(lang).0.to_string() };
            for calc in [&mut self.aef_class, &mut self.similar, &mut self.tracing, &mut self.object_tasks, &mut self.recognize].into_iter().flatten() {
                calc.rasters = rasters.clone();
                calc.basemap = Some(base.clone());
                calc.accepted.clone_from(&self.settings.analysis_ok);
                calc.show(ctx, lang);
                calc.step(lang, &*self.loaders.request);
                changed |= std::mem::take(&mut calc.changed);
                if let Some(url) = calc.accept_new.take()
                    && !self.settings.analysis_ok.contains(&url)
                {
                    self.settings.analysis_ok.push(url);
                    self.settings_dirty = true;
                }
            }
            // Другая задача или источник «Распознавания»: окно
            // пересобирается с тем же участком, модуль 1.40.
            if let Some((goal, source)) = self.recognize.as_mut().and_then(|c| c.wants.take())
                && let Some(old) = self.recognize.take()
            {
                self.recognize = Some(old.rebuilt(lang, goal, source));
                changed = true;
            }
            // Снимки окна «Выделение объектов» - растры программы, источник
            // и других окон анализа.
            for calc in [&mut self.object_tasks, &mut self.recognize].into_iter().flatten() {
                for (name, bytes) in calc.loaded.drain(..) {
                    self.band_rasters.retain(|r| r.0 != name);
                    self.band_rasters.push((name, bytes));
                }
            }
            // «Уточнить точками», `refine` модуля 1.37: «Классификация»
            // с тем же участком, снимком и классами окна задач.
            if let Some((ring, source, pairs)) = self.object_tasks.as_mut().and_then(|c| c.take_refine(lang)) {
                let n = ring.len() as f64;
                let (lat, lon) = (ring.iter().map(|p| p.0).sum::<f64>() / n, ring.iter().map(|p| p.1).sum::<f64>() / n);
                let title = format!("{lat:.5}, {lon:.5}");
                let all = self.open_rasters();
                self.area_site = Some(ring);
                match &mut self.aef_class {
                    Some(c) => c.set_point(lat, lon, title),
                    None => self.aef_class = Some(crate::aefclass::ClassCalc::new(lat, lon, title, all.clone(), lang)),
                }
                self.join_site(&crate::globemenu::Choice::Classes, lat, lon);
                if let Some(c) = &mut self.aef_class {
                    c.rasters = all;
                    c.refine_from(lang, source, &pairs);
                }
            }
            // «Классы из трассировки»: классы окна задач, без них - окна
            // «Трассировка».
            if self.aef_class.as_ref().is_some_and(|c| c.wants_traced) {
                let pairs = self.object_tasks.as_ref().map(crate::aefclass::ClassCalc::class_pairs).filter(|p| !p.is_empty()).or_else(|| self.tracing.as_ref().map(crate::aefclass::ClassCalc::class_pairs)).unwrap_or_default();
                if let Some(c) = &mut self.aef_class {
                    c.take_traced(lang, &pairs);
                }
            }
            // Названия классов от помощника одним запросом.
            for which in 0..3u8 {
                let calc = match which {
                    0 => &mut self.tracing,
                    1 => &mut self.object_tasks,
                    _ => &mut self.recognize,
                };
                if let Some(calc) = calc
                    && let Some((system, text)) = calc.take_ask()
                {
                    let loaders = &self.loaders;
                    let request = |limit: u64| (loaders.request)(limit);
                    if self.ai.complete(lang, &system, &text, &self.settings.ai, &request) {
                        self.names_for = Some(which);
                    } else {
                        calc.ask_refused(lang);
                    }
                }
            }
            if let Some((answer, error)) = self.ai.completed.take() {
                let calc = match self.names_for.take() {
                    Some(0) => self.tracing.as_mut(),
                    Some(1) => self.object_tasks.as_mut(),
                    Some(_) => self.recognize.as_mut(),
                    None => None,
                };
                if let Some(calc) = calc {
                    calc.names_came(lang, &answer, &error);
                    changed = true;
                }
            }
            if let Some(calc) = &mut self.spectral {
                calc.rasters = rasters.clone();
                calc.show(ctx, lang, &*self.loaders.request);
                calc.step(lang, now, &*self.loaders.request);
                changed |= std::mem::take(&mut calc.changed);
                // Снимок ASTER окна - растр программы, источник и других
                // окон анализа, камера остаётся на месте.
                if let Some((name, bytes)) = calc.loaded.take() {
                    self.band_rasters.retain(|r| r.0 != name);
                    self.band_rasters.push((name, bytes));
                }
            }
            let dir = self.data_dir();
            if let Some(calc) = &mut self.emit {
                calc.key.clone_from(&self.settings.earthdata);
                calc.dir = dir;
                calc.show(ctx, lang, &*self.loaders.request);
                calc.step(ctx, lang, &*self.loaders.request);
                changed |= std::mem::take(&mut calc.changed);
                if let Some(key) = calc.key_new.take() {
                    self.settings.earthdata = key;
                    self.settings_dirty = true;
                }
            }
            changed |= std::mem::take(&mut self.hydro.changed);
            changed |= std::mem::take(&mut self.earthwork.changed);
            self.folder_windows(ctx, lang);
            let rasters = self.open_rasters();
            if let Some(v) = &mut self.volumes
                && !v.show(ctx, lang, &rasters, self.area_site.as_deref())
            {
                self.volumes = None;
            }
        }
        self.calc_keys = keys;
        if let Some(text) = self.hydro.message.take() {
            self.open_message = Some(text);
        }
        // Уровень ниже нынешнего открывает шельф: нужны глубины морей,
        // они включаются, как `set_sea_level` модуля.
        if self.hydro.sea_level < 0.0 && !self.settings.sea && self.planet.earth && self.geoid.is_none() {
            self.settings.sea = true;
            self.settings_dirty = true;
            self.sea_changed();
        }
        // Расчёты по «OK» - папками «Анализа участка» и файлами GeoTIFF.
        let mut saved: Vec<crate::analysis::Saved> = [&mut self.viewshed, &mut self.insolation, &mut self.shadows].into_iter().flatten().filter_map(|c| c.saved.take()).collect();
        saved.extend(self.relief_calc.as_mut().and_then(|c| c.saved.take()));
        saved.append(&mut self.hydro.saved);
        saved.extend(self.earthwork.saved.take());
        saved.extend(self.volumes.as_mut().and_then(|v| v.saved.take()));
        saved.extend(self.aef_change.as_mut().and_then(|c| c.saved.take()));
        saved.extend(self.aef_class.as_mut().and_then(|c| c.saved.take()));
        saved.extend(self.similar.as_mut().and_then(|c| c.saved.take()));
        saved.extend(self.tracing.as_mut().and_then(|c| c.saved.take()));
        saved.extend(self.object_tasks.as_mut().and_then(|c| c.saved.take()));
        saved.extend(self.recognize.as_mut().and_then(|c| c.saved.take()));
        saved.extend(self.spectral.as_mut().and_then(|c| c.saved.take()));
        saved.extend(self.emit.as_mut().and_then(|c| c.saved.take()));
        for s in saved {
            self.save_analysis(s);
        }
        // Квадрат уклона занимает место строк «Уклон» и «Экспозиция».
        if self.relief_calc.as_mut().is_some_and(|c| std::mem::take(&mut c.built)) {
            for key in ["slope", "aspect"] {
                if let Some(n) = crate::maps::index(key)
                    && self.extras_on.contains(&crate::layers::Extra::Map(n))
                {
                    self.set_extra(crate::layers::Extra::Map(n), false);
                }
            }
        }
        // Новый результат занимает общий слой вида, прежний снимается.
        changed |= self.site_layer_settle();
        if changed {
            self.refresh_painters();
        }
        // Окно закрыто и слоя нет - расчёт забыт.
        for slot in [&mut self.viewshed, &mut self.insolation, &mut self.shadows] {
            if slot.as_ref().is_some_and(|c| !c.open && c.layer.is_none() && !c.busy()) {
                *slot = None;
            }
        }
        if self.relief_calc.as_ref().is_some_and(|c| !c.open && c.layer().is_none() && !c.busy()) {
            self.relief_calc = None;
        }
        if self.aef_change.as_ref().is_some_and(|c| !c.open && c.layer.is_none() && !c.busy()) {
            self.aef_change = None;
        }
        if self.aef_class.as_ref().is_some_and(|c| !c.open && c.layer.is_none() && !c.busy()) {
            self.aef_class = None;
        }
        if self.similar.as_ref().is_some_and(|c| !c.open && c.layer.is_none() && !c.busy()) {
            self.similar = None;
        }
        if self.tracing.as_ref().is_some_and(|c| !c.open && c.layer.is_none() && !c.busy()) {
            self.tracing = None;
        }
        if self.object_tasks.as_ref().is_some_and(|c| !c.open && c.layer.is_none() && !c.busy()) {
            self.object_tasks = None;
        }
        if self.recognize.as_ref().is_some_and(|c| !c.open && c.layer.is_none() && !c.busy()) {
            self.recognize = None;
        }
        if self.spectral.as_ref().is_some_and(|c| !c.open && c.layer.is_none() && !c.busy()) {
            self.spectral = None;
        }
        if self.emit.as_ref().is_some_and(|c| !c.open && c.layer.is_none() && !c.busy()) {
            self.emit = None;
        }
        if let Some(crate::pointcalc::Layer::Insolation(r)) = self.insolation.as_ref().and_then(|c| c.layer.as_ref()) {
            // Над глобусом, левая панель её не закрывает.
            let rect = ctx.content_rect();
            let left = rect.right() - self.globe_width_px / ctx.pixels_per_point();
            crate::pointcalc::legend(ctx, lang, r.top(), egui::pos2(left.max(rect.left()) + 12.0, rect.bottom() - 120.0));
        }
    }

    /// Многоугольники, линии и точки расчёта - слоями «Моих слоёв» в папку
    /// folder, как слои проекта модуля 1.40. Папки меток, которые стали
    /// слоем, уходят из saved. Отдельные метки расчёта - вершины, створы -
    /// одним слоем «Метки расчёта».
    pub(super) fn vectors_to_layers(&mut self, saved: &mut crate::analysis::Saved, folder: &str) {
        let mut layers = Vec::new();
        // Описания папок - источник, настройки, классы - в подсказки слоёв.
        let mut abouts = Vec::new();
        saved.extra.retain(|item| match item {
            scene::kml::Item::Folder(f) => match crate::datalayers::folder_layer(f) {
                Some(layer) => {
                    abouts.push((layer.name.clone(), f.description.clone()));
                    layers.push(layer);
                    false
                }
                None => true,
            },
            _ => true,
        });
        let mut loose = scene::kml::Folder::new(tr(self.lang, "Calculation marks"));
        saved.extra.retain(|item| match item {
            scene::kml::Item::Place(p) => {
                loose.children.push(scene::kml::Item::Place(p.clone()));
                false
            }
            _ => true,
        });
        match crate::datalayers::folder_layer(&loose) {
            Some(layer) => layers.push(layer),
            // Слоем метки не стали - остаются метками.
            None => saved.extra.extend(loose.children),
        }
        for layer in layers.into_iter().rev() {
            let title = layer.name.clone();
            self.data_layers.add_to(vec![layer], &title, folder);
        }
        for (title, about) in abouts.iter().filter(|(_, a)| !a.trim().is_empty()) {
            self.data_layers.set_about(folder, title, about);
        }
    }

    /// Сохранённый расчёт анализа участка, `Session` и `write_lat_lon`
    /// модуля: файлы GeoTIFF слоёв - в программе в папку results папки
    /// данных, на странице скачиванием. Расчёт ложится в «Мои слои» папкой
    /// «Название ЧЧ:ММ:СС UTC», как результаты инструментов модуля - в
    /// проект своей группой, решение автора от 6 ноября 2026 года: контур
    /// участка и векторы - слоями, растры - слоями-картинками под ними.
    /// Расчёт той же сессии окна - в её папку, пока она есть. Что слоем
    /// не стало, - папкой «Анализа участка» «Моих меток».
    pub(super) fn save_analysis(&mut self, mut saved: crate::analysis::Saved) {
        let lang = self.lang;
        let unix = crate::pointcalc::clock_now();
        let mut files = Vec::new();
        let mut failed = None;
        for layer in &saved.layers {
            if layer.tiff.is_empty() {
                files.push(String::new());
                continue;
            }
            let name = crate::analysis::file_name(&layer.stem, unix);
            match self.write_result(&name, &layer.tiff) {
                Ok(path) => files.push(path),
                Err(e) => {
                    files.push(String::new());
                    failed = Some(e);
                }
            }
        }
        // RustPlanetX со связью: расчёт - в проект RustGIS своей группой,
        // как результаты инструментов модуля - в проект QGIS.
        #[cfg(not(target_arch = "wasm32"))]
        if self.edition.linked() {
            if self.rustgis.as_ref().is_some_and(|b| b.connected) {
                self.analysis_to_rustgis(&saved, &files, failed, unix);
            } else {
                // «Моих слоёв» у RustPlanetX нет: без связи расчёт
                // остаётся файлами.
                self.open_message = Some(tr(lang, "No link with RustGIS: the calculation is not added to the project. The GeoTIFF files are written to the results folder.").to_string());
            }
            return;
        }
        // Расчёт той же сессии - в её папку, пока она есть, `Session.group_for`
        // модуля: контур участка там уже лежит.
        let earlier = saved.session.and_then(|s| self.analysis_sessions.get(&s).cloned()).filter(|name| self.data_layers.list.iter().any(|l| &l.folder == name));
        let folder = earlier.clone().unwrap_or_else(|| format!("{} {} UTC", saved.title, crate::analysis::clock(unix)));
        if let Some(s) = saved.session {
            self.analysis_sessions.insert(s, folder.clone());
        }
        // Растры - слоями-картинками, векторы - над ними, контур - сверху.
        for picture in saved.pictures(lang, &files) {
            self.data_layers.add_picture(picture, &folder);
        }
        self.vectors_to_layers(&mut saved, &folder);
        if saved.ring.len() >= 3 && earlier.is_none() {
            let site = tr(lang, "Area|site");
            let mut outline = scene::kml::Folder::new(site);
            outline.children.push(scene::kml::Item::Place(scene::kml::Place::new(site, scene::features::Kind::Polygon, saved.ring.clone())));
            if let Some(layer) = crate::datalayers::folder_layer(&outline) {
                self.data_layers.add_to(vec![layer], site, &folder);
                let [r, g, b, _] = crate::analysis::SITE_COLOR;
                let style = crate::datalayers::Style { color: [r, g, b], width: 2.0, fill: 0.0, ..crate::datalayers::Style::new(0) };
                if let Some(id) = self.data_layers.list.iter().find(|l| l.folder == folder && l.title == site).map(|l| l.id) {
                    self.data_layers.apply(id, &crate::datalayers::Act::Style(style));
                }
            }
        }
        // Что слоем не стало - наложения, модели - папкой «Анализа участка»
        // «Моих меток».
        if !saved.extra.is_empty() {
            let root = marks::root_name(lang);
            let group = tr(lang, "Area analysis");
            let tree = self.marks.root(root);
            let found = tree.children.iter().position(|c| matches!(c, scene::kml::Item::Folder(f) if f.name == group));
            let at = match found {
                Some(i) => i,
                None => self.marks.insert_at(&[], self.marks.first_index(), scene::kml::Item::Folder(scene::kml::Folder::new(group)), root).first().copied().unwrap_or(0),
            };
            let mut rest = scene::kml::Folder::new(&folder);
            rest.children = std::mem::take(&mut saved.extra);
            self.marks.insert_at(&[at], 0, scene::kml::Item::Folder(rest), root);
            self.marks_changed();
        }
        let written: Vec<&str> = files.iter().map(String::as_str).filter(|f| !f.is_empty()).collect();
        self.open_message = Some(match failed {
            Some(e) => trf(lang, "Saved to «My layers», folder {folder}. The GeoTIFF file is not written: {e}", &[("folder", &folder), ("e", &e)]),
            None if written.is_empty() => trf(lang, "Saved to «My layers», folder {folder}.", &[("folder", &folder)]),
            None => trf(lang, "Saved to «My layers», folder {folder}, the values - {path}.", &[("folder", &folder), ("path", &written.join(", "))]),
        });
    }

    /// Файл результата name: в программе - в папку results папки данных,
    /// повтор имени получает номер, на странице - скачиванием. Возвращает
    /// путь или имя файла.
    pub(super) fn write_result(&self, name: &str, bytes: &[u8]) -> Result<String, String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let folder = self.data_dir().ok_or("нет папки данных")?.join(crate::analysis::FOLDER);
            std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
            let stem = name.trim_end_matches(".tif");
            let mut path = folder.join(name);
            let mut number = 2;
            while path.exists() {
                path = folder.join(format!("{stem}_{number}.tif"));
                number += 1;
            }
            std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(path.display().to_string())
        }
        #[cfg(target_arch = "wasm32")]
        {
            crate::export::save_bytes(name, ("GeoTIFF", "tif"), bytes)?;
            Ok(name.to_string())
        }
    }

    /// Слои над картами по готовым результатам: видимость, инсоляция,
    /// контуры горных разработок.
    pub(super) fn refresh_painters(&mut self) {
        let mut painters: Vec<(&'static str, crate::vector::Painter)> = Vec::new();
        for (n, layer) in &self.plugged {
            if let (Some(e), Some(p)) = (crate::registry::get(*n), layer.painter()) {
                painters.push((e.key(), p));
            }
        }
        for (id, c) in [("insolation", &self.insolation), ("shadows", &self.shadows)] {
            if let Some(l) = c.as_ref().and_then(|c| c.layer.as_ref()) {
                painters.push((id, crate::pointcalc::painter(l)));
            }
        }
        if let Some((r, radius)) = self.relief_calc.as_ref().and_then(crate::reliefcalc::ReliefCalc::layer) {
            painters.push(("relief", crate::reliefcalc::painter(r, radius)));
        }
        painters.extend(self.hydro.painters());
        if let Some(l) = self.aef_change.as_ref().and_then(|c| c.layer.clone()) {
            painters.push(("aef_change", crate::aefcalc::painter(l)));
        }
        // Общий слой вида: результат одного окна, `SiteLayer` модуля.
        // Слой, сетка которого помещается в картинку, идёт картинкой поверх
        // картинок меток, как предпросмотр анализа над слоями проекта
        // модуля 1.30.
        let mut preview = None;
        for tool in self.site_tools() {
            match tool.layer().and_then(|l| crate::aefclass::preview_overlay(&l)) {
                Some(overlay) => preview = Some(overlay),
                None => painters.extend(tool.painter().map(|p| (tool.id(), p))),
            }
        }
        self.vector.set_painters(painters);
        let key = |o: &Option<scene::kml::Overlay>| o.as_ref().map(crate::pictures::key_of);
        if key(&preview) != key(&self.preview) {
            self.preview = preview;
            self.preview_dirty = true;
            self.pictures_changed();
        }
    }

    /// Картинки наложений: «Мои метки», слои-картинки «Моих слоёв»
    /// и предпросмотр окна анализа.
    pub(super) fn picture_overlays(&self) -> Vec<&scene::kml::Overlay> {
        let mut list = self.marks.all_overlays();
        list.extend(self.data_layers.pictures());
        list.extend(self.preview.as_ref());
        list
    }

    /// Набор картинок сменился: картинки удалённых забываются.
    pub(super) fn pictures_changed(&mut self) {
        let list: Vec<scene::kml::Overlay> = self.picture_overlays().into_iter().cloned().collect();
        self.pictures.set(&list.iter().collect::<Vec<_>>());
        let keys: Vec<String> = self.pictures.states.keys().cloned().collect();
        self.screen_textures.retain(|k, _| keys.contains(k));
        if let Some(globe) = &mut self.globe {
            globe.keep_picture_images(&keys);
        }
    }

    /// Окна анализа на общем слое вида: видимость, земляные работы,
    /// классификация, похожие области, трассировка, минералы по спектру.
    pub(super) fn site_tools(&mut self) -> Vec<&mut dyn crate::analysis::SiteTool> {
        let mut out: Vec<&mut dyn crate::analysis::SiteTool> = Vec::new();
        if let Some(c) = &mut self.viewshed {
            out.push(c);
        }
        out.push(&mut self.earthwork);
        for c in [&mut self.aef_class, &mut self.similar, &mut self.tracing, &mut self.object_tasks, &mut self.recognize].into_iter().flatten() {
            out.push(c);
        }
        if let Some(c) = &mut self.spectral {
            out.push(c);
        }
        if let Some(c) = &mut self.emit {
            out.push(c);
        }
        out
    }

    /// Шкала классов хозяина общего слоя вида, `_show_legend` модуля.
    pub(super) fn site_scale(&mut self) -> Option<crate::legend::SiteScale> {
        let owner = self.site_layer.owner()?;
        let lang = self.lang;
        let tools = self.site_tools();
        let tool = tools.iter().find(|t| t.id() == owner && t.shown())?;
        crate::analysis::site_legend(lang, &**tool)
    }

    /// Крестик шкалы классов снимает результат хозяина, окно остаётся.
    pub(super) fn drop_site_owner(&mut self) {
        let Some(owner) = self.site_layer.owner() else {
            return;
        };
        for tool in self.site_tools() {
            if tool.id() == owner {
                tool.drop_result();
            }
        }
    }

    /// Хозяин общего слоя вида после кадра. true - слой сменился.
    pub(super) fn site_layer_settle(&mut self) -> bool {
        let mut layer = std::mem::take(&mut self.site_layer);
        let changed = layer.settle(&mut self.site_tools());
        self.site_layer = layer;
        changed
    }

    /// Окна точки меню на глобусе: номера листов и список своих сайтов.
    pub(super) fn point_windows(&mut self, ctx: &egui::Context) {
        if let Some(w) = &mut self.sheets_window
            && !w.show(ctx, self.lang).1
        {
            self.sheets_window = None;
        }
        if let Some(w) = &mut self.links_window {
            let (items, open) = w.show(ctx, self.lang);
            if let Some(items) = items {
                self.settings.links = items;
                self.settings_dirty = true;
            }
            if !open {
                self.links_window = None;
            }
        }
    }

    /// Окно «Новый источник тайлов»: название, адрес, уровни, подпись.
    /// «Добавить» ставит источник основой и в настройки.
    pub(super) fn own_window(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        let Some(d) = &mut self.own_dialog else {
            self.own_caps = None;
            return;
        };
        // Ответ GetCapabilities: слои службы, название и подпись - из него.
        if let Some(request) = &self.own_caps
            && let Some(answer) = request.take()
        {
            self.own_caps = None;
            let caps = answer.and_then(|b| scene::wms::parse(&b).map_err(|e| e.to_string()));
            if let Ok(c) = &caps {
                d.layer = 0;
                if d.name.trim().is_empty() {
                    d.name = c.layers.first().map_or_else(|| c.title.clone(), |l| l.title.clone());
                }
                if d.credit.trim().is_empty() {
                    d.credit = if c.attribution.is_empty() { c.title.clone() } else { c.attribution.clone() };
                }
            }
            d.caps = Some(caps);
        }
        let wms = scene::wms::is_wms(&d.url);
        let mut ask_caps = false;
        let mut open = true;
        let mut done = None;
        egui::Window::new(tr(lang, "New tile source")).open(&mut open).collapsible(false).resizable(false).default_width(420.0).show(ctx, |ui| {
            egui::Grid::new("own-source").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Address"));
                ui.add(egui::TextEdit::singleline(&mut d.url).desired_width(300.0).hint_text("https://example.org/{z}/{x}/{y}.png"))
                    .on_hover_text(tr(lang, "Address template of the tiles with the places {z}, {x} and {y}. {-y} - rows from bottom to top, as in TMS. Or the address of a WMS service - the window knows it by the words WMSServer, /wms or service=WMS and reads the list of its layers."));
                ui.end_row();
                if wms {
                    ui.label(tr(lang, "Layer"));
                    ui.horizontal(|ui| {
                        if let Some(Ok(caps)) = &d.caps {
                            let title = |l: &scene::wms::Layer| if l.group { format!("{} - {}", l.title, tr(lang, "all layers")) } else { l.title.clone() };
                            let current = caps.layers.get(d.layer).map(title).unwrap_or_default();
                            egui::ComboBox::from_id_salt("own-wms-layer").width(240.0).selected_text(current).show_ui(ui, |ui| {
                                for (n, l) in caps.layers.iter().enumerate() {
                                    if ui.selectable_label(d.layer == n, title(l)).clicked() {
                                        d.layer = n;
                                        d.name = l.title.clone();
                                    }
                                }
                            });
                        }
                        let waiting = self.own_caps.is_some();
                        if ui.add_enabled(!waiting, egui::Button::new(tr(lang, "Read layers"))).on_hover_text(tr(lang, "Read the list of layers of the service, the GetCapabilities request.")).clicked() {
                            ask_caps = true;
                        }
                        if waiting {
                            ui.spinner();
                        }
                    });
                    ui.end_row();
                }
                ui.label(tr(lang, "Name"));
                ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(300.0));
                ui.end_row();
                if !wms {
                    ui.label(tr(lang, "Levels up to"));
                    ui.add(egui::DragValue::new(&mut d.max_level).range(1..=crate::basemap::OWN_MAX_LEVEL));
                    ui.end_row();
                }
                ui.label(tr(lang, "Credit"));
                ui.add(egui::TextEdit::singleline(&mut d.credit).desired_width(300.0)).on_hover_text(tr(lang, "Credit of the source on the screen, as its terms of use require."));
                ui.end_row();
            });
            let error = if wms {
                match &d.caps {
                    Some(Ok(_)) if d.asked == d.url.trim() => None,
                    Some(Err(e)) if d.asked == d.url.trim() => {
                        ui.colored_label(crate::theme::pal().accent, crate::i18n::trf(lang, "The service did not give its layers: {error}", &[("error", e)]));
                        Some("")
                    }
                    _ => Some(""),
                }
            } else {
                crate::basemap::template_error(&d.url)
            };
            if let Some(e) = error.filter(|e| !d.url.trim().is_empty() && !e.is_empty()) {
                ui.colored_label(crate::theme::pal().accent, tr(lang, e));
            }
            if wms {
                ui.label(egui::RichText::new(tr(lang, "A WMS map lies over the base map of the globe, outside the area of the layer the base map is seen. The service draws each picture anew, so the map loads slower than tiles.")).weak());
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(error.is_none(), egui::Button::new(tr(lang, "Add"))).clicked() {
                    done = Some(true);
                }
                if ui.button(tr(lang, "Cancel")).clicked() {
                    done = Some(false);
                }
            });
        });
        if !open {
            done = Some(false);
        }
        if ask_caps {
            d.asked = d.url.trim().to_string();
            d.caps = None;
            let request = (self.loaders.request)(8 << 20);
            request.start(&scene::wms::capabilities_url(&scene::wms::base_url(&d.asked)));
            self.own_caps = Some(request);
        }
        // Служба WMS - карта поверх подложки, `wms.source` модуля.
        if done == Some(true)
            && wms
            && let Some(Ok(caps)) = &d.caps
            && let Some(layer) = caps.layers.get(d.layer)
        {
            let base = scene::wms::base_url(&d.asked);
            let template = scene::wms::getmap_template(&base, &caps.version, &layer.names, scene::wms::mercator_code(&layer.crs), scene::wms::pick_format(&caps.formats));
            let saved = crate::maps::WmsSaved { name: d.name.clone(), template, bbox: layer.bbox, credit: d.credit.clone(), max_level: None };
            let layer_box = layer.bbox;
            self.own_dialog = None;
            if let Some(n) = crate::maps::register_wms(&saved) {
                if !self.settings.wms.contains(&saved) {
                    self.settings.wms.push(saved);
                }
                self.extras.insert(crate::layers::Extra::Map(n));
                self.set_extra(crate::layers::Extra::Map(n), true);
                // Перелёт к району службы.
                if let Some(b) = layer_box.filter(|b| !scene::wms::is_world(Some(*b))) {
                    let (lat, lon) = ((b[1] + b[3]) / 2.0, (b[0] + b[2]) / 2.0);
                    let span = globe_core::ellipsoid::EARTH.a * (b[3] - b[1]).max((b[2] - b[0]) * lat.to_radians().cos()).to_radians();
                    let now = ctx.input(|i| i.time);
                    crate::input::fly(&mut self.nav, Pose::new(lat, lon, span.clamp(2000.0, 4.0e6) * 1.5, 0.0, 0.0), now);
                }
            }
            return;
        }
        match done {
            Some(true) => {
                let own = crate::basemap::Own { name: d.name.clone(), url: d.url.trim().to_string(), max_level: d.max_level, credit: d.credit.clone() };
                self.own_dialog = None;
                if let Some(b) = crate::basemap::register(&own) {
                    if !self.settings.own.contains(&own) {
                        self.settings.own.push(own);
                    }
                    self.settings_dirty = true;
                    self.set_basemap(b);
                }
            }
            Some(false) => self.own_dialog = None,
            None => {}
        }
    }

    /// Шаг разреза Земли: загрузки, грани и полосы плит, сектор
    /// в шейдере тайла.
    pub(super) fn step_cutaway(&mut self) {
        let Some(c) = &mut self.cutaway else {
            return;
        };
        let loaders = &self.loaders;
        let (meshes, saved) = c.step(self.nav.pose.distance, &|limit| (loaders.request)(limit));
        let wedge = c.uniform();
        self.labels.wedge = Some(c.wedge);
        if let Some(globe) = &mut self.globe {
            globe.set_wedge(Some(wedge));
            if let Some(m) = meshes {
                globe.set_subsurface("cutaway", m.faces.as_ref());
                globe.set_subsurface("slabs", m.slabs.as_ref());
            }
        }
        if let Some(bytes) = saved {
            self.store_crust(&bytes);
        }
    }

    /// Архив CRUST1.0 из кэша программы, как `_crust_path` модуля.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn crust_cache(&self) -> Option<Vec<u8>> {
        std::fs::read(self.demo_cache.as_ref()?.parent()?.join(data::crust::FILE)).ok()
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn crust_cache(&self) -> Option<Vec<u8>> {
        None
    }

    /// Пришедший архив CRUST1.0 - в кэш программы.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn store_crust(&self, bytes: &[u8]) {
        if let Some(dir) = self.demo_cache.as_ref().and_then(|d| d.parent()) {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(dir.join(data::crust::FILE), bytes);
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn store_crust(&self, _bytes: &[u8]) {}

    /// Шаг подземного режима: сетки заново, маска, прозрачность, пол
    /// камеры, окно.
    pub(super) fn step_subsurface(&mut self, ctx: &egui::Context, now: f64) {
        let lang = self.lang;
        let mut remove = false;
        if let Some(sub) = &mut self.subsurface {
            let (changed, gone) = if self.check.is_none() && (self.shot.is_none() || self.shot_ui) { sub.window(ctx, lang) } else { (false, false) };
            remove = gone;
            let _ = changed;
            let (version, scale) = {
                let s = self.heights.read();
                (s.version(), s.scale())
            };
            *self.floor.write().unwrap_or_else(std::sync::PoisonError::into_inner) = sub.floor();
            if let Some(globe) = &mut self.globe {
                globe.set_opacity(sub.options.opacity);
                if sub.due(now, version, scale)
                    && let Some(model) = &sub.model
                {
                    let heights = &self.heights;
                    let ground = |pts: &[(f64, f64)]| heights.read().heights_at(pts, false);
                    let (meshes, count) = crate::subsurface::build(model, scale, &ground, sub.cut_on());
                    for (name, mesh) in &meshes {
                        globe.set_subsurface(name, mesh.as_ref());
                    }
                    sub.built(lang, now, version, scale, count);
                }
                if sub.mask_dirty {
                    let made = sub.model.as_ref().and_then(|m| crate::subsurface::mask(m, sub.cut_on()));
                    globe.set_cut(made.as_ref().map(|(b, w, h, rgba)| (*b, *w, *h, rgba.as_slice())));
                    sub.mask_dirty = false;
                }
            }
        }
        if remove {
            self.subsurface = None;
            *self.floor.write().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
            if let Some(globe) = &mut self.globe {
                globe.clear_subsurface();
                globe.set_cut(None);
                globe.set_opacity(1.0);
            }
        }
    }
}
