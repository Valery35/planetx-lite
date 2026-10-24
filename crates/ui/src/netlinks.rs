// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сетевые ссылки KML: окно ссылки и загрузка содержимого. Перенос
//! `ui/netlink.py` (`LinkDialog`, `NetLinks`) и `core/netlink.py`
//! модуля.
//!
//! Ссылка - папка «Моих меток» с адресом. Пока папка отмечена вместе
//! с родителями, документ по адресу загружается один раз за сеанс или
//! через промежуток обновления и заменяет содержимое папки. Адрес в сети
//! идёт запросом, файл на диске в программе перечитывается, когда
//! меняется время его правки. Адреса внутри документа - вложенные ссылки
//! и картинки наложений - считаются от адреса документа, как в Google
//! Earth. Вложенные ссылки загружаются так же, не глубже [`DEPTH`].
//! Ссылки суперналожений (viewRefreshMode onRegion) здесь не грузятся.

use std::collections::HashMap;

use eframe::egui;
use scene::kml::{Folder, Item};

use crate::i18n::{Lang, tr, trf};

/// Промежуток обновления не короче стольких секунд, `MIN_REFRESH` модуля.
pub const MIN_REFRESH: f64 = 10.0;
/// Сетевые ссылки внутри загруженных документов - не глубже.
pub const DEPTH: usize = 3;
/// Секунд между проверками сроков ссылок, `TICK` модуля.
const TICK: f64 = 1.0;
/// Предел ответа документа ссылки, байты.
pub const MAX_BYTES: u64 = 64 << 20;

/// Адрес в сети: http или https.
pub fn is_web(href: &str) -> bool {
    let lower = href.trim().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Путь файла по адресу file:// или пути на диске, иначе None.
pub fn local_path(href: &str) -> Option<String> {
    let text = href.trim();
    if let Some(rest) = text.strip_prefix("file://") {
        let path = percent_decode(rest);
        // file:///C:/x.kml даёт путь /C:/x.kml.
        let b = path.as_bytes();
        if b.len() > 2 && b[0] == b'/' && b[2] == b':' {
            return Some(path[1..].to_string());
        }
        return Some(path);
    }
    let scheme = text.split_once("://").map(|(s, _)| s);
    if scheme.is_none() { Some(text.to_string()) } else { None }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Адрес href относительно адреса документа base, `resolve` модуля.
pub fn resolve(href: &str, base: &str) -> String {
    let href = href.trim();
    if href.is_empty() || is_web(href) || base.is_empty() {
        return href.to_string();
    }
    if href.to_ascii_lowercase().starts_with("file://") {
        return href.to_string();
    }
    if is_web(base) {
        // Относительный адрес - от папки документа, «/» - от корня сайта.
        let (scheme, rest) = base.split_once("://").unwrap_or(("https", base));
        let host_end = rest.find('/').unwrap_or(rest.len());
        let (host, path) = rest.split_at(host_end);
        if href.starts_with('/') {
            return format!("{scheme}://{host}{href}");
        }
        let mut dirs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if !path.ends_with('/') {
            dirs.pop();
        }
        for piece in href.split('/') {
            match piece {
                "" | "." => {}
                ".." => {
                    dirs.pop();
                }
                p => dirs.push(p),
            }
        }
        return format!("{scheme}://{host}/{}", dirs.join("/"));
    }
    let absolute = href.starts_with('/') || href.starts_with('\\') || href.as_bytes().get(1) == Some(&b':');
    if absolute {
        return href.to_string();
    }
    let file = local_path(base).unwrap_or_else(|| base.to_string());
    let folder = file.rsplit_once(['/', '\\']).map_or("", |(f, _)| f);
    if folder.is_empty() { percent_decode(href) } else { format!("{folder}/{}", percent_decode(href)) }
}

/// Адреса вложенных ссылок и картинок наложений дерева - от адреса
/// документа base, `resolve_tree` модуля.
pub fn resolve_tree(tree: &mut Folder, base: &str) {
    for child in &mut tree.children {
        match child {
            Item::Folder(f) => {
                if !f.link.is_empty() {
                    f.link = resolve(&f.link, base);
                }
                resolve_tree(f, base);
            }
            Item::Overlay(o) if !o.href.is_empty() && o.image.as_ref().is_none_or(Vec::is_empty) => o.href = resolve(&o.href, base),
            _ => {}
        }
    }
}

/// Промежуток обновления, с: 0 - один раз, иначе не короче MIN_REFRESH.
pub fn interval(refresh: f64) -> f64 {
    if refresh <= 0.0 { 0.0 } else { refresh.max(MIN_REFRESH) }
}

/// Пора ли загружать ссылку: loaded - время прошлой загрузки или None.
pub fn due(loaded: Option<f64>, refresh: f64, now: f64) -> bool {
    let Some(loaded) = loaded else {
        return true;
    };
    let step = interval(refresh);
    step > 0.0 && now - loaded >= step
}

/// Окно «Сетевая ссылка»: название, адрес, промежуток обновления.
pub struct LinkDialog {
    /// Папка-ссылка по пути, None - новая ссылка в папке parent.
    pub path: Option<Vec<usize>>,
    pub parent: Vec<usize>,
    pub name: String,
    pub link: String,
    pub refresh: f64,
}

impl LinkDialog {
    /// Окно. Some(true) - «OK», Some(false) - «Отмена», None - открыто.
    pub fn window(&mut self, ctx: &egui::Context, lang: Lang) -> Option<bool> {
        let mut result = None;
        let mut open = true;
        egui::Window::new(tr(lang, "Network link")).open(&mut open).collapsible(false).min_width(420.0).show(ctx, |ui| {
            ui.label(tr(lang, "A My Places folder with the contents of a KML or KMZ document at an address. The contents are loaded again after the refresh interval, edits inside the folder are lost then."));
            egui::Grid::new("planetx-link").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Name"));
                ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(300.0)).on_hover_text(tr(lang, "The folder name in My Places."));
                ui.end_row();
                ui.label(tr(lang, "Address"));
                ui.add(egui::TextEdit::singleline(&mut self.link).hint_text("https://…/doc.kml").desired_width(300.0))
                    .on_hover_text(tr(lang, "The address of a KML or KMZ document on the web (http, https) or a file path on disk. The file is read again when another program changes it."));
                ui.end_row();
                ui.label(tr(lang, "Refresh"));
                let once = tr(lang, "Once");
                let unit = tr(lang, " s");
                ui.add(egui::DragValue::new(&mut self.refresh).range(0.0..=7.0 * 86400.0).speed(1.0).custom_formatter(move |v, _| if v <= 0.0 { once.to_string() } else { format!("{v:.0}{unit}") }))
                    .on_hover_text(tr(lang, "After how many seconds the document is loaded again. Zero is once per session. The interval is never shorter than 10 s."));
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "OK")).clicked() {
                    result = Some(true);
                }
                if ui.button(tr(lang, "Cancel")).clicked() {
                    result = Some(false);
                }
            });
        });
        if !open {
            result = Some(false);
        }
        result
    }
}

