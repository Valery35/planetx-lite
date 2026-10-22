// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Чтение моделей Wavefront OBJ с материалами MTL, перенос `core/obj.py`
//! модуля.
//!
//! Из файла берутся вершины v, текстурные координаты vt, нормали vn
//! и грани f - многоугольник режется веером, номера бывают
//! и отрицательными, от конца списка. Материал - usemtl, из файлов
//! mtllib берутся цвет Kd, непрозрачность d или Tr и картинка map_Kd.
//! Прочие строки (линии l, точки p, группы) пропускаются.
//!
//! Единицы в OBJ не записаны, считаются метрами. Ось вверх тоже не
//! записана. Blender и большинство программ пишут ось Y вверх, как
//! glTF, - так файл и читается ([`from_y_up`]). Текстурная координата
//! v переворачивается: в OBJ начало внизу картинки.

use std::collections::HashMap;

use globe_core::{DVec2, DVec3};

use crate::models::{GRAY, ModelError, Part, error, from_y_up};

/// Многоугольник длиннее пропускается: защита от испорченной строки.
pub const MAX_POLYGON: usize = 4096;

/// Материал MTL: картинка или None и цвет RGBA 0-1.
pub type Material = (Option<String>, [f64; 4]);

/// Строки текста без пробелов по краям.
fn lines(data: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(data).split(['\n', '\r']).map(|l| l.trim().to_string()).collect()
}

/// Остаток строки после первого слова: имя файла может быть
/// с пробелами.
fn rest(line: &str) -> &str {
    line.trim_start().split_once(char::is_whitespace).map_or("", |(_, r)| r.trim())
}

/// Имя картинки map_Kd без ключей вида -s 1 1 1.
fn texture_name(rest: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut skip = 0;
    for word in rest.split_whitespace() {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        if word.starts_with('-') && out.is_empty() {
            // -s, -o, -t берут три числа, -mm - два, прочие - одно.
            skip = match word {
                "-s" | "-o" | "-t" => 3,
                "-mm" => 2,
                _ => 1,
            };
            continue;
        }
        out.push(word);
    }
    out.join(" ").replace('\\', "/")
}

/// Первые count чисел строки или None, если их нет или запись неверна -
/// такая строка MTL пропускается.
fn floats(text: &str, count: usize) -> Option<Vec<f64>> {
    let words: Vec<&str> = text.split_whitespace().take(count).collect();
    if words.len() < count {
        return None;
    }
    words.iter().map(|w| w.parse::<f64>().ok()).collect()
}

/// Материалы файла MTL по порядку первого появления имени: имя
/// и материал. Повтор имени заменяет материал.
pub fn read_mtl(data: &[u8]) -> Vec<(String, Material)> {
    let mut out: Vec<(String, Material)> = Vec::new();
    let mut name: Option<String> = None;
    let mut color = [GRAY[0], GRAY[1], GRAY[2]];
    let mut alpha = 1.0;
    let mut image: Option<String> = None;
    let close = |out: &mut Vec<(String, Material)>, name: Option<String>, color: [f64; 3], alpha: f64, image: Option<String>| {
        if let Some(name) = name {
            let material = (image, [color[0], color[1], color[2], alpha]);
            match out.iter_mut().find(|(n, _)| *n == name) {
                Some(slot) => slot.1 = material,
                None => out.push((name, material)),
            }
        }
    };
    for line in lines(data) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let key = line.split_whitespace().next().unwrap_or_default();
        let rest = rest(&line);
        match key {
            "newmtl" => {
                close(&mut out, name.take(), color, alpha, image.take());
                name = Some(rest.to_string());
                color = [GRAY[0], GRAY[1], GRAY[2]];
                alpha = 1.0;
            }
            "Kd" => {
                if let Some(v) = floats(rest, 3) {
                    color = [v[0].clamp(0.0, 1.0), v[1].clamp(0.0, 1.0), v[2].clamp(0.0, 1.0)];
                }
            }
            "d" | "Tr" => {
                if let Some(v) = floats(rest, 1) {
                    let value = v[0].clamp(0.0, 1.0);
                    alpha = if key == "d" { value } else { 1.0 - value };
                }
            }
            "map_Kd" => {
                let found = texture_name(rest);
                image = (!found.is_empty()).then_some(found);
            }
            _ => {}
        }
    }
    close(&mut out, name, color, alpha, image);
    out
}

