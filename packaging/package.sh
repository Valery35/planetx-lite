#!/usr/bin/env bash
# PlanetX Lite - глобус PlanetX без QGIS.
# Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
# SPDX-License-Identifier: Apache-2.0
#
# Пакеты выпуска для системы, на которой запущен, в папку release-dist:
#   Windows - установщик Inno Setup и переносной zip,
#   Linux   - AppImage и tar.gz,
#   macOS   - dmg с приложением PlanetX Lite.app.
# Программа уже собрана: target/release/planetx-lite[.exe]. Рядом
# лежит target/THIRD-PARTY-LICENSES.html от cargo-about.
#
#   packaging/package.sh 0.1.0

set -euo pipefail

version="${1:?версия, например 0.1.0}"
root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/release-dist"
stage="$(mktemp -d)"
notices="$root/target/THIRD-PARTY-LICENSES.html"
mkdir -p "$out"
test -s "$notices" || { echo "нет $notices"; exit 1; }

windows() {
    local dir="$stage/PlanetX-Lite-$version-windows-x86_64"
    mkdir -p "$dir"
    cp "$root/target/release/planetx-lite.exe" "$dir/"
    cp "$root/LICENSE" "$dir/LICENSE.txt"
    cp "$notices" "$dir/"
    (cd "$stage" && 7z a -tzip -mx=9 "$out/PlanetX-Lite-$version-windows-x86_64-portable.zip" "$(basename "$dir")" > /dev/null)
    local iscc="/c/Program Files (x86)/Inno Setup 6/ISCC.exe"
    command -v iscc > /dev/null && iscc="$(command -v iscc)"
    # Git Bash превращает аргументы с «/» в пути Windows, ключам ISCC
    # это преобразование не нужно.
    MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' "$iscc" /Q "/DVersion=$version" "/DSource=$(cygpath -w "$dir")" "/O$(cygpath -w "$out")" \
        "$(cygpath -w "$root/packaging/windows/planetx-lite.iss")"
}

linux() {
    local arch
    arch="$(uname -m)"
    local name="PlanetX-Lite-$version-linux-$arch"
    local dir="$stage/$name"
    mkdir -p "$dir"
    install -m 755 "$root/target/release/planetx-lite" "$dir/planetx-lite"
    cp "$root/LICENSE" "$notices" "$dir/"
    cp "$root/packaging/linux/planetx-lite.desktop" "$dir/"
    cp "$root/crates/ui/assets/planetx.png" "$dir/planetx-lite.png"
    tar -C "$stage" -czf "$out/$name.tar.gz" "$name"

    # AppImage: программа, ярлык и значок в AppDir, AppRun - сама программа.
    local app="$stage/AppDir"
    mkdir -p "$app/usr/bin" "$app/usr/share/applications" "$app/usr/share/icons/hicolor/128x128/apps" "$app/usr/share/doc/planetx-lite"
    install -m 755 "$root/target/release/planetx-lite" "$app/usr/bin/planetx-lite"
    cp "$root/packaging/linux/planetx-lite.desktop" "$app/usr/share/applications/"
    cp "$root/packaging/linux/planetx-lite.desktop" "$app/"
    cp "$root/crates/ui/assets/planetx.png" "$app/usr/share/icons/hicolor/128x128/apps/planetx-lite.png"
    cp "$root/crates/ui/assets/planetx.png" "$app/planetx-lite.png"
    cp "$root/LICENSE" "$notices" "$app/usr/share/doc/planetx-lite/"
    ln -s usr/bin/planetx-lite "$app/AppRun"
    ARCH="$arch" APPIMAGE_EXTRACT_AND_RUN=1 "${APPIMAGETOOL:-appimagetool}" --no-appstream "$app" "$out/$name.AppImage"
}

macos() {
    local arch
    arch="$(uname -m)"
    [ "$arch" = arm64 ] && arch=aarch64
    local app="$stage/dmg/PlanetX Lite.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    install -m 755 "$root/target/release/planetx-lite" "$app/Contents/MacOS/planetx-lite"
    sed "s/VERSION/$version/g" "$root/packaging/macos/Info.plist" > "$app/Contents/Info.plist"
    # Значок: набор размеров из значка окна 128 × 128.
    local set="$stage/planetx.iconset"
    mkdir -p "$set"
    for s in 16 32 64 128; do
        sips -z "$s" "$s" "$root/crates/ui/assets/planetx.png" --out "$set/icon_${s}x${s}.png" > /dev/null
    done
    cp "$set/icon_32x32.png" "$set/icon_16x16@2x.png"
    cp "$set/icon_64x64.png" "$set/icon_32x32@2x.png"
    rm "$set/icon_64x64.png"
    iconutil -c icns "$set" -o "$app/Contents/Resources/planetx.icns"
    cp "$root/LICENSE" "$notices" "$app/Contents/Resources/"
    # Подписи Apple нет: подпись ad-hoc, без неё Apple Silicon не запускает.
    codesign --force --deep --sign - "$app"
    ln -s /Applications "$stage/dmg/Applications"
    hdiutil create -volname "PlanetX Lite" -srcfolder "$stage/dmg" -ov -format UDZO \
        "$out/PlanetX-Lite-$version-macos-$arch.dmg"
}

case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) windows ;;
    Linux) linux ;;
    Darwin) macos ;;
    *) echo "неизвестная система $(uname -s)"; exit 1 ;;
esac
ls -l "$out"
