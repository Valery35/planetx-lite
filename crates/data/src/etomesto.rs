// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Каталог старых карт сайта «Это Место» (etomesto.com) по точке. Перенос
//! `core/etomesto.py` модуля.
//!
//! Каталогом, карты сайта программа не показывает. Сайт отдаёт список
//! карт, накрывающих точку, страницей listmap.php по широте y и долготе x,
//! без ключа и учётной записи. Строка списка - год, название и имя
//! страницы карты. Карта открывается на сайте браузером, по адресу
//! страницы, тайлы сайта не запрашиваются - они закрыты ключом сеанса,
//! файлы карт сайт даёт для личного некоммерческого использования. Запрос -
//! один на действие пользователя.
//!
//! Разбор строки списка повторяет регулярное выражение `ROW` модуля
//! вручную, без зависимости от крейта регулярных выражений.

/// Сайт.
pub const SITE: &str = "https://etomesto.com/";
pub const ATTRIBUTION: (&str, &str) = ("Это Место", SITE);

/// Строка списка карт: год или None, название, имя страницы.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub year: Option<i32>,
    pub title: String,
    pub name: String,
}

/// Адрес списка карт у точки.
pub fn list_url(lat: f64, lon: f64) -> String {
    format!("{SITE}listmap.php?y={lat:.6}&x={lon:.6}")
}

/// Страница карты на сайте, вид на точке.
pub fn page_url(name: &str, lat: f64, lon: f64) -> String {
    format!("{SITE}map-{name}/?y={lat:.6}&x={lon:.6}")
}

/// Первый год 1000-2099 в тексте, не часть более длинного числа.
fn find_year(text: &str) -> Option<i32> {
    let chars: Vec<char> = text.chars().collect();
    for i in 0..chars.len() {
        if i > 0 && chars.get(i - 1).is_some_and(char::is_ascii_digit) {
            continue;
        }
        let Some(four) = chars.get(i..i + 4) else { break };
        if !four.iter().all(char::is_ascii_digit) || chars.get(i + 4).is_some_and(char::is_ascii_digit) {
            continue;
        }
        let ok = four[0] == '1' || (four[0] == '2' && four[1] == '0');
        if ok {
            return four.iter().collect::<String>().parse().ok();
        }
    }
    None
}

/// Год карты из столбца года, иначе первый год в названии, иначе в имени
/// страницы («perm_permskaya-gub-1800»).
fn year(text: &str, title: &str, name: &str) -> Option<i32> {
    [text, title, name].into_iter().find_map(find_year)
}

/// Разбор по месту: текст и позиция в байтах.
struct Cursor<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn rest(&self) -> &'a str {
        self.text.get(self.pos..).unwrap_or("")
    }

    /// Литерал без учёта регистра ASCII.
    fn literal(&mut self, word: &str) -> bool {
        let rest = self.rest().as_bytes();
        let ok = rest.len() >= word.len() && rest.iter().zip(word.as_bytes()).all(|(a, b)| a.eq_ignore_ascii_case(b));
        if ok {
            self.pos += word.len();
        }
        ok
    }

    /// Пробелы, `\s*`.
    fn spaces(&mut self) {
        let rest = self.rest();
        self.pos += rest.len() - rest.trim_start().len();
    }

    /// Знаки до stop, не включая его, `[^stop]*`.
    fn until(&mut self, stop: char) -> &'a str {
        let rest = self.rest();
        let n = rest.find(stop).unwrap_or(rest.len());
        self.pos += n;
        rest.get(..n).unwrap_or("")
    }

    /// Имя страницы, `[A-Za-z0-9_.\-]+`.
    fn name(&mut self) -> Option<&'a str> {
        let rest = self.rest();
        let n = rest.find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))).unwrap_or(rest.len());
        self.pos += n;
        (n > 0).then(|| rest.get(..n).unwrap_or(""))
    }
}

