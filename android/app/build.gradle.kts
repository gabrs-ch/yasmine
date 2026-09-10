import org.gradle.internal.os.OperatingSystem

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

// Raiz do workspace Cargo (este projeto Gradle vive em android/).
val rustRoot: File = rootProject.projectDir.parentFile
val abis: List<String> = (project.findProperty("yasmineAbis") as String?
    ?: "arm64-v8a,armeabi-v7a,x86_64").split(",").map { it.trim() }.filter { it.isNotEmpty() }

val jniLibsDir = layout.projectDirectory.dir("src/main/jniLibs")
val uniffiOutDir = layout.buildDirectory.dir("generated/uniffi")
val cargoTargetDir: String =
    System.getenv("CARGO_TARGET_DIR") ?: rustRoot.resolve("target").absolutePath

fun cargo(vararg args: String) = listOf(if (OperatingSystem.current().isWindows) "cargo.exe" else "cargo") + args

/**
 * Compila o cdylib Rust (player-core + yasmine-sync via uniffi) para cada ABI,
 * com `cargo-ndk`, e joga os `.so` em src/main/jniLibs/<abi>/.
 * Precisa de `cargo-ndk` no PATH e ANDROID_NDK_HOME apontando pro NDK.
 * `scripts/setup-android.sh` deixa isso pronto.
 */
val buildRustLib by tasks.registering(Exec::class) {
    group = "rust"
    description = "cargo-ndk build do yasmine-android-ffi para ${abis.joinToString()}"
    workingDir = rustRoot
    val cmd = cargo("ndk").toMutableList()
    abis.forEach { cmd += listOf("-t", it) }
    cmd += listOf("-o", jniLibsDir.asFile.absolutePath, "build", "--release", "-p", "yasmine-android-ffi")
    commandLine(cmd)
    inputs.dir(rustRoot.resolve("crates"))
    outputs.dir(jniLibsDir)
}

/**
 * cdylib do **host** — só pra o uniffi-bindgen extrair a metadata. O modo
 * `--library` do bindgen não lê um `.so` de outra arquitetura, então o `.so`
 * arm64 do `buildRustLib` não serve pra isso; os bindings são Kotlin puro e a
 * arquitetura não importa.
 */
val buildFfiHostLib by tasks.registering(Exec::class) {
    group = "rust"
    description = "cargo build (host) do yasmine-android-ffi p/ os bindings"
    workingDir = rustRoot
    commandLine(cargo("build", "-p", "yasmine-android-ffi", "--lib"))
    inputs.dir(rustRoot.resolve("crates/android-ffi/src"))
    outputs.file("$cargoTargetDir/debug/libyasmine_ffi.so")
}

/** Gera os bindings Kotlin (`uniffi.yasmine_ffi.*`). */
val generateUniffiBindings by tasks.registering(Exec::class) {
    group = "rust"
    description = "uniffi-bindgen generate (Kotlin)"
    dependsOn(buildFfiHostLib)
    workingDir = rustRoot
    commandLine(
        cargo(
            "run", "-p", "yasmine-android-ffi", "--bin", "uniffi-bindgen", "--",
            "generate", "--library", "$cargoTargetDir/debug/libyasmine_ffi.so",
            "--language", "kotlin",
            "--out-dir", uniffiOutDir.get().asFile.absolutePath,
        )
    )
    inputs.files(buildFfiHostLib.get().outputs.files)
    outputs.dir(uniffiOutDir)
}

android {
    namespace = "app.yasmine"
    compileSdk = 35

    defaultConfig {
        applicationId = "app.yasmine"
        minSdk = 26
        targetSdk = 35
        // O Release passa `-PversionName`/`-PversionCode` derivados da tag; o
        // build local usa o default.
        versionCode = (project.findProperty("versionCode") as String?)?.toInt() ?: 2
        versionName = (project.findProperty("versionName") as String?) ?: "0.5.8"
        ndk { abiFilters += abis }
    }

    buildTypes {
        // R8 ligado também no debug: sem ele o dex do Compose deixa o APK
        // com ~35 MB. As regras de keep de JNA/uniffi/media3 estão em
        // proguard-rules.pro.
        debug {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }

    buildFeatures { compose = true }

    sourceSets["main"].kotlin.srcDir(uniffiOutDir)
    sourceSets["main"].jniLibs.srcDir(jniLibsDir)

    packaging {
        resources.excludes += setOf("/META-INF/{AL2.0,LGPL2.1}", "/META-INF/DEPENDENCIES")
    }
}

// Os bindings e os .so por ABI têm de existir antes de compilar/empacotar.
tasks.named("preBuild").configure { dependsOn(generateUniffiBindings, buildRustLib) }
tasks.withType<org.jetbrains.kotlin.gradle.tasks.KotlinCompile>().configureEach {
    dependsOn(generateUniffiBindings)
}
// O merge das libs nativas precisa dos .so recém-copiados pra jniLibs.
tasks.matching { it.name.matches(Regex("merge.*(JniLibFolders|NativeLibs)")) }
    .configureEach { dependsOn(buildRustLib) }

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.navigation.compose)
    implementation(libs.kotlinx.coroutines.android)

    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.graphics)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons.extended)
    debugImplementation(libs.compose.ui.tooling)

    implementation(libs.media3.exoplayer)
    implementation(libs.media3.session)
    implementation(libs.media3.ui)

    implementation(libs.camera.camera2)
    implementation(libs.camera.lifecycle)
    implementation(libs.camera.view)
    implementation(libs.mlkit.barcode.scanning)

    implementation(libs.coil.compose)

    // Runtime da FFI uniffi no Android.
    implementation("${libs.jna.get().module}:${libs.jna.get().version}@aar")
}
