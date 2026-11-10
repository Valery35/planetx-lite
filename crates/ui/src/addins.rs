// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Аддины Lite: окно «Аддины», каталог, установка, обновление
//! и удаление, команды аддинов в меню слоя «Моих слоёв».
//!
//! Устройство - как каталог расширений RustGIS (решение автора
//! от 10 ноября 2026 года). Каталог - `registry.json` хранилища
//! `planetx-lite-addins` на GitHub Pages: у каждого аддина - адрес
//! пакета `.pxaddin` и его SHA-256. Пакет ставится, только если сумма
//! совпала. Аддин из файла ставится мимо каталога, с предупреждением.
//! При установке окно показывает права аддина. Аддин из списка
//! `blocked` каталога выключается.
//!
//! Элементы интерфейса аддина задаются его описанием, своего рисования
//! у аддина нет: команда - пункт меню слоя, параметры - окно из готовых
//! полей.

use std::sync::{Arc, Mutex, RwLock};

use addin_host::{Addin, Manifest, Package, ParamKind, Text};
use eframe::egui;
use serde_json::Value;

use crate::i18n::{Lang, tr, trf};

/// Каталог аддинов по умолчанию.
pub const DEFAULT_CATALOG: &str = "https://valery35.github.io/planetx-lite-addins/registry.json";
/// Ключ списка установленных аддинов в хранилище.
pub const KEY: &str = "addins";
/// Папка пакетов в папке данных программы.
pub const DIR: &str = "addins";
/// Наибольший ответ каталога, байт.
const CATALOG_BYTES: u64 = 1 << 20;
/// Наибольший пакет, байт.
const PACKAGE_BYTES: u64 = 40 << 20;

/// Команда аддина для меню.
#[derive(Debug, Clone)]
pub struct MenuItem {
    pub addin: String,
    pub command: String,
    pub title: Text,
    pub kinds: Vec<String>,
}

/// Команды включённых аддинов: меню слоя читает их, не зная окна.
static MENU: RwLock<Vec<MenuItem>> = RwLock::new(Vec::new());

/// Команды включённых аддинов.
pub fn menu_items() -> Vec<MenuItem> {
    MENU.read().map(|m| m.clone()).unwrap_or_default()
}

fn ru(lang: Lang) -> bool {
    lang == Lang::RU
}

/// Установленный аддин.
pub struct Installed {
    pub package: Package,
    pub enabled: bool,
    /// Поставлен из файла, мимо каталога.
    pub from_file: bool,
    /// Загруженный модуль, грузится при первой команде.
    addin: Option<Addin>,
}

/// Строка каталога.
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: String,
    pub version: String,
    pub title: Text,
    pub description: Text,
    pub author: String,
    pub license: String,
    /// Полный адрес пакета.
    pub url: String,
    pub sha256: String,
    pub permissions: Vec<String>,
}

/// Каталог: строки и снятые аддины (id или id@версия).
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    pub blocked: Vec<String>,
}

/// Адрес относительно адреса каталога.
fn resolve(base: &str, path: &str) -> String {
    if path.starts_with("https://") || path.starts_with("http://") {
        return path.to_string();
    }
    let dir = base.rsplit_once('/').map_or(base, |(d, _)| d);
    format!("{dir}/{}", path.trim_start_matches("./"))
}

/// Разобрать `registry.json`.
pub fn parse_catalog(text: &str, base: &str) -> Result<Catalog, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("каталог не JSON: {e}"))?;
    let mut out = Catalog::default();
    for e in v.get("addins").and_then(Value::as_array).map_or(&[][..], Vec::as_slice).iter().take(1000) {
        let s = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or_default().chars().take(2000).collect::<String>();
        let (id, package, sha256) = (s("id"), s("package"), s("sha256").to_lowercase());
        if id.is_empty() || package.is_empty() || sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let text = |k: &str| {
            let t = |l: &str| e.get(k).and_then(|x| x.get(l)).and_then(Value::as_str).unwrap_or_default().to_string();
            Text { ru: t("ru"), en: t("en") }
        };
        out.entries.push(Entry {
            title: text("title"),
            description: text("description"),
            version: s("version"),
            author: s("author"),
            license: s("license"),
            url: resolve(base, &package),
            permissions: e.get("permissions").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default(),
            sha256,
            id,
        });
    }
    out.blocked = v.get("blocked").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    Ok(out)
}

