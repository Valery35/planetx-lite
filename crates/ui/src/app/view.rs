// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Снимок и выгрузка вида, запись облёта и фильма, снимок окна по ключу --shot.

use super::*;

impl LiteApp {
    /// Кнопка записи тура, как `_record_toggled` модуля: начать или
    /// закончить. Позы прореживаются (`scene::tour::thin`), тур ложится
    /// в выделенную папку «Моих меток». Камера не двигалась - тура нет.
    pub(super) fn toggle_record(&mut self, now: f64) {
        let Some((_, samples)) = self.recording.take() else {
            self.recording = Some((now, Vec::new()));
            self.record_sample(now);
            return;
        };
        let samples = scene::tour::thin(&samples);
        if samples.len() < 2 || samples.last().is_none_or(|s| s[0] <= 0.0) {
            self.open_message = Some(tr(self.lang, "The tour is not recorded, the camera did not move.").to_string());
            return;
        }
        // Название тура спрашивает окно «Сохранить тур», как у модуля.
        let name = scene::placetree::numbered_name(tr(self.lang, "Tour"), &self.marks.names());
        let points = samples.iter().map(|s| (s[1], s[2])).collect();
        let mut place = scene::kml::Place::new(&name, scene::features::Kind::Line, points);
        place.tour = Some(samples);
        self.name_dialog = Some((NameFor::Tour(Box::new(place)), name));
        self.open_message = None;
    }

    /// Поза камеры в запись тура не чаще [`RECORD_PERIOD`].
    pub(super) fn record_sample(&mut self, now: f64) {
        let Some((start, samples)) = &mut self.recording else {
            return;
        };
        let t = now - *start;
        if samples.last().is_some_and(|s| t - s[0] < RECORD_PERIOD) {
            return;
        }
        let p = &self.nav.pose;
        samples.push([t, p.lat, p.lon, p.distance, p.heading, p.tilt]);
        self.open_message = Some(trf(
            self.lang,
            "Recording a tour {clock}. Another click on the record button ends it.",
            &[("clock", &scene::tour::clock(t))],
        ));
    }

    /// Пришёл снимок вида - файлом PNG или в буфер обмена.
    /// Снимок вида: в кадре со скрытыми окнами - запрос картинки кадра,
    /// в следующих - запись готовой картинки.
    pub(super) fn request_snapshot(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        if let Some(Snapshot::Hide(to)) = self.snapshot {
            // Окна скрыты в этом кадре: снимок - глобус с подписью
            // источников.
            let cell = Arc::new(std::sync::Mutex::new(None));
            let sink = cell.clone();
            let ppp_points = ctx.pixels_per_point();
            ctx.request_screenshot(move |image| {
                if let Ok(mut slot) = sink.lock() {
                    *slot = Some(crop(&image, rect, ppp_points));
                }
            });
            self.snapshot = Some(Snapshot::Wait(to, cell));
        } else {
            self.save_snapshot(ctx);
        }
    }

    /// Куски сцены вида, `collect` модуля.
    pub(super) fn view_items(&self) -> Vec<scene::sceneexport::Item> {
        let mut items = Vec::new();
        let store = self.heights.read();
        if let Some(b) = &self.buildings {
            let heights = |lats: &[f64], lons: &[f64]| -> Vec<f64> {
                let points: Vec<(f64, f64)> = lats.iter().copied().zip(lons.iter().copied()).collect();
                store.heights_at(&points, true)
            };
            items.extend(b.meshes(&heights).iter().map(crate::viewexport::building_item));
        }
        if let Some(sub) = &self.subsurface
            && let Some(model) = &sub.model
        {
            let ground = |pts: &[(f64, f64)]| store.heights_at(pts, false);
            let (meshes, _) = crate::subsurface::build(model, store.scale(), &ground, sub.cut_on());
            items.extend(meshes.iter().filter_map(|(name, mesh)| mesh.as_ref().map(|m| crate::viewexport::sub_item(name, m))).filter(|i| !i.positions.is_empty()));
        }
        for (i, mesh) in self.export_models.iter().enumerate() {
            if !self.movers.iter().any(|m| m.index == i) {
                items.extend(crate::viewexport::model_items(mesh, &format!("model {}", i + 1)));
            }
        }
        for (i, mesh) in self.tilesets.meshes().iter().enumerate() {
            items.extend(crate::viewexport::model_items(mesh, &format!("3D Tiles {}", i + 1)));
        }
        items
    }

