// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Чтение netCDF-3: формат classic (CDF1) и 64-bit offset (CDF2).
//!
//! Модуль читает выборки ERDDAP через GDAL. Ответ griddap в виде `.nc` -
//! netCDF-3 по документации ERDDAP. Здесь разбор без GDAL: измерения,
//! переменные, атрибуты и значения переменной в f64. Формат netCDF-4
//! (HDF5) и CDF5 не читаются, на них ошибка.
//!
//! Файл считается враждебным. Каждое смещение и размер сверяются
//! с длиной файла до выделения памяти, плохой файл даёт [`NcError`].
//! Значения переменной идут по строкам (порядок C): последнее измерение
//! меняется быстрее всех, измерение записей - первое.

/// Метка списка измерений.
const NC_DIMENSION: u32 = 0x0A;
/// Метка списка переменных.
const NC_VARIABLE: u32 = 0x0B;
/// Метка списка атрибутов.
const NC_ATTRIBUTE: u32 = 0x0C;
/// Количество записей не известно, файл пишется потоком.
const STREAMING: u32 = 0xFFFF_FFFF;
/// Заполнитель по умолчанию у float и double, NC_FILL_FLOAT
/// и NC_FILL_DOUBLE библиотеки netCDF.
const DEFAULT_FILL_FLOAT: f64 = 9.969_209_968_386_869e36;

/// Ошибка чтения netCDF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NcError(pub String);

impl std::fmt::Display for NcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "netCDF не читается: {}", self.0)
    }
}

impl std::error::Error for NcError {}

/// Ошибка с текстом what.
pub(crate) fn error(what: impl Into<String>) -> NcError {
    NcError(what.into())
}

fn truncated() -> NcError {
    error("файл обрезан")
}

/// Вариант формата.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// CDF1, смещения 32 бита.
    Classic,
    /// CDF2, смещения 64 бита.
    Offset64,
}

/// Тип значений netCDF-3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NcType {
    Byte,
    Char,
    Short,
    Int,
    Float,
    Double,
}

impl NcType {
    fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            1 => Self::Byte,
            2 => Self::Char,
            3 => Self::Short,
            4 => Self::Int,
            5 => Self::Float,
            6 => Self::Double,
            _ => return None,
        })
    }

    /// Размер одного значения в байтах.
    pub fn size(self) -> usize {
        match self {
            Self::Byte | Self::Char => 1,
            Self::Short => 2,
            Self::Int | Self::Float => 4,
            Self::Double => 8,
        }
    }

    /// Значение из size() байтов, старший байт первый. Byte - со знаком,
    /// Char - код байта.
    fn decode(self, b: &[u8]) -> f64 {
        let at = |i: usize| b.get(i).copied().unwrap_or(0);
        match self {
            Self::Byte => f64::from(at(0) as i8),
            Self::Char => f64::from(at(0)),
            Self::Short => f64::from(i16::from_be_bytes([at(0), at(1)])),
            Self::Int => f64::from(i32::from_be_bytes([at(0), at(1), at(2), at(3)])),
            Self::Float => f64::from(f32::from_be_bytes([at(0), at(1), at(2), at(3)])),
            Self::Double => f64::from_be_bytes([at(0), at(1), at(2), at(3), at(4), at(5), at(6), at(7)]),
        }
    }
}

/// Измерение. У измерения записей len - количество записей файла.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dimension {
    pub name: String,
    pub len: usize,
    pub unlimited: bool,
}

/// Значение атрибута: текст у типа Char, числа у остальных.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrValue {
    Text(String),
    Numbers(Vec<f64>),
}

/// Атрибут файла или переменной.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub kind: NcType,
    pub value: AttrValue,
}

impl Attribute {
    /// Текст атрибута типа Char.
    pub fn text(&self) -> Option<&str> {
        match &self.value {
            AttrValue::Text(text) => Some(text),
            AttrValue::Numbers(_) => None,
        }
    }

    /// Числа атрибута, пусто у текста.
    pub fn numbers(&self) -> &[f64] {
        match &self.value {
            AttrValue::Numbers(values) => values,
            AttrValue::Text(_) => &[],
        }
    }

