#!/usr/bin/env python3
"""Render actual Slint pages with identical fixtures into a theme comparison gallery."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent

# Fixed palette from the original interface, used only for visual comparison.
ORIGINAL_THEME = """export global Theme {
    in-out property <bool> dark: true;
    changed dark => { Palette.color-scheme = root.dark ? ColorScheme.dark : ColorScheme.light; }
    out property <color> bg: dark ? #11151e : #f5f5fa;
    out property <color> panel: dark ? #171d29 : #ffffff;
    out property <color> raised: dark ? #222a3b : #eeeef6;
    out property <color> border: dark ? #303a4d : #dddded;
    out property <color> text: dark ? #e5e9f3 : #252b3b;
    out property <color> muted: dark ? #9aa7bc : #656c80;
    out property <color> accent: dark ? #aa9aff : #6951cd;
    out property <color> selected: dark ? #383254 : #e5defa;
    out property <color> success: dark ? #63d7ac : #16805e;
    out property <color> warning: dark ? #f0bf75 : #946017;
    out property <color> error: dark ? #ff8995 : #b82c4a;
    public pure function chart-color(index: int) -> color {
        return index == 0 ? root.accent : index == 1 ? root.success : index == 2 ? root.warning
            : index == 3 ? (root.dark ? #75bbff : #2676c9) : root.error;
    }
}

"""


def literal(value):
    if isinstance(value, dict):
        return '{' + ','.join(f'{key}:{literal(item)}' for key, item in value.items()) + '}'
    if isinstance(value, list):
        return '[' + ','.join(literal(item) for item in value) + ']'
    return json.dumps(value, ensure_ascii=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--viewer', default=shutil.which('slint-viewer'))
    parser.add_argument('--output', default='target/theme-preview/hand-tuned')
    parser.add_argument('--pages', nargs='+', choices=['overview', 'proxies', 'config', 'connect'],
                        default=['overview', 'proxies', 'config', 'connect'])
    args = parser.parse_args()
    if not args.viewer:
        parser.error('Provide --viewer pointing to Slint 1.18.1 slint-viewer')
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    translations = json.loads((ROOT / 'assets/i18n/zh.json').read_text())
    translations.update({'coreConfig': '核心配置', 'dnsQuery': 'DNS 查询',
                         'connected': '已连接', 'notConnected': '未连接', 'settings': '设置',
                         'testAll': '全部测速', 'cardMode': '卡片', 'listMode': '列表'})
    stats = [dict(label=label, value=value, icon=icon) for label, value, icon in [
        ('上传', '128 KB/s', 8), ('下载', '3.6 MB/s', 9), ('上传总量', '349.8 MB', 8),
        ('下载总量', '1.1 GB', 9), ('活动连接', '24', 3), ('内存使用情况', '78.6 MB', 4)]]
    charts = [dict(kind=0, title='实时流量', maximum='4 MB/s', footer='', series=[
        {'label': '上传', 'path': 'M 0 148 L 60 139 L 120 145 L 180 119 L 240 134 L 300 110 L 360 122 L 420 95 L 480 118 L 540 108 L 600 121', 'color-index': 0},
        {'label': '下载', 'path': 'M 0 130 L 60 95 L 120 105 L 180 30 L 240 65 L 300 48 L 360 70 L 420 20 L 480 50 L 540 30 L 600 40', 'color-index': 1}]),
        dict(kind=0, title='内存使用情况', maximum='128 MB', footer='', series=[
        {'label': '内存', 'path': 'M 0 115 L 60 108 L 120 85 L 180 85 L 240 80 L 300 72 L 360 84 L 420 75 L 480 62 L 540 67 L 600 62', 'color-index': 0}])]
    groups = []
    for group, names in [('Proxy', ['香港 01', '香港 02', '日本 01', '新加坡 01', '美国 01', 'DIRECT']),
                         ('Streaming', ['日本 01', '日本 02', '新加坡 01', '美国 01'])]:
        groups.append(dict(name=group, kind='Selector', current=names[0], fixed=False, expanded=True,
                           count=str(len(names)), nodes=[dict(name=name, kind='Vless' if name != 'DIRECT' else 'Direct',
                           delay=f'{34 + index * 23} ms', latency=34 + index * 23, selected=index == 0, udp=True)
                           for index, name in enumerate(names)]))
    settings = [dict(key=key, label=label, value=value, kind=kind, checked=checked, options=options)
                for key, label, value, kind, checked, options in [
                    ('mode', '运行模式', '规则', 2, False, ['规则', '全局', '直连']),
                    ('log-level', '日志级别', 'info', 2, False, ['debug', 'info', 'warning', 'error']),
                    ('ipv6', 'IPv6', '', 0, True, []), ('allow-lan', '允许局域网连接', '', 0, False, []),
                    ('mixed-port', '混合端口', '7890', 1, False, [])]]
    for setting in settings:
        setting['choice-index'] = setting['options'].index(setting['value']) if setting['value'] in setting['options'] else -1
    with tempfile.TemporaryDirectory(prefix='nekodash-theme-preview-') as temporary:
        staging = Path(temporary)
        shutil.copytree(ROOT / 'ui', staging / 'ui')
        shutil.copytree(ROOT / 'assets/icons', staging / 'assets/icons')
        state = staging / 'ui/state.slint'
        source = state.read_text()
        # Only the preview translation callback is replaced; all visual components are copied verbatim.
        translation = 'public pure function translate(key: string, language: string) -> string {\n'
        for key, value in translations.items():
            if isinstance(value, str):
                translation += f'if key == {literal(key)} {{ return {literal(value)}; }}\n'
        translation += 'return key; }'
        translated = source.replace('pure callback translate(string, string) -> string;', translation)
        for variant in ['original', 'adjusted']:
            current = translated
            if variant == 'original':
                start = current.index('export global Theme')
                end = current.index('export global I18n')
                current = current[:start] + ORIGINAL_THEME + current[end:]
            state.write_text(current)
            for scheme in ['dark', 'light']:
                for size, dimensions in [('desktop', '1280x820'), ('mobile', '412x892')]:
                    for page, index in [('overview', 0), ('proxies', 1), ('config', 6), ('connect', 0)]:
                        if page not in args.pages:
                            continue
                        values = {'page': index, 'connected': page != 'connect', 'status': '未连接' if page == 'connect' else '已连接',
                                  'endpoint-label': '本机', 'endpoint-url': 'http://127.0.0.1:9090/',
                                  'stats': stats, 'charts': charts, 'groups': groups, 'config-rows': settings,
                                  'count': '2', 'secondary-count': '1', 'mode-index': 0}
                        init = f'Theme.dark = {literal(scheme == "dark")};\n'
                        init += '\n'.join(f'ViewData.{key} = {literal(value)};' for key, value in values.items())
                        wrapper = staging / 'preview.slint'
                        wrapper.write_text('import { AppWindow, Theme, ViewData } from "ui/app-window.slint";\n'
                                           f'export component Preview inherits AppWindow {{ init => {{ {init} }} }}\n')
                        name = f'{variant}-{scheme}-{size}-{page}.png'
                        subprocess.run([args.viewer, '--style', 'fluent', '--size', dimensions, '--screenshot', str(output / name), str(wrapper)],
                                       check=True)
                        print(name, flush=True)
    (output / 'index.html').write_text('''<!doctype html><html lang="zh-CN"><meta charset="utf-8">
<title>NekoDash 主题对比</title><style>
body{margin:0;background:#131318;color:#e4e1e9;font:15px system-ui}header{padding:24px;position:sticky;top:0;background:#1b1b21;z-index:1}h1{font-size:22px;margin:0 0 8px}p{color:#c7c5d0}select{margin:4px 16px 0 4px;padding:8px;background:#35343a;color:inherit;border:1px solid #918f9a;border-radius:8px}main{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:16px;padding:24px}h2{font-size:16px}img{width:100%;border-radius:12px;box-shadow:0 0 0 1px #46464f}a{color:inherit}main.mobile img{max-width:412px}small{color:#c7c5d0}@media(max-width:900px){main{grid-template-columns:1fr}}</style>
<header><h1>NekoDash · 配色对比</h1><p>相同的 Slint 页面与固定示例数据。点击图片查看原尺寸。</p>
<label>页面<select id="page"><option value="overview">概览</option><option value="proxies">代理</option><option value="config">配置</option><option value="connect">连接</option></select></label>
<label>模式<select id="scheme"><option value="dark">深色</option><option value="light">浅色</option></select></label>
<label>尺寸<select id="size"><option value="desktop">桌面 1280 × 820</option><option value="mobile">窄屏 412 × 892</option></select></label></header>
<main id="grid"></main><script>
const variants=[['original','原配色'],['adjusted','手调配色 · 雾蓝']];
const grid=document.getElementById('grid'), page=document.getElementById('page'), scheme=document.getElementById('scheme'), size=document.getElementById('size');
function render(){grid.className=size.value;grid.replaceChildren(...variants.map(([id,title])=>{const section=document.createElement('section');const heading=document.createElement('h2');heading.textContent=title;const link=document.createElement('a');link.target='_blank';link.href=`${id}-${scheme.value}-${size.value}-${page.value}.png`;const image=document.createElement('img');image.src=link.href;image.alt=title;link.append(image);section.append(heading,link);return section}));}
for(const id of ['page','scheme','size'])document.getElementById(id).addEventListener('change',render);render();</script></html>''')
    print(f'Gallery: {output / "index.html"}')


if __name__ == '__main__':
    main()
