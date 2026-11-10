# Writing add-ins for PlanetX Lite

[Русский](README.ru.md)

An add-in adds a command to PlanetX Lite. The command takes the objects
of a layer of My layers and gives new objects back, Lite puts them into
a new layer. The same add-in works in the desktop program for Windows,
Linux and macOS and on the web page
https://valery35.github.io/planetx-lite/.

This guide builds a complete add-in, **Mean point**, step by step. The
second example is [`buffer`](buffer) in this folder - a buffer around
a point, a line or a polygon. Both are tested with the sandbox of Lite.

## What an add-in is

- A WebAssembly module for `wasm32-unknown-unknown` and a description
  `addin.json`, packed together into a `.pxaddin` file (a ZIP).
- The module runs in a sandbox (wasmi). It has no imports - no files,
  no network, no clock, no threads. Lite writes the request into its
  memory and reads the answer from it.
- The add-in states its rights in `addin.json`. The rights of the first
  version are `features` (to receive the objects the user chose) and
  `layers` (to put the result into My layers). A command needs both.
  Lite shows the rights before installation.
- Each call has a limit of steps and memory. A loop without an end
  stops with an error, the program and the page keep running.
- Any language that builds a WebAssembly module without imports fits.
  This guide uses Rust and the crate `addin-api` of Lite, which reads
  and writes the binary formats.

## 1. Tools

