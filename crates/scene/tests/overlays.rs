// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Наложения картинок: углы, место на экране, фото, KML и KMZ, рамка
//! 3D-модели. Перенос `tests/test_overlays.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Cursor, Write};

use globe_core::ellipsoid::Ellipsoid;
use scene::kml;
use scene::overlays::{self as ov, Handle, Kind, Overlay};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n0000000000000000";
const GROUND: &str = r#"<kml xmlns="http://www.opengis.net/kml/2.2"
 xmlns:gx="http://www.google.com/kml/ext/2.2"><Document><name>D</name>
<GroundOverlay><name>Box</name><color>80ffffff</color>
<drawOrder>2</drawOrder><Icon><href>files/a.png</href></Icon>
<LatLonBox><north>11</north><south>10</south><east>21</east>
<west>20</west></LatLonBox></GroundOverlay>
<GroundOverlay><name>Quad</name><Icon><href>./files/a.png</href></Icon>
<gx:LatLonQuad><coordinates>20,10 21,10 21.5,11 20.2,11</coordinates>
</gx:LatLonQuad></GroundOverlay>
<ScreenOverlay><name>Logo</name><Icon><href>files/a.png</href></Icon>
<overlayXY x="0" y="1" xunits="fraction" yunits="fraction"/>
<screenXY x="10" y="10" xunits="pixels" yunits="insetPixels"/>
<size x="0.2" y="0" xunits="fraction" yunits="fraction"/>
</ScreenOverlay>
<PhotoOverlay><name>Photo</name><Icon><href>files/a.png</href></Icon>
<Camera><longitude>56</longitude><latitude>58</latitude>
<altitude>500</altitude><heading>30</heading><tilt>80</tilt>
<roll>0</roll></Camera><ViewVolume><leftFov>-20</leftFov>
<rightFov>20</rightFov><bottomFov>-15</bottomFov><topFov>15</topFov>
<near>100</near></ViewVolume><Point><coordinates>56,58,500</coordinates>
</Point></PhotoOverlay>
</Document></kml>"#;

/// Равенство до places знаков после запятой, как assertAlmostEqual.
fn close(a: f64, b: f64, places: i32) -> bool {
    (a - b).abs() < 0.5 * 10f64.powi(-places)
}

