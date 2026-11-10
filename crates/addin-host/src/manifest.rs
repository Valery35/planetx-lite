// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Описание аддина - JSON `addin.json` и ответ `pxa_manifest`.
//!
//! ```json
//! {
//!   "id": "buffer", "version": "0.1.0", "author": "...", "license": "Apache-2.0",
//!   "title": {"ru": "Буфер", "en": "Buffer"},
//!   "description": {"ru": "...", "en": "..."},
//!   "permissions": ["features", "layers"],
//!   "extensions": [],
//!   "commands": [{
//!     "id": "buffer", "title": {"ru": "Буфер…", "en": "Buffer…"},
//!     "places": ["object", "layer"], "kinds": ["point", "line", "polygon"],
//!     "params": [{"id": "distance", "type": "number", "unit": "m",
//!                 "title": {"ru": "Расстояние", "en": "Distance"},
//!                 "default": 100, "min": -100000, "max": 1000000}]
//!   }]
//! }
//! ```
//!
//! Описание приходит извне и считается враждебным: длины, количество
//! и значения ограничены, неизвестное право - отказ.

use serde_json::Value;

/// Наибольшее количество команд, параметров, вариантов.
const MAX_ITEMS: usize = 64;
/// Наибольшая длина строки описания, символов.
const MAX_TEXT: usize = 2000;

/// Текст на двух языках.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Text {
    pub ru: String,
    pub en: String,
}

impl Text {
    /// Текст на языке интерфейса: ru - русский, иначе английский. Если
    /// перевода нет, берётся другой язык.
    pub fn get(&self, ru: bool) -> &str {
        let (a, b) = if ru { (&self.ru, &self.en) } else { (&self.en, &self.ru) };
        if a.is_empty() { b } else { a }
    }

    fn parse(v: Option<&Value>, fallback: &str) -> Text {
        let s = |x: Option<&Value>| x.and_then(Value::as_str).map(clip).unwrap_or_default();
        match v {
            Some(Value::String(t)) => Text { ru: clip(t), en: clip(t) },
            Some(o @ Value::Object(_)) => Text { ru: s(o.get("ru")), en: s(o.get("en")) },
            _ => Text { ru: clip(fallback), en: clip(fallback) },
        }
    }
}

fn clip(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(MAX_TEXT).collect()
}

/// Право аддина.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// Получать объекты, выбранные пользователем для команды.
    Features,
    /// Класть результат в «Мои слои».
    Layers,
}

impl Permission {
    /// Ключ права в описании.
    pub fn key(self) -> &'static str {
        match self {
            Permission::Features => "features",
            Permission::Layers => "layers",
        }
    }

    fn parse(s: &str) -> Option<Permission> {
        match s {
            "features" => Some(Permission::Features),
            "layers" => Some(Permission::Layers),
            _ => None,
        }
    }
}

/// Вид параметра команды.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamKind {
    /// Число с единицами (m - метры, km, deg и т. п.), пределами.
    Number { unit: String, default: f64, min: f64, max: f64 },
    /// Строка.
    Text { default: String },
    /// Флажок.
    Bool { default: bool },
    /// Выбор из вариантов: ключ и подпись.
    Choice { options: Vec<(String, Text)>, default: String },
}

/// Параметр команды.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub id: String,
    pub title: Text,
    pub kind: ParamKind,
}

impl Param {
    /// Значение по умолчанию в JSON.
    pub fn default_value(&self) -> Value {
        match &self.kind {
            ParamKind::Number { default, .. } => Value::from(*default),
            ParamKind::Text { default } => Value::from(default.clone()),
            ParamKind::Bool { default } => Value::from(*default),
            ParamKind::Choice { default, .. } => Value::from(default.clone()),
        }
    }
}

/// Команда аддина.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub id: String,
    pub title: Text,
    /// Где команда видна: object - меню объекта, layer - меню слоя
    /// «Моих слоёв».
    pub places: Vec<String>,
    /// Какие геометрии команда принимает: point, line, polygon.
    pub kinds: Vec<String>,
    pub params: Vec<Param>,
}

impl Command {
    /// Видна ли команда в месте place.
    pub fn at(&self, place: &str) -> bool {
        self.places.iter().any(|p| p == place)
    }

    /// Принимает ли команда геометрию kind.
    pub fn takes(&self, kind: addin_api::Kind) -> bool {
        let k = match kind {
            addin_api::Kind::Point => "point",
            addin_api::Kind::Line => "line",
            addin_api::Kind::Polygon => "polygon",
        };
        self.kinds.is_empty() || self.kinds.iter().any(|x| x == k)
    }
}

/// Описание аддина.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub author: String,
    pub license: String,
    pub title: Text,
    pub description: Text,
    pub permissions: Vec<Permission>,
    /// Расширения файлов, которые аддин читает.
    pub extensions: Vec<String>,
    pub commands: Vec<Command>,
}

/// Ключ: латиница, цифры, дефис и подчёркивание, 1-64 знака.
fn key(v: Option<&Value>, what: &str) -> Result<String, String> {
    let s = v.and_then(Value::as_str).ok_or(format!("в описании нет {what}"))?;
    let ok = !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    if ok { Ok(s.to_string()) } else { Err(format!("неверный ключ {what}: {}", clip(s))) }
}