    /// Первое число атрибута.
    pub fn number(&self) -> Option<f64> {
        self.numbers().first().copied()
    }
}

/// Атрибут по имени в списке.
pub fn find_attr<'b>(attrs: &'b [Attribute], name: &str) -> Option<&'b Attribute> {
    attrs.iter().find(|a| a.name == name)
}

/// Переменная. dims - номера измерений в [`NcFile::dims`], begin -
/// смещение данных от начала файла, record - переменная записей.
#[derive(Debug, Clone, PartialEq)]
pub struct Variable {
    pub name: String,
    pub dims: Vec<usize>,
    pub attrs: Vec<Attribute>,
    pub kind: NcType,
    pub begin: u64,
    pub record: bool,
}

impl Variable {
    /// Атрибут переменной по имени.
    pub fn attr(&self, name: &str) -> Option<&Attribute> {
        find_attr(&self.attrs, name)
    }
}

/// Разобранный заголовок файла netCDF-3 над его байтами.
#[derive(Debug, Clone)]
pub struct NcFile<'a> {
    bytes: &'a [u8],
    pub format: Format,
    /// Количество записей.
    pub numrecs: usize,
    pub dims: Vec<Dimension>,
    /// Глобальные атрибуты.
    pub attrs: Vec<Attribute>,
    pub vars: Vec<Variable>,
    /// Размер одной записи в байтах, сумма частей переменных записей.
    record_size: u64,
}

/// Чтение заголовка по порядку.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], NcError> {
        let end = self.pos.checked_add(n).ok_or_else(truncated)?;
        let out = self.bytes.get(self.pos..end).ok_or_else(truncated)?;
        self.pos = end;
        Ok(out)
    }

    fn u32(&mut self) -> Result<u32, NcError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, NcError> {
        let b = self.take(8)?;
        Ok(u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    /// Количество элементов, не больше того, что вмещает остаток файла
    /// при наименьшем размере элемента least байтов.
    fn count(&mut self, least: usize) -> Result<usize, NcError> {
        let n = self.u32()? as usize;
        if n > self.remaining() / least.max(1) {
            return Err(error("количество элементов больше файла"));
        }
        Ok(n)
    }

    /// n байтов и выравнивание до 4.
    fn padded(&mut self, n: usize) -> Result<&'a [u8], NcError> {
        let out = self.take(n)?;
        self.take((4 - n % 4) % 4)?;
        Ok(out)
    }

    fn name(&mut self) -> Result<String, NcError> {
        let n = self.count(1)?;
        let raw = self.padded(n)?;
        Ok(String::from_utf8_lossy(raw).into_owned())
    }

    /// Метка списка и количество элементов. Пустой список - два нуля.
    fn list(&mut self, tag: u32, least: usize) -> Result<usize, NcError> {
        let got = self.u32()?;
        let n = self.count(least)?;
        if got == 0 && n == 0 {
            return Ok(0);
        }
        if got != tag {
            return Err(error("неверная метка списка заголовка"));
        }
        Ok(n)
    }

    fn attrs(&mut self) -> Result<Vec<Attribute>, NcError> {
        // Наименьший атрибут: имя 4, тип 4, количество 4.
        let n = self.list(NC_ATTRIBUTE, 12)?;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let name = self.name()?;
            let kind = NcType::from_code(self.u32()?).ok_or_else(|| error("неизвестный тип атрибута"))?;
            let count = self.count(kind.size())?;
            let raw = self.padded(count * kind.size())?;
            let value = if kind == NcType::Char {
                AttrValue::Text(String::from_utf8_lossy(raw).trim_end_matches('\0').to_string())
            } else {
                AttrValue::Numbers(raw.chunks_exact(kind.size()).map(|b| kind.decode(b)).collect())
            };
            out.push(Attribute { name, kind, value });
        }
        Ok(out)
    }
}

/// Округление вверх до кратного 4.
fn pad4(n: u64) -> Option<u64> {
    n.checked_add(3).map(|x| x & !3)
}

