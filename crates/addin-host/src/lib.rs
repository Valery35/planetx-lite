// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Песочница аддинов PlanetX Lite, слой L4. Прототип фазы 3.
//!
//! Аддин - модуль WebAssembly с ABI [`addin_api`]. Исполняет его wasmi -
//! интерпретатор на Rust, один и тот же в программе и на странице.
//! Модуль проверяется при загрузке. Импортов у аддина нет, значит нет
//! и выхода из песочницы: ни файлов, ни сети, ни часов. Память модуля
//! не больше [`MAX_MEMORY`], у каждого вызова свой запас шагов
//! исполнения (топливо wasmi): бесконечный цикл аддина кончается
//! ошибкой, а не зависанием Lite. Ошибка или ловушка аддина - текст
//! ошибки, Lite не падает.

use addin_api::{ABI_VERSION, Point, decode_points};
use wasmi::{Config, Engine, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc};

/// Встроенный аддин-прототип: точки из CSV (`addins/csv-points`).
pub const CSV_POINTS: &[u8] = include_bytes!("../addins/csv_points.wasm");

/// Образец CSV для самопроверки аддина.
pub const SAMPLE_CSV: &str = "name,lat,lon,alt\nПермь,58.0105,56.2294,171\nЭльбрус,43.34778,42.44001,5642\nКарьер,59.486,56.97,210.5\nБока-Чика,25.997,-97.157,3\n";

/// Отпечаток точек: FNV-1a 64 по их байтам [`addin_api::encode_points`].
/// Одинаковый отпечаток - одинаковое чтение до бита.
pub fn digest(points: &[Point]) -> u64 {
    addin_api::encode_points(points).iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// Отпечаток чтения [`SAMPLE_CSV`] аддином [`CSV_POINTS`] в программе,
/// 23 октября 2026 года. Страница обязана дать тот же.
pub const SAMPLE_DIGEST: u64 = 0x2697_3e66_6467_121e;

/// Наибольший модуль аддина, байт.
pub const MAX_MODULE: usize = 32 << 20;
/// Наибольшая память аддина, байт.
pub const MAX_MEMORY: usize = 512 << 20;
/// Наибольший вход одного вызова, байт.
pub const MAX_INPUT: usize = 64 << 20;
/// Топливо на загрузку и короткие вызовы.
pub const FUEL_BASE: u64 = 100_000_000;
/// Топливо на байт входа при чтении.
pub const FUEL_PER_BYTE: u64 = 2_000;

struct Data {
    limits: StoreLimits,
}

/// Загруженный аддин.
pub struct Addin {
    store: Store<Data>,
    memory: Memory,
    input: TypedFunc<i32, i32>,
    read: TypedFunc<i32, i32>,
    output: TypedFunc<(), i32>,
    /// Описание аддина, JSON.
    pub manifest: String,
}

impl std::fmt::Debug for Addin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Addin").field("manifest", &self.manifest).finish()
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Addin {
    /// Загрузить аддин из байтов модуля.
    pub fn load(bytes: &[u8]) -> Result<Addin, String> {
        if bytes.len() > MAX_MODULE {
            return Err(format!("модуль аддина больше {MAX_MODULE} байт"));
        }
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, bytes).map_err(err)?;
        if module.imports().len() > 0 {
            return Err("аддин просит импорты, а у песочницы их нет".into());
        }
        let limits = StoreLimitsBuilder::new().memory_size(MAX_MEMORY).instances(1).memories(1).tables(1).build();
        let mut store = Store::new(&engine, Data { limits });
        store.limiter(|d| &mut d.limits);
        store.set_fuel(FUEL_BASE).map_err(err)?;
        let linker = Linker::<Data>::new(&engine);
        let instance = linker.instantiate_and_start(&mut store, &module).map_err(err)?;
        let memory = instance.get_memory(&store, "memory").ok_or("у аддина нет памяти memory")?;
        let abi: TypedFunc<(), i32> = instance.get_typed_func(&store, "pxa_abi").map_err(err)?;
        let manifest_fn: TypedFunc<(), i32> = instance.get_typed_func(&store, "pxa_manifest").map_err(err)?;
        let input = instance.get_typed_func(&store, "pxa_input").map_err(err)?;
        let read = instance.get_typed_func(&store, "pxa_read").map_err(err)?;
        let output = instance.get_typed_func(&store, "pxa_output").map_err(err)?;
        let version = abi.call(&mut store, ()).map_err(err)?;
        if version != ABI_VERSION {
            return Err(format!("версия ABI аддина {version}, у Lite {ABI_VERSION}"));
        }
        let mut addin = Addin { store, memory, input, read, output, manifest: String::new() };
        let len = manifest_fn.call(&mut addin.store, ()).map_err(err)?;
        let bytes = addin.take_output(len)?;
        addin.manifest = String::from_utf8_lossy(&bytes).into_owned();
        Ok(addin)
    }

    /// Выход аддина длиной len.
    fn take_output(&mut self, len: i32) -> Result<Vec<u8>, String> {
        let len = usize::try_from(len).map_err(|_| "длина выхода меньше нуля")?;
        if len > MAX_MEMORY {
            return Err("выход больше памяти аддина".into());
        }
        let at = self.output.call(&mut self.store, ()).map_err(err)?;
        let at = usize::try_from(at).map_err(|_| "адрес выхода меньше нуля")?;
        let mut buf = vec![0u8; len];
        self.memory.read(&self.store, at, &mut buf).map_err(err)?;
        Ok(buf)
    }

    /// Прочитать данные input: точки или текст ошибки аддина.
    pub fn read_points(&mut self, input: &[u8]) -> Result<Vec<Point>, String> {
        if input.len() > MAX_INPUT {
            return Err(format!("вход больше {MAX_INPUT} байт"));
        }
        let fuel = FUEL_BASE.saturating_add(FUEL_PER_BYTE.saturating_mul(input.len() as u64));
        self.store.set_fuel(fuel).map_err(err)?;
        let len = i32::try_from(input.len()).map_err(err)?;
        let at = self.input.call(&mut self.store, len).map_err(err)?;
        let at = usize::try_from(at).ok().filter(|a| *a != 0).ok_or("у аддина нет места под вход")?;
        self.memory.write(&mut self.store, at, input).map_err(err)?;
        let result = self.read.call(&mut self.store, len).map_err(err)?;
        if result < 0 {
            let text = self.take_output(result.saturating_neg())?;
            return Err(format!("аддин: {}", String::from_utf8_lossy(&text)));
        }
        let bytes = self.take_output(result)?;
        decode_points(&bytes)
    }
}
