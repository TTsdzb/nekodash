#!/usr/bin/env python3
"""Stress the UI with bounded, synthetic WebSocket log bursts (no external core)."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import threading
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

parser = argparse.ArgumentParser()
parser.add_argument('--binary', default='target/debug/nekodash')
parser.add_argument('--output', default='target/ui-log-stress')
parser.add_argument('--tun-config', action='store_true')
parser.add_argument('--page', choices=['overview','logs'], default='overview')
args = parser.parse_args()
output = Path(args.output).resolve()
output.mkdir(parents=True, exist_ok=True)
start = threading.Event()
stop = threading.Event()
finished = threading.Event()
tun_config={'enable':False,'stack':'FutureStack'}
patches=[]
accept_patch=threading.Event()

def frame(value):
    data = json.dumps(value).encode()
    return b'\x81' + (bytes([len(data)]) if len(data)<126 else b'\x7e'+struct.pack('!H',len(data))) + data

class Core(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *_):
        pass

    def do_PATCH(self):
        body=json.loads(self.rfile.read(int(self.headers.get('Content-Length','0'))))
        patches.append(body)
        if accept_patch.is_set():
            tun_config.update(body.get('tun',{}))
            self.send_response(204)
            self.send_header('Content-Length','0')
            self.end_headers()
            return
        response=json.dumps({'message':'fixture rejects TUN change'}).encode()
        self.send_response(400)
        self.send_header('Content-Type','application/json')
        self.send_header('Content-Length',str(len(response)))
        self.end_headers(); self.wfile.write(response)

    def do_GET(self):
        path=self.path.split('?')[0]
        if self.headers.get('Upgrade','').lower()=='websocket':
            self.send_response(101)
            self.send_header('Upgrade','websocket')
            self.send_header('Connection','Upgrade')
            key=self.headers['Sec-WebSocket-Key']+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11'
            self.send_header('Sec-WebSocket-Accept',base64.b64encode(hashlib.sha1(key.encode()).digest()).decode())
            self.end_headers()
            try:
                if path=='/logs':
                    while not start.wait(.1):
                        if stop.is_set(): return
                    for burst in range(100):
                        self.wfile.write(b''.join(frame({'type':'info','payload':f'burst {burst}:{i}'}) for i in range(1000)))
                        if stop.wait(.05): return
                    self.wfile.write(frame({'type':'info','payload':'stress-finished'}))
                    finished.set()
                    stop.wait(30)
                else:
                    while not stop.is_set():
                        value={'/traffic':{'up':12345,'down':54321},'/memory':{'inuse':123456},'/connections':{'downloadTotal':0,'uploadTotal':0,'connections':[]}}[path]
                        self.wfile.write(frame(value))
                        stop.wait(.1)
            except (BrokenPipeError,ConnectionResetError):
                pass
            return
        values={'/version':{'version':'stress'},'/configs':{'mode':'rule','tun':tun_config},'/proxies':{'proxies':{}},'/providers/proxies':{'providers':{}},'/rules':{'rules':[]},'/providers/rules':{'providers':{}},'/connections':{'downloadTotal':0,'uploadTotal':0,'connections':[]}}
        body=json.dumps(values.get(path,{})).encode()
        self.send_response(200)
        self.send_header('Content-Type','application/json')
        self.send_header('Content-Length',str(len(body)))
        self.end_headers()
        self.wfile.write(body)

with socket.socket() as s:
    s.bind(('127.0.0.1',0)); mcp_port=s.getsockname()[1]

def rpc(name, **arguments):
    payload={'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':name,'arguments':arguments}}
    req=urllib.request.Request(f'http://127.0.0.1:{mcp_port}/mcp',json.dumps(payload).encode(),headers={'Content-Type':'application/json','Accept':'application/json, text/event-stream'})
    with urllib.request.urlopen(req,timeout=10) as response: raw=response.read().decode()
    raw=next((v[5:] for v in raw.splitlines() if v.startswith('data:')),raw)
    value=json.loads(raw)['result']
    if value.get('isError'): raise RuntimeError(value)
    return value

def result(value):
    return value.get('structuredContent') or json.loads(next(c['text'] for c in value['content'] if c['type']=='text'))

def capture(name):
    for c in rpc('take_screenshot',windowHandle=window)['content']:
        if c['type']=='image': (output/f'{name}.png').write_bytes(base64.b64decode(c['data']))

server=ThreadingHTTPServer(('127.0.0.1',0),Core)
threading.Thread(target=server.serve_forever,daemon=True).start()
try:
    with tempfile.TemporaryDirectory() as home, (output/'app.log').open('w') as log:
        Path(home,'endpoints.json').write_text(json.dumps({'schema_version':1,'selected':'stress','endpoints':[{'id':'stress','label':'Stress','url':f'http://127.0.0.1:{server.server_port}/','secret':''}]}))
        app=subprocess.Popen([str(Path(args.binary).resolve())],stdout=log,stderr=log,env=dict(os.environ,SLINT_BACKEND='headless',SLINT_MCP_PORT=str(mcp_port),NEKODASH_DATA_DIR=home,NEKODASH_WINDOW_SIZE='1280x820'))
        try:
            for attempt in range(100):
                try:
                    windows=result(rpc('list_windows'))['windowHandles']
                    if windows: break
                except (OSError,KeyError): pass
                time.sleep(.1)
            else: raise RuntimeError('UI did not start')
            window=windows[0]
            for attempt in range(100):
                nav=result(rpc('find_elements_by_id',windowHandle=window,elementsId='AppWindow::nav-button')).get('elementHandles',[])
                if nav: break
                time.sleep(.1)
            else: raise RuntimeError('Core did not connect')
            if args.tun_config:
                def find(identifier):
                    return result(rpc('find_elements_by_id',windowHandle=window,elementsId=identifier)).get('elementHandles',[])
                def props(handle):
                    return result(rpc('get_element_properties',elementHandle=handle))
                rpc('click_element',elementHandle=nav[6]); time.sleep(.5)
                choices=find('ConfigPage::config-choice')
                choice=next(h for h in choices if props(h).get('accessibleLabel')=='TUN 模式堆栈')
                assert props(choice).get('accessibleValue')=='FutureStack'
                capture('tun-unknown-current')
                rpc('click_element',elementHandle=choice); time.sleep(.2)
                root=result(rpc('get_window_properties',windowHandle=window))['rootElementHandle']
                tree=result(rpc('get_element_tree',elementHandle=root,maxElements=1500))
                (output/'tun-popup.json').write_text(json.dumps(tree,ensure_ascii=False,indent=2))
                items=[i for i in tree['elements'] if i.get('accessibleLabel')=='Mips']
                assert items, 'Mips option missing'
                rpc('click_element',elementHandle=items[0]['handle']); time.sleep(1)
                assert patches and patches[-1]=={'tun':{'stack':'Mips'}}, patches
                assert props(choice).get('accessibleValue')=='FutureStack', props(choice)
                toggle=next(h for h in find('ConfigPage::config-toggle') if 'TUN' in props(h).get('accessibleLabel',''))
                rpc('click_element',elementHandle=toggle); time.sleep(1)
                assert patches[-1]=={'tun':{'enable':True}}, patches
                capture('tun-rejected-restored')
                (output/'tun-toggle.json').write_text(json.dumps(props(toggle),ensure_ascii=False,indent=2))
                assert props(toggle).get('accessibleChecked',False) is False, props(toggle)
                assert find('AppWindow::toast'), 'Rejected save should explain the error'
                accept_patch.set()
                rpc('click_element',elementHandle=choice); time.sleep(.2)
                tree=result(rpc('get_element_tree',elementHandle=root,maxElements=1500))
                item=next(i for i in tree['elements'] if i.get('accessibleLabel')=='Mips')
                rpc('click_element',elementHandle=item['handle']); time.sleep(1)
                assert props(choice).get('accessibleValue')=='Mips', props(choice)
                rpc('click_element',elementHandle=toggle); time.sleep(1)
                assert props(toggle).get('accessibleChecked',False) is True, props(toggle)
                capture('tun-accepted')
                print('PASS: unknown TUN stack retained, Mips offered, rejected changes restored and successful changes displayed')
                raise SystemExit(0)
            if args.page=='logs':
                rpc('click_element',elementHandle=nav[5])
            capture("before-burst")
            start.set()
            errors=[]
            deadline=time.monotonic()+25
            samples=0
            while not finished.is_set() and time.monotonic()<deadline:
                handles=result(rpc('find_elements_by_id',windowHandle=window,elementsId='AppWindow::toast')).get('elementHandles',[])
                if handles:
                    root=result(rpc('get_window_properties',windowHandle=window))['rootElementHandle']
                    tree=result(rpc('get_element_tree',elementHandle=root,maxElements=1000))
                    if 'consumer missed' in json.dumps(tree): errors.append(samples)
                samples+=1
                time.sleep(.1)
            capture('burst-end')
            assert finished.is_set(), 'Log producer stalled'
            time.sleep(1)
            capture('overview-after-burst')
            nav=result(rpc('find_elements_by_id',windowHandle=window,elementsId='AppWindow::nav-button'))['elementHandles']
            rpc('click_element',elementHandle=nav[5])
            time.sleep(.5)
            search=result(rpc('find_elements_by_id',windowHandle=window,elementsId='PageToolbar::search-input'))['elementHandles'][0]
            rpc('set_element_value',elementHandle=search,value='stress-finished')
            time.sleep(.5)
            root=result(rpc('get_window_properties',windowHandle=window))['rootElementHandle']
            tree=result(rpc('get_element_tree',elementHandle=root,maxElements=1000))
            capture('latest-log')
            cells=result(rpc('find_elements_by_id',windowHandle=window,elementsId='DataTable::cell-text')).get('elementHandles',[])
            assert cells, 'Latest log missing from table'
            (output/'result.json').write_text(json.dumps({'events':100000,'samples':samples,'error_samples':errors},indent=2))
            assert not errors, f'Lag errors flooded UI: {len(errors)} samples'
            print('PASS: 100000 logs, responsive overview and latest log retained')
        finally:
            app.terminate()
            try: app.wait(timeout=10)
            except subprocess.TimeoutExpired: app.kill(); app.wait()
finally:
    stop.set()
    server.shutdown()
    server.server_close()
