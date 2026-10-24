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
