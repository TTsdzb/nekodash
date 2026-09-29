# nekodash-bin

从 GitHub Release 下载 Linux x86_64 二进制并生成 Arch Linux 包。

## 构建与安装

在此目录运行：

```sh
makepkg -si
```

需要 `base-devel`。安装后可通过应用菜单或 `nekodash` 命令启动。
包内包含程序、桌面入口、256×256 图标和许可证，文件安装至 `/usr`。

## 更新版本

1. 将 `PKGBUILD` 的 `pkgver` 改为已发布版本，`pkgrel` 重置为 `1`。
2. 下载该版本 Linux 产物，核对 Release 的 `SHA256SUMS`，更新 `sha256sums`。
3. 生成元数据并重新构建：

   ```sh
   makepkg --printsrcinfo > .SRCINFO
   makepkg --cleanbuild
   ```

打包依据：[Arch package guidelines](https://wiki.archlinux.org/title/Arch_package_guidelines)。
