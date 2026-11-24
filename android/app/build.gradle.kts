plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

// ABIs: só arm64-v8a (celulares) por padrão; para o emulador use -PeloAbis=arm64-v8a,x86_64.
val eloAbis = (project.findProperty("eloAbis") as String?)?.split(",")?.map { it.trim() } ?: listOf("arm64-v8a")

android {
    namespace = "io.github.ricardomendes95.elo"
    compileSdk = 36

    defaultConfig {
        applicationId = "io.github.ricardomendes95.elo"
        minSdk = 26
        targetSdk = 36
        versionCode = 7
        versionName = "0.3.7"
        ndk { abiFilters += eloAbis }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlin { compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) } }
    buildFeatures { compose = true }
}

dependencies {
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.icons.extended)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.navigation.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.kotlinx.coroutines.android)
    // Exigido pelos bindings UniFFI (a lib nativa vem no AAR).
    implementation("net.java.dev.jna:jna:${libs.versions.jna.get()}@aar")
}

// Compila o núcleo Rust e gera os bindings; pule com -PskipRust.
val buildRust by tasks.registering(Exec::class) {
    group = "build"
    description = "Compila o núcleo Rust (libelo_android.so) e gera os bindings UniFFI."
    workingDir = rootProject.projectDir
    commandLine("bash", "build-rust.sh")
    environment("ELO_ABIS", eloAbis.joinToString(" "))
    onlyIf { !project.hasProperty("skipRust") }
}
tasks.named("preBuild") { dependsOn(buildRust) }