/// Прошлая загрузка ссылки: время, адрес, время правки файла.
#[derive(Debug, Clone, PartialEq)]
struct Loaded {
    at: f64,
    link: String,
    stamp: Option<f64>,
}

/// Ключ ссылки: путь папки в дереве и адрес.
type Key = (Vec<usize>, String);
/// Чтение файла на диске: байты или ошибка.
pub type ReadFile<'a> = &'a dyn Fn(&str) -> Result<Vec<u8>, String>;

/// Загрузка содержимого сетевых ссылок «Моих меток».
#[derive(Default)]
pub struct NetLinks {
    /// Ключ папки - путь в дереве и адрес.
    loaded: HashMap<Key, Loaded>,
    asking: HashMap<Key, (Box<dyn net::Request>, f64)>,
    pub errors: HashMap<Vec<usize>, String>,
    last_tick: f64,
}

/// Пришедший документ ссылки: путь папки, адрес, дерево или ошибка.
pub type Arrived = (Vec<usize>, String, Result<Folder, String>);

impl NetLinks {
    /// «Загрузить заново» меню ссылки.
    pub fn reload(&mut self, path: &[usize]) {
        self.loaded.retain(|(p, _), _| p != path);
    }

    /// Запросы в пути.
    pub fn busy(&self) -> bool {
        !self.asking.is_empty()
    }

