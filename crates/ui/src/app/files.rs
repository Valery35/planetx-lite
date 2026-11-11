// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Открытие файлов, ссылок и демо: сцены, KML, векторные слои, растры, облака точек, 3D Tiles, брошенные и выбранные файлы.

use super::*;

impl LiteApp {
    /// Вставить метки из текста KML в выделенную папку или корень.
    /// Не KML - сообщение, «Мои метки» не меняются.
    pub(super) fn paste_kml(&mut self, text: &str) {
        let folder = self.tree_state.selected.clone().unwrap_or_default();
        self.paste_kml_into(&folder, text);
    }

    /// Вставить метки из текста KML в папку folder.
    pub(super) fn paste_kml_into(&mut self, folder: &[usize], text: &str) {
        let looks_kml = text.contains("<kml") || text.contains("<Placemark") || text.contains("<Folder") || text.contains("<Document");
        if !looks_kml {
            return;
        }
        match marks::open(text.as_bytes(), "clipboard.kml") {
            Ok(opened) => {
                let root = marks::root_name(self.lang);
                for item in opened.tree.children {
                    self.marks.insert(folder, item, root);
                }
                self.marks_changed();
                self.open_message = Some(tr(self.lang, "Pasted from the clipboard.").to_string());
            }
            Err(e) => self.open_message = Some(trf(self.lang, "Not pasted: {e}", &[("e", &e)])),
        }
    }

    /// Наборы 3D Tiles прошлого запуска - снова на глобус, первым кадром:
    /// файл читается с диска, ссылка встаёт в очередь ссылок. Камера
    /// к ним не летит. Файл, которого больше нет, - в строке итога.
    pub(super) fn restore_tilesets(&mut self) {
        let Some(text) = self.tilesets_saved.take() else {
            return;
        };
        let mut lost = Vec::new();
        for (origin, visible) in crate::tilesets::parse_index(&text) {
            self.tiles_restoring.insert(origin.text().to_string(), visible);
            match origin {
                crate::tilesets::Origin::Link(url) => self.open_link(&url),
                crate::tilesets::Origin::File(path) => {
                    if !self.reopen_tiles(std::path::Path::new(&path)) {
                        self.tiles_restoring.remove(&path);
                        lost.push(path);
                    }
                }
            }
        }
        if !lost.is_empty() {
            self.open_message = Some(trf(self.lang, "3D Tiles: these sets were not found: {names}", &[("names", &lost.join(", "))]));
            // Пропавшие наборы уходят и из списка в хранилище.
            self.tilesets.save_dirty = true;
        }
    }