fn list(v: Option<&Value>) -> Result<&[Value], String> {
    match v {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(a)) if a.len() <= MAX_ITEMS => Ok(a),
        _ => Err("список в описании длиннее предела или не список".into()),
    }
}

fn strings(v: Option<&Value>) -> Result<Vec<String>, String> {
    Ok(list(v)?.iter().filter_map(Value::as_str).map(clip).collect())
}

fn num(v: Option<&Value>, d: f64) -> f64 {
    v.and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(d)
}

fn param(v: &Value) -> Result<Param, String> {
    let id = key(v.get("id"), "id параметра")?;
    let title = Text::parse(v.get("title"), &id);
    let kind = match v.get("type").and_then(Value::as_str).unwrap_or("number") {
        "number" => {
            let min = num(v.get("min"), f64::MIN);
            let max = num(v.get("max"), f64::MAX);
            if min > max {
                return Err(format!("у параметра {id} min больше max"));
            }
            let unit = v.get("unit").and_then(Value::as_str).map(clip).unwrap_or_default();
            ParamKind::Number { unit, default: num(v.get("default"), 0.0).clamp(min, max), min, max }
        }
        "text" => ParamKind::Text { default: v.get("default").and_then(Value::as_str).map(clip).unwrap_or_default() },
        "bool" => ParamKind::Bool { default: v.get("default").and_then(Value::as_bool).unwrap_or(false) },
        "choice" => {
            let mut options = Vec::new();
            for o in list(v.get("options"))? {
                options.push((key(o.get("id"), "варианта")?, Text::parse(o.get("title"), "")));
            }
            let first = options.first().map(|o| o.0.clone()).ok_or(format!("у параметра {id} нет вариантов"))?;
            let default = v.get("default").and_then(Value::as_str).filter(|d| options.iter().any(|o| o.0 == *d)).map_or(first, str::to_string);
            ParamKind::Choice { options, default }
        }
        t => return Err(format!("неизвестный вид параметра {}", clip(t))),
    };
    Ok(Param { id, title, kind })
}

impl Manifest {
    /// Разобрать описание. Аддин версии 1 без id берёт id из name.
    pub fn parse(json: &str) -> Result<Manifest, String> {
        if json.len() > crate::MAX_MANIFEST {
            return Err("описание аддина длиннее предела".into());
        }
        let v: Value = serde_json::from_str(json).map_err(|e| format!("описание аддина не JSON: {e}"))?;
        let id = key(v.get("id").or_else(|| v.get("name")), "id")?;
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(clip).unwrap_or_default();
        let mut permissions = Vec::new();
        for p in strings(v.get("permissions"))? {
            let p = Permission::parse(&p).ok_or(format!("неизвестное право аддина «{p}»"))?;
            if !permissions.contains(&p) {
                permissions.push(p);
            }
        }
        let mut commands = Vec::new();
        for c in list(v.get("commands"))? {
            let cid = key(c.get("id"), "id команды")?;
            let mut params = Vec::new();
            for p in list(c.get("params"))? {
                params.push(param(p)?);
            }
            commands.push(Command {
                title: Text::parse(c.get("title"), &cid),
                places: strings(c.get("places"))?,
                kinds: strings(c.get("kinds"))?,
                params,
                id: cid,
            });
        }
        // Команда получает объекты и кладёт результат в «Мои слои»: без
        // этих прав в описании команда не берётся.
        if !commands.is_empty() && !(permissions.contains(&Permission::Features) && permissions.contains(&Permission::Layers)) {
            return Err("у команд аддина нет прав features и layers в описании".into());
        }
        Ok(Manifest {
            title: Text::parse(v.get("title"), &id),
            description: Text::parse(v.get("description"), ""),
            version: text("version"),
            author: text("author"),
            license: text("license"),
            extensions: strings(v.get("extensions"))?,
            permissions,
            commands,
            id,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{Manifest, ParamKind, Permission};

    #[test]
    fn manifest_parses_and_rejects() {
        let m = Manifest::parse(r#"{"name":"csv-points","version":"0.1.0","extensions":["csv"]}"#).unwrap();
        assert_eq!((m.id.as_str(), m.title.get(true), m.extensions.len()), ("csv-points", "csv-points", 1));
        let m = Manifest::parse(
            r#"{"id":"buffer","version":"0.1.0","title":{"ru":"Буфер","en":"Buffer"},"permissions":["features","layers"],
               "commands":[{"id":"buffer","places":["object"],"params":[{"id":"distance","unit":"m","default":5e9,"min":-1,"max":10}]}]}"#,
        )
        .unwrap();
        assert_eq!(m.title.get(false), "Buffer");
        assert_eq!(m.permissions, vec![Permission::Features, Permission::Layers]);
        assert!(matches!(m.commands[0].params[0].kind, ParamKind::Number { default, .. } if default == 10.0));
        // Неизвестное право, команда без прав, плохой ключ - отказ.
        assert!(Manifest::parse(r#"{"id":"x","permissions":["network"]}"#).is_err());
        assert!(Manifest::parse(r#"{"id":"x","commands":[{"id":"c"}]}"#).is_err());
        assert!(Manifest::parse(r#"{"id":"../x"}"#).is_err());
        assert!(Manifest::parse("not json").is_err());
    }
}
