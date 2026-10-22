// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Значки меток, как в Google Earth, перенос `core/icons.py` модуля.
//!
//! Картинки модуль берёт из библиотеки SVG, которая входит в QGIS 3.36
//! и 4. Значки Google Earth под условиями Google, копировать их нельзя.
//! В KML значок пишется адресом стандартного значка Google Earth той же
//! темы, так файл открывается в Google Earth со своим значком. Адрес
//! из чужого файла узнаётся по имени файла картинки. Незнакомый адрес
//! даёт кружок.
//!
//! Геодезическому пункту и карьеру точной пары в Google Earth нет, им
//! отданы «треугольник» и «падающие камни».

pub const GE_BASE: &str = "http://maps.google.com/mapfiles/kml/";
pub const DEFAULT: &str = "dot";

/// Номер значка, путь SVG в библиотеке QGIS и значок Google Earth.
/// У кружка SVG нет, его рисует система надписей.
pub const ICONS: [(&str, Option<&str>, &str); 35] = [
    ("dot", None, "shapes/placemark_circle.png"),
    ("pushpin", Some("symbol/red-marker.svg"), "pushpin/ylw-pushpin.png"),
    ("flag", Some("gpsicons/flag.svg"), "shapes/flag.png"),
    ("info", Some("amenity/amenity_information.svg"), "shapes/info-i.png"),
    ("camera", Some("gpsicons/camera.svg"), "shapes/camera.png"),
    ("house", Some("gpsicons/house.svg"), "shapes/homegardenbusiness.png"),
    ("peak", Some("symbol/poi_peak.svg"), "shapes/mountains.png"),
    ("tree", Some("gpsicons/tree.svg"), "shapes/parks.png"),
    ("water", Some("symbol/fountain.svg"), "shapes/water.png"),
    ("survey", Some("amenity/amenity_survey_point.svg"), "shapes/triangle.png"),
    ("quarry", Some("symbol/landuse_quary.svg"), "shapes/falling_rocks.png"),
    ("hiker", Some("gpsicons/walker.svg"), "shapes/hiker.png"),
    ("camp", Some("accommodation/accommodation_camping.svg"), "shapes/campground.png"),
    ("car", Some("gpsicons/car.svg"), "shapes/cabs.png"),
    ("bus", Some("transport/transport_bus_stop.svg"), "shapes/bus.png"),
    ("rail", Some("transport/transport_train_station.svg"), "shapes/rail.png"),
    ("tram", Some("transport/transport_tram_stop.svg"), "shapes/tram.png"),
    ("airport", Some("transport/transport_airport.svg"), "shapes/airports.png"),
    ("ship", Some("gpsicons/boat.svg"), "shapes/ferry.png"),
    ("fuel", Some("transport/transport_fuel.svg"), "shapes/gas_stations.png"),
    ("parking", Some("transport/transport_parking.svg"), "shapes/parking_lot.png"),
    ("hospital", Some("health/health_hospital.svg"), "shapes/hospitals.png"),
    ("pharmacy", Some("health/health_pharmacy.svg"), "shapes/pharmacy_rx.png"),
    ("school", Some("symbol/education_school.svg"), "shapes/schools.png"),
    ("church", Some("religion/place_of_worship_christian.svg"), "shapes/church.png"),
    ("museum", Some("landmark/tourism=museum.svg"), "shapes/arts.png"),
    ("hotel", Some("accommodation/accommodation_hotel.svg"), "shapes/lodging.png"),
    ("food", Some("entertainment/amenity=restaurant.svg"), "shapes/dining.png"),
    ("cafe", Some("entertainment/amenity=cafe.svg"), "shapes/coffee.png"),
    ("shop", Some("shopping/shopping_supermarket.svg"), "shapes/shopping.png"),
    ("police", Some("amenity/amenity_police.svg"), "shapes/police.png"),
    ("fire", Some("amenity/amenity_firestation.svg"), "shapes/firedept.png"),
    ("post", Some("amenity/amenity_post_office.svg"), "shapes/post_office.png"),
    ("ski", Some("sport/sport_skiing_downhill.svg"), "shapes/ski.png"),
    ("swim", Some("sport/sport_swimming_outdoor.svg"), "shapes/swimming.png"),
];

/// Номера значков по порядку.
pub fn ids() -> impl Iterator<Item = &'static str> {
    ICONS.iter().map(|(icon, _, _)| *icon)
}

/// Путь SVG значка в библиотеке QGIS или None.
pub fn svg(icon: &str) -> Option<&'static str> {
    ICONS.iter().find(|(id, _, _)| *id == icon).and_then(|(_, svg, _)| *svg)
}

/// Адрес значка Google Earth для KML. Незнакомый номер - кружок.
pub fn href(icon: &str) -> String {
    let ge = ICONS.iter().find(|(id, _, _)| *id == icon).or_else(|| ICONS.first()).map_or("", |(_, _, ge)| *ge);
    format!("{GE_BASE}{ge}")
}

/// Имя файла в пути с косыми чертами.
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Значок по адресу из KML. Кнопки любого цвета - кнопка, незнакомый
/// адрес - кружок.
pub fn from_href(link: Option<&str>) -> &'static str {
    let Some(link) = link.filter(|l| !l.is_empty()) else {
        return DEFAULT;
    };
    let clean = link.trim().split('?').next().unwrap_or_default();
    let name = basename(clean).to_lowercase();
    if let Some((icon, _, _)) = ICONS.iter().find(|(_, _, ge)| basename(ge) == name) {
        return icon;
    }
    if name.ends_with("-pushpin.png") {
        return "pushpin";
    }
    DEFAULT
}

/// Известный номер значка или кружок.
pub fn normal(icon: &str) -> &'static str {
    ids().find(|id| *id == icon).unwrap_or(DEFAULT)
}
