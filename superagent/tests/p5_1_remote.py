import hashlib,json,os,secrets,socket,subprocess,sys,time
ENG="/Users/fanjia/Desktop/code/codex/superagent/packaging/superagent-engine"; HOME=os.path.expanduser("~/Library/Application Support/SuperAgent/codex-home")
PORT=18768; WORK="/tmp/p5-1-work"; OUT=os.path.expanduser("~/p5-1-outside.txt")
os.makedirs(WORK,exist_ok=True)
try: os.remove(OUT)
except FileNotFoundError: pass
token=secrets.token_hex(16); digest=hashlib.sha256(token.encode()).hexdigest()
base=os.environ.copy(); base["CODEX_HOME"]=HOME; base["RUST_LOG"]="error"
es_env=base.copy(); es_env["P5_REMOTE_MARKER"]="REMOTE-ENV-51"
ap_env=base.copy(); ap_env.pop("P5_REMOTE_MARKER",None)
es=subprocess.Popen([ENG,"exec-server","--listen",f"ws://127.0.0.1:{PORT}","--ws-auth","capability-token","--ws-token-sha256",digest,"-c",'sandbox_mode="workspace-write"',"-c",f'sandbox_workspace_write.writable_roots=["{WORK}"]'],stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,text=True,env=es_env)
def wait_port(p,t=15):
    e=time.time()+t
    while time.time()<e:
        try:
            with socket.create_connection(("127.0.0.1",p),timeout=.5): return True
        except OSError: time.sleep(.2)
    return False
if not wait_port(PORT):
    print("no server"); print(es.stderr.read()[-1500:]); sys.exit(1)
ap=subprocess.Popen([ENG,"app-server","--listen","stdio://"],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,env=ap_env)
def send(o): ap.stdin.write(json.dumps(o)+"\n"); ap.stdin.flush()
def ru(i,t=30):
    e=time.time()+t
    while time.time()<e:
        l=ap.stdout.readline()
        if not l: return None
        try: m=json.loads(l)
        except: continue
        if m.get("id")==i: return m
send({"id":1,"method":"initialize","params":{"clientInfo":{"name":"p5-1-rb","version":"0.1.0"},"capabilities":{"experimentalApi":True}}}); ru(1)
send({"method":"initialized","params":{}})
send({"id":2,"method":"environment/add","params":{"environmentId":"p5-1-rb","execServerUrl":f"ws://127.0.0.1:{PORT}","authBearerToken":token,"connectTimeoutMs":8000}}); print("env",ru(2,30))
send({"id":3,"method":"thread/start","params":{"cwd":WORK,"environments":[{"environmentId":"p5-1-rb","cwd":WORK}]}}); ts=ru(3,30); tid=ts["result"]["thread"]["id"]
prompt=("Use your shell tool to run exactly this command and report the full output and exit code: "
        "`echo MARKER=$P5_REMOTE_MARKER; pwd; echo ESCAPE > ~/p5-1-outside.txt; echo OUTSIDE_EXIT=$?`. "
        "Do not use any other command.")
send({"id":4,"method":"turn/start","params":{"threadId":tid,"input":[{"type":"text","text":prompt}]}})
e=time.time()+240; events=[]; text=[]
while time.time()<e:
    l=ap.stdout.readline()
    if not l: break
    try: m=json.loads(l)
    except: continue
    meth=m.get("method",""); events.append(meth)
    if meth=="item/agentMessage/delta":
        text.append((m.get("params",{}) or {}).get("delta","") or "")
    if meth in ("turn/completed","turn/failed","turn/error"): break
print("terminal",events[-1] if events else None,"outside_exists",os.path.exists(OUT),"outside_content",open(OUT).read() if os.path.exists(OUT) else "")
print("agent", "".join(text)[-1800:])
ap.terminate(); es.terminate()
