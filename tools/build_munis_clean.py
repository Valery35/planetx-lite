# PlanetX Lite - глобус PlanetX без QGIS.
# Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
# SPDX-License-Identifier: Apache-2.0

"""Границы муниципальных образований СберИндекса - в файл слоя Lite, общей сеткой.

Источник тот же, что у `build_munis.py`: «Версионный справочник границ
и преобразований муниципальных образований России» СберИндекса,
CC BY-SA 4.0. Итог - `munis.json.gz` того же устройства, но у соседей
общая граница одна и та же, без зазоров и наложений.

Порядок сборки.

1. Очистка 1.03 модуля Topoliner 0.13.5 в проекции EPSG:3576, допуск 1 м,
   порог площади 1 м². Итог - GeoPackage очищенного покрытия, вход этого
   скрипта.
2. Заливка щелей. Щель отходит соседу с самой длинной общей границей.
   Щели крупнее KEEP_AREA остаются, это Невская губа. Перекрытие
   отходит большему соседу.
3. Недостающие узлы там, где соседи касаются без общей вершины. Ядро
   Topoliner, режим вставки с проекцией на ребро, форма и площадь
   не меняются. Нужен путь к пакету topoliner в TOPOLINER.
4. Разрез по меридиану 180°, упрощение покрытия целиком
   (`shapely.coverage_simplify`, GEOS 3.12 и новее), перевод в широту
   и долготу, 4 знака после запятой. Вершины шва ставятся на ±180.
5. Названия, тип, ОКТМО, регион и центр из таблицы, только нынешняя
   структура (year_to 9999).

6. С ключом --arcs граница пишется дугами: общий список `arcs`
   и у муниципалитета `parts` - кольца из номеров дуг, `-i-1` - дуга
   в обратную сторону. Общая граница соседей записана один раз. Дуги
   режет `build_arcs` Topoliner по узлам, где меняется набор соседей.

Запуск - Python из QGIS (GDAL, shapely 2.1, pandas):

    set TOPOLINER=C:\\Dev\\topoliner\\topoliner
    python build_munis_clean.py очищенное.gpkg таблица.xlsx munis.json.gz [допуск_м] [--arcs]

15 ноября 2026 года при допуске 300 м: 2570 образований, 760 647 точек,
4 149 018 байт. Заливка - 57 щелей, 2.3 км². Узел вставлен один.
"""

import gzip
import json
import os
import sys

import numpy as np
import pandas as pd
import shapely
from osgeo import ogr, osr
from shapely.ops import split
from shapely.strtree import STRtree

ogr.UseExceptions()

# Допуск упрощения покрытия, метры EPSG:3576. 300 м дают примерно
# столько же точек, сколько 0.002° в build_munis.py.
TOLERANCE = 300.0
# Щели крупнее, м², не заливаются.
KEEP_AREA = 100e6
DIGITS = 4
# Градусы: вершина ближе к ±180 ставится на шов.
SNAP = 1e-6


def multi(g):
    return shapely.MultiPolygon([p for p in shapely.get_parts(shapely.make_valid(g))
                                 if p.geom_type == "Polygon"])


def fill_gaps(geoms):
    """Перекрытия - большему соседу, щели мельче KEEP_AREA - соседу с длинной общей границей."""
    tree = STRtree(geoms)
    a, b = tree.query(geoms, predicate="intersects")
    for i, j in zip(a, b):
        if i >= j or geoms[i].intersection(geoms[j]).area <= 1.0:
            continue
        loser, keeper = (j, i) if geoms[i].area >= geoms[j].area else (i, j)
        geoms[loser] = multi(geoms[loser].difference(geoms[keeper]))
    holes = [shapely.Polygon(r) for p in shapely.get_parts(shapely.union_all(geoms))
             for r in p.interiors]
    tree = STRtree(geoms)
    filled = 0
    for h in sorted(holes, key=lambda p: p.area):
        if h.area > KEEP_AREA:
            continue
        best, best_len = None, 0.0
        for i in tree.query(h, predicate="intersects"):
            shared = h.boundary.intersection(geoms[i].boundary).length
            if shared > best_len:
                best, best_len = i, shared
        if best is not None:
            geoms[best] = multi(shapely.union(geoms[best], h))
            filled += 1
    return filled


