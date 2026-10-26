// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Видимое снаружи поведение Python в помощнике модуля - истинность
//! значений, `json.dumps(ensure_ascii=False)`, `str()` значения
//! и `str.splitlines()`. Текст этих функций уходит в модель и в окно,
//! поэтому он совпадает с текстом модуля.

use serde_json::{Number, Value};

/// Истинность значения JSON, как `bool()` Python - пустые строка,
/// список, словарь, ноль и `None` ложны.
pub(crate) fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Поле словаря, если оно истинно, как `d.get(key) or ...` Python.
pub(crate) fn get_true<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key).filter(|v| truthy(v))
}

/// Количество знаков строки, как `len()` Python.
pub(crate) fn chars(text: &str) -> usize {
    text.chars().count()
}

/// Текст `json.dumps(value, ensure_ascii=False, sort_keys=sort)` -
/// разделители «, » и «: », буквы без `\u`.
pub(crate) fn dumps(value: &Value, sort: bool) -> String {
    let mut out = String::new();
    write_json(&mut out, value, sort);
    out
}

fn write_json(out: &mut String, value: &Value, sort: bool) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&number(n)),
        Value::String(s) => quote_json(out, s),
        Value::Array(items) => {
            out.push('[');
            for (n, item) in items.iter().enumerate() {
                if n > 0 {
                    out.push_str(", ");
                }
                write_json(out, item, sort);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            if sort {
                entries.sort_by(|a, b| a.0.cmp(b.0));
            }
            out.push('{');
            for (n, (key, item)) in entries.into_iter().enumerate() {
                if n > 0 {
                    out.push_str(", ");
                }
                quote_json(out, key);
                out.push_str(": ");
                write_json(out, item, sort);
            }
            out.push('}');
        }
    }
}

/// Строка JSON с экранированием `json.dumps(ensure_ascii=False)`.
fn quote_json(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if c < ' ' => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Число, как его пишут `json.dumps` и `repr` Python.
fn number(n: &Number) -> String {
    if let Some(i) = n.as_i64() {
        i.to_string()
    } else if let Some(u) = n.as_u64() {
        u.to_string()
    } else {
        n.as_f64().map(float_repr).unwrap_or_default()
    }
}

/// `repr(float)` Python - кратчайшие цифры, показатель степени при
/// порядке меньше -4 или от 16, у целого значения «.0».
pub(crate) fn float_repr(x: f64) -> String {
    if x.is_nan() {
        return "nan".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0" } else { "0.0" }.to_string();
    }
    let short = format!("{x:e}");
    let Some((mantissa, exp)) = short.split_once('e') else {
        return short;
    };
    let exp: i64 = exp.parse().unwrap_or(0);
    let sign = if mantissa.starts_with('-') { "-" } else { "" };
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let body = if !(-4..16).contains(&exp) {
        let (first, rest) = digits.split_at(1.min(digits.len()));
        let mant = if rest.is_empty() { first.to_string() } else { format!("{first}.{rest}") };
        format!("{mant}e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs())
    } else if exp >= 0 {
        let whole = usize::try_from(exp).unwrap_or(0) + 1;
        if digits.len() <= whole {
            format!("{digits}{}.0", "0".repeat(whole - digits.len()))
        } else {
            format!("{}.{}", &digits[..whole], &digits[whole..])
        }
    } else {
        let zeros = usize::try_from(-exp - 1).unwrap_or(0);
        format!("0.{}{digits}", "0".repeat(zeros))
    };
    format!("{sign}{body}")
}

/// `str(value)` Python - строка как есть, остальное как `repr`.
pub(crate) fn py_str(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => repr(other),
    }
}

/// `repr` значения JSON, разобранного `json.loads` - словарь
/// в кавычках-апострофах, `True`, `False`, `None`.
fn repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(b) => if *b { "True" } else { "False" }.to_string(),
        Value::Number(n) => number(n),
        Value::String(s) => quote_repr(s),
        Value::Array(items) => format!("[{}]", items.iter().map(repr).collect::<Vec<_>>().join(", ")),
        Value::Object(map) => {
            let items: Vec<String> = map.iter().map(|(k, v)| format!("{}: {}", quote_repr(k), repr(v))).collect();
            format!("{{{}}}", items.join(", "))
        }
    }
}

/// `repr(str)` Python: апострофы, двойные кавычки - если в строке есть
/// апостроф и нет двойной кавычки.
fn quote_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') { '"' } else { '\'' };
    let mut out = String::new();
    out.push(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if c < ' ' || ('\u{7f}'..='\u{a0}').contains(&c) => out.push_str(&format!("\\x{:02x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Строки текста, как `str.splitlines()` Python: разрывы `\n`, `\r`,
/// `\r\n` и другие разделители строк Юникода, последний разрыв пустой
/// строки не даёт.
pub(crate) fn splitlines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut iter = text.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        if matches!(c, '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}') {
            out.push(&text[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r' && iter.peek().is_some_and(|&(_, next)| next == '\n') {
                iter.next();
                end += 1;
            }
            start = end;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_as_python() {
        for (x, text) in [
            (1.0, "1.0"),
            (2.5, "2.5"),
            (100.0, "100.0"),
            (0.0001, "0.0001"),
            (1e-5, "1e-05"),
            (1.5e-7, "1.5e-07"),
            (1e16, "1e+16"),
            (1.2345e16, "1.2345e+16"),
            (123456789012345.6, "123456789012345.6"),
            (-36.5, "-36.5"),
            (0.1, "0.1"),
        ] {
            assert_eq!(float_repr(x), text, "{x}");
        }
    }

    #[test]
    fn repr_as_python() {
        let v: Value = serde_json::from_str(r#"{"a": "it's", "b": [true, null, 1.0], "c": "x'\"y"}"#).unwrap_or_default();
        assert_eq!(py_str(&v), r#"{'a': "it's", 'b': [True, None, 1.0], 'c': 'x\'"y'}"#);
    }
}
