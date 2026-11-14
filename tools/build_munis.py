# PlanetX Lite - глобус PlanetX без QGIS.
# Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
# SPDX-License-Identifier: Apache-2.0

"""Границы муниципальных образований России СберИндекса - в файл слоя Lite.

Источник - «Версионный справочник границ и преобразований муниципальных
образований России» СберИндекса, CC BY-SA 4.0: GeoPackage границ
и таблица xlsx с названиями. Итог - производный набор под той же
CC BY-SA 4.0, файл `munis.json.gz` (JSON, сжатый gzip). В нём `units` -
муниципальные образования нынешней структуры (year_to 9999): номер `id`,
название с типом, тип, ОКТМО, регион, центр, год начала, точка подписи
и кольца границы. Точки кольца подряд «широта, долгота…», 4 знака после
запятой.

Контуры исправляются до валидных и упрощаются по одному. Общей топологии
у набора нет - у соседей зазоры и наложения больше 10 м, поэтому общая
граница при сильном приближении видна двумя близкими линиями.

Запуск - Python из QGIS (GDAL, shapely 2, pandas):

    python build_munis.py папка_набора munis.json.gz [допуск_градусов]
"""

import gzip
import json
import sys

import numpy as np
import pandas as pd
import shapely
from osgeo import ogr

ogr.UseExceptions()

# Допуск упрощения, градусы. 0.002 - около 220 м по широте.
TOLERANCE = 0.002
# Знаков после запятой у координат: 4 - около 11 м.
DIGITS = 4


def read_bounds(path):
    """Номера id_m и мультиполигоны границ, исправленные до валидных."""
    ds = ogr.Open(path)
    layer = ds.GetLayer(0)
    ids, geoms = [], []
    for f in layer:
        if f.GetField("year_to") != 9999:
            continue
        ids.append(f.GetField("id_m"))
        geoms.append(shapely.from_wkb(bytes(f.GetGeometryRef().ExportToWkb())))
    geoms = shapely.make_valid(np.array(geoms), method="structure", keep_collapsed=False)
    fixed = []
    for g in geoms:
        parts = [p for p in shapely.get_parts(g) if p.geom_type == "Polygon"]
        fixed.append(shapely.MultiPolygon(parts))
    return ids, np.array(fixed)


def read_names(path):
    """Строки таблицы нынешней структуры по id_m."""
    x = pd.read_excel(path)
    x = x[x.year_to == 9999]
    return {r.id_m: r for r in x.itertuples()}


def text(value):
    return "" if pd.isna(value) else str(value).strip()


def flat(ring):
    """Точки кольца (lon, lat) подряд «широта, долгота…»."""
    out = []
    for lon, lat in shapely.get_coordinates(ring):
        out.append(round(float(lat), DIGITS))
        out.append(round(float(lon), DIGITS))
    return out


def rings(geom):
    """Кольца мультиполигона: внешние и дыры, без выродившихся."""
    out = []
    for p in shapely.get_parts(geom):
        if p.is_empty:
            continue
        for ring in [p.exterior, *p.interiors]:
            if len(ring.coords) >= 4:
                out.append(flat(ring))
    return out


def main():
    src, out = sys.argv[1], sys.argv[2]
    tolerance = float(sys.argv[3]) if len(sys.argv) > 3 else TOLERANCE
    ids, geoms = read_bounds(src + "/t_dict_municipal_districts_poly.gpkg")
    names = read_names(src + "/t_dict_municipal_districts.xlsx")
    simple = shapely.simplify(geoms, tolerance, preserve_topology=True)
    units = []
    for id_m, g, s in zip(ids, geoms, simple):
        row = names.get(id_m)
        if row is None:
            continue
        biggest = max(shapely.get_parts(g), key=shapely.area)
        p = shapely.point_on_surface(biggest)
        units.append({
            "id": id_m,
            "name": text(row.municipal_district_name_type),
            "kind": text(row.municipal_district_type),
            "oktmo": text(row.oktmo).zfill(5) if text(row.oktmo) else "",
            "region": text(row.region_name),
            "center": text(row.municipal_district_center),
            "from": int(row.year_from),
            "lat": round(p.y, DIGITS),
            "lon": round(p.x, DIGITS),
            "rings": rings(s),
        })
    data = {
        "source": "СберИндекс, «Версионный справочник границ и преобразований муниципальных образований России», 2026",
        "license": "CC BY-SA 4.0",
        "date": "2025-01-01",
        "units": units,
    }
    raw = json.dumps(data, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    with open(out, "wb") as f:
        f.write(gzip.compress(raw, compresslevel=9, mtime=0))
    points = sum(len(r) // 2 for u in units for r in u["rings"])
    print(f"units={len(units)} points={points} json={len(raw)}")


if __name__ == "__main__":
    main()
