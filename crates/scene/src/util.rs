// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Общие мелочи крейта: запись чисел как в Python, пути с косыми
//! чертами, адреса с %XX, архив ZIP в памяти.

use std::io::{Cursor, Read, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// Наибольший размер файла внутри архива после распаковки, байт.
/// Архив-«бомба» с файлом больше этого не читается, а не съедает
/// память.
pub const MAX_ENTRY: u64 = 1 << 30;

/// Число, как `{:g}` в Python: шесть значащих цифр, без хвостовых нулей.
pub fn fmt_g(v: f64) -> String {
    if v.is_nan() {
        return "nan".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    let sci = format!("{v:.5e}");
    let (mantissa, exp) = sci.split_once('e').unwrap_or((sci.as_str(), "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    if (-4..6).contains(&exp) {
        let digits = usize::try_from(5 - exp).unwrap_or(0);
        strip_zeros(&format!("{v:.digits$}"))
    } else {
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{}e{sign}{:02}", strip_zeros(mantissa), exp.abs())
    }
}

fn strip_zeros(text: &str) -> String {
    if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_string() } else { text.to_string() }
}

/// Округление до digits знаков после запятой, половина - к чётному,
/// как round в Python.
pub fn py_round(v: f64, digits: i32) -> f64 {
    let m = 10f64.powi(digits);
    let r = (v * m).round_ties_even() / m;
    if r.is_finite() { r } else { v }
}

/// Адрес с %XX в обычную строку, как unquote в Python. Неверные
/// байты UTF-8 заменяются знаком замены.
pub fn unquote(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = bytes.get(i + 1..i + 3)
            && let Ok(s) = std::str::from_utf8(hex)
            && let Ok(b) = u8::from_str_radix(s, 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Путь без «.», «..» и двойных косых черт, как posixpath.normpath.
pub fn normpath(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let rooted = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|p| *p != "..") {
                    parts.pop();
                } else if !rooted {
                    parts.push("..");
                }
            }
            p => parts.push(p),
        }
    }
    let joined = parts.join("/");
    match (rooted, joined.is_empty()) {
        (true, _) => format!("/{joined}"),
        (false, true) => ".".into(),
        (false, false) => joined,
    }
}

/// Имя файла после последней косой черты.
pub fn basename(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

/// Папка до последней косой черты, пустая без неё.
pub fn dirname(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| if dir.is_empty() { "/" } else { dir })
}

/// Архив ZIP в памяти для чтения.
pub struct Archive<'a> {
    inner: ZipArchive<Cursor<&'a [u8]>>,
    names: Vec<String>,
}

impl<'a> Archive<'a> {
    /// Архив из байтов или текст ошибки.
    pub fn open(data: &'a [u8]) -> Result<Self, String> {
        let inner = ZipArchive::new(Cursor::new(data)).map_err(|e| e.to_string())?;
        let names = (0..inner.len())
            .filter_map(|i| inner.name_for_index(i).and_then(Result::ok).map(|n| n.into_owned()))
            .collect();
        Ok(Self { inner, names })
    }

    /// Имена файлов в порядке каталога архива.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Байты файла name не больше [`MAX_ENTRY`].
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>, String> {
        let file = self.inner.by_name(name).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        file.take(MAX_ENTRY + 1).read_to_end(&mut out).map_err(|e| e.to_string())?;
        if out.len() as u64 > MAX_ENTRY {
            return Err(format!("{name}: file is too large"));
        }
        Ok(out)
    }
}

/// Байты архива ZIP со сжатием deflate из пар (путь, байты).
pub fn write_zip<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> Result<Vec<u8>, String> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, data) in files {
        writer.start_file(name, options).map_err(|e| e.to_string())?;
        writer.write_all(data).map_err(|e| e.to_string())?;
    }
    writer.finish().map(Cursor::into_inner).map_err(|e| e.to_string())
}
