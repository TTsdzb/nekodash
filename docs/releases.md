# 构建与发布

推送 tag 后，[Release 工作流](../.github/workflows/release.yml) 会构建以下产物，
全部成功后发布到对应的 GitHub Release，并附上 `SHA256SUMS`。

| 平台 | 文件 | 使用方法 |
| --- | --- | --- |
| Linux x64 | `NekoDash-linux-x86_64.tar.gz` | 解压后运行 `NekoDash/nekodash` |
| Windows x64 | `NekoDash-windows-x86_64.zip` | 解压后运行 `NekoDash/nekodash.exe` |
| macOS Apple Silicon | `NekoDash-macos-aarch64.zip` | 解压后将 `NekoDash.app` 拖入「应用程序」 |
| macOS Intel | `NekoDash-macos-x86_64.zip` | 同上 |
| Android arm64 | `NekoDash-android-arm64-v8a.apk` | 在设备上安装 APK |

Linux 在 Ubuntu 22.04 构建，需要 glibc 2.35+、Fontconfig、xkbcommon、
Wayland 或 X11 桌面环境及可用的图形驱动。
macOS 最低版本为 13；Android 最低版本为 9（API 28）。

## Android 签名

在仓库 **Settings → Secrets and variables → Actions → New repository secret**
中配置以下四项：

| Secret | 内容 |
| --- | --- |
| `ANDROID_KEYSTORE_BASE64` | keystore 文件的完整 Base64 编码 |
| `ANDROID_KEYSTORE_PASSWORD` | keystore 密码 |
| `ANDROID_KEY_ALIAS` | 要使用的密钥别名 |
| `ANDROID_KEY_PASSWORD` | 该别名对应的密钥密码 |

密钥密码与 keystore 密码相同时，两项填相同值。已有安装要保持签名密钥一致，
以便直接更新。

Linux 上可直接通过 GitHub CLI 上传编码后的文件：

```sh
base64 -w 0 /path/to/release.keystore | gh secret set ANDROID_KEYSTORE_BASE64 --repo TTsdzb/nekodash
gh secret set ANDROID_KEYSTORE_PASSWORD --repo TTsdzb/nekodash
gh secret set ANDROID_KEY_ALIAS --repo TTsdzb/nekodash
gh secret set ANDROID_KEY_PASSWORD --repo TTsdzb/nekodash
```

后三条命令会交互式读取值。macOS 将第一条中的 `base64 -w 0 ...` 替换为
`base64 -i /path/to/release.keystore | tr -d '\n'`。
也可以在 Windows PowerShell 中生成编码后，填入网页上的 Secret：

```powershell
[Convert]::ToBase64String([IO.File]::ReadAllBytes('C:\path\release.keystore'))
```

签名文件在步骤结束时清理。APK 使用 SDK 的 `apksigner` 签名和验证，
并通过 `zipalign -c -P 16 4` 检查对齐。

## 发版

1. 更新根 `Cargo.toml` 中的 `package.version`，运行 `cargo check --locked`；
   如果提示锁文件需要更新，运行 `cargo check` 并提交 `Cargo.lock`。
   APK 的 `versionName` 和 `versionCode` 由 cargo-apk 根据这个版本生成，
   每次正式更新都应递增版本号。
2. 确认 Core 工作流通过，将版本修改提交并推送。
3. 创建并推送 tag，例如：

   ```sh
   git tag -a v0.1.0 -m 'NekoDash 0.1.0'
   git push origin v0.1.0
   ```

所有 tag 都会触发构建，建议用 `v<package.version>` 命名。
Release 先以草稿创建，上传五个平台产物和校验文件后发布。
如果上传中断，可以在 Actions 中重跑失败任务。

在 **Actions → Release → Run workflow** 中也可以手动验证构建，
完成后从该次运行的 Artifacts 下载产物。手动运行只保存构建产物。
Android Secrets 尚未配置时，可取消勾选 Android 来验证四种桌面构建。

## macOS 首次打开

`.app` 在构建时完成 ad-hoc 签名并验证。首次打开时若系统要求确认来源，
先尝试打开应用，再到 **系统设置 → 隐私与安全性 → 仍要打开** 确认。
具体界面见 [Apple 的打开应用说明](https://support.apple.com/102445)。

## 校验下载

将 `SHA256SUMS` 与下载的文件放在同一目录。Linux 可运行：

```sh
sha256sum --check --ignore-missing SHA256SUMS
```

macOS 可使用 `shasum -a 256 <文件>`，Windows 可使用
`Get-FileHash <文件> -Algorithm SHA256`，与 `SHA256SUMS` 中对应值比较。

## 构建工具

Rust 固定为 1.98.1，Cargo 依赖使用 `--locked`。
Android 使用 NDK r29（29.0.14206865）、SDK API 35、Build Tools 35.0.1
和 cargo-apk 0.10.0。打包脚本位于 `scripts/package_desktop.py`、
`scripts/build_android.sh` 和 `scripts/sign_android.sh`。

Android 页面对齐依据 [Android 官方文档](https://developer.android.com/guide/practices/page-sizes)，
签名参数见 [apksigner 文档](https://developer.android.com/tools/apksigner)。