def insert_nodes(geoms):
    """Узлы у объектов с неверными рёбрами покрытия и у их соседей."""
    sys.path.insert(0, os.environ["TOPOLINER"])
    import topo_core as tc
    inserted = 0
    for _ in range(3):
        bad = {i for i, e in enumerate(shapely.coverage_invalid_edges(geoms)) if not e.is_empty}
        if not bad:
            break
        tree = STRtree(geoms)
        group = set(bad)
        for i in bad:
            group.update(int(j) for j in tree.query(geoms[i], predicate="intersects"))
        rings, addr = [], []
        for gi in sorted(group):
            for pi, p in enumerate(shapely.get_parts(geoms[gi])):
                for ri, r in enumerate([p.exterior, *p.interiors]):
                    rings.append([tuple(c) for c in r.coords])
                    addr.append((gi, pi, ri))
        res = tc.clean_topology(rings, tolerance=0.01, mode=tc.MODE_INSERT, project_onto_edge=True)
        inserted += res["stats"]["nodes_inserted"]
        new = {}
        for (gi, pi, ri), ring in zip(addr, res["rings"]):
            new.setdefault(gi, {}).setdefault(pi, {})[ri] = [(p[0], p[1]) for p in ring]
        for gi, parts in new.items():
            geoms[gi] = shapely.MultiPolygon([
                shapely.Polygon(rs[0], [rs[k] for k in sorted(rs) if k > 0])
                for _pi, rs in sorted(parts.items())])
    if not shapely.coverage_is_valid(geoms):
        raise SystemExit("coverage is still invalid")
    return inserted


def text(value):
    return "" if pd.isna(value) else str(value).strip()


def to_arcs(per_obj):
    """
    Кольца «широта, долгота…» - в дуги и ссылки на них.

    Возвращает (arcs, parts): arcs - плоские списки точек, parts[k] -
    кольца объекта k как списки номеров дуг. Повторы точек подряд, след
    округления, снимаются. Сборка колец из дуг сверяется с исходными
    кольцами по кругу, расхождение останавливает сборку.
    """
    sys.path.insert(0, os.environ["TOPOLINER"])
    from topo_simplify import build_arcs
    rings, owner = [], []
    for k, flats in per_obj.items():
        for flat in flats:
            pts = []
            for p in zip(flat[0::2], flat[1::2]):
                if not pts or p != pts[-1]:
                    pts.append(p)
            while len(pts) > 1 and pts[0] == pts[-1]:
                pts.pop()
            if len(set(pts)) >= 3:
                rings.append(pts)
                owner.append(k)
    arcs, paths = build_arcs(rings, grid=1e-7)
    parts = {}
    for ring, k, path in zip(rings, owner, paths):
        refs = [idx if not rev else -idx - 1 for idx, rev in path]
        back = []
        for idx, rev in path:
            seq = arcs[idx][::-1] if rev else arcs[idx]
            back.extend(seq if not back else seq[1:])
        back = back[:-1]
        n = len(ring)
        if len(back) != n or not any(back == ring[s:] + ring[:s] for s in range(n) if ring[s] == back[0]):
            raise SystemExit("arcs do not rebuild a ring of unit %r" % k)
        parts.setdefault(k, []).append(refs)
    flat_arcs = [[c for p in a for c in p] for a in arcs]
    return flat_arcs, parts


