// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Шаблон всплывающего окна метки - BalloonStyle KML, перенос
//! `core/balloon.py` модуля, и описание метки простым текстом для окна
//! без HTML.
//!
//! Текст шаблона - HTML с местами $[...], как у Google Earth: $[name] -
//! название, $[description] - описание, $[поле] - значение поля объекта
//! (ExtendedData), $[поле/displayName] - подпись поля. Неизвестное место
//! и $[geDirections] дают пустую строку. Значения полей и название
//! экранируются, описание - HTML метки, оно вставляется как есть.

/// Экранирование HTML, как html.escape Python.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

/// HTML окна метки по шаблону template.
pub fn fill(template: &str, name: &str, description: &str, data: &[(String, String)]) -> String {
    let value = |key: &str| data.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("$[") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find(']').filter(|e| *e > 0) else {
            out.push_str(&rest[start..]);
            return out;
        };
        let key = after[..end].trim();
        match key {
            "name" => out.push_str(&escape(name)),
            "description" => out.push_str(description),
            _ => {
                if let Some(field) = key.strip_suffix("/displayName") {
                    if value(field).is_some() {
                        out.push_str(&escape(field));
                    }
                } else if let Some(v) = value(key) {
                    out.push_str(&escape(v));
                }
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Кусок описания для окна: текст или ссылка (текст, адрес).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Text(String),
    Link(String, String),
}

/// Сущности HTML в текст: именованные основные и числовые.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let Some(semi) = after.find(';').filter(|s| *s <= 10) else {
            out.push('&');
            rest = after;
            continue;
        };
        let entity = &after[..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Описание метки кусками для окна без HTML, как `description_html`
/// модуля: простой текст - с переносами строк, HTML - текст без тегов,
/// переносы по br, p, div, li, tr, ссылки a href - ссылками. Скрипты
/// и стили выбрасываются.
pub fn pieces(text: &str) -> Vec<Piece> {
    if !(text.contains('<') && text.contains('>')) {
        return if text.is_empty() { Vec::new() } else { vec![Piece::Text(text.to_string())] };
    }
    let mut out: Vec<Piece> = Vec::new();
    let mut buffer = String::new();
    let mut link: Option<(String, String)> = None;
    let mut skip: Option<String> = None;
    let mut rest = text;
    let flush = |buffer: &mut String, out: &mut Vec<Piece>| {
        if !buffer.is_empty() {
            let text = unescape(buffer);
            match out.last_mut() {
                Some(Piece::Text(t)) => t.push_str(&text),
                _ => out.push(Piece::Text(text)),
            }
            buffer.clear();
        }
    };
    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            if skip.is_none() {
                buffer.push_str(rest);
            }
            break;
        };
        let before = &rest[..open];
        let Some(close) = rest[open..].find('>') else {
            buffer.push_str(rest);
            break;
        };
        let tag = &rest[open + 1..open + close];
        rest = &rest[open + close + 1..];
        if skip.is_none() {
            match &mut link {
                Some((t, _)) => t.push_str(before),
                None => buffer.push_str(before),
            }
        }
        let lower = tag.trim().to_ascii_lowercase();
        let name: String = lower.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        let closing = lower.starts_with('/');
        if let Some(open_name) = &skip {
            if closing && *open_name == name {
                skip = None;
            }
            continue;
        }
        match name.as_str() {
            "script" | "style" if !closing => skip = Some(name),
            "a" if !closing => {
                flush(&mut buffer, &mut out);
                link = attribute(tag, "href").map(|href| (String::new(), unescape(&href)));
            }
            "a" => {
                if let Some((t, href)) = link.take() {
                    let t = unescape(t.trim());
                    out.push(Piece::Link(if t.is_empty() { href.clone() } else { t }, href));
                }
            }
            "br" | "p" | "div" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "table" => {
                if !(closing && matches!(name.as_str(), "br")) {
                    match &mut link {
                        Some((t, _)) => t.push('\n'),
                        None => buffer.push('\n'),
                    }
                }
            }
            "td" | "th" if closing => buffer.push(' '),
            _ => {}
        }
    }
    flush(&mut buffer, &mut out);
    // Пустые строки подряд - одна, края без пустых строк.
    for piece in &mut out {
        if let Piece::Text(t) = piece {
            let mut clean = String::with_capacity(t.len());
            let mut newlines = 0;
            for c in t.chars() {
                if c == '\n' {
                    newlines += 1;
                    if newlines <= 2 {
                        clean.push(c);
                    }
                } else {
                    newlines = 0;
                    clean.push(c);
                }
            }
            *t = clean;
        }
    }
    if let Some(Piece::Text(t)) = out.first_mut() {
        *t = t.trim_start().to_string();
    }
    if let Some(Piece::Text(t)) = out.last_mut() {
        *t = t.trim_end().to_string();
    }
    out.retain(|p| !matches!(p, Piece::Text(t) if t.is_empty()));
    out
}

/// Значение атрибута тега name="..." или name='...' или без кавычек.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let at = from + found;
        from = at + name.len();
        let before_ok = at == 0 || lower[..at].ends_with(|c: char| c.is_whitespace());
        let after = tag[from..].trim_start();
        if !before_ok || !after.starts_with('=') {
            continue;
        }
        let value = after[1..].trim_start();
        return Some(match value.chars().next() {
            Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or("").to_string(),
            _ => value.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or("").to_string(),
        });
    }
    None
}
