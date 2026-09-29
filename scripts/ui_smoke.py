#!/usr/bin/env python3
"""UI smoke test against an owned Mihomo process and Slint's debug MCP server."""
import argparse
import base64
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request

parser = argparse.ArgumentParser()
parser.add_argument("--binary", default="target/debug/nekodash")
parser.add_argument("--mihomo", default=shutil.which("mihomo"))
parser.add_argument("--size", default="1280x820")
parser.add_argument("--output", default="target/ui-smoke")
parser.add_argument("--proxy-scroll-only", action="store_true")
parser.add_argument("--group-icons-only", action="store_true")
args = parser.parse_args()
if not args.mihomo:
    parser.error("Provide --mihomo with a test executable")
output = Path(args.output).resolve()
output.mkdir(parents=True, exist_ok=True)
home = Path(tempfile.mkdtemp(prefix="nekodash-ui-smoke-"))

def port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]

core_port, mcp_port, proxy_port = port(), port(), port()
delay_port, chart_port = port(), port()
config=f'''external-controller: 127.0.0.1:{core_port}
secret: ui-fixture
mixed-port: {proxy_port}
bind-address: 127.0.0.1
port: 0
socks-port: 0
redir-port: 0
tproxy-port: 0
allow-lan: false
mode: rule
log-level: debug
ipv6: false
tun:
  enable: false
profile:
  store-selected: false
proxies:
'''
for name in ['Hong Kong 01','Hong Kong 02','Tokyo 01','Tokyo 02','Singapore 01','Singapore 02','Los Angeles 01','London 01']:
 config+=f'  - name: {name}\n    type: direct\n'
config+=f'''proxy-groups:
  - name: Proxy
    type: select
    url: http://127.0.0.1:{delay_port}/delay
    proxies: [Hong Kong 01, Hong Kong 02, Tokyo 01, Tokyo 02, Singapore 01, Singapore 02, Los Angeles 01, London 01, DIRECT, REJECT]
  - name: Streaming
    type: select
    url: http://127.0.0.1:{delay_port}/delay
    use: [fixture-provider]
    proxies: [Tokyo 01, Singapore 01, Los Angeles 01, DIRECT]
  - name: Development
    type: select
    proxies: [Proxy, Hong Kong 01, Tokyo 01, DIRECT]
  - name: Fallback
    type: select
    proxies: [Proxy, DIRECT, REJECT]
proxy-providers:
  fixture-provider:
    type: file
    path: provider.yaml
    health-check:
      enable: false
      url: http://127.0.0.1:{delay_port}/delay
rule-providers:
  fixture-rules:
    type: file
    behavior: domain
    format: yaml
    path: rules.yaml
rules:
  - DOMAIN-SUFFIX,example.com,Proxy
  - DST-PORT,{chart_port},Development
  - DOMAIN-SUFFIX,github.com,Development
  - DOMAIN-SUFFIX,youtube.com,Streaming
  - RULE-SET,fixture-rules,Proxy
'''
for index in range(8):
    config += f'  - DOMAIN,sort-{index}.example.test,DIRECT\n'
config += '  - MATCH,Fallback\n'
(home/'config.yaml').write_text(config)
if args.group_icons_only:
    svg='<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><circle cx="12" cy="12" r="10" fill="#f0bf75"/></svg>'
    inline='data:image/svg+xml;base64,'+base64.b64encode(svg.encode()).decode()
    for group,source in [('Proxy','http://icons.example.invalid/group.svg'),('Streaming',inline),('Development','http://icons.example.invalid/group.svg'),('Fallback','http://icons.example.invalid/missing.svg')]:
        config=config.replace(f'  - name: {group}\n',f'  - name: {group}\n    icon: "{source}"\n')
    (home/'config.yaml').write_text(config)
if args.proxy_scroll_only:
    extra=[f'Scroll node {i:03}' for i in range(90)]
    config=config.replace('proxy-groups:', ''.join(f'  - name: {name}\n    type: direct\n' for name in extra)+'proxy-groups:')
    config=config.replace('proxies: [Hong Kong 01, Hong Kong 02,', 'proxies: ['+', '.join(extra)+', Hong Kong 01, Hong Kong 02,',1)
    config=config.replace('proxy-providers:', ''.join(f'  - name: Short group {i}\n    type: select\n    proxies: [DIRECT, REJECT]\n' for i in range(6))+'proxy-providers:')
    (home/'config.yaml').write_text(config)
