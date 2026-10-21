#!/usr/bin/env bash
# Builds the Omnarc APK: the node and the engine cross-built for Android (OMNARC_ANDROID_NODE,
# OMNARC_ANDROID_ENGINE) go in as native libraries, the panel's build goes in as assets.
#
#   OMNARC_ANDROID_NODE=… OMNARC_ANDROID_ENGINE=… android/build.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
abi=${OMNARC_ANDROID_ABI:-armeabi-v7a}
libs="$here/app/src/main/jniLibs/$abi"
mkdir -p "$libs" "$here/app/src/main/assets"
cp "${OMNARC_ANDROID_NODE:?the node built for $abi}" "$libs/libomnarc_node.so"
cp "${OMNARC_ANDROID_ENGINE:?the engine built for $abi}" "$libs/libomnarc_engine.so"
rm -rf "$here/app/src/main/assets/panel"
cp -r "$root/web/apps/panel/build" "$here/app/src/main/assets/panel"
cd "$here"
gradle --offline -q assembleDebug
ls -l app/build/outputs/apk/debug/*.apk
