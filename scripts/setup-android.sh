#!/usr/bin/env bash
# Instala, SEM root, tudo que o app Android precisa, dentro de ./.toolchain/:
# JDK 17, Android cmdline-tools, platform-34, build-tools, NDK, e (via rustup/
# cargo) os targets *-linux-android + cargo-ndk.
#
# Depois: `source scripts/setup-android.sh` exporta JAVA_HOME / ANDROID_*
# pro shell atual, ou o build-apk.sh já cuida disso.
#
# Precisa de ~6 GB livres. Idempotente: rodar de novo só completa o que falta.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# O sdkmanager / NDK quebram com espaço no caminho. Se a raiz do repo tem
# espaço (ex.: "Yasmine APP"), a toolchain vai pra um lugar sem espaço.
if [[ "$ROOT" == *" "* ]]; then
  TC="${YASMINE_TC:-$HOME/.yasmine-android}"
else
  TC="${YASMINE_TC:-$ROOT/.toolchain}"
fi
mkdir -p "$TC"
echo ">> toolchain em: $TC"

JDK_VER=17.0.13+11
NDK_VER=27.1.12297006
CMDLINE_VER=11076708   # cmdline-tools 13
PLATFORM=android-34
BUILD_TOOLS=34.0.0

fetch() { # url dest
  if command -v curl >/dev/null; then curl -fL --retry 3 -o "$2" "$1"
  else wget -q -O "$2" "$1"; fi
}

# --- JDK 17 (Temurin) ---
export JAVA_HOME="$TC/jdk"
if [ ! -x "$JAVA_HOME/bin/javac" ]; then
  echo ">> JDK $JDK_VER"
  arch=$(uname -m); case "$arch" in x86_64) jarch=x64;; aarch64) jarch=aarch64;; *) jarch=x64;; esac
  url="https://github.com/adoptium/temurin17-binaries/releases/download/jdk-${JDK_VER//+/%2B}/OpenJDK17U-jdk_${jarch}_linux_hotspot_${JDK_VER/+/_}.tar.gz"
  fetch "$url" "$TC/jdk.tgz"
  mkdir -p "$JAVA_HOME"; tar -xzf "$TC/jdk.tgz" -C "$JAVA_HOME" --strip-components=1
  rm -f "$TC/jdk.tgz"
fi
echo "JAVA_HOME=$JAVA_HOME"

# --- Android SDK cmdline-tools ---
export ANDROID_HOME="$TC/android-sdk"
export ANDROID_SDK_ROOT="$ANDROID_HOME"
SDKM="$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager"
if [ ! -x "$SDKM" ]; then
  echo ">> cmdline-tools $CMDLINE_VER"
  fetch "https://dl.google.com/android/repository/commandlinetools-linux-${CMDLINE_VER}_latest.zip" "$TC/cmdline.zip"
  rm -rf "$TC/_cmdline"; mkdir -p "$TC/_cmdline"
  unzip -q "$TC/cmdline.zip" -d "$TC/_cmdline"
  mkdir -p "$ANDROID_HOME/cmdline-tools/latest"
  mv "$TC/_cmdline/cmdline-tools/"* "$ANDROID_HOME/cmdline-tools/latest/"
  rm -rf "$TC/_cmdline" "$TC/cmdline.zip"
fi

echo ">> aceitando licenças e instalando pacotes do SDK"
yes | "$SDKM" --sdk_root="$ANDROID_HOME" --licenses >/dev/null || true
"$SDKM" --sdk_root="$ANDROID_HOME" \
  "platform-tools" "platforms;$PLATFORM" "build-tools;$BUILD_TOOLS" "ndk;$NDK_VER"

export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/$NDK_VER"
echo "ANDROID_NDK_HOME=$ANDROID_NDK_HOME"

# --- Rust targets + cargo-ndk ---
echo ">> targets Rust p/ Android"
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android
command -v cargo-ndk >/dev/null || cargo install cargo-ndk --locked

cat > "$ROOT/.toolchain-env.sh" <<EOF
export JAVA_HOME="$JAVA_HOME"
export ANDROID_HOME="$ANDROID_HOME"
export ANDROID_SDK_ROOT="$ANDROID_HOME"
export ANDROID_NDK_HOME="$ANDROID_NDK_HOME"
export PATH="\$JAVA_HOME/bin:\$ANDROID_HOME/platform-tools:\$PATH"
EOF
echo ">> pronto. 'source .toolchain-env.sh' pra usar no shell atual."
