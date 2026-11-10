// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сохранённое окно. eframe пишет в `app.ron` и окно, закрытое свёрнутым:
//! у Windows такое окно стоит в точке -32000 и имеет размер 0 × 0.
//! При следующем запуске программа открывалась окном 116 × 64 точки
//! в углу экрана, самопроверки снимали кадр 232 × 128. Запись такого окна
//! убирается до запуска, окно открывается обычного размера.

use std::path::Path;

/// Меньше стольких точек по стороне окно считается испорченным.
const MIN_SIDE: f64 = 200.0;
/// Левее и выше стольких пикселей окно за краем всех экранов: так Windows
/// ставит свёрнутое окно.
const OFF_SCREEN: f64 = -10_000.0;
/// Имя файла хранилища eframe.
const FILE: &str = "app.ron";

/// Число после метки key в тексте записи окна.
fn number_after(text: &str, key: &str) -> Option<f64> {
    let start = text.find(key)? + key.len();
    let rest = text.get(start..)?;
    let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(rest.len());
    rest.get(..end)?.parse().ok()
}

/// Испорчена ли запись окна: свёрнуто, за краем экранов или крошечное.
fn broken(entry: &str) -> bool {
    let size = entry.find("inner_size_points").and_then(|i| entry.get(i..)).map_or((None, None), |s| (number_after(s, "x:"), number_after(s, "y:")));
    let place = entry.find("outer_position_pixels").and_then(|i| entry.get(i..)).map_or((None, None), |s| (number_after(s, "x:"), number_after(s, "y:")));
    let tiny = [size.0, size.1].into_iter().flatten().any(|v| v < MIN_SIDE);
    let away = [place.0, place.1].into_iter().flatten().any(|v| v <= OFF_SCREEN);
    tiny || away
}

/// Текст хранилища без испорченной записи окна. None - менять нечего.
pub fn without_broken_window(text: &str) -> Option<String> {
    let mut changed = false;
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| {
            let drop = line.trim_start().starts_with("\"window\"") && broken(line);
            changed |= drop;
            !drop
        })
        .collect();
    changed.then(|| lines.join("\n") + if text.ends_with('\n') { "\n" } else { "" })
}

/// Убрать испорченную запись окна из хранилища в папке dir. Ошибки
/// чтения и записи не мешают запуску.
pub fn fix_saved_window(dir: &Path) {
    let path = dir.join(FILE);
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Some(fixed) = without_broken_window(&text)
        && let Err(e) = std::fs::write(&path, fixed)
    {
        eprintln!("{}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::without_broken_window;

    const MINIMIZED: &str = "    \"window\": \"(normal_geometry:(outer_position_pixels:Some((x:-32000.0,y:-32000.0)),scale_factor:2.0,inner_size_points:(x:0.0,y:0.0)),fullscreen:false,maximized:false)\",";
    const TINY: &str = "    \"window\": \"(normal_geometry:(outer_position_pixels:Some((x:0.0,y:0.0)),scale_factor:2.0,inner_size_points:(x:116.0,y:64.0)),fullscreen:false,maximized:false)\",";
    const GOOD: &str = "    \"window\": \"(normal_geometry:(outer_position_pixels:Some((x:40.0,y:-8.0)),scale_factor:2.0,inner_size_points:(x:1100.0,y:640.0)),fullscreen:false,maximized:true)\",";

    fn store(window: &str) -> String {
        format!("{{\n    \"planetx_settings\": \"scale=1\\n\",\n{window}\n    \"egui\": \"()\",\n}}\n")
    }

    #[test]
    fn minimized_and_tiny_windows_are_dropped() {
        for bad in [MINIMIZED, TINY] {
            let fixed = without_broken_window(&store(bad)).unwrap();
            assert!(!fixed.contains("\"window\""), "{fixed}");
            assert!(fixed.contains("planetx_settings") && fixed.contains("\"egui\"") && fixed.ends_with("}\n"));
        }
        // Обычное окно, в том числе чуть за краем у развёрнутого, остаётся.
        assert_eq!(without_broken_window(&store(GOOD)), None);
        assert_eq!(without_broken_window("{\n}\n"), None);
    }
}
