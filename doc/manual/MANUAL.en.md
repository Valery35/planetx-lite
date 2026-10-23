# PlanetX Lite user guide

PlanetX Lite is the [PlanetX](https://github.com/Valery35/planetx) globe
without QGIS: one program for Windows, Linux and macOS and the same
program as a web page, on phones too.

[Русская версия](MANUAL.ru.md)

![PlanetX Lite window](images/window_en.jpg)

## Contents

- [Install and start](#install-and-start)
- [Window](#window)
- [Moving around the globe](#moving-around-the-globe)
- [Finding a place](#finding-a-place)
- [Opening files](#opening-files)
- [My Places](#my-places)
- [Tours and the time slider](#tours-and-the-time-slider)
- [Demo scenes](#demo-scenes)
- [Settings](#settings)
- [The web page](#the-web-page)
- [Data sources and privacy](#data-sources-and-privacy)
- [Problems](#problems)

## Install and start

Download a package from the
[releases](https://github.com/Valery35/planetx-lite/releases) page.

| System | Package |
|---|---|
| Windows | `...-windows-x86_64-setup.exe` installs the program for the current user, `...-portable.zip` runs without installation |
| Linux | `...-linux-x86_64.AppImage` - make it executable and run it, `...tar.gz` - unpack and run `planetx-lite` |
| macOS | `...-macos-aarch64.dmg` for Apple Silicon, `...-macos-x86_64.dmg` for Intel - drag PlanetX Lite to Applications |

The files are not signed yet. On Windows the "Windows protected your PC"
window appears: choose "More info" - "Run anyway". On macOS, after the
first attempt to open the program, open System Settings - Privacy &
Security - "Open Anyway".

The web page needs no installation: https://valery35.github.io/planetx-lite/

## Window

- **Left panel.** Place search, the Places section with My Places, the
  Layers section with the basemap, and the status line at the bottom:
  the eye altitude and the number of requests in progress.
- **Toolbar** in the top left corner of the globe:

| Button | Action |
|---|---|
| Panel | Shows or hides the left panel. On a narrow screen the panel starts hidden |
| Open | Opens a scene, KML, KMZ or a data file |
| Demo | The menu of demo scenes |
| Share | Copies a link to this view: the camera and the scenes opened by address or from the Demo menu. On a phone the system Share window opens. Files opened from the disk are not in the link |
| Settings | The Settings window |
| Full screen | Full screen on and off |
| About | Version, the user guide, links |

- **Source credits** in the bottom right corner. A click opens the Data
  sources window with full credits and links.

## Moving around the globe

The controls follow Google Earth.

| Action | Mouse | Keys |
|---|---|---|
| Turn the globe | Drag with the left button, a throw keeps it turning | Arrows |
| Zoom | Wheel, right button drag, double click to fly closer, double right click to fly away | Page Up, Page Down, plus, minus |
| Tilt and rotate | Middle button or Shift with the left button | Shift with arrows |
| Look around | Ctrl and Shift with the left button | Ctrl with arrows |
| Shift the view along the screen | Ctrl with the left button | |
| North up, look straight down, both | | N, U, R |
| Stop | | Space |

On a phone: one finger turns the globe, a pinch zooms to the middle of
the pinch, turning two fingers rotates, two fingers up and down tilt.

## Finding a place

Type a name or coordinates in the search field at the top of the panel
and press Enter or Search. The camera flies to the place, a red mark
shows it. With several results the list appears under the field. The
search uses Nominatim OpenStreetMap.

## Opening files

The Open button, a file dropped onto the window, or file names on the
command line open:

| File | What appears |
|---|---|
| `.planetx` | A PlanetX scene: camera, placemarks, tours, overlays |
| `.kml`, `.kmz` | Placemarks, paths, polygons, overlays, tours, 3D models |
| `.gpkg` | GeoPackage layers as placemarks |
| `.tif`, `.tiff` | A GeoTIFF or COG elevation raster cut into the globe terrain |
| `.las`, `.laz` | A point cloud, COPC included, in the colours of the file |
| `.csv` | Points, read by a sandboxed WebAssembly add-in |

Several files open in order: an elevation raster first, then the scene
over it.

## My Places

Every opened scene or file becomes a folder in My Places. The tree keeps
its content between runs.

- A checkbox shows or hides a row and everything in it.
- A double click flies to the row.
- A click selects the row. Delete removes it, F2 renames it.
- Drag a row onto a folder to put it inside, onto another row to put it
  before that row, onto My Places to put it at the end.
- The right button opens the menu:

| Row | Menu |
|---|---|
| Placemark, overlay | Fly to, Play tour (for a recorded tour), Save as KML, Save scene, Rename, Delete |
| Folder | Fly to, Play tour, Save as KML, Save scene, Rename, Delete |
| My Places | Open, Play tour, Save as KML, Save scene, Clear My Places |

**Saving.** "Save as KML" writes the row to KML, or to KMZ when it
contains pictures or models. "Save scene" writes a `.planetx` scene with
the current camera. The PlanetX plugin for QGIS opens these scenes too.
On the web page the file is downloaded.

## Tours and the time slider

The play button next to a recorded tour starts it, "Play tour" in a
folder menu flies over the folder's placemarks. During a tour the panel
on top shows pause and stop. Dragging the globe stops the tour.

Placemarks with dates turn on the Time button in the Places section. The
time slider on top shows the placemarks of the chosen period.

## Demo scenes

The Demo button opens scenes of the PlanetX plugin from the
[planetx-terrain](https://github.com/Valery35/planetx-terrain)
repository: placemarks and tours, imagery and maps, the Japan Trench and
the Vegas Loop tunnels, own terrain of a quarry and the bed of Lake
Brienz, 3D models, haul trucks in open pits. The program downloads the
demo files once and keeps them, the web page relies on the browser
cache.

## Settings

| Setting | Values |
|---|---|
| Interface language | As in the system or one of 16 languages |
| Label language | Language of place names on the globe: as the interface, local names or a chosen language |
| Appearance | Dark or Light |
| Vertical exaggeration | Terrain height multiplier from 0.5 to 10 |

Settings are kept between runs. `PLANETX_LANGUAGE=de` sets the language
for one run of the program.

## The web page

https://valery35.github.io/planetx-lite/ works in Chrome, Edge, Firefox
and Safari, on phones too.

- `?scene=address` opens a scene, KML or KMZ by a link. Several
  `scene` parameters open in order.
- `?pose=latitude,longitude,distance,heading,tilt` sets the camera.
- My Places are kept in the browser, up to about 4 MB.
- Chinese, Japanese and Korean download a font once, 1-3 MB.

## Data sources and privacy

The globe shows Esri World Imagery, OpenStreetMap, OpenFreeMap labels and
Mapzen Terrain Tiles. Their credits are on the screen, full terms are in
the Data sources window.

The program does not collect or send user information. It requests only
the data it shows, the place search and the demo files. User files stay
on the user's computer.

## Problems

- **The globe is not drawn.** The graphics card has to support Vulkan,
  DirectX 12, Metal or OpenGL. On the web page a current browser is
  needed.
- **Tiles load slowly.** The status line shows the number of requests in
  progress. Tiles are cached: the program keeps them on disk, the web
  page in the browser cache.

Problems and wishes: https://github.com/Valery35/planetx-lite/issues
