# 图标与主题

`logo.png` 是透明图标源文件。`python3 scripts/generate_icons.py` 使用 ImageMagick
生成窗口图标、Windows ICO、macOS ICNS 和 Android 各密度启动器资源。
Android 启动器使用自适应图标：`#30466E` 纯色背景与透明前景分层，由系统提供图标形状。
前景位于 108 dp 画布中央，主体宽度为 66 dp。
Linux 发布包中的 `install.sh` 安装用户级应用、菜单项与图标；需要将
`~/.local/bin` 加入 `PATH`。

## 配色

配色定义在 `ui/state.slint` 的 `Theme` 中。明暗两套颜色参考图片中的冷蓝色、
蓝灰阴影和灰粉色，保留卡片、边框及选中状态的层次。

| 用途 | 深色 | 浅色 |
| --- | --- | --- |
| 背景 | `#101521` | `#F1F5FC` |
| 卡片 | `#192131` | `#FCFDFF` |
| 主色 | `#A8BFFA` | `#4665A2` |
| 选中背景 | `#30466E` | `#DDE7FA` |
| 正文 | `#EDF2FC` | `#24324C` |
| 辅助文字 | `#B0BDD4` | `#526582` |

正文、辅助文字、选中项和主要按钮文字与对应背景的对比度均不低于 4.5:1。
标准控件使用 Slint Fluent 样式，通过 `Palette.color-scheme` 跟随明暗模式。

## 生成对比页

使用 Slint 1.18.1 的 `slint-viewer`：

```sh
python3 scripts/preview_themes.py --viewer /path/to/slint-viewer
```

打开 `target/theme-preview/hand-tuned/index.html`，比较原配色和手调配色。
可切换概览、代理、配置、连接页面，以及明暗模式和桌面、窄屏尺寸。
截图渲染项目的 Slint 组件，数据来自固定示例。
实际交互通过 `scripts/ui_smoke.py` 对独立 Mihomo 测试进程验证。
