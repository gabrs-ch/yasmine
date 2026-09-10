# --- JNA: classes nativas e callbacks têm de sobreviver ao R8 ---
-dontwarn java.awt.**
-keep class com.sun.jna.** { *; }
-keep interface com.sun.jna.** { *; }
-keep class * implements com.sun.jna.Library { *; }
-keep class * implements com.sun.jna.Callback { *; }
-keepclassmembers class * extends com.sun.jna.Structure { <fields>; }

# --- Bindings uniffi: carregados por nome via JNA ---
-keep class uniffi.yasmine_ffi.** { *; }
-keep interface uniffi.yasmine_ffi.** { *; }

# --- media3 / ExoPlayer ---
-keep class androidx.media3.** { *; }
-dontwarn androidx.media3.**

# --- ML Kit barcode ---
-keep class com.google.mlkit.** { *; }
-keep class com.google.android.gms.internal.mlkit_vision_barcode.** { *; }
-dontwarn com.google.mlkit.**

# --- Aplicação (referenciada pelo manifest, mas garantindo) ---
-keep class app.yasmine.YasmineApp { *; }
-keep class app.yasmine.playback.PlaybackService { *; }
