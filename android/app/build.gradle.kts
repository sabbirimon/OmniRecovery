plugins {
    id("com.android.application") version "9.2.1"
}

android {
    namespace = "local.omnirecovery.phone"
    compileSdk = 36

    defaultConfig {
        applicationId = "local.omnirecovery.phone"
        minSdk = 26
        targetSdk = 36
        versionCode = 3
        versionName = "0.3.0"
    }
}