(home/'provider.yaml').write_text('proxies:\n  - name: Provider Direct\n    type: direct\n')
(home/'rules.yaml').write_text('payload:\n  - example.test\n')


def rpc(name, **arguments):
    data = json.dumps({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}}).encode()
    request = urllib.request.Request(f"http://127.0.0.1:{mcp_port}/mcp", data, headers={"Content-Type":"application/json","Accept":"application/json, text/event-stream"})
    with urllib.request.urlopen(request, timeout=10) as response:
        raw = response.read().decode()
    for line in raw.splitlines():
        if line.startswith("data:"):
            raw = line[5:]
            break
    value = json.loads(raw)
    if "error" in value:
        raise RuntimeError(value["error"])
    value = value["result"]
    if value.get("isError"):
        raise RuntimeError(value)
    return value

def result(value):
    if "structuredContent" in value:
        return value["structuredContent"]
    return json.loads(next(item["text"] for item in value["content"] if item["type"] == "text"))

def find(identifier):
    return result(rpc("find_elements_by_id",windowHandle=window,elementsId=identifier)).get("elementHandles",[])

def screenshot(name):
    for content in rpc("take_screenshot",windowHandle=window)["content"]:
        if content["type"] == "image":
            (output / f"{name}.png").write_bytes(base64.b64decode(content["data"]))

def elements():
    root=result(rpc("get_window_properties",windowHandle=window))["rootElementHandle"]
    return result(rpc("get_element_tree",elementHandle=root,maxElements=1000))["elements"]

def button(label):
    matches=[item for item in elements() if item.get("accessibleLabel")==label and item.get("accessibleRole")=="Button" and item.get("computedOpacity",1)>0]
    if not matches:
        raise RuntimeError(f"Missing button: {label}")
    return matches[0]["handle"]

def click_label(label):
    rpc("click_element",elementHandle=button(label))
    time.sleep(.25)

def properties(handle):
    return result(rpc("get_element_properties",elementHandle=handle))

def table_headers():
    return [(handle, properties(handle)) for handle in find("DataTable::sort-header")]

def show_column(title):
    viewport = find("DataTable::list")[0]
    viewport_geometry = properties(viewport)
    left = viewport_geometry["absolutePosition"]["x"]
    right = left + viewport_geometry["size"]["width"]
    for attempt in range(30):
        for handle, item in table_headers():
            if item.get("accessibleLabel", "").split(" · ")[0] != title:
                continue
            center = item["absolutePosition"]["x"] + item["size"]["width"] / 2
            if not left + 16 < center < right - 16:
                rpc("scroll_element", elementHandle=viewport, deltaX=(left + right) / 2 - center)
                time.sleep(.2)
            return handle
        rpc("scroll_element", elementHandle=viewport,
            deltaX=10000 if attempt == 0 else -(right - left) / 2)
        time.sleep(.2)
    raise RuntimeError(f"Missing table column: {title}")

def sort_column(title, descending=False):
    header = show_column(title)
    rpc("click_element", elementHandle=header)
    direction = "降序" if descending else "升序"
    wait_for(lambda: properties(header).get("accessibleLabel", "").endswith(f" · {direction}"))
    return header

def table_rows():
    rows = []
    headers = [item for _, item in table_headers()]
    for handle in find("DataTable::data-row"):
        tree = result(rpc("get_element_tree", elementHandle=handle, maxElements=150))["elements"]
        cells = [item for item in tree if any(
            entry.get("id") == "DataTable::cell-text" for entry in item.get("typeNamesAndIds", [])
        )]
        values = {}
        for cell in cells:
            for header in headers:
                if abs(cell["absolutePosition"]["x"] - header["absolutePosition"]["x"] - 12) < 1:
                    values[header.get("accessibleLabel", "").split(" · ")[0]] = cell.get("accessibleLabel", "")
        rows.append({"handle": handle, "y": properties(handle)["absolutePosition"]["y"],
                     "cells": values, "tree": tree})
    return sorted(rows, key=lambda row: row["y"])