- Rust (https://rustup.rs) and the WebAssembly target:

  ```
  rustup target add wasm32-unknown-unknown
  ```

- Python 3 for the packing script of the catalogue (any build, without
  extra modules).
- PlanetX Lite 0.28.0 or later to try the add-in.

## 2. A new crate

```
cargo new --lib mean-point
```

`Cargo.toml`:

```toml
[package]
name = "mean-point"
version = "0.1.0"
edition = "2024"
license = "Apache-2.0"

[lib]
crate-type = ["cdylib", "rlib"]   # cdylib - the module, rlib - for tests

[dependencies]
addin-api = { git = "https://github.com/Valery35/planetx-lite", tag = "v0.28.0" }
serde_json = "1"

[dev-dependencies]
# The sandbox of Lite for a test of the built module.
addin-host = { git = "https://github.com/Valery35/planetx-lite", tag = "v0.28.0" }

[profile.release]
opt-level = "s"   # a smaller module
lto = true
panic = "abort"   # a panic stops the call with an error
```

## 3. The description `addin.json`

```json
{
  "id": "mean-point",
  "version": "0.1.0",
  "author": "Your Name",
  "license": "Apache-2.0",
  "title": {"ru": "Средняя точка", "en": "Mean point"},
  "description": {"ru": "Точка в среднем вершин объекта.", "en": "A point at the mean of the vertices of an object."},
  "permissions": ["features", "layers"],
  "commands": [{
    "id": "mean",
    "title": {"ru": "Средняя точка…", "en": "Mean point…"},
    "places": ["layer"],
    "kinds": ["point", "line", "polygon"],
    "params": [
      {"id": "height", "type": "number", "unit": "m", "title": {"ru": "Высота", "en": "Height"}, "default": 0, "min": -500, "max": 9000},
      {"id": "label", "type": "bool", "title": {"ru": "Название объекта", "en": "Object name"}, "default": true}
    ]
  }]
}
```

| Field | Meaning |
|---|---|
| `id` | The key of the add-in - Latin letters, digits, `-`, `_`, `.`, up to 64 characters. One id - one add-in in the catalogue |
| `version` | The version, `major.minor.patch`. A higher version in the catalogue is offered as an update |
| `author`, `license` | Shown in the window Add-ins |
| `title`, `description` | Texts in Russian and English. With one language missing, the other one is shown |
| `permissions` | `features` and `layers`. An unknown right stops the installation |
| `commands` | Up to 64 commands |

A command:

| Field | Meaning |
|---|---|
| `id` | The key of the command, it comes to the module with the request |
| `title` | The item of the layer menu. The item opens the window of parameters with the button **Run**, so `…` at the end fits |
| `places` | Where the item is. `layer` and `object` both put it into the menu of a layer of My layers, `object` is kept for the menu of an object in later versions |
| `kinds` | Geometries the command takes - `point`, `line`, `polygon`. Empty - all |
| `params` | Parameters, up to 64. Lite draws their window itself |

Parameters:

| `type` | Fields | The value in the request |
|---|---|---|
| `number` | `unit` (`m`, `km`, `deg` and others), `default`, `min`, `max` | a number within `min`..`max` |
| `text` | `default` | a string |
| `bool` | `default` | `true` or `false` |
| `choice` | `options` - a list of `{"id", "title"}`, `default` - the id of an option | the id of the option |

Every parameter has `id` and `title`. The request carries the values as
one JSON object, `{"height": 0, "label": true}`. The first number
parameter with its unit goes into the name of the result layer, for
example «Roads · Buffer 100 m».

## 4. The module `src/lib.rs`

```rust
//! Add-in "Mean point": a point at the mean of the vertices of each object.

use std::sync::Mutex;

use addin_api::{ABI_VERSION, Feature, Kind, decode_request, encode_features};

/// The description: the same addin.json that goes into the package.
const MANIFEST: &str = include_str!("../addin.json");

static INPUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());
static OUTPUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// Put bytes to the output, the answer is their length.
fn put(bytes: Vec<u8>) -> i32 {
    let len = i32::try_from(bytes.len()).unwrap_or(i32::MAX);
    if let Ok(mut out) = OUTPUT.lock() {
        *out = bytes;
    }
    len
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_abi() -> i32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_manifest() -> i32 {
    put(MANIFEST.as_bytes().to_vec())
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_input(len: i32) -> i32 {
    let (Ok(n), Ok(mut input)) = (usize::try_from(len), INPUT.lock()) else {
        return 0;
    };
    *input = vec![0; n.max(1)];
    input.as_ptr() as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_output() -> i32 {
    OUTPUT.lock().map_or(0, |out| out.as_ptr() as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_command(len: i32) -> i32 {
    let request = match INPUT.lock() {
        Ok(input) => decode_request(input.get(..usize::try_from(len).unwrap_or(0)).unwrap_or(&[])),
        Err(_) => Err("no input".to_string()),
    };
    match request.and_then(|(command, params, features)| run(&command, &params, &features)) {
        Ok(features) => put(encode_features(&features)),
        Err(e) => -put(e.into_bytes()).max(1),
    }
}

/// The command itself: plain Rust, tested without WebAssembly.
pub fn run(command: &str, params: &str, features: &[Feature]) -> Result<Vec<Feature>, String> {
    if command != "mean" {
        return Err(format!("unknown command {command}"));
    }
    let p: serde_json::Value = serde_json::from_str(params).map_err(|e| e.to_string())?;
    let height = p.get("height").and_then(serde_json::Value::as_f64).unwrap_or(0.0);
    let label = p.get("label").and_then(serde_json::Value::as_bool).unwrap_or(true);
    let mut out = Vec::new();
    for f in features {
        let v: Vec<&[f64; 3]> = f.parts.iter().flatten().collect();
        if v.is_empty() {
            continue;
        }
        let n = v.len() as f64;
        let lat = v.iter().map(|x| x[0]).sum::<f64>() / n;
        let lon = v.iter().map(|x| x[1]).sum::<f64>() / n;
        let name = if label { f.name.clone() } else { String::new() };
        out.push(Feature { kind: Kind::Point, parts: vec![vec![[lat, lon, height]]], name });
    }
    Ok(out)
}
```

The simple mean of longitudes is wrong for an object across the 180th
meridian. The buffer example shows how to handle it.

### The exports

| Export | What Lite expects |
|---|---|
| `memory` | The memory of the module, Rust exports it by itself |
| `pxa_abi() -> i32` | The version of the interface, `2` for commands |
| `pxa_manifest() -> i32` | Puts the text of `addin.json` to the output, returns its length. It must equal the `addin.json` of the package by id and version, and its rights must not exceed the rights of the package |
| `pxa_input(len) -> i32` | Gives room for `len` bytes of the request and returns its address, `0` - no room |
| `pxa_command(len) -> i32` | Reads the request of `len` bytes, puts the result to the output. The answer is the length of the result, or a negative length of the error text |
| `pxa_output() -> i32` | The address of the output |

The order of a call is `pxa_input` - Lite writes the request -
`pxa_command` - `pxa_output` - Lite reads the answer.

### The objects

`addin_api::Feature` is one object:

- `kind` - `Kind::Point`, `Kind::Line` or `Kind::Polygon`.
- `parts` - parts of the geometry, a vertex is `[latitude, longitude,
  height]`, degrees of WGS84 and metres.
- `name` - the name of the object, up to 1024 bytes.

The parts of a point are single vertices, the parts of a line are
polylines. The first part of a polygon is the outer ring, the other
parts are its holes. A ring does not repeat the first vertex at its
end. A multipolygon of a layer comes as several polygon objects with
one name.

The objects of the request are the selected objects of the layer or,
with nothing selected, all of its objects. The result may have any
kinds and any count of objects. Lite keeps the names of the result in
the field `name`. The heights of the result are not used yet.

`addin_api` checks every request and every result. Truncated data, a
vertex outside latitude ±90 or a number that is not finite is an error.

## 5. Tests

A test of `run` needs no WebAssembly:

```rust
#[cfg(test)]
mod tests {
    use addin_api::{Feature, Kind};

    #[test]
    fn mean_of_a_square() {
        let square = Feature { kind: Kind::Polygon, parts: vec![vec![[0.0, 0.0, 0.0], [0.0, 2.0, 0.0], [2.0, 2.0, 0.0], [2.0, 0.0, 0.0]]], name: "A".into() };
        let out = super::run("mean", r#"{"height":10,"label":true}"#, &[square]).unwrap();
        assert_eq!(out[0].parts[0][0], [1.0, 1.0, 10.0]);
        assert_eq!(out[0].name, "A");
    }
}
```

The built module goes through the same sandbox and the same checks as
in Lite, `tests/sandbox.rs`:

```rust
use addin_api::{Feature, Kind};
use addin_host::{Addin, Package};

#[test]
fn module_runs_in_the_sandbox() {
    let bytes = std::fs::read("target/wasm32-unknown-unknown/release/mean_point.wasm").unwrap();
    // The same checks as at installation: id, version, rights, commands.
    let package = Package::from_parts(include_str!("../addin.json"), bytes.clone()).unwrap();
    assert_eq!(package.manifest.id, "mean-point");
    let mut addin = Addin::load(&bytes).unwrap();
    let line = Feature { kind: Kind::Line, parts: vec![vec![[55.0, 37.0, 0.0], [56.0, 38.0, 0.0]]], name: "L".into() };
    let params = serde_json::json!({"height": 0, "label": true});
    let out = addin.run_command("mean", &params, &[line]).unwrap();
    assert_eq!(out[0].parts[0][0], [55.5, 37.5, 0.0]);
}
```

```
cargo build --release --target wasm32-unknown-unknown
cargo test
```

The module is `target/wasm32-unknown-unknown/release/mean_point.wasm`,
about 90 KB.

## 6. The package

The script `pack.py` of the catalogue
[planetx-lite-addins](https://github.com/Valery35/planetx-lite-addins)
packs the description and the module:

```
python scripts/pack.py path/to/addin.json path/to/mean_point.wasm packages/
```

It writes `packages/mean-point-0.1.0.pxaddin` and prints its SHA-256.
The package is the same byte for byte at every run.

## 7. Trying it in Lite

1. Open the package - the button Open, a drop on the window, or Search ▸
   **Add-ins** ▸ **From file…**. The page of Lite takes the package the
   same way.
2. The window shows the title, the author, the license and the rights,
   with a warning that the catalogue did not check the add-in. **Install**.
3. Open a layer in My layers - GeoPackage, Shapefile, GeoJSON, GPX,
   MapInfo, FlatGeobuf or DXF.
4. The menu **⋯** of the layer has the item **Mean point…**. The window
   of parameters shows the layer and the count of objects. **Run**.
5. The result is a new layer «<layer> · Mean point 0 m». An error of the
   add-in is shown in the status line.

The program keeps installed add-ins in the folder of its data, the page
keeps them in the storage of the browser. The tab **Installed** turns an
add-in off and on and removes it.

## 8. Publishing in the catalogue

The catalogue is
[planetx-lite-addins](https://github.com/Valery35/planetx-lite-addins).
Lite reads it from
https://valery35.github.io/planetx-lite-addins/registry.json.

1. Publish the `.pxaddin` as a release of your own repository.
2. Open a pull request to the catalogue that adds `registry/<id>.json`:

   ```json
   {
     "id": "mean-point",
     "version": "0.1.0",
     "package": "https://github.com/you/mean-point/releases/download/v0.1.0/mean-point-0.1.0.pxaddin",
     "sha256": "…",
     "source": "https://github.com/you/mean-point"
   }
   ```

3. The workflow of the catalogue downloads the package and checks its
   SHA-256, the id and the version against the entry and the rights.
   After the merge the package is copied to GitHub Pages.
4. An update is a new release and a pull request with the new `version`,
   `package` and `sha256`. Lite offers it on the tab **Installed**.

A package from the catalogue is installed only when its SHA-256 equals
the catalogue. An add-in withdrawn by `blocklist.json` (`id` or
`id@version`) is turned off in Lite.

## Limits

| What | Limit |
|---|---|
| Module | 32 MB |
| `addin.json` | 256 KB |
| Memory of the module | 512 MB |
| Request | 64 MB |
| Parameters JSON | 64 KB |
| Name of a command | 256 bytes |
| Name of an object | 1024 bytes |
| Text of the description | 2000 characters |
| Commands, parameters, options | 64 each |
| Steps of one command | 20 billion and 200 thousand per byte of the request |

## Advice

- Check the parameters in `run` too. The window keeps `min` and `max`,
  but the request is data from outside.
- An error is a short text for the user, without a stack.
- A panic stops the call with an error, Lite loads the module anew.
  It is better to return an error.
- Keep the command fast. The call runs in the frame of Lite, a long
  command holds the window.
- Without new rights an add-in has no way to read files or the
  network. Rights beyond `features` and `layers` come with new versions
  of Lite.

## The interface in bytes

For a language without `addin-api`. All numbers are little-endian.

The request:

```
"PXQ1"
u16 length + UTF-8   - id of the command
u32 length + UTF-8   - parameters, JSON
objects
```

The objects (request and result):

```
"PXG1"
u32 count
  u8 kind            - 1 point, 2 line, 3 polygon
  u16 length + UTF-8 - name
  u32 count of parts
    u32 count of vertices
      f64 latitude, f64 longitude, f64 height
```

There are no bytes after the objects. ABI 1 (`pxa_read`, a file to
points) is described in the documentation of
[`addin-api`](../crates/addin-api/src/lib.rs).
