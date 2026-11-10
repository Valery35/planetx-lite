// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Живые и сетевые данные: события, погода, течения, спутники, самолёты и суда, снимки, LiDAR, маршруты, рейс самосвалов.

use super::*;

impl LiteApp {
    /// Рейс самосвалов демо, как `_play_model_trucks` и `_haul_loop`
    /// модуля: шкала-момент в начале рейса, ×[`HAUL_RATE`], показ идёт.
    /// Момент у конца рейса или до его начала - снова начало. Пути
    /// сменились - демо снято, рейс больше не ведётся.
    pub(super) fn step_haul(&mut self, now: f64) {
        let span = crate::models::span(&self.movers);
        if self.haul_pending && let Some(span) = span {
            self.haul_pending = false;
            self.haul = Some(span);
            self.follow_truck(span.0);
            self.timebar.open = true;
            self.timebar.rate = Some(HAUL_RATE);
            self.timebar.set_point(true, Some(span.0));
            if !self.timebar.playing() {
                self.timebar.toggle(now);
            }
            return;
        }
        let Some(haul) = self.haul else {
            return;
        };
        if span.map(|s| s.0) != Some(haul.0) {
            self.haul = None;
            return;
        }
        let moment = self.timebar.range.1;
        if moment >= haul.1 - HAUL_END || moment < haul.0 {
            self.timebar.set_point(true, Some(haul.0));
            if !self.timebar.playing() {
                self.timebar.toggle(now);
            }
        }
    }

    /// На сенсорном экране камера идёт следом за самосвалом, который едет
    /// в начале рейса. Попасть пальцем в самосвал над карьером нельзя,
    /// без этого пункта «Камера следом» меню недоступен и самосвалов не
    /// видно. Долгое нажатие открывает меню и отпускает камеру.
    pub(super) fn follow_truck(&mut self, start: f64) {
        if !self.touch {
            return;
        }
        let store = self.heights.read();
        let at = |m: &crate::models::Mover, t: f64| m.place(Some((t, t)), &store).center;
        let moved: Vec<f64> = self.movers.iter().map(|m| (at(m, start + HAUL_LOOK) - at(m, start)).length()).collect();
        let index = busiest(&moved);
        let center = index.and_then(|i| self.movers.get(i)).map(|m| at(m, start));
        drop(store);
        let (Some(index), Some(center)) = (index, center) else {
            return;
        };
        let (lat, lon, _) = self.nav.camera.ellipsoid.ecef_to_geodetic(center);
        let p = &self.nav.pose;
        let pose = Pose::new(lat, lon, FOLLOW_REACH, p.heading, p.tilt);
        self.nav.stop();
        self.nav.show(pose);
        self.followed = Some(index);
    }

    /// Шаг снимков событий в каждом кадре, с окнами и без: каталоги,
    /// обход события, событие ключа `--event`.
    pub(super) fn step_events(&mut self, now: f64) {
        let lang = self.lang;
        let mut actions = self.events.step(lang, &*self.loaders.request);
        if let Some(key) = self.pending_event.clone()
            && !self.events.events.is_empty()
            && !self.events.busy()
        {
            self.pending_event = None;
            if !self.events.show_key(lang, &key, &mut actions) {
                self.open_message = Some(trf(lang, "There is no event {key} in the catalogs.", &[("key", &key)]));
            }
        }
        self.event_actions(actions, now);
    }

    /// Просьбы снимков событий: перелёт к событию, смена снимков стороны.
    pub(super) fn event_actions(&mut self, actions: Vec<crate::events::Action>, now: f64) {
        for action in actions {
            match action {
                crate::events::Action::Fly([w, s, e, n]) => {
                    let camera = &self.nav.camera;
                    let (lat, lon, distance) = globe_core::flight::fit_view(w, s, e, n, camera.fov_y, camera.aspect());
                    if lat.is_finite() && lon.is_finite() && distance.is_finite() {
                        crate::input::fly(&mut self.nav, Pose::new(lat, lon, distance, 0.0, 0.0), now);
                    }
                }
                crate::events::Action::Changed(side) => {
                    if let Some(globe) = &mut self.globe {
                        globe.clear_events(side);
                    }
                    if let Some(empty) = self.event_empty.get_mut(side) {
                        empty.clear();
                    }
                }
            }
        }
    }

