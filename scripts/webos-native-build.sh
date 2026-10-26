#!/usr/bin/env bash
set -euo pipefail
MANIFEST="${1:?manifest path required}"
NAME="${2:?binary name required}"
OUTPUT="${3:?output path required}"
SDK="${WEBOS_SDK:-$HOME/.local/share/webosbrew-native-toolchain/arm-webos-linux-gnueabi_sdk-buildroot}"
SDK="$(cd "$SDK" && pwd)"
BIN="$(cd "$(dirname "$0")" && pwd)"
TARGET=armv7-unknown-linux-gnueabi
"$BIN/webos-native-cc" --version >/dev/null
export CARGO_TARGET_DIR="$(cd "$(dirname "$MANIFEST")" && pwd)/target/native"
export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABI_LINKER="$BIN/webos-native-cc"
export CC_armv7_unknown_linux_gnueabi="$BIN/webos-native-cc"
export AR_armv7_unknown_linux_gnueabi="$SDK/bin/arm-webos-linux-gnueabi-ar"
export RANLIB_armv7_unknown_linux_gnueabi="$SDK/bin/arm-webos-linux-gnueabi-ranlib"
if [ -n "${WEBOS_RUSTC:-}" ] || [ -n "${WEBOS_CARGO:-}" ]; then
    : "${WEBOS_RUSTC:?Set WEBOS_RUSTC and WEBOS_CARGO together}"
    : "${WEBOS_CARGO:?Set WEBOS_RUSTC and WEBOS_CARGO together}"
    test -x "$WEBOS_RUSTC" && test -x "$WEBOS_CARGO" || { echo "Supplied Rust compiler/Cargo is not executable." >&2; exit 1; }
    export RUSTC="$WEBOS_RUSTC"
    "$WEBOS_CARGO" build --locked --release --target "$TARGET" --manifest-path "$MANIFEST" --bin "$NAME"
else
    # Homebrew cargo/rustc can shadow rustup and cannot see its installed targets.
    RUSTC="$(rustup which --toolchain stable rustc)"
    export RUSTC
    rustup run stable cargo build --locked --release --target "$TARGET" --manifest-path "$MANIFEST" --bin "$NAME"
fi
BINARY="$CARGO_TARGET_DIR/$TARGET/release/$NAME"
mkdir -p "$(dirname "$OUTPUT")"
STAGED="$(mktemp "$OUTPUT.XXXXXX")"
trap 'rm -f "$STAGED"' EXIT
cp "$BINARY" "$STAGED"
READELF="$SDK/bin/arm-webos-linux-gnueabi-readelf"
HEADER="$("$READELF" -h "$STAGED")"
grep -q 'Class:.*ELF32' <<< "$HEADER"
grep -q 'Data:.*little endian' <<< "$HEADER"
grep -q 'Machine:.*ARM' <<< "$HEADER"
grep -q 'Flags:.*Version5 EABI, soft-float ABI' <<< "$HEADER"
PROGRAM_HEADERS="$("$READELF" -l "$STAGED")"
grep -Fq '[Requesting program interpreter: /lib/ld-linux.so.3]' <<< "$PROGRAM_HEADERS"
VERSIONS="$("$READELF" --version-info "$STAGED")"
grep -o 'GLIBC_[0-9][0-9.]*' <<< "$VERSIONS" | awk -F_ \
  '{split($2,v,"."); if(v[1]>2 || (v[1]==2 && (v[2]>28 || (v[2]==28 && v[3]>0)))) exit 1}'
chmod 755 "$STAGED"
mv -f "$STAGED" "$OUTPUT"
echo "Verified ELF32 ARM, soft-float ABI, glibc <= 2.28: $OUTPUT"