/// Имена файлов mtllib.
pub fn libraries(data: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in lines(data) {
        if line.starts_with("mtllib") {
            let name = rest(&line).replace('\\', "/");
            if !name.is_empty() && !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

/// Файлы, на которые ссылается OBJ: файлы MTL и их картинки.
/// fetch(путь) - байты файла рядом или None.
pub fn refs(data: &[u8], fetch: &dyn Fn(&str) -> Option<Vec<u8>>) -> Vec<String> {
    let mut out = Vec::new();
    for name in libraries(data) {
        out.push(name.clone());
        let Some(found) = fetch(&name) else {
            continue;
        };
        for (_, (image, _)) in read_mtl(&found) {
            if let Some(image) = image.filter(|i| !out.contains(i)) {
                out.push(image);
            }
        }
    }
    out
}

/// Номер из записи грани: от 1, отрицательный - от конца. Пустая
/// запись - None.
fn index(text: &str, count: usize) -> Result<Option<usize>, String> {
    if text.is_empty() {
        return Ok(None);
    }
    let value: i64 = text.parse().map_err(|e| format!("{e}"))?;
    let count = i64::try_from(count).map_err(|e| format!("{e}"))?;
    let value = if value < 0 { value + count } else { value - 1 };
    if !(0..count).contains(&value) {
        return Err("face index out of range".into());
    }
    usize::try_from(value).map(Some).map_err(|e| format!("{e}"))
}

/// Числа строки после слова, не больше count.
fn numbers(line: &str, count: usize) -> Result<Vec<f64>, String> {
    line.split_whitespace().skip(1).take(count).map(|w| w.parse::<f64>().map_err(|e| format!("{e}"))).collect()
}

/// Угол грани: номера v, vt, vn.
type Corner = (usize, Option<usize>, Option<usize>);

/// Части с осью Z вверх из OBJ. fetch(путь) - байты файла MTL рядом
/// или None. Пути картинок частей - как в MTL.
pub fn read(data: &[u8], fetch: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Vec<Part>, ModelError> {
    let mut materials: HashMap<String, Material> = HashMap::new();
    for name in libraries(data) {
        if let Some(found) = fetch(&name) {
            materials.extend(read_mtl(&found));
        }
    }
    let (mut positions, mut texcoords, mut normals): (Vec<DVec3>, Vec<DVec2>, Vec<DVec3>) = (Vec::new(), Vec::new(), Vec::new());
    // Материал и углы треугольников по порядку первого появления.
    let mut groups: Vec<(Option<String>, Vec<Corner>)> = Vec::new();
    let mut slots: HashMap<Option<String>, usize> = HashMap::new();
    let mut current: Option<String> = None;
    let mut total = 0usize;
    for (number, line) in lines(data).iter().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let key = line.split_whitespace().next().unwrap_or_default();
        let bad = |e: String| error(format!("bad OBJ line {}: {e}", number + 1));
        match key {
            "v" | "vn" => {
                let v = numbers(line, 3).map_err(bad)?;
                let [x, y, z] = v[..] else {
                    return Err(error("bad OBJ vertex"));
                };
                if key == "v" { &mut positions } else { &mut normals }.push(DVec3::new(x, y, z));
            }
            "vt" => {
                let v = numbers(line, 2).map_err(bad)?;
                texcoords.push(DVec2::new(v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(0.0)));
            }
            "usemtl" => current = Some(rest(line).to_string()),
            "f" => {
                let mut corners: Vec<Corner> = Vec::new();
                for word in line.split_whitespace().skip(1) {
                    let mut items = word.split('/');
                    let mut next = |count: usize| index(items.next().unwrap_or_default(), count);
                    let v = next(positions.len()).map_err(bad)?.ok_or_else(|| bad("face without vertex".into()))?;
                    let vt = next(texcoords.len()).map_err(bad)?;
                    let vn = next(normals.len()).map_err(bad)?;
                    corners.push((v, vt, vn));
                    if corners.len() > MAX_POLYGON {
                        break;
                    }
                }
                if corners.len() < 3 || corners.len() > MAX_POLYGON {
                    continue;
                }
                total += 3 * (corners.len() - 2);
                if total > crate::models::MAX_VERTICES {
                    return Err(error(format!("too many vertices: more than {}", crate::models::MAX_VERTICES)));
                }
                let slot = *slots.entry(current.clone()).or_insert_with(|| {
                    groups.push((current.clone(), Vec::new()));
                    groups.len() - 1
                });
                let Some((_, target)) = groups.get_mut(slot) else {
                    continue;
                };
                for k in 1..corners.len() - 1 {
                    target.extend([corners[0], corners[k], corners[k + 1]]);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for (name, corners) in groups {
        let (image, rgba) = name.as_ref().and_then(|n| materials.get(n)).cloned().unwrap_or((None, GRAY));
        // Номера проверены при чтении граней.
        let pos: Vec<DVec3> = corners.iter().filter_map(|c| positions.get(c.0).copied()).collect();
        let uv: Option<Vec<DVec2>> = image.as_ref().and_then(|_| {
            corners.iter().map(|c| c.1.and_then(|i| texcoords.get(i)).map(|t| DVec2::new(t.x, 1.0 - t.y))).collect()
        });
        let nrm: Option<Vec<DVec3>> = corners.iter().map(|c| c.2.and_then(|i| normals.get(i).copied())).collect();
        let image = if uv.is_some() { image } else { None };
        out.push(from_y_up(Part::new(pos, nrm, uv, image, rgba)));
    }
    Ok(out)
}
