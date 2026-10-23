// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Шаблон окна метки, перенос `tests/test_balloon.py` модуля, и описание
//! кусками для окна без HTML.

use scene::balloon::{self, Piece};

#[test]
fn places() {
    let data = vec![("depth".to_string(), "412.5".to_string()), ("owner".to_string(), "A & B".to_string())];
    let text = balloon::fill(
        "<h3>$[name]</h3>$[description]<br>$[depth/displayName]: $[depth] м $[owner] $[nope]$[geDirections]",
        "Скв. <7>",
        "<b>керн</b>",
        &data,
    );
    assert_eq!(text, "<h3>Скв. &lt;7&gt;</h3><b>керн</b><br>depth: 412.5 м A &amp; B ");
}

#[test]
fn empty_template() {
    assert_eq!(balloon::fill("", "x", "", &[]), "");
}

#[test]
fn plain_text_stays() {
    assert_eq!(balloon::pieces("строка 1\nстрока 2"), [Piece::Text("строка 1\nстрока 2".into())]);
    assert!(balloon::pieces("").is_empty());
}

#[test]
fn html_to_text_and_links() {
    let pieces = balloon::pieces(
        "<p>Пуск <b>Starship</b> &amp; посадка</p><script>alert(1)</script><a href=\"https://x.org/a?b=1&amp;c=2\">Сайт</a><br/>конец",
    );
    assert_eq!(
        pieces,
        [
            // Абзац кончается переносом строки.
            Piece::Text("Пуск Starship & посадка\n".into()),
            Piece::Link("Сайт".into(), "https://x.org/a?b=1&c=2".into()),
            Piece::Text("\nконец".into()),
        ]
    );
}
