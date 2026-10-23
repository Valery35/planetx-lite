// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Ссылка на вид: адрес страницы, адреса сцен и поза камеры.
//!
//! `?scene=адрес&scene=адрес&pose=широта,долгота,расстояние,азимут,наклон`.
//! Сцены открываются по порядку, как файлы в командной строке
//! программы: сначала растр высот, потом сцена над ним. Поза сильнее
//! позы сцены. Файлы с диска в ссылку не попадают - у них нет адреса.

use globe_core::navigation::Pose;

/// Страница Lite на GitHub Pages. Программа делится ссылкой на неё.
pub const PAGE: &str = "https://valery35.github.io/planetx-lite/";

/// Поза из строки «широта,долгота,расстояние,азимут,наклон», за ними
/// могут идти высота точки взгляда в метрах и крен в градусах.
pub fn parse_pose(text: &str) -> Option<Pose> {
    let v: Vec<f64> = text.split(',').map(|p| p.trim().parse::<f64>()).collect::<Result<_, _>>().ok()?;
    let (&[lat, lon, distance, heading, tilt], rest) = v.split_first_chunk::<5>()?;
    let ok = rest.len() <= 2 && v.iter().all(|x| x.is_finite()) && lat.abs() <= 90.0 && distance > 0.0;
    let mut pose = Pose::new(lat, lon, distance, heading, tilt).with_h(rest.first().copied().unwrap_or(0.0));
    pose.roll = rest.get(1).copied().unwrap_or(0.0);
    ok.then_some(pose)
}

/// Поза строкой для ссылки. Широта и долгота - 7 знаков после запятой,
/// около сантиметра на земле, расстояние - десятые метра, углы - сотые
/// градуса. Нули в конце отбрасываются. Высота точки взгляда и крен
/// пишутся, когда они не нулевые.
pub fn format_pose(p: &Pose) -> String {
    let trim = |s: String| {
        let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
        if s == "-0" { "0".to_string() } else { s }
    };
    let mut parts = vec![
        trim(format!("{:.7}", p.lat)),
        trim(format!("{:.7}", p.lon)),
        trim(format!("{:.1}", p.distance)),
        trim(format!("{:.2}", p.heading)),
        trim(format!("{:.2}", p.tilt)),
    ];
    let roll = trim(format!("{:.2}", p.roll));
    let h = trim(format!("{:.1}", p.h));
    if roll != "0" {
        parts.extend([h, roll]);
    } else if h != "0" {
        parts.push(h);
    }
    parts.join(",")
}

/// Ссылка: base - адрес страницы без параметров, scenes - адреса сцен
/// по порядку.
pub fn link(base: &str, scenes: &[String], pose: &Pose) -> String {
    let mut out = base.split(['?', '#']).next().unwrap_or(base).to_string();
    let mut sep = '?';
    for scene in scenes {
        out.push(sep);
        out.push_str("scene=");
        out.push_str(&encode(scene));
        sep = '&';
    }
    out.push(sep);
    out.push_str("pose=");
    out.push_str(&format_pose(pose));
    out
}

/// Адрес, к которому строится ссылка: у программы - [`PAGE`].
#[cfg(not(target_arch = "wasm32"))]
pub fn page_address() -> String {
    PAGE.to_string()
}

/// Адрес, к которому строится ссылка: страница на своём сервере
/// делится своим адресом.
#[cfg(target_arch = "wasm32")]
pub fn page_address() -> String {
    let location = web_sys::window().map(|w| w.location());
    match location.and_then(|l| Some(format!("{}{}", l.origin().ok()?, l.pathname().ok()?))) {
        Some(address) => address,
        None => PAGE.to_string(),
    }
}

/// Окно «Поделиться» системы. У программы его нет, ссылка идёт
/// в буфер обмена.
#[cfg(not(target_arch = "wasm32"))]
pub fn system_share(_link: &str) -> bool {
    false
}

/// Окно «Поделиться» системы - на устройстве с касанием, где браузер
/// его даёт (`navigator.share`). На компьютере ссылка идёт в буфер
/// обмена. false - окна нет.
#[cfg(target_arch = "wasm32")]
pub fn system_share(link: &str) -> bool {
    use js_sys::{Function, Object, Reflect};
    use wasm_bindgen::{JsCast, JsValue};
    let global = js_sys::global();
    let get = |target: &JsValue, key: &str| Reflect::get(target, &key.into()).ok().filter(|v| !v.is_undefined());
    let touch = get(&global, "matchMedia")
        .and_then(|f| f.dyn_into::<Function>().ok())
        .and_then(|f| f.call1(&global, &"(pointer: coarse)".into()).ok())
        .and_then(|list| get(&list, "matches"))
        .is_some_and(|m| m.is_truthy());
    let Some(navigator) = get(&global, "navigator") else {
        return false;
    };
    let Some(share) = get(&navigator, "share").and_then(|f| f.dyn_into::<Function>().ok()) else {
        return false;
    };
    if !touch {
        return false;
    }
    let data = Object::new();
    let set = Reflect::set(&data, &"title".into(), &"PlanetX Lite".into()).and(Reflect::set(&data, &"url".into(), &link.into()));
    let Ok(promise) = set.and_then(|_| share.call1(&navigator, &data)) else {
        return false;
    };
    // Отказ пользователя - не ошибка, ответ ждать незачем.
    if let Ok(promise) = promise.dyn_into::<js_sys::Promise>() {
        wasm_bindgen_futures::spawn_local(async move {
            let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
        });
    }
    true
}