    /// Шаг облаков лидара по сети: пришедшие облака - на глобус.
    pub(super) fn step_lidar(&mut self, now: f64) {
        let Some(lidar) = &mut self.lidar else {
            return;
        };
        let arrived = lidar.step(now, &*self.loaders.request);
        let done = !lidar.busy();
        let mut first = true;
        for a in arrived {
            match a {
                crate::lidarnet::Arrived::Cloud(tile, cloud) if !cloud.points.is_empty() => {
                    let name = format!("{} {}", tile.name, tile.date).trim().to_string();
                    let credit = data::lidarnet::credit(tile.source);
                    if !self.lidar_credits.contains(&credit) {
                        self.lidar_credits.push(credit);
                    }
                    // Камера летит к первому облаку.
                    self.show_cloud(&cloud, &name, first.then_some(now));
                    first = false;
                }
                crate::lidarnet::Arrived::Cloud(tile, _) => self.open_message = Some(trf(self.lang, "The point cloud did not open: {name}: {e}", &[("name", &tile.name), ("e", "0")])),
                crate::lidarnet::Arrived::Message(text) => self.open_message = Some(trf(self.lang, "Lidar point clouds: {error}", &[("error", &text)])),
                crate::lidarnet::Arrived::Nothing(errors) => {
                    self.open_message = Some(if errors.is_empty() { tr(self.lang, "There are no lidar tiles of IGN LiDAR HD and USGS 3DEP near the point.").to_string() } else { trf(self.lang, "Lidar point clouds: {error}", &[("error", &errors)]) });
                }
            }
        }
        if done {
            self.lidar = None;
        }
    }

