// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Электростанции WRI и месторождения USGS: разбор архивов, топливо,
//! группы, размеры. Перенос `tests/test_plants.py` и `test_deposits.py`
//! модуля, без файла профиля: Lite его не пишет.

use std::io::Write;

use data::{csv, deposits as dp, plants};

fn zipped(name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let started = writer.start_file(name, zip::write::SimpleFileOptions::default());
    assert!(started.is_ok());
    assert!(writer.write_all(bytes).is_ok());
    writer.finish().map(|c| c.into_inner()).unwrap_or_default()
}

const PLANTS: &str = "country,country_long,name,gppd_idnr,capacity_mw,latitude,longitude,primary_fuel,other_fuel1,other_fuel2,other_fuel3,commissioning_year,owner,source,url\n\
RUS,Russia,Beloyarsk,X1,1485.0,56.84,61.32,Nuclear,,,,1964.0,Rosenergoatom,src,http://example.org/b\n\
RUS,Russia,Bad,X2,10,,,Gas,,,,,,,\n\
RUS,Russia,Kama HPP,X3,522,58.07,56.33,Hydro,,,,1954,RusHydro,,\n";

#[test]
fn plants_parse_zip_and_fuel() {
    let data = plants::parse(&zipped("global_power_plant_database.csv", PLANTS.as_bytes())).unwrap_or_default();
    // Без координат - пропущена.
    assert_eq!(data.len(), 2);
    assert_eq!(plants::FUELS[data[0].fuel].0, "nuclear");
    assert_eq!(plants::FUELS[data[1].fuel].0, "hydro");
    let first = &data[0];
    assert_eq!(
        [&first.name, &first.country, &first.fuel_name, &first.year, &first.owner, &first.url].map(|s| s.as_str()),
        ["Beloyarsk", "Russia", "Nuclear", "1964", "Rosenergoatom", "http://example.org/b"]
    );
    assert_eq!(data[1].name, "Kama HPP");
    assert_eq!(data.iter().map(|p| p.capacity).collect::<Vec<_>>(), [1485.0, 522.0]);
    // Таблица без архива разбирается так же.
    assert_eq!(plants::parse(PLANTS.as_bytes()).unwrap_or_default().len(), 2);
}

#[test]
fn plant_sizes_grow_with_capacity() {
    let s: Vec<f64> = [0.5, 1.0, 100.0, 5000.0, 20000.0].map(plants::size).to_vec();
    assert_eq!(s[0], plants::SIZE_MIN);
    assert_eq!(s[4], plants::SIZE_MAX);
    assert!(s[1] < s[2] && s[2] < s[3]);
    assert_eq!(plants::fuel_of("Petcoke"), 0);
    assert_eq!(plants::fuel_of("Unknown"), plants::FUELS.len() - 1);
}

const MRDS: &str = "dep_id,site_name,latitude,longitude,country,commod1,commod2,commod3,dep_type,dev_stat\n\
10001,Carlin,40.9,-116.3,United States,\"Gold, Silver\",,,Sediment-hosted,Producer\n\
10002,Без координат,,,Russia,Copper,,,,Occurrence\n\
10003,Sand pit,45.0,-100.0,United States,\"Sand and Gravel\",,,,Past Producer\n";
const MAJOR: &str = "gid,dep_name,country,latitude,longitude,commodity,dep_type,type_detail,model_name\n\
1536,Abakanskoye,Russia,52.4166,90.0333,Iron,Hydrothermal,,\n";

#[test]
fn deposit_groups() {
    let name = |s: &str| dp::GROUPS.get(dp::group_of(s)).map_or("other", |g| g.0);
    assert_eq!(name("Gold, Silver"), "precious");
    assert_eq!(name("Copper, Gold"), "base");
    assert_eq!(name("Sand and Gravel, Construction"), "industrial");
    assert_eq!(name("Geothermal"), "energy");
    assert_eq!(dp::group_of(""), dp::OTHER);
    assert_eq!(dp::color(dp::OTHER), dp::OTHER_COLOR);
}

#[test]
fn deposits_parse_both() {
    let (mrds, major) = (zipped("mrds.csv", MRDS.as_bytes()), zipped("ofr20051294/deposit.csv", MAJOR.as_bytes()));
    let data = dp::parse(Some(&mrds), Some(&major), None).unwrap_or_default();
    // Строка без координат пропущена.
    assert_eq!(data.len(), 3);
    assert!(data[0].major);
    assert_eq!(data[0].fields()[0], "Abakanskoye");
    assert_eq!(data[1].fields()[..3], ["Carlin", "United States", "Gold, Silver"]);
    assert_eq!(data[1].fields()[5], "10001");
    assert_eq!(dp::STATUSES[data[1].status], "Producer");
    assert_eq!(data.iter().map(dp::Deposit::size).collect::<Vec<_>>(), [dp::MAJOR_SIZE, 4.5, 4.0]);
    assert_eq!(data[2].fields()[0], "Sand pit");
}

#[test]
fn deposits_world_operations() {
    // Строка набора minfac в cp1252, как у USGS: знак градуса - 0xB0.
    let mut text = b"position,rec_id,year,country,commodity,location,fac_name,fac_type,dmslat,dmslong,latitude,longitude,precision,mm,op_comp,maininvest,othinvest,status,capacity,units,notes,cite\n".to_vec();
    text.extend_from_slice(b"322,1850,2007,Russia,Bauxite,\"Severoural'sk Region\",,Mine,\"60\xb0 09'  N\",\"060\xb0 00'  E\",60.15,60,A,,Sevuralboksitruda,,,Active,3000,\"thousand metric tons\",,848\n");
    let data = dp::parse(None, None, Some(&zipped("minfac.csv", &text))).unwrap_or_default();
    assert_eq!(data.len(), 1);
    let Some(first) = data.first() else {
        return;
    };
    assert!(first.facility && !first.major);
    let fields = first.fields();
    assert_eq!(fields[0], "Severoural'sk Region");
    assert_eq!(fields[6], "Sevuralboksitruda");
    assert_eq!(fields[7], "3000 thousand metric tons (2007)");
    assert_eq!(fields[8], "A");
    assert_eq!(first.size(), dp::FACILITY_SIZE);
    // Бокситы.
    assert_eq!(dp::GROUPS[first.group].0, "rare");
}

#[test]
fn csv_stream_matches_rows() {
    let text = "a,b\n\"x, \"\"y\"\"\",2\n\n3,\"line\nbreak\"\n";
    let mut streamed: Vec<Vec<String>> = Vec::new();
    let done = csv::records(text.as_bytes(), |cells| streamed.push(cells.iter().map(|c| String::from_utf8_lossy(c).into_owned()).collect()));
    assert!(done.is_ok());
    assert_eq!(streamed, csv::rows(text));
    assert_eq!(csv::text_1252(&[vec![0x80, 0xb0]], Some(0)), "€°");
}