/// Параметры адреса страницы.
#[derive(Debug, Default)]
pub struct Query {
    /// Адреса сцен по порядку.
    pub scenes: Vec<String>,
    /// Поза камеры, если задана и верна.
    pub pose: Option<Pose>,
}

/// Разбор строки параметров `?a=b&c=d`. Неверная поза пропускается.
pub fn parse_query(search: &str) -> Query {
    let mut query = Query::default();
    for part in search.trim_start_matches('?').split('&') {
        if let Some(v) = part.strip_prefix("scene=") {
            let url = decode(v);
            if !url.is_empty() {
                query.scenes.push(url);
            }
        } else if let Some(v) = part.strip_prefix("pose=") {
            query.pose = parse_pose(&decode(v));
        }
    }
    query
}

/// Кодирование значения параметра. Остаются как есть буквы, цифры,
/// `-_.~` и знаки адреса `:/@,`, остальное - `%XX` байтов UTF-8:
/// ссылка читается, а `&`, `=`, `?`, `#`, `+`, `%` и пробел в адресе
/// сцены не ломают её.
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~:/@,".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Раскодирование `%XX`, как decodeURIComponent браузера. Неверная
/// последовательность остаётся как есть.
pub fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |k: usize| bytes.get(k).and_then(|c| char::from(*c).to_digit(16));
        match (bytes[i], hex(i + 1), hex(i + 2)) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose() -> Pose {
        let mut p = Pose::new(58.0104, 56.2294, 12_345.678, 30.5, 45.25).with_h(5642.3);
        p.roll = -3.5;
        p
    }

    #[test]
    fn link_and_back() {
        let scenes = vec![
            "https://raw.githubusercontent.com/Valery35/planetx-terrain/main/demo/quarry.tif".to_string(),
            "https://example.org/a b/сцена.planetx?x=1&y=2#t".to_string(),
        ];
        let url = link("https://valery35.github.io/planetx-lite/?old=1", &scenes, &pose());
        assert!(url.starts_with("https://valery35.github.io/planetx-lite/?scene=https://raw.githubusercontent.com/"));
        assert!(!url.contains("old=1"));
        assert!(!url.contains(' '));
        let back = parse_query(url.split_once('?').map_or("", |(_, q)| q));
        assert_eq!(back.scenes, scenes);
        let p = back.pose.unwrap_or_else(|| Pose::new(0.0, 0.0, 1.0, 0.0, 0.0));
        let want = pose();
        assert!((p.lat - want.lat).abs() < 1e-7);
        assert!((p.lon - want.lon).abs() < 1e-7);
        assert!((p.distance / want.distance - 1.0).abs() < 1e-5);
        assert!((p.heading - want.heading).abs() < 1e-2);
        assert!((p.tilt - want.tilt).abs() < 1e-2);
        assert!((p.h - want.h).abs() < 0.1);
        assert!((p.roll - want.roll).abs() < 1e-2);
    }

    #[test]
    fn height_without_roll() {
        let url = link(PAGE, &[], &Pose::new(43.35, 42.44, 9000.0, 0.0, 60.0).with_h(5642.0));
        assert!(url.ends_with("pose=43.35,42.44,9000,0,60,5642"), "{url}");
        let p = parse_pose("43.35,42.44,9000,0,60,5642");
        assert!(p.is_some_and(|p| p.h == 5642.0 && p.roll == 0.0));
    }

    #[test]
    fn link_without_scenes() {
        let url = link(PAGE, &[], &Pose::new(10.0, -20.5, 1000.0, 0.0, 0.0));
        assert_eq!(url, "https://valery35.github.io/planetx-lite/?pose=10,-20.5,1000,0,0");
    }

    #[test]
    fn bad_pose_is_skipped() {
        for bad in ["pose=1,2,3", "pose=1,2,3,4,5,6,7,8", "pose=95,0,100,0,0", "pose=1,2,-5,0,0", "pose=1,2,NaN,0,0", "pose=a,b,c,d,e"] {
            assert!(parse_query(bad).pose.is_none(), "{bad}");
        }
    }

    #[test]
    fn decode_keeps_broken_escapes() {
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%zz%4"), "%zz%4");
        assert_eq!(decode("%D0%BF"), "п");
        assert_eq!(decode("a+b"), "a+b");
    }

    #[test]
    fn empty_scene_is_skipped() {
        assert_eq!(parse_query("?scene=&gl").scenes, Vec::<String>::new());
    }
}
