# OmniRecovery on Windows

1. Extract the OmniRecovery Windows portable ZIP to a writable folder. Open `OmniRecovery.exe` from the extracted folder.
2. Get the Windows ZIP from the [Android SDK Platform-Tools download page](https://developer.android.com/tools/releases/platform-tools). Extract its complete `platform-tools` folder next to `OmniRecovery.exe`. Keep the DLLs with `adb.exe`. OmniRecovery also checks the default Android Studio SDK location and PATH.
3. Connect the unlocked Android phone with a USB data cable. Turn on **USB debugging** in **Developer options**, approve this computer's fingerprint on the phone, then choose **My devices → Refresh devices**.

```text
OmniRecovery.exe
OmniRecovery-cli.exe
platform-tools/
  adb.exe
  AdbWinApi.dll
  AdbWinUsbApi.dll
```

If the phone appears in Windows but OmniRecovery asks for a USB driver, check **Device Manager** for the phone’s **ADB Interface**. Install the manufacturer’s driver if Windows requires one. File Explorer access can work over MTP while ADB access is unavailable. For Samsung, use the [Samsung USB driver](https://developer.samsung.com/galaxy/others/android-usb-driver-for-windows). For other brands, see [Android’s OEM USB driver directory](https://developer.android.com/studio/run/oem-usb).

To check what ADB sees, open PowerShell in the extracted OmniRecovery folder and run `& .\platform-tools\adb.exe devices -l`. **unauthorized** means the phone is waiting for approval. **offline** means ADB has detected the transport and the phone is unresponsive. An empty list means ADB has no connected device.

**Screen mirror** also needs the [Windows scrcpy package](https://github.com/Genymobile/scrcpy/blob/master/doc/windows.md). Extract its entire contents to a `scrcpy` folder beside OmniRecovery, with `scrcpy.exe` directly inside. Open **Screen mirror → Detect devices** after ADB recognizes the phone.

For **Forensics**, place installed Sleuth Kit executables in a `sleuthkit` folder and the [CGSecurity Windows package](https://www.cgsecurity.org/wiki/TestDisk_Download) contents in a `testdisk` folder beside OmniRecovery, or put the helper directories on PATH. Keep each tool's required files alongside its executable. The portable ZIP contains OmniRecovery's desktop and CLI executables and this guide. Download helpers from their upstream publishers.
