// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! «Мои метки» между запусками, как у модуля: дерево пишется KMZ
//! (`scene::kml::write_kmz`) с картинками и моделями внутри и лежит
//! в хранилище eframe строкой base64 - в программе файлом в папке
//! данных, на странице в localStorage. У localStorage предел около 5 МБ:
//! больше - на странице не сохраняется, об этом говорит строка
//! сообщения.

use scene::kml::{self, Folder};

/// Ключ хранилища.
pub const KEY: &str = "planetx_my_places_kmz";

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Байты в base64 с выравниванием «=».
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for k in 0..4 {
            if k <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> (18 - 6 * k)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Байты из base64. None - чужие знаки или длина не кратна 4.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let text = text.trim().as_bytes();
    if !text.len().is_multiple_of(4) {
        return None;
    }
    let value = |c: u8| ALPHABET.iter().position(|a| *a == c).map(|p| p as u32);
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    for chunk in text.chunks(4) {
        let pad = chunk.iter().rev().take_while(|c| **c == b'=').count();
        if pad > 2 {
            return None;
        }
        let mut n = 0u32;
        for (k, c) in chunk.iter().enumerate() {
            let v = if k >= 4 - pad { 0 } else { value(*c)? };
            n = (n << 6) | v;
        }
        let three = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&three[..3 - pad]);
    }
    Some(out)
}

/// Строка хранилища по дереву «Моих меток».
pub fn to_text(root: &Folder) -> Result<String, String> {
    kml::write_kmz(root).map(|kmz| encode(&kmz)).map_err(|e| format!("{e:?}"))
}

/// Дерево «Моих меток» из строки хранилища. Строка считается
/// враждебной: ошибка разбора - None, программа начинает с пустых меток.
pub fn from_text(text: &str) -> Option<Folder> {
    let bytes = decode(text)?;
    kml::read_file(&bytes, "places.kmz").ok()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{decode, encode, from_text, to_text};
    use scene::features::Kind;
    use scene::kml::{Folder, Item, Place};

    #[test]
    fn base64_round_trip() {
        for data in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar", &[0, 255, 128, 7]] {
            assert_eq!(decode(&encode(data)).unwrap(), data);
        }
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert!(decode("Zm8").is_none());
        assert!(decode("Z!8=").is_none());
        assert!(decode("====").is_none());
    }

    #[test]
    fn my_places_survive_a_restart() {
        let mut root = Folder::new("Мои метки");
        let mut scene = Folder::new("Сцена");
        scene.children.push(Item::Place(Place::new("Пермь", Kind::Point, vec![(58.0, 56.25)])));
        let mut hidden = Place::new("Скрытая", Kind::Point, vec![(1.0, 2.0)]);
        hidden.visible = false;
        scene.children.push(Item::Place(hidden));
        root.children.push(Item::Folder(scene));
        let back = from_text(&to_text(&root).unwrap()).unwrap();
        assert_eq!(back.name, "Мои метки");
        let Some(Item::Folder(f)) = back.children.first() else { panic!("нет папки") };
        assert_eq!(f.name, "Сцена");
        let names: Vec<(String, bool)> = f
            .children
            .iter()
            .filter_map(|i| match i {
                Item::Place(p) => Some((p.name.clone(), p.visible)),
                _ => None,
            })
            .collect();
        assert_eq!(names, [("Пермь".to_string(), true), ("Скрытая".to_string(), false)]);
        assert!(from_text("мусор").is_none());
    }
}
