// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Песочница аддинов PlanetX Lite, слой L4.
//!
//! Аддин - модуль WebAssembly с ABI [`addin_api`]. Исполняет его wasmi -
//! интерпретатор на Rust, один и тот же в программе и на странице.
//! Модуль проверяется при загрузке. Импортов у аддина нет, значит нет
//! и выхода из песочницы: ни файлов, ни сети, ни часов. Память модуля
//! не больше [`MAX_MEMORY`], у каждого вызова свой запас шагов
//! исполнения (топливо wasmi): бесконечный цикл аддина кончается
//! ошибкой, а не зависанием Lite. Ошибка или ловушка аддина - текст
//! ошибки, Lite не падает.
//!
//! Пакет аддина `.pxaddin` - ZIP с описанием `addin.json` и модулем
//! `addin.wasm` ([`Package`]). Описание в пакете - то, что видит
//! пользователь при установке. Модуль повторяет его своим `pxa_manifest`,
//! и права модуля не могут быть шире прав описания.

use std::io::Read;

use addin_api::{ABI_MIN, ABI_VERSION, Feature, Point, decode_features, decode_points, encode_request};
use serde_json::Value;
use wasmi::{Config, Engine, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc};

mod manifest;
pub use manifest::{Command, Manifest, Param, ParamKind, Permission, Text};

/// Встроенный аддин-прототип: точки из CSV (`addins/csv-points`).
pub const CSV_POINTS: &[u8] = include_bytes!("../addins/csv_points.wasm");

/// Образец CSV для самопроверки аддина.
pub const SAMPLE_CSV: &str = "name,lat,lon,alt\nПермь,58.0105,56.2294,171\nЭльбрус,43.34778,42.44001,5642\nКарьер,59.486,56.97,210.5\nБока-Чика,25.997,-97.157,3\n";

/// Отпечаток точек: FNV-1a 64 по их байтам [`addin_api::encode_points`].
/// Одинаковый отпечаток - одинаковое чтение до бита.
pub fn digest(points: &[Point]) -> u64 {
    fnv(&addin_api::encode_points(points))
}

/// Отпечаток объектов: FNV-1a 64 по их байтам
/// [`addin_api::encode_features`].
pub fn digest_features(features: &[Feature]) -> u64 {
    fnv(&addin_api::encode_features(features))
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// Отпечаток чтения [`SAMPLE_CSV`] аддином [`CSV_POINTS`] в программе,
/// 23 октября 2026 года. Страница обязана дать тот же.
pub const SAMPLE_DIGEST: u64 = 0x2697_3e66_6467_121e;

/// Наибольший модуль аддина, байт.
pub const MAX_MODULE: usize = 32 << 20;
/// Наибольшее описание аддина, байт.
pub const MAX_MANIFEST: usize = 256 << 10;
/// Наибольшая память аддина, байт.
pub const MAX_MEMORY: usize = 512 << 20;
/// Наибольший вход одного вызова, байт.
pub const MAX_INPUT: usize = 64 << 20;
/// Топливо на загрузку и короткие вызовы.
pub const FUEL_BASE: u64 = 100_000_000;
/// Топливо на байт входа при чтении.
pub const FUEL_PER_BYTE: u64 = 2_000;
/// Топливо на команду: геометрия считается дольше разбора текста.
pub const FUEL_COMMAND: u64 = 20_000_000_000;
/// Топливо команды на байт входа.
pub const FUEL_COMMAND_PER_BYTE: u64 = 200_000;

struct Data {
    limits: StoreLimits,
}

/// Загруженный аддин.
pub struct Addin {
    store: Store<Data>,
    memory: Memory,
    input: TypedFunc<i32, i32>,
    read: Option<TypedFunc<i32, i32>>,
    command: Option<TypedFunc<i32, i32>>,
    output: TypedFunc<(), i32>,
    /// Версия ABI модуля.
    pub abi: i32,
    /// Описание аддина, JSON, как его отдал модуль.
    pub manifest: String,
}

impl std::fmt::Debug for Addin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Addin").field("abi", &self.abi).field("manifest", &self.manifest).finish()
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
        let abi_fn: TypedFunc<(), i32> = instance.get_typed_func(&store, "pxa_abi").map_err(err)?;
        let abi = abi_fn.call(&mut store, ()).map_err(err)?;
        if !(ABI_MIN..=ABI_VERSION).contains(&abi) {
            return Err(format!("версия ABI аддина {abi}, Lite понимает {ABI_MIN}-{ABI_VERSION}"));
        }
        let manifest_fn: TypedFunc<(), i32> = instance.get_typed_func(&store, "pxa_manifest").map_err(err)?;
        let input = instance.get_typed_func(&store, "pxa_input").map_err(err)?;
        let output = instance.get_typed_func(&store, "pxa_output").map_err(err)?;
        let read = instance.get_typed_func(&store, "pxa_read").ok();
        // Команды - с версии 2: у модуля версии 1 экспорт не ищется.
        let command = if abi >= 2 { instance.get_typed_func(&store, "pxa_command").ok() } else { None };
        if read.is_none() && command.is_none() {
            return Err("у аддина нет ни pxa_read, ни pxa_command".into());
        }
        let mut addin = Addin { store, memory, input, read, command, output, abi, manifest: String::new() };
        let len = manifest_fn.call(&mut addin.store, ()).map_err(err)?;
        if usize::try_from(len).unwrap_or(usize::MAX) > MAX_MANIFEST {
            return Err("описание аддина длиннее предела".into());
        }
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

    /// Положить вход и вызвать f, результат - байты выхода.
    fn call(&mut self, f: TypedFunc<i32, i32>, input: &[u8], fuel: u64) -> Result<Vec<u8>, String> {
        if input.len() > MAX_INPUT {
            return Err(format!("вход больше {MAX_INPUT} байт"));
        }
        self.store.set_fuel(fuel).map_err(err)?;
        let len = i32::try_from(input.len()).map_err(err)?;
        let at = self.input.call(&mut self.store, len).map_err(err)?;
        let at = usize::try_from(at).ok().filter(|a| *a != 0).ok_or("у аддина нет места под вход")?;
        self.memory.write(&mut self.store, at, input).map_err(err)?;
        let result = f.call(&mut self.store, len).map_err(err)?;
        if result < 0 {
            let text = self.take_output(result.saturating_neg())?;
            return Err(format!("аддин: {}", String::from_utf8_lossy(&text)));
        }
        self.take_output(result)
    }

    /// Прочитать данные input: точки или текст ошибки аддина.
    pub fn read_points(&mut self, input: &[u8]) -> Result<Vec<Point>, String> {
        let f = self.read.ok_or("аддин не читает файлы")?;
        let fuel = FUEL_BASE.saturating_add(FUEL_PER_BYTE.saturating_mul(input.len() as u64));
        let bytes = self.call(f, input, fuel)?;
        decode_points(&bytes)
    }

    /// Есть ли у аддина команды.
    pub fn has_commands(&self) -> bool {
        self.command.is_some()
    }

    /// Выполнить команду над объектами. params - JSON со значениями
    /// параметров команды. Ответ - объекты результата или текст ошибки.
    pub fn run_command(&mut self, command: &str, params: &Value, features: &[Feature]) -> Result<Vec<Feature>, String> {
        let f = self.command.ok_or("у аддина нет команд")?;
        let input = encode_request(command, &params.to_string(), features);
        let fuel = FUEL_COMMAND.saturating_add(FUEL_COMMAND_PER_BYTE.saturating_mul(input.len() as u64));
        let bytes = self.call(f, &input, fuel)?;
        decode_features(&bytes)
    }
}

