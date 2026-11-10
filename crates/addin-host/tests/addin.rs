// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Песочница аддинов: аддин-прототип CSV читает точки, враждебные
//! модули кончаются ошибкой, а не зависанием или падением.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use addin_api::{Feature, Kind};
use addin_host::{Addin, Package};

const CSV_POINTS: &[u8] = include_bytes!("../addins/csv_points.wasm");
/// Аддин-пример команды (`addins/buffer`). В программу не встроен: его
/// ставят из каталога аддинов.
const BUFFER: &[u8] = include_bytes!("../addins/buffer.wasm");
/// Описание пакета буфера - `addin.json` пакета.
const BUFFER_JSON: &str = include_str!("../../../addins/buffer/addin.json");

/// Отпечаток буфера 1000 м вокруг Перми, линии и полигона в программе,
/// 10 ноября 2026 года.
/// Вся математика идёт внутри модуля, поэтому отпечаток не зависит от
/// машины.
const BUFFER_DIGEST: u64 = 0x8f0d_5405_bf86_4f88;

fn sample() -> Vec<Feature> {
    vec![
        Feature { kind: Kind::Point, parts: vec![vec![[58.0105, 56.2294, 171.0]]], name: "Пермь".into() },
        Feature { kind: Kind::Line, parts: vec![vec![[58.0, 56.0, 0.0], [58.05, 56.1, 0.0], [58.0, 56.2, 0.0]]], name: "ЛЭП".into() },
        Feature { kind: Kind::Polygon, parts: vec![vec![[57.9, 56.0, 0.0], [57.9, 56.1, 0.0], [57.95, 56.1, 0.0]]], name: String::new() },
    ]
}

#[test]
fn buffer_addin_runs_command() {
    let mut a = Addin::load(BUFFER).unwrap();
    assert_eq!(a.abi, 2);
    assert!(a.has_commands());
    let t0 = std::time::Instant::now();
    let out = a.run_command("buffer", &serde_json::json!({"distance": 1000}), &sample()).unwrap();
    eprintln!("буфер трёх объектов в песочнице: {:.0} мс", t0.elapsed().as_secs_f64() * 1000.0);
    assert_eq!(out.len(), 3);
    assert!(out.iter().all(|f| f.kind == Kind::Polygon && f.parts[0].len() > 60));
    assert_eq!(out[0].name, "Пермь");
    let d = addin_host::digest_features(&out);
    assert_eq!(d, BUFFER_DIGEST, "{d:#018x}");
    // Ошибка аддина - текстом, читать файлы аддин команды не умеет.
    assert!(a.run_command("buffer", &serde_json::json!({"distance": 0}), &sample()).unwrap_err().contains("расстояние"));
    assert!(a.read_points(b"lat,lon\n1,2\n").is_err());
    // Аддин версии 1 команд не выполняет.
    let mut csv = Addin::load(CSV_POINTS).unwrap();
    assert!(csv.run_command("buffer", &serde_json::json!({}), &sample()).is_err());
}