/// Строка списка с позиции pos: год, имя, название и конец совпадения.
/// Образец модуля - `<b>([^<]*)</b>\s*</td>\s*<td[^>]*>\s*<a
/// href="JavaScript:gomap\('([A-Za-z0-9_.\-]+)'\);"[^>]*>([^<]*)</a>`
/// без учёта регистра.
fn row_at(text: &str, pos: usize) -> Option<(&str, &str, &str, usize)> {
    let mut c = Cursor { text, pos };
    if !c.literal("<b>") {
        return None;
    }
    let year = c.until('<');
    if !c.literal("</b>") {
        return None;
    }
    c.spaces();
    if !c.literal("</td>") {
        return None;
    }
    c.spaces();
    if !c.literal("<td") {
        return None;
    }
    c.until('>');
    if !c.literal(">") {
        return None;
    }
    c.spaces();
    if !c.literal("<a href=\"JavaScript:gomap('") {
        return None;
    }
    let name = c.name()?;
    if !c.literal("');\"") {
        return None;
    }
    c.until('>');
    if !c.literal(">") {
        return None;
    }
    let title = c.until('<');
    if !c.literal("</a>") {
        return None;
    }
    Some((year, name, title, c.pos))
}

/// Страница listmap.php - строки списка в порядке сайта, без повторов
/// имени. Байты не UTF-8 заменяются знаком замены.
pub fn parse_list(data: &[u8]) -> Vec<Item> {
    let text = String::from_utf8_lossy(data);
    let mut out: Vec<Item> = Vec::new();
    let mut pos = 0;
    while let Some(found) = text.get(pos..).and_then(|rest| rest.find('<')) {
        let start = pos + found;
        let Some((year_text, name, title, end)) = row_at(&text, start) else {
            pos = start + 1;
            continue;
        };
        pos = end;
        if out.iter().any(|item| item.name == name) {
            continue;
        }
        let title = unescape(title).split_whitespace().collect::<Vec<_>>().join(" ");
        out.push(Item { year: year(&unescape(year_text), &title, name), title, name: name.to_string() });
    }
    out
}

/// Подходит ли строка списка под слова поиска - все слова в названии или
/// в годе. Пустой поиск подходит всем.
pub fn matches(item: &Item, words: &str) -> bool {
    let year = item.year.filter(|y| *y != 0).map(|y| y.to_string()).unwrap_or_default();
    let hay = format!("{year} {}", item.title).to_lowercase();
    words.to_lowercase().split_whitespace().all(|w| hay.contains(w))
}

/// Знаки HTML: числовые и частые именованные. Неизвестный знак остаётся
/// как есть.
fn unescape(text: &str) -> String {
    const NAMED: [(&str, char); 14] = [
        ("amp", '&'),
        ("lt", '<'),
        ("gt", '>'),
        ("quot", '"'),
        ("apos", '\''),
        ("nbsp", '\u{a0}'),
        ("laquo", '«'),
        ("raquo", '»'),
        ("mdash", '—'),
        ("ndash", '–'),
        ("hellip", '…'),
        ("copy", '©'),
        ("deg", '°'),
        ("numero", '№'),
    ];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(rest.get(..at).unwrap_or(""));
        let tail = rest.get(at + 1..).unwrap_or("");
        let end = tail.find(';').filter(|n| *n <= 10);
        let decoded = end.and_then(|n| {
            let body = tail.get(..n)?;
            let c = if let Some(hex) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X")) {
                char::from_u32(u32::from_str_radix(hex, 16).ok()?)
            } else if let Some(dec) = body.strip_prefix('#') {
                char::from_u32(dec.parse().ok()?)
            } else {
                NAMED.iter().find(|(k, _)| *k == body).map(|(_, c)| *c)
            }?;
            Some((c, n + 1))
        });
        match decoded {
            Some((c, used)) => {
                out.push(c);
                rest = tail.get(used..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}