impl Catalog {
    /// Снят ли аддин id версии version.
    pub fn blocks(&self, id: &str, version: &str) -> bool {
        self.blocked.iter().any(|b| b == id || *b == format!("{id}@{version}"))
    }
}

/// SHA-256 байтов шестнадцатеричной строкой.
pub fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Сравнение версий вида 1.2.3 по числам.
pub fn newer(a: &str, b: &str) -> bool {
    let n = |s: &str| s.split('.').map(|p| p.trim().parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    n(a) > n(b)
}

/// Выбранные файлы пакетов: имя и байты. Диалог страницы кладёт их
/// сюда из своей задачи.
type Picked = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

/// Вкладка окна.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Installed,
    Catalog,
}

/// Окно параметров команды над слоем.
pub struct Run {
    pub layer: u64,
    pub layer_title: String,
    /// Объектов пойдёт в команду: выделенные или все.
    pub count: usize,
    pub addin: String,
    pub command: String,
    pub values: serde_json::Map<String, Value>,
}

/// Что окно просит у программы.
pub enum Ask {
    /// Выполнить команду: окно параметров закрыто кнопкой «Выполнить».
    Run(Run),
}

/// Аддины программы.
pub struct Addins {
    pub installed: Vec<Installed>,
    pub open: bool,
    pub catalog_url: String,
    catalog: Option<Catalog>,
    catalog_error: Option<String>,
    catalog_request: Option<Box<dyn net::Request>>,
    /// Скачивание пакета строки каталога.
    download: Option<(Entry, Box<dyn net::Request>)>,
    /// Аддин ждёт подтверждения установки: пакет и откуда он.
    confirm: Option<(Package, bool)>,
    /// Выбранные файлы пакетов: имя и байты.
    picked: Picked,
    tab: Tab,
    pub message: Option<String>,
    /// Список изменился - записать.
    pub dirty: bool,
    /// Пакеты убранных аддинов: удалить файлы.
    pub dropped: Vec<String>,
    pub run: Option<Run>,
}

impl Default for Addins {
    fn default() -> Self {
        Addins {
            installed: Vec::new(),
            open: false,
            catalog_url: DEFAULT_CATALOG.to_string(),
            catalog: None,
            catalog_error: None,
            catalog_request: None,
            download: None,
            confirm: None,
            picked: Arc::new(Mutex::new(Vec::new())),
            tab: Tab::Installed,
            message: None,
            dirty: false,
            dropped: Vec::new(),
            run: None,
        }
    }
}

/// Имя файла пакета аддина id.
pub fn file_name(id: &str) -> String {
    format!("{id}.pxaddin")
}

impl Addins {
    /// Список установленных в текст хранилища: id, включён, из файла.
    pub fn index(&self) -> String {
        let list: Vec<Value> = self.installed.iter().map(|i| serde_json::json!({"id": i.package.manifest.id, "enabled": i.enabled, "file": i.from_file})).collect();
        serde_json::json!({"addins": list, "catalog": self.catalog_url}).to_string()
    }

