// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Системы координат и пересчёт между ними, одинаково для обеих сборок.
//!
//! С признаком `import` (PlanetX Lite) за ними стоит движок terrio, он
//! знает все коды EPSG и сверен с GDAL (doc/PLAN.md, «Проверка terrio»).
//! Без него (RustPlanetX, doc/EDITIONS.md) - свой пересчёт
//! [`crate::proj`]: широта с долготой, Web Mercator, UTM на WGS84, NAD83,
//! ETRS89, GDA и Lambert-93, без сдвига датума. Растр в другой системе
//! RustPlanetX получает от RustGIS уже пересчитанным.
//!
//! Порядок осей у географической системы - долгота, широта, как у GDAL
//! с традиционным порядком.

/// Система координат. Неизвестная - пересчёта нет.
pub struct SpatialRef {
    #[cfg(feature = "import")]
    inner: terrio::crs::SpatialRef,
    #[cfg(not(feature = "import"))]
    inner: Option<crate::proj::Crs>,
}

/// Пересчёт из одной системы в другую.
pub struct Transformer {
    #[cfg(feature = "import")]
    inner: terrio::crs::Transformer,
    #[cfg(not(feature = "import"))]
    src: crate::proj::Crs,
    #[cfg(not(feature = "import"))]
    dst: crate::proj::Crs,
}

#[cfg(feature = "import")]
impl SpatialRef {
    /// Система по коду EPSG, None - код неизвестен.
    pub fn from_epsg(code: u32) -> Option<Self> {
        terrio::crs::SpatialRef::from_epsg(code).ok().map(|inner| Self { inner })
    }

    /// Система растра или слоя terrio.
    pub fn from_terrio(inner: terrio::crs::SpatialRef) -> Self {
        Self { inner }
    }

    pub fn is_unknown(&self) -> bool {
        self.inner.is_unknown()
    }

    /// Код EPSG, если он известен.
    pub fn epsg(&self) -> Option<u32> {
        self.inner.authority().filter(|(a, _)| a.eq_ignore_ascii_case("EPSG")).map(|(_, c)| c)
    }
}

#[cfg(feature = "import")]
impl Transformer {
    /// Пересчёт из src в dst, None - одна из систем неизвестна.
    pub fn new(src: &SpatialRef, dst: &SpatialRef) -> Option<Self> {
        if src.is_unknown() || dst.is_unknown() {
            return None;
        }
        terrio::crs::Transformer::new(&src.inner, &dst.inner).ok().map(|inner| Self { inner })
    }

    /// Точка (x, y) системы src в системе dst. None - точка вне области.
    pub fn transform(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        self.inner.transform(x, y).ok().filter(|(a, b)| a.is_finite() && b.is_finite())
    }
}

#[cfg(not(feature = "import"))]
impl SpatialRef {
    /// Система по коду EPSG, None - код неизвестен.
    pub fn from_epsg(code: u32) -> Option<Self> {
        crate::proj::from_epsg(code).map(|c| Self { inner: Some(c) })
    }

    /// Неизвестная система.
    pub fn unknown() -> Self {
        Self { inner: None }
    }

    pub fn is_unknown(&self) -> bool {
        self.inner.is_none()
    }

    /// Код EPSG, если он известен.
    pub fn epsg(&self) -> Option<u32> {
        self.inner.as_ref().and_then(|c| c.epsg)
    }
}

#[cfg(not(feature = "import"))]
impl Transformer {
    /// Пересчёт из src в dst, None - одна из систем неизвестна.
    pub fn new(src: &SpatialRef, dst: &SpatialRef) -> Option<Self> {
        Some(Self { src: src.inner.clone()?, dst: dst.inner.clone()? })
    }

    /// Точка (x, y) системы src в системе dst. None - точка вне области.
    pub fn transform(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let (lat, lon) = self.src.to_lat_lon(x, y)?;
        self.dst.from_lat_lon(lat, lon)
    }
}

/// Пересчёт из системы EPSG from в систему EPSG to.
pub fn between(from: u32, to: u32) -> Option<Transformer> {
    Transformer::new(&SpatialRef::from_epsg(from)?, &SpatialRef::from_epsg(to)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Пересчёт туда и обратно возвращает точку: широта и долгота
    /// в UTM 37N и Web Mercator.
    #[test]
    fn round_trip_through_utm_and_mercator() {
        let (lon, lat) = (37.62, 55.75);
        for code in [32637, 3857] {
            let (Some(to), Some(back)) = (between(4326, code), between(code, 4326)) else {
                unreachable!("нет пересчёта для EPSG:{code}");
            };
            let Some((x, y)) = to.transform(lon, lat) else { unreachable!("точка вне EPSG:{code}") };
            let Some((lon2, lat2)) = back.transform(x, y) else { unreachable!("обратно вне EPSG:{code}") };
            assert!((lon2 - lon).abs() < 1e-9 && (lat2 - lat).abs() < 1e-9, "EPSG:{code}: {lon2} {lat2}");
        }
    }
}
