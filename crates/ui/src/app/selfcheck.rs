// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Самопроверка: сцены, туры проверки, итог.

use super::*;

impl LiteApp {
    /// Сцена самопроверки по имени: из папки сразу, по ссылке - позже.
    pub(super) fn load_check_scene(&mut self, name: &str, now: f64) {
        let file = format!("{name}.planetx");
        let Some(source) = self.scene_source.clone() else {
            if let Some(check) = &mut self.check {
                check.scene_opened(Err("не задано, откуда брать сцены демо".into()), now);
            }
            return;
        };
        if source.starts_with("http://") || source.starts_with("https://") || cfg!(target_arch = "wasm32") {
            self.loaders.files.start(&format!("{source}{file}"));
            self.fetching = Some((file, true));
        } else {
            self.read_check_scene(&source, &file, now);
        }
    }

    /// Сцена самопроверки из папки.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn read_check_scene(&mut self, dir: &str, file: &str, now: f64) {
        match std::fs::read(std::path::Path::new(dir).join(file)) {
            Ok(bytes) => self.open_check_scene(&bytes, file, now),
            Err(e) => {
                if let Some(check) = &mut self.check {
                    check.scene_opened(Err(format!("{file}: {e}")), now);
                }
            }
        }
    }

    /// На странице папок нет, сцены приходят по ссылке.
    #[cfg(target_arch = "wasm32")]
    pub(super) fn read_check_scene(&mut self, _dir: &str, _file: &str, _now: f64) {}

    /// Открыть сцену самопроверки и отдать итог проверке: строки
    /// дерева, поза, количество туров.
    pub(super) fn open_check_scene(&mut self, bytes: &[u8], name: &str, now: f64) {
        // Самопроверка сверяет одну сцену: «Мои метки» перед ней пусты.
        self.marks.clear();
        let opened = self.open_file(bytes, name, None).map(|pose| {
            let p = &self.nav.pose;
            let pose_ok = pose.is_some_and(|s| {
                (p.lat - s.lat).abs() < check::POSE_TOLERANCE
                    && (p.lon - s.lon).abs() < check::POSE_TOLERANCE
                    && (p.distance / s.distance - 1.0).abs() < check::POSE_TOLERANCE
                    && (p.heading - s.heading).abs() < check::POSE_TOLERANCE
                    && (p.tilt - s.tilt).abs() < check::POSE_TOLERANCE
            });
            let camera = &self.nav.camera;
            let (counts, tours) = self.marks.last().map_or(([0; 4], 0), |t| {
                (crate::tours::counts(t), crate::tours::scene_tours(t, camera.fov_y, camera.aspect()).len())
            });
            (counts, pose_ok, tours)
        });
        if let Some(check) = &mut self.check {
            check.scene_opened(opened, now);
        }
    }

    /// Тур сцены самопроверки по номеру.
    pub(super) fn start_check_tour(&mut self, index: usize, now: f64) {
        let camera = &self.nav.camera;
        let tour = self
            .marks
            .last()
            .and_then(|t| crate::tours::scene_tours(t, camera.fov_y, camera.aspect()).into_iter().nth(index));
        self.player = tour.and_then(|(name, stops)| Player::new(&name, &self.nav.pose, stops, camera.fov_y, now));
        self.tour_index = None;
        self.nav.stop();
        // Длинный тур проигрывается целиком, но ускоренно.
        let started = self.player.as_mut().map(|p| {
            p.rate = (p.tour.duration / check::TOUR_LIMIT).max(1.0);
            (p.tour.duration / p.rate, p.rate)
        });
        if let Some(check) = &mut self.check {
            check.tour_started(started);
        }
    }

    /// Снимок окна вместе с левой панелью, а не только кадр глобуса.
    /// Показать при запуске снимки события по ключу «источник/id».
    pub fn with_event(mut self, key: &str) -> Self {
        self.events.open(&*self.loaders.request);
        self.pending_event = Some(key.to_string());
        self
    }

    /// Начать с позы pose вместо начального вида.
    pub fn with_pose(mut self, pose: Pose) -> Self {
        self.nav.show(pose);
        self.keep_pose = true;
        self
    }

    pub(super) fn finish_check(&mut self, ctx: &egui::Context) {
        let Some(check) = &self.check else {
            return;
        };
        if !check.done() {
            return;
        }
        let source = |name: &'static str, limit: usize, loader: &dyn TileLoader| {
            let s = loader.stats();
            SourceStats {
                name,
                limit,
                peak: s.requests_peak.load(Ordering::SeqCst),
                total: s.requests_total.load(Ordering::SeqCst),
                disk: s.disk_hits.load(Ordering::SeqCst),
            }
        };
        let sources = [
            source("osm", net::OSM.max_requests, self.loaders.osm.as_ref()),
            source("esri", net::ESRI.max_requests, self.loaders.esri.as_ref()),
            source("terrarium", net::TERRARIUM.max_requests, self.loaders.terrain.as_ref()),
            source("openfreemap", net::OPENFREEMAP.max_requests, self.loaders.places.as_ref()),
        ];
        let (report, ok) = check.report(&sources, (self.nav.camera.width, self.nav.camera.height), &self.search.sent);
        (self.report)(&report, ok);
        self.check = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}
