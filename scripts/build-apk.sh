#!/usr/bin/env bash
# Compila o APK de debug. Roda o setup-android.sh se a toolchain não existir.
#
#   scripts/build-apk.sh                      # ABIs do gradle.properties
#   scripts/build-apk.sh -PyasmineAbis=arm64-v8a
#
# Se a partição do repo estiver quase cheia e existir mais espaço em
# $YASMINE_BUILD_DIR (ou /mnt/data), redireciona pra lá o cache do Gradle, o
# target do Cargo e as pastas de build (por symlink) — o build do Android
# precisa de ~4 GB de rascunho.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [ ! -f "$ROOT/.toolchain-env.sh" ]; then
  bash "$ROOT/scripts/setup-android.sh"
fi
# shellcheck disable=SC1091
source "$ROOT/.toolchain-env.sh"

free_mb() { df -Pm "$1" 2>/dev/null | awk 'NR==2{print $4}'; }

BD="${YASMINE_BUILD_DIR:-}"
if [ -z "$BD" ] && [ "$(free_mb "$ROOT")" -lt 6000 ]; then
  for cand in /mnt/data /mnt/build /var/tmp; do
    if [ -w "$cand" ] && [ "$(free_mb "$cand")" -gt 6000 ]; then BD="$cand/yasmine-build"; break; fi
  done
fi

if [ -n "$BD" ]; then
  echo ">> pouco espaço em $ROOT — build redirecionado pra $BD"
  mkdir -p "$BD"/{gradle-home,cargo-target,gbuild-root,gbuild-app,dotgradle}
  export GRADLE_USER_HOME="$BD/gradle-home"
  export CARGO_TARGET_DIR="$BD/cargo-target"
  ln -sfn "$BD/dotgradle"   "$ROOT/android/.gradle"
  ln -sfn "$BD/gbuild-root" "$ROOT/android/build"
  ln -sfn "$BD/gbuild-app"  "$ROOT/android/app/build"
fi

cd "$ROOT/android"
./gradlew --no-daemon "$@" :app:assembleDebug

APK="app/build/outputs/apk/debug/app-debug.apk"
echo
echo "APK: $ROOT/android/$APK"
ls -la "$APK" 2>/dev/null || true
