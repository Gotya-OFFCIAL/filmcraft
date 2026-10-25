#!/bin/bash
# Build a debug FilmCraft APK without Gradle, from parts fetched straight from
# Google Maven (dl.google.com/android/maven2) — fork-experiment tooling.
#
# Prerequisites (see docs/mobile-ui.md and the porting handover notes):
#   - libfilmcraft.so built: cargo build --target aarch64-linux-android -p filmcraft --lib
#   - JDK 17 (java on PATH)
#   - Android build-tools 35 (aapt2, d8, zipalign, apksigner) and platforms;android-35
#     (android.jar) via sdkmanager — OR the maven-fetched equivalents noted inline.
#   - The games-activity AAR 3.0.5 extracted (classes.jar supplies the Java GameActivity).
#
# Usage: build-apk.sh [-o OUT.apk]
set -euo pipefail

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
AAR_DIR="${AAR_DIR:-$REPO/../games-activity}"
SO="${SO:-$REPO/target/aarch64-linux-android/debug/libfilmcraft.so}"
BUILD="${BUILD_DIR:-$REPO/target/android-apk}"
OUT="${2:-$BUILD/FilmCraft-debug.apk}"
WORK="$BUILD/work"

JDK="${JAVA_HOME:?set JAVA_HOME to the JDK 17}"
BT="${BUILD_TOOLS:?set BUILD_TOOLS to build-tools/35.0.0}"
PLATFORM_JAR="${PLATFORM_JAR:?set PLATFORM_JAR to platforms/android-35/android.jar}"

mkdir -p "$WORK/dex"

# 1. The manifest, compiled and linked (no resources yet: the shell declares none).
"$BT/aapt2.exe" link -o "$WORK/base.apk" -I "$PLATFORM_JAR" --manifest "$REPO/packaging/android/AndroidManifest.xml" \
  --min-sdk-version 26 --target-sdk-version 35

# 2. Java: the GameActivity classes from the AAR to dex.
(cd "$WORK" && "$JDK/bin/jar" -xf "$AAR_DIR/classes.jar")
find "$WORK/com" -name "*.class" > "$WORK/classes.list"
"$BT/d8.bat" --release --lib "$PLATFORM_JAR" --min-api 26 --output "$WORK/dex" $(cat "$WORK/classes.list")

# 3. Assemble: the dex into the apk root, the .so into lib/arm64-v8a. Python's zipfile appends
#    entries to the aapt2-linked apk (the .so stored uncompressed for the 16 KB page alignment
#    rule; zipalign fixes offsets before signing).
mkdir -p "$BUILD/lib/arm64-v8a"
cp "$SO" "$BUILD/lib/arm64-v8a/libfilmcraft.so"
cp "$WORK/dex/classes.dex" "$WORK/classes.dex"
/d/AI/ComfyUI_MiniMaxH3_A770/python/python.exe "$REPO/packaging/android/apk_add.py" "$WORK/base.apk" "$WORK/classes.dex" "classes.dex" "$BUILD/lib/arm64-v8a/libfilmcraft.so" "lib/arm64-v8a/libfilmcraft.so"

# 4. Align, then sign with the debug key (generated once into the work dir).
KS="$WORK/debug.keystore"
if [ ! -f "$KS" ]; then
  "$JDK/bin/keytool" -genkeypair -keystore "$KS" -storepass filmcraft -keypass filmcraft \
    -alias filmcraft -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=FilmCraft Debug"
fi
"$BT/zipalign.exe" -f -P 16 4 "$WORK/base.apk" "$OUT"
"$BT/apksigner.bat" sign --ks "$KS" --ks-pass pass:filmcraft --key-pass pass:filmcraft "$OUT"

echo "APK: $OUT"
