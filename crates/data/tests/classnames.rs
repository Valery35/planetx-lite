// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Названия классов от помощника, перенос `tests/test_classnames.py`
//! модуля 1.33.

use data::classnames::{self as cn, Row};

fn table() -> Vec<Row> {
    vec![
        Row { means: vec![0.08, 0.2, 0.02], share: 0.43, regions: 12, area_ha: 230.5, rule: Some("Хвойный лес".into()) },
        Row { means: vec![0.3, 0.22, 0.08], share: 0.21, regions: 3, area_ha: 44.0, rule: None },
    ]
}

fn roles(list: &[Option<&str>]) -> Vec<Option<String>> {
    list.iter().map(|r| r.map(str::to_string)).collect()
}

#[test]
fn roles_values_and_rules() {
    let text = cn::question(&table(), Some(&roles(&[Some("swir1"), Some("nir"), Some("blue")])), "отражение 0-1", Some((58.05, 56.2)), Some(7), true);
    assert!(text.contains("SWIR 1,6 мкм, ближний ИК, синий"));
    assert!(text.contains("Класс 1 - 0.080, 0.200, 0.020, доля участка 43 %"));
    assert!(text.contains("по правилам отражения - Хвойный лес"));
    assert!(text.contains("Месяц съёмки - 7."));
    assert!(text.contains("Класс 2 - 0.300, 0.220, 0.080, доля участка 21 %, областей 3, средняя область 44.0 га."));
}

#[test]
fn second_image_bands() {
    let rows = vec![Row { means: vec![0.1, 0.2, 0.3, 0.4], share: 1.0, regions: 1, area_ha: 1.0, rule: None }];
    let text = cn::question(&rows, Some(&roles(&[Some("nir"), Some("red"), Some("nir@2"), None])), "отражение", None, None, true);
    assert!(text.contains("ближний ИК, красный, ближний ИК второго снимка, канал 4"));
}

#[test]
fn unknown_bands_numbered() {
    let text = cn::question(&table(), None, "яркость 0-255", None, None, true);
    assert!(text.contains("канал 1, канал 2, канал 3"));
    assert!(!text.contains("Место"));
}

#[test]
fn system_language() {
    assert!(cn::system_text(true, "Russian").contains("русском"));
    assert!(cn::system_text(true, "English").contains("английском"));
    assert!(cn::system_text(true, "Russian").contains("{\"names\""));
    assert!(cn::system_text(false, "German").contains("in German"));
}

#[test]
fn object_in_text() {
    let text = "Вот ответ:\n```json\n{\"names\": [\"Хвоя\", \"Пашня\"]}\n```";
    assert_eq!(cn::parse_names(text, 2), Some(vec!["Хвоя".to_string(), "Пашня".to_string()]));
}

#[test]
fn bare_list_and_extra_names() {
    assert_eq!(cn::parse_names("[\"a\", \" b \", \"c\"]", 2), Some(vec!["a".to_string(), "b".to_string()]));
}

#[test]
fn short_or_broken_answer() {
    assert_eq!(cn::parse_names("{\"names\": [\"a\"]}", 2), None);
    assert_eq!(cn::parse_names("не знаю", 2), None);
    assert_eq!(cn::parse_names("", 2), None);
}

#[test]
fn long_name_cut() {
    let text = format!("[\"{}\"]", "х".repeat(100));
    assert_eq!(cn::parse_names(&text, 1).map(|n| n[0].chars().count()), Some(cn::NAME_LIMIT));
}