impl<'a> NcFile<'a> {
    /// Разбор заголовка. Данные не копируются, они читаются из bytes
    /// по запросу.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, NcError> {
        if bytes.starts_with(b"\x89HDF") {
            return Err(error("файл netCDF-4 (HDF5), читается только netCDF-3"));
        }
        let format = match bytes.get(..4) {
            Some(b"CDF\x01") => Format::Classic,
            Some(b"CDF\x02") => Format::Offset64,
            Some(b"CDF\x05") => return Err(error("формат CDF5 не читается")),
            _ => return Err(error("нет подписи CDF")),
        };
        let mut r = Reader { bytes, pos: 4 };
        let numrecs_raw = r.u32()?;

        // Наименьшее измерение: имя 4, длина 4.
        let ndims = r.list(NC_DIMENSION, 8)?;
        let mut dims = Vec::with_capacity(ndims);
        for _ in 0..ndims {
            let name = r.name()?;
            let len = r.u32()? as usize;
            dims.push(Dimension { name, len, unlimited: len == 0 });
        }
        if dims.iter().filter(|d| d.unlimited).count() > 1 {
            return Err(error("больше одного измерения записей"));
        }
        let attrs = r.attrs()?;

        // Наименьшая переменная: имя 4, количество измерений 4, атрибуты 8,
        // тип 4, размер 4, смещение 4.
        let nvars = r.list(NC_VARIABLE, 28)?;
        let mut vars = Vec::with_capacity(nvars);
        for _ in 0..nvars {
            let name = r.name()?;
            let nd = r.count(4)?;
            let mut ids = Vec::with_capacity(nd);
            for _ in 0..nd {
                let id = r.u32()? as usize;
                if id >= dims.len() {
                    return Err(error("номер измерения вне списка"));
                }
                ids.push(id);
            }
            let vattrs = r.attrs()?;
            let kind = NcType::from_code(r.u32()?).ok_or_else(|| error("неизвестный тип переменной"))?;
            let _vsize = r.u32()?;
            let begin = match format {
                Format::Classic => u64::from(r.u32()?),
                Format::Offset64 => r.u64()?,
            };
            let record = ids.first().is_some_and(|&id| dims[id].unlimited);
            if ids.iter().skip(1).any(|&id| dims[id].unlimited) {
                return Err(error("измерение записей не первое"));
            }
            vars.push(Variable { name, dims: ids, attrs: vattrs, kind, begin, record });
        }

