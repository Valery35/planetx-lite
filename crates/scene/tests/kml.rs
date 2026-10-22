// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! KML и KMZ: папки, геометрии, стили, виды, запись и чтение. Перенос
//! `tests/test_kml.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use scene::features::{Highlight, Kind};
use scene::kml::{self, Folder, Item, Place};
use scene::lookat;
use serde_json::{Value, json};

fn close(a: f64, b: f64, places: i32) -> bool {
    (a - b).abs() < 0.5 * 10f64.powi(-places)
}

fn folder(item: &Item) -> &Folder {
    match item {
        Item::Folder(f) => f,
        other => panic!("не папка: {other:?}"),
    }
}

fn place(item: &Item) -> &Place {
    match item {
        Item::Place(p) => p,
        other => panic!("не метка: {other:?}"),
    }
}

fn overlay(item: &Item) -> &kml::Overlay {
    match item {
        Item::Overlay(o) => o,
        other => panic!("не наложение: {other:?}"),
    }
}

fn read(text: &str) -> Folder {
    kml::read_kml(text.as_bytes(), "").unwrap()
}

fn places(tree: &Folder) -> Vec<&Place> {
    tree.children.iter().map(place).collect()
}

fn write(children: Vec<Place>) -> String {
    kml::write_kml(&Folder::with("x", children.into_iter().map(Item::Place).collect()))
}

const SAMPLE: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<kml xmlns="http://www.opengis.net/kml/2.2"
     xmlns:gx="http://www.google.com/kml/ext/2.2">
<Document>
  <name>Поездка</name>
  <Style id="red"><LineStyle><color>ff0000ff</color><width>4</width>
    </LineStyle><PolyStyle><color>7f00ff00</color></PolyStyle></Style>
  <StyleMap id="map"><Pair><key>normal</key><styleUrl>#red</styleUrl>
    </Pair><Pair><key>highlight</key><styleUrl>#none</styleUrl></Pair>
  </StyleMap>
  <Placemark><name>Пермь</name><description>город &amp; порт</description>
    <LookAt><longitude>56.23</longitude><latitude>58.01</latitude>
      <heading>30</heading><tilt>45</tilt><range>2500</range></LookAt>
    <Point><coordinates>56.2294,58.0105,0</coordinates></Point>
  </Placemark>
  <Folder><name>Маршруты</name><visibility>0</visibility>
    <Placemark><name>Путь</name><styleUrl>#map</styleUrl>
      <LineString><coordinates>
        56.0,58.0,120 56.1,58.1,130
      </coordinates></LineString></Placemark>
    <Folder><name>Участки</name>
      <Placemark><name>Поле</name><styleUrl>#red</styleUrl>
        <Polygon><outerBoundaryIs><LinearRing><coordinates>
          56,58 56.2,58 56.2,58.1 56,58
        </coordinates></LinearRing></outerBoundaryIs></Polygon>
      </Placemark>
      <Placemark><name>Двойная</name><MultiGeometry>
        <Point><coordinates>56.5,58.5</coordinates></Point>
        <LineString><coordinates>56.5,58.5 56.6,58.6</coordinates>
        </LineString></MultiGeometry></Placemark>
    </Folder>
  </Folder>
</Document>
</kml>"##;

// Чтение.

#[test]
fn document_becomes_root_with_folders() {
    let tree = read(SAMPLE);
    assert_eq!(tree.name, "Поездка");
    let names: Vec<&str> = tree
        .children
        .iter()
        .map(|c| match c {
            Item::Folder(f) => f.name.as_str(),
            Item::Place(p) => p.name.as_str(),
            Item::Overlay(o) => o.name.as_str(),
        })
        .collect();
    assert_eq!(names, ["Пермь", "Маршруты"]);
    let routes = folder(&tree.children[1]);
    assert!(!routes.visible);
    assert_eq!(place(&routes.children[0]).name, "Путь");
    assert_eq!(folder(&routes.children[1]).name, "Участки");
}

#[test]
fn point_with_view_and_description() {
    let tree = read(SAMPLE);
    let perm = place(&tree.children[0]);
    assert_eq!(perm.kind, Kind::Point);
    assert_eq!(perm.points, [(58.0105, 56.2294)]);
    // Точка взгляда LookAt не совпадает с меткой, как в Google Earth.
    assert_eq!(perm.view, Some([58.01, 56.23, 2500.0, 30.0, 45.0]));
    assert_eq!(perm.description, "город & порт");
}

#[test]
fn view_of_polygon_and_without_look_point() {
    let data = r#"<kml><Document><Placemark><name>A</name>
        <LookAt><heading>10</heading><tilt>20</tilt><range>900</range>
        </LookAt><Polygon><outerBoundaryIs><LinearRing><coordinates>
        56,58 56.2,58 56.2,58.1 56,58</coordinates></LinearRing>
        </outerBoundaryIs></Polygon></Placemark></Document></kml>"#;
    let field = read(data);
    // Без точки взгляда ею становится первая вершина метки.
    assert_eq!(place(&field.children[0]).view, Some([58.0, 56.0, 900.0, 10.0, 20.0]));
}

#[test]
fn style_through_stylemap() {
    let tree = read(SAMPLE);
    let path = place(&folder(&tree.children[1]).children[0]);
    assert_eq!(path.kind, Kind::Line);
    assert_eq!(path.color, [255, 0, 0, 255]); // ff0000ff - красный
    assert_eq!(path.width, 4.0);
    assert_eq!(path.points, [(58.0, 56.0), (58.1, 56.1)]);
}

#[test]
fn polygon_ring_and_fill() {
    let tree = read(SAMPLE);
    let field = place(&folder(&folder(&tree.children[1]).children[1]).children[0]);
    assert_eq!(field.kind, Kind::Polygon);
    assert_eq!(field.points.len(), 3); // замыкающая точка снята
    assert_eq!(field.fill, Some([0, 255, 0, 127]));
}

#[test]
fn multigeometry_splits() {
    let tree = read(SAMPLE);
    let parts: Vec<(&str, Kind)> =
        folder(&folder(&tree.children[1]).children[1]).children[1..].iter().map(|p| (place(p).name.as_str(), place(p).kind)).collect();
    assert_eq!(parts, [("Двойная", Kind::Point), ("Двойная", Kind::Line)]);
    assert_eq!(tree.places().len(), 5);
}

