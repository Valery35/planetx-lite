// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Каталог карт «Это Место»: адреса и разбор списка карт у точки. Перенос
//! `tests/test_etomesto.py` модуля.

use data::etomesto::{self, Item};

/// Три строки ответа listmap.php у Перми 13 октября 2026 года в разметке
/// сайта: с годом, без года (год в имени страницы) и повтор.
const ROWS: &str = r#"<table>
<tr><td rowspan=2 valign=top bgcolor=#EDEBD3 class=s><b>1782</b></td>
<td width=100% bgcolor=#ecece1><a href="JavaScript:gomap('perm_1782');"
 class=bb>План города Перми 1782 года</a></td><td bgcolor=#ebebe5>
<a href="JavaScript:pm(5836,'79d769c','perm_1782');" class=bb>
<img src=/i/toprev1.gif></a></td></tr>
<tr><td rowspan=2 valign=top bgcolor=#EDEBD3 class=s><b></b></td>
<td width=100% bgcolor=#dff0f6>
<a href="JavaScript:gomap('perm_permskaya-gub-1800');"
 class=bb>Карта Пермской губернии из атласа Вильбрехта</a></td></tr>
<tr><td rowspan=2 valign=top bgcolor=#EDEBD3 class=s><b>1782</b></td>
<td width=100% bgcolor=#ecece1><a href="JavaScript:gomap('perm_1782');"
 class=bb>План города Перми 1782 года</a></td></tr>
</table>"#;

fn item(year: Option<i32>, title: &str, name: &str) -> Item {
    Item { year, title: title.to_string(), name: name.to_string() }
}

#[test]
fn rows() {
    let items = etomesto::parse_list(ROWS.as_bytes());
    assert_eq!(
        items,
        [item(Some(1782), "План города Перми 1782 года", "perm_1782"), item(Some(1800), "Карта Пермской губернии из атласа Вильбрехта", "perm_permskaya-gub-1800")]
    );
}

#[test]
fn entities_case_and_year_in_title() {
    let page = r#"<B>&#49;9 в.</B></TD><TD><A HREF="javascript:GOMAP('x.y-z_1');" class=bb>Карта  &laquo;Кама&raquo; 1905&amp;12345</a>"#;
    assert_eq!(etomesto::parse_list(page.as_bytes()), [item(Some(1905), "Карта «Кама» 1905&12345", "x.y-z_1")]);
}

#[test]
fn addresses() {
    assert_eq!(etomesto::list_url(58.0105, 56.2502), "https://etomesto.com/listmap.php?y=58.010500&x=56.250200");
    assert_eq!(etomesto::page_url("perm_1782", 58.0105, 56.2502), "https://etomesto.com/map-perm_1782/?y=58.010500&x=56.250200");
}

#[test]
fn search_words() {
    let it = item(Some(1782), "План города Перми 1782 года", "perm_1782");
    assert!(etomesto::matches(&it, "перми план"));
    assert!(etomesto::matches(&it, "ПЛАН"));
    assert!(etomesto::matches(&it, "1782"));
    assert!(!etomesto::matches(&it, "казань"));
    assert!(etomesto::matches(&it, ""));
}

#[test]
fn foreign_page_gives_nothing() {
    assert_eq!(etomesto::parse_list(b"<html>Not found</html>"), []);
    assert_eq!(etomesto::parse_list(&[0xff, b'<', 0xfe, b'<', b'b', b'>']), []);
    assert_eq!(etomesto::parse_list(b""), []);
}
