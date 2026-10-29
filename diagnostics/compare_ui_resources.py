"""Medicion observada por proceso, dos binarios reales y mismo borrador sin enviar."""
import argparse
from datetime import datetime, timezone
import hashlib,json,shutil,subprocess,tempfile,time
from pathlib import Path
import test_console as c
import measure_e2 as metrics

def run(binary,model,long_history=False):
    with tempfile.TemporaryDirectory(prefix='pill-ui-resource-') as temp:
        folder=Path(temp);project=folder/'project';(project/'.pillcli').mkdir(parents=True)
        (project/'.pillcli/config.json').write_text(json.dumps({'ollama':{'model':model,'num_ctx':16384,'num_predict':2048,'think':True}}),encoding='utf-8')
        history_text='\n'.join(f'history-{n:05}: '+('a'*64) for n in range(8192)) if long_history else ''
        if history_text:
            (project/'.pillcli/console.json').write_text(json.dumps({'lines':[history_text]}),encoding='utf-8')
        start=subprocess.STARTUPINFO();start.dwFlags|=subprocess.STARTF_USESHOWWINDOW;start.wShowWindow=0
        host=subprocess.Popen([shutil.which('pwsh'),'-NoProfile','-File',str(c.ROOT/'diagnostics/console_host.ps1'),str(folder),str(binary.resolve()),str(project)],creationflags=subprocess.CREATE_NEW_CONSOLE,startupinfo=start)
        try:
            c.wait_for(lambda:(folder/'ready').exists(),'host');c.K.FreeConsole();c.check(c.K.AttachConsole(host.pid));before_state=c.state();(folder/'start').touch()
            c.wait_for(lambda:'Preparado' in c.screen(),'seleccion de metadatos sin inferencia',seconds=30)
            first=metrics.process_sample(host.pid);started=time.monotonic()
            for _ in range(80):c.press('abc');time.sleep(.02)
            c.press('TEST_FIN');c.wait_for(lambda:'TEST_FIN' in c.screen(),'entrada retenida')
            after=metrics.process_sample(host.pid);elapsed=time.monotonic()-started
            screen=c.screen();c.press('\x03');c.wait_for(lambda:(folder/'result').exists(),'salida')
            assert int((folder/'result').read_text(encoding='utf-8-sig'))==0
            assert before_state==c.state()
            old={p['pid']:p for p in first};measured=[]
            for p in after:
                if p['group']=='prueba_cli' and p['name'].lower()=='pill.exe':
                    a=old[p['pid']];measured.append(dict(p,cpu_delta_seconds=p['cpu_seconds']-a['cpu_seconds']))
            assert len(measured)==1
            return {'sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'elapsed_seconds':elapsed,'events_text_chars':248,'seed_history_bytes':len(history_text.encode('utf-8')), 'cli':measured,'before':first,'after':after,'screen':screen,'console_restored':True}
        finally:
            if host.poll() is None:
                if not(folder/'result').exists():c.press('\x03')
                try:host.wait(timeout=5)
                except subprocess.TimeoutExpired:host.terminate();host.wait(timeout=3)
            c.K.FreeConsole()

def main():
    p=argparse.ArgumentParser();p.add_argument('--before',type=Path,required=True);p.add_argument('--after',type=Path,required=True);p.add_argument('--model',required=True);p.add_argument('--evidence',type=Path,required=True);p.add_argument('--long-history',action='store_true');a=p.parse_args()
    if a.evidence.exists():raise FileExistsError(a.evidence)
    report={'fecha_utc':datetime.now(timezone.utc).isoformat(),'result':'FAIL','scope':'Una muestra por binario con 80 entradas de3caracteres y marca final. Solo metadatos Ollama; borrador nunca enviado. Procesos independientes; no demuestra ahorro universal ni calidad de inferencia.'}
    try:
        report['before']=run(a.before,a.model,a.long_history);report['after']=run(a.after,a.model,a.long_history);report['result']='PASS'
    except Exception as e:report['error']=repr(e)
    a.evidence.write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8');print(report['result']);return 0 if report['result']=='PASS' else 1
if __name__=='__main__':raise SystemExit(main())