    /// Восстановить список: read даёт байты пакета по id.
    pub fn restore(&mut self, text: &str, read: &dyn Fn(&str) -> Option<Vec<u8>>) -> Vec<String> {
        let mut lost = Vec::new();
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return lost;
        };
        if let Some(url) = v.get("catalog").and_then(Value::as_str).filter(|u| !u.is_empty()) {
            self.catalog_url = url.to_string();
        }
        for a in v.get("addins").and_then(Value::as_array).map_or(&[][..], Vec::as_slice) {
            let Some(id) = a.get("id").and_then(Value::as_str) else {
                continue;
            };
            match read(id).ok_or_else(|| "нет файла".to_string()).and_then(|b| Package::read(&b)) {
                Ok(package) => self.installed.push(Installed {
                    package,
                    enabled: a.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                    from_file: a.get("file").and_then(Value::as_bool).unwrap_or(false),
                    addin: None,
                }),
                Err(e) => lost.push(format!("{id}: {e}")),
            }
        }
        self.sync_menu();
        lost
    }

    /// Обновить команды меню по включённым аддинам.
    pub fn sync_menu(&self) {
        let items = self
            .installed
            .iter()
            .filter(|i| i.enabled)
            .flat_map(|i| {
                i.package.manifest.commands.iter().filter(|c| c.at("layer") || c.at("object")).map(|c| MenuItem {
                    addin: i.package.manifest.id.clone(),
                    command: c.id.clone(),
                    title: c.title.clone(),
                    kinds: c.kinds.clone(),
                })
            })
            .collect();
        if let Ok(mut m) = MENU.write() {
            *m = items;
        }
    }

    /// Поставить проверенный пакет: новая версия заменяет старую.
    pub fn install(&mut self, package: Package, from_file: bool) {
        let id = package.manifest.id.clone();
        self.installed.retain(|i| i.package.manifest.id != id);
        self.installed.push(Installed { package, enabled: true, from_file, addin: None });
        self.installed.sort_by(|a, b| a.package.manifest.id.cmp(&b.package.manifest.id));
        self.dirty = true;
        self.sync_menu();
    }

    /// Убрать аддин id.
    pub fn remove(&mut self, id: &str) {
        self.installed.retain(|i| i.package.manifest.id != id);
        self.dropped.push(id.to_string());
        self.dirty = true;
        self.sync_menu();
    }

    /// Предложить установку пакета из байтов файла name.
    pub fn offer_file(&mut self, lang: Lang, name: &str, bytes: &[u8]) {
        match Package::read(bytes) {
            Ok(p) => {
                self.confirm = Some((p, true));
                self.open = true;
            }
            Err(e) => self.message = Some(trf(lang, "The add-in {name} is not installed: {e}", &[("name", name), ("e", &e)])),
        }
    }

    /// Выполнить команду аддина над объектами.
    pub fn run_command(&mut self, addin: &str, command: &str, params: &Value, features: &[addin_api::Feature]) -> Result<Vec<addin_api::Feature>, String> {
        let i = self.installed.iter_mut().find(|i| i.package.manifest.id == addin && i.enabled).ok_or("аддин не установлен или выключен")?;
        if i.addin.is_none() {
            i.addin = Some(Addin::load(&i.package.module)?);
        }
        let a = i.addin.as_mut().ok_or("аддин не загружен")?;
        let out = a.run_command(command, params, features);
        // После ловушки модуль может быть в плохом состоянии: следующая
        // команда загрузит его заново.
        if out.is_err() {
            i.addin = None;
        }
        out
    }

    /// Описание установленного аддина id.
    pub fn manifest(&self, id: &str) -> Option<&Manifest> {
        self.installed.iter().find(|i| i.package.manifest.id == id).map(|i| &i.package.manifest)
    }

    /// Открыть окно параметров команды над слоем.
    pub fn ask_params(&mut self, layer: u64, layer_title: &str, count: usize, addin: &str, command: &str) {
        let Some(c) = self.manifest(addin).and_then(|m| m.commands.iter().find(|c| c.id == command)) else {
            return;
        };
        let values = c.params.iter().map(|p| (p.id.clone(), p.default_value())).collect();
        self.run = Some(Run { layer, layer_title: layer_title.to_string(), count, addin: addin.to_string(), command: command.to_string(), values });
    }

    /// Ответы сети: каталог и пакет.
    fn poll(&mut self, lang: Lang) {
        if let Some(r) = &self.catalog_request
            && let Some(result) = r.take()
        {
            self.catalog_request = None;
            match result.map_err(|e| e.to_string()).and_then(|b| parse_catalog(&String::from_utf8_lossy(&b), &self.catalog_url)) {
                Ok(c) => {
                    // Снятый аддин выключается.
                    for i in &mut self.installed {
                        if i.enabled && c.blocks(&i.package.manifest.id, &i.package.manifest.version) {
                            i.enabled = false;
                            self.dirty = true;
                            self.message = Some(trf(lang, "The add-in {name} is withdrawn by the catalogue and turned off.", &[("name", i.package.manifest.title.get(ru(lang)))]));
                        }
                    }
                    self.sync_menu();
                    self.catalog = Some(c);
                    self.catalog_error = None;
                }
                Err(e) => self.catalog_error = Some(e),
            }
        }
        if let Some((entry, r)) = &self.download
            && let Some(result) = r.take()
        {
            let entry = entry.clone();
            self.download = None;
            let checked = result.map_err(|e| e.to_string()).and_then(|b| {
                let sum = sha256(&b);
                if sum != entry.sha256 {
                    return Err(format!("SHA-256 пакета {sum} не совпадает с каталогом {}", entry.sha256));
                }
                let p = Package::read(&b)?;
                if p.manifest.id != entry.id || p.manifest.version != entry.version {
                    return Err("id или версия пакета не совпадают с каталогом".into());
                }
                Ok(p)
            });
            match checked {
                Ok(p) => {
                    let name = p.manifest.title.get(ru(lang)).to_string();
                    self.install(p, false);
                    self.message = Some(trf(lang, "The add-in {name} is installed.", &[("name", &name)]));
                }
                Err(e) => self.message = Some(trf(lang, "The add-in {name} is not installed: {e}", &[("name", &entry.id), ("e", &e)])),
            }
        }
        let picked = self.picked.lock().map(|mut p| std::mem::take(&mut *p)).unwrap_or_default();
        for (name, bytes) in picked {
            self.offer_file(lang, &name, &bytes);
        }
    }

    /// Запросить каталог.
    pub fn refresh(&mut self, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        let r = request(CATALOG_BYTES);
        r.start(&self.catalog_url);
        self.catalog_request = Some(r);
    }

    fn pick_file(&self, ctx: &egui::Context) {
        let title = "PlanetX Lite add-in (*.pxaddin)";
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            if let Some(path) = rfd::FileDialog::new().add_filter(title, &["pxaddin"]).pick_file() {
                let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
                let bytes = std::fs::read(&path).unwrap_or_default();
                if let Ok(mut p) = self.picked.lock() {
                    p.push((name, bytes));
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let (slot, ctx) = (self.picked.clone(), ctx.clone());
            wasm_bindgen_futures::spawn_local(async move {
                if let Some(file) = rfd::AsyncFileDialog::new().add_filter(title, &["pxaddin"]).pick_file().await {
                    let bytes = file.read().await;
                    if let Ok(mut s) = slot.lock() {
                        s.push((file.file_name(), bytes));
                    }
                }
                ctx.request_repaint();
            });
        }
    }

    /// Окно «Аддины» и окно параметров команды. Ответ - что выполнить.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Option<Ask> {
        self.poll(lang);
        let asked = self.params_window(ctx, lang);
        if self.confirm.is_some() {
            self.confirm_window(ctx, lang);
        }
        if !self.open {
            return asked;
        }
        // Каталог запрашивается при первом открытии окна.
        if self.catalog.is_none() && self.catalog_request.is_none() && self.catalog_error.is_none() {
            self.refresh(request);
        }
        let mut open = self.open;
        egui::Window::new(tr(lang, "Add-ins")).open(&mut open).default_width(460.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Installed, tr(lang, "Installed"));
                ui.selectable_value(&mut self.tab, Tab::Catalog, tr(lang, "Catalogue"));
                if ui.button(tr(lang, "From file…")).on_hover_text(tr(lang, "Install an add-in from a .pxaddin file. Such an add-in is not checked by the catalogue.")).clicked() {
                    self.pick_file(ctx);
                }
            });
            ui.separator();
            match self.tab {
                Tab::Installed => {
                    if let Some(e) = self.installed_tab(ui, lang) {
                        self.start_download(e, request);
                    }
                }
                Tab::Catalog => self.catalog_tab(ui, lang, request),
            }
            if let Some(m) = &self.message {
                ui.separator();
                ui.label(m.as_str());
            }
        });
        self.open = open;
        asked
    }

    /// Вкладка установленных. Ответ - строка каталога для обновления.
    fn installed_tab(&mut self, ui: &mut egui::Ui, lang: Lang) -> Option<Entry> {
        if self.installed.is_empty() {
            ui.label(tr(lang, "No add-ins installed. The catalogue tab lists the checked add-ins."));
            return None;
        }
        let updates: Vec<(String, Entry)> = self
            .catalog
            .iter()
            .flat_map(|c| c.entries.iter())
            .filter(|e| self.installed.iter().any(|i| i.package.manifest.id == e.id && newer(&e.version, &i.package.manifest.version)))
            .map(|e| (e.id.clone(), e.clone()))
            .collect();
        let (mut remove, mut update, mut changed) = (None, None, false);
        for i in &mut self.installed {
            let m = &i.package.manifest;
            ui.horizontal(|ui| {
                if ui.checkbox(&mut i.enabled, egui::RichText::new(format!("{} {}", m.title.get(ru(lang)), m.version)).strong()).on_hover_text(tr(lang, "The check turns the add-in on or off.")).changed() {
                    changed = true;
                }
                if let Some((_, e)) = updates.iter().find(|(id, _)| *id == m.id)
                    && ui.button(trf(lang, "Update to {version}", &[("version", &e.version)])).clicked()
                {
                    update = Some(e.clone());
                }
                if ui.button(tr(lang, "Remove|add-in")).clicked() {
                    remove = Some(m.id.clone());
                }
            });
            let d = m.description.get(ru(lang));
            if !d.is_empty() {
                ui.label(d);
            }
            let source = if i.from_file { tr(lang, "from a file, not checked by the catalogue") } else { tr(lang, "from the catalogue") };
            ui.label(egui::RichText::new(format!("{} · {} · {source}", m.author, m.license)).color(crate::theme::pal().muted));
            ui.separator();
        }
        if changed {
            self.dirty = true;
            self.sync_menu();
        }
        if let Some(id) = remove {
            self.remove(&id);
        }
        update
    }

    /// Скачать пакет строки каталога.
    fn start_download(&mut self, e: Entry, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        self.message = None;
        let r = request(PACKAGE_BYTES);
        r.start(&e.url);
        self.download = Some((e, r));
    }

    fn catalog_tab(&mut self, ui: &mut egui::Ui, lang: Lang, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        ui.horizontal(|ui| {
            ui.label(tr(lang, "Catalogue address"));
            let edit = ui.add(egui::TextEdit::singleline(&mut self.catalog_url).desired_width(260.0));
            if edit.changed() {
                self.dirty = true;
            }
            if ui.button(tr(lang, "Refresh|catalogue")).clicked() {
                self.catalog_error = None;
                self.refresh(request);
            }
        });
        if self.catalog_request.is_some() {
            ui.label(tr(lang, "Loading the catalogue…"));
            return;
        }
        if let Some(e) = &self.catalog_error {
            ui.label(trf(lang, "The catalogue is not loaded: {e}", &[("e", e)]));
            return;
        }
        let Some(c) = &self.catalog else {
            return;
        };
        let mut get = None;
        for e in &c.entries {
            let have = self.installed.iter().find(|i| i.package.manifest.id == e.id).map(|i| i.package.manifest.version.clone());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{} {}", e.title.get(ru(lang)), e.version)).strong());
                let label = match &have {
                    None => Some(tr(lang, "Install")),
                    Some(v) if newer(&e.version, v) => Some(tr(lang, "Update|add-in")),
                    Some(_) => None,
                };
                match label {
                    Some(l) => {
                        if ui.add_enabled(self.download.is_none(), egui::Button::new(l)).clicked() {
                            get = Some(e.clone());
                        }
                    }
                    None => {
                        ui.label(tr(lang, "installed"));
                    }
                }
            });
            ui.label(e.description.get(ru(lang)));
            ui.label(egui::RichText::new(format!("{} · {} · {}", e.author, e.license, rights_text(lang, &e.permissions))).color(crate::theme::pal().muted));
            ui.separator();
        }
        if c.entries.is_empty() {
            ui.label(tr(lang, "The catalogue is empty."));
        }
        if self.download.is_some() {
            ui.label(tr(lang, "Downloading the add-in…"));
        }
        if let Some(e) = get {
            self.start_download(e, request);
        }
    }

    /// Подтверждение установки: права и источник.
    fn confirm_window(&mut self, ctx: &egui::Context, lang: Lang) {
        let Some((p, from_file)) = &self.confirm else {
            return;
        };
        let m = &p.manifest;
        let (mut yes, mut no) = (false, false);
        // По центру экрана, иначе окно ложится под окно «Аддины».
        egui::Window::new(tr(lang, "Install the add-in?")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(egui::RichText::new(format!("{} {}", m.title.get(ru(lang)), m.version)).strong());
            ui.label(m.description.get(ru(lang)));
            ui.label(format!("{} · {}", m.author, m.license));
            let keys: Vec<String> = m.permissions.iter().map(|p| p.key().to_string()).collect();
            ui.label(rights_text(lang, &keys));
            if *from_file {
                ui.colored_label(egui::Color32::from_rgb(230, 160, 40), tr(lang, "The add-in comes from a file, the catalogue did not check it. Install add-ins only from authors you trust."));
            }
            ui.horizontal(|ui| {
                yes = ui.button(tr(lang, "Install")).clicked();
                no = ui.button(tr(lang, "Cancel")).clicked();
            });
        });
        if yes && let Some((p, from_file)) = self.confirm.take() {
            let name = p.manifest.title.get(ru(lang)).to_string();
            self.install(p, from_file);
            self.message = Some(trf(lang, "The add-in {name} is installed.", &[("name", &name)]));
        } else if no {
            self.confirm = None;
        }
    }

    /// Окно параметров команды. Ответ - команда к выполнению.
    fn params_window(&mut self, ctx: &egui::Context, lang: Lang) -> Option<Ask> {
        let run = self.run.as_mut()?;
        let command = self.installed.iter().find(|i| i.package.manifest.id == run.addin).and_then(|i| i.package.manifest.commands.iter().find(|c| c.id == run.command)).cloned();
        let Some(command) = command else {
            self.run = None;
            return None;
        };
        let (mut go, mut cancel) = (false, false);
        egui::Window::new(command.title.get(ru(lang)).trim_end_matches('…').to_string()).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(trf(lang, "Layer {layer}, objects: {count}.", &[("layer", &run.layer_title), ("count", &run.count.to_string())]));
            egui::Grid::new("addin-params").num_columns(2).show(ui, |ui| {
                for p in &command.params {
                    ui.label(p.title.get(ru(lang)));
                    let v = run.values.entry(p.id.clone()).or_insert_with(|| p.default_value());
                    match &p.kind {
                        ParamKind::Number { unit, min, max, .. } => {
                            let mut x = v.as_f64().unwrap_or(0.0);
                            if ui.add(egui::DragValue::new(&mut x).range(*min..=*max).suffix(format!(" {unit}"))).changed() {
                                *v = Value::from(x);
                            }
                        }
                        ParamKind::Text { .. } => {
                            let mut s = v.as_str().unwrap_or_default().to_string();
                            if ui.text_edit_singleline(&mut s).changed() {
                                *v = Value::from(s);
                            }
                        }
                        ParamKind::Bool { .. } => {
                            let mut b = v.as_bool().unwrap_or(false);
                            if ui.checkbox(&mut b, "").changed() {
                                *v = Value::from(b);
                            }
                        }
                        ParamKind::Choice { options, .. } => {
                            let now = v.as_str().unwrap_or_default().to_string();
                            let shown = options.iter().find(|o| o.0 == now).map_or(now.clone(), |o| o.1.get(ru(lang)).to_string());
                            egui::ComboBox::from_id_salt(("addin-choice", &p.id)).selected_text(shown).show_ui(ui, |ui| {
                                for (key, title) in options {
                                    if ui.selectable_label(*key == now, title.get(ru(lang))).clicked() {
                                        *v = Value::from(key.clone());
                                    }
                                }
                            });
                        }
                    }
                    ui.end_row();
                }
            });
            ui.horizontal(|ui| {
                go = ui.button(tr(lang, "Run|add-in")).clicked();
                cancel = ui.button(tr(lang, "Cancel")).clicked();
            });
        });
        if cancel {
            self.run = None;
        }
        if go { self.run.take().map(Ask::Run) } else { None }
    }
}