def main():
    cov, xlsx, out = sys.argv[1], sys.argv[2], sys.argv[3]
    with_arcs = "--arcs" in sys.argv
    args = [a for a in sys.argv[4:] if a != "--arcs"]
    tol = float(args[0]) if args else TOLERANCE
    src = osr.SpatialReference()
    src.ImportFromEPSG(3576)
    src.SetAxisMappingStrategy(osr.OAMS_TRADITIONAL_GIS_ORDER)
    wgs = osr.SpatialReference()
    wgs.ImportFromEPSG(4326)
    wgs.SetAxisMappingStrategy(osr.OAMS_TRADITIONAL_GIS_ORDER)
    to_wgs = osr.CoordinateTransformation(src, wgs)
    to_src = osr.CoordinateTransformation(wgs, src)

    ds = ogr.Open(cov)
    ids, geoms = [], []
    for f in ds.GetLayer(0):
        ids.append(f.GetField("id_m"))
        geoms.append(shapely.from_wkb(bytes(f.GetGeometryRef().ExportToWkb())))
    ds = None
    filled = fill_gaps(geoms)
    nodes = insert_nodes(geoms)

    # Меридиан 180° в EPSG:3576 - луч из полюса.
    pole = to_src.TransformPoint(0.0, 90.0)[:2]
    far = to_src.TransformPoint(180.0, 30.0)[:2]
    ray = shapely.LineString([pole, far])
    elems, owner = [], []
    for k, g in enumerate(geoms):
        for p in shapely.get_parts(g):
            if p.intersection(ray).length > 0:
                pieces = list(shapely.get_parts(split(p, ray)))
            else:
                pieces = [p]
            elems.extend(pieces)
            owner.extend([k] * len(pieces))
    simple = shapely.coverage_simplify(np.array(elems), tol, simplify_boundary=True)
    if not shapely.coverage_is_valid(simple):
        raise SystemExit("simplified coverage is invalid")

    def lonlat(ring, east):
        flat = []
        for x, y in ring.coords:
            lon, lat, _ = to_wgs.TransformPoint(x, y)
            if abs(abs(lon) - 180.0) <= SNAP:
                lon = 180.0 if east else -180.0
            flat.append(round(float(lat), DIGITS))
            flat.append(round(float(lon), DIGITS))
        return flat

    per_obj = {}
    for q, k in zip(simple, owner):
        for p in shapely.get_parts(q):
            c = p.representative_point()
            east = to_wgs.TransformPoint(c.x, c.y)[0] > 0
            for r in [p.exterior, *p.interiors]:
                if len(r.coords) >= 4:
                    per_obj.setdefault(k, []).append(lonlat(r, east))

    arcs, parts = to_arcs(per_obj) if with_arcs else (None, None)
    x = pd.read_excel(xlsx)
    x = x[x.year_to == 9999]
    names = {r.id_m: r for r in x.itertuples()}
    units = []
    for k, (id_m, g) in enumerate(zip(ids, geoms)):
        row = names.get(id_m)
        if row is None or k not in per_obj:
            continue
        p = max(shapely.get_parts(g), key=shapely.area).point_on_surface()
        lon, lat, _ = to_wgs.TransformPoint(p.x, p.y)
        units.append({
            "id": id_m,
            "name": text(row.municipal_district_name_type),
            "kind": text(row.municipal_district_type),
            "oktmo": text(row.oktmo).zfill(5) if text(row.oktmo) else "",
            "region": text(row.region_name),
            "center": text(row.municipal_district_center),
            "from": int(row.year_from),
            "lat": round(lat, DIGITS),
            "lon": round(lon, DIGITS),
        })
        if with_arcs:
            units[-1]["parts"] = parts.get(k, [])
        else:
            units[-1]["rings"] = per_obj[k]
    data = {
        "source": "СберИндекс, «Версионный справочник границ и преобразований муниципальных образований России», 2026",
        "license": "CC BY-SA 4.0",
        "date": "2025-01-01",
    }
    if with_arcs:
        data["arcs"] = arcs
    data["units"] = units
    raw = json.dumps(data, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    with open(out, "wb") as f:
        f.write(gzip.compress(raw, compresslevel=9, mtime=0))
    if with_arcs:
        points = sum(len(a) // 2 for a in arcs)
    else:
        points = sum(len(r) // 2 for u in units for r in u["rings"])
    print(f"filled={filled} nodes={nodes} units={len(units)} points={points} json={len(raw)}")


if __name__ == "__main__":
    main()
