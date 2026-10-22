// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Песочница аддинов: аддин-прототип CSV читает точки, враждебные
//! модули кончаются ошибкой, а не зависанием или падением.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use addin_host::Addin;

const CSV_POINTS: &[u8] = include_bytes!("../addins/csv_points.wasm");

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