def transfer(sender, receiver, size):
    sender.sendall(b"x" * size)
    remaining = size
    while remaining:
        chunk = receiver.recv(min(remaining, 65536))
        assert chunk, "Fixture connection closed during transfer"
        remaining -= len(chunk)

def search(value):
    rpc("set_element_value",elementHandle=find("PageToolbar::search-input")[0],value=value)
    time.sleep(.3)

def api(path):
    request=urllib.request.Request(f"http://127.0.0.1:{core_port}{path}",headers={"Authorization":"Bearer ui-fixture"})
    with urllib.request.urlopen(request,timeout=5) as response:
        return json.load(response)

def wait_for(check):
    end=time.monotonic()+15
    while time.monotonic()<end:
        try:
            if check():
                return
        except (urllib.error.URLError, ConnectionError):
            pass
        except RuntimeError as error:
            # A live row can disappear between obtaining and inspecting its handle.
            if "element that was destroyed" not in str(error):
                raise
        time.sleep(.1)
    raise RuntimeError("Timed out waiting for UI/core state")

core=None
app=None
listener=None
peer=None
tunnel=None
chart_connections=[]
delay_server=None
icon_requests=[]

class DelayHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path.startswith('http://icons.example.invalid/'):
            icon_requests.append((self.path,self.headers.get('Authorization')))
            self.send_response(404 if self.path.endswith('missing.svg') else 200)
            self.send_header('Content-Type','image/svg+xml')
            self.end_headers()
            self.wfile.write(b'<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><circle cx="12" cy="12" r="10" fill="#f0bf75"/></svg>')
            return
        time.sleep(.025)
        self.send_response(204)
        self.end_headers()

    do_HEAD = do_GET

    def log_message(self, *_):
        pass