#[test]
fn root_name_from_document_then_file() {
    assert_eq!(kml::read_kml(SAMPLE.as_bytes(), "файл").unwrap().name, "Поездка");
    let bare = b"<kml><Document><Placemark><Point><coordinates>1,2</coordinates></Point></Placemark></Document></kml>";
    assert_eq!(kml::read_kml(bare, "файл").unwrap().name, "файл");
}

#[test]
fn bad_input() {
    for data in [&b"not xml"[..], b"<html/>", b"PK\x03\x04broken"] {
        assert!(kml::read_file(data, "").is_err());
    }
}

#[test]
fn entities_are_refused() {
    let bomb = r#"<?xml version="1.0"?><!DOCTYPE kml [
        <!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;&a;">]>
        <kml><Document><name>&b;</name></Document></kml>"#;
    assert!(kml::read_kml(bomb.as_bytes(), "").is_err());
}

// Запись.

#[test]
fn round_trip_kml_and_kmz() {
    let tree = read(SAMPLE);
    for data in [kml::write_kml(&tree).into_bytes(), kml::write_kmz(&tree).unwrap()] {
        let again = kml::read_file(&data, "Поездка").unwrap();
        assert_eq!(again.places().len(), 5);
        let perm = place(&again.children[0]);
        assert_eq!(perm.view, Some([58.01, 56.23, 2500.0, 30.0, 45.0]));
        assert!(close(perm.points[0].0, 58.0105, 7));
        assert_eq!(perm.description, "город & порт");
        let routes = folder(&again.children[1]);
        assert_eq!(place(&routes.children[0]).color, [255, 0, 0, 255]);
        assert!(!routes.visible);
    }
}

#[test]
fn height_and_extrude() {
    let data = r#"<kml><Document><Placemark><name>Wall</name><LineString>
        <extrude>1</extrude><altitudeMode>relativeToGround</altitudeMode>
        <coordinates>56,58,120 56.1,58.1,120</coordinates></LineString>
        </Placemark><Placemark><name>Sea</name><Point>
        <altitudeMode>absolute</altitudeMode>
        <coordinates>56,58,300</coordinates></Point></Placemark>
        </Document></kml>"#;
    let tree = read(data);
    let (wall, sea) = (place(&tree.children[0]), place(&tree.children[1]));
    assert_eq!((wall.height, wall.extrude), (120.0, true));
    // absolute - высота над уровнем моря, режим переносится.
    assert_eq!((sea.height, sea.extrude, sea.mode.as_str()), (300.0, false, "absolute"));
    let again = read(&write(vec![wall.clone()]));
    let again = place(&again.children[0]);
    assert_eq!((again.height, again.extrude), (120.0, true));
}

#[test]
fn altitude_modes_round_trip() {
    // Пять режимов высоты Google Earth переживают запись и чтение,
    // режимы дна - gx:altitudeMode.
    let cases = [
        ("clampToGround", 0.0),
        ("clampToSeaFloor", 0.0),
        ("relativeToGround", 50.0),
        ("relativeToSeaFloor", 20.0),
        ("absolute", -40.0),
    ];
    let marks: Vec<Place> = cases
        .iter()
        .map(|(mode, h)| {
            let mut p = Place::new(mode, Kind::Point, vec![(58.0, 56.0)]);
            p.height = *h;
            p.mode = (*mode).into();
            p
        })
        .collect();
    let text = write(marks);
    assert!(text.contains("<gx:altitudeMode>clampToSeaFloor"));
    let again = read(&text);
    let got: Vec<(&str, f64)> = places(&again).iter().map(|p| (p.mode.as_str(), p.height)).collect();
    assert_eq!(got, cases);
}

#[test]
fn polygon_holes_round_trip() {
    // Дыры innerBoundaryIs читаются и пишутся обратно. Прежде они
    // отбрасывались, озеро внутри контура закрашивалось.
    let text = r#"<kml xmlns="http://www.opengis.net/kml/2.2"><Placemark>
      <name>Двор</name><Polygon>
      <outerBoundaryIs><LinearRing><coordinates>
        56,58 56.4,58 56.4,58.4 56,58.4 56,58</coordinates>
      </LinearRing></outerBoundaryIs>
      <innerBoundaryIs><LinearRing><coordinates>
        56.1,58.1 56.2,58.1 56.2,58.2 56.1,58.1</coordinates>
      </LinearRing></innerBoundaryIs>
      <innerBoundaryIs><LinearRing><coordinates>
        56.3,58.3 56.35,58.3 56.35,58.35</coordinates>
      </LinearRing></innerBoundaryIs></Polygon></Placemark></kml>"#;
    let tree = read(text);
    let mark = place(&tree.children[0]);
    assert_eq!(mark.points.len(), 4);
    let holes = mark.holes.as_ref().unwrap();
    assert_eq!(holes.iter().map(Vec::len).collect::<Vec<_>>(), [3, 3]);
    assert_eq!(holes[0][0], (58.1, 56.1));
    let written = write(vec![mark.clone()]);
    assert_eq!(written.matches("<innerBoundaryIs>").count(), 2);
    let again = read(&written);
    assert_eq!(place(&again.children[0]).holes.as_ref().unwrap().iter().map(Vec::len).collect::<Vec<_>>(), [3, 3]);
}