        let mut file = NcFile { bytes, format, numrecs: 0, dims, attrs, vars, record_size: 0 };
        let parts: Vec<u64> = file
            .vars
            .iter()
            .filter(|v| v.record)
            .map(|v| file.slab_bytes(v))
            .collect::<Option<_>>()
            .ok_or_else(|| error("размер переменной слишком велик"))?;
        // Единственная переменная записей не выравнивается до 4 байтов.
        file.record_size = if parts.len() == 1 {
            parts[0]
        } else {
            parts
                .iter()
                .try_fold(0u64, |sum, &p| sum.checked_add(pad4(p)?))
                .ok_or_else(|| error("размер записи слишком велик"))?
        };
        file.numrecs = if numrecs_raw == STREAMING {
            // Записи до конца файла.
            let first = file.vars.iter().filter(|v| v.record).map(|v| v.begin).min();
            match first {
                Some(begin) if file.record_size > 0 => {
                    ((bytes.len() as u64).saturating_sub(begin) / file.record_size) as usize
                }
                _ => 0,
            }
        } else {
            numrecs_raw as usize
        };
        let numrecs = file.numrecs;
        for d in file.dims.iter_mut().filter(|d| d.unlimited) {
            d.len = numrecs;
        }
        Ok(file)
    }

    /// Измерение по имени.
    pub fn dimension(&self, name: &str) -> Option<&Dimension> {
        self.dims.iter().find(|d| d.name == name)
    }

    /// Переменная по имени.
    pub fn variable(&self, name: &str) -> Option<&Variable> {
        self.vars.iter().find(|v| v.name == name)
    }

    /// Глобальный атрибут по имени.
    pub fn attribute(&self, name: &str) -> Option<&Attribute> {
        find_attr(&self.attrs, name)
    }

    /// Размеры переменной по измерениям, у переменной записей первое -
    /// количество записей.
    pub fn shape(&self, var: &Variable) -> Vec<usize> {
        var.dims.iter().map(|&id| self.dims.get(id).map_or(0, |d| d.len)).collect()
    }

    /// Количество значений одной записи, у обычной переменной - всех.
    fn slab_len(&self, var: &Variable) -> Option<u64> {
        let skip = usize::from(var.record);
        var.dims
            .iter()
            .skip(skip)
            .try_fold(1u64, |n, &id| n.checked_mul(self.dims.get(id)?.len as u64))
    }

    fn slab_bytes(&self, var: &Variable) -> Option<u64> {
        self.slab_len(var)?.checked_mul(var.kind.size() as u64)
    }

    /// Значения переменной как есть, без заполнителей и масштаба.
    pub fn read_raw(&self, name: &str) -> Result<Vec<f64>, NcError> {
        let var = self.variable(name).ok_or_else(|| error(format!("нет переменной {name}")))?;
        self.raw_values(var)
    }

    fn raw_values(&self, var: &Variable) -> Result<Vec<f64>, NcError> {
        let big = || error("данные переменной за концом файла");
        let size = var.kind.size();
        let slab = self.slab_bytes(var).ok_or_else(big)?;
        if slab == 0 {
            return Ok(Vec::new());
        }
        let (count, stride) = if var.record { (self.numrecs as u64, self.record_size) } else { (1, 0) };
        // Конец последнего куска - в пределах файла.
        if count > 0 {
            let last = (count - 1)
                .checked_mul(stride)
                .and_then(|x| x.checked_add(var.begin))
                .and_then(|x| x.checked_add(slab))
                .ok_or_else(big)?;
            if last > self.bytes.len() as u64 {
                return Err(big());
            }
        }
        let total = slab.checked_mul(count).ok_or_else(big)?;
        if total > self.bytes.len() as u64 {
            return Err(big());
        }
        let mut out = Vec::with_capacity((total / size as u64) as usize);
        for rec in 0..count {
            let start = (var.begin + rec * stride) as usize;
            let chunk = self.bytes.get(start..start + slab as usize).ok_or_else(big)?;
            out.extend(chunk.chunks_exact(size).map(|b| var.kind.decode(b)));
        }
        Ok(out)
    }

    /// Значения переменной в f64 по строкам (порядок C). Значения,
    /// равные `_FillValue` или `missing_value`, и NaN дают NaN, остальные
    /// распаковываются как `value * scale_factor + add_offset`.
    /// Заполнитель сравнивается до распаковки, как в соглашении CF.
    /// Без `_FillValue` у float и double пустым считается заполнитель
    /// netCDF по умолчанию.
    pub fn read_f64(&self, name: &str) -> Result<Vec<f64>, NcError> {
        let var = self.variable(name).ok_or_else(|| error(format!("нет переменной {name}")))?;
        let mut values = self.raw_values(var)?;
        let fill = match var.attr("_FillValue").and_then(Attribute::number) {
            Some(f) => Some(f),
            None if matches!(var.kind, NcType::Float | NcType::Double) => Some(DEFAULT_FILL_FLOAT),
            None => None,
        };
        let missing: Vec<f64> = var.attr("missing_value").map(|a| a.numbers().to_vec()).unwrap_or_default();
        let scale = var.attr("scale_factor").and_then(Attribute::number).unwrap_or(1.0);
        let offset = var.attr("add_offset").and_then(Attribute::number).unwrap_or(0.0);
        let near = |x: f64, f: f64| {
            // Заполнитель float в атрибуте double сравнивается в точности
            // float.
            x == f || (var.kind == NcType::Float && x as f32 == f as f32)
        };
        for x in &mut values {
            let empty = x.is_nan() || fill.is_some_and(|f| near(*x, f)) || missing.iter().any(|&m| near(*x, m));
            *x = if empty { f64::NAN } else { *x * scale + offset };
        }
        Ok(values)
    }
}
