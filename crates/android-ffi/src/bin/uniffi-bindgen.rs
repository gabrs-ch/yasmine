//! Gerador dos bindings Kotlin. Uso:
//!
//! ```sh
//! cargo run -p yasmine-android-ffi --bin uniffi-bindgen -- \
//!     generate --library target/<abi>/release/libyasmine_ffi.so \
//!     --language kotlin --out-dir android/app/build/generated/uniffi
//! ```
//!
//! O Gradle chama isto via `cargo-ndk` no `preBuild` (ver
//! `android/app/build.gradle.kts`).

fn main() {
    uniffi::uniffi_bindgen_main()
}
