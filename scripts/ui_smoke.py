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
  - MATCH,Fallback
'''
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
        time.sleep(.1)
    raise RuntimeError("Timed out waiting for UI/core state")

core=None
app=None
listener=None
peer=None
tunnel=None
chart_connections=[]
delay_server=None

class DelayHandler(BaseHTTPRequestHandler):
    def do_GET(self):
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
        app=subprocess.Popen([str(Path(args.binary).resolve())],stdout=app_log,stderr=app_log,env=dict(os.environ,SLINT_BACKEND="headless",SLINT_MCP_PORT=str(mcp_port),NEKODASH_WINDOW_SIZE=args.size,NEKODASH_DATA_DIR=str(home/"settings")))
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
                    click_label(["流量","日志","配置"][index-4])
                else:
                    rpc("click_element",elementHandle=find("AppWindow::mobile-nav")[[0,1,2,3].index(index)+(1 if index>1 else 0)])
            else:
                rpc("click_element",elementHandle=find("AppWindow::nav-button")[index])
            time.sleep(.3)
        navigate(1)
        screenshot("proxies")
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
        click_label("⇄")
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
        peer.sendall(b"ui-smoke"*1024)
        assert tunnel.recv(8192)
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
        chart_peer.sendall(b"chart"*8192)
        assert chart_tunnel.recv(40960)
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
        for connection in chart_connections:
            connection.close()
        chart_connections.clear()
        wait_for(lambda:len(api("/connections").get("connections") or [])==1)
        navigate(3)
        wait_for(lambda: any(item.get("accessibleLabel")=="关闭" and item.get("accessibleRole")=="Button" for item in elements()))
        screenshot("live-connections")
        click_label("关闭")
        wait_for(lambda: not api("/connections").get("connections"))
        for index,name in [(3,"connections"),(4,"traffic"),(5,"logs"),(6,"config")]:
            navigate(index)
            screenshot(name)
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