try:
    delay_server=ThreadingHTTPServer(("127.0.0.1",delay_port),DelayHandler)
    threading.Thread(target=delay_server.serve_forever,daemon=True).start()
    with (output/"core.log").open("w") as core_log, (output/"app.log").open("w") as app_log:
        core=subprocess.Popen([args.mihomo,"-d",str(home)],stdout=core_log,stderr=core_log)
        app_env=dict(os.environ,SLINT_BACKEND="headless",SLINT_MCP_PORT=str(mcp_port),NEKODASH_WINDOW_SIZE=args.size,NEKODASH_DATA_DIR=str(home/"settings"))
        if args.group_icons_only:
            for key in ['HTTP_PROXY','http_proxy','HTTPS_PROXY','https_proxy','ALL_PROXY','all_proxy']:
                app_env[key]=f'http://127.0.0.1:{delay_port}'
            app_env['NO_PROXY']=app_env['no_proxy']=''
        app=subprocess.Popen([str(Path(args.binary).resolve())],stdout=app_log,stderr=app_log,env=app_env)
        wait_for(lambda: result(rpc("list_windows")).get("windowHandles",[]))
        window=result(rpc("list_windows")).get("windowHandles",[])[0]
        screenshot("connect")
        for name,value in [("endpoint-label","UI Fixture"),("endpoint-url",f"http://127.0.0.1:{core_port}"),("endpoint-secret","invalid-ui-fixture")]:
            rpc("set_element_value",elementHandle=find("ConnectPage::"+name)[0],value=value)
        rpc("click_element",elementHandle=find("ConnectPage::connect-button")[0])
        wait_for(lambda: any("鉴权失败" in item.get("accessibleLabel", "") for item in elements()))
        screenshot("authentication-error")
        rpc("set_element_value",elementHandle=find("ConnectPage::endpoint-secret")[0],value="ui-fixture")
        rpc("click_element",elementHandle=find("ConnectPage::connect-button")[0])
        wait_for(lambda: not find("ConnectPage::connect-button"))
        screenshot("overview")
        if int(args.size.split("x")[0])<720:
            header=properties(find("AppWindow::mobile-header")[0])
            for identifier in ["mobile-endpoints-button", "mobile-language-button"]:
                item=properties(find("AppWindow::"+identifier)[0])
                assert abs(item["size"]["height"]-36)<1
                assert abs(item["absolutePosition"].get("y",0)-header["absolutePosition"].get("y",0)-8)<1
                assert abs(header["size"]["height"]-item["size"]["height"]-16)<1
        # Read geometry as well as pixels: changing values must not change column widths.
        stat_geometry=[properties(handle) for handle in find("OverviewPage::stat-card")]
        assert len(stat_geometry)==6
        widths=[item["size"]["width"] for item in stat_geometry]
        assert max(widths)-min(widths)<1, "Overview columns must have equal widths"
        (output/"stat-geometry.json").write_text(json.dumps(stat_geometry,ensure_ascii=False,indent=2))
        click_label("切换后端")
        screenshot("connected-endpoint-form")
        form_geometry=properties(find("ConnectPage::endpoint-form")[0])
        back_geometry=properties(find("ConnectPage::back-button")[0])
        assert back_geometry["absolutePosition"]["y"]+back_geometry["size"]["height"] <= form_geometry["absolutePosition"]["y"]+form_geometry["size"]["height"]-20
        (output/"endpoint-geometry.json").write_text(json.dumps([form_geometry,back_geometry],indent=2))
        click_label("返回")
        click_label("切换语言")
        assert properties(find("AppWindow::settings-dialog")[0])["size"]["height"]<300
        screenshot("language")
        click_label("取消")
        rpc("hover_element",elementHandle=button("切换语言"))
        time.sleep(1.2)
        screenshot("tooltip")
        mobile=int(args.size.split("x")[0])<720
        def navigate(index):
            if mobile:
                if index in [4,5,6]:
                    rpc("click_element",elementHandle=find("AppWindow::mobile-nav")[2])
                    time.sleep(.15)
                    menu=properties(find("AppWindow::more-menu")[0])
                    nav=properties(find("AppWindow::mobile-nav")[2])
                    assert abs(menu["size"]["height"]-64)<1
                    assert menu["absolutePosition"]["y"]>=0
                    assert menu["absolutePosition"]["y"]+menu["size"]["height"]<nav["absolutePosition"]["y"]
                    for handle in find("AppWindow::more-menu-button"):
                        assert abs(properties(handle)["size"]["height"]-36)<1
                    screenshot("more-menu")
                    click_label(["流量","日志","配置"][index-4])
                else:
                    rpc("click_element",elementHandle=find("AppWindow::mobile-nav")[[0,1,2,3].index(index)+(1 if index>1 else 0)])
            else:
                rpc("click_element",elementHandle=find("AppWindow::nav-button")[index])
            time.sleep(.3)
        navigate(1)
        screenshot("proxies")
        if args.group_icons_only:
            wait_for(lambda:len(find("ProxyGroup::group-icon"))>=2)
            time.sleep(.5)
            screenshot("group-icons")
            for _ in range(3):
                click_label("刷新")
                time.sleep(.5)
            assert icon_requests.count(('http://icons.example.invalid/group.svg',None))==1, icon_requests
            assert icon_requests.count(('http://icons.example.invalid/missing.svg',None))==1, icon_requests
            assert all(auth is None for _,auth in icon_requests), 'Icon requests leaked core credentials'
            assert not find("AppWindow::toast"), 'Image errors must not interrupt the page'
            print('PASS: remote icons via proxy, inline SVG, shared cache and failed-load backoff')
            raise SystemExit(0)
        if args.proxy_scroll_only:
            viewport=find("ProxiesPage::proxy-scroll")[0]
            def group_positions():
                for attempt in range(5):
                    try:
                        return [(round(p["absolutePosition"]["x"],1), round(p["absolutePosition"]["y"],1), round(p["size"]["height"],1))
                                for p in (properties(h) for h in find("ProxiesPage::group"))]
                    except RuntimeError as error:
                        if "destroyed" not in str(error) or attempt==4:
                            raise
                        time.sleep(.1)
            initial=group_positions()
            rpc("scroll_element",elementHandle=viewport,deltaY=-(initial[0][2]+340))
            time.sleep(.5)
            samples=[group_positions()]
            assert samples[0], "Expected visible proxy group geometry"
            assert samples[0]!=initial, "Proxy viewport did not scroll"
            for x in {p[0] for p in samples[0]}:
                column=sorted(p for p in samples[0] if p[0]==x)
                assert all(a[1]+a[2]<=b[1] for a,b in zip(column,column[1:])), "Proxy groups overlap"
            screenshot("proxy-scroll-before")
            for tick in range(6):
                time.sleep(1)
                request=urllib.request.Request(f"http://127.0.0.1:{core_port}/proxies/Proxy", data=json.dumps({"name": "Tokyo 01" if tick%2 else "Tokyo 02"}).encode(), method="PUT", headers={"Authorization":"Bearer ui-fixture","Content-Type":"application/json"})
                with urllib.request.urlopen(request,timeout=5):
                    pass
                click_label("刷新")
                samples.append(group_positions())
            screenshot("proxy-scroll-after")
            (output/"proxy-scroll-positions.json").write_text(json.dumps(samples,ensure_ascii=False,indent=2))
            assert all(s==samples[0] for s in samples), f"Proxy groups moved during refresh: {samples}"
            print(f"PASS: stable proxy scroll at {args.size}")
            raise SystemExit(0)
        expanded=properties(find("ProxyGroup::expand-button")[0])
        rpc("click_element",elementHandle=find("ProxyGroup::expand-button")[0])
        time.sleep(.3)
        screenshot("proxy-collapsed")
        collapsed=properties(find("ProxyGroup::expand-button")[0])
        assert expanded["accessibleLabel"]!=collapsed["accessibleLabel"]
        rpc("click_element",elementHandle=find("ProxyGroup::expand-button")[0])
        time.sleep(.3)
        if not mobile:
            click_label("Tokyo 02")
            wait_for(lambda:api("/proxies/Proxy")["now"]=="Tokyo 02")
            screenshot("selected-proxy")
        search("Tokyo 02")
        selected=api("/proxies/Proxy")["now"]
        click_label("测速 · Tokyo 02")
        wait_for(lambda:any(sample["delay"]>0 for sample in api("/proxies/Tokyo%2002")["history"]))
        assert api("/proxies/Proxy")["now"]==selected, "Testing must preserve node selection"
        wait_for(lambda:any(item.get("accessibleLabel")=="操作完成" for item in elements()))
        wait_for(lambda:not find("AppWindow::toast"))
        screenshot("node-tested")
        search("Provider Direct")
        wait_for(lambda:any("Direct · UDP" in item.get("accessibleLabel","") for item in elements()))
        selected=api("/proxies/Streaming")["now"]
        assert "Provider Direct" not in api("/proxies")["proxies"], "Fixture must exercise provider-only lookup"
        click_label("测速 · Provider Direct")
        wait_for(lambda:any(sample["delay"]>0 for sample in api("/providers/proxies/fixture-provider")["proxies"][0]["history"]))
        assert api("/proxies/Streaming")["now"]==selected
        wait_for(lambda:not find("AppWindow::toast"))
        screenshot("provider-node-tested")
        search("")
        navigate(2)
        screenshot("rules")
        click_label("禁用规则")
        wait_for(lambda:api("/rules")["rules"][0].get("extra",{}).get("disabled",False))
        wait_for(lambda:any(item.get("accessibleLabel")=="启用规则" for item in elements()))
        rpc("hover_element",elementHandle=button("启用规则"))
        time.sleep(1.2)
        screenshot("rule-toggle")
        click_label("启用规则")
        wait_for(lambda:not api("/rules")["rules"][0].get("extra",{}).get("disabled",False))
        original_rules = api("/rules")["rules"]
        sort_column("ID")
        assert [int(row["cells"]["ID"]) for row in table_rows()[:3]] == [0, 1, 2]
        header = sort_column("ID", descending=True)
        assert [int(row["cells"]["ID"]) for row in table_rows()[:3]] == [13, 12, 11]
        screenshot("rules-sorted")
        assert [rule["payload"] for rule in api("/rules")["rules"]] == [rule["payload"] for rule in original_rules]
        rpc("dispatch_key_event", windowHandle=window, text=" ")
        wait_for(lambda: properties(header)["accessibleLabel"].endswith(" · 升序"))
        assert table_rows()[0]["cells"]["ID"] == "0"
        sort_column("类型")
        assert table_rows()[0]["cells"]["类型"] == "Domain"
        sort_column("ID")
        click_label("禁用规则")
        wait_for(lambda:api("/rules")["rules"][0]["extra"]["disabled"])
        listener=socket.socket()
        listener.settimeout(5)
        listener.bind(("127.0.0.1",0))
        listener.listen()
        destination=listener.getsockname()[1]
        tunnel=socket.create_connection(("127.0.0.1",proxy_port),timeout=5)
        tunnel.sendall(f"CONNECT 127.0.0.1:{destination} HTTP/1.1\r\nHost: 127.0.0.1:{destination}\r\n\r\n".encode())
        peer,_=listener.accept()
        assert b"200" in tunnel.recv(4096)
        transfer(peer, tunnel, 8192)
        wait_for(lambda: api("/connections").get("connections"))
        chart_listener=socket.socket()
        chart_connections.append(chart_listener)
        chart_listener.settimeout(5)
        chart_listener.bind(("127.0.0.1",chart_port))
        chart_listener.listen()
        chart_tunnel=socket.create_connection(("127.0.0.1",proxy_port),timeout=5)
        chart_connections.append(chart_tunnel)
        chart_tunnel.sendall(f"CONNECT 127.0.0.1:{chart_port} HTTP/1.1\r\nHost: 127.0.0.1:{chart_port}\r\n\r\n".encode())
        chart_peer,_=chart_listener.accept()
        chart_connections.append(chart_peer)
        assert b"200" in chart_tunnel.recv(4096)
        transfer(chart_peer, chart_tunnel, 40960)
        wait_for(lambda:len(api("/connections").get("connections") or [])==2)
        navigate(0)
        time.sleep(1.2)
        screenshot("overview-charts")
        overview_scroll=find("OverviewPage::overview-scroll")[0]
        rpc("scroll_element",elementHandle=overview_scroll,deltaY=-450)
        time.sleep(.25)
        screenshot("overview-middle-charts")
        rpc("scroll_element",elementHandle=overview_scroll,deltaY=-2000)
        time.sleep(.25)
        screenshot("overview-lower-charts")
        navigate(3)
        wait_for(lambda: len(table_rows()) == 2)
        sort_column("下载速度")
        sort_column("下载速度", descending=True)
        sort_column("下载量")
        show_column("主机")
        assert table_rows()[0]["cells"]["主机"].endswith(f":{destination}")
        header = sort_column("下载量", descending=True)
        screenshot("connections-sorted")
        show_column("主机")
        assert table_rows()[0]["cells"]["主机"].endswith(f":{chart_port}")
        # Live updates must re-sort using the selected column, preserving row identity.
        transfer(peer, tunnel, 65536)
        wait_for(lambda: table_rows()[0]["cells"]["主机"].endswith(f":{destination}"))
        assert properties(header)["accessibleLabel"].endswith(" · 降序")
        active = api("/connections")["connections"]
        closing_id = next(item["id"] for item in active if str(item["metadata"]["destinationPort"]) == str(destination))
        close = next(item["handle"] for item in table_rows()[0]["tree"]
                     if item.get("accessibleRole") == "Button" and item.get("accessibleLabel") == "关闭")
        rpc("click_element", elementHandle=close)
        wait_for(lambda: len(api("/connections")["connections"]) == 1)
        assert all(item["id"] != closing_id for item in api("/connections")["connections"])
        wait_for(lambda: len(table_rows()) == 1)
        screenshot("live-connections")
        click_label("关闭")
        wait_for(lambda: not api("/connections").get("connections"))
        for connection in chart_connections:
            connection.close()
        chart_connections.clear()
        for index,name in [(3,"connections"),(4,"traffic"),(5,"logs"),(6,"config")]:
            navigate(index)
            screenshot(name)
        # Enter each configuration tab: absent core values must remain unselected.
        click_label("XD 配置")
        screenshot("panel-settings")
        click_label("核心配置")
        assert 'ComboBox:' not in (output / "app.log").read_text(), "Invalid ComboBox selection warning"
        print(f"PASS: {args.size}; screenshots: {output}")
except Exception:
    if app is not None and app.poll() is None:
        screenshot("failure")
        (output/"failure-tree.json").write_text(json.dumps(elements(),ensure_ascii=False,indent=2))
    raise
finally:
    for connection in [tunnel,peer,listener,*chart_connections]:
        if connection is not None:
            connection.close()
    for process in [app,core]:
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
    if delay_server is not None:
        delay_server.shutdown()
        delay_server.server_close()
    shutil.rmtree(home)