#[test]
fn package_checks_manifest_against_module() {
    let p = Package::from_parts(BUFFER_JSON, BUFFER.to_vec()).unwrap();
    assert_eq!(p.manifest.commands[0].title.get(true), "Буфер…");
    let zip = p.write().unwrap();
    let back = Package::read(&zip).unwrap();
    assert_eq!(back.manifest, p.manifest);
    // Пакет обещает меньше прав, чем просит модуль, - отказ.
    let fewer = BUFFER_JSON.replace(r#""permissions":["features","layers"],"#, r#""permissions":[],"#).replace(r#""commands":[{"#, r#""x":[{"#);
    let e = Package::from_parts(&fewer, BUFFER.to_vec()).unwrap_err();
    assert!(e.contains("сверх описания"), "{e}");
    // Чужая версия в описании пакета - отказ.
    assert!(Package::from_parts(&BUFFER_JSON.replace("0.1.0", "0.2.0"), BUFFER.to_vec()).is_err());
    // Не ZIP и испорченный ZIP - без паники.
    assert!(Package::read(b"not zip").is_err());
    for n in (0..zip.len()).step_by(4099) {
        let mut bad = zip.clone();
        bad[n] ^= 0x5A;
        let _ = Package::read(&bad);
    }
}

#[test]
fn csv_addin_reads_points() {
    let mut a = Addin::load(CSV_POINTS).unwrap();
    assert!(a.manifest.contains("\"csv-points\""), "{}", a.manifest);
    let pts = a.read_points("name,lat,lon,alt\nПермь,58.0105,56.2294,171\nЭльбрус,43.34778,42.44001,5642\n".as_bytes()).unwrap();
    assert_eq!(pts.len(), 2);
    assert_eq!((pts[1].name.as_str(), pts[1].alt), ("Эльбрус", 5642.0));
    // Ошибка аддина - текстом, повторный вызов работает.
    let e = a.read_points(b"x,y\n1,2\n").unwrap_err();
    assert!(e.contains("широты"), "{e}");
    assert_eq!(a.read_points(b"lat,lon\n1,2\n").unwrap().len(), 1);
}

#[test]
fn sample_digest_is_fixed() {
    let mut a = Addin::load(addin_host::CSV_POINTS).unwrap();
    let pts = a.read_points(addin_host::SAMPLE_CSV.as_bytes()).unwrap();
    assert_eq!(pts.len(), 4);
    let d = addin_host::digest(&pts);
    eprintln!("отпечаток образца: {d:#018x}");
    assert_eq!(d, addin_host::SAMPLE_DIGEST, "{d:#018x}");
}

#[test]
fn big_input_crosses_the_boundary() {
    // 100 тысяч строк: замер передачи большого массива через границу
    // песочницы в обе стороны.
    let mut text = String::from("lat,lon,alt,name\n");
    for i in 0..100_000 {
        text.push_str(&format!("{},{},{},p{i}\n", 50.0 + f64::from(i) * 1e-5, 30.0, f64::from(i)));
    }
    let mut a = Addin::load(CSV_POINTS).unwrap();
    let t0 = std::time::Instant::now();
    let pts = a.read_points(text.as_bytes()).unwrap();
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    eprintln!("100 000 точек, {} КБ входа: {ms:.0} мс", text.len() / 1024);
    assert_eq!(pts.len(), 100_000);
    assert_eq!(pts[99_999].name, "p99999");
}

#[test]
fn hostile_modules_fail_cleanly() {
    // Не модуль.
    assert!(Addin::load(b"not wasm").is_err());
    // Импорт функции хоста - отказ.
    let imports = wat::parse_str(r#"(module (import "env" "open" (func)) (memory (export "memory") 1))"#).unwrap();
    assert!(Addin::load(&imports).unwrap_err().contains("импорт"));
    // Бесконечный цикл в чтении - кончается топливо, Lite не висит.
    let lp = wat::parse_str(
        r#"(module
            (memory (export "memory") 1)
            (func (export "pxa_abi") (result i32) i32.const 1)
            (func (export "pxa_manifest") (result i32) i32.const 0)
            (func (export "pxa_input") (param i32) (result i32) i32.const 16)
            (func (export "pxa_output") (result i32) i32.const 0)
            (func (export "pxa_read") (param i32) (result i32) (loop $l br $l) i32.const 0))"#,
    )
    .unwrap();
    let mut a = Addin::load(&lp).unwrap();
    let t0 = std::time::Instant::now();
    assert!(a.read_points(b"lat,lon\n1,2\n").is_err());
    assert!(t0.elapsed().as_secs() < 30);
    // Рост памяти сверх предела - отказ.
    let grow = wat::parse_str(
        r#"(module
            (memory (export "memory") 1)
            (func (export "pxa_abi") (result i32) i32.const 1)
            (func (export "pxa_manifest") (result i32) i32.const 0)
            (func (export "pxa_input") (param i32) (result i32) i32.const 16)
            (func (export "pxa_output") (result i32) i32.const 0)
            (func (export "pxa_read") (param i32) (result i32) i32.const 60000 memory.grow))"#,
    )
    .unwrap();
    let mut a = Addin::load(&grow).unwrap();
    // memory.grow на 60000 страниц (3.9 ГБ) не даётся: ответ -1, это
    // «ошибка» с пустым текстом, а не выделение памяти.
    assert!(a.read_points(b"lat,lon\n1,2\n").is_err());
    // Чужая версия ABI.
    let old = wat::parse_str(r#"(module (memory (export "memory") 1) (func (export "pxa_abi") (result i32) i32.const 9))"#).unwrap();
    assert!(Addin::load(&old).is_err());
    // Испорченные байты аддина - без паники.
    for n in (0..CSV_POINTS.len()).step_by(997) {
        let mut bad = CSV_POINTS.to_vec();
        bad[n] ^= 0x5A;
        let _ = Addin::load(&bad);
    }
}