    /// Облако на глобус, камера - к нему, если fly или поза не задана.
    pub(super) fn show_cloud(&mut self, cloud: &data::cloud::Cloud, name: &str, fly: Option<f64>) -> Option<Pose> {
        let e = Ellipsoid::wgs84();
        let xyz: Vec<DVec3> = cloud.points.iter().map(|p| e.geodetic_to_ecef(p[0], p[1], p[2])).collect();
        let (center, offsets) = scene::features::centered(&xyz);
        self.clouds.push(render::PointsMesh { center, offsets, colors: cloud.colors.clone(), size: CLOUD_POINT_SIZE, ..Default::default() });
        if let Some(globe) = &mut self.globe {
            globe.set_points(&self.clouds);
            self.dots.dirty = true;
        }
        // Охват облака - углы его рамки.
        let (mut lat0, mut lon0, mut lat1, mut lon1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in &cloud.points {
            (lat0, lon0, lat1, lon1) = (lat0.min(p[0]), lon0.min(p[1]), lat1.max(p[0]), lon1.max(p[1]));
        }
        let camera = &self.nav.camera;
        let pose = marks::fit_points(&[(lat0, lon0), (lat1, lon1)], camera.fov_y, camera.aspect()).map(|mut p| {
            // Облако видно сбоку: наклон, как у перелёта к пути.
            p.tilt = 55.0;
            p
        });
        if let Some(pose) = pose.clone().filter(|_| fly.is_some() || !self.keep_pose) {
            match fly {
                Some(now) => crate::input::fly(&mut self.nav, pose, now),
                None => self.nav.show(pose),
            }
        }
        self.open_message = Some(trf(
            self.lang,
            "Point cloud {name}: {shown} of {total} points",
            &[("name", name), ("shown", &cloud.points.len().to_string()), ("total", &cloud.total.to_string())],
        ));
        pose
    }

    /// Ползунок возраста палеогеографии внизу слева, `PaleoBar` модуля:
    /// прошлое слева, настоящее справа, показ к настоящему.
    pub(super) fn paleo_bar(&mut self, ctx: &egui::Context, now: f64) {
        let lang = self.lang;
        let Some(state) = &mut self.paleo else {
            return;
        };
        let ages = &data::paleo::AGES;
        let last = ages.len() - 1;
        let mut changed = false;
        if state.playing && now - state.last >= PALEO_STEP {
            state.last = now;
            if state.index == 0 {
                state.playing = false;
            } else {
                state.index -= 1;
                changed = true;
            }
        }
        if state.playing {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(PALEO_STEP / 3.0));
        }
        egui::Area::new(egui::Id::unique("paleo-bar")).order(egui::Order::Foreground).anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -70.0)).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Прошлое слева: ползунок идёт от последнего возраста.
                    let mut position = last - state.index.min(last);
                    let slider = ui.add(egui::Slider::new(&mut position, 0..=last).show_value(false)).on_hover_text(tr(lang, "Age in millions of years ago. The map of relief and depths for this age - PaleoDEM PALEOMAP, Scotese and Wright 2018."));
                    if slider.changed() {
                        state.index = last - position;
                        changed = true;
                    }
                    let play = if state.playing { "⏸" } else { "▶" };
                    if ui.button(play).on_hover_text(tr(lang, "Show from the chosen age to the present through all maps of the set.")).clicked() {
                        state.playing = !state.playing && state.index > 0;
                        state.last = now;
                    }
                    let age = ages[state.index.min(last)];
                    let text = if age == 0 {
                        tr(lang, "Present").to_string()
                    } else {
                        trf(lang, "{age} million years ago, {period}", &[("age", &age.to_string()), ("period", period_name(lang, data::paleo::period(f64::from(age))))])
                    };
                    ui.label(text);
                });
            });
        });
        if changed {
            let age = ages[state.index.min(last)];
            self.set_basemap(Basemap::Paleo(age));
        }
    }

    /// Окно «Снимки OpenAerialMap»: поиск, перелёт, снимок - строкой карт
    /// поверх подложки, `add_oam_image` модуля. Тот же снимок новой строки
    /// не даёт.
    pub(super) fn step_oam(&mut self, ctx: &egui::Context, now: f64) {
        let lang = self.lang;
        let view = self.view_box();
        let Some(o) = &mut self.oam else {
            return;
        };
        {
            let loaders = &self.loaders;
            let request = |limit: u64| (loaders.request)(limit);
            o.step(lang, ctx);
            o.show(ctx, lang, &request);
            if std::mem::take(&mut o.search) {
                o.show_view(view, &request);
            }
        }
        if o.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if let Some(b) = o.fly.take() {
            let distance = data::rumsey::fly_distance(&b, self.nav.camera.fov_y);
            crate::input::fly(&mut self.nav, Pose::new((b[1] + b[3]) / 2.0, (b[0] + b[2]) / 2.0, distance, 0.0, 0.0), now);
        }
        let add = o.add.take();
        if !o.open {
            self.oam = None;
        }
        let Some(image) = add else {
            return;
        };
        let name: String = [image.day.as_str(), image.title.as_str()].iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" ").chars().take(100).collect();
        let saved = crate::maps::WmsSaved { name: if name.is_empty() { image.url.clone() } else { name }, template: image.tiles.clone(), bbox: Some(image.bbox), credit: data::oam::rights(&image), max_level: Some(data::oam::max_level(&image)) };
        if let Some(n) = crate::maps::register_wms(&saved) {
            if !self.settings.wms.contains(&saved) {
                self.settings.wms.push(saved);
                self.settings_dirty = true;
            }
            self.extras.insert(crate::layers::Extra::Map(n));
            self.set_extra(crate::layers::Extra::Map(n), true);
        }
    }

    /// Окно архива Wayback: выбранный выпуск ложится строкой карт.
    pub(super) fn step_wayback(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        let (lat, lon, width) = (self.nav.pose.lat, self.nav.pose.lon, self.view_width());
        let Some(w) = &mut self.wayback else {
            return;
        };
        {
            let loaders = &self.loaders;
            let request = |limit: u64| (loaders.request)(limit);
            w.step(lang, &request);
            w.show(ctx, lang, &request);
            if std::mem::take(&mut w.search) {
                w.show_view(lat, lon, width, &request);
            }
        }
        if w.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let add = w.add.take();
        if !w.open {
            self.wayback = None;
        }
        let Some(release) = add else {
            return;
        };
        let saved = crate::maps::WmsSaved { name: format!("Wayback {}", release.day), template: data::wayback::tile_url(release.number), bbox: None, credit: data::wayback::ATTRIBUTION.to_string(), max_level: Some(data::wayback::MAX_LEVEL) };
        if let Some(n) = crate::maps::register_wms(&saved) {
            if !self.settings.wms.contains(&saved) {
                self.settings.wms.push(saved);
                self.settings_dirty = true;
            }
            self.extras.insert(crate::layers::Extra::Map(n));
            self.set_extra(crate::layers::Extra::Map(n), true);
        }
    }

    /// Окно «Карты Рамсея»: каталог, сборка, готовая карта - слоем-картинкой
    /// «Моих слоёв», папка «Карты Рамсея», как слой проекта модуля.
    pub(super) fn step_rumsey(&mut self, ctx: &egui::Context, now: f64) {
        let lang = self.lang;
        let Some(r) = &mut self.rumsey else {
            return;
        };
        {
            let loaders = &self.loaders;
            let request = |limit: u64| (loaders.request)(limit);
            r.step(lang, &request);
            r.show(ctx, lang, &request);
        }
        if r.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if let Some(b) = r.fly.take() {
            let distance = data::rumsey::fly_distance(&b, self.nav.camera.fov_y);
            crate::input::fly(&mut self.nav, Pose::new((b[1] + b[3]) / 2.0, (b[0] + b[2]) / 2.0, distance, 0.0, 0.0), now);
        }
        let built = r.built.take();
        if !r.open && !r.busy() {
            self.rumsey = None;
        }
        let Some(b) = built else {
            return;
        };
        let year = b.year.map_or(String::new(), |y| y.to_string());
        let name = if year.is_empty() { b.title.clone() } else { format!("{year} {}", b.title) };
        // Год карты идёт по шкале времени, как «Год карты» модуля.
        self.picture_to_layers(tr(lang, "Rumsey maps"), &name, b.png, b.bbox, (!year.is_empty()).then(|| (year.clone(), year)), String::new());
    }

    /// Окно «Космоснимки»: поиск, чтение каналов, готовый снимок -
    /// слоем-картинкой «Моих слоёв», папка «Космоснимки».
    pub(super) fn step_sentinel(&mut self, ctx: &egui::Context, now: f64) {
        let lang = self.lang;
        let Some(s) = &mut self.sentinel else {
            return;
        };
        {
            let loaders = &self.loaders;
            let request = |limit: u64| (loaders.request)(limit);
            s.step(lang, now, &request);
            s.show(ctx, lang, &request);
        }
        if s.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let built = s.built.take();
        let trend = s.series.as_mut().and_then(|x| x.built.take());
        if !s.shown() && !s.busy() {
            self.sentinel = None;
        }
        // Снимки и карты тренда - слоями-картинками «Моих слоёв», как слои
        // проекта модуля. Карта тренда - без времени: она о всём ряде.
        if let Some(b) = trend {
            self.picture_to_layers(tr(lang, "Trend maps"), &b.name, b.png, b.bbox, None, b.credit);
        }
        let Some(b) = built else {
            return;
        };
        self.picture_to_layers(tr(lang, "Satellite images"), &b.name, b.png, b.bbox, Some((b.day.clone(), b.day.clone())), b.credit);
    }

    /// Окно «Треки»: смена охвата времени треков - новый охват шкалы,
    /// кнопка «3D-модель…» - выбор файла модели.
    pub(super) fn step_tracks(&mut self, ctx: &egui::Context) {
        let before = self.tracks.span();
        self.tracks.show(ctx, self.lang);
        self.show_addins(ctx);
        if self.tracks.span() != before {
            self.time_extent();
        }
        if let Some(n) = self.tracks.pick_model.take() {
            self.pick_for(ctx, Pick::TrackModel(n));
        }
    }

    /// «Моё место»: кнопка у правого нижнего края вида, перелёт к первому
    /// месту, камера следом, окно с записью трека.
    pub(super) fn step_gps(&mut self, ctx: &egui::Context, now: f64) {
        if !crate::gps::Gps::available() || !self.shows_ui() {
            return;
        }
        let lang = self.lang;
        let mut toggle = false;
        egui::Area::new(egui::Id::unique("planetx-gps"))
            .order(egui::Order::Middle)
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -40.0))
            .show(ctx, |ui| {
                crate::theme::plate().inner_margin(egui::Margin::same(3)).show(ui, |ui| {
                    let text = egui::RichText::new("◎").size(22.0).color(if self.gps.on { egui::Color32::from_rgb(30, 140, 255) } else { ui.visuals().text_color() });
                    toggle = ui.add(egui::Button::new(text).frame(false)).on_hover_text(tr(lang, "My location: the location of this device on the globe, the camera follows, a track can be recorded.")).clicked();
                });
            });
        if toggle {
            if self.gps.on {
                self.gps.open = !self.gps.open;
            } else {
                self.gps.start(ctx);
            }
        }
        if let Some(fix) = self.gps.step() {
            let p = &self.nav.pose;
            let (distance, heading, tilt) = (p.distance, p.heading, p.tilt);
            if !self.gps.flown {
                self.gps.flown = true;
                crate::input::fly(&mut self.nav, Pose::new(fix.lat, fix.lon, crate::gps::FIX_DISTANCE, heading, tilt), now);
            } else if self.gps.follow {
                self.nav.show(Pose::new(fix.lat, fix.lon, distance, heading, tilt));
            }
        }
        if self.gps.window(ctx, lang) == Some(crate::gps::Action::Save)
            && let Some(place) = self.gps.track_place(lang)
        {
            let name = place.name.clone();
            let folder = self.tree_state.selected.clone().unwrap_or_default();
            let at = self.marks.insert(&folder, scene::kml::Item::Place(place), marks::root_name(lang));
            self.tree_state.selected = Some(at);
            self.marks_changed();
            self.gps.track.clear();
            self.gps.dirty = true;
            self.open_message = Some(trf(lang, "The track is in My Places: {name}.", &[("name", &name)]));
        }
    }

    /// Шаг кадра включённых слоёв реестра [`crate::registry`]: загрузка,
    /// движение, сообщение, картинки тайлов.
    /// Окно «Объекты» открыто щелчком: слои реестра дозапрашивают сведения.
    pub(super) fn objects_clicked(&mut self) {
        let Some(objects) = &self.objects else {
            return;
        };
        let loaders = &self.loaders;
        let request = |limit: u64| (loaders.request)(limit);
        for layer in self.plugged.values_mut() {
            layer.clicked(self.lang, &objects.groups, &request);
        }
    }

    pub(super) fn step_plugged(&mut self, ctx: &egui::Context, now: f64) {
        if self.plugged.is_empty() {
            return;
        }
        let moment = match self.timebar.window() {
            Some((_, hi)) if hi.is_finite() => Some(hi),
            _ => None,
        };
        let altitude = self.nav.camera.ellipsoid.ecef_to_geodetic(self.nav.camera.eye).2;
        let view = (self.nav.pose.lat, self.nav.pose.lon, altitude, self.view_width());
        let loaders = &self.loaders;
        let request = |limit: u64| (loaders.request)(limit);
        let heights = &self.heights;
        let ground = |lat: f64, lon: f64| heights.read().height_at(lat, lon);
        let cx = crate::registry::Ctx { lang: self.lang, now, moment, view, request: &request, camera: &self.nav.camera, ground: &ground };
        let mut painters = false;
        let mut message = None;
        for layer in self.plugged.values_mut() {
            let stepped = layer.step(&cx);
            painters |= stepped.painters;
            message = stepped.message.or(message);
            if let Some(ms) = stepped.tick {
                ctx.request_repaint_after(std::time::Duration::from_millis(ms));
            }
            // Пришедшие сведения - в открытое окно «Объекты».
            if let Some(objects) = &mut self.objects {
                layer.enrich(self.lang, &mut objects.groups);
            }
        }
        if message.is_some() {
            self.open_message = message;
        }
        if painters {
            self.refresh_painters();
        }
    }

    /// Пришли сведения об участках: группа окна «Объекты» и подсветка
    /// контуров, `_parcels_arrived` модуля.
    pub(super) fn step_parcels(&mut self) {
        let Some(((lat, lon), ask)) = &self.parcel_ask else {
            return;
        };
        let Some(answer) = ask.take() else {
            return;
        };
        let (lat, lon) = (*lat, *lon);
        self.parcel_ask = None;
        let lang = self.lang;
        let result = answer.and_then(|d| data::cadastre::parse_info(&d));
        let features: Vec<crate::identify::Feature> = match &result {
            Err(e) => vec![(trf(lang, "not received: {error}", &[("error", e)]), Vec::new())],
            Ok(list) if list.is_empty() => vec![(tr(lang, "No parcels under the point").to_string(), Vec::new())],
            Ok(list) => list
                .iter()
                .map(|p| {
                    let mut values: Vec<(String, String)> = p
                        .values
                        .iter()
                        .map(|(k, v)| {
                            let unit = data::cadastre::FIELDS.iter().find(|(f, _)| f == k).map_or("", |(_, u)| *u);
                            let text = if unit.is_empty() { v.clone() } else { format!("{v} {unit}") };
                            (tr(lang, crate::identify::parcel_label(k)).to_string(), text)
                        })
                        .collect();
                    values.push((tr(lang, "NSPD map").to_string(), data::cadastre::map_link(lat, lon)));
                    (trf(lang, "Land parcel {number}", &[("number", &p.number)]), values)
                })
                .collect(),
        };
        self.parcels = result.unwrap_or_default();
        if let Some(objects) = &mut self.objects
            && objects.at == (lat, lon)
        {
            objects.groups.insert(0, (tr(lang, "Rosreestr cadastre").to_string(), features));
        }
    }

    /// Шаг загрузчиков Overpass: тайлы у точки взгляда, пришедшие
    /// объекты - в слои точек и линий.
    pub(super) fn step_overpass(&mut self, now: f64) {
        // Загрузчик идёт за включённой строкой: строки включаются и из
        // настроек при запуске, мимо щелчка по строке.
        let keys = self.dots.keys();
        let mines_on = keys.contains(&crate::dots::Layer::OsmMines);
        let infra_on = keys.contains(&crate::dots::Layer::InfraOsm);
        if mines_on != self.osm_mines.is_some() {
            self.osm_mines = mines_on.then(|| crate::overpass::Overpass::new(data::mining::query, data::mining::parse));
        }
        if infra_on != self.osm_infra.is_some() {
            self.osm_infra = infra_on.then(|| crate::overpass::Overpass::new(data::infra::query, data::infra::parse));
        }
        if self.osm_mines.is_none() && self.osm_infra.is_none() {
            return;
        }
        let altitude = self.nav.camera.ellipsoid.ecef_to_geodetic(self.nav.camera.eye).2;
        let view = (self.nav.pose.lat, self.nav.pose.lon, altitude);
        let loaders = &self.loaders;
        let request = |limit: u64| (loaders.request)(limit);
        let mut failed = false;
        if let Some(o) = &mut self.osm_mines {
            o.step(now, view, &mut self.overpass_mirrors, &request);
            failed |= o.failed;
            if std::mem::take(&mut o.dirty) {
                self.dots.set_data(crate::dots::Layer::OsmMines, crate::dots::Data::Mines(data::mining::to_deposits(&o.all())));
            }
        }
        if let Some(o) = &mut self.osm_infra {
            o.step(now, view, &mut self.overpass_mirrors, &request);
            failed |= o.failed;
            if std::mem::take(&mut o.dirty) {
                self.dots.set_data(crate::dots::Layer::InfraOsm, crate::dots::infra_data(o.all()));
            }
        }
        if failed {
            self.open_message = Some(tr(self.lang, "The OpenStreetMap server did not answer, the request will repeat by itself.").to_string());
        }
    }

    /// Папка файлов групп спутников, у страницы None.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn satellite_dir(&self) -> Option<std::path::PathBuf> {
        Some(self.demo_cache.as_ref()?.parent()?.join("satellites"))
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn satellite_dir(&self) -> Option<std::path::PathBuf> {
        None
    }

    /// Момент спутников: правый бегунок открытой шкалы времени, иначе
    /// часы, `moment` модуля.
    pub(super) fn satellite_moment(&self) -> f64 {
        match self.timebar.window() {
            Some((_, hi)) if hi.is_finite() => hi,
            _ => web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64()),
        }
    }

    /// Шаг спутников: загрузка групп, точки по такту, камера следом.
    pub(super) fn step_satellites(&mut self, now: f64) {
        let moment = self.satellite_moment();
        let lang = self.lang;
        let Some(sats) = self.plugged.values_mut().find_map(|l| l.satellites_mut()) else {
            return;
        };
        {
            let loaders = &self.loaders;
            sats.step(lang, moment, &|limit: u64| (loaders.request)(limit));
        }
        let tick = if sats.follow { crate::satellites::FOLLOW_TICK } else { crate::satellites::TICK };
        if now - sats.shown_at < tick && !self.sat_points.is_empty() {
            return;
        }
        sats.shown_at = now;
        let eye = self.nav.camera.eye;
        let shown = sats.points(moment, eye.to_array());
        let colors: Vec<[u8; 3]> = shown.iter().map(|(i, _)| sats.swarm.colors.get(*i).map_or([255; 3], |c| [(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8])).collect();
        let sizes: Vec<f32> = shown.iter().map(|(i, _)| sats.swarm.sizes.get(*i).copied().unwrap_or(data::satellites::SIZE)).collect();
        self.sat_shown = shown.iter().map(|(i, p)| (*i, DVec3::from_array(*p))).collect();
        self.sat_points.clear();
        if !self.sat_shown.is_empty() {
            let center = self.sat_shown.iter().fold(DVec3::ZERO, |a, (_, p)| a + *p) / self.sat_shown.len() as f64;
            self.sat_points.push(render::PointsMesh { side: render::Side::Both,
                center,
                offsets: self.sat_shown.iter().map(|(_, p)| (*p - center).as_vec3().to_array()).collect(),
                colors,
                size: data::satellites::SIZE,
                sizes,
                dot: Some(render::Dot { alpha: 1.0, lift: 0.0, space: true }),
            });
        }
        self.dots.dirty = true;
        // Камера следом: точка под спутником, азимут по курсу, расстояние
        // и наклон остаются у пользователя.
        if sats.follow
            && let Some(index) = sats.selected.and_then(|n| sats.swarm.index_of(n))
            && let Some((point, velocity)) = sats.swarm.motion(index, moment)
        {
            let (lat, lon, _) = self.nav.camera.ellipsoid.ecef_to_geodetic(DVec3::from_array(point));
            let heading = data::satellites::heading(point, velocity);
            let (distance, tilt) = (self.nav.pose.distance, self.nav.pose.tilt);
            self.nav.show(Pose::new(lat, lon, distance, heading, tilt));
        }
    }

    /// Спутник под пикселем кадра: номер NORAD.
    pub(super) fn satellite_under(&self, px: f64, py: f64, ppp: f64) -> Option<i64> {
        let sats = self.plugged.values().find_map(|l| l.satellites())?;
        let radius = crate::satellites::PICK_PIXELS * ppp;
        self.sat_shown
            .iter()
            .filter_map(|(i, p)| {
                let (q, front) = self.nav.camera.project(*p);
                let gap = (q.x - px).hypot(q.y - py);
                (front && gap <= radius).then_some((gap, *i))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .and_then(|(_, i)| sats.swarm.numbers.get(i).copied())
    }


    /// Шаг полёта, `FlightSim._tick` модуля: клавиши, физика, камера,
    /// модель. Касание земли - надпись, через END_SHOW секунд режим
    /// снимается.
    pub(super) fn step_flight(&mut self, ctx: &egui::Context, now: f64) {
        let Some(flight) = &mut self.flight else {
            return;
        };
        let (escape, view, hud) = ctx.input(|i| (i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::V), i.key_pressed(egui::Key::H)));
        if hud {
            flight.hud = !flight.hud;
        }
        if view {
            let tilt = self.nav.pose.tilt;
            let back = !flight.cockpit;
            flight.set_cockpit(back, tilt);
            if !back {
                let p = &flight.plane;
                self.nav.air = false;
                self.nav.show(Pose::new(p.lat, p.lon, crate::flight::CHASE_REACH, p.heading, flight.chase_tilt()));
            }
        }
        let store = self.heights.read();
        let scale = store.scale();
        let true_ground = |lat: f64, lon: f64| if scale != 0.0 { (store.height_at(lat, lon) / scale).max(0.0) } else { 0.0 };
        let ground = true_ground(flight.plane.lat, flight.plane.lon);
        let going = !escape && flight.step(now, crate::flight::controls(ctx), ground);
        let screen_ground = |lat: f64, lon: f64| store.height_at(lat, lon).max(0.0);
        if going {
            let alt = flight.plane.alt * scale;
            match flight.camera(alt, &screen_ground, self.nav.pose.tilt) {
                crate::flight::Camera::Chase(la, lo, distance, look, tilt) => self.nav.show(Pose::new(la, lo, distance, look, tilt)),
                crate::flight::Camera::Cockpit { lat, lon, h, distance, heading, tilt, roll } => {
                    self.nav.stop();
                    let mut pose = Pose::new(lat, lon, distance, heading, tilt).with_h(h).with_terrain(self.nav.terrain.clone());
                    pose.roll = roll;
                    self.nav.set_pose(pose);
                    self.nav.air = true;
                }
            }
            if let Some(globe) = &mut self.globe {
                if flight.fresh {
                    flight.fresh = false;
                    globe.set_flight_model(Some(&flight.mesh()));
                }
                let center = self.nav.camera.ellipsoid.geodetic_to_ecef(flight.plane.lat, flight.plane.lon, alt);
                globe.place_flight(render::ModelPlace { center, orient: flight.orient(), sun: crate::models::sun_at(flight.plane.lat, flight.plane.lon), shown: !flight.cockpit });
            }
            ctx.request_repaint();
        }
        drop(store);
        if !going {
            self.flight = None;
            self.nav.air = false;
            if let Some(globe) = &mut self.globe {
                globe.set_flight_model(None);
            }
        }
    }

    /// Шаг живых строк, `LiveManager` модуля: опрос служб, точки
    /// и подписи издалека, модели вблизи, камера следом.
    pub(super) fn step_live(&mut self, now: f64) {
        if !self.plugged.values().any(|l| l.live().is_some()) {
            self.live_objects.clear();
            return;
        }
        let pose = self.nav.pose.clone();
        let view = (pose.lat, pose.lon, self.view_width());
        let mut rebuild = false;
        let mut all: Vec<(crate::live::Kind, crate::live::Object)> = Vec::new();
        {
            let loaders = &self.loaders;
            let request = |limit: u64| (loaders.request)(limit);
            for live in self.plugged.values_mut().filter_map(|l| l.live_mut()) {
                live.step(now, view, &request);
                rebuild |= live.dirty;
                let kind = live.kind;
                all.extend(live.objects(now).into_iter().map(|o| (kind, o)));
            }
        }
        let store = self.heights.read();
        let scale = store.scale();
        let ground = |lat: f64, lon: f64| store.height_at(lat, lon);
        let e = self.nav.camera.ellipsoid;
        let level = |o: &crate::live::Object| if o.alt.is_finite() { o.alt * scale } else { ground(o.lat, o.lon) };
        // Камера следом: объект в середине вида, азимут по курсу.
        if let Some((kind, key)) = self.live_follow.clone() {
            match all.iter().find(|(k, o)| *k == kind && o.key == key) {
                Some((_, o)) => {
                    let heading = if o.heading.is_finite() { o.heading } else { pose.heading };
                    let reach = self.plugged.values().filter_map(|l| l.live()).find(|l| l.kind == kind).map_or(crate::live::START_REACH, |l| l.reach);
                    let (la, lo, look, distance) = data::movers::chase_on(o.lat, o.lon, heading, level(o), &ground, reach, pose.tilt);
                    self.nav.show(Pose::new(la, lo, distance, look, pose.tilt));
                }
                None => self.live_follow = None,
            }
        }
        // Модели вблизи и объект под камерой следом.
        let mut keys: Vec<(crate::live::Kind, String)> = Vec::new();
        for live in self.plugged.values().filter_map(|l| l.live()) {
            let objects: Vec<crate::live::Object> = all.iter().filter(|(k, _)| *k == live.kind).map(|(_, o)| o.clone()).collect();
            let followed = self.live_follow.as_ref().filter(|(k, _)| *k == live.kind).map(|(_, key)| key.as_str());
            keys.extend(crate::live::model_keys(live.kind, &objects, view, followed).into_iter().map(|k| (live.kind, k)));
        }
        if let Some(globe) = &mut self.globe {
            if keys != self.live_keys {
                let mut meshes = Vec::new();
                for (kind, key) in &keys {
                    if let (Some(live), Some((_, o))) = (self.plugged.values_mut().filter_map(|l| l.live_mut()).find(|l| l.kind == *kind), all.iter().find(|(k, o)| k == kind && o.key == *key)) {
                        meshes.extend(live.meshes(&[o]));
                    }
                }
                globe.set_live_models(&meshes);
                self.live_keys = keys.clone();
            }
            for (i, (kind, key)) in keys.iter().enumerate() {
                if let Some((_, o)) = all.iter().find(|(k, o)| k == kind && o.key == *key) {
                    let center = e.geodetic_to_ecef(o.lat, o.lon, level(o));
                    globe.place_live(i, render::ModelPlace { center, orient: crate::live::orient(o), sun: crate::models::sun_at(o.lat, o.lon), shown: true });
                }
            }
        }
        // Точки и подписи издалека: не те, что видны моделью крупно.
        if rebuild {
            let eye = self.nav.camera.eye;
            let mut points: Vec<DVec3> = Vec::new();
            let mut colors = Vec::new();
            let mut labels = Vec::new();
            for live in self.plugged.values_mut().filter_map(|l| l.live_mut()) {
                let objects: Vec<crate::live::Object> = all.iter().filter(|(k, _)| *k == live.kind).map(|(_, o)| o.clone()).collect();
                let followed = self.live_follow.as_ref().filter(|(k, _)| *k == live.kind).map(|(_, key)| key.clone());
                let near = crate::live::near(live.kind, view.2);
                let skip = |o: &crate::live::Object| {
                    followed.as_deref() == Some(o.key.as_str()) || (near && (e.geodetic_to_ecef(o.lat, o.lon, level(o)) - eye).length() <= crate::live::SEEN * o.size.1)
                };
                for (n, o) in crate::live::nearest(&objects, view, &skip).into_iter().enumerate() {
                    let p = e.geodetic_to_ecef(o.lat, o.lon, level(o));
                    points.push(p);
                    colors.push(crate::live::rgba(o.color));
                    if n < LIVE_LABELS {
                        labels.push((o.label.clone(), p));
                    }
                }
                live.built(now);
            }
            self.live_points.clear();
            if !points.is_empty() {
                let center = points.iter().fold(DVec3::ZERO, |a, p| a + *p) / points.len() as f64;
                self.live_points.push(render::PointsMesh { side: render::Side::Both,
                    center,
                    offsets: points.iter().map(|p| (*p - center).as_vec3().to_array()).collect(),
                    colors,
                    size: crate::live::POINT_SIZE,
                    sizes: Vec::new(),
                    dot: Some(render::Dot { alpha: 1.0, lift: crate::dots::LIFT, space: false }),
                });
            }
            self.live_labels = labels;
            self.dots.dirty = true;
        }
        drop(store);
        self.live_objects = all;
    }

    /// Шаг маршрута: готовый путь ложится линией в выделенную папку
    /// «Моих меток», окно летит к нему, как `_done` модуля.
    pub(super) fn step_route(&mut self, now: f64) {
        let loaders = &self.loaders;
        let request = || (loaders.request)(crate::route::SERVICE_BYTES);
        let Some((route, summary)) = self.route.step(self.lang, now, &request) else {
            return;
        };
        let lang = self.lang;
        let name = scene::placetree::numbered_name(tr(lang, "Route"), &self.marks.names());
        let mut place = scene::kml::Place::new(&name, scene::features::Kind::Line, route.points);
        place.color = crate::route::ROUTE_COLOR;
        place.width = crate::route::ROUTE_WIDTH;
        place.description = summary.clone();
        let folder = self.tree_state.selected.clone().unwrap_or_default();
        let at = self.marks.insert(&folder, scene::kml::Item::Place(place), marks::root_name(lang));
        self.tree_state.selected = Some(at.clone());
        self.marks_changed();
        self.queued.push(crate::marks::Action::Fly(at));
        self.route.show(trf(lang, "{name}: {summary}", &[("name", &name), ("summary", &summary)]), true);
    }

    /// Шаг сетевых ссылок: пришедший документ заменяет содержимое папки,
    /// как `NetLinks._arrived` модуля.
    pub(super) fn step_links(&mut self, now: f64) {
        let loaders = &self.loaders;
        let request = || (loaders.request)(crate::netlinks::MAX_BYTES);
        let read = |path: &str| -> Result<Vec<u8>, String> { read_local(path).ok_or_else(|| format!("{path}?")) };
        let arrived = self.netlinks.step(self.marks.tree.as_ref(), now, &request, &read, &file_stamp_of);
        let mut changed = false;
        for (path, link, got) in arrived {
            let Some(folder) = self.marks.folder(&path) else {
                continue;
            };
            if folder.link.trim() != link {
                continue;
            }
            let name = folder.name.clone();
            match got {
                Ok(tree) => {
                    self.netlinks.errors.remove(&path);
                    changed |= self.marks.edit_folder(&path, |f| f.children = tree.children);
                }
                Err(e) => {
                    self.open_message = Some(crate::netlinks::error_text(self.lang, &name, &e));
                    self.netlinks.errors.insert(path, e);
                }
            }
        }
        if changed {
            self.marks_changed();
        }
    }
}