    /// Открыть набор с диска снова. Папка последнего открытия остаётся
    /// прежней. false - файл не прочёлся.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn reopen_tiles(&mut self, path: &std::path::Path) -> bool {
        let Ok(bytes) = std::fs::read(path) else {
            return false;
        };
        let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let dir = std::mem::replace(&mut self.open_dir, path.parent().map(std::path::Path::to_path_buf));
        self.opening_path = Some(path.to_path_buf());
        let opened = self.open_tiles(&bytes, &name, None).is_ok();
        self.opening_path = None;
        self.open_dir = dir;
        opened
    }

    /// У страницы диска нет.
    #[cfg(target_arch = "wasm32")]
    pub(super) fn reopen_tiles(&mut self, _path: &std::path::Path) -> bool {
        false
    }

    /// Слой сцены I3S по ссылке link в раздел «Слои», камера - к его
    /// охвату. Узлы и куски приходят запросами по мере надобности.
    pub(super) fn open_i3s(&mut self, bytes: &[u8], link: &str, fly: Option<f64>) -> Result<Option<Pose>, String> {
        let layer = match scene::i3s::layer(bytes) {
            Ok(l) => l,
            Err(e) => {
                self.open_message = Some(trf(self.lang, "The file did not open: {name}: {e}", &[("name", link), ("e", &e.0)]));
                return Err(e.0);
            }
        };
        let title = if layer.name.is_empty() { "I3S".to_string() } else { layer.name.clone() };
        let spots = layer.extent.map(|[w, s, e, n]| vec![((s + n) / 2.0, (w + e) / 2.0), (s, w), (s, e), (n, w), (n, e)]).unwrap_or_default();
        let index = scene::i3s::Index::new(layer, self.nav.camera.ellipsoid, crate::tilesets::THRESHOLD);
        let origin = crate::tilesets::Origin::Link(link.to_string());
        let restoring = self.tiles_restoring.remove(origin.text());
        let id = self.tilesets.add_i3s(crate::tilesets::I3s::new(index), crate::tilesets::scene_server_source(link), &title, Some(origin), spots.clone());
        if let Some(visible) = restoring {
            self.tilesets.apply(id, crate::tilesets::Act::Visible(visible));
        }
        self.open_message = Some(trf(self.lang, "I3S scene layer: {title}.", &[("title", &title)]));
        let camera = &self.nav.camera;
        let pose = marks::fit_points(&spots, camera.fov_y, camera.aspect());
        if let Some(pose) = pose.clone().filter(|_| restoring.is_none() && (fly.is_some() || !self.keep_pose)) {
            match fly {
                Some(now) => crate::input::fly(&mut self.nav, pose, now),
                None => self.nav.show(pose),
            }
        }
        Ok(pose)
    }

    /// Набор 3D Tiles файла name в раздел «Слои», камера - к нему. Архив
    /// несёт куски в себе. У tileset.json куски берутся в программе из
    /// папки последнего открытия, у страницы их нет - нужен архив.
    pub(super) fn open_tiles(&mut self, bytes: &[u8], name: &str, fly: Option<f64>) -> Result<Option<Pose>, String> {
        use crate::tilesets::Source;
        let link = self.fetching_url.clone().filter(|_| !bytes.starts_with(b"PK"));
        let read = if bytes.starts_with(b"PK") {
            scene::tiles3d::ZipSet::open(bytes).and_then(|zip| Ok((scene::tiles3d::Tree::open(&zip.root, &|p| zip.get(p))?, Source::Zip(zip))))
        } else if let Some(link) = &link {
            // По ссылке: куски и внешние наборы - запросами по мере надобности.
            scene::tiles3d::Tree::open_lazy(bytes).map(|tree| (tree, Source::url(link)))
        } else {
            let source = Source::Dir(self.open_dir.clone().unwrap_or_default());
            scene::tiles3d::Tree::open(bytes, &|p| source.get(p)).map(|tree| (tree, source))
        };
        let (tree, source) = match read {
            Ok(set) => set,
            Err(e) => {
                self.open_message = Some(trf(self.lang, "The file did not open: {name}: {e}", &[("name", name), ("e", &e.0)]));
                return Err(e.0);
            }
        };
        let skipped = tree.skipped.len();
        let dir_less = cfg!(target_arch = "wasm32") && !bytes.starts_with(b"PK") && link.is_none();
        let folder = match &link {
            Some(l) => crate::tilesets::link_folder(l),
            None => self.open_dir_name(),
        };
        let title = if name.to_lowercase().ends_with("tileset.json") { folder.unwrap_or_else(|| name.to_string()) } else { data::layers::stem(name).to_string() };
        // Откуда набор: ссылка или файл с диска - по ним он откроется при
        // следующем запуске.
        let origin = match (&self.fetching_url, &self.opening_path) {
            (Some(url), _) => Some(crate::tilesets::Origin::Link(url.clone())),
            (None, Some(path)) => Some(crate::tilesets::Origin::File(path.to_string_lossy().into_owned())),
            (None, None) => None,
        };
        let restoring = origin.as_ref().and_then(|o| self.tiles_restoring.remove(o.text()));
        let added = if dir_less { None } else { self.tilesets.add(tree, source, &title, origin, &self.nav.camera.ellipsoid) };
        if let (Some(id), Some(visible)) = (added, restoring) {
            self.tilesets.apply(id, crate::tilesets::Act::Visible(visible));
        }
        let Some(id) = added else {
            let e = if dir_less {
                tr(self.lang, "The page cannot read the pieces next to tileset.json - open the set as a ZIP archive").to_string()
            } else {
                tr(self.lang, "No piece of the 3D Tiles set was read").to_string()
            };
            self.open_message = Some(trf(self.lang, "The file did not open: {name}: {e}", &[("name", name), ("e", &e)]));
            return Err(e);
        };
        self.open_message = Some(if skipped > 0 {
            trf(self.lang, "3D Tiles: {title}, inner sets not read: {skipped}.", &[("title", &title), ("skipped", &skipped.to_string())])
        } else {
            trf(self.lang, "3D Tiles: {title}.", &[("title", &title)])
        });
        let spots = self.tilesets.get(id).map(|s| s.spots.clone()).unwrap_or_default();
        let camera = &self.nav.camera;
        let pose = marks::fit_points(&spots, camera.fov_y, camera.aspect());
        // Набор прошлого запуска камеру не трогает.
        if let Some(pose) = pose.clone().filter(|_| restoring.is_none() && (fly.is_some() || !self.keep_pose)) {
            match fly {
                Some(now) => crate::input::fly(&mut self.nav, pose, now),
                None => self.nav.show(pose),
            }
        }
        Ok(pose)
    }

    /// Имя папки последнего открытия - название набора tileset.json.
    pub(super) fn open_dir_name(&self) -> Option<String> {
        self.open_dir.as_ref()?.file_name().map(|n| n.to_string_lossy().into_owned())
    }

    /// Векторные слои файла name в «Мои слои», камера - к их охвату.
    /// Подземная модель и треки идут своими окнами. Возвращает позу.
    pub(super) fn open_vector(&mut self, layers: Vec<data::layers::LayerData>, name: &str, fly: Option<f64>) -> Result<Option<Pose>, String> {
        if crate::subsurface::is_subsurface(&layers) {
            return Ok(self.open_subsurface(layers, name, fly));
        }
        if !crate::tracks::Tracks::split(&layers, name).0.is_empty() {
            return Ok(self.open_tracks(layers, name, fly).flatten());
        }
        let before = self.data_layers.list.len();
        let source = self.opening_source();
        let count = self.data_layers.add_from(layers, name, source);
        if count == 0 {
            let e = tr(self.lang, "The file has no layers with geometry").to_string();
            self.open_message = Some(trf(self.lang, "The file did not open: {name}: {e}", &[("name", name), ("e", &e)]));
            return Err(e);
        }
        self.open_message = Some(trf(self.lang, "Layers added: {count}. The «⋯» menu of a row in the My layers section sets the style, labels and extrusion.", &[("count", &count.to_string())]));
        let points: Vec<(f64, f64)> = self.data_layers.list.iter().take(self.data_layers.list.len() - before).flat_map(crate::datalayers::DataLayer::extent_points).collect();
        let camera = &self.nav.camera;
        let pose = marks::fit_points(&points, camera.fov_y, camera.aspect());
        if let Some(pose) = pose.clone().filter(|_| fly.is_some() || !self.keep_pose) {
            match fly {
                Some(now) => crate::input::fly(&mut self.nav, pose, now),
                None => self.nav.show(pose),
            }
        }
        Ok(pose)
    }

    /// Соседние файлы главного файла name: пришедшие отдельно и, в
    /// программе, лежащие рядом с ним в папке последнего открытия.
    pub(super) fn sidecars_for(&self, name: &str) -> Vec<(String, Vec<u8>)> {
        let found: Vec<(String, Vec<u8>)> = data::layers::sidecars_of(name, &self.sidecars).into_iter().cloned().collect();
        #[cfg(not(target_arch = "wasm32"))]
        let mut found = found;
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(dir) = &self.open_dir
            && let Ok(entries) = std::fs::read_dir(dir)
        {
            let base = data::layers::stem(name).to_lowercase();
            for entry in entries.flatten() {
                let file = entry.file_name().to_string_lossy().into_owned();
                let ext = data::layers::extension(&file);
                if data::layers::is_sidecar(&file)
                    && data::layers::stem(&file).to_lowercase() == base
                    && !found.iter().any(|(n, _)| data::layers::extension(n) == ext)
                    && let Ok(bytes) = std::fs::read(entry.path())
                {
                    found.push((file, bytes));
                }
            }
        }
        found
    }

    /// Слои векторного файла: шейпфайл и MapInfo - с соседними файлами.
    pub(super) fn read_vector(&self, bytes: &[u8], name: &str) -> Result<Vec<data::layers::LayerData>, String> {
        if data::layers::needs_sidecars(name) {
            let around = self.sidecars_for(name);
            let refs: Vec<&(String, Vec<u8>)> = around.iter().collect();
            data::layers::bundle(bytes, name, &refs).and_then(|(zip, zip_name)| data::layers::read(zip, &zip_name))
        } else {
            data::layers::read(bytes.to_vec(), name)
        }
    }

    /// Файл с диска, который сейчас открывается, и время его изменения -
    /// источник слоёв для строки «Обновить».
    pub(super) fn opening_source(&self) -> Option<crate::datalayers::Source> {
        let path = self.opening_path.as_ref()?;
        Some(crate::datalayers::Source { path: path.to_string_lossy().into_owned(), modified: crate::datalayers::file_modified(&path.to_string_lossy())? })
    }

    /// Перечитать файл слоя id с диска, «Обновить» строки слоя.
    pub(super) fn reload_layer(&mut self, id: u64) {
        let Some(source) = self.data_layers.get(id).and_then(|l| l.source.clone()) else {
            return;
        };
        let path = std::path::PathBuf::from(&source.path);
        let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let modified = crate::datalayers::file_modified(&source.path).unwrap_or(source.modified);
        let dir = std::mem::replace(&mut self.open_dir, path.parent().map(std::path::Path::to_path_buf));
        let read = crate::datalayers::read_file(&path).and_then(|bytes| self.read_vector(&bytes, &name));
        self.open_dir = dir;
        let read = read.map(|layers| self.data_layers.reload(id, layers, modified));
        self.open_message = Some(match read {
            Ok(true) => trf(self.lang, "Layer refreshed from the file {name}.", &[("name", &name)]),
            Ok(false) => trf(self.lang, "The file {name} has no such layer any more.", &[("name", &name)]),
            Err(e) => trf(self.lang, "The file did not open: {name}: {e}", &[("name", &name), ("e", &e)]),
        });
    }

    /// Открыть векторный файл: шейпфайл и MapInfo - с соседними файлами.
    pub(super) fn open_vector_file(&mut self, bytes: &[u8], name: &str, fly: Option<f64>) -> Result<Option<Pose>, String> {
        let read = self.read_vector(bytes, name);
        // Взятые соседние файлы больше не нужны.
        let base = data::layers::stem(name).to_lowercase();
        self.sidecars.retain(|(n, _)| data::layers::stem(n).to_lowercase() != base);
        match read {
            Ok(layers) => self.open_vector(layers, name, fly),
            Err(e) => {
                let e = if e == data::layers::NO_CRS { tr(self.lang, data::layers::NO_CRS).to_string() } else { e };
                self.open_message = Some(trf(self.lang, "The file did not open: {name}: {e}", &[("name", name), ("e", &e)]));
                Err(e)
            }
        }
    }

    /// Открыть сцену `.planetx`, KML или KMZ при старте: камера сразу
    /// встаёт в позу сцены.
    pub fn with_file(mut self, bytes: Vec<u8>, name: &str) -> Self {
        self.pending_open.push((bytes, name.to_string()));
        self
    }

    /// Папка файлов командной строки: из неё берутся соседние файлы
    /// шейпфайла и MapInfo.
    pub fn with_open_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.open_dir = Some(dir);
        self
    }

    /// Откуда самопроверка `scenes` берёт сцены демо: в программе -
    /// папка, на странице - начало адреса, к нему добавляется
    /// «имя.planetx».
    pub fn with_scenes(mut self, source: &str) -> Self {
        self.scene_source = Some(source.to_string());
        self
    }

    /// Папка данных пользователя для «Моих слоёв».
    pub fn with_user_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.user_dir = Some(dir);
        self
    }

    /// Папка кэша файлов демо.
    pub fn with_demo_cache(mut self, dir: std::path::PathBuf) -> Self {
        self.demo_cache = Some(dir);
        self
    }

    /// Открыть демо: файлы по порядку, из кэша или по ссылке.
    pub(super) fn open_demo(&mut self, demo: &crate::demos::Demo, now: f64) {
        // Одна загрузка за раз: ответ прежней пришёл бы новому демо.
        if self.fetching.is_some() {
            return;
        }
        self.demo_queue = demo.files.iter().copied().collect();
        self.demo_fetching = None;
        self.haul_pending = crate::demos::is_haul(demo);
        self.next_demo_file(now);
    }

    /// Открыть следующие файлы демо из кэша, первый без кэша - начать
    /// грузить.
    pub(super) fn next_demo_file(&mut self, now: f64) {
        while let Some(path) = self.demo_queue.pop_front() {
            let name = crate::demos::name(path).to_string();
            self.remember_url(&crate::demos::url(path));
            if let Some(bytes) = self.cached_demo(path) {
                // Ошибку открытия показывает панель.
                let _ = self.open_file(&bytes, &name, Some(now));
                continue;
            }
            self.loaders.files.start(&crate::demos::url(path));
            self.open_message = Some(trf(self.lang, "Loading the demo: {name}…", &[("name", &name)]));
            self.fetching = Some((name, false));
            self.demo_fetching = Some(path);
            return;
        }
    }

    /// Файл демо из кэша программы.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn cached_demo(&self, path: &str) -> Option<Vec<u8>> {
        crate::demos::cached(self.demo_cache.as_deref()?, path)
    }

    /// У страницы своего кэша нет, файлы хранит кэш браузера.
    #[cfg(target_arch = "wasm32")]
    pub(super) fn cached_demo(&self, _path: &str) -> Option<Vec<u8>> {
        None
    }

    /// Открыть сцену по ссылке, когда она загрузится. Страница так
    /// открывает `?scene=адрес`. Сцены из нескольких вызовов
    /// открываются по порядку.
    pub fn with_scene_url(mut self, url: &str) -> Self {
        self.url_queue.push_back(crate::tilesets::scene_server_link(url));
        if self.fetching.is_none() {
            self.next_scene_url();
        }
        self
    }

    /// Начать грузить следующую сцену из ссылки.
    pub(super) fn next_scene_url(&mut self) {
        let Some(url) = self.url_queue.pop_front() else {
            return;
        };
        let name = url.rsplit('/').next().unwrap_or(&url).split(['?', '#']).next().unwrap_or("").to_string();
        self.loaders.files.start(&url);
        self.remember_url(&url);
        self.fetching = Some((name, false));
        self.fetching_url = Some(url);
    }

    /// Открыть файл по ссылке из строки поиска: сцену, KML, набор
    /// 3D Tiles. Ссылка встаёт в очередь сцен из ссылки.
    pub(super) fn open_link(&mut self, url: &str) {
        self.url_queue.push_back(crate::tilesets::scene_server_link(url));
        if self.fetching.is_none() {
            self.next_scene_url();
        }
    }

    /// Адрес открытого файла - в ссылку «Поделиться», без повторов.
    pub(super) fn remember_url(&mut self, url: &str) {
        if !self.shared.iter().any(|u| u == url) {
            self.shared.push(url.to_string());
        }
    }

    /// Ссылка на вид: адреса открытых сцен и поза камеры. На странице -
    /// её собственный адрес, у программы - страница на GitHub Pages.
    pub(super) fn share_link(&self) -> String {
        crate::share::link(&crate::share::page_address(), &self.shared, &self.nav.pose)
    }

    /// Поделиться ссылкой на вид. На телефоне - окно «Поделиться»
    /// системы, иначе ссылка в буфер обмена.
    pub(super) fn share(&mut self, ctx: &egui::Context) {
        let link = self.share_link();
        if !crate::share::system_share(&link) {
            ctx.copy_text(link);
        }
        let mut message = tr(self.lang, "The link to this view is copied.").to_string();
        if self.shared.is_empty() && self.marks.own_count() > 0 {
            message.push(' ');
            message.push_str(tr(self.lang, "Files opened from the disk are not in the link."));
        }
        self.open_message = Some(message);
    }

    /// Ответ на загрузку сцены по ссылке.
    pub(super) fn step_fetch(&mut self, now: f64) {
        if self.fetching.is_none() {
            return;
        }
        let Some(answer) = self.loaders.files.take() else {
            return;
        };
        let Some((name, for_check)) = self.fetching.take() else {
            return;
        };
        // Пришёл шрифт с иероглифами: он ставится в следующем кадре.
        if let Some(code) = self.font_fetching.take() {
            match answer {
                Ok(bytes) if crate::fonts::is_font(&bytes) => {
                    self.web_fonts.insert(code, bytes);
                    self.fonts_dirty = true;
                    self.open_message = None;
                }
                Ok(_) => self.open_message = Some(format!("{name}: не шрифт")),
                Err(e) => self.open_message = Some(format!("{name}: {e}")),
            }
            return;
        }
        match (answer, for_check) {
            (Ok(bytes), true) => self.open_check_scene(&bytes, &name, now),
            (Ok(bytes), false) => match self.demo_fetching.take() {
                Some(path) => {
                    #[cfg(not(target_arch = "wasm32"))]
                    if let Some(dir) = &self.demo_cache {
                        // Кэш не записался - демо всё равно открывается.
                        let _ = crate::demos::store(dir, path, &bytes);
                    }
                    let _ = path;
                    let _ = self.open_file(&bytes, &name, Some(now));
                    self.next_demo_file(now);
                }
                None => {
                    let _ = self.open_file(&bytes, &name, None);
                    self.fetching_url = None;
                    self.next_scene_url();
                }
            },
            (Err(e), true) => {
                if let Some(check) = &mut self.check {
                    check.scene_opened(Err(format!("{name}: {e}")), now);
                }
            }
            (Err(e), false) => {
                self.demo_fetching = None;
                self.fetching_url = None;
                self.demo_queue.clear();
                self.url_queue.clear();
                self.open_message = Some(format!("{name}: {e}"));
            }
        }
    }

    /// Облако точек LAS, LAZ или COPC, камера - к нему.
    pub(super) fn open_cloud(&mut self, bytes: &[u8], name: &str, fly: Option<f64>) -> Result<Option<Pose>, String> {
        let cloud = match data::cloud::read(bytes.to_vec(), name, CLOUD_POINTS) {
            Ok(c) if !c.points.is_empty() => c,
            Ok(_) => Err("в облаке нет точек".to_string())?,
            Err(e) => {
                self.open_message =
                    Some(trf(self.lang, "The point cloud did not open: {name}: {e}", &[("name", name), ("e", &e)]));
                return Err(e);
            }
        };
        Ok(self.show_cloud(&cloud, name, fly))
    }

    /// Растр высот GeoTIFF - врезка своего рельефа, камера - к нему.
    pub(super) fn open_relief(&mut self, bytes: &[u8], name: &str, fly: Option<f64>) -> Result<Option<Pose>, String> {
        let opened = data::grid::Grid::read(bytes.to_vec(), name)
            .and_then(|grid| data::dem::Dem::new(grid, terrain::inset::BAND_SHARE));
        let dem = match opened {
            Ok(dem) => dem,
            Err(e) => {
                self.open_message = Some(trf(self.lang, "The raster did not open: {name}: {e}", &[("name", name), ("e", &e)]));
                return Err(e);
            }
        };
        let corners = dem.grid().corners();
        {
            let mut store = self.heights.write();
            self.relief.add(name, dem, bytes.to_vec(), &mut store);
            // Дно озера демо - сразу батиметрия, как у модуля.
            if let Some(lake) = crate::demos::lake(name) {
                let _ = self.relief.set_lake(name, Some(lake), &mut store);
                self.water = None;
            }
        }
        let camera = &self.nav.camera;
        let pose = corners.and_then(|c| marks::fit_points(&c, camera.fov_y, camera.aspect()));
        if let Some(pose) = pose.clone().filter(|_| fly.is_some() || !self.keep_pose) {
            match fly {
                Some(now) => crate::input::fly(&mut self.nav, pose, now),
                None => self.nav.show(pose),
            }
        }
        self.open_message = Some(trf(self.lang, "Own terrain: {names}", &[("names", &self.relief.names().join(", "))]));
        Ok(pose)
    }

    /// Растры для классификации: свой рельеф и многоканальные.
    pub(super) fn open_rasters(&self) -> Vec<crate::aefclass::Raster> {
        let mut out = self.relief.files();
        out.extend(self.band_rasters.iter().cloned());
        out
    }

    /// Многоканальный растр: высот в нём нет, рельеф остаётся прежним.
    /// Каналы - признаки окна «Классификация», камера - к растру.
    pub(super) fn open_bands(&mut self, bytes: &[u8], name: &str, fly: Option<f64>) -> Result<Option<Pose>, String> {
        let source = match data::rasterclass::Source::read(bytes, name) {
            Ok(source) => source,
            Err(problem) => {
                let e = crate::aefclass::problem_text(self.lang, problem);
                self.open_message = Some(trf(self.lang, "The raster did not open: {name}: {e}", &[("name", name), ("e", e)]));
                return Err(e.to_string());
            }
        };
        self.band_rasters.retain(|r| r.0 != name);
        self.band_rasters.push((name.to_string(), Arc::new(bytes.to_vec())));
        let camera = &self.nav.camera;
        let pose = source.corners().and_then(|c| marks::fit_points(&c, camera.fov_y, camera.aspect()));
        if let Some(pose) = pose.clone().filter(|_| fly.is_some() || !self.keep_pose) {
            match fly {
                Some(now) => crate::input::fly(&mut self.nav, pose, now),
                None => self.nav.show(pose),
            }
        }
        self.open_message = Some(trf(self.lang, "Raster {name}: bands {n}. It is a source of the Classification window, the terrain has not changed.", &[("name", name), ("n", &source.bands().to_string())]));
        Ok(pose)
    }

    /// Открыть файл. fly - лететь к позе сцены, иначе встать в неё сразу.
    /// Возвращает позу сцены или ошибку.
    pub(super) fn open_file(&mut self, bytes: &[u8], name: &str, fly: Option<f64>) -> Result<Option<Pose>, String> {
        let lower = name.to_lowercase();
        // Пакет аддина - окно установки с правами и предупреждением.
        #[cfg(feature = "addins")]
        if lower.ends_with(".pxaddin") {
            self.addins.offer_file(self.lang, name, bytes);
            return Ok(None);
        }
        // Растр кровли, которого ждёт подземная модель, - ей.
        if let Some(sub) = self.subsurface.as_mut().filter(|s| s.wants(name)) {
            sub.add_side(name, bytes.to_vec());
            return Ok(None);
        }
        // Соседний файл шейпфайла или MapInfo ждёт своего главного.
        if data::layers::is_sidecar(name) {
            self.sidecars.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
            self.sidecars.push((name.to_string(), bytes.to_vec()));
            if let Some(at) = fly {
                self.waiting_since = at;
            }
            return Ok(None);
        }
        // На странице файлы приходят по одному: главный ждёт соседних.
        if cfg!(target_arch = "wasm32")
            && data::layers::needs_sidecars(name)
            && let Some(at) = fly
        {
            self.waiting.push((bytes.to_vec(), name.to_string()));
            self.waiting_since = at;
            return Ok(None);
        }
        // Набор 3D Tiles - архив с tileset.json или сам tileset.json, раньше
        // слоёв: у них тоже .zip и .json.
        if (lower.ends_with(".zip") && scene::tiles3d::zip_has_tileset(bytes)) || (lower.ends_with(".json") && scene::tiles3d::is_tileset(bytes)) {
            return self.open_tiles(bytes, name, fly);
        }
        // Слой сцены I3S или сервис SceneServer по ссылке.
        if let Some(link) = self.fetching_url.clone().filter(|u| u.contains("/SceneServer")) {
            if scene::i3s::is_layer(bytes) {
                return self.open_i3s(bytes, &link, fly);
            }
            if let Some(first) = scene::i3s::service_layers(bytes).first() {
                let layer = crate::tilesets::scene_server_layer(&link, *first);
                self.url_queue.push_front(layer);
                return Ok(None);
            }
        }
        if data::layers::is_vector(name) && !lower.ends_with(".kmz") {
            return self.open_vector_file(bytes, name, fly);
        }
        // Модель самосвала демо ждёт слой самосвалов.
        if name == crate::tracks::DEMO_MODEL {
            let file = PickedFile { name: name.to_string(), path: None, bytes: bytes.to_vec() };
            let (main, _, package) = Self::model_package(std::slice::from_ref(&file)).ok_or_else(|| trf(self.lang, "The model cannot be read: {name}", &[("name", name)]))?;
            self.tracks.demo_model = Some(crate::tracks::Model { name: main, package: Arc::new(package) });
            return Ok(None);
        }
        if lower.ends_with(".tif") || lower.ends_with(".tiff") {
            if data::rasterclass::feature_bands(bytes).is_some_and(|n| n > 1) {
                return self.open_bands(bytes, name, fly);
            }
            return self.open_relief(bytes, name, fly);
        }
        if lower.ends_with(".las") || lower.ends_with(".laz") {
            return self.open_cloud(bytes, name, fly);
        }
        match marks::open(bytes, name) {
            Ok(opened) => {
                // Слои хозяина сцены Lite не открывает, но хранит для
                // записи сцены обратно.
                if let Some(s) = opened.scene.as_ref().filter(|s| !s.layers.is_empty()) {
                    self.scene_layers.insert(opened.tree.name.clone(), s.layers.clone());
                }
                // Сцена ложится папкой в «Мои метки», прежние остаются.
                self.marks.add(opened.tree, &opened.name, marks::root_name(self.lang));
                self.player = None;
                self.marks_changed();
                // Тело сцены - до её позы, как `ui/scene.py` модуля. Сцена
                // неба открывает небо со своим взглядом.
                match opened.body.as_deref() {
                    Some("sky") => {
                        self.enter_sky();
                        if let (Some([ra, dec, fov]), Some(sky)) = (opened.sky, &mut self.sky) {
                            sky.view.set(ra, dec, Some(fov));
                        }
                        self.open_message = None;
                        return Ok(None);
                    }
                    Some(body) => {
                        self.leave_sky();
                        self.set_body(body);
                    }
                    None => {}
                }
                // Слои сцены, как `_apply_extra` модуля: известные строки
                // каталога включаются или выключаются. Земные строки на
                // другом теле не трогаются.
                for (key, on) in &opened.extras {
                    if let Some(extra) = crate::layers::Extra::from_key(key)
                        && (self.planet.earth || !extra.earth_only())
                        && self.extras_on.contains(&extra) != *on
                    {
                        self.extras.insert(extra);
                        self.set_extra(extra, *on);
                    }
                }
                if let Some(s) = &opened.scene {
                    self.apply_scene_view(s);
                }
                // Поза, заданная при запуске, сильнее позы сцены.
                if let Some(pose) = opened.pose.clone().filter(|_| fly.is_some() || !self.keep_pose) {
                    match fly {
                        Some(now) => crate::input::fly(&mut self.nav, pose, now),
                        None => self.nav.show(pose),
                    }
                }
                // Название сцены видно в «Моих метках», строка
                // сообщения - для ошибок и загрузки.
                self.open_message = None;
                Ok(opened.pose)
            }
            Err(e) => {
                self.open_message = Some(trf(self.lang, "The file did not open: {name}: {e}", &[("name", name), ("e", &e)]));
                Err(e)
            }
        }
    }

    /// Открыть подземную модель из слоёв файла name, `start` модуля:
    /// прежняя модель уходит, окно режима открывается, камера летит
    /// к модели.
    /// Точечные слои файла с полем времени - треки, остальные слои -
    /// в «Мои данные». None - треков в файле нет, иначе поза к охвату
    /// точек треков.
    pub(super) fn open_tracks(&mut self, layers: Vec<data::layers::LayerData>, name: &str, fly: Option<f64>) -> Option<Option<Pose>> {
        let (found, rest) = crate::tracks::Tracks::split(&layers, name);
        if found.is_empty() {
            return None;
        }
        let points: Vec<(f64, f64)> = found.iter().flat_map(crate::tracks::Layer::places).collect();
        let others: Vec<data::layers::LayerData> = layers.into_iter().enumerate().filter(|(n, _)| rest.contains(n)).map(|(_, l)| l).collect();
        self.data_layers.add(others, name);
        let count = found.len();
        let demo = self.tracks.demo_model.is_some();
        self.tracks.add(found, name);
        self.time_extent();
        // Окно «Треки» открывается у своего файла, у демо - нет.
        self.tracks.open |= !demo;
        self.open_message = Some(trf(self.lang, "Tracks: {count}. The time slider grows the paths.", &[("count", &count.to_string())]));
        let camera = &self.nav.camera;
        let pose = marks::fit_points(&points, camera.fov_y, camera.aspect()).map(|p| Pose::new(p.lat, p.lon, p.distance, TRACK_HEADING, TRACK_TILT));
        if let Some(pose) = pose.clone().filter(|_| fly.is_some() || !self.keep_pose) {
            match fly {
                Some(now) => crate::input::fly(&mut self.nav, pose, now),
                None => self.nav.show(pose),
            }
        }
        Some(pose)
    }

    /// Охват шкалы времени - время меток и треков.
    pub(super) fn time_extent(&mut self) {
        // Метки, треки и слои-картинки со временем - снимки по датам.
        // Ряды дат включённых тем NASA GIBS: день темы задаёт шкала, как
        // ползунок темы модуля.
        let themes = (0..crate::maps::count())
            .filter(|n| self.extras_on.contains(&crate::layers::Extra::Map(*n)))
            .filter_map(crate::maps::entry)
            .filter_map(|e| match self.theme_days.get(e.key) {
                Some(ThemeDay::Day(_, ready)) => data::gibs::span(ready),
                _ => None,
            })
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
        let extent = [self.marks.extent(), self.tracks.span(), self.data_layers.extent(), themes].into_iter().flatten().reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
        self.timebar.set_extent(extent);
    }

    pub(super) fn open_subsurface(&mut self, layers: Vec<data::layers::LayerData>, name: &str, fly: Option<f64>) -> Option<Pose> {
        let now = fly.unwrap_or(0.0);
        let mut sub = crate::subsurface::Subsurface::new(layers, name, self.open_dir.clone(), now);
        sub.window_open = true;
        let pose = sub.middle().map(|(lat, lon, extent)| Pose::new(lat, lon, (extent * 1.5).max(2000.0), 30.0, 60.0));
        self.subsurface = Some(sub);
        if let Some(globe) = &mut self.globe {
            globe.clear_subsurface();
        }
        if let Some(pose) = pose.clone().filter(|_| fly.is_some() || !self.keep_pose) {
            match fly {
                Some(now) => crate::input::fly(&mut self.nav, pose, now),
                None => self.nav.show(pose),
            }
        }
        self.open_message = None;
        pose
    }

    /// Папка данных программы, у страницы None.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn data_dir(&self) -> Option<std::path::PathBuf> {
        self.demo_cache.as_ref()?.parent().map(std::path::Path::to_path_buf)
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn data_dir(&self) -> Option<std::path::PathBuf> {
        None
    }

    /// Выбор файла сцены. В программе - диалог системы, на странице -
    /// выбор файла браузером, байты приходят в ящик брошенных файлов.
    pub(super) fn pick_file(&mut self, ctx: &egui::Context, now: f64) {
        // Векторные форматы слоёв - из data::layers::EXTENSIONS, на
        // странице и соседние файлы шейпфайла: браузер не видит папку.
        const BASE: [&str; 9] = ["planetx", "kml", "kmz", "tif", "tiff", "las", "laz", "csv", "pxaddin"];
        let mut kinds: Vec<&str> = BASE.to_vec();
        kinds.extend(data::layers::EXTENSIONS);
        if cfg!(target_arch = "wasm32") {
            kinds.extend(data::layers::SIDECARS);
        }
        let title = "PlanetX, KML, KMZ, GeoPackage, Shapefile, GeoJSON, GPX, MapInfo, FlatGeobuf, DXF, GeoTIFF, LAS, LAZ, CSV, 3D Tiles";
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            if let Some(path) = rfd::FileDialog::new().add_filter(title, &kinds).pick_file() {
                let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
                match std::fs::read(&path) {
                    Ok(bytes) => {
                    self.open_dir = path.parent().map(std::path::Path::to_path_buf);
                    // Ошибку открытия показывает панель.
                    self.opening_path = Some(path.clone());
                    let _ = self.open_file(&bytes, &name, Some(now));
                    self.opening_path = None;
                }
                    Err(e) => self.open_message = Some(format!("{name}: {e}")),
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = now;
            let (slot, ctx) = (self.dropped_box.clone(), ctx.clone());
            let dialog = rfd::AsyncFileDialog::new().add_filter(title, &kinds);
            wasm_bindgen_futures::spawn_local(async move {
                // Несколько файлов - шейпфайл с его .dbf и .prj.
                for file in dialog.pick_files().await.unwrap_or_default() {
                    let bytes = file.read().await;
                    if let Ok(mut s) = slot.lock() {
                        s.push((Ok(bytes), file.file_name()));
                    }
                }
                ctx.request_repaint();
            });
        }
    }

    /// Выбор картинки или модели для наложения, как окна выбора файла
    /// `add_overlay`, `add_model` и «Обзор…» модуля. У модели в программе
    /// пакет собирается из файла и файлов рядом с ним, на странице -
    /// из всех выбранных файлов.
    pub(super) fn pick_for(&mut self, ctx: &egui::Context, pick: Pick) {
        let model = match &pick {
            Pick::New(kind, _) => *kind == scene::overlays::Kind::Model,
            Pick::Browse => self.overlay_props.as_ref().is_some_and(|p| p.edit.kind() == scene::overlays::Kind::Model),
            Pick::TrackModel(_) => true,
        };
        let lang = self.lang;
        let (title, kinds): (&str, &[&str]) = if model {
            (tr(lang, "3D models (*.dae *.gltf *.glb *.obj *.kmz)"), &["dae", "gltf", "glb", "obj", "kmz"])
        } else {
            (tr(lang, "Images (*.png *.jpg *.jpeg *.gif)"), &["png", "jpg", "jpeg", "gif"])
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            let Some(path) = rfd::FileDialog::new().add_filter(title, kinds).pick_file() else {
                return;
            };
            let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let file = PickedFile { name, path: Some(path.to_string_lossy().into_owned()), bytes };
                    if let Ok(mut s) = self.picked_box.lock() {
                        s.push((pick, vec![file]));
                    }
                }
                Err(e) => self.open_message = Some(format!("{name}: {e}")),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let (slot, ctx) = (self.picked_box.clone(), ctx.clone());
            let title = title.to_string();
            let kinds: Vec<String> = kinds.iter().map(|k| k.to_string()).collect();
            wasm_bindgen_futures::spawn_local(async move {
                let dialog = rfd::AsyncFileDialog::new().add_filter(&title, &kinds);
                // У модели - файл модели и его картинки сразу.
                let files = if model { dialog.pick_files().await.unwrap_or_default() } else { dialog.pick_file().await.into_iter().collect() };
                let mut read = Vec::new();
                for file in files {
                    read.push(PickedFile { name: file.file_name(), path: None, bytes: file.read().await });
                }
                if !read.is_empty()
                    && let Ok(mut s) = slot.lock()
                {
                    s.push((pick, read));
                }
                ctx.request_repaint();
            });
        }
    }

    /// Пакет модели из выбранных файлов: главный - первый файл модели,
    /// остальные - по ссылкам из него. В программе файлы рядом читаются
    /// с диска. Отдаёт имя главного файла, путь на диске и пакет.
    pub(super) fn model_package(files: &[PickedFile]) -> Option<(String, Option<String>, Vec<u8>)> {
        let main = files.iter().find(|f| scene::models::EXTENSIONS.iter().any(|e| f.name.to_lowercase().ends_with(e)))?;
        let folder = main.path.as_deref().and_then(|p| std::path::Path::new(p).parent().map(std::path::Path::to_path_buf));
        let fetch = |reference: &str| -> Option<Vec<u8>> {
            let short = reference.rsplit(['/', '\\']).next().unwrap_or(reference);
            if let Some(f) = files.iter().find(|f| f.name == reference || f.name == short) {
                return Some(f.bytes.clone());
            }
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(dir) = &folder {
                return std::fs::read(dir.join(reference.replace('\\', "/"))).ok();
            }
            let _ = &folder;
            None
        };
        let package = scene::models::collect(&main.name, &main.bytes, &fetch, &std::collections::BTreeMap::new()).ok()?;
        Some((main.name.clone(), main.path.clone(), package))
    }

    /// Выбранные файлы: новое наложение и его окно свойств или новая
    /// картинка открытого окна.
    pub(super) fn picked(&mut self, now: f64) {
        let arrived: Vec<(Pick, Vec<PickedFile>)> = self.picked_box.lock().map(|mut s| s.drain(..).collect()).unwrap_or_default();
        for (pick, files) in arrived {
            let Some(first) = files.first() else {
                continue;
            };
            let (first_name, first_bytes, first_path) = (first.name.clone(), first.bytes.clone(), first.path.clone());
            let model = files.iter().any(|f| scene::models::EXTENSIONS.iter().any(|e| f.name.to_lowercase().ends_with(e)));
            // KMZ модели открывается как файл меток: модель в нём уже стоит.
            if first_name.to_lowercase().ends_with(".kmz") {
                let _ = self.open_file(&first_bytes, &first_name, Some(now));
                continue;
            }
            let stem = first_name.rsplit_once('.').map_or(first_name.as_str(), |(s, _)| s).to_string();
            match pick {
                Pick::New(kind, folder) => {
                    let overlay = if model || kind == scene::overlays::Kind::Model {
                        let Some((name, path, package)) = Self::model_package(&files) else {
                            self.open_message = Some(trf(self.lang, "The model cannot be read: {name}", &[("name", &first_name)]));
                            continue;
                        };
                        let mut o = crate::overlayprops::new_overlay(scene::overlays::Kind::Model, &stem, &self.nav, 1.0);
                        // Модель - ссылкой на файл, как Link у Google Earth:
                        // правка файла видна на глобусе. На странице пути нет,
                        // модель - в файле меток.
                        match path {
                            Some(p) => {
                                self.linked_models.insert(p.clone(), (file_stamp_of(&p), now, Some(Arc::new(package))));
                                o.href = p;
                            }
                            None => {
                                o.href = name;
                                o.image = Some(package);
                            }
                        }
                        o
                    } else {
                        let dims = image::ImageReader::new(std::io::Cursor::new(&first_bytes)).with_guessed_format().ok().and_then(|r| r.into_dimensions().ok());
                        let Some((w, h)) = dims else {
                            self.open_message = Some(trf(self.lang, "The image cannot be read: {name}", &[("name", &first_name)]));
                            continue;
                        };
                        let mut o = crate::overlayprops::new_overlay(kind, &stem, &self.nav, f64::from(w) / f64::from(h.max(1)));
                        o.href = first_name.clone();
                        o.image = Some(first_bytes.clone());
                        o
                    };
                    let at = self.marks.insert(&folder, scene::kml::Item::Overlay(overlay), marks::root_name(self.lang));
                    self.tree_state.selected = Some(at.clone());
                    self.marks_changed();
                    self.open_overlay_props(at, now);
                }
                Pick::Browse => {
                    let package = if model { Self::model_package(&files) } else { Some((first_name.clone(), first_path, first_bytes)) };
                    match (package, &mut self.overlay_props) {
                        (Some((name, path, bytes)), Some(props)) => props.set_file(bytes, &name, path),
                        (None, _) => self.open_message = Some(trf(self.lang, "The model cannot be read: {name}", &[("name", &first_name)])),
                        _ => {}
                    }
                }
                Pick::TrackModel(n) => match Self::model_package(&files) {
                    Some((name, _, package)) => self.tracks.set_model(n, crate::tracks::Model { name, package: Arc::new(package) }),
                    None => self.open_message = Some(trf(self.lang, "The model cannot be read: {name}", &[("name", &first_name)])),
                },
            }
        }
    }

    /// Пакет модели по ссылке на файл в программе или None. Пакет
    /// помнится до смены времени правки файла.
    pub(super) fn linked_package(&mut self, href: &str, now: f64) -> Option<Arc<Vec<u8>>> {
        if href.is_empty() || crate::pictures::is_remote(href) || cfg!(target_arch = "wasm32") {
            return None;
        }
        let path = crate::netlinks::local_path(href)?;
        if let Some((_, _, Some(p))) = self.linked_models.get(href) {
            return Some(p.clone());
        }
        let bytes = read_local(&path)?;
        let name = std::path::Path::new(&path).file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let file = PickedFile { name, path: Some(path.clone()), bytes };
        let package = Self::model_package(std::slice::from_ref(&file)).map(|(_, _, p)| Arc::new(p));
        self.linked_models.insert(href.to_string(), (file_stamp_of(&path), now, package.clone()));
        package
    }

    /// Файлы, брошенные в окно. В программе файл читается сразу, на
    /// странице браузер читает его асинхронно, байты приходят в ящик
    /// [`LiteApp::dropped_box`] и открываются в следующем кадре.
    pub(super) fn dropped(&mut self, ctx: &egui::Context, now: f64) {
        let mut files = ctx.input(|i| i.raw.dropped_files.clone());
        // Соседние файлы шейпфайла - раньше главного.
        files.sort_by_key(|f| !data::layers::is_sidecar(&f.path().to_string_lossy()));
        for file in files {
            let name = file.path().file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
            #[cfg(not(target_arch = "wasm32"))]
            match file.bytes() {
                Ok(bytes) => {
                    if let Some(dir) = file.path().parent().filter(|d| !d.as_os_str().is_empty()) {
                        self.open_dir = Some(dir.to_path_buf());
                    }
                    // Ошибку открытия показывает панель.
                    self.opening_path = Some(file.path().to_path_buf()).filter(|p| p.is_file());
                    let _ = self.open_file(&bytes, &name, Some(now));
                    self.opening_path = None;
                }
                Err(e) => self.open_message = Some(format!("{name}: {e}")),
            }
            #[cfg(target_arch = "wasm32")]
            {
                let (slot, ctx) = (self.dropped_box.clone(), ctx.clone());
                wasm_bindgen_futures::spawn_local(async move {
                    let read = file.bytes_async().await;
                    if let Ok(mut s) = slot.lock() {
                        s.push((read, name));
                    }
                    ctx.request_repaint();
                });
            }
        }
        let mut arrived: Vec<(Result<Vec<u8>, String>, String)> =
            self.dropped_box.lock().map(|mut s| s.drain(..).collect()).unwrap_or_default();
        arrived.sort_by_key(|(_, name)| !data::layers::is_sidecar(name));
        for (read, name) in arrived {
            match read {
                Ok(bytes) => {
                    // Ошибку открытия показывает панель.
                    let _ = self.open_file(&bytes, &name, Some(now));
                }
                Err(e) => self.open_message = Some(format!("{name}: {e}")),
            }
        }
        // Главные файлы открываются, когда соседние перестали приходить.
        if !self.waiting.is_empty() {
            if now - self.waiting_since >= SIDECAR_PAUSE {
                for (bytes, name) in std::mem::take(&mut self.waiting) {
                    let _ = self.open_vector_file(&bytes, &name, Some(now));
                }
                self.sidecars.clear();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(SIDECAR_PAUSE));
            }
        }
    }
}