fn kmz(text: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    writer.start_file("doc.kml", options).unwrap();
    writer.write_all(text.as_bytes()).unwrap();
    for (name, data) in files {
        writer.start_file(*name, options).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn xy(x: f64, y: f64, xu: &str, yu: &str) -> ov::Xy {
    (x, y, xu.into(), yu.into())
}

// Углы.

#[test]
fn box_without_rotation() {
    let corners = ov::box_corners(11.0, 10.0, 21.0, 20.0, 0.0);
    assert_eq!(corners, vec![(10.0, 20.0), (10.0, 21.0), (11.0, 21.0), (11.0, 20.0)]);
    assert_eq!(ov::corners_box(&corners), Some([11.0, 10.0, 21.0, 20.0]));
}

#[test]
fn rotation_turns_counterclockwise() {
    let corners = ov::box_corners(1.0, -1.0, 1.0, -1.0, 90.0);
    // Левый нижний угол после поворота на 90° против часовой - справа
    // снизу.
    let (lat, lon) = corners[0];
    assert!(close(lat, -1.0, 6));
    assert!(close(lon, 1.0, 6));
    assert_eq!(ov::corners_box(&ov::box_corners(1.0, -1.0, 1.0, -1.0, 30.0)), None);
}

#[test]
fn affine_of_rotated_box() {
    let corners = ov::box_corners(11.0, 10.0, 21.0, 20.0, 15.0);
    let [x0, a, b, y0, d, e] = ov::affine(&corners).unwrap();
    // Нижний правый угол - сумма обеих сторон.
    assert!(close(x0 + a + b, corners[1].1, 7));
    assert!(close(y0 + d + e, corners[1].0, 7));
}

#[test]
fn affine_rejects_free_quad() {
    let quad = [(10.0, 20.0), (10.0, 21.0), (11.0, 21.5), (11.0, 20.2)];
    assert_eq!(ov::affine(&quad), None);
}

#[test]
fn box_kept_with_rotation() {
    let mut overlay = Overlay::new(Kind::Ground);
    overlay.set_box([11.0, 10.0, 21.0, 20.0, 30.0]);
    assert_eq!(overlay.corners.len(), 4);
    let back = ov::from_params(Kind::Ground, &overlay.params(), None);
    assert_eq!(back.latlon_box, Some([11.0, 10.0, 21.0, 20.0, 30.0]));
    let corners = overlay.corners.clone();
    overlay.set_corners(corners);
    assert_eq!(overlay.latlon_box, None);
}

#[test]
fn drag_corner_keeps_opposite() {
    let frame = [11.0, 10.0, 21.0, 20.0, 0.0];
    let [north, south, east, west, _] = ov::box_drag(frame, Handle::Corner, 2, 12.0, 22.0, false);
    assert!(close(south, 10.0, 6));
    assert!(close(west, 20.0, 6));
    assert!(close(north, 12.0, 6));
    assert!(close(east, 22.0, 4));
}

#[test]
fn drag_side_and_centre() {
    let frame = [11.0, 10.0, 21.0, 20.0, 0.0];
    let north = ov::box_drag(frame, Handle::Side, 2, 12.0, 20.5, false);
    assert!(close(north[0], 12.0, 6));
    assert!(close(north[1], 10.0, 6));
    assert!(close(north[2], 21.0, 6));
    let moved = ov::box_drag(frame, Handle::Center, 0, 0.5, 0.5, false);
    assert!(close((moved[0] + moved[1]) / 2.0, 0.5, 6));
    let wide = ov::box_drag(frame, Handle::Corner, 1, 9.0, 22.0, true);
    assert!(close(wide[0], 12.0, 4));
}

#[test]
fn rotate_handle() {
    let frame = [1.0, -1.0, 1.0, -1.0, 0.0];
    // Ромб к западу от середины - поворот на 90° против часовой.
    let turned = ov::box_drag(frame, Handle::Rotate, 0, 0.0, -1.0, false);
    assert!(close(turned[4], 90.0, 6));
    let handles = ov::box_handles(turned);
    assert!(close(handles.rotate.0, 0.0, 6));
    assert!(handles.rotate.1 < 0.0);
}

#[test]
fn fit_keeps_aspect() {
    let corners = ov::fit_corners(0.0, 0.0, 2000.0, 2.0);
    let [north, south, east, west] = ov::corners_box(&corners).unwrap();
    let (my, mx) = ov::metres_per_degree(0.0);
    assert!(close((east - west) * mx, 2000.0, 3));
    assert!(close((north - south) * my, 1000.0, 3));
}

// Картинка на экране.

#[test]
fn top_left_inset() {
    let mut overlay = Overlay::new(Kind::Screen);
    overlay.overlay_xy = xy(0.0, 1.0, "fraction", "fraction");
    overlay.screen_xy = xy(10.0, 10.0, "pixels", "insetPixels");
    overlay.size = xy(0.2, 0.0, "fraction", "fraction");
    assert_eq!(ov::screen_rect(&overlay, 1000.0, 500.0, 200.0, 100.0), (10.0, 10.0, 200.0, 100.0));
}

#[test]
fn native_size_centre() {
    let mut overlay = Overlay::new(Kind::Screen);
    overlay.overlay_xy = xy(0.5, 0.5, "fraction", "fraction");
    overlay.screen_xy = xy(0.5, 0.5, "fraction", "fraction");
    assert_eq!(ov::screen_rect(&overlay, 800.0, 600.0, 100.0, 50.0), (350.0, 275.0, 100.0, 50.0));
}

// Фото.

#[test]
fn corners_at_near_distance() {
    let camera = [58.0, 56.0, 500.0, 30.0, 80.0, 0.0];
    let corners = ov::photo_corners(&camera, [-20.0, 20.0, -15.0, 15.0], 100.0);
    let eye = Ellipsoid::wgs84().geodetic_to_ecef(58.0, 56.0, 500.0);
    let centre = corners.iter().fold(globe_core::DVec3::ZERO, |a, b| a + *b) / 4.0;
    assert!(close(centre.distance(eye), 100.0, 6));
    let width = corners[1].distance(corners[0]);
    assert!(close(width, 200.0 * 20f64.to_radians().tan(), 6));
}

#[test]
fn looking_down_is_below_eye() {
    let camera = [0.0, 0.0, 1000.0, 0.0, 0.0, 0.0];
    let e = Ellipsoid::wgs84();
    for c in ov::photo_corners(&camera, [-10.0, 10.0, -10.0, 10.0], 100.0) {
        let (_, _, height) = e.ecef_to_geodetic(c);
        assert!((height - 900.0).abs() <= 1.0, "{height}");
    }
}

#[test]
fn pose_looks_at_ground() {
    let [_, lon, distance, _, tilt] = ov::photo_pose(&[0.0, 0.0, 1000.0, 90.0, 45.0, 0.0], 100.0);
    assert!(close(distance, 1000.0 * 2f64.sqrt(), 6));
    assert!(lon > 0.0);
    assert!(close(tilt, 45.0, 7));
}

#[test]
fn params_round_trip() {
    let mut overlay = Overlay::new(Kind::Photo);
    overlay.camera = Some([1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    overlay.fov = [-1.0, 1.0, -2.0, 2.0];
    overlay.near = 50.0;
    overlay.color = [255, 255, 255, 128];
    let back = ov::from_params(Kind::Photo, &overlay.params(), None);
    assert_eq!(back.camera, Some([1.0, 2.0, 3.0, 4.0, 5.0, 6.0]));
    assert_eq!(back.fov, [-1.0, 1.0, -2.0, 2.0]);
    assert_eq!(back.color, [255, 255, 255, 128]);
}

// KML и KMZ.

#[test]
fn read_kmz_with_images() {
    let tree = kml::read_file(&kmz(GROUND, &[("files/a.png", PNG)]), "").unwrap();
    let items = tree.overlays();
    assert_eq!(items.iter().map(|i| i.kind()).collect::<Vec<_>>(), [Kind::Ground, Kind::Ground, Kind::Screen, Kind::Photo]);
    assert!(items.iter().all(|i| i.image.as_deref() == Some(PNG)));
    let (bx, quad, screen, photo) = (items[0], items[1], items[2], items[3]);
    assert_eq!(bx.overlay.order, 2);
    assert_eq!(bx.overlay.latlon_box, Some([11.0, 10.0, 21.0, 20.0, 0.0]));
    assert_eq!(quad.overlay.latlon_box, None);
    assert_eq!(bx.overlay.color[3], 128);
    assert_eq!(quad.overlay.corners[2], (11.0, 21.5));
    assert_eq!(screen.overlay.screen_xy, xy(10.0, 10.0, "pixels", "insetPixels"));
    assert_eq!(photo.overlay.camera.unwrap()[..3], [58.0, 56.0, 500.0]);
    // Наложения не входят в метки.
    assert!(tree.places().is_empty());
}

#[test]
fn kmz_round_trip() {
    let tree = kml::read_file(&kmz(GROUND, &[("files/a.png", PNG)]), "").unwrap();
    let back = kml::read_file(&kml::write_kmz(&tree).unwrap(), "").unwrap();
    let items = back.overlays();
    assert_eq!(items.len(), 4);
    assert!(items.iter().all(|i| i.image.as_deref() == Some(PNG)));
    let before = tree.overlays();
    assert_eq!(items[0].overlay.corners, before[0].overlay.corners);
    for (a, b) in items[1].overlay.corners.iter().zip(&before[1].overlay.corners) {
        assert!(close(a.0, b.0, 7));
        assert!(close(a.1, b.1, 7));
    }
    assert_eq!(items[3].overlay.fov, [-20.0, 20.0, -15.0, 15.0]);
}

#[test]
fn plain_kml_keeps_href() {
    let tree = kml::read_file(GROUND.as_bytes(), "").unwrap();
    let item = tree.overlays()[0];
    assert_eq!(item.image, None);
    assert_eq!(item.href, "files/a.png");
}

// Обновление картинки по ссылке через промежуток.

fn linked() -> String {
    GROUND.replace(
        "<Icon><href>files/a.png</href></Icon>\n<LatLonBox>",
        "<Icon><href>https://example.org/a.png</href><refreshMode>onInterval</refreshMode>\
         <refreshInterval>60</refreshInterval></Icon>\n<LatLonBox>",
    )
}

#[test]
fn interval_has_floor() {
    assert_eq!(ov::refresh_interval(Some(0.0)), 0.0);
    assert_eq!(ov::refresh_interval(None), 0.0);
    assert_eq!(ov::refresh_interval(Some(-5.0)), 0.0);
    assert_eq!(ov::refresh_interval("x".parse().ok()), 0.0);
    assert_eq!(ov::refresh_interval(Some(f64::INFINITY)), 0.0);
    assert_eq!(ov::refresh_interval(Some(4.0)), ov::MIN_REFRESH);
    assert_eq!(ov::refresh_interval(Some(300.0)), 300.0);
}

#[test]
fn params_keep_interval() {
    let mut overlay = Overlay::new(Kind::Screen);
    overlay.refresh = ov::refresh_interval(Some(120.0));
    let back = ov::from_params(Kind::Screen, &overlay.params(), None);
    assert_eq!(back.refresh, 120.0);
    assert!(!Overlay::new(Kind::Screen).params().contains("refresh"));
}

#[test]
fn kml_round_trip() {
    let text = linked();
    assert!(text.contains("onInterval"));
    let tree = kml::read_file(text.as_bytes(), "").unwrap();
    assert_eq!(tree.overlays()[0].overlay.refresh, 60.0);
    assert_eq!(tree.overlays()[1].overlay.refresh, 0.0);
    let written = kml::write_kml(&tree);
    assert_eq!(written.matches("<refreshMode>onInterval").count(), 1);
    let back = kml::read_file(written.as_bytes(), "").unwrap();
    assert_eq!(back.overlays()[0].overlay.refresh, 60.0);
}

#[test]
fn kml_default_interval_raised_to_floor() {
    let text = linked().replace("<refreshInterval>60</refreshInterval>", "");
    let tree = kml::read_file(text.as_bytes(), "").unwrap();
    assert_eq!(tree.overlays()[0].overlay.refresh, ov::MIN_REFRESH);
}

// Рамка 3D-модели на земле: сдвиг, поворот, масштаб мышью.

const CAMERA: [f64; 6] = [56.0, 40.0, 0.0, 0.0, 0.0, 0.0];
const EXTENT: [f64; 4] = [0.0, 10.0, 0.0, 20.0];
const ONE: [f64; 3] = [1.0, 1.0, 1.0];

fn frame(camera: &[f64; 6], scale: [f64; 3]) -> (f64, f64, f64, f64, f64) {
    ov::box_frame(ov::model_box(camera, scale, EXTENT))
}

#[test]
fn box_follows_model_extent() {
    let (lat0, lon0, hx, hy, _) = frame(&CAMERA, ONE);
    let (x, y) = ov::local(56.0, 40.0, lat0, lon0);
    assert!(close(x, 5.0, 3));
    assert!(close(y, 10.0, 3));
    assert!(close(hx, 5.0, 3));
    assert!(close(hy, 10.0, 3));
    // Азимут 90° - модель смотрит осью Y на восток.
    let mut turned = CAMERA;
    turned[3] = 90.0;
    let (lat0, lon0, _, _, _) = frame(&turned, ONE);
    let (x, y) = ov::local(56.0, 40.0, lat0, lon0);
    assert!(close(x, 10.0, 3));
    assert!(close(y, -5.0, 3));
}

#[test]
fn centre_moves_model() {
    let (lat0, lon0, ..) = ov::box_frame(ov::model_box(&CAMERA, ONE, EXTENT));
    let (camera, scale) = ov::model_drag(&CAMERA, ONE, EXTENT, Handle::Center, 0, lat0 + 0.001, lon0 + 0.002, false);
    assert!(close(camera[0], 56.001, 6));
    assert!(close(camera[1], 40.002, 6));
    assert_eq!(scale, ONE);
}

#[test]
fn rotate_keeps_centre() {
    let before = frame(&CAMERA, ONE);
    let (_, mx) = ov::metres_per_degree(before.0);
    let (camera, _) = ov::model_drag(&CAMERA, ONE, EXTENT, Handle::Rotate, 0, before.0, before.1 + 30.0 / mx, false);
    assert!(close(camera[3], 90.0, 6));
    let after = frame(&camera, ONE);
    assert!(close(after.0, before.0, 7));
    assert!(close(after.1, before.1, 7));
}

#[test]
fn corner_scales_all_axes_from_opposite_corner() {
    let corners = ov::box_handles(ov::model_box(&CAMERA, ONE, EXTENT)).corners;
    let (fixed, far) = (corners[0], corners[2]);
    // Северо-восточный угол вдвое дальше от юго-западного.
    let target = (2.0 * far.0 - fixed.0, 2.0 * far.1 - fixed.1);
    let (camera, scale) = ov::model_drag(&CAMERA, ONE, EXTENT, Handle::Corner, 2, target.0, target.1, false);
    for value in scale {
        assert!(close(value, 2.0, 4), "{value}");
    }
    let again = ov::box_handles(ov::model_box(&camera, scale, EXTENT)).corners[0];
    assert!(close(again.0, fixed.0, 7));
    assert!(close(again.1, fixed.1, 7));
}

#[test]
fn side_stretches_one_axis() {
    let east = ov::box_handles(ov::model_box(&CAMERA, ONE, EXTENT)).sides[1];
    let (_, mx) = ov::metres_per_degree(east.0);
    let (_, scale) = ov::model_drag(&CAMERA, ONE, EXTENT, Handle::Side, 1, east.0, east.1 + 10.0 / mx, false);
    assert!(close(scale[0], 2.0, 3));
    assert_eq!(scale[1..], [1.0, 1.0]);
}