#[test]
fn extended_data_fields_round_trip() {
    // Поля Data и SchemaData/SimpleData переносятся парами, служебные
    // поля PlanetX в них не входят.
    let text = r##"<kml xmlns="http://www.opengis.net/kml/2.2"><Document>
      <Schema name="wells" id="wells"><SimpleField name="depth"
        type="double"/></Schema>
      <Placemark><name>Скв. 7</name><ExtendedData>
        <Data name="code"><displayName>Код</displayName>
          <value>C-07</value></Data>
        <Data name="planetx:flow"><value>1</value></Data>
        <SchemaData schemaUrl="#wells">
          <SimpleData name="depth">412.5</SimpleData>
          <SimpleData name="owner">A &amp; B</SimpleData>
        </SchemaData></ExtendedData>
        <Point><coordinates>56,58</coordinates></Point></Placemark>
      </Document></kml>"##;
    let tree = read(text);
    let mark = place(&tree.children[0]);
    let expected: Vec<(String, String)> =
        [("Код", "C-07"), ("depth", "412.5"), ("owner", "A & B")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    assert_eq!(mark.data.as_ref(), Some(&expected));
    let again = read(&write(vec![mark.clone()]));
    assert_eq!(place(&again.children[0]).data, mark.data);
}

#[test]
fn label_style_and_icon_scale_round_trip() {
    // LabelStyle (цвет, размер, 0 - без подписи) и scale значка читаются
    // из стиля по styleUrl и пишутся обратно.
    let text = r##"<kml xmlns="http://www.opengis.net/kml/2.2"><Document>
      <Style id="s"><IconStyle><scale>1.5</scale></IconStyle>
        <LabelStyle><color>ff0000ff</color><scale>0.8</scale>
        </LabelStyle></Style>
      <Placemark><name>А</name><styleUrl>#s</styleUrl>
        <Point><coordinates>56,58</coordinates></Point></Placemark>
      <Placemark><name>Б</name><Style><LabelStyle><scale>0</scale>
        </LabelStyle></Style>
        <Point><coordinates>56,58</coordinates></Point></Placemark>
      </Document></kml>"##;
    let tree = read(text);
    let got = |p: &Place| (p.icon_scale, p.label_color, p.label_scale);
    let expected = [(1.5, Some([255, 0, 0, 255]), 0.8), (1.0, None, 0.0)];
    assert_eq!(places(&tree).iter().map(|p| got(p)).collect::<Vec<_>>(), expected);
    let again = read(&write(places(&tree).into_iter().cloned().collect()));
    assert_eq!(places(&again).iter().map(|p| got(p)).collect::<Vec<_>>(), expected);
}

#[test]
fn style_map_highlight_round_trip() {
    // Вариант highlight StyleMap - вид точки под курсором: ссылкой на Style,
    // своим Style внутри Pair и StyleMap внутри метки. Обычный вид берётся
    // из normal, запись - StyleMap обратно.
    let text = r##"<kml xmlns="http://www.opengis.net/kml/2.2"><Document>
      <Style id="n"><IconStyle><color>ff00ffff</color><scale>1.1
        </scale></IconStyle></Style>
      <Style id="h"><IconStyle><scale>1.3</scale></IconStyle>
        <LabelStyle><color>ff0000ff</color></LabelStyle></Style>
      <StyleMap id="m"><Pair><key>normal</key><styleUrl>#n</styleUrl>
        </Pair><Pair><key>highlight</key><styleUrl>#h</styleUrl>
        </Pair></StyleMap>
      <Placemark><name>А</name><styleUrl>#m</styleUrl>
        <Point><coordinates>56,58</coordinates></Point></Placemark>
      <Placemark><name>Б</name><StyleMap><Pair><key>highlight</key>
        <Style><IconStyle><scale>2</scale></IconStyle></Style></Pair>
        </StyleMap><Point><coordinates>56,58</coordinates></Point>
      </Placemark>
      <Placemark><name>В</name><styleUrl>#n</styleUrl>
        <Point><coordinates>56,58</coordinates></Point></Placemark>
      </Document></kml>"##;
    let tree = read(text);
    let marks = places(&tree);
    let (a, b, c) = (marks[0], marks[1], marks[2]);
    assert_eq!(a.icon_scale, 1.1);
    assert_eq!(a.color, [255, 255, 0, 255]);
    assert_eq!(a.highlight, Some(Highlight { scale: Some(1.3), label: Some([255, 0, 0, 255]), ..Default::default() }));
    assert_eq!(b.highlight, Some(Highlight { scale: Some(2.0), ..Default::default() }));
    assert_eq!(c.highlight, None);
    let again = read(&write(marks.iter().map(|p| (*p).clone()).collect()));
    let again = places(&again);
    let expected = [
        Some(Highlight {
            color: Some([255, 255, 0, 255]),
            scale: Some(1.3),
            icon: Some(a.icon.clone()),
            label: Some([255, 0, 0, 255]),
            label_scale: Some(1.0),
        }),
        Some(Highlight { color: Some(b.color), scale: Some(2.0), icon: Some(b.icon.clone()), ..Default::default() }),
        None,
    ];
    assert_eq!(again.iter().map(|p| p.highlight.clone()).collect::<Vec<_>>(), expected);
    assert_eq!(again[0].icon_scale, 1.1);
}

#[test]
fn balloon_and_icon_heading_round_trip() {
    // BalloonStyle и heading значка читаются из стиля и пишутся обратно.
    let text = r##"<kml xmlns="http://www.opengis.net/kml/2.2"><Document>
      <Style id="w"><IconStyle><heading>135</heading></IconStyle>
        <BalloonStyle><text><![CDATA[<b>$[name]</b> $[speed]]]>
        </text></BalloonStyle></Style>
      <Placemark><name>Ветер</name><styleUrl>#w</styleUrl>
        <Point><coordinates>56,58</coordinates></Point></Placemark>
      </Document></kml>"##;
    let tree = read(text);
    let mark = place(&tree.children[0]);
    assert_eq!(mark.icon_heading, 135.0);
    assert_eq!(mark.balloon, "<b>$[name]</b> $[speed]");
    let again = read(&write(vec![mark.clone()]));
    let again = place(&again.children[0]);
    assert_eq!((again.icon_heading, again.balloon.as_str()), (135.0, "<b>$[name]</b> $[speed]"));
}

#[test]
fn animated_update_and_bounce_round_trip() {
    // gx:AnimatedUpdate: цели - метка, общий стиль и точка, они сводятся
    // к id меток. flyToMode bounce - номер позы. Запись и чтение обратно
    // дают те же изменения.
    let text = r##"<kml xmlns="http://www.opengis.net/kml/2.2"
      xmlns:gx="http://www.google.com/kml/ext/2.2"><Document>
      <Style id="s"><IconStyle id="si"><scale>1</scale></IconStyle>
      </Style>
      <Placemark id="a"><name>А</name><styleUrl>#s</styleUrl>
        <Point id="ap"><coordinates>56,58</coordinates></Point>
      </Placemark>
      <Placemark id="b"><name>Б</name><visibility>0</visibility>
        <Point><coordinates>56.1,58</coordinates></Point></Placemark>
      <gx:Tour><name>Тур</name><gx:Playlist>
        <gx:FlyTo><gx:duration>0</gx:duration><LookAt>
          <longitude>56</longitude><latitude>58</latitude>
          <range>5000</range></LookAt></gx:FlyTo>
        <gx:AnimatedUpdate><gx:duration>2</gx:duration><Update>
          <targetHref/><Change>
          <IconStyle targetId="si"><scale>3</scale></IconStyle>
          <Point targetId="ap"><coordinates>56.05,58</coordinates>
          </Point></Change></Update></gx:AnimatedUpdate>
        <gx:FlyTo><gx:duration>4</gx:duration>
          <gx:flyToMode>bounce</gx:flyToMode><LookAt>
          <longitude>57</longitude><latitude>58</latitude>
          <range>5000</range></LookAt></gx:FlyTo>
        <gx:AnimatedUpdate><gx:duration>0</gx:duration>
          <gx:delayedStart>1</gx:delayedStart><Update><targetHref/>
          <Change><Placemark targetId="b"><visibility>1</visibility>
          </Placemark></Change></Update></gx:AnimatedUpdate>
      </gx:Playlist></gx:Tour></Document></kml>"##;
    let tree = read(text);
    let marks = places(&tree);
    let (a, b, tour) = (marks[0], marks[1], marks[2]);
    assert_eq!((a.kml_id.as_str(), b.kml_id.as_str()), ("a", "b"));
    let extra = tour.tour_extra.as_ref().unwrap();
    assert_eq!(extra.bounce, [1]);
    let got: Vec<(f64, f64, Vec<String>, Value)> =
        extra.updates.iter().map(|u| (u.t, u.d, u.ids.clone(), Value::Object(u.set.clone()))).collect();
    let ids = |s: &str| vec![s.to_string()];
    assert_eq!(
        got,
        [
            (0.0, 2.0, ids("a"), json!({"icon_scale": 3.0})),
            (0.0, 2.0, ids("a"), json!({"points": [[58.0, 56.05]]})),
            (5.0, 0.0, ids("b"), json!({"visible": true})),
        ]
    );
    let again = read(&write(marks.iter().map(|p| (*p).clone()).collect()));
    let extra = places(&again)[2].tour_extra.clone().unwrap();
    assert_eq!(extra.bounce, [1]);
    let mut got: Vec<(f64, f64, Vec<String>, Vec<String>)> =
        extra.updates.iter().map(|u| (u.t, u.d, u.ids.clone(), u.set.keys().cloned().collect())).collect();
    got.sort_by(|x, y| x.partial_cmp(y).unwrap());
    assert_eq!(
        got,
        [
            (0.0, 2.0, ids("a"), ids("icon_scale")),
            (0.0, 2.0, ids("a"), ids("points")),
            (5.0, 0.0, ids("b"), ids("visible")),
        ]
    );
}

#[test]
fn model_on_track_round_trip() {
    // gx:Track с Model - модель едет по треку: путь со временем и углами
    // в Overlay::track, линии пути нет. Запись даёт тот же gx:Track.
    let text = r#"<kml xmlns="http://www.opengis.net/kml/2.2"
      xmlns:gx="http://www.google.com/kml/ext/2.2"><Placemark>
      <name>Борт</name><gx:Track>
      <altitudeMode>absolute</altitudeMode>
      <when>2026-10-20T10:00:00Z</when><when>2026-10-20T10:01:00Z</when>
      <gx:coord>56.0 58.0 1000</gx:coord>
      <gx:coord>56.1 58.0 1500</gx:coord>
      <gx:angles>90 5 10</gx:angles><gx:angles>95 0 0</gx:angles>
      <Model><Link><href>files/plane.dae</href></Link>
        <Scale><x>2</x><y>2</y><z>2</z></Scale></Model>
      </gx:Track></Placemark></kml>"#;
    let tree = read(text);
    assert_eq!(tree.children.len(), 1);
    let item = overlay(&tree.children[0]);
    assert_eq!(item.kind(), scene::overlays::Kind::Model);
    assert_eq!(item.href, "files/plane.dae");
    assert_eq!(item.overlay.mode, "absolute");
    assert_eq!(item.overlay.scale, [2.0, 2.0, 2.0]);
    let rows = item.overlay.track.clone().unwrap();
    let tail: Vec<[f64; 6]> =
        rows.iter().map(|r| [r.lat, r.lon, r.alt, r.heading.unwrap_or(f64::NAN), r.tilt, r.roll]).collect();
    assert_eq!(tail, [[58.0, 56.0, 1000.0, 90.0, 5.0, 10.0], [58.0, 56.1, 1500.0, 95.0, 0.0, 0.0]]);
    assert_eq!(rows[1].t - rows[0].t, 60.0);
    assert_eq!(item.time, Some(("2026-10-20T10:00:00Z".to_string(), "2026-10-20T10:01:00Z".to_string())));
    let written = kml::model_kml(item, 0, "files/plane.dae", None);
    let again = read(&format!(
        r#"<kml xmlns="http://www.opengis.net/kml/2.2" xmlns:gx="http://www.google.com/kml/ext/2.2">{written}</kml>"#
    ));
    assert_eq!(overlay(&again.children[0]).overlay.track, Some(rows));
}

#[test]
fn camera_view() {
    // Camera метки и шага тура становится видом: отвесно вниз - точка под
    // глазом на расстоянии высоты, наклон 60° - точка впереди по курсу.
    let text = r#"<kml xmlns="http://www.opengis.net/kml/2.2"
      xmlns:gx="http://www.google.com/kml/ext/2.2"><Document>
      <Placemark><name>Отвесно</name><Camera><longitude>56</longitude>
        <latitude>58</latitude><altitude>1000</altitude>
        <heading>0</heading><tilt>0</tilt></Camera>
        <Point><coordinates>56,58</coordinates></Point></Placemark>
      <Placemark><name>Наклон</name><Camera><longitude>56</longitude>
        <latitude>58</latitude><altitude>1000</altitude>
        <heading>90</heading><tilt>60</tilt><roll>5</roll></Camera>
        <Point><coordinates>56,58</coordinates></Point></Placemark>
      <gx:Tour><name>Тур</name><gx:Playlist>
        <gx:FlyTo><gx:duration>0</gx:duration><Camera>
          <longitude>56</longitude><latitude>58</latitude>
          <altitude>2000</altitude><tilt>0</tilt></Camera></gx:FlyTo>
        <gx:FlyTo><gx:duration>3</gx:duration><Camera>
          <longitude>56.1</longitude><latitude>58</latitude>
          <altitude>2000</altitude><tilt>0</tilt></Camera></gx:FlyTo>
      </gx:Playlist></gx:Tour></Document></kml>"#;
    let tree = read(text);
    let marks = places(&tree);
    let [lat, lon, distance, _, tilt] = marks[0].view.unwrap();
    assert!(close(lat, 58.0, 6));
    assert!(close(lon, 56.0, 6));
    assert!(close(distance, 1000.0, 1));
    assert!(close(tilt, 0.0, 3));
    let [lat, lon, distance, heading, tilt] = marks[1].view.unwrap();
    assert!((distance - 2000.0).abs() <= 1.0);
    assert!(lon > 56.0); // впереди на восток
    assert!(close(lat, 58.0, 3));
    assert!((heading - 90.0).abs() <= 0.1);
    assert!((tilt - 60.0).abs() <= 0.1);
    let samples = marks[2].tour.as_ref().unwrap();
    assert_eq!(samples.len(), 2);
    assert!(close(samples[1][3], 2000.0, 1));
}

#[test]
fn colors() {
    assert_eq!(kml::color_kml([255, 128, 0, 200]), "c80080ff");
    assert_eq!(kml::kml_color("c80080ff"), Some([255, 128, 0, 200]));
    assert_eq!(kml::kml_color("zz"), None);
}

// Слои вида остановки тура - ExtendedData planetx:layers.

fn layers(value: Value) -> lookat::Layers {
    value.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

#[test]
fn round_trip_with_flow() {
    let mut mark = Place::new("Остановка", Kind::Line, vec![(58.0, 56.0), (58.1, 56.1)]);
    mark.view = Some([58.0, 56.0, 3000.0, 10.0, 40.0]);
    mark.flow = true;
    mark.layers = Some(layers(json!({"gravity": true, "quakes": false})));
    let text = write(vec![mark]);
    assert_eq!(text.matches("<ExtendedData>").count(), 1);
    let again = read(&text);
    let again = place(&again.children[0]);
    assert_eq!(again.layers, Some(layers(json!({"gravity": true, "quakes": false}))));
    assert!(again.flow);
}

#[test]
fn pace_round_trip() {
    let mut mark = Place::new("Остановка", Kind::Point, vec![(58.0, 56.0)]);
    mark.layers = Some(layers(json!({"grid": true})));
    mark.pace = Some((Some(7.5), 0.5));
    let text = write(vec![mark]);
    assert_eq!(text.matches("<ExtendedData>").count(), 1);
    let again = read(&text);
    let again = place(&again.children[0]);
    assert_eq!(again.pace, Some((Some(7.5), 0.5)));
    assert_eq!(again.layers, Some(layers(json!({"grid": true}))));
    // Только скорость, пауза тура.
    let mut mark = Place::new("Б", Kind::Point, vec![(58.0, 56.0)]);
    mark.pace = Some((None, 2.0));
    let again = read(&write(vec![mark]));
    assert_eq!(place(&again.children[0]).pace, Some((None, 2.0)));
}

#[test]
fn view_state_round_trip() {
    // Сектор разреза, земля над моделью и непрозрачности строк переживают
    // KML, неверные значения отбрасываются.
    let mut mark = Place::new("Остановка", Kind::Point, vec![(58.0, 56.0)]);
    mark.layers = Some(layers(json!({
        "cutaway": true, "@wedge": [120, 160, 0, 38.5],
        "@ground": {"opacity": 0.4, "cut": true, "under": 0, "junk": "x"},
        "@opacity": {"quakes": 0.5, "grid": "bad"},
        "@unknown": 5
    })));
    let again = read(&write(vec![mark]));
    let again = place(&again.children[0]);
    assert_eq!(
        again.layers,
        Some(layers(json!({
            "cutaway": true, "@wedge": [120.0, 160.0, 0.0, 38.5],
            "@ground": {"opacity": 0.4, "cut": true, "under": false},
            "@opacity": {"quakes": 0.5}
        })))
    );
    // Без состояния - только строки.
    assert_eq!(lookat::switches(again.layers.as_ref()), [("cutaway".to_string(), true)].into());
}

#[test]
fn no_layers_no_data() {
    let text = write(vec![Place::new("Точка", Kind::Point, vec![(58.0, 56.0)])]);
    assert!(!text.contains("ExtendedData"));
    assert_eq!(place(&read(&text).children[0]).layers, None);
}

// Значки и время.

const TIMED: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<kml xmlns="http://www.opengis.net/kml/2.2"
     xmlns:gx="http://www.google.com/kml/ext/2.2">
<Document>
  <Style id="hike"><IconStyle><color>ff00ff00</color><Icon>
    <href>http://maps.google.com/mapfiles/kml/shapes/hiker.png</href>
  </Icon></IconStyle></Style>
  <Placemark><name>Поход</name><styleUrl>#hike</styleUrl>
    <TimeStamp><when>2026-09-30</when></TimeStamp>
    <LookAt><gx:TimeSpan><begin>2026-09-01</begin><end>2026-09-30</end>
      </gx:TimeSpan><longitude>56.2</longitude><latitude>58.0</latitude>
      <range>900</range></LookAt>
    <Point><coordinates>56.2,58.0</coordinates></Point></Placemark>
  <Folder><name>Лето</name>
    <TimeSpan><begin>2026-06</begin><end>2026-08</end></TimeSpan>
    <Placemark><name>Кнопка</name><Style><IconStyle><Icon>
      <href>http://maps.google.com/mapfiles/kml/pushpin/red-pushpin.png</href>
      </Icon></IconStyle></Style>
      <Point><coordinates>56.3,58.1</coordinates></Point></Placemark>
    <Placemark><name>Своё</name>
      <TimeStamp><when>2026-07-15T10:00:00Z</when></TimeStamp>
      <Style><IconStyle><Icon><href>http://example.com/x.png</href>
      </Icon></IconStyle></Style>
      <Point><coordinates>56.4,58.2</coordinates></Point></Placemark>
  </Folder>
</Document>
</kml>"##;

fn by_name(tree: &Folder, name: &str) -> Place {
    tree.places().into_iter().find(|p| p.name == name).cloned().unwrap()
}

fn time(a: &str, b: &str) -> Option<(String, String)> {
    Some((a.to_string(), b.to_string()))
}

#[test]
fn icons() {
    let tree = read(TIMED);
    assert_eq!(by_name(&tree, "Поход").icon, "hiker");
    assert_eq!(by_name(&tree, "Поход").color, [0, 255, 0, 255]);
    assert_eq!(by_name(&tree, "Кнопка").icon, "pushpin");
    assert_eq!(by_name(&tree, "Своё").icon, "dot");
}

#[test]
fn times() {
    let tree = read(TIMED);
    let hike = by_name(&tree, "Поход");
    assert_eq!(hike.time, time("2026-09-30", "2026-09-30"));
    assert_eq!(hike.view_time, time("2026-09-01", "2026-09-30"));
    // Время папки достаётся метке без своего времени.
    assert_eq!(by_name(&tree, "Кнопка").time, time("2026-06", "2026-08"));
    assert_eq!(by_name(&tree, "Своё").time, time("2026-07-15T10:00:00Z", "2026-07-15T10:00:00Z"));
}

#[test]
fn round_trip() {
    let tree = read(TIMED);
    let again = read(&kml::write_kml(&tree));
    for mark in tree.places() {
        let other = by_name(&again, &mark.name);
        assert_eq!(other.icon, mark.icon, "{}", mark.name);
        assert_eq!(other.time, mark.time, "{}", mark.name);
        assert_eq!(other.view_time, mark.view_time, "{}", mark.name);
    }
}

// gx:Track - линия с промежутком времени от первого момента до последнего.

const TRACKS: &str = r#"<kml xmlns="http://www.opengis.net/kml/2.2"
 xmlns:gx="http://www.google.com/kml/ext/2.2"><Document>
<Placemark><name>Плавание</name><gx:Track>
<when>1492-08-03</when><when>1492-09-06</when><when>1492-10-12</when>
<gx:coord>-6.89 37.23 0</gx:coord><gx:coord>-17.1 28.1 0</gx:coord>
<gx:coord>-74.5 24.0 0</gx:coord></gx:Track></Placemark>
<Placemark><name>Со своим временем</name>
<TimeSpan><begin>1493</begin><end>1496</end></TimeSpan>
<gx:MultiTrack><gx:Track><when>1493-09-25</when><when>1493-11-03</when>
<gx:coord>-6.3 36.5 0</gx:coord><gx:coord>-61.4 15.4 0</gx:coord>
</gx:Track></gx:MultiTrack></Placemark>
<Placemark><name>Без моментов</name><Track>
<coord>10 20</coord><coord>11 21</coord><coord>плохо</coord></Track>
</Placemark></Document></kml>"#;

#[test]
fn track_is_line_with_span() {
    let voyage = by_name(&read(TRACKS), "Плавание");
    assert_eq!(voyage.kind, Kind::Line);
    assert_eq!(voyage.points, [(37.23, -6.89), (28.1, -17.1), (24.0, -74.5)]);
    assert_eq!(voyage.time, time("1492-08-03", "1492-10-12"));
}

#[test]
fn own_time_wins_and_multitrack() {
    let second = by_name(&read(TRACKS), "Со своим временем");
    assert_eq!(second.kind, Kind::Line);
    assert_eq!(second.time, time("1493", "1496"));
}

#[test]
fn track_without_when_and_prefix() {
    let bare = by_name(&read(TRACKS), "Без моментов");
    assert_eq!(bare.points, [(20.0, 10.0), (21.0, 11.0)]);
    assert_eq!(bare.time, None);
}

// Геометрия в папке без Placemark, как её пишут модели помощника.

const BARE: &str = r##"<kml xmlns="http://www.opengis.net/kml/2.2"><Document>
<name>Путешествия Колумба</name>
<Style id="voyage1"><LineStyle><color>ff0000ff</color><width>3</width>
</LineStyle></Style>
<Folder><name>Общее описание</name>
<Placemark><name>Колумб</name><description>Мореплаватель.</description>
</Placemark></Folder>
<Folder><name>1-е путешествие (1492-1493)</name>
<TimeSpan><begin>1492-08-03</begin><end>1493-03-15</end></TimeSpan>
<LineString styleUrl="#voyage1"><coordinates>
-6.30,36.98 -13.85,28.68 -74.67,24.08</coordinates></LineString>
<Placemark><name>Палос</name><Point><coordinates>-6.89,37.23,0
</coordinates></Point></Placemark>
<Polygon><name>Эспаньола</name><outerBoundaryIs><LinearRing><coordinates>
-74,18 -68,18 -68,20 -74,20 -74,18</coordinates></LinearRing>
</outerBoundaryIs><TimeStamp><when>1492-12-05</when></TimeStamp></Polygon>
</Folder></Document></kml>"##;

#[test]
fn bare_line_takes_folder_name_time_and_style() {
    let tree = read(BARE);
    let lines: Vec<&Place> = tree.places().into_iter().filter(|p| p.kind == Kind::Line).collect();
    assert_eq!(lines.len(), 1);
    let line = lines[0];
    assert_eq!(line.name, "1-е путешествие (1492-1493)");
    assert_eq!(line.points, [(36.98, -6.3), (28.68, -13.85), (24.08, -74.67)]);
    assert_eq!(line.time, time("1492-08-03", "1493-03-15"));
    assert_eq!(line.color, [255, 0, 0, 255]);
    assert_eq!(line.width, 3.0);
}

#[test]
fn bare_polygon_keeps_own_name_and_time() {
    let tree = read(BARE);
    let areas: Vec<&Place> = tree.places().into_iter().filter(|p| p.kind == Kind::Polygon).collect();
    assert_eq!(areas.len(), 1);
    assert_eq!(areas[0].name, "Эспаньола");
    assert_eq!(areas[0].time, time("1492-12-05", "1492-12-05"));
    assert_eq!(areas[0].points.len(), 4);
}

#[test]
fn point_in_placemark_stays() {
    let tree = read(BARE);
    let mut kinds: Vec<&str> = tree.places().iter().map(|p| p.kind.as_str()).collect();
    kinds.sort();
    assert_eq!(kinds, ["line", "point", "polygon"]);
}

// Вольности, которые пишут модели помощника.

fn doc(body: &str) -> Folder {
    read(&format!(r#"<kml xmlns="http://www.opengis.net/kml/2.2"><Document>{body}</Document></kml>"#))
}

#[test]
fn spaces_after_commas() {
    let tree = doc(
        "<Placemark><name>П</name><Point><coordinates>-6.932, 37.181, 0\
         </coordinates></Point></Placemark><Placemark><name>Л</name>\
         <LineString><coordinates>-15.5, 28.0, 0 -30.0, 25.0, 0\
         </coordinates></LineString></Placemark>",
    );
    let points: Vec<Vec<(f64, f64)>> = tree.places().iter().map(|p| p.points.clone()).collect();
    assert_eq!(points, [vec![(37.181, -6.932)], vec![(28.0, -15.5), (25.0, -30.0)]]);
}

#[test]
fn polygon_without_outer_boundary() {
    let tree = doc(
        "<Placemark><Polygon><LinearRing><coordinates>0,0 1,0 1,1 0,0\
         </coordinates></LinearRing></Polygon></Placemark>\
         <Placemark><Polygon><coordinates>0,0 2,0 2,2 0,0\
         </coordinates></Polygon></Placemark>",
    );
    let marks = tree.places();
    assert_eq!(marks.iter().map(|p| p.kind).collect::<Vec<_>>(), [Kind::Polygon, Kind::Polygon]);
    assert_eq!(marks[1].points.len(), 3);
}

#[test]
fn time_inside_geometry() {
    let tree = doc(
        "<Folder><TimeSpan><begin>1492</begin></TimeSpan><Placemark>\
         <LineString><TimeSpan><begin>1492-08-03</begin><end>\
         1492-10-12</end></TimeSpan><coordinates>-6,37 -16,28\
         </coordinates></LineString></Placemark></Folder>",
    );
    assert_eq!(tree.places()[0].time, time("1492-08-03", "1492-10-12"));
}

#[test]
fn lon_over_180_and_unicode_minus() {
    let tree = doc(
        "<Placemark><Point><coordinates>200,10</coordinates></Point>\
         </Placemark><Placemark><Point><coordinates>\u{2212}74,18\
         </coordinates></Point></Placemark>",
    );
    let first: Vec<(f64, f64)> = tree.places().iter().map(|p| p.points[0]).collect();
    assert_eq!(first, [(10.0, -160.0), (18.0, -74.0)]);
}

#[test]
fn lookat_only_placemark_is_point() {
    let tree = doc(
        "<Placemark><name>Вид</name><LookAt><longitude>56.2\
         </longitude><latitude>58.0</latitude><range>1000</range>\
         </LookAt></Placemark>",
    );
    let mark = tree.places()[0];
    assert_eq!((mark.kind, mark.points.clone()), (Kind::Point, vec![(58.0, 56.2)]));
}

#[test]
fn html_in_text_does_not_lose_document() {
    let tree = doc(
        "<name>Битвы &amp; осады</name><Placemark><name>Ватерлоо\
         &nbsp;1815</name><description>Итог<br>войн & мира\
         </description><Point><coordinates>4.4,50.7</coordinates>\
         </Point></Placemark>",
    );
    let mark = tree.places()[0];
    assert_eq!(mark.name, "Ватерлоо\u{a0}1815");
    assert_eq!(mark.description, "Итог\nвойн & мира");
    assert_eq!(tree.name, "Битвы & осады");
}

#[test]
fn dtd_entities_still_refused() {
    let bomb = r#"<?xml version="1.0"?><!DOCTYPE k [<!ENTITY a "aaaa">]><kml><Document><name>&a;</name></Document></kml>"#;
    assert!(kml::read_kml(bomb.as_bytes(), "").is_err());
}

// Записанный тур как gx:Tour туда и обратно, gx:Wait держит позу.

#[test]
fn tour_round_trip() {
    let samples = vec![
        [0.0, 58.0, 56.0, 3000.0, 10.0, 30.0],
        [1.5, 58.01, 56.02, 2500.0, 20.0, 45.0],
        [4.0, 58.02, 56.03, 2000.0, 30.0, 60.0],
    ];
    let mut mark = Place::new("Облёт", Kind::Line, samples.iter().map(|s| (s[1], s[2])).collect());
    mark.tour = Some(samples.clone());
    let text = kml::write_kml(&Folder::with("Туры", vec![Item::Place(mark)]));
    assert!(text.contains("<gx:Tour>"));
    let tree = read(&text);
    let back = tree.places()[0];
    assert_eq!(back.name, "Облёт");
    for (got, want) in back.tour.as_ref().unwrap().iter().zip(&samples) {
        for (a, b) in got.iter().zip(want) {
            assert!(close(*a, *b, 2));
        }
    }
}

#[test]
fn wait() {
    let data = r#"<kml xmlns="http://www.opengis.net/kml/2.2"
      xmlns:gx="http://www.google.com/kml/ext/2.2"><Document>
      <gx:Tour><name>Ждать</name><gx:Playlist>
        <gx:FlyTo><gx:duration>2</gx:duration><LookAt>
          <longitude>56</longitude><latitude>58</latitude>
          <range>1000</range></LookAt></gx:FlyTo>
        <gx:Wait><gx:duration>3</gx:duration></gx:Wait>
        <gx:FlyTo><gx:duration>1</gx:duration><Camera>
          <longitude>57</longitude><latitude>59</latitude>
          </Camera></gx:FlyTo>
      </gx:Playlist></gx:Tour></Document></kml>"#;
    let tree = read(data);
    let tour = tree.places()[0].tour.clone().unwrap();
    // Шаг с Camera - поза над своей точкой, с 20 октября 2026 года.
    assert_eq!(tour.iter().map(|s| s[0]).collect::<Vec<_>>(), [2.0, 5.0, 6.0]);
    assert_eq!(tour[0][1..], tour[1][1..]);
    assert!(close(tour[2][1], 59.0, 5));
    assert!(close(tour[2][2], 57.0, 5));
}

// 3D-путь и 3D-многоугольник - altitudeMode absolute с высотой у каждой
// вершины, отрезки не садятся на рельеф.

#[test]
fn space_round_trip() {
    let mut path = Place::new("Крыши", Kind::Line, vec![(58.0, 56.0), (58.001, 56.002)]);
    path.alts = Some(vec![152.5, 187.25]);
    let mut roof = Place::new("Скат", Kind::Polygon, vec![(58.0, 56.0), (58.0, 56.001), (58.001, 56.001)]);
    roof.alts = Some(vec![140.0, 140.0, 151.5]);
    let text = kml::write_kml(&Folder::with("3D", vec![Item::Place(path), Item::Place(roof)]));
    assert!(text.contains("<altitudeMode>absolute</altitudeMode>"));
    assert!(!text.contains("<tessellate>"));
    let tree = read(&text);
    assert_eq!(by_name(&tree, "Крыши").alts, Some(vec![152.5, 187.25]));
    assert_eq!(by_name(&tree, "Скат").alts, Some(vec![140.0, 140.0, 151.5]));
}

#[test]
fn flat_objects_have_no_heights() {
    let line = Place::new("Путь", Kind::Line, vec![(58.0, 56.0), (58.1, 56.1)]);
    let text = kml::write_kml(&Folder::with("2D", vec![Item::Place(line)]));
    assert!(text.contains("<tessellate>1</tessellate>"));
    assert_eq!(read(&text).places()[0].alts, None);
    // Высота есть не у каждой вершины - не 3D-объект.
    let data = r#"<kml xmlns="http://www.opengis.net/kml/2.2"><Placemark>
      <LineString><altitudeMode>absolute</altitudeMode>
      <coordinates>56,58,100 56.1,58.1</coordinates></LineString>
      </Placemark></kml>"#;
    assert_eq!(read(data).places()[0].alts, None);
}

// Свойства папки, как в окне папки Google Earth.

#[test]
fn folder_round_trip() {
    let mut inner = Folder::with("Б", vec![Item::Place(Place::new("т", Kind::Point, vec![(1.0, 2.0)]))]);
    inner.radio = true;
    let mut closed = Folder::new("В");
    closed.expandable = false;
    let mut top = Folder::with("А", vec![Item::Folder(inner), Item::Folder(closed)]);
    top.description = "о папке".into();
    top.view = Some([58.0, 56.2, 5000.0, 30.0, 45.0]);
    let back = read(&kml::write_kml(&top));
    assert_eq!(back.description, "о папке");
    let view = back.view.unwrap().map(|v| scene_round(v, 6));
    assert_eq!(view, [58.0, 56.2, 5000.0, 30.0, 45.0]);
    let (b, c) = (folder(&back.children[0]), folder(&back.children[1]));
    assert_eq!((b.radio, b.expandable), (true, true));
    assert_eq!((c.radio, c.expandable), (false, false));
    assert_eq!((back.radio, back.expandable), (false, true));
}

fn scene_round(v: f64, digits: i32) -> f64 {
    let m = 10f64.powi(digits);
    (v * m).round() / m
}

#[test]
fn list_style_by_url() {
    let data = r##"<kml xmlns="http://www.opengis.net/kml/2.2"><Document><Style id="r"><ListStyle><listItemType>radioFolder</listItemType></ListStyle></Style><Folder><name>x</name><styleUrl>#r</styleUrl></Folder><Folder><name>y</name></Folder></Document></kml>"##;
    let tree = read(data);
    assert!(folder(&tree.children[0]).radio);
    assert!(!folder(&tree.children[1]).radio);
}

// Сетевые ссылки.

const USGS_LINK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<kml xmlns="http://www.opengis.net/kml/2.2"><Document><name>Earthquakes</name>
<NetworkLink><visibility>1</visibility><name>USGS 4.5+ Week</name>
<Link><href>https://earthquake.usgs.gov/feed/4.5_week_depth.kml</href>
<refreshMode>onInterval</refreshMode><refreshInterval>60</refreshInterval>
</Link></NetworkLink>
<NetworkLink><name>Tectonic Plates</name><visibility>0</visibility>
<Link><href>https://earthquake.usgs.gov/learn/plate-boundaries.kmz</href>
</Link></NetworkLink></Document></kml>"#;

#[test]
fn usgs_link_file() {
    let tree = kml::read_kml(USGS_LINK.as_bytes(), "file").unwrap();
    assert_eq!(tree.name, "Earthquakes");
    let (week, plates) = (folder(&tree.children[0]), folder(&tree.children[1]));
    assert_eq!(week.link, "https://earthquake.usgs.gov/feed/4.5_week_depth.kml");
    assert_eq!(week.refresh, 60.0);
    assert!(week.visible);
    assert_eq!(plates.refresh, 0.0);
    assert!(!plates.visible);
    assert!(week.children.is_empty());
}

#[test]
fn single_link_is_not_unwrapped() {
    let data = br#"<kml xmlns="http://www.opengis.net/kml/2.2"><NetworkLink><name>One</name><Url><href>a.kml</href></Url></NetworkLink></kml>"#;
    let tree = kml::read_kml(data, "file").unwrap();
    assert_eq!(tree.name, "file");
    assert_eq!(folder(&tree.children[0]).link, "a.kml");
}

#[test]
fn round_trip_keeps_link_without_children() {
    let mut tree = kml::read_kml(USGS_LINK.as_bytes(), "file").unwrap();
    let extra = read(USGS_LINK).children[1].clone();
    if let Item::Folder(week) = &mut tree.children[0] {
        week.children.push(extra);
    }
    let text = kml::write_kml(&tree);
    assert_eq!(text.matches("<NetworkLink>").count(), 2);
    assert!(!text.contains("<Folder>"));
    let again = read(&text);
    let links = |f: &Folder| f.children.iter().map(|c| folder(c).link.clone()).collect::<Vec<_>>();
    assert_eq!(links(&again), links(&tree));
    assert_eq!(folder(&again.children[0]).refresh, 60.0);
}

// Region с Lod у метки и папки: чтение и запись туда и обратно.

const REGION_DOC: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<kml xmlns="http://www.opengis.net/kml/2.2"><Document><name>R</name>
<Folder><name>Ближние</name>
  <Region><LatLonAltBox><north>58.1</north><south>57.9</south>
    <east>56.4</east><west>56.0</west></LatLonAltBox>
    <Lod><minLodPixels>128</minLodPixels></Lod></Region>
  <Placemark><name>Пермь</name>
    <Region><LatLonAltBox><north>58.02</north><south>58.0</south>
      <east>56.24</east><west>56.22</west></LatLonAltBox>
      <Lod><minLodPixels>64</minLodPixels><maxLodPixels>2048</maxLodPixels>
      </Lod></Region>
    <Point><coordinates>56.2294,58.0105,0</coordinates></Point>
  </Placemark>
  <Placemark><name>Без рамки</name><Region><Lod><minLodPixels>8
    </minLodPixels></Lod></Region>
    <Point><coordinates>56,58,0</coordinates></Point></Placemark>
</Folder></Document></kml>"#;

#[test]
fn region_round_trip() {
    let tree = read(REGION_DOC);
    let group = folder(&tree.children[0]);
    let region = group.region.unwrap();
    assert_eq!(region.min_lod, 128.0);
    assert_eq!(region.max_lod, -1.0);
    let (mark, bare) = (place(&group.children[0]), place(&group.children[1]));
    let own = mark.region.unwrap();
    assert_eq!(own.rect(), [58.02, 58.0, 56.24, 56.22]);
    assert_eq!((own.min_lod, own.max_lod), (64.0, 2048.0));
    assert_eq!(bare.region, None);
    let again = read(&kml::write_kml(&tree));
    let again = folder(&again.children[0]);
    assert_eq!(again.region, group.region);
    assert_eq!(place(&again.children[0]).region, mark.region);
}
