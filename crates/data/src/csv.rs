// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Таблица CSV по RFC 4180: запятые, ячейки в кавычках с запятыми
//! и переводами строк внутри, двойная кавычка - кавычка. Текст
//! считается враждебным: незакрытая кавычка просто доходит до конца.

/// Строки таблицы ячейками. Пустые строки пропускаются.
pub fn rows(text: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    cell.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                _ => cell.push(c),
            }
            continue;
        }
        match c {
            '"' if cell.is_empty() => quoted = true,
            ',' => row.push(std::mem::take(&mut cell)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut cell));
                if row.iter().any(|c| !c.is_empty()) {
                    out.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            _ => cell.push(c),
        }
    }
    row.push(cell);
    if row.iter().any(|c| !c.is_empty()) {
        out.push(row);
    }
    out
}

/// Таблица с заголовком: номер столбца по имени и строки.
pub struct Table {
    pub head: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    pub fn parse(text: &str) -> Table {
        let mut all = rows(text);
        if all.is_empty() {
            return Table { head: Vec::new(), rows: Vec::new() };
        }
        let head = all.remove(0).into_iter().map(|h| h.trim().trim_start_matches('\u{feff}').to_string()).collect();
        Table { head, rows: all }
    }

    /// Номер столбца name.
    pub fn column(&self, name: &str) -> Option<usize> {
        self.head.iter().position(|h| h == name)
    }
}

/// Ячейка числом: только конечное число.
pub fn number(cell: Option<&String>) -> Option<f64> {
    cell?.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Записи CSV потоком из reader ячейками байтов, без таблицы в памяти:
/// файл MRDS USGS - 137 МБ, 300 тысяч записей. Правила те же, что
/// у [rows]. Пустые записи пропускаются.
pub fn records<R: std::io::Read>(mut reader: R, mut each: impl FnMut(&[Vec<u8>])) -> std::io::Result<()> {
    let mut row: Vec<Vec<u8>> = Vec::new();
    let mut cell: Vec<u8> = Vec::new();
    let mut quoted = false;
    // Кавычка внутри ячейки в кавычках: конец ячейки или первая из двух.
    let mut quote = false;
    let mut buffer = vec![0u8; 1 << 16];
    let flush = |row: &mut Vec<Vec<u8>>, each: &mut dyn FnMut(&[Vec<u8>])| {
        if row.iter().any(|c| !c.is_empty()) {
            each(row);
        }
        row.clear();
    };
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        for &b in &buffer[..n] {
            if quoted {
                if quote {
                    quote = false;
                    if b == b'"' {
                        cell.push(b'"');
                        continue;
                    }
                    quoted = false;
                } else if b == b'"' {
                    quote = true;
                    continue;
                } else {
                    cell.push(b);
                    continue;
                }
            }
            match b {
                b'"' if cell.is_empty() => quoted = true,
                b',' => row.push(std::mem::take(&mut cell)),
                b'\r' => {}
                b'\n' => {
                    row.push(std::mem::take(&mut cell));
                    flush(&mut row, &mut each);
                }
                _ => cell.push(b),
            }
        }
    }
    row.push(cell);
    flush(&mut row, &mut each);
    Ok(())
}

/// Записи CSV с заголовком потоком: each получает номер столбца по
/// имени и ячейки записи.
pub fn table_records<R: std::io::Read>(reader: R, mut each: impl FnMut(&dyn Fn(&str) -> Option<usize>, &[Vec<u8>])) -> std::io::Result<()> {
    let mut head: Option<Vec<String>> = None;
    records(reader, |cells| match &head {
        None => head = Some(cells.iter().map(|c| String::from_utf8_lossy(c).trim().trim_start_matches('\u{feff}').to_string()).collect()),
        Some(h) => each(&|name: &str| h.iter().position(|x| x == name), cells),
    })
}

/// Текст ячейки в UTF-8, неверные байты заменяются.
pub fn text(cells: &[Vec<u8>], column: Option<usize>) -> String {
    column.and_then(|c| cells.get(c)).map(|c| String::from_utf8_lossy(c).into_owned()).unwrap_or_default()
}

/// Текст ячейки в кодировке Windows-1252: файлы USGS minfac.
pub fn text_1252(cells: &[Vec<u8>], column: Option<usize>) -> String {
    // Байты 0x80-0x9F, у которых в 1252 свой знак, прочие - Latin-1.
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ',
        '\u{9d}', 'ž', 'Ÿ',
    ];
    column
        .and_then(|c| cells.get(c))
        .map(|c| c.iter().map(|&b| if (0x80..0xA0).contains(&b) { HIGH[usize::from(b - 0x80)] } else { char::from(b) }).collect())
        .unwrap_or_default()
}

/// Ячейка числом: только конечное число.
pub fn cell_number(cells: &[Vec<u8>], column: Option<usize>) -> Option<f64> {
    let cell = column.and_then(|c| cells.get(c))?;
    std::str::from_utf8(cell).ok()?.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}
/// Наибольший размер распакованной таблицы архива: MRDS - 137 МБ.
/// Больше - архив считается враждебным.
pub const ZIP_LIMIT: u64 = 512 << 20;

/// Записи таблицы CSV архива ZIP data, первый член, чьё имя без учёта
/// регистра кончается на suffix. Не ZIP - data сам считается таблицей.
pub fn zip_table(data: &[u8], suffix: &str, each: impl FnMut(&dyn Fn(&str) -> Option<usize>, &[Vec<u8>])) -> Result<(), String> {
    use std::io::Read;
    if !data.starts_with(b"PK") {
        return table_records(data, each).map_err(|e| e.to_string());
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data)).map_err(|e| e.to_string())?;
    let index = (0..archive.len())
        .find(|&i| archive.name_for_index(i).and_then(Result::ok).is_some_and(|n| n.to_lowercase().ends_with(suffix)))
        .ok_or_else(|| format!("no {suffix} in the archive"))?;
    let member = archive.by_index(index).map_err(|e| e.to_string())?;
    table_records(member.take(ZIP_LIMIT), each).map_err(|e| e.to_string())
}