    /// Вид файлом: один GLB или набор 3D Tiles архивом, `export_scene`
    /// модуля.
    pub(super) fn export_view(&mut self, tiles: bool) {
        let items = self.view_items();
        if items.is_empty() {
            self.open_message = Some(crate::viewexport::nothing(self.lang).to_string());
            return;
        }
        let saved = if tiles {
            let Some(bytes) = scene::sceneexport::tiles_zip(&items) else {
                self.open_message = Some(trf(self.lang, "Not saved: {e}", &[("e", "ZIP")]));
                return;
            };
            crate::export::save_bytes("PlanetX-3dtiles.zip", ("3D Tiles ZIP", "zip"), &bytes)
        } else {
            let p = &self.nav.pose;
            let bytes = scene::sceneexport::scene_glb(&self.nav.camera.ellipsoid, &items, p.lat, p.lon, 0.0);
            crate::export::save_bytes("PlanetX.glb", ("glTF", "glb"), &bytes)
        };
        self.open_message = match saved {
            Ok(Some(_)) => Some(crate::viewexport::report(self.lang, items.len(), self.heights.read().scale())),
            Ok(None) => None,
            Err(e) => Some(trf(self.lang, "Not saved: {e}", &[("e", &e)])),
        };
    }

    pub(super) fn save_snapshot(&mut self, ctx: &egui::Context) {
        let Some(Snapshot::Wait(to, cell)) = &self.snapshot else {
            return;
        };
        let to = *to;
        let Some(result) = cell.lock().ok().and_then(|mut slot| slot.take()) else {
            return;
        };
        self.snapshot = None;
        if to == SnapshotTo::Frame {
            self.film_frame(result);
            return;
        }
        if to == SnapshotTo::Mcp {
            let shot = result.ok().and_then(|image| {
                let png = to_png(&image).ok()?;
                // Большой снимок - уменьшенный, ответ агенту не раздувается.
                let img = image::load_from_memory(&png).ok()?;
                let img = if img.width().max(img.height()) > MCP_SHOT_SIDE { img.resize(MCP_SHOT_SIDE, MCP_SHOT_SIDE, image::imageops::FilterType::Triangle) } else { img };
                // JPEG: снимок снимков земли в PNG весит мегабайты.
                let mut out = Vec::new();
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, MCP_SHOT_QUALITY).encode_image(&img.to_rgb8()).ok()?;
                let p = &self.nav.pose;
                Some((out, format!("Точка взгляда {:.4}, {:.4}, расстояние {:.1} км, азимут {:.0}°, наклон {:.0}°.", p.lat, p.lon, p.distance / 1000.0, p.heading, p.tilt)))
            });
            #[cfg(not(target_arch = "wasm32"))]
            if let Some((message, reply, _, _)) = self.mcp_shot.take() {
                let server = assistant::mcp::Server::new(&crate::basemap::READY.map(|r| r.key));
                let _ = reply.send(server.handle(&mut ShotHost(shot), &message));
            }
            #[cfg(target_arch = "wasm32")]
            let _ = shot;
            return;
        }
        self.open_message = match (to, result) {
            (SnapshotTo::Clipboard, Ok(image)) => {
                ctx.copy_image(image);
                Some(tr(self.lang, "The view is copied to the clipboard.").to_string())
            }
            (SnapshotTo::File, Ok(image)) => match to_png(&image).and_then(|png| crate::export::save_bytes("planetx.png", ("PNG", "png"), &png)) {
                Ok(Some(path)) => Some(trf(self.lang, "Saved: {to}", &[("to", &path)])),
                Ok(None) => None,
                Err(e) => Some(trf(self.lang, "Not saved: {e}", &[("e", &e)])),
            },
            (_, Err(e)) => Some(trf(self.lang, "Not saved: {e}", &[("e", &e)])),
            (SnapshotTo::Mcp | SnapshotTo::Frame, Ok(_)) => None,
        };
    }

    /// Начать запись тура кадрами PNG в папку из окна выбора, `start`
    /// записи модуля. Запись идёт с первой остановки. Время данных идёт
    /// по шкале времени, если у остановок нет своих дат.
    pub(super) fn start_film(&mut self) {
        let Some(folder) = crate::export::pick_folder() else {
            return;
        };
        if crate::film::taken(&folder) {
            self.open_message = Some(tr(self.lang, "The folder already holds tour frames. Choose another folder.").to_string());
            return;
        }
        let Some(player) = &mut self.player else {
            return;
        };
        let bar = &self.timebar;
        let span = if bar.point { bar.extent } else { Some(bar.range) };
        let data = (bar.open && player.stops.iter().all(|s| s.time.is_none())).then_some(span).flatten();
        let Some(line) = scene::timeline::Timeline::new(player.stops.clone(), player.pause, data, scene::timeline::FPS, self.nav.camera.fov_y) else {
            return;
        };
        player.film(line.tour.clone());
        self.tour_index = None;
        self.timebar.stop();
        self.film = Some(crate::film::Film::new(line, folder, self.timebar.range));
    }

    /// Шаг записи: камера в позе кадра, вид догружается, снимок.
    /// Тур закрыли - запись прервана.
    pub(super) fn step_film(&mut self, ctx: &egui::Context, now: f64) {
        use crate::film::Stage;
        let Some(film) = &mut self.film else {
            return;
        };
        ctx.request_repaint();
        let Some(player) = &mut self.player else {
            let text = film.stopped(self.lang);
            self.end_film(text);
            return;
        };
        match film.stage {
            Stage::Place => {
                let frame = film.line.frame(film.n);
                player.stand(frame.t);
                if let Some(moment) = frame.moment {
                    self.timebar.set_moment(moment);
                }
                film.placed(now);
                self.idle_frames = 0;
            }
            Stage::Load => {
                if self.snapshot.is_none() && film.due(self.idle_frames, now) {
                    film.stage = Stage::Shoot;
                    self.snapshot = Some(Snapshot::Hide(SnapshotTo::Frame));
                }
            }
            Stage::Shoot => {}
        }
    }

    /// Снимок кадра записи пришёл.
    pub(super) fn film_frame(&mut self, result: Result<egui::ColorImage, String>) {
        let lang = self.lang;
        let Some(film) = &mut self.film else {
            return;
        };
        let text = match result.and_then(|image| film.write(lang, &image)) {
            Ok(()) if !film.finished() => return,
            Ok(()) => film.finish(lang).unwrap_or_else(|e| trf(lang, "Not saved: {e}", &[("e", &e)])),
            Err(e) => trf(lang, "Not saved: {e}", &[("e", &e)]),
        };
        self.end_film(text);
    }

    /// Запись кончилась: шкала времени возвращается, тур закрывается.
    pub(super) fn end_film(&mut self, text: String) {
        if let Some(film) = self.film.take() {
            self.timebar.range = film.range;
        }
        self.player = None;
        self.open_message = Some(text);
    }

    pub fn with_shot_ui(mut self) -> Self {
        self.shot_ui = true;
        self
    }

    /// Когда место загрузится, снять окно, отдать снимок в sink
    /// и закрыться.
    pub fn with_shot(mut self, sink: ShotSink) -> Self {
        self.shot = Some(Shot { sink: Some(sink), done: Arc::new(AtomicBool::new(false)), idle: 0 });
        self
    }

    /// Шаг снимка окна. busy - загрузка или движение идёт.
    pub(super) fn step_shot(&mut self, ctx: &egui::Context, busy: bool) {
        self.idle_frames = if busy { 0 } else { self.idle_frames.saturating_add(1) };
        let Some(shot) = &mut self.shot else {
            return;
        };
        if shot.done.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        shot.idle = if busy { 0 } else { shot.idle + 1 };        if shot.idle >= SHOT_IDLE_FRAMES
            && let Some(sink) = shot.sink.take()
        {
            let done = shot.done.clone();
            ctx.request_screenshot(move |image| {
                sink(&image);
                done.store(true, Ordering::SeqCst);
            });
        }
        ctx.request_repaint();
    }
}
