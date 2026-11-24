#!/usr/bin/env bash
# Compila o núcleo Rust (crates/android → libelo_android.so) para o Android e
# gera os bindings Kotlin (UniFFI). Rode antes do Gradle, ou deixe o Gradle
# chamar (tarefa `buildRust`, ligada ao preBuild).
#
# ABIs: ELO_ABIS="arm64-v8a x86_64" (padrão: arm64-v8a, celulares reais; inclua
# x86_64 para o emulador). Perfil: ELO_PROFILE=release|debug (padrão release).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
if [ -z "${ANDROID_NDK_HOME:-}" ]; then
  ANDROID_NDK_HOME="$(ls -d "$ANDROID_HOME"/ndk/* 2>/dev/null | sort -V | tail -1)"
  export ANDROID_NDK_HOME
fi
abis="${ELO_ABIS:-arm64-v8a}"
profile="${ELO_PROFILE:-release}"
args=()
for a in $abis; do args+=(-t "$a"); done
rel=()
[ "$profile" = release ] && rel=(--release)

cd "$root"
cargo ndk "${args[@]}" -P 26 -o "$here/app/src/main/jniLibs" build "${rel[@]}" -p elo-android

# Bindings: gerados a partir da primeira ABI (a API é a mesma em todas).
first="${abis%% *}"
case "$first" in
  arm64-v8a) triple=aarch64-linux-android ;;
  armeabi-v7a) triple=armv7-linux-androideabi ;;
  x86_64) triple=x86_64-linux-android ;;
  x86) triple=i686-linux-android ;;
esac
lib="$root/target/$triple/$profile/libelo_android.so"
cargo run -q -p elo-android --features bindgen --bin uniffi-bindgen -- \
  generate --library "$lib" --language kotlin --no-format \
  --out-dir "$here/app/src/main/java"
echo "ok: jniLibs ($abis) e bindings em app/src/main/java/io/github/ricardomendes95/elo/ffi"
