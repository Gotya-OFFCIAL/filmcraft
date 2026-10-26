#!/bin/bash
# Build a debug FilmCraft APK without Gradle — NativeActivity shell, zero app-side Java.
#
# NativeActivity ships with Android itself: the manifest declares it, the APK carries only
# resources (the launcher icon) + the uncompressed lib/arm64-v8a/libfilmcraft.so, and the
# system dlopens ANativeActivity_onCreate from it (android-activity's Rust native-activity
# glue, which runs android_main on its own thread). The GameActivity experiment died because
# its Java class extends androidx.appcompat.app.AppCompatActivity — packaging that closure
# without Gradle is a follow-up (docs/mobile-ui.md, revisit with the IME work).
#
# Prerequisites:
#   - libfilmcraft.so built: cargo build --target aarch64-linux-android -p filmcraft --lib
#   - JDK 17 (keytool; apksigner.bat needs JAVA_HOME)
#   - Android build-tools 35 (aapt2, zipalign, apksigner) and platforms;android-35 (android.jar)
#
# Usage: build-apk.sh [-o OUT.apk]   (positional $2 is honored for legacy callers)
set -euo pipefail

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
SO="${SO:-$REPO/target/aarch64-linux-android/debug/libfilmcraft.so}"
BUILD="${BUILD_DIR:-$REPO/target/android-apk}"
OUT="${2:-$BUILD/FilmCraft-debug.apk}"
WORK="$BUILD/work"

JDK="${JAVA_HOME:?set JAVA_HOME to the JDK 17}"
BT="${BUILD_TOOLS:?set BUILD_TOOLS to build-tools/35.0.0}"
PLATFORM_JAR="${PLATFORM_JAR:?set PLATFORM_JAR to platforms/android-35/android.jar}"

mkdir -p "$WORK"

# 1. Compile the launcher-icon resources, then compile and link the manifest.
"$BT/aapt2.exe" compile --dir "$REPO/packaging/android/res" -o "$WORK/res.zip"
"$BT/aapt2.exe" link -o "$WORK/base.apk" -I "$PLATFORM_JAR" --manifest "$REPO/packaging/android/AndroidManifest.xml" \
  --min-sdk-version 26 --target-sdk-version 35 --version-code 1 --version-name 0.12.0 "$WORK/res.zip"

# 2. Assemble: only the .so goes into the apk (no dex: hasCode=false, NativeActivity is a
#    framework class). Stored uncompressed for direct mapping; zipalign fixes offsets before
#    signing. 7-Zip appends the entry to the aapt2-linked apk without rebuilding the others.
mkdir -p "$BUILD/lib/arm64-v8a"
cp "$SO" "$BUILD/lib/arm64-v8a/libfilmcraft.so"
SEVENZIP="${SEVENZIP:-/d/NET/MuMuPlayer/nx_main/7za.exe}"
(cd "$BUILD" && "$SEVENZIP" a -tzip "$WORK/base.apk" "lib/arm64-v8a/libfilmcraft.so" > /dev/null)

# 3. Align, then sign with the debug key (generated once into the work dir).
KS="$WORK/debug.keystore"
if [ ! -f "$KS" ]; then
  "$JDK/bin/keytool" -genkeypair -keystore "$KS" -storepass filmcraft -keypass filmcraft \
    -alias filmcraft -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=FilmCraft Debug"
fi
"$BT/zipalign.exe" -f -P 16 4 "$WORK/base.apk" "$OUT"
"$BT/apksigner.bat" sign --ks "$KS" --ks-pass pass:filmcraft --key-pass pass:filmcraft "$OUT"

echo "APK: $OUT"
