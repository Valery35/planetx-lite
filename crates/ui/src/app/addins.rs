// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Аддины программы: список в хранилище, пакеты в программе - файлами
//! `addins/<id>.pxaddin` папки данных, на странице - записями хранилища
//! браузера `addin:<id>` в Base64.

use eframe::egui;

use super::LiteApp;

/// Ключ списка аддинов в хранилище.
pub(super) const ADDINS_KEY: &str = "addins";

/// Base64 без переносов, RFC 4648.
#[cfg(any(target_arch = "wasm32", test))]
pub(super) fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = c.iter().enumerate().fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= c.len() {
                out.push(char::from(A[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Байты из Base64, None - испорченный текст.
#[cfg(any(target_arch = "wasm32", test))]
pub(super) fn unbase64(text: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(u32::from(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        }))
    };
    let t = text.trim().trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(t.len() * 3 / 4);
    for c in t.chunks(4) {
        if c.len() == 1 {
            return None;
        }
        let mut n = 0u32;
        for (i, b) in c.iter().enumerate() {
            n |= val(*b)? << (18 - 6 * i);
        }
        for i in 0..c.len() - 1 {
            out.push((n >> (16 - 8 * i) & 0xFF) as u8);
        }
    }
    Some(out)
}

/// Страница: пакеты аддинов из хранилища браузера при запуске.
#[cfg(target_arch = "wasm32")]
pub(super) fn addins_from_storage(storage: Option<&dyn eframe::Storage>, index: Option<&str>) -> Vec<(String, Vec<u8>)> {
    let (Some(s), Some(index)) = (storage, index) else {
        return Vec::new();
    };
    let ids: Vec<String> = serde_json::from_str::<serde_json::Value>(index)
        .ok()
        .and_then(|v| v.get("addins").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.get("id").and_then(|i| i.as_str()).map(str::to_string)).collect()))
        .unwrap_or_default();
    ids.into_iter().filter_map(|id| s.get_string(&format!("addin:{id}")).and_then(|t| unbase64(&t)).map(|b| (id, b))).collect()
}

impl LiteApp {
    /// Аддины прошлого запуска - в окно, первым кадром.
    #[cfg(feature = "addins")]
    pub(super) fn restore_addins(&mut self) {
        let Some(text) = self.addins_saved.take() else {
            return;
        };
        #[cfg(not(target_arch = "wasm32"))]
        let lost = {
            let dir = self.user_dir.as_ref().map(|d| d.join(crate::addins::DIR));
            self.addins.restore(&text, &|id| dir.as_ref().and_then(|d| std::fs::read(d.join(crate::addins::file_name(id))).ok()))
        };
        #[cfg(target_arch = "wasm32")]
        let lost = {
            let store = std::mem::take(&mut self.addins_store);
            self.addins.restore(&text, &|id| store.iter().find(|(i, _)| i == id).map(|(_, b)| b.clone()))
        };
        if !lost.is_empty() {
            self.open_message = Some(crate::i18n::trf(self.lang, "Add-ins not loaded: {names}", &[("names", &lost.join("; "))]));
        }
    }

    #[cfg(not(feature = "addins"))]
    pub(super) fn restore_addins(&mut self) {
        self.addins_saved = None;
    }

    /// Список аддинов и их пакеты - в хранилище, если изменились.
    #[cfg(feature = "addins")]
    pub(super) fn save_addins(&mut self, storage: &mut dyn eframe::Storage) {
        // Список прошлого запуска ещё не прочитан - не затирать его.
        if !self.addins.dirty || self.addins_saved.is_some() {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(dir) = self.user_dir.as_ref().map(|d| d.join(crate::addins::DIR)) else {
                return;
            };
            if let Err(e) = std::fs::create_dir_all(&dir) {
                self.open_message = Some(format!("{}: {e}", dir.display()));
                return;
            }
            for i in &self.addins.installed {
                let path = dir.join(crate::addins::file_name(&i.package.manifest.id));
                match i.package.write() {
                    Ok(bytes) => {
                        if std::fs::read(&path).ok().as_deref() != Some(&bytes[..])
                            && let Err(e) = std::fs::write(&path, bytes)
                        {
                            self.open_message = Some(format!("{}: {e}", path.display()));
                        }
                    }
                    Err(e) => self.open_message = Some(e),
                }
            }
            for id in std::mem::take(&mut self.addins.dropped) {
                if !self.addins.installed.iter().any(|i| i.package.manifest.id == id) {
                    // Файла уже нет - удалять нечего.
                    let _ = std::fs::remove_file(dir.join(crate::addins::file_name(&id)));
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            for i in &self.addins.installed {
                if let Ok(bytes) = i.package.write() {
                    storage.set_string(&format!("addin:{}", i.package.manifest.id), base64(&bytes));
                }
            }
            for id in std::mem::take(&mut self.addins.dropped) {
                if !self.addins.installed.iter().any(|i| i.package.manifest.id == id) {
                    storage.set_string(&format!("addin:{id}"), String::new());
                }
            }
        }
        storage.set_string(ADDINS_KEY, self.addins.index());
        self.addins.dirty = false;
    }

    #[cfg(not(feature = "addins"))]
    pub(super) fn save_addins(&mut self, _storage: &mut dyn eframe::Storage) {}

    /// Окно «Аддины» и команда, которую оно попросило.
    #[cfg(feature = "addins")]
    pub(super) fn show_addins(&mut self, ctx: &egui::Context) {
        let loaders = &self.loaders;
        let request = |limit: u64| (loaders.request)(limit);
        if let Some(crate::addins::Ask::Run(run)) = self.addins.show(ctx, self.lang, &request) {
            self.run_addin(run);
        }
        // Окно закрыто - сообщение идёт в строку сообщений программы.
        if !self.addins.open
            && let Some(m) = self.addins.message.take()
        {
            self.open_message = Some(m);
        }
    }

    #[cfg(not(feature = "addins"))]
    pub(super) fn show_addins(&mut self, _ctx: &egui::Context) {}
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{base64, unbase64};

    #[test]
    fn base64_round_trip() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        for n in 0..300usize {
            let b: Vec<u8> = (0..n).map(|i| (i * 7 + 3) as u8).collect();
            assert_eq!(unbase64(&base64(&b)).unwrap(), b);
        }
        assert!(unbase64("T").is_none());
        assert!(unbase64("T!==").is_none());
    }
}