    /// Шаг: сроки ссылок, новые запросы, пришедшие ответы. tree - «Мои
    /// метки», request - новый запрос, read_file - байты и время правки
    /// файла на диске (в программе), now - секунды.
    pub fn step(
        &mut self,
        tree: Option<&Folder>,
        now: f64,
        request: &dyn Fn() -> Box<dyn net::Request>,
        read_file: ReadFile<'_>,
        stamp_of: &dyn Fn(&str) -> Option<f64>,
    ) -> Vec<Arrived> {
        let mut out = Vec::new();
        // Пришедшие ответы.
        let keys: Vec<Key> = self.asking.keys().cloned().collect();
        for key in keys {
            let Some((r, at)) = self.asking.get(&key) else {
                continue;
            };
            let at = *at;
            if let Some(answer) = r.take() {
                self.asking.remove(&key);
                self.loaded.insert(key.clone(), Loaded { at, link: key.1.clone(), stamp: None });
                out.push((key.0.clone(), key.1.clone(), answer.and_then(|b| parse(&b, &key.1))));
            }
        }
        if now - self.last_tick < TICK {
            return out;
        }
        self.last_tick = now;
        let Some(tree) = tree else {
            return out;
        };
        let mut links = Vec::new();
        collect(tree, &mut Vec::new(), 0, &mut links);
        for (path, link, refresh) in links {
            let key = (path.clone(), link.clone());
            if self.asking.contains_key(&key) {
                continue;
            }
            let last = self.loaded.get(&key).cloned();
            if !is_web(&link) && let Some(file) = local_path(&link) {
                // Файл на диске перечитывается, когда сменилось время его
                // правки.
                let stamp = stamp_of(&file);
                if last.as_ref().is_some_and(|l| l.stamp == stamp && stamp.is_some()) || (last.is_some() && stamp.is_none()) {
                    continue;
                }
                self.loaded.insert(key.clone(), Loaded { at: now, link: link.clone(), stamp });
                let got = read_file(&file).and_then(|b| parse(&b, &link));
                out.push((path, link, got));
                continue;
            }
            if due(last.map(|l| l.at), refresh, now) {
                let r = request();
                r.start(&link);
                self.asking.insert(key, (r, now));
            }
        }
        out
    }
}

/// Папки-ссылки дерева: путь, адрес, промежуток. Папка или её родитель
/// снят флажком - ссылка не грузится. depth - ссылок-предков.
fn collect(folder: &Folder, path: &mut Vec<usize>, depth: usize, out: &mut Vec<(Vec<usize>, String, f64)>) {
    if !folder.visible {
        return;
    }
    for (n, child) in folder.children.iter().enumerate() {
        let Item::Folder(f) = child else {
            continue;
        };
        path.push(n);
        let link = f.link.trim();
        let is_link = !link.is_empty() && f.view_mode != "onRegion";
        if is_link && depth <= DEPTH && f.visible {
            out.push((path.clone(), link.to_string(), f.refresh));
        }
        collect(f, path, depth + usize::from(is_link), out);
        path.pop();
    }
}

/// Дерево документа ссылки из байтов KML или KMZ, адреса - от адреса.
fn parse(bytes: &[u8], link: &str) -> Result<Folder, String> {
    let name = if bytes.starts_with(b"PK") { "link.kmz" } else { "link.kml" };
    let mut tree = crate::marks::open(bytes, name)?.tree;
    resolve_tree(&mut tree, link);
    Ok(tree)
}

/// Текст ошибки ссылки для панели.
pub fn error_text(lang: Lang, name: &str, error: &str) -> String {
    trf(lang, "The network link {name} was not loaded, {error}", &[("name", name), ("error", if error.is_empty() { "?" } else { error })])
}

#[cfg(test)]
mod tests {
    use super::{due, interval, is_web, local_path, resolve};

    #[test]
    fn addresses() {
        assert!(is_web("HTTPS://x.org/a.kml"));
        assert!(!is_web("C:\\data\\a.kml"));
        assert_eq!(local_path("file:///C:/data/a%20b.kml").as_deref(), Some("C:/data/a b.kml"));
        assert_eq!(local_path("C:\\data\\a.kml").as_deref(), Some("C:\\data\\a.kml"));
        assert_eq!(local_path("https://x.org/a.kml"), None);
        assert_eq!(resolve("b.kml", "https://x.org/dir/a.kml"), "https://x.org/dir/b.kml");
        assert_eq!(resolve("../img/p.png", "https://x.org/dir/sub/a.kml"), "https://x.org/dir/img/p.png");
        assert_eq!(resolve("https://y.org/c.kml", "https://x.org/a.kml"), "https://y.org/c.kml");
        assert_eq!(resolve("p.png", "C:/data/a.kml"), "C:/data/p.png");
    }

    #[test]
    fn refresh_times() {
        assert_eq!(interval(0.0), 0.0);
        assert_eq!(interval(4.0), 10.0);
        assert!(due(None, 0.0, 5.0));
        assert!(!due(Some(1.0), 0.0, 500.0));
        assert!(!due(Some(1.0), 30.0, 20.0));
        assert!(due(Some(1.0), 30.0, 31.0));
    }
}