/// Права текстом для окна.
fn rights_text(lang: Lang, keys: &[String]) -> String {
    if keys.is_empty() {
        return tr(lang, "Rights: none").to_string();
    }
    let names: Vec<&str> = keys
        .iter()
        .map(|k| match k.as_str() {
            "features" => tr(lang, "get the objects chosen for a command"),
            "layers" => tr(lang, "add layers to «My layers»"),
            other => other,
        })
        .collect();
    trf(lang, "Rights: {rights}", &[("rights", &names.join(", "))])
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{newer, parse_catalog, resolve, sha256};

    #[test]
    fn catalog_and_versions() {
        let text = r#"{"addins":[{"id":"buffer","version":"0.1.0","title":{"ru":"Буфер","en":"Buffer"},"package":"packages/buffer-0.1.0.pxaddin","sha256":"AB","permissions":["features"]},
            {"id":"good","version":"1.0.0","package":"https://x.org/g.pxaddin","sha256":"0000000000000000000000000000000000000000000000000000000000000000"}],"blocked":["evil","good@0.9.0"]}"#;
        let c = parse_catalog(text, "https://valery35.github.io/planetx-lite-addins/registry.json").unwrap();
        // Строка без верной суммы пропускается.
        assert_eq!(c.entries.len(), 1);
        assert_eq!(c.entries[0].url, "https://x.org/g.pxaddin");
        assert!(c.blocks("evil", "1.0") && c.blocks("good", "0.9.0") && !c.blocks("good", "1.0.0"));
        assert_eq!(resolve("https://a.io/x/registry.json", "packages/b.pxaddin"), "https://a.io/x/packages/b.pxaddin");
        assert!(newer("0.10.0", "0.9.1") && !newer("1.0.0", "1.0.0"));
        assert_eq!(sha256(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert!(parse_catalog("x", "").is_err());
    }
}
