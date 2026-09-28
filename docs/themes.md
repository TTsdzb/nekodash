# 图标与主题

`logo.png` 是图标源文件。`python3 scripts/generate_icons.py` 使用 ImageMagick
生成窗口图标、Windows ICO、macOS ICNS 和 Android 各密度启动器资源。
Linux 发布包中的 `install.sh` 安装用户级应用、菜单项与图标；需要将
`~/.local/bin` 加入 `PATH`。

主题源文件为 `assets/theme/material-tonalSpot.json`，保留原始导出的完整调色板。
运行 `python3 scripts/generate_theme.py` 更新 `ui/monet.slint`。界面使用明暗两套
主色、容器、表面、文字与错误颜色；成功、警告继续使用独立的语义色。

默认使用当前布局与莫奈配色。Material 对比版本使用 Slint Material 标准控件，
配合圆角卡片与紧凑图标按钮：

```sh
SLINT_STYLE=material cargo run --locked
```

调试构建中可通过 `NEKODASH_PALETTE=original cargo run --locked` 查看原配色。
`SLINT_STYLE=fluent cargo run --locked` 切回默认控件样式。

Slint 1.18.1 的标准控件调色板为只读。`ui/styles` 保存固定版本的控件源文件，
将调色板角色映射到导入的主题；来源和再生成方式见该目录 README。

## 生成对比页

使用 Slint 1.18.1 的 `slint-viewer`：

```sh
python3 scripts/preview_themes.py --viewer /path/to/slint-viewer
```

打开 `target/theme-preview/gallery/index.html`，可切换概览、代理、配置、连接页面，
以及明暗模式和桌面、窄屏尺寸。截图直接渲染项目的 Slint 组件，数据来自固定示例。
实际交互通过 `scripts/ui_smoke.py` 对独立 Mihomo 测试进程验证。