/// Имена файлов в пакете.
pub const PACKAGE_MANIFEST: &str = "addin.json";
pub const PACKAGE_MODULE: &str = "addin.wasm";

/// Пакет аддина `.pxaddin`, проверенный: описание разобрано, модуль
/// загружается, его описание совпадает с описанием пакета.
#[derive(Debug, Clone)]
pub struct Package {
    pub manifest: Manifest,
    /// Текст `addin.json`.
    pub manifest_json: String,
    /// Байты `addin.wasm`.
    pub module: Vec<u8>,
}

/// Файл из ZIP не длиннее max.
fn zip_entry(archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, name: &str, max: usize) -> Result<Vec<u8>, String> {
    let file = archive.by_name(name).map_err(|_| format!("в пакете нет {name}"))?;
    let mut out = Vec::new();
    file.take(max as u64 + 1).read_to_end(&mut out).map_err(err)?;
    if out.len() > max {
        return Err(format!("{name} в пакете больше предела"));
    }
    Ok(out)
}

impl Package {
    /// Пакет из байтов ZIP.
    pub fn read(bytes: &[u8]) -> Result<Package, String> {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("пакет не ZIP: {e}"))?;
        let json = zip_entry(&mut archive, PACKAGE_MANIFEST, MAX_MANIFEST)?;
        let module = zip_entry(&mut archive, PACKAGE_MODULE, MAX_MODULE)?;
        Package::from_parts(&String::from_utf8_lossy(&json), module)
    }

    /// Пакет из описания и модуля.
    pub fn from_parts(manifest_json: &str, module: Vec<u8>) -> Result<Package, String> {
        let manifest = Manifest::parse(manifest_json)?;
        let addin = Addin::load(&module)?;
        let inner = Manifest::parse(&addin.manifest)?;
        if inner.id != manifest.id || inner.version != manifest.version {
            return Err(format!(
                "описание модуля ({} {}) не совпадает с описанием пакета ({} {})",
                inner.id, inner.version, manifest.id, manifest.version
            ));
        }
        if let Some(p) = inner.permissions.iter().find(|p| !manifest.permissions.contains(p)) {
            return Err(format!("аддин просит право «{}» сверх описания пакета", p.key()));
        }
        if let Some(c) = inner.commands.iter().find(|c| !manifest.commands.iter().any(|m| m.id == c.id)) {
            return Err(format!("команды «{}» нет в описании пакета", c.id));
        }
        if !inner.commands.is_empty() && !addin.has_commands() {
            return Err("в описании есть команды, а в модуле нет pxa_command".into());
        }
        Ok(Package { manifest, manifest_json: manifest_json.to_string(), module })
    }

    /// Пакет в байты ZIP.
    pub fn write(&self) -> Result<Vec<u8>, String> {
        use std::io::Write;
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut cursor);
            let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            z.start_file(PACKAGE_MANIFEST, opts).map_err(err)?;
            z.write_all(self.manifest_json.as_bytes()).map_err(err)?;
            z.start_file(PACKAGE_MODULE, opts).map_err(err)?;
            z.write_all(&self.module).map_err(err)?;
            z.finish().map_err(err)?;
        }
        Ok(cursor.into_inner())
    }
}